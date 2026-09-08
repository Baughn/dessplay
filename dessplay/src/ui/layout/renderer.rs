use super::compiler::Node;
use super::css::{Computed, resolve};
use super::{Diagnostic, Fragment, LayoutBundle, measure_text};
use std::collections::{BTreeMap, HashMap};
use taffy::prelude::*;
use tuirealm::ratatui::{
    Frame,
    layout::Rect,
    style::Style as PaintStyle,
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

/// Typed presentation fields. Strings are always data, never parsed as markup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presentation {
    texts: BTreeMap<String, String>,
    progress: BTreeMap<String, (u64, u64)>,
    lists: BTreeMap<String, Vec<PresentedItem>>,
    rich: BTreeMap<String, Vec<RichSpan>>,
    bools: BTreeMap<String, bool>,
    slots: BTreeMap<String, (u16, u16)>,
    states: BTreeMap<String, Vec<String>>,
    styles: BTreeMap<String, PaintStyle>,
    component_styles: BTreeMap<String, PaintStyle>,
    preserve_end: Vec<String>,
    inherited_style: PaintStyle,
    inherited_indent: u16,
    shares: BTreeMap<String, u16>,
    intrinsic_widths: BTreeMap<String, u16>,
    root_states: Vec<String>,
    color_variables: BTreeMap<String, String>,
}
impl Presentation {
    fn same_measurement(&self, other: &Self) -> bool {
        if !self.lists.is_empty() || !other.lists.is_empty() {
            return false;
        }
        let normalize = |data: &Self| {
            let mut data = data.clone();
            for span in data.rich.values_mut().flatten() {
                span.marks.clear();
                // ASCII letter/digit substitutions preserve every word boundary,
                // source index, and cell width. Non-ASCII text remains exact.
                if span.text.is_ascii() {
                    span.text = span
                        .text
                        .chars()
                        .map(|ch| if ch.is_ascii_alphanumeric() { 'a' } else { ch })
                        .collect();
                }
            }
            data
        };
        normalize(self) == normalize(other)
    }
    /// Semantic selection state on the root of a component or repeated item.
    pub fn selected(mut self, selected: bool) -> Self {
        self.root_states.retain(|state| state != "selected");
        if selected {
            self.root_states.push("selected".into());
        }
        self
    }
    /// Keyed semantic items for a template-authored repeat.
    pub fn list(mut self, name: &str, items: Vec<PresentedItem>) -> Self {
        self.lists.insert(name.into(), items);
        self
    }
    /// Text properties inherited across a controller primitive boundary.
    pub fn inherit(mut self, style: PaintStyle, indent: u16) -> Self {
        self.inherited_style = style;
        self.inherited_indent = indent;
        self
    }
    /// Styled data with optional controller-owned action identities, never markup.
    pub fn rich(mut self, name: &str, spans: Vec<RichSpan>) -> Self {
        self.rich.insert(name.into(), spans);
        self
    }
    /// Drag proportions in basis points, separate from authored CSS.
    pub(crate) fn shares(mut self, shares: BTreeMap<String, u16>) -> Self {
        self.shares = shares;
        self
    }
    /// A typed semantic theme color. Authored custom-property declarations win.
    pub fn color_variable(mut self, name: &str, color: tuirealm::ratatui::style::Color) -> Self {
        use tuirealm::ratatui::style::Color;
        let color = if let Color::Indexed(index) = color {
            crate::ui::theme::xterm_rgb(index)
        } else {
            color
        };
        let value = match color {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            Color::Reset => "default".into(),
            other => other.to_string().to_lowercase(),
        };
        self.color_variables.insert(name.into(), value);
        self
    }
    /// Unwrapped shared-column content width, supplied without padding text.
    pub fn intrinsic_width(mut self, binding: &str, width: u16) -> Self {
        self.intrinsic_widths.insert(binding.into(), width);
        self
    }
    /// Progress quantities; ratios are clamped by the terminal fill primitive.
    pub fn progress(mut self, name: &str, completed: u64, total: u64) -> Self {
        self.progress.insert(name.into(), (completed, total));
        self
    }
    /// Supply a plain-text binding.
    pub fn text(mut self, name: &str, value: impl Into<String>) -> Self {
        self.texts.insert(name.into(), value.into());
        self
    }
    /// Supply a condition binding.
    pub fn boolean(mut self, name: &str, value: bool) -> Self {
        self.bools.insert(name.into(), value);
        self
    }
    /// Supply the intrinsic unwrapped size of a controller-owned primitive.
    pub fn slot(mut self, name: &str, width: u16, height: u16) -> Self {
        self.slots.insert(name.into(), (width, height));
        self
    }
    /// Supply semantic focus/selection/disabled state for a stable node id.
    pub fn state(mut self, id: &str, state: &str) -> Self {
        self.states.entry(id.into()).or_default().push(state.into());
        self
    }
    /// Semantic control appearance before authored declarations, including inline separators.
    pub fn component_style(mut self, id: &str, style: PaintStyle) -> Self {
        self.component_styles.insert(id.into(), style);
        self
    }
    /// Semantic text style; authored CSS has precedence over this default.
    pub fn style(mut self, binding: &str, style: PaintStyle) -> Self {
        self.styles.insert(binding.into(), style);
        self
    }
    /// Keep the filename end of a path visible when its primitive is clipped.
    pub fn preserve_end(mut self, binding: &str) -> Self {
        self.preserve_end.push(binding.into());
        self
    }
}

/// One application-supplied rich-text span. Actions are opaque controller keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichSpan {
    /// Literal display text.
    pub text: String,
    /// Semantic appearance, before authored CSS overrides.
    pub style: PaintStyle,
    /// Existing controller action identity, such as a spoiler toggle.
    pub action: Option<String>,
    /// Paint-only combining diacritics (U+0300–U+036F), keyed by source character.
    pub marks: BTreeMap<usize, String>,
}

/// Source identity and cell range emitted from the same fragments that are painted.
#[derive(Clone, Debug)]
pub struct TextRegion {
    /// Stable authored node identity, when supplied.
    pub node: String,
    /// Presentation field supplying these characters.
    pub binding: String,
    /// Character offsets within that field, independent of UTF-8 byte width.
    pub source: std::ops::Range<usize>,
    /// Unclipped cell rectangle in the scene's coordinate system.
    pub bounds: Rect,
    /// The same ancestor/content clip used while painting this span.
    pub clip: Rect,
    /// Controller action associated with this span.
    pub action: Option<String>,
}

#[derive(Clone, Debug)]
struct TextRun {
    range: std::ops::Range<usize>,
    source: usize,
    binding: String,
    node: String,
    style: PaintStyle,
    action: Option<String>,
    marks: BTreeMap<usize, String>,
}

/// A stable semantic item; templates own its spacing and composition.
#[derive(Clone, Debug, PartialEq)]
pub struct PresentedItem {
    /// Stable controller identity, independent of list order.
    pub key: String,
    /// Fields in the list's item binding contract.
    pub data: Presentation,
}

/// A stable, controller-owned row in a virtualized collection.
#[derive(Clone, Debug, PartialEq)]
pub struct PresentedRow {
    /// Semantic identity, independent of ordering.
    pub key: String,
    /// Typed fields exposed before padding or clipping.
    pub data: Presentation,
    /// A blank, noninteractive row following the item.
    pub gap_after: bool,
}

/// Interaction records produced alongside a painted semantic collection.
#[derive(Clone, Debug, Default)]
pub struct RenderedCollection {
    rows: Vec<(Rect, usize)>,
}
impl RenderedCollection {
    /// Controller row identity under the pointer, including scrolled offsets.
    pub fn hit(&self, column: u16, row: u16) -> Option<usize> {
        let point = tuirealm::ratatui::layout::Position::new(column, row);
        self.rows
            .iter()
            .find_map(|(bounds, index)| bounds.contains(point).then_some(*index))
    }
}

/// Painted geometry and semantic slots, published as one immutable frame result.
#[derive(Clone, Debug, Default)]
pub struct RenderedScene {
    nodes: Vec<Arranged>,
    depth: crate::ui::theme::ColorDepth,
    paint_order: Vec<usize>,
    height: u16,
    width: u16,
    /// Arranged keyed items for controller navigation and inspection.
    pub items: Vec<RepeatedItem>,
    /// Rich/plain text source ranges; consumers apply the scene's viewport transform.
    pub text_regions: Vec<TextRegion>,
    pub(crate) splits: Vec<SplitRegion>,
}

#[derive(Clone, Debug)]
/// Geometry for one expanded semantic item.
pub struct RepeatedItem {
    /// The list binding in the owning presentation.
    pub binding: String,
    /// The controller-supplied stable item key.
    pub key: String,
    /// Qualified template identity, including enclosing repetition keys.
    pub instance: String,
    /// Original allocated item rectangle.
    pub bounds: Rect,
    /// Ancestor clip shared with painting.
    pub clip: Rect,
}

/// A handle and its adjacent children, derived from the painted scene.
#[derive(Clone, Debug)]
pub(crate) struct SplitRegion {
    pub id: String,
    pub handle: Rect,
    horizontal: bool,
    origin: u16,
    extent: u16,
    before: usize,
    children: Vec<(String, u16)>,
}
impl SplitRegion {
    pub fn drag(&self, position: tuirealm::ratatui::layout::Position) -> BTreeMap<String, u16> {
        let coordinate = if self.horizontal {
            position.x
        } else {
            position.y
        };
        let offset = u32::from(coordinate.saturating_sub(self.origin));
        let pointer = (offset * 10_000 / u32::from(self.extent.max(1))).min(10_000) as u16;
        let mut children = self.children.clone();
        let preceding: u16 = children[..self.before].iter().map(|(_, n)| n).sum();
        let pair = children[self.before].1 + children[self.before + 1].1;
        // Preserve the default ten-percent minimum of the entire container.
        let minimum = 1000.min(pair / 2);
        let share = pointer
            .saturating_sub(preceding)
            .clamp(minimum, pair - minimum);
        children[self.before].1 = share;
        children[self.before + 1].1 = pair - share;
        children.into_iter().collect()
    }
}
#[derive(Clone, Debug)]
struct Arranged {
    subtree_end: usize,
    progress: Option<(u64, u64)>,
    overlay: bool,
    overlay_root: bool,
    id: String,
    slot: String,
    bounds: Rect,
    margin_bottom: u16,
    content: Rect,
    clip: Rect,
    style: Computed,
    title: String,
    title_bottom: String,
    fragments: Vec<Fragment>,
    lines: Vec<Line<'static>>,
    align_offsets: Vec<u16>,
    runs: Vec<TextRun>,
}
impl RenderedScene {
    /// Natural height of the arranged root, including its chrome.
    pub fn height(&self) -> u16 {
        self.height
    }
    /// Arranged visible bounds of a named component.
    pub fn bounds(&self, id: &str) -> Rect {
        self.nodes
            .iter()
            .find(|n| n.id == id)
            .map_or(Rect::default(), |n| n.bounds.intersection(n.clip))
    }
    /// Content box of this entry, after authored border, padding, and margins.
    pub fn root_content(&self) -> Rect {
        self.nodes
            .first()
            .map_or(Rect::default(), |node| node.content.intersection(node.clip))
    }
    pub(super) fn root_style(&self) -> PaintStyle {
        crate::ui::theme::paint_style(
            self.nodes
                .first()
                .map_or(PaintStyle::default(), |node| node.style.paint),
            self.depth,
        )
    }
    #[cfg(test)]
    pub(super) fn line_text(&self, row: u16) -> String {
        let mut parts: Vec<_> = self
            .nodes
            .iter()
            .flat_map(|node| {
                node.fragments.iter().filter_map(move |fragment| {
                    (usize::from(node.content.y) + fragment.row == usize::from(row))
                        .then_some((node.content.x, fragment.text.as_str()))
                })
            })
            .collect();
        parts.sort_by_key(|(x, _)| *x);
        parts
            .into_iter()
            .map(|(_, text)| text)
            .collect::<Vec<_>>()
            .join(" ")
    }
    /// Visible controller slots, in markup order (independent of box placement).
    pub fn visible_slots(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|n| !n.slot.is_empty() && !n.content.intersection(n.clip).is_empty())
            .map(|n| n.slot.as_str())
            .collect()
    }
    /// Visible content rectangle for a controller primitive, from painted geometry.
    pub fn slot(&self, name: &str) -> Rect {
        self.nodes
            .iter()
            .find(|n| n.slot == name)
            .map_or(Rect::default(), |n| n.content.intersection(n.clip))
    }
    /// Terminal continuation indentation resolved for a text primitive.
    pub fn hanging_indent(&self, name: &str) -> usize {
        self.nodes
            .iter()
            .find(|n| n.slot == name)
            .map_or(0, |n| usize::from(n.style.hanging_indent))
    }
    /// Vertical extent occupied by the template children, including margins.
    pub(crate) fn occupied_height(&self) -> u16 {
        let children = if self.nodes.len() > 1 {
            &self.nodes[1..]
        } else {
            &self.nodes[..]
        };
        children
            .iter()
            .map(|n| n.bounds.bottom().saturating_add(n.margin_bottom))
            .max()
            .unwrap_or(0)
    }
    /// Resolved text appearance for a controller primitive.
    pub fn style(&self, name: &str) -> PaintStyle {
        self.nodes
            .iter()
            .find(|n| n.slot == name)
            .map_or(PaintStyle::default(), |n| n.style.paint)
    }
    /// Arranged bounds and matched declarations for the layout inspector.
    pub fn inspect(&self) -> Vec<String> {
        self.nodes
            .iter()
            .map(|n| {
                format!(
                    "{} {} {:?} clip {:?}\n{}",
                    n.id,
                    n.slot,
                    n.bounds,
                    n.clip,
                    n.style
                        .matched
                        .iter()
                        .map(|d| format!(
                            "  {}: {} ({}:{})",
                            d.name,
                            d.value,
                            d.location.file.display(),
                            d.location.line
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            })
            .collect()
    }
    /// Whether a visible explicit overlay needs terminal-graphics suppression.
    pub fn has_overlay(&self) -> bool {
        self.nodes
            .iter()
            .any(|n| n.overlay_root && !n.bounds.intersection(n.clip).is_empty())
    }
    /// Paint container chrome and already-measured text. Slots are filled by primitives.
    pub fn paint(&self, frame: &mut Frame<'_>) {
        self.paint_layer(frame, false, PaintView::new(frame.area(), 0, 0));
    }
    /// Paint declared overlays after ordinary content primitives.
    pub fn paint_overlays(&self, frame: &mut Frame<'_>) {
        self.paint_layer(frame, true, PaintView::new(frame.area(), 0, 0));
    }
    /// Fill primitive slots in the same layer order as their template chrome.
    pub fn paint_with_slots(
        &self,
        frame: &mut Frame<'_>,
        mut paint: impl FnMut(&str, &mut Frame<'_>, Rect, PaintStyle),
    ) {
        let view = PaintView::new(frame.area(), 0, 0);
        self.paint_layer_with_slots(frame, false, view, &mut paint);
        self.paint_layer_with_slots(frame, true, view, &mut paint);
    }
    /// Paint an already arranged local scene through a scrolling viewport.
    pub fn paint_scrolled(&self, frame: &mut Frame<'_>, viewport: Rect, row_offset: i32) {
        let view = PaintView::new(
            viewport.intersection(frame.area()),
            i32::from(viewport.x),
            i32::from(viewport.y) + row_offset,
        );
        self.paint_layer(frame, false, view);
        self.paint_layer(frame, true, view);
    }
    /// Highlight a semantic source range using painted fragments. `None` selects
    /// all text in the scene, including first-line prefix separators.
    #[allow(clippy::too_many_arguments)]
    pub fn highlight_text(
        &self,
        frame: &mut Frame<'_>,
        viewport: Rect,
        row_offset: i32,
        binding: Option<&str>,
        source: Option<std::ops::Range<usize>>,
        text: &str,
        style: PaintStyle,
    ) {
        let view = PaintView::new(
            viewport.intersection(frame.area()),
            i32::from(viewport.x),
            i32::from(viewport.y) + row_offset,
        );
        let mut rows = BTreeMap::<u16, Rect>::new();
        for region in &self.text_regions {
            if binding.is_some_and(|binding| binding != region.binding) {
                continue;
            }
            let mut bounds = region.bounds;
            if let Some(source) = &source {
                let start = source.start.max(region.source.start);
                let end = source.end.min(region.source.end);
                if start >= end {
                    continue;
                }
                let prefix: String = text
                    .chars()
                    .skip(region.source.start)
                    .take(start - region.source.start)
                    .collect();
                let selected: String = text.chars().skip(start).take(end - start).collect();
                bounds.x = bounds.x.saturating_add(prefix.width() as u16);
                bounds.width = selected.width().min(u16::MAX as usize) as u16;
            }
            bounds = bounds.intersection(region.clip);
            if bounds.is_empty() {
                continue;
            }
            rows.entry(bounds.y)
                .and_modify(|row| *row = row.union(bounds))
                .or_insert(bounds);
        }
        for bounds in rows.into_values() {
            let visible = view.rect(bounds);
            if !visible.is_empty() {
                frame
                    .buffer_mut()
                    .set_style(visible, crate::ui::theme::paint_style(style, self.depth));
            }
        }
    }
    /// Outer box for a semantic primitive, including authored borders.
    pub fn slot_bounds(&self, name: &str) -> Rect {
        self.nodes
            .iter()
            .find(|n| n.slot == name)
            .map_or(Rect::default(), |n| n.bounds.intersection(n.clip))
    }
    /// Visible image/primitive bounds using the same scroll translation as paint.
    pub fn scrolled_slot(&self, name: &str, viewport: Rect, row_offset: i32) -> Rect {
        PaintView::new(
            viewport,
            i32::from(viewport.x),
            i32::from(viewport.y) + row_offset,
        )
        .rect(self.slot(name))
    }
    fn paint_layer(&self, frame: &mut Frame<'_>, overlay: bool, view: PaintView) {
        self.paint_layer_with_slots(frame, overlay, view, &mut |_, _, _, _| {});
    }
    fn paint_layer_with_slots(
        &self,
        frame: &mut Frame<'_>,
        overlay: bool,
        view: PaintView,
        paint: &mut impl FnMut(&str, &mut Frame<'_>, Rect, PaintStyle),
    ) {
        for n in self
            .paint_order
            .iter()
            .map(|index| &self.nodes[*index])
            .filter(|n| n.overlay == overlay)
        {
            let visible = view.rect(n.bounds.intersection(n.clip));
            if visible.is_empty() {
                continue;
            }
            if n.overlay_root {
                frame.render_widget(tuirealm::ratatui::widgets::Clear, visible);
                frame
                    .buffer_mut()
                    .set_style(visible, crate::ui::theme::canvas_style(self.depth));
            }
            frame.buffer_mut().set_style(
                visible,
                crate::ui::theme::paint_style(n.style.paint, self.depth),
            );
            let border = &n.style.layout.border;
            let has_border = [border.left, border.right, border.top, border.bottom]
                .iter()
                .any(|edge| *edge != LengthPercentage::Length(0.0));
            if has_border || !n.title.is_empty() || !n.title_bottom.is_empty() {
                // Draw only original frame edges inside the clip. Memory use is
                // bounded by the terminal, even for enormous authored dimensions.
                if has_border {
                    for y in visible.y..visible.bottom() {
                        for x in visible.x..visible.right() {
                            let left = border.left != LengthPercentage::Length(0.0)
                                && i32::from(x) - view.x == i32::from(n.bounds.x);
                            let right = border.right != LengthPercentage::Length(0.0)
                                && i32::from(x) - view.x
                                    == i32::from(n.bounds.right().saturating_sub(1));
                            let top = border.top != LengthPercentage::Length(0.0)
                                && i32::from(y) - view.y == i32::from(n.bounds.y);
                            let bottom = border.bottom != LengthPercentage::Length(0.0)
                                && i32::from(y) - view.y
                                    == i32::from(n.bounds.bottom().saturating_sub(1));
                            let glyph = match (left, right, top, bottom) {
                                (true, _, true, _) => "┌",
                                (_, true, true, _) => "┐",
                                (true, _, _, true) => "└",
                                (_, true, _, true) => "┘",
                                (_, _, true, _) | (_, _, _, true) => "─",
                                (true, _, _, _) | (_, true, _, _) => "│",
                                _ => continue,
                            };
                            frame.buffer_mut()[(x, y)].set_symbol(glyph).set_style(
                                crate::ui::theme::paint_style(
                                    n.style.paint.fg(n.style.border),
                                    self.depth,
                                ),
                            );
                        }
                    }
                }
                for (text, y) in [
                    (&n.title, n.bounds.y),
                    (&n.title_bottom, n.bounds.bottom().saturating_sub(1)),
                ] {
                    let title = Rect::new(
                        n.bounds.x.saturating_add(1),
                        y,
                        n.bounds.width.saturating_sub(2),
                        1,
                    );
                    paint_line(
                        frame,
                        text.clone(),
                        title,
                        n.clip,
                        n.style.paint,
                        Default::default(),
                        view,
                        self.depth,
                    );
                }
            }
            for (i, fragment) in n.fragments.iter().enumerate() {
                let y = n
                    .content
                    .y
                    .saturating_add(fragment.row.min(u16::MAX as usize) as u16);
                if y >= n.content.bottom() {
                    break;
                }
                let row = Rect::new(
                    n.content
                        .x
                        .saturating_add(fragment.indent as u16)
                        .saturating_add(n.align_offsets[i]),
                    y,
                    n.content
                        .width
                        .saturating_sub(fragment.indent as u16)
                        .saturating_sub(n.align_offsets[i]),
                    1,
                );
                paint_line(
                    frame,
                    n.lines.get(i).cloned().unwrap_or_default(),
                    row,
                    n.clip,
                    n.style.paint,
                    Default::default(),
                    view,
                    self.depth,
                );
            }
            if let Some((done, total)) = n.progress {
                let filled = if total == 0 {
                    0
                } else {
                    ((done.min(total) as f64 / total as f64) * f64::from(n.content.width)).round()
                        as u16
                };
                let bar = Rect {
                    height: n.content.height.min(1),
                    ..n.content
                };
                let visible = view.rect(bar.intersection(n.clip));
                for x in visible.x..visible.right() {
                    let column = i32::from(x) - view.x - i32::from(n.content.x);
                    frame.buffer_mut()[(x, visible.y)]
                        .set_symbol(if column < i32::from(filled) { "#" } else { " " })
                        .set_style(crate::ui::theme::paint_style(n.style.paint, self.depth));
                }
            }
            if !n.slot.is_empty() {
                let area = view.rect(n.content.intersection(n.clip));
                if !area.is_empty() {
                    paint(&n.slot, frame, area, n.style.paint);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct PaintView {
    clip: Rect,
    x: i32,
    y: i32,
}
impl PaintView {
    fn new(clip: Rect, x: i32, y: i32) -> Self {
        Self { clip, x, y }
    }
    fn rect(self, source: Rect) -> Rect {
        let left = (i32::from(source.x) + self.x).max(i32::from(self.clip.x));
        let top = (i32::from(source.y) + self.y).max(i32::from(self.clip.y));
        let right = (i32::from(source.right()) + self.x).min(i32::from(self.clip.right()));
        let bottom = (i32::from(source.bottom()) + self.y).min(i32::from(self.clip.bottom()));
        if right <= left || bottom <= top {
            return Rect::default();
        }
        Rect::new(
            left as u16,
            top as u16,
            (right - left) as u16,
            (bottom - top) as u16,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_line(
    frame: &mut Frame<'_>,
    text: impl Into<Line<'static>>,
    row: Rect,
    clip: Rect,
    style: PaintStyle,
    alignment: tuirealm::ratatui::layout::Alignment,
    view: PaintView,
    depth: crate::ui::theme::ColorDepth,
) {
    let mut text = text.into();
    let base = style.patch(text.style);
    text.style = crate::ui::theme::paint_style(base, depth);
    for span in &mut text.spans {
        span.style = crate::ui::theme::paint_style(base.patch(span.style), depth);
    }
    let visible = view.rect(row.intersection(clip));
    if visible.is_empty() || text.width() == 0 {
        return;
    }
    let padding = row
        .width
        .saturating_sub(text.width().min(u16::MAX as usize) as u16);
    let indent = match alignment {
        tuirealm::ratatui::layout::Alignment::Center => padding / 2,
        tuirealm::ratatui::layout::Alignment::Right => padding,
        _ => 0,
    };
    let origin = row.x.saturating_add(indent);
    let painted = view
        .rect(Rect::new(
            origin,
            row.y,
            row.width.saturating_sub(indent),
            1,
        ))
        .intersection(visible);
    if !painted.is_empty() {
        frame.render_widget(
            Paragraph::new(text)
                .style(crate::ui::theme::paint_style(style, depth))
                .scroll((
                    0,
                    (i32::from(painted.x) - i32::from(origin) - view.x).max(0) as u16,
                )),
            painted,
        );
    }
}

struct Built<'a> {
    node: &'a Node,
    data: &'a Presentation,
    namespace: String,
    item: Option<(&'a str, &'a str)>,
    id: NodeId,
    style: Computed,
    children: Vec<Built<'a>>,
    text: String,
    runs: Vec<TextRun>,
    prefix_chars: usize,
}
#[derive(Clone)]
struct Measure {
    width: f32,
    height: f32,
    text: String,
    wrap: bool,
    hanging_indent: u16,
    prefix_chars: usize,
}

/// UI-thread renderer. Its Taffy state never crosses the controller's thread move.
pub struct Renderer {
    bundle: LayoutBundle,
    depth: crate::ui::theme::ColorDepth,
    tree: TaffyTree<Measure>,
    cache: HashMap<String, CachedScene>,
    cache_clock: u64,
    image_regions: Vec<Rect>,
    #[cfg(test)]
    arrangements: usize,
    text_cache: HashMap<(String, u16), std::sync::Arc<Vec<Fragment>>>,
}
struct CachedScene {
    template: String,
    area: Rect,
    data: Presentation,
    scene: RenderedScene,
    natural_height: bool,
    natural_width: bool,
    last_used: u64,
}
impl Renderer {
    /// Construct locally after entering the UI thread.
    pub fn new(bundle: LayoutBundle) -> Self {
        Self {
            bundle,
            depth: Default::default(),
            tree: TaffyTree::new(),
            cache: HashMap::new(),
            cache_clock: 0,
            image_regions: Vec::new(),
            #[cfg(test)]
            arrangements: 0,
            text_cache: HashMap::new(),
        }
    }
    #[cfg(test)]
    pub(crate) fn arrangement_count(&self) -> usize {
        self.arrangements
    }
    /// Color depth changes paint, not measurements or controller state.
    pub fn set_color_depth(&mut self, depth: crate::ui::theme::ColorDepth) {
        if self.depth == depth {
            return;
        }
        self.depth = depth;
        for cached in self.cache.values_mut() {
            cached.scene.depth = depth;
        }
    }
    /// Resolve a specialized primitive's semantic style before it paints.
    pub fn paint_style(&self, style: PaintStyle) -> PaintStyle {
        crate::ui::theme::paint_style(style, self.depth)
    }
    /// Terminal palette for intrinsically painted editor/map contents.
    pub fn color_depth(&self) -> crate::ui::theme::ColorDepth {
        self.depth
    }
    /// Clear a canvas or overlay, installing the terminal's semantic defaults.
    pub fn clear(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(tuirealm::ratatui::widgets::Clear, area);
        frame
            .buffer_mut()
            .set_style(area, crate::ui::theme::canvas_style(self.depth));
    }
    /// Replace definitions atomically; controller state is external to this renderer.
    pub fn install(&mut self, bundle: LayoutBundle) {
        if self.bundle.revision != bundle.revision {
            self.cache.clear();
            self.text_cache.clear();
            self.tree.clear();
        }
        self.bundle = bundle;
    }
    /// Currently installed immutable definitions.
    pub fn bundle(&self) -> &LayoutBundle {
        &self.bundle
    }
    /// Cache immutable wrapped log rows by their text and frozen width.
    pub fn measured_text(&mut self, text: &str, width: u16) -> std::sync::Arc<Vec<Fragment>> {
        let key = (text.to_string(), width);
        if let Some(rows) = self.text_cache.get(&key) {
            return rows.clone();
        }
        // The log tail is capped at 2000 lines. Two widths retain useful work
        // across a resize without allowing unbounded historical measurements.
        if self.text_cache.len() >= 4096 {
            self.text_cache.clear();
        }
        let rows = std::sync::Arc::new(measure_text(text, width as usize, 0, 0, true, false));
        self.text_cache.insert(key, rows.clone());
        rows
    }
    /// Record protocol image operations alongside the frame's painted geometry.
    pub fn record_image_regions(&mut self, regions: &[Rect]) {
        self.image_regions = regions.to_vec();
    }
    /// Latest arranged templates, including matched declarations and bounds.
    pub fn inspect(&self) -> String {
        let mut names = self.cache.keys().collect::<Vec<_>>();
        names.sort();
        let mut rows = names
            .into_iter()
            .map(|name| format!("{name}\n{}", self.cache[name].scene.inspect().join("\n")))
            .collect::<Vec<_>>();
        rows.extend(
            self.image_regions
                .iter()
                .map(|area| format!("image {area:?}")),
        );
        rows.join("\n")
    }
    /// Expand, style, and allocate a template in terminal cells.
    pub fn arrange(
        &mut self,
        template: &str,
        area: Rect,
        data: &Presentation,
    ) -> Result<RenderedScene, Diagnostic> {
        self.arrange_instance(template, template, area, data)
    }
    /// Measure scrollable content at a frozen width, independently of its viewport.
    pub fn measure_content(
        &mut self,
        template: &str,
        key: &str,
        width: u16,
        data: &Presentation,
    ) -> Result<RenderedScene, Diagnostic> {
        self.arrange_instance_mode(
            template,
            key,
            Rect::new(0, 0, width, u16::MAX),
            data,
            true,
            false,
        )
    }
    /// Measure unwrapped intrinsic content, including authored chrome and spacing.
    pub fn intrinsic_width(
        &mut self,
        template: &str,
        data: &Presentation,
    ) -> Result<u16, Diagnostic> {
        self.arrange_instance_mode(
            template,
            &format!("intrinsic:{template}"),
            Rect::new(0, 0, u16::MAX, u16::MAX),
            data,
            true,
            true,
        )
        .map(|scene| scene.width)
    }
    /// Fit image pixels inside authored attachment chrome and its whole-frame cap.
    pub(crate) fn attachment(
        &mut self,
        key: &str,
        timestamp: &str,
        available: tuirealm::ratatui::layout::Size,
        image: &image::DynamicImage,
        font: ratatui_image::FontSize,
    ) -> Option<RenderedScene> {
        let area = Rect::new(0, 0, available.width, available.height);
        let gutter = timestamp.width().saturating_add(1).min(u16::MAX as usize) as u16;
        let probe = Presentation::default()
            .slot("timestamp-gutter", gutter, 0)
            .slot("image", available.width, available.height);
        let measured = self
            .arrange_instance(
                "chat-attachment",
                &format!("attachment-measure:{key}"),
                area,
                &probe,
            )
            .ok()?;
        let mut interior = measured.slot("image");
        interior.height = interior
            .height
            .saturating_sub(measured.occupied_height().saturating_sub(available.height));
        if interior.is_empty() {
            return None;
        }
        let fitted = ratatui_image::Resize::Fit(None).size_for(
            image,
            font,
            tuirealm::ratatui::layout::Size::new(interior.width, interior.height),
        );
        if fitted.width == 0 || fitted.height == 0 {
            return None;
        }
        let data = Presentation::default()
            .slot("timestamp-gutter", gutter, 0)
            .slot("image", fitted.width, fitted.height);
        let mut scene = self
            .arrange_instance("chat-attachment", &format!("attachment:{key}"), area, &data)
            .ok()?;
        if scene.occupied_height() > available.height {
            return None;
        }
        let extent = Rect::new(0, 0, area.width, scene.occupied_height());
        for node in &mut scene.nodes {
            node.clip = node.clip.intersection(extent);
        }
        (!scene.slot("image").is_empty()).then_some(scene)
    }
    pub(super) fn arrange_named(
        &mut self,
        template: &str,
        key: &str,
        area: Rect,
        data: &Presentation,
    ) -> Result<RenderedScene, Diagnostic> {
        self.arrange_instance(template, key, area, data)
    }
    fn arrange_instance(
        &mut self,
        template: &str,
        key: &str,
        area: Rect,
        data: &Presentation,
    ) -> Result<RenderedScene, Diagnostic> {
        self.arrange_instance_mode(template, key, area, data, false, false)
    }
    fn arrange_instance_mode(
        &mut self,
        template: &str,
        key: &str,
        area: Rect,
        data: &Presentation,
        natural_height: bool,
        natural_width: bool,
    ) -> Result<RenderedScene, Diagnostic> {
        self.cache_clock = self.cache_clock.wrapping_add(1);
        if self.cache_clock == 0 {
            self.cache.clear();
        }
        if let Some(cached) = self.cache.get_mut(key)
            && cached.area == area
            && cached.template == template
            && cached.natural_height == natural_height
            && cached.natural_width == natural_width
        {
            if cached.data == *data {
                cached.last_used = self.cache_clock;
                return Ok(cached.scene.clone());
            }
            if cached.data.same_measurement(data) {
                validate_rich(data)?;
                refresh_paint(&mut cached.scene, data);
                cached.data = data.clone();
                cached.last_used = self.cache_clock;
                return Ok(cached.scene.clone());
            }
        }
        validate_rich(data)?;
        let root = self.bundle.templates.get(template).ok_or_else(|| {
            Diagnostic::at(
                std::path::Path::new("<renderer>"),
                "",
                0,
                "unknown template",
            )
        })?;
        #[cfg(test)]
        {
            self.arrangements += 1;
        }
        self.tree.clear();
        let mut inherited = Computed {
            paint: data.inherited_style,
            hanging_indent: data.inherited_indent,
            ..Computed::default()
        };
        inherited.variables.extend(data.color_variables.clone());
        let built = build(
            &mut self.tree,
            &self.bundle,
            root,
            data,
            &mut Vec::new(),
            &inherited,
            &BuildScope {
                viewport: area,
                namespace: String::new(),
                item: None,
            },
        )?;
        let available = Size {
            width: if natural_width {
                AvailableSpace::MaxContent
            } else {
                AvailableSpace::Definite(area.width as f32)
            },
            height: if natural_height {
                AvailableSpace::MaxContent
            } else {
                AvailableSpace::Definite(area.height as f32)
            },
        };
        // A real containing box preserves authored root dimensions and margins.
        // Stretching the single grid item supplies the viewport only for auto
        // dimensions; natural-height rows keep their complete outer extent.
        let viewport = self
            .tree
            .new_with_children(
                taffy::Style {
                    display: Display::Grid,
                    size: Size {
                        width: if natural_width {
                            Dimension::Auto
                        } else {
                            Dimension::Length(area.width as f32)
                        },
                        height: if natural_height {
                            Dimension::Auto
                        } else {
                            Dimension::Length(area.height as f32)
                        },
                    },
                    grid_template_columns: vec![fr(1.0_f32)],
                    grid_template_rows: vec![fr(1.0_f32)],
                    align_items: Some(AlignItems::Stretch),
                    justify_items: Some(AlignItems::Stretch),
                    ..Default::default()
                },
                &[built.id],
            )
            .map_err(internal)?;
        // Pass one depends only on unwrapped width, never on wrapped height.
        self.tree
            .compute_layout_with_measure(viewport, available, |known, _, _, context, _| {
                let Some(m) = context else {
                    return Size::ZERO;
                };
                Size {
                    width: known.width.unwrap_or(m.width),
                    height: known.height.unwrap_or(m.height),
                }
            })
            .map_err(internal)?;
        freeze_widths(&mut self.tree, &built)?;
        self.tree
            .compute_layout_with_measure(viewport, available, |known, available, _, context, _| {
                let Some(m) = context else {
                    return Size::ZERO;
                };
                let width = known.width.unwrap_or(match available.width {
                    AvailableSpace::Definite(w) => w,
                    _ => m.width,
                });
                let height = if m.text.is_empty() || !m.wrap {
                    m.height
                } else {
                    super::text::measure_flow(
                        &m.text,
                        width.max(0.0) as usize,
                        m.prefix_chars,
                        usize::from(m.hanging_indent),
                        true,
                        false,
                    )
                    .last()
                    .map_or(0, |fragment| fragment.row + 1) as f32
                };
                Size {
                    width,
                    height: known.height.unwrap_or(height),
                }
            })
            .map_err(internal)?;
        let mut scene = RenderedScene {
            depth: self.depth,
            width: self
                .tree
                .layout(viewport)
                .map_err(internal)?
                .size
                .width
                .round()
                .clamp(0.0, u16::MAX as f32) as u16,
            height: self
                .tree
                .layout(viewport)
                .map_err(internal)?
                .size
                .height
                .round()
                .clamp(0.0, u16::MAX as f32) as u16,
            ..Default::default()
        };
        collect(
            &self.tree,
            &built,
            (area.x as f32, area.y as f32),
            area,
            area,
            false,
            &mut scene,
        )?;
        scene.paint_order = paint_order(&scene.nodes, 0..scene.nodes.len());
        if self.cache.len() >= 2048
            && !self.cache.contains_key(key)
            && let Some(oldest) = self
                .cache
                .iter()
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(key, _)| key.clone())
        {
            self.cache.remove(&oldest);
        }
        self.cache.insert(
            key.into(),
            CachedScene {
                template: template.into(),
                area,
                data: data.clone(),
                scene: scene.clone(),
                natural_height,
                natural_width,
                last_used: self.cache_clock,
            },
        );
        Ok(scene)
    }
    /// Paint the visible slice of a measured semantic collection. Only rows
    /// around the viewport are instantiated, with cached width-first measurements.
    pub fn paint_rows(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        template: &str,
        rows: &[PresentedRow],
        selected: Option<usize>,
        style: PaintStyle,
    ) -> Result<(), Diagnostic> {
        self.paint_collection(frame, area, template, rows, selected, selected, style)
            .map(|_| ())
    }
    /// Paint collection rows and publish their interaction bounds in the same pass.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_collection(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        template: &str,
        rows: &[PresentedRow],
        selected: Option<usize>,
        center: Option<usize>,
        style: PaintStyle,
    ) -> Result<RenderedCollection, Diagnostic> {
        let mut rendered = RenderedCollection::default();
        let mut keys = std::collections::BTreeSet::new();
        if rows.iter().any(|row| !keys.insert(&row.key)) {
            return Err(internal("duplicate semantic collection key"));
        }
        let mut positions = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            positions.push(Some(index));
            if row.gap_after {
                positions.push(None);
            }
        }
        if area.is_empty() || positions.is_empty() {
            return Ok(rendered);
        }
        let mut measured: BTreeMap<usize, (u16, Option<RenderedScene>)> = BTreeMap::new();
        let measure = |renderer: &mut Self,
                       position: usize,
                       measured: &mut BTreeMap<usize, (u16, Option<RenderedScene>)>|
         -> Result<u16, Diagnostic> {
            if let Some((height, _)) = measured.get(&position) {
                return Ok(*height);
            }
            let Some(index) = positions[position] else {
                measured.insert(position, (1, None));
                return Ok(1);
            };
            let row = &rows[index];
            let mut data = row.data.clone();
            data.inherited_style = style;
            if selected == Some(index) {
                data.root_states.push("selected".into());
            }
            let scene = renderer.arrange_instance_mode(
                template,
                &format!("row:{template}/{}", row.key),
                Rect::new(0, 0, area.width, u16::MAX),
                &data,
                true,
                false,
            )?;
            let height = scene.height();
            measured.insert(position, (height, Some(scene)));
            Ok(height)
        };
        let target = center
            .and_then(|s| positions.iter().position(|p| *p == Some(s)))
            .unwrap_or(0);
        let target_height = usize::from(measure(self, target, &mut measured)?);
        let half = if center.is_some() && target_height < area.height as usize {
            area.height as usize / 2
        } else {
            0
        };
        let mut top = target;
        let mut preceding = 0;
        while top > 0 && preceding < half {
            top -= 1;
            preceding += usize::from(measure(self, top, &mut measured)?);
        }
        let mut clipped_rows = preceding.saturating_sub(half);
        let mut remaining = 0;
        for index in top..positions.len() {
            remaining += usize::from(measure(self, index, &mut measured)?);
            if remaining >= area.height as usize + clipped_rows {
                break;
            }
        }
        // Fill the viewport near the tail, shifting backwards by measured cells.
        if remaining < area.height as usize + clipped_rows {
            let mut missing = area.height as usize + clipped_rows - remaining;
            let recovered = missing.min(clipped_rows);
            clipped_rows -= recovered;
            missing -= recovered;
            while top > 0 && missing > 0 {
                top -= 1;
                let height = usize::from(measure(self, top, &mut measured)?);
                clipped_rows = height.saturating_sub(missing);
                missing = missing.saturating_sub(height);
            }
        }
        let mut offset = -(clipped_rows as i32);
        for (position, index) in positions.iter().enumerate().skip(top) {
            if offset >= i32::from(area.height) {
                break;
            }
            let height = measure(self, position, &mut measured)?;
            if let Some(index) = *index
                && let Some((_, Some(scene))) = measured.get(&position)
            {
                scene.paint_scrolled(frame, area, offset);
                if let Some(root) = scene.nodes.first() {
                    let bounds =
                        PaintView::new(area, i32::from(area.x), i32::from(area.y) + offset)
                            .rect(root.bounds.intersection(root.clip));
                    if !bounds.is_empty() {
                        rendered.rows.push((bounds, index));
                        if selected == Some(index) {
                            frame
                                .buffer_mut()
                                .set_style(bounds, crate::ui::theme::highlight_style());
                        }
                    }
                }
            }
            offset += i32::from(height);
        }
        Ok(rendered)
    }
}
fn paint_order(nodes: &[Arranged], range: std::ops::Range<usize>) -> Vec<usize> {
    let mut order = Vec::new();
    let mut overlays = Vec::new();
    let mut index = range.start;
    while index < range.end {
        if nodes[index].overlay_root {
            overlays.push(index);
            index = nodes[index].subtree_end;
        } else {
            order.push(index);
            index += 1;
        }
    }
    for index in overlays {
        order.push(index);
        order.extend(paint_order(nodes, index + 1..nodes[index].subtree_end));
    }
    order
}
pub(super) fn internal(e: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::at(
        std::path::Path::new("<renderer>"),
        "",
        0,
        format!("layout allocation: {e}"),
    )
}
fn validate_rich(data: &Presentation) -> Result<(), Diagnostic> {
    for items in data.lists.values() {
        let mut keys = std::collections::BTreeSet::new();
        if items.len() > 1024
            || items
                .iter()
                .any(|item| item.key.is_empty() || !keys.insert(&item.key))
        {
            return Err(internal(
                "repeated items require nonempty unique keys and at most 1024 items",
            ));
        }
        for item in items {
            validate_rich(&item.data)?;
        }
    }
    for span in data.rich.values().flatten() {
        let length = span.text.chars().count();
        if span.marks.iter().any(|(index, marks)| {
            *index >= length
                || marks
                    .chars()
                    .any(|mark| !(('\u{0300}'..='\u{036f}').contains(&mark)))
        }) {
            return Err(internal(
                "rich combining marks require valid source indices and U+0300–U+036F diacritics",
            ));
        }
    }
    Ok(())
}
fn fragment_line(fragment: &Fragment, runs: &[TextRun], style: PaintStyle) -> Line<'static> {
    let chars: Vec<_> = fragment.text.chars().collect();
    let mut spans = Vec::new();
    let mut cursor = fragment.source.start;
    for run in runs {
        let start = run.range.start.max(fragment.source.start);
        let end = run.range.end.min(fragment.source.end);
        if start >= end {
            continue;
        }
        if start > cursor {
            let before: String = chars
                [cursor - fragment.source.start..start - fragment.source.start]
                .iter()
                .collect();
            spans.push(Span::styled(before, style));
        }
        let mut painted = String::new();
        for (index, ch) in chars[start - fragment.source.start..end - fragment.source.start]
            .iter()
            .enumerate()
        {
            painted.push(*ch);
            if let Some(marks) = run
                .marks
                .get(&(run.source + start - run.range.start + index))
            {
                painted.push_str(marks);
            }
        }
        spans.push(Span::styled(painted, run.style));
        cursor = end;
    }
    let tail: String = chars[cursor - fragment.source.start..].iter().collect();
    if !tail.is_empty() {
        spans.push(Span::styled(tail, style));
    }
    Line::from(spans)
}
fn refresh_paint(scene: &mut RenderedScene, data: &Presentation) {
    let fields: BTreeMap<_, _> = data
        .rich
        .iter()
        .map(|(name, spans)| {
            let mut chars = Vec::new();
            let mut marks = BTreeMap::new();
            for span in spans {
                marks.extend(
                    span.marks
                        .iter()
                        .map(|(index, marks)| (chars.len() + index, marks.clone())),
                );
                chars.extend(span.text.chars());
            }
            (name, (chars, marks))
        })
        .collect();
    for node in &mut scene.nodes {
        for run in &mut node.runs {
            if let Some((_, marks)) = fields.get(&run.binding) {
                run.marks = marks
                    .range(run.source..run.source + run.range.len())
                    .map(|(index, marks)| (*index, marks.clone()))
                    .collect();
            }
        }
        for fragment in &mut node.fragments {
            let mut chars: Vec<_> = fragment.text.chars().collect();
            for run in &node.runs {
                let Some((source, _)) = fields.get(&run.binding) else {
                    continue;
                };
                let start = run.range.start.max(fragment.source.start);
                let end = run.range.end.min(fragment.source.end);
                if start >= end {
                    continue;
                }
                let from = run.source + start - run.range.start;
                chars[start - fragment.source.start..end - fragment.source.start]
                    .copy_from_slice(&source[from..from + end - start]);
            }
            fragment.text = chars.into_iter().collect();
        }
        node.lines = node
            .fragments
            .iter()
            .map(|fragment| fragment_line(fragment, &node.runs, node.style.paint))
            .collect();
    }
}

// A full modal uses viewport-relative dimensions, floors percentage cells, and
// yields its minimum size to the viewport. Recovery popups retain their inset.
fn constrain_modal(style: &mut taffy::Style, viewport: Rect) {
    let axis = |size: &mut Dimension, min: &mut Dimension, max: &mut Dimension, available: u16| {
        let available = f32::from(available);
        let resolve = |value: Dimension| match value {
            Dimension::Length(value) => Some(value),
            Dimension::Percent(value) => Some((value * available).floor()),
            Dimension::Auto => None,
        };
        let minimum = resolve(*min).unwrap_or(0.0).min(available);
        let maximum = resolve(*max)
            .unwrap_or(available)
            .max(minimum)
            .min(available);
        *size = resolve(*size).map_or(Dimension::Auto, |value| {
            Dimension::Length(value.clamp(minimum, maximum))
        });
        *min = Dimension::Length(minimum);
        *max = Dimension::Length(maximum);
    };
    axis(
        &mut style.size.width,
        &mut style.min_size.width,
        &mut style.max_size.width,
        viewport.width,
    );
    axis(
        &mut style.size.height,
        &mut style.min_size.height,
        &mut style.max_size.height,
        viewport.height,
    );
    // Auto modal dimensions measure their content; opposite definite insets
    // would otherwise stretch an absolutely positioned Taffy box to the viewport.
    if style.size.width == Dimension::Auto {
        style.inset.right = LengthPercentageAuto::Auto;
    }
    if style.size.height == Dimension::Auto {
        style.inset.bottom = LengthPercentageAuto::Auto;
    }
}

struct BuildScope<'a> {
    viewport: Rect,
    namespace: String,
    item: Option<(&'a str, &'a str)>,
}
fn build<'a>(
    tree: &mut TaffyTree<Measure>,
    bundle: &LayoutBundle,
    node: &'a Node,
    data: &'a Presentation,
    path: &mut Vec<(&'a Node, Vec<&'a str>)>,
    parent: &Computed,
    scope: &BuildScope<'a>,
) -> Result<Built<'a>, Diagnostic> {
    let mut states: Vec<&str> = data
        .states
        .get(node.attr("id"))
        .map(|s| s.iter().map(String::as_str).collect())
        .unwrap_or_default();
    if path.is_empty() || scope.item.is_some() {
        states.extend(data.root_states.iter().map(String::as_str));
    }
    path.push((node, states));
    let mut style = resolve(bundle, path, parent)?;
    if node.tag == "overlay" && node.attr("placement") == "modal" {
        constrain_modal(&mut style.layout, scope.viewport);
    }
    if let Some(semantic) = data.component_styles.get(node.attr("id")) {
        style.paint = crate::ui::theme::with_authored_style(*semantic, style.paint);
    }
    if let Some(share) = data.shares.get(node.attr("id")) {
        style.layout.flex_basis = Dimension::Percent(f32::from(*share) / 10_000.0);
        // Percentage shares divide the available track after authored gaps.
        style.layout.flex_shrink = 1.0;
    }
    if let Some(semantic) = data.styles.get(node.attr("bind")) {
        style.paint = crate::ui::theme::with_authored_style(*semantic, style.paint);
    }
    if !node.attr("if").is_empty() && !data.bools.get(node.attr("if")).copied().unwrap_or(false) {
        style.layout.display = Display::None;
    }
    let mut children = Vec::new();
    if node.tag == "repeat" {
        for row in data.lists.get(node.attr("bind")).into_iter().flatten() {
            let item_scope = BuildScope {
                viewport: scope.viewport,
                namespace: format!("{}{}/{:?}/", scope.namespace, node.attr("bind"), row.key),
                item: Some((node.attr("bind"), row.key.as_str())),
            };
            children.push(build(
                tree,
                bundle,
                &node.children[0],
                &row.data,
                path,
                &style,
                &item_scope,
            )?);
        }
    } else {
        let child_scope = BuildScope {
            viewport: scope.viewport,
            namespace: scope.namespace.clone(),
            item: None,
        };
        for child in &node.children {
            children.push(build(
                tree,
                bundle,
                child,
                data,
                path,
                &style,
                &child_scope,
            )?);
        }
    }
    path.pop();
    let mut text = data
        .texts
        .get(node.attr("bind"))
        .cloned()
        .unwrap_or_default();
    let mut runs = Vec::new();
    let mut prefix_chars = 0;
    if node.tag == "rich" {
        text.clear();
        let mut start = 0;
        for span in data.rich.get(node.attr("bind")).into_iter().flatten() {
            let end = start + span.text.chars().count();
            runs.push(TextRun {
                range: start..end,
                source: start,
                binding: node.attr("bind").into(),
                node: format!("{}{}", scope.namespace, node.attr("id")),
                style: crate::ui::theme::with_authored_style(span.style, style.paint),
                action: span.action.clone(),
                marks: span
                    .marks
                    .iter()
                    .map(|(index, marks)| (start + index, marks.clone()))
                    .collect(),
            });
            text.push_str(&span.text);
            start = end;
        }
    } else if matches!(node.tag.as_str(), "flow" | "prefix") {
        text.clear();
        let separator = node.attrs.get("separator").map_or(" ", String::as_str);
        let mut offset = 0;
        for child in children
            .iter()
            .filter(|child| child.style.layout.display != Display::None)
        {
            if child.text.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push_str(separator);
                offset += separator.chars().count();
            }
            for run in &child.runs {
                let mut run = run.clone();
                run.range = run.range.start + offset..run.range.end + offset;
                runs.push(run);
            }
            text.push_str(&child.text);
            offset += child.text.chars().count();
            if child.node.tag == "prefix" {
                prefix_chars = offset;
            }
        }
        if prefix_chars > 0 && prefix_chars < offset {
            prefix_chars += separator.chars().count();
        }
        // Inline children share the flow's measured lines, not separate boxes.
        children.clear();
    } else if !text.is_empty() {
        runs.push(TextRun {
            range: 0..text.chars().count(),
            source: 0,
            binding: node.attr("bind").into(),
            node: format!("{}{}", scope.namespace, node.attr("id")),
            style: style.paint,
            action: None,
            marks: BTreeMap::new(),
        });
    }
    let (width, height) = data.slots.get(node.attr("name")).copied().unwrap_or((0, 0));
    let measure = Measure {
        width: if let Some(width) = data.intrinsic_widths.get(node.attr("bind")) {
            f32::from(*width)
        } else if text.is_empty() {
            width as f32
        } else {
            text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0) as f32
        },
        height: if node.tag == "progress" {
            1.0
        } else if text.is_empty() {
            height as f32
        } else {
            text.lines().count().max(1) as f32
        },
        text: text.clone(),
        wrap: style.wrap,
        hanging_indent: style.hanging_indent,
        prefix_chars,
    };
    if tree.total_node_count() >= 65_536 {
        return Err(Diagnostic {
            message: "expanded presentation exceeds 65536 layout nodes".into(),
            ..node.location.clone()
        });
    }
    let id = if children.is_empty() {
        tree.new_leaf_with_context(style.layout.clone(), measure)
    } else {
        tree.new_with_children(
            style.layout.clone(),
            &children.iter().map(|c| c.id).collect::<Vec<_>>(),
        )
    }
    .map_err(internal)?;
    Ok(Built {
        node,
        data,
        namespace: scope.namespace.clone(),
        item: scope.item,
        id,
        style,
        children,
        text,
        runs,
        prefix_chars,
    })
}
fn freeze_widths(tree: &mut TaffyTree<Measure>, built: &Built<'_>) -> Result<(), Diagnostic> {
    let mut width = tree.layout(built.id).map_err(internal)?.size.width;
    let mut style = tree.style(built.id).map_err(internal)?.clone();
    if built.style.floor_size {
        let size = tree.unrounded_layout(built.id).size;
        // Percentage parsing and allocation can land a couple of f32 ULPs
        // below an exact cell boundary (e.g. two thirds of 30).
        let floor = |value: f32| value.next_up().next_up().floor();
        width = floor(size.width);
        if style.size.height != Dimension::Auto {
            let height = Dimension::Length(floor(size.height));
            style.size.height = height;
            style.min_size.height = height;
            style.max_size.height = height;
        }
    }
    style.size.width = Dimension::Length(width);
    style.min_size.width = Dimension::Length(width);
    style.max_size.width = Dimension::Length(width);
    tree.set_style(built.id, style).map_err(internal)?;
    for child in &built.children {
        freeze_widths(tree, child)?;
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn collect(
    tree: &TaffyTree<Measure>,
    built: &Built<'_>,
    origin: (f32, f32),
    clip: Rect,
    containing: Rect,
    in_overlay: bool,
    scene: &mut RenderedScene,
) -> Result<(), Diagnostic> {
    let data = built.data;
    if built.style.layout.display == Display::None {
        return Ok(());
    }
    // Centered overlays reserve a one-cell safety inset even when their border
    // minimum exceeds the available track (including a zero-sized interior).
    let clip = if built.node.tag == "overlay" && built.node.attr("placement") == "after" {
        // Attached popups escape the anchor box, but stay inside the entry viewport.
        scene
            .nodes
            .first()
            .map_or(clip, |root| root.content.intersection(root.clip))
    } else if built.node.tag == "overlay" && built.node.attr("placement") == "before" {
        scene
            .nodes
            .first()
            .map_or(clip, |root| root.bounds.intersection(root.clip))
    } else if built.node.tag == "overlay" && built.node.attr("placement") == "center" {
        clip.intersection(containing.inner(tuirealm::ratatui::layout::Margin::new(1, 1)))
    } else {
        clip
    };
    if clip.is_empty() {
        return Ok(());
    }
    let layout = tree.layout(built.id).map_err(internal)?;
    let modal = built.node.tag == "overlay" && built.node.attr("placement") == "modal";
    let containing = if modal {
        scene.nodes.first().map_or(containing, |root| root.bounds)
    } else {
        containing
    };
    let centered =
        built.node.tag == "overlay" && matches!(built.node.attr("placement"), "center" | "modal");
    let centered_offset = |available: u16, extent: f32| {
        let offset = (f32::from(available) - extent).max(0.0) / 2.0;
        if modal { offset.floor() } else { offset }
    };
    let x = if centered {
        f32::from(containing.x) + centered_offset(containing.width, layout.size.width)
    } else {
        origin.0 + layout.location.x
    };
    let y = if centered {
        f32::from(containing.y) + centered_offset(containing.height, layout.size.height)
    } else {
        origin.1 + layout.location.y
    };
    let cell = |n: f32| n.round().clamp(0.0, u16::MAX as f32) as u16;
    let bounds = Rect::new(
        cell(x),
        cell(y),
        cell(x + layout.size.width).saturating_sub(cell(x)),
        cell(y + layout.size.height).saturating_sub(cell(y)),
    );
    let left = layout.border.left + layout.padding.left;
    let top = layout.border.top + layout.padding.top;
    let content = Rect::new(
        cell(x + left),
        cell(y + top),
        cell(layout.size.width - left - layout.border.right - layout.padding.right),
        cell(layout.size.height - top - layout.border.bottom - layout.padding.bottom),
    );
    if let Some((binding, key)) = built.item {
        scene.items.push(RepeatedItem {
            binding: binding.into(),
            key: key.into(),
            instance: built.namespace.clone(),
            bounds,
            clip,
        });
    }
    let text = if data
        .preserve_end
        .iter()
        .any(|name| name == built.node.attr("bind"))
    {
        crate::ui::widgets::table::truncate_display_start(&built.text, content.width as usize).0
    } else {
        built.text.clone()
    };
    let fragments = super::text::measure_flow(
        &text,
        content.width as usize,
        built.prefix_chars,
        usize::from(built.style.hanging_indent),
        built.style.wrap,
        built.style.ellipsis,
    );
    let mut line_widths = BTreeMap::<usize, usize>::new();
    for fragment in &fragments {
        let width = line_widths.entry(fragment.row).or_default();
        *width = (*width).max(fragment.indent + fragment.width);
    }
    let align_offsets: Vec<u16> = fragments
        .iter()
        .map(|fragment| {
            let spare = content
                .width
                .saturating_sub(line_widths[&fragment.row].min(u16::MAX as usize) as u16);
            match built.style.align {
                tuirealm::ratatui::layout::Alignment::Center => spare / 2,
                tuirealm::ratatui::layout::Alignment::Right => spare,
                _ => 0,
            }
        })
        .collect();
    let mut runs = built.runs.clone();
    if text != built.text {
        // A filename-preserving ellipsis is decoration, not a source character.
        let suffix = built
            .text
            .chars()
            .count()
            .saturating_sub(text.chars().count().saturating_sub(1));
        runs.retain_mut(|run| {
            let start = run.range.start.max(suffix);
            if start >= run.range.end {
                return false;
            }
            run.source += start - run.range.start;
            run.range = start - suffix + 1..run.range.end - suffix + 1;
            true
        });
    }
    for (index, fragment) in fragments.iter().enumerate() {
        let chars: Vec<_> = fragment.text.chars().collect();
        for run in &runs {
            let start = run.range.start.max(fragment.source.start);
            let end = run.range.end.min(fragment.source.end);
            if start >= end {
                continue;
            }
            let text: String = chars[start - fragment.source.start..end - fragment.source.start]
                .iter()
                .collect();
            let prefix: String = chars[..start - fragment.source.start].iter().collect();
            let bounds = Rect::new(
                content
                    .x
                    .saturating_add(align_offsets[index])
                    .saturating_add(fragment.indent as u16)
                    .saturating_add(prefix.width() as u16),
                content
                    .y
                    .saturating_add(fragment.row.min(u16::MAX as usize) as u16),
                text.width().min(u16::MAX as usize) as u16,
                1,
            );
            if !bounds.intersection(content).intersection(clip).is_empty() {
                scene.text_regions.push(TextRegion {
                    node: run.node.clone(),
                    binding: run.binding.clone(),
                    source: run.source + start - run.range.start
                        ..run.source + end - run.range.start,
                    bounds,
                    clip: content.intersection(clip),
                    action: run.action.clone(),
                });
            }
        }
    }
    let lines = fragments
        .iter()
        .map(|fragment| fragment_line(fragment, &runs, built.style.paint))
        .collect();
    let node_index = scene.nodes.len();
    scene.nodes.push(Arranged {
        subtree_end: node_index + 1,
        progress: if built.node.tag == "progress" {
            data.progress.get(built.node.attr("bind")).copied()
        } else {
            None
        },
        overlay: in_overlay || built.node.tag == "overlay",
        overlay_root: built.node.tag == "overlay",
        id: if built.node.attr("id").is_empty() {
            String::new()
        } else {
            format!("{}{}", built.namespace, built.node.attr("id"))
        },
        slot: if built.node.tag == "slot" {
            built.node.attr("name").into()
        } else {
            String::new()
        },
        bounds,
        margin_bottom: cell(layout.margin.bottom),
        content,
        clip,
        style: built.style.clone(),
        title: data
            .texts
            .get(built.node.attr("title"))
            .cloned()
            .unwrap_or_default(),
        title_bottom: data
            .texts
            .get(built.node.attr("title-bottom"))
            .cloned()
            .unwrap_or_default(),
        fragments,
        lines,
        align_offsets,
        runs,
    });
    let mut split_children = Vec::new();
    for child in &built.children {
        let index = scene.nodes.len();
        collect(
            tree,
            child,
            (x, y),
            clip.intersection(content),
            content,
            in_overlay || built.node.tag == "overlay",
            scene,
        )?;
        if let Some(node) = scene.nodes.get(index)
            && !node.bounds.intersection(node.clip).is_empty()
        {
            split_children.push((child.node.attr("id").to_string(), node.bounds));
        }
    }
    scene.nodes[node_index].subtree_end = scene.nodes.len();
    if built.node.attr("resizable") == "true" && split_children.len() >= 2 {
        let horizontal = built.style.layout.flex_direction == FlexDirection::Row;
        let origin = if horizontal { content.x } else { content.y };
        let extent = if horizontal {
            content.width
        } else {
            content.height
        };
        let total: u32 = split_children
            .iter()
            .map(|(_, r)| u32::from(if horizontal { r.width } else { r.height }))
            .sum();
        let saved: Option<Vec<_>> = split_children
            .iter()
            .map(|(id, _)| data.shares.get(id).map(|n| (id.clone(), *n)))
            .collect();
        let children = saved
            .filter(|shares| shares.iter().map(|(_, n)| u32::from(*n)).sum::<u32>() == 10_000)
            .unwrap_or_else(|| {
                let mut used = 0;
                let mut assigned = 0;
                split_children
                    .iter()
                    .map(|(id, r)| {
                        used += u32::from(if horizontal { r.width } else { r.height });
                        let edge = used * 10_000 / total.max(1);
                        let share = edge - assigned;
                        assigned = edge;
                        (id.clone(), share as u16)
                    })
                    .collect()
            });
        // Parent handles precede nested handles at intersecting boundaries.
        let insertion = 0;
        for (before, pair) in split_children.windows(2).enumerate() {
            let a = pair[0].1;
            let b = pair[1].1;
            let handle = if horizontal {
                Rect::new(
                    a.right().saturating_sub(1),
                    a.y.max(b.y),
                    b.x.saturating_sub(a.right()).saturating_add(2),
                    a.bottom().min(b.bottom()).saturating_sub(a.y.max(b.y)),
                )
            } else {
                Rect::new(
                    a.x.max(b.x),
                    a.bottom().saturating_sub(1),
                    a.right().min(b.right()).saturating_sub(a.x.max(b.x)),
                    b.y.saturating_sub(a.bottom()).saturating_add(2),
                )
            }
            .intersection(content)
            .intersection(clip);
            scene.splits.insert(
                insertion,
                SplitRegion {
                    id: built.node.attr("id").into(),
                    handle,
                    horizontal,
                    origin,
                    extent,
                    before,
                    children: children.clone(),
                },
            );
        }
    }
    Ok(())
}
