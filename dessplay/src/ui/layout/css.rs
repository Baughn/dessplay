use super::compiler::{Diagnostic, LayoutBundle, Node};
use cssparser::{Parser, ParserInput, Token};
use std::collections::BTreeMap;
use std::path::Path;
use taffy::prelude::*;
use tuirealm::ratatui::style::{Color, Modifier, Style as PaintStyle};

#[derive(Clone, Debug)]
pub(super) struct Declaration {
    pub name: String,
    pub value: String,
    pub location: Diagnostic,
}
#[derive(Clone, Debug)]
pub(super) struct Rule {
    selectors: Vec<Selector>,
    declarations: Vec<Declaration>,
}
#[derive(Clone, Debug)]
struct Selector {
    parts: Vec<Part>,
    specificity: (usize, usize, usize),
}
#[derive(Clone, Debug, Default)]
struct Part {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    states: Vec<String>,
    child: bool,
}

pub(super) fn declarations(file: &Path, source: &str) -> Result<Vec<Declaration>, Diagnostic> {
    strict_syntax(file, source)?;
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_declarations(file, source, &mut parser)
        .map_err(|e| e.to_string())
        .map_err(|e| Diagnostic::at(file, source, 0, e))
}
fn parse_declarations<'i, 't>(
    file: &Path,
    source: &str,
    parser: &mut Parser<'i, 't>,
) -> Result<Vec<Declaration>, cssparser::ParseError<'i, String>> {
    let mut out = Vec::new();
    while !parser.is_exhausted() {
        let loc = parser.current_source_location();
        let name = parser.expect_ident_cloned()?.to_string();
        parser.expect_colon()?;
        let start = parser.position();
        parser.parse_until_before(cssparser::Delimiter::Semicolon, |p| {
            while !p.is_exhausted() {
                consume_value(p)?;
            }
            Ok(())
        })?;
        let value = parser.slice_from(start).trim().to_string();
        let location = Diagnostic {
            file: file.into(),
            line: loc.line as usize + 1,
            column: loc.column as usize,
            message: String::new(),
        };
        let declaration = Declaration {
            name,
            value,
            location,
        };
        check(&declaration).map_err(|e| parser.new_custom_error(e.message))?;
        out.push(declaration);
        let _ = parser.try_parse(|p| p.expect_semicolon());
    }
    let _ = source;
    Ok(out)
}
fn consume_value<'i>(p: &mut Parser<'i, '_>) -> Result<(), cssparser::ParseError<'i, String>> {
    match p.next()?.clone() {
        Token::Function(name) if name == "var" || name == "minmax" => p.parse_nested_block(|p| {
            while !p.is_exhausted() {
                consume_value(p)?;
            }
            Ok(())
        }),
        Token::Ident(_)
        | Token::Number { .. }
        | Token::Dimension { .. }
        | Token::Percentage { .. }
        | Token::Hash(_)
        | Token::IDHash(_)
        | Token::Comma
        | Token::Delim('/') => Ok(()),
        token => Err(p.new_custom_error(format!("unsupported CSS token {token:?}"))),
    }
}
pub(super) fn parse(file: &Path, source: &str) -> Result<Vec<Rule>, Diagnostic> {
    strict_syntax(file, source)?;
    let mut input = ParserInput::new(source);
    let mut p = Parser::new(&mut input);
    let mut out = Vec::new();
    while !p.is_exhausted() {
        let start = p.position();
        let location = p.current_source_location();
        loop {
            let before = p.position();
            match p.next_including_whitespace_and_comments().map_err(|e| {
                Diagnostic::at(file, source, 0, format!("expected selector block: {e:?}"))
            })? {
                Token::CurlyBracketBlock => {
                    let selectors =
                        selector_list(p.slice(start..before)).map_err(|message| Diagnostic {
                            file: file.into(),
                            line: location.line as usize + 1,
                            column: location.column as usize,
                            message,
                        })?;
                    let declarations = p
                        .parse_nested_block(|p| parse_declarations(file, source, p))
                        .map_err(|e| Diagnostic {
                            file: file.into(),
                            line: e.location.line as usize + 1,
                            column: e.location.column as usize,
                            message: format!("invalid declaration: {:?}", e.kind),
                        })?;
                    // cssparser follows browser error recovery for EOF. Our contract is strict.
                    if !p.slice_from(before).trim_end().ends_with('}') {
                        return Err(Diagnostic::at(
                            file,
                            source,
                            source.len(),
                            "unclosed CSS block",
                        ));
                    }
                    out.push(Rule {
                        selectors,
                        declarations,
                    });
                    break;
                }
                Token::AtKeyword(_) => {
                    return Err(Diagnostic::at(
                        file,
                        source,
                        0,
                        "CSS at-rules are unsupported",
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(out)
}
fn strict_syntax(file: &Path, source: &str) -> Result<(), Diagnostic> {
    fn scan<'i>(
        p: &mut Parser<'i, '_>,
        depth: usize,
    ) -> Result<(), cssparser::ParseError<'i, String>> {
        if depth > 64 {
            return Err(p.new_custom_error("CSS nesting limit exceeded"));
        }
        loop {
            let start = p.position();
            let token = match p.next_including_whitespace_and_comments() {
                Ok(token) => token.clone(),
                Err(error) if matches!(error.kind, cssparser::BasicParseErrorKind::EndOfInput) => {
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            let ending = match token {
                Token::CurlyBracketBlock => Some("}"),
                Token::SquareBracketBlock => Some("]"),
                Token::ParenthesisBlock | Token::Function(_) => Some(")"),
                Token::Comment(_) => {
                    if !p.slice_from(start).ends_with("*/") {
                        return Err(p.new_custom_error("unclosed CSS comment"));
                    }
                    None
                }
                Token::CloseCurlyBracket | Token::CloseSquareBracket | Token::CloseParenthesis => {
                    return Err(p.new_custom_error("unmatched CSS delimiter"));
                }
                _ => None,
            };
            if let Some(ending) = ending {
                p.parse_nested_block(|p| scan(p, depth + 1))?;
                if !p.slice_from(start).ends_with(ending) {
                    return Err(
                        p.new_custom_error(format!("unclosed CSS block; expected {ending}"))
                    );
                }
            }
        }
        Ok(())
    }
    let mut input = ParserInput::new(source);
    scan(&mut Parser::new(&mut input), 0).map_err(|e| Diagnostic {
        file: file.into(),
        line: e.location.line as usize + 1,
        column: e.location.column as usize,
        message: format!("invalid CSS syntax: {:?}", e.kind),
    })
}
fn selector_list(source: &str) -> Result<Vec<Selector>, String> {
    use cssparser::ToCss;
    let mut input = ParserInput::new(source);
    let mut p = Parser::new(&mut input);
    let mut text = String::new();
    let mut out = Vec::new();
    while let Ok(token) = p.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::Comment(_) => {}
            Token::Comma => {
                out.push(selector(&text)?);
                text.clear();
            }
            token => text.push_str(&token.to_css_string()),
        }
    }
    out.push(selector(&text)?);
    Ok(out)
}
fn selector(source: &str) -> Result<Selector, String> {
    let mut input = ParserInput::new(source);
    let mut p = Parser::new(&mut input);
    let mut parts = Vec::new();
    let mut part = Part::default();
    let mut specificity = (0, 0, 0);
    let mut have = false;
    let mut pending_space = false;
    while let Ok(token) = p.next_including_whitespace_and_comments().cloned() {
        if matches!(token, Token::Comment(_)) {
            continue;
        }
        if matches!(token, Token::WhiteSpace(_)) {
            pending_space = have;
            continue;
        }
        if matches!(token, Token::Delim('>')) {
            if !have {
                return Err("child selector requires a left operand".into());
            }
            parts.push(part);
            part = Part {
                child: true,
                ..Part::default()
            };
            have = false;
            pending_space = false;
            continue;
        }
        if pending_space {
            parts.push(part);
            part = Part::default();
            have = false;
            pending_space = false;
        }
        match token {
            Token::Ident(tag) if part.tag.is_none() && !have => {
                part.tag = Some(tag.to_string());
                specificity.2 += 1;
            }
            Token::Delim('*') if !have => {}
            Token::IDHash(id) if part.id.is_none() => {
                part.id = Some(id.to_string());
                specificity.0 += 1;
            }
            Token::Delim('.') => {
                part.classes.push(
                    p.expect_ident()
                        .map_err(|_| "expected class name")?
                        .to_string(),
                );
                specificity.1 += 1;
            }
            Token::Colon => {
                let state = p
                    .expect_ident()
                    .map_err(|_| "expected state name")?
                    .to_string();
                if !["focus", "selected", "disabled"].contains(&state.as_str()) {
                    return Err(format!("unsupported state :{state}"));
                }
                part.states.push(state);
                specificity.1 += 1;
            }
            other => return Err(format!("unsupported selector token {other:?}")),
        }
        have = true;
    }
    if !have {
        return Err("empty or incomplete selector".into());
    }
    parts.push(part);
    Ok(Selector { parts, specificity })
}
impl Part {
    fn matches(&self, node: &Node, states: &[&str]) -> bool {
        self.tag.as_ref().is_none_or(|tag| *tag == node.tag)
            && self.id.as_ref().is_none_or(|id| id == node.attr("id"))
            && self
                .classes
                .iter()
                .all(|c| node.attr("class").split_whitespace().any(|v| v == c))
            && self.states.iter().all(|s| states.contains(&s.as_str()))
    }
}
impl Selector {
    fn matches(&self, path: &[(&Node, Vec<&str>)]) -> bool {
        fn walk(parts: &[Part], path: &[(&Node, Vec<&str>)]) -> bool {
            let (Some(part), Some((node, states))) = (parts.last(), path.last()) else {
                return false;
            };
            if !part.matches(node, states) {
                return false;
            }
            if parts.len() == 1 {
                return true;
            }
            if part.child {
                return walk(&parts[..parts.len() - 1], &path[..path.len() - 1]);
            }
            (1..path.len())
                .rev()
                .any(|len| walk(&parts[..parts.len() - 1], &path[..len]))
        }
        walk(&self.parts, path)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Computed {
    pub layout: Style,
    pub paint: PaintStyle,
    pub border: Color,
    pub wrap: bool,
    pub ellipsis: bool,
    pub align: tuirealm::ratatui::layout::Alignment,
    pub variables: BTreeMap<String, String>,
    pub matched: Vec<Declaration>,
}
impl Default for Computed {
    fn default() -> Self {
        Self {
            layout: Style {
                flex_direction: FlexDirection::Column,
                flex_shrink: 0.0,
                min_size: Size {
                    width: Dimension::Length(0.0),
                    height: Dimension::Length(0.0),
                },
                ..Style::default()
            },
            paint: PaintStyle::default(),
            border: Color::DarkGray,
            wrap: false,
            ellipsis: false,
            align: Default::default(),
            variables: BTreeMap::new(),
            matched: Vec::new(),
        }
    }
}
pub(super) fn resolve(
    bundle: &LayoutBundle,
    path: &[(&Node, Vec<&str>)],
    parent: &Computed,
) -> Result<Computed, Diagnostic> {
    let Some((node, _)) = path.last() else {
        return Ok(Computed::default());
    };
    let mut out = Computed {
        paint: parent.paint,
        wrap: parent.wrap,
        ellipsis: parent.ellipsis,
        align: parent.align,
        variables: parent.variables.clone(),
        ..Computed::default()
    };
    if node.tag == "row" {
        out.layout.flex_direction = FlexDirection::Row;
    }
    if node.tag == "grid" {
        out.layout.display = Display::Grid;
    }
    if node.tag == "overlay" {
        out.layout.position = Position::Absolute;
        out.layout.inset = taffy::Rect {
            left: LengthPercentageAuto::Length(1.0),
            right: LengthPercentageAuto::Length(1.0),
            top: LengthPercentageAuto::Auto,
            bottom: LengthPercentageAuto::Length(0.0),
        };
    }
    let mut matched = Vec::new();
    for rule in &bundle.rules {
        if let Some(specificity) = rule
            .selectors
            .iter()
            .filter(|s| s.matches(path))
            .map(|s| s.specificity)
            .max()
        {
            matched.push((specificity, &rule.declarations));
        }
    }
    matched.sort_by_key(|(specificity, _)| *specificity);
    out.matched = matched
        .into_iter()
        .flat_map(|(_, d)| d.iter().cloned())
        .chain(node.inline.iter().cloned())
        .collect();
    for d in &out.matched {
        if d.name.starts_with("--") {
            out.variables.insert(d.name.clone(), d.value.clone());
        }
    }
    for d in out.matched.clone() {
        if d.name.starts_with("--") {
            continue;
        }
        let value =
            substitute(&d.value, &out.variables, &mut Vec::new()).map_err(|m| fail(&d, m))?;
        apply(&mut out, &d.name, &value).map_err(|m| fail(&d, m))?;
    }
    Ok(out)
}
fn substitute(
    value: &str,
    variables: &BTreeMap<String, String>,
    stack: &mut Vec<String>,
) -> Result<String, String> {
    substitute_bounded(value, variables, stack, &mut 4096)
}
fn substitute_bounded(
    value: &str,
    variables: &BTreeMap<String, String>,
    stack: &mut Vec<String>,
    fuel: &mut usize,
) -> Result<String, String> {
    let mut value = value.to_string();
    while let Some(start) = value.find("var(") {
        if *fuel == 0 || value.len() > 1024 * 1024 {
            return Err("custom property expansion limit exceeded".into());
        }
        *fuel -= 1;
        let mut depth = 1;
        let mut end = None;
        for (i, c) in value[start + 4..].char_indices() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + 4 + i);
                    break;
                }
            }
        }
        let end = end.ok_or("unclosed var()")?;
        let (name, fallback) = value[start + 4..end]
            .split_once(',')
            .map_or((&value[start + 4..end], None), |(n, f)| (n, Some(f.trim())));
        let name = name.trim();
        if !name.starts_with("--") || stack.iter().any(|n| n == name) || stack.len() >= 32 {
            return Err(format!("invalid or cyclic custom property {name:?}"));
        }
        stack.push(name.into());
        let replacement = variables
            .get(name)
            .map(String::as_str)
            .or(fallback)
            .ok_or_else(|| format!("undefined custom property {name:?}"))?;
        let replacement = substitute_bounded(replacement, variables, stack, fuel)?;
        stack.pop();
        if value.len() + replacement.len() > 1024 * 1024 {
            return Err("custom property expansion limit exceeded".into());
        }
        value.replace_range(start..end + 1, &replacement);
    }
    Ok(value)
}

fn fail(d: &Declaration, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        message: message.into(),
        ..d.location.clone()
    }
}
fn check(d: &Declaration) -> Result<(), Diagnostic> {
    if d.name.starts_with("--") {
        return Ok(());
    }
    let mut computed = Computed::default();
    if d.value.contains("var(") {
        // Validate property names now, substituted values on every possible state below.
        if !PROPERTIES.contains(&d.name.as_str()) {
            return Err(fail(d, format!("unsupported property {:?}", d.name)));
        }
        return Ok(());
    }
    apply(&mut computed, &d.name, &d.value).map_err(|m| fail(d, m))
}
const PROPERTIES: &[&str] = &[
    "display",
    "flex-direction",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "gap",
    "row-gap",
    "column-gap",
    "padding",
    "margin",
    "border",
    "border-color",
    "color",
    "background-color",
    "font-weight",
    "font-style",
    "text-decoration",
    "text-align",
    "white-space",
    "text-overflow",
    "align-items",
    "align-self",
    "justify-content",
    "grid-template-columns",
    "grid-template-rows",
    "grid-column",
    "grid-row",
];
fn length(value: &str) -> Result<Dimension, String> {
    if value == "auto" {
        return Ok(Dimension::Auto);
    }
    if value == "0" {
        return Ok(length_dim(0.0));
    }
    for unit in ["ch", "lh", "%"] {
        if let Some(raw) = value.strip_suffix(unit) {
            let n = number(raw)?;
            return Ok(if unit == "%" {
                Dimension::Percent(n / 100.0)
            } else {
                length_dim(n)
            });
        }
    }
    Err(format!(
        "expected auto, 0, ch, lh, or percentage length; got {value:?}"
    ))
}
fn length_dim(n: f32) -> Dimension {
    Dimension::Length(n)
}
fn number(value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 65535.0)
        .ok_or_else(|| format!("invalid nonnegative number {value:?}"))
}
fn lp(value: &str) -> Result<LengthPercentage, String> {
    match length(value)? {
        Dimension::Length(n) => Ok(LengthPercentage::Length(n)),
        Dimension::Percent(n) => Ok(LengthPercentage::Percent(n)),
        _ => Err("auto is unsupported here".into()),
    }
}
fn lpa(value: &str) -> Result<LengthPercentageAuto, String> {
    Ok(match length(value)? {
        Dimension::Length(n) => LengthPercentageAuto::Length(n),
        Dimension::Percent(n) => LengthPercentageAuto::Percent(n),
        _ => LengthPercentageAuto::Auto,
    })
}
fn edges<T: Copy>(
    value: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<taffy::Rect<T>, String> {
    let v = value
        .split_whitespace()
        .map(parse)
        .collect::<Result<Vec<_>, _>>()?;
    let (top, right, bottom, left) = match &v[..] {
        [a] => (*a, *a, *a, *a),
        [a, b] => (*a, *b, *a, *b),
        [a, b, c] => (*a, *b, *c, *b),
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => return Err("expected one to four edge lengths".into()),
    };
    Ok(taffy::Rect {
        top,
        right,
        bottom,
        left,
    })
}
fn color(value: &str) -> Result<Color, String> {
    Ok(match value {
        "default" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" => Color::Gray,
        "darkgray" => Color::DarkGray,
        value if value.starts_with('#') => {
            let raw = &value[1..];
            let expanded = if raw.len() == 3 {
                raw.chars().flat_map(|c| [c, c]).collect::<String>()
            } else {
                raw.into()
            };
            if expanded.len() != 6 || !expanded.is_ascii() {
                return Err("expected #RGB or #RRGGBB".into());
            }
            let rgb = u32::from_str_radix(&expanded, 16).map_err(|_| "invalid RGB color")?;
            Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
        }
        _ => return Err(format!("unsupported color {value:?}")),
    })
}
fn tracks(value: &str) -> Result<Vec<TrackSizingFunction>, String> {
    // Spaces inside minmax() do not separate tracks.
    let mut items = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in value.char_indices() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
        }
        if c.is_whitespace() && depth == 0 {
            if start < i {
                items.push(&value[start..i]);
            }
            start = i + 1;
        }
    }
    if start < value.len() {
        items.push(&value[start..]);
    }
    if items.is_empty() || items.len() > 64 {
        return Err("expected 1–64 explicit grid tracks".into());
    }
    items
        .into_iter()
        .map(|v| {
            if let Some(inner) = v.strip_prefix("minmax(").and_then(|v| v.strip_suffix(')')) {
                let (a, b) = inner.split_once(',').ok_or("minmax requires two tracks")?;
                let min = match a.trim() {
                    "auto" => MinTrackSizingFunction::Auto,
                    "max-content" => MinTrackSizingFunction::MaxContent,
                    v => MinTrackSizingFunction::Fixed(lp(v)?),
                };
                let max = max_track(b.trim())?;
                Ok(TrackSizingFunction::Single(taffy::MinMax { min, max }))
            } else {
                let max = max_track(v)?;
                let min = match v {
                    "auto" => MinTrackSizingFunction::Auto,
                    "max-content" => MinTrackSizingFunction::MaxContent,
                    v if v.ends_with("fr") => {
                        MinTrackSizingFunction::Fixed(LengthPercentage::Length(0.0))
                    }
                    v => MinTrackSizingFunction::Fixed(lp(v)?),
                };
                Ok(TrackSizingFunction::Single(taffy::MinMax { min, max }))
            }
        })
        .collect()
}
fn max_track(value: &str) -> Result<MaxTrackSizingFunction, String> {
    Ok(match value {
        "auto" => MaxTrackSizingFunction::Auto,
        "max-content" => MaxTrackSizingFunction::MaxContent,
        v if v.ends_with("fr") => MaxTrackSizingFunction::Fraction(number(&v[..v.len() - 2])?),
        v => MaxTrackSizingFunction::Fixed(lp(v)?),
    })
}
fn placement(value: &str) -> Result<taffy::Line<GridPlacement>, String> {
    let parse = |v: &str| -> Result<GridPlacement, String> {
        let v = v.trim();
        if v == "auto" {
            Ok(GridPlacement::Auto)
        } else if let Some(n) = v.strip_prefix("span ") {
            Ok(GridPlacement::Span(
                n.parse::<u16>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or("invalid grid span")?,
            ))
        } else {
            Ok(GridPlacement::Line(
                v.parse::<i16>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or("expected positive grid line")?
                    .into(),
            ))
        }
    };
    let (a, b) = value.split_once('/').unwrap_or((value, "auto"));
    Ok(taffy::Line {
        start: parse(a)?,
        end: parse(b)?,
    })
}
fn apply(out: &mut Computed, name: &str, value: &str) -> Result<(), String> {
    let invalid = || format!("unsupported {name} value {value:?}");
    match name {
        "display" => {
            out.layout.display = match value {
                "flex" => Display::Flex,
                "grid" => Display::Grid,
                "none" => Display::None,
                _ => return Err(invalid()),
            }
        }
        "flex-direction" => {
            out.layout.flex_direction = match value {
                "row" => FlexDirection::Row,
                "column" => FlexDirection::Column,
                _ => return Err(invalid()),
            }
        }
        "flex-grow" => out.layout.flex_grow = number(value)?,
        "flex-shrink" => out.layout.flex_shrink = number(value)?,
        "flex-basis" => out.layout.flex_basis = length(value)?,
        "width" => out.layout.size.width = length(value)?,
        "height" => out.layout.size.height = length(value)?,
        "min-width" => out.layout.min_size.width = length(value)?,
        "min-height" => out.layout.min_size.height = length(value)?,
        "max-width" => out.layout.max_size.width = length(value)?,
        "max-height" => out.layout.max_size.height = length(value)?,
        "gap" => {
            let v = value.split_whitespace().collect::<Vec<_>>();
            match &v[..] {
                [a] => {
                    out.layout.gap = Size {
                        width: lp(a)?,
                        height: lp(a)?,
                    }
                }
                [a, b] => {
                    out.layout.gap = Size {
                        width: lp(b)?,
                        height: lp(a)?,
                    }
                }
                _ => return Err(invalid()),
            }
        }
        "row-gap" => out.layout.gap.height = lp(value)?,
        "column-gap" => out.layout.gap.width = lp(value)?,
        "padding" => out.layout.padding = edges(value, lp)?,
        "margin" => out.layout.margin = edges(value, lpa)?,
        "border" => {
            let v = match value {
                "0" | "none" => 0.0,
                "1" | "1ch" | "1lh" => 1.0,
                _ => return Err(invalid()),
            };
            out.layout.border = taffy::Rect {
                left: LengthPercentage::Length(v),
                right: LengthPercentage::Length(v),
                top: LengthPercentage::Length(v),
                bottom: LengthPercentage::Length(v),
            };
        }
        "border-color" => out.border = color(value)?,
        "color" => out.paint = out.paint.fg(color(value)?),
        "background-color" => out.paint = out.paint.bg(color(value)?),
        "font-weight" => {
            out.paint = match value {
                "bold" => out.paint.add_modifier(Modifier::BOLD),
                "normal" => out.paint.remove_modifier(Modifier::BOLD),
                _ => return Err(invalid()),
            }
        }
        "font-style" => {
            out.paint = match value {
                "italic" => out.paint.add_modifier(Modifier::ITALIC),
                "normal" => out.paint.remove_modifier(Modifier::ITALIC),
                _ => return Err(invalid()),
            }
        }
        "text-decoration" => {
            out.paint = match value {
                "underline" => out.paint.add_modifier(Modifier::UNDERLINED),
                "none" => out.paint.remove_modifier(Modifier::UNDERLINED),
                _ => return Err(invalid()),
            }
        }
        "white-space" => {
            out.wrap = match value {
                "normal" => true,
                "nowrap" | "pre" => false,
                _ => return Err(invalid()),
            }
        }
        "text-overflow" => {
            out.ellipsis = match value {
                "ellipsis" => true,
                "clip" => false,
                _ => return Err(invalid()),
            }
        }
        "text-align" => {
            out.align = match value {
                "left" => tuirealm::ratatui::layout::Alignment::Left,
                "center" => tuirealm::ratatui::layout::Alignment::Center,
                "right" => tuirealm::ratatui::layout::Alignment::Right,
                _ => return Err(invalid()),
            }
        }
        "align-items" | "align-self" => {
            let v = match value {
                "start" => AlignItems::Start,
                "end" => AlignItems::End,
                "center" => AlignItems::Center,
                "stretch" => AlignItems::Stretch,
                _ => return Err(invalid()),
            };
            if name == "align-items" {
                out.layout.align_items = Some(v);
            } else {
                out.layout.align_self = Some(v);
            }
        }
        "justify-content" => {
            out.layout.justify_content = Some(match value {
                "start" => JustifyContent::Start,
                "end" => JustifyContent::End,
                "center" => JustifyContent::Center,
                "space-between" => JustifyContent::SpaceBetween,
                "space-around" => JustifyContent::SpaceAround,
                _ => return Err(invalid()),
            })
        }
        "grid-template-columns" => out.layout.grid_template_columns = tracks(value)?,
        "grid-template-rows" => out.layout.grid_template_rows = tracks(value)?,
        "grid-column" => out.layout.grid_column = placement(value)?,
        "grid-row" => out.layout.grid_row = placement(value)?,
        _ => return Err(format!("unsupported property {name:?}")),
    }
    Ok(())
}
pub(super) fn validate_bundle(bundle: &LayoutBundle) -> Result<(), Diagnostic> {
    fn visit<'a>(
        bundle: &LayoutBundle,
        node: &'a Node,
        path: &mut Vec<(&'a Node, Vec<&'static str>)>,
        parent: &Computed,
        remaining: &mut usize,
    ) -> Result<(), Diagnostic> {
        let possible = bundle
            .rules
            .iter()
            .flat_map(|r| &r.selectors)
            .flat_map(|s| &s.parts)
            .filter(|p| p.matches(node, &["focus", "selected", "disabled"]))
            .flat_map(|p| &p.states)
            .fold(0, |mask, state| {
                mask | match state.as_str() {
                    "focus" => 1,
                    "selected" => 2,
                    "disabled" => 4,
                    _ => 0,
                }
            });
        // Include ancestor states: inherited variables and descendant selectors
        // must also validate before the first focus change at runtime.
        for bits in 0..8 {
            if bits & !possible != 0 {
                continue;
            }
            if *remaining == 0 {
                return Err(Diagnostic {
                    message: "layout has too many conditional style combinations".into(),
                    ..node.location.clone()
                });
            }
            *remaining -= 1;
            let states = ["focus", "selected", "disabled"]
                .into_iter()
                .enumerate()
                .filter_map(|(i, s)| (bits & (1 << i) != 0).then_some(s))
                .collect();
            path.push((node, states));
            let computed = resolve(bundle, path, parent)?;
            for child in &node.children {
                visit(bundle, child, path, &computed, remaining)?;
            }
            path.pop();
        }
        Ok(())
    }
    for root in bundle.templates.values() {
        visit(
            bundle,
            root,
            &mut Vec::new(),
            &Computed::default(),
            &mut 65536,
        )?;
    }
    Ok(())
}
