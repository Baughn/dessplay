use super::css::{Declaration, Rule};
use quick_xml::{Reader, events::Event};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MAX_FILE: u64 = 1024 * 1024;
const MAX_NODES: usize = 4096;
const DEFAULTS: &[(&str, &str)] = &[
    (
        "templates/chrome.xml",
        include_str!("assets/templates/chrome.xml"),
    ),
    (
        "templates/text.xml",
        include_str!("assets/templates/text.xml"),
    ),
    (
        "templates/rogue.xml",
        include_str!("assets/templates/rogue.xml"),
    ),
    (
        "templates/collections.xml",
        include_str!("assets/templates/collections.xml"),
    ),
    (
        "templates/chat.xml",
        include_str!("assets/templates/chat.xml"),
    ),
    (
        "templates/app.xml",
        include_str!("assets/templates/app.xml"),
    ),
    (
        "templates/form.xml",
        include_str!("assets/templates/form.xml"),
    ),
    (
        "templates/form-row.xml",
        include_str!("assets/templates/form-row.xml"),
    ),
    (
        "templates/log.xml",
        include_str!("assets/templates/log.xml"),
    ),
    (
        "templates/layout-tools.xml",
        include_str!("assets/templates/layout-tools.xml"),
    ),
    ("style.css", include_str!("assets/style.css")),
];

/// An actionable compiler error with one-based source coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// File that failed compilation.
    pub file: PathBuf,
    /// One-based line.
    pub line: usize,
    /// One-based character column.
    pub column: usize,
    /// Explanation, without user message contents.
    pub message: String,
}
impl Diagnostic {
    pub(super) fn at(file: &Path, source: &str, offset: usize, message: impl Into<String>) -> Self {
        let mut offset = offset.min(source.len());
        while !source.is_char_boundary(offset) {
            offset -= 1;
        }
        let before = &source[..offset];
        Self {
            file: file.into(),
            line: before.bytes().filter(|b| *b == b'\n').count() + 1,
            column: before.rsplit('\n').next().unwrap_or("").chars().count() + 1,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file.display(),
            self.line,
            self.column,
            self.message
        )
    }
}
impl std::error::Error for Diagnostic {}

/// Binding types accepted by a template's presentation contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindingType {
    Text,
    Rich,
    Progress,
    Bool,
    Slot,
    List(&'static str),
}

/// Application-owned binding contract, versioned separately from the binary.
#[derive(Clone, Debug)]
pub struct TemplateSchema {
    pub(super) templates: BTreeMap<String, BTreeMap<String, BindingType>>,
}
impl Default for TemplateSchema {
    fn default() -> Self {
        let mut templates = BTreeMap::new();
        for (name, slots, texts, bools) in [
            (
                "chat-message",
                &[][..],
                &[
                    "timestamp",
                    "origin",
                    "action-marker",
                    "sender",
                    "sender-delimiter",
                ][..],
                &["external", "action", "named"][..],
            ),
            (
                "file-browser",
                &["body"][..],
                &["title", "filter-label"][..],
                &["filter-visible"][..],
            ),
            ("episode-browser", &["body"][..], &["title"][..], &[][..]),
            (
                "anidb-search",
                &["body", "editor"][..],
                &["title", "message"][..],
                &["editing", "has-message", "has-results"][..],
            ),
            (
                "nyaa-search",
                &["body", "editor"][..],
                &["title", "message"][..],
                &["editing", "has-message", "has-results"][..],
            ),
            (
                "anidb-result",
                &[][..],
                &["title", "matched", "series"][..],
                &["alias"][..],
            ),
            (
                "nyaa-result",
                &[][..],
                &["filename", "title", "size", "seeders", "stage", "progress"][..],
                &["alias", "result", "active"][..],
            ),
            (
                "confirm-dialog",
                &[][..],
                &["title", "prompt", "yes", "no"][..],
                &[][..],
            ),
            (
                "name-dialog",
                &["editor"][..],
                &["title", "note"][..],
                &[][..],
            ),
            (
                "copy-dialog",
                &["body"][..],
                &["title", "filename", "note"][..],
                &[][..],
            ),
            ("copy-row", &[][..], &["filename", "evidence"][..], &[][..]),
            (
                "form-tab",
                &[][..],
                &["open", "label", "missing", "close"][..],
                &["invalid"][..],
            ),
            (
                "work-row",
                &[][..],
                &["stage", "filename", "open", "close"][..],
                &[][..],
            ),
            (
                "page-shell",
                &["page", "recent", "keybar"][..],
                &[][..],
                &[][..],
            ),
            ("form-note", &[][..], &["body"][..], &[][..]),
            (
                "form-save",
                &[][..],
                &["save-label", "save-needs", "save-hint"][..],
                &["save-blocked"][..],
            ),
            ("form-error", &[][..], &["error"][..], &[][..]),
            ("changelog", &["body"][..], &["title", "ok"][..], &[][..]),
            (
                "changelog-row",
                &[][..],
                &["date", "bullet", "category", "body"][..],
                &["day", "entry", "categorized"][..],
            ),
            (
                "status",
                &[][..],
                &["marker", "state", "blockers", "now-label", "title"][..],
                &["has-title", "blocked"][..],
            ),
            (
                "health",
                &["progress", "middle", "metrics"][..],
                &[][..],
                &[][..],
            ),
            (
                "health-progress",
                &[][..],
                &["open", "fill", "close", "elapsed", "slash", "duration"][..],
                &["available"][..],
            ),
            (
                "health-middle",
                &["marquee"][..],
                &["body"][..],
                &["animation", "suggestion"][..],
            ),
            (
                "health-metric",
                &[][..],
                &[
                    "separator",
                    "label",
                    "value",
                    "up-marker",
                    "down-marker",
                    "upload",
                    "download",
                ][..],
                &["bandwidth", "ordinary", "suffix", "separated"][..],
            ),
            (
                "keybinding",
                &[][..],
                &["separator", "key", "label"][..],
                &["separated"][..],
            ),
            ("file-row", &[][..], &["marker", "name"][..], &["entry"][..]),
            (
                "episode-row",
                &[][..],
                &[
                    "marker", "episode", "filename", "holders", "gutter", "title", "count", "note",
                ][..],
                &["file", "has-episode", "child", "season", "branch", "empty"][..],
            ),
            ("chat-separator", &[][..], &["label"][..], &[][..]),
            (
                "rogue-inspection",
                &["body"][..],
                &["heading", "note"][..],
                &["has-note"][..],
            ),
            (
                "rogue-equipment-row",
                &[][..],
                &["description"][..],
                &[][..],
            ),
            (
                "rogue-epitaph",
                &[][..],
                &["heading", "summary", "actions"][..],
                &[][..],
            ),
            ("rogue-document", &[][..], &["body"][..], &[][..]),
            (
                "rogue-recovery",
                &["wounds"][..],
                &[
                    "title",
                    "footer",
                    "phase",
                    "blood-label",
                    "blood",
                    "breath-label",
                    "breath",
                    "nutrition-label",
                    "nutrition",
                    "bleed-label",
                    "bleed",
                    "pain-label",
                    "pain",
                    "linen-label",
                    "linen",
                    "linen-used",
                    "splints-label",
                    "splints",
                    "splints-used",
                    "food-label",
                    "food",
                    "food-used",
                ][..],
                &[][..],
            ),
            (
                "rogue",
                &["body"][..],
                &["title", "footer", "notices", "error"][..],
                &["has-notices", "has-error"][..],
            ),
            (
                "rogue-game",
                &["map", "sidebar", "journal"][..],
                &[
                    "objective",
                    "weapon",
                    "reach",
                    "depth-label",
                    "depth",
                    "blood-label",
                    "blood",
                    "breath-label",
                    "breath",
                    "pain-label",
                    "pain",
                    "bleed-label",
                    "bleed",
                    "linen-label",
                    "linen",
                    "splints-label",
                    "splints",
                    "food-label",
                    "food",
                    "nutrition-label",
                    "nutrition",
                    "gold-label",
                    "gold",
                ][..],
                &["wide", "has-reach"][..],
            ),
            (
                "series",
                &["body"][..],
                &["title", "filter-label"][..],
                &["filter-visible"][..],
            ),
            (
                "series-row",
                &[][..],
                &[
                    "title",
                    "year",
                    "marker",
                    "count",
                    "unavailable",
                    "name",
                    "nero",
                    "episode",
                    "available",
                    "watchers",
                ][..],
                &[
                    "franchise",
                    "heading",
                    "entry",
                    "has-year",
                    "has-nero",
                    "unlinked",
                ][..],
            ),
            ("playlist", &["body"][..], &["title"][..], &[][..]),
            ("users", &["body"][..], &["title"][..], &[][..]),
            (
                "playlist-row",
                &[][..],
                &["marker", "title", "download", "temporary", "watch"][..],
                &["entry", "show-download", "show-temporary"][..],
            ),
            (
                "user-row",
                &[][..],
                &["name", "status", "last-seen", "holders"][..],
                &["online", "offline", "seeders"][..],
            ),
            ("recent-chat", &["log"][..], &["title"][..], &[][..]),
            ("subtitles", &["body"][..], &["title"][..], &[][..]),
            (
                "subtitle-row",
                &[][..],
                &["timestamp", "speaker", "body"][..],
                &["named"][..],
            ),
            (
                "chat",
                &["log", "input", "suggestions"][..],
                &["title", "input-title"][..],
                &["suggesting"][..],
            ),
            (
                "chat-suggestion",
                &[][..],
                &["signature", "help"][..],
                &[][..],
            ),
            (
                "chat-attachment",
                &["timestamp-gutter", "image"][..],
                &[][..],
                &[][..],
            ),
            (
                "app",
                &[
                    "chat",
                    "subtitles",
                    "series",
                    "users",
                    "playlist",
                    "health",
                    "status",
                    "keybar",
                ][..],
                &[][..],
                &["separate-subtitles"][..],
            ),
            (
                "form",
                &["header", "body", "notes", "save", "editor", "error"][..],
                &["title"][..],
                &["editing", "invalid"][..],
            ),
            (
                "log",
                &["body", "app-options", "other-options"][..],
                &[
                    "title",
                    "session-label",
                    "startup",
                    "app-label",
                    "app-level",
                    "other-label",
                    "other-level",
                    "picker-end",
                    "footer",
                    "unavailable",
                ][..],
                &["choose-app", "choose-other", "has-unavailable", "available"][..],
            ),
            ("log-option", &[][..], &["label"][..], &[][..]),
            ("layout-tools", &[][..], &["title", "body"][..], &[][..]),
            (
                "form-row",
                &[][..],
                &["label", "value", "annotation", "open", "close"][..],
                &["labelled", "annotated", "action"][..],
            ),
        ] {
            let mut fields = BTreeMap::new();
            for (names, kind) in [
                (slots, BindingType::Slot),
                (texts, BindingType::Text),
                (bools, BindingType::Bool),
            ] {
                fields.extend(names.iter().map(|name| (name.to_string(), kind)));
            }
            templates.insert(name.into(), fields);
        }
        templates.insert(
            "rich-text".into(),
            [("body".into(), BindingType::Rich)].into(),
        );
        templates.insert(
            "health-metrics".into(),
            [("metrics".into(), BindingType::List("health-metric"))].into(),
        );
        templates.insert(
            "keybar".into(),
            [("bindings".into(), BindingType::List("keybinding"))].into(),
        );
        templates.insert(
            "form-tabs".into(),
            [("tabs".into(), BindingType::List("form-tab"))].into(),
        );
        templates.insert(
            "form-notes".into(),
            [("notes".into(), BindingType::List("form-note"))].into(),
        );
        templates
            .entry("form".into())
            .or_default()
            .insert("tabs".into(), BindingType::List("form-tab"));
        templates
            .entry("form".into())
            .or_default()
            .insert("note-items".into(), BindingType::List("form-note"));
        templates
            .entry("file-browser".into())
            .or_default()
            .insert("filter".into(), BindingType::Rich);
        templates
            .entry("series".into())
            .or_default()
            .insert("filter".into(), BindingType::Rich);
        templates
            .entry("chat-message".into())
            .or_default()
            .insert("body".into(), BindingType::Rich);
        templates.insert(
            "work-overlay".into(),
            [
                ("title".into(), BindingType::Text),
                ("jobs".into(), BindingType::List("work-row")),
            ]
            .into(),
        );
        templates
            .entry("work-row".into())
            .or_default()
            .insert("progress".into(), BindingType::Progress);
        for name in ["settings-form", "list-edit-form"] {
            templates.insert(name.into(), templates["form"].clone());
        }
        Self { templates }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Node {
    pub tag: String,
    pub attrs: BTreeMap<String, String>,
    pub inline: Vec<Declaration>,
    pub children: Vec<Node>,
    pub location: Diagnostic,
}
impl Node {
    pub fn attr(&self, name: &str) -> &str {
        self.attrs.get(name).map(String::as_str).unwrap_or("")
    }
}

/// Fully validated immutable definitions, safe to send to the UI thread.
#[derive(Clone, Debug)]
pub struct LayoutBundle {
    pub(super) templates: BTreeMap<String, Node>,
    pub(super) rules: Vec<Rule>,
    /// Content identity used to invalidate caches and saved drag proportions.
    pub revision: String,
    /// Override directory, or none for embedded assets.
    pub source: Option<PathBuf>,
}
impl LayoutBundle {
    /// Compile embedded defaults using precisely the custom-bundle compiler.
    pub fn builtin() -> Result<Self, Diagnostic> {
        static BUNDLE: std::sync::OnceLock<Result<LayoutBundle, Diagnostic>> =
            std::sync::OnceLock::new();
        BUNDLE
            .get_or_init(|| Self::compile(None, &TemplateSchema::default()))
            .clone()
    }

    /// Overlay local files over embedded definitions, validating the complete result.
    pub fn load(path: &Path) -> Result<Self, Diagnostic> {
        Self::compile(Some(path), &TemplateSchema::default())
    }

    fn compile(path: Option<&Path>, schema: &TemplateSchema) -> Result<Self, Diagnostic> {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        let mut templates = BTreeMap::new();
        let mut rules = Vec::new();
        for (file, source) in DEFAULTS {
            hash.update(file);
            hash.update(source);
            if file.ends_with(".xml") {
                parse_xml(Path::new(file), source, &mut templates)?;
            } else {
                rules.extend(super::css::parse(Path::new(file), source)?);
            }
        }
        if let Some(path) = path {
            let dir = path.join("templates");
            let mut files = match std::fs::read_dir(&dir) {
                Ok(entries) => entries
                    .map(|entry| entry.map(|e| e.path()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| Diagnostic::at(&dir, "", 0, e.to_string()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(Diagnostic::at(&dir, "", 0, e.to_string())),
            };
            files.retain(|p| p.extension().is_some_and(|e| e == "xml"));
            files.sort();
            if files.len() > 128 {
                return Err(Diagnostic::at(
                    &dir,
                    "",
                    0,
                    "at most 128 template files are supported",
                ));
            }
            let mut overrides = BTreeMap::new();
            for file in files {
                let source = read(&file)?;
                hash.update(
                    file.strip_prefix(path)
                        .unwrap_or(&file)
                        .to_string_lossy()
                        .as_bytes(),
                );
                hash.update(&source);
                parse_xml(&file, &source, &mut overrides)?;
            }
            templates.extend(overrides);
            let css = path.join("style.css");
            match std::fs::metadata(&css) {
                Ok(_) => {
                    let source = read(&css)?;
                    hash.update(&source);
                    rules.extend(super::css::parse(&css, &source)?);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Diagnostic::at(&css, "", 0, e.to_string())),
            }
        }
        let definitions = templates;
        let mut templates = BTreeMap::new();
        let mut referenced = BTreeSet::new();
        for name in schema.templates.keys() {
            let Some(root) = definitions.get(name) else {
                continue;
            };
            let mut budget = MAX_NODES;
            let root = expand(
                root,
                &definitions,
                &mut vec![name.clone()],
                &mut referenced,
                &mut budget,
            )?;
            let fields = &schema.templates[name];
            let mut ids = BTreeSet::new();
            let mut slots = BTreeSet::new();
            validate(
                &root,
                fields,
                &mut ids,
                &mut slots,
                false,
                &schema.templates,
            )?;
            templates.insert(name.clone(), root);
        }
        for (name, node) in &definitions {
            if !schema.templates.contains_key(name) && !referenced.contains(name) {
                return Err(error(
                    node,
                    format!(
                        "unknown entry template {name:?}; helper templates must be referenced by <use>"
                    ),
                ));
            }
        }
        let bundle = Self {
            templates,
            rules,
            revision: format!("{:x}", hash.finalize()),
            source: path.map(Path::to_path_buf),
        };
        super::css::validate_bundle(&bundle)?;
        Ok(bundle)
    }

    /// Export editable defaults. Existing files are never truncated or replaced.
    pub fn init(path: &Path) -> Result<(), Diagnostic> {
        use std::io::Write;
        std::fs::create_dir_all(path.join("templates"))
            .map_err(|e| Diagnostic::at(path, "", 0, e.to_string()))?;
        for (name, source) in DEFAULTS {
            let file = path.join(name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)
            {
                Ok(mut out) => out
                    .write_all(source.as_bytes())
                    .map_err(|e| Diagnostic::at(&file, "", 0, e.to_string()))?,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(Diagnostic::at(&file, "", 0, e.to_string())),
            }
        }
        Ok(())
    }
}
fn read(path: &Path) -> Result<String, Diagnostic> {
    use std::io::Read;
    let mut source = String::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE + 1).read_to_string(&mut source))
        .map_err(|e| Diagnostic::at(path, "", 0, e.to_string()))?;
    if source.len() as u64 > MAX_FILE {
        return Err(Diagnostic::at(path, "", 0, "layout file exceeds 1 MiB"));
    }
    Ok(source)
}
fn error(node: &Node, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        message: message.into(),
        ..node.location.clone()
    }
}
fn expand(
    node: &Node,
    definitions: &BTreeMap<String, Node>,
    stack: &mut Vec<String>,
    referenced: &mut BTreeSet<String>,
    budget: &mut usize,
) -> Result<Node, Diagnostic> {
    if *budget == 0 || stack.len() > 64 {
        return Err(error(node, "expanded template complexity limit exceeded"));
    }
    *budget -= 1;
    if node.tag == "use" {
        if !node.children.is_empty() {
            return Err(error(node, "use cannot contain children"));
        }
        let name = node.attr("template");
        let target = definitions
            .get(name)
            .ok_or_else(|| error(node, format!("unknown template reference {name:?}")))?;
        if stack.iter().any(|part| part == name) {
            return Err(error(
                node,
                format!("template cycle: {} → {name}", stack.join(" → ")),
            ));
        }
        referenced.insert(name.into());
        stack.push(name.into());
        let expanded = expand(target, definitions, stack, referenced, budget)?;
        stack.pop();
        return Ok(expanded);
    }
    let mut expanded = node.clone();
    expanded.children = node
        .children
        .iter()
        .map(|child| expand(child, definitions, stack, referenced, budget))
        .collect::<Result<_, _>>()?;
    Ok(expanded)
}
fn validate(
    node: &Node,
    fields: &BTreeMap<String, BindingType>,
    ids: &mut BTreeSet<String>,
    slots: &mut BTreeSet<String>,
    in_inline: bool,
    schemas: &BTreeMap<String, BTreeMap<String, BindingType>>,
) -> Result<(), Diagnostic> {
    if !node.attr("id").is_empty() && !ids.insert(node.attr("id").into()) {
        return Err(error(node, "duplicate node id"));
    }
    if node.attrs.contains_key("resizable") {
        if node.attr("resizable") != "true" || node.attr("id").is_empty() {
            return Err(error(
                node,
                "resizable containers require resizable=\"true\" and a stable id",
            ));
        }
        if node.children.len() < 2 || node.children.iter().any(|c| c.attr("id").is_empty()) {
            return Err(error(
                node,
                "resizable containers require at least two children with stable ids",
            ));
        }
    }
    for (attr, kind) in [
        ("if", BindingType::Bool),
        (
            "bind",
            if node.tag == "repeat" {
                match fields.get(node.attr("bind")) {
                    Some(kind @ BindingType::List(_)) => *kind,
                    _ => return Err(error(node, "repeat requires a typed list binding")),
                }
            } else if node.tag == "progress" {
                BindingType::Progress
            } else if node.tag == "rich" {
                BindingType::Rich
            } else {
                BindingType::Text
            },
        ),
        ("title", BindingType::Text),
        ("title-bottom", BindingType::Text),
        ("name", BindingType::Slot),
    ] {
        let value = node.attr(attr);
        if value.is_empty() || (attr == "name" && node.tag != "slot") {
            continue;
        }
        if fields.get(value) != Some(&kind) {
            return Err(error(
                node,
                format!("unknown or incorrectly typed {attr} binding {value:?}"),
            ));
        }
    }
    if node.tag == "slot"
        && (node.attr("name").is_empty() || !slots.insert(node.attr("name").into()))
    {
        return Err(error(node, "slots require a unique controller binding"));
    }
    if matches!(node.tag.as_str(), "text" | "rich" | "progress") && node.attr("bind").is_empty() {
        return Err(error(node, "text requires a bind attribute"));
    }
    if node.tag == "rich" && !slots.insert(format!("rich:{}", node.attr("bind"))) {
        return Err(error(
            node,
            "action-bearing rich bindings must occur once per entry; use a separate read-only projection",
        ));
    }
    if matches!(node.tag.as_str(), "slot" | "text" | "rich" | "progress")
        && !node.children.is_empty()
    {
        return Err(error(node, "leaf elements cannot contain children"));
    }
    if matches!(node.tag.as_str(), "flow" | "prefix")
        && node
            .children
            .iter()
            .any(|child| !matches!(child.tag.as_str(), "text" | "rich" | "flow" | "prefix"))
    {
        return Err(error(
            node,
            "flow accepts only text, rich, prefix, and nested flow children",
        ));
    }
    if node.tag == "prefix" && !in_inline {
        return Err(error(
            node,
            "prefix must be the first child of an outer flow",
        ));
    }
    for (index, child) in node.children.iter().enumerate() {
        if child.tag == "prefix" && (node.tag != "flow" || in_inline || index != 0) {
            return Err(error(
                child,
                "prefix must be the first child of an outer flow",
            ));
        }
    }
    if node.attrs.contains_key("placement")
        && !matches!(
            node.attr("placement"),
            "center" | "bottom" | "after" | "before" | "modal"
        )
    {
        return Err(error(
            node,
            "overlay placement must be center, modal, bottom, before, or after",
        ));
    }
    if node.tag == "repeat" {
        let Some(BindingType::List(contract)) = fields.get(node.attr("bind")) else {
            return Err(error(node, "repeat requires a typed list binding"));
        };
        if node.children.len() != 1 || !slots.insert(format!("list:{}", node.attr("bind"))) {
            return Err(error(
                node,
                "repeat requires one item root and a unique list binding",
            ));
        }
        return validate(
            &node.children[0],
            &schemas[*contract],
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            false,
            schemas,
        );
    }
    for child in &node.children {
        validate(
            child,
            fields,
            ids,
            slots,
            in_inline || matches!(node.tag.as_str(), "flow" | "prefix"),
            schemas,
        )?;
    }
    Ok(())
}
fn parse_xml(
    file: &Path,
    source: &str,
    out: &mut BTreeMap<String, Node>,
) -> Result<(), Diagnostic> {
    let mut reader = Reader::from_str(source);
    let mut stack: Vec<Node> = Vec::new();
    let mut count = 0;
    let mut saw_root = false;
    loop {
        let offset = reader.buffer_position() as usize;
        let event = reader
            .read_event()
            .map_err(|e| Diagnostic::at(file, source, offset, e.to_string()))?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                count += 1;
                if count > MAX_NODES || stack.len() > 64 {
                    return Err(Diagnostic::at(
                        file,
                        source,
                        offset,
                        "template complexity limit exceeded",
                    ));
                }
                let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let mut attrs = BTreeMap::new();
                for attr in e.attributes() {
                    let attr =
                        attr.map_err(|e| Diagnostic::at(file, source, offset, e.to_string()))?;
                    let name = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|e| Diagnostic::at(file, source, offset, e.to_string()))?
                        .into_owned();
                    let allowed = match tag.as_str() {
                        "templates" => &["version"][..],
                        "template" => &["name"][..],
                        "use" => &["template"][..],
                        "row" | "column" => &[
                            "id",
                            "class",
                            "style",
                            "if",
                            "title",
                            "title-bottom",
                            "resizable",
                        ][..],
                        "grid" | "box" | "scroll" => {
                            &["id", "class", "style", "if", "title", "title-bottom"][..]
                        }
                        "overlay" => &[
                            "id",
                            "class",
                            "style",
                            "if",
                            "title",
                            "title-bottom",
                            "placement",
                        ][..],
                        "flow" | "prefix" => &["id", "class", "style", "if", "separator"][..],
                        "slot" => &["id", "class", "style", "if", "name"][..],
                        "text" | "rich" | "repeat" | "progress" => {
                            &["id", "class", "style", "if", "bind"][..]
                        }
                        _ => {
                            return Err(Diagnostic::at(
                                file,
                                source,
                                offset,
                                format!("unknown element {tag:?}"),
                            ));
                        }
                    };
                    if !allowed.contains(&name.as_str()) {
                        return Err(Diagnostic::at(
                            file,
                            source,
                            offset,
                            format!("unsupported attribute {name:?} on {tag}"),
                        ));
                    }
                    attrs.insert(name, value);
                }
                if ![
                    "templates",
                    "template",
                    "use",
                    "row",
                    "column",
                    "grid",
                    "box",
                    "scroll",
                    "overlay",
                    "slot",
                    "text",
                    "rich",
                    "flow",
                    "prefix",
                    "repeat",
                    "progress",
                ]
                .contains(&tag.as_str())
                {
                    return Err(Diagnostic::at(
                        file,
                        source,
                        offset,
                        format!("unknown element {tag:?}"),
                    ));
                }
                if stack.is_empty() {
                    if tag != "templates"
                        || saw_root
                        || attrs.get("version").map(String::as_str) != Some("1")
                    {
                        return Err(Diagnostic::at(
                            file,
                            source,
                            offset,
                            "expected one <templates version=\"1\"> root; unsupported contract version",
                        ));
                    }
                    saw_root = true;
                } else if (stack.len() == 1) != (tag == "template") || tag == "templates" {
                    return Err(Diagnostic::at(
                        file,
                        source,
                        offset,
                        "templates must contain named templates, each with one component root",
                    ));
                }
                let inline = attrs
                    .get("style")
                    .map(|s| super::css::declarations(file, s))
                    .transpose()?
                    .unwrap_or_default();
                let node = Node {
                    tag,
                    attrs,
                    inline,
                    children: Vec::new(),
                    location: Diagnostic::at(file, source, offset, ""),
                };
                stack.push(node);
                if empty {
                    close(&mut stack, out)?;
                }
            }
            Event::End(_) => close(&mut stack, out)?,
            Event::Text(e)
                if e.unescape()
                    .map_err(|e| Diagnostic::at(file, source, offset, e.to_string()))?
                    .trim()
                    .is_empty() => {}
            Event::Comment(_) => {}
            Event::Decl(_) if !saw_root => {}
            Event::Eof => break,
            _ => {
                return Err(Diagnostic::at(
                    file,
                    source,
                    offset,
                    "only elements and comments are supported; text must use typed bindings",
                ));
            }
        }
    }
    if !saw_root || !stack.is_empty() {
        return Err(Diagnostic::at(
            file,
            source,
            source.len(),
            "incomplete template file",
        ));
    }
    Ok(())
}
fn close(stack: &mut Vec<Node>, out: &mut BTreeMap<String, Node>) -> Result<(), Diagnostic> {
    let Some(mut node) = stack.pop() else {
        return Ok(());
    };
    if node.tag == "template" {
        let name = node.attr("name").to_string();
        if name.is_empty() || node.children.len() != 1 {
            return Err(error(
                &node,
                "a named template requires exactly one component root",
            ));
        }
        if out.contains_key(&name) {
            return Err(error(&node, format!("duplicate template {name:?}")));
        }
        out.insert(name, node.children.remove(0));
    } else if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    }
    Ok(())
}
