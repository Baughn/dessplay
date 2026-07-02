//! Modal components: centered overlays that capture input while open
//! (ui-architecture.md, Modals). The dispatcher keeps a modal stack;
//! the background keeps rendering and the event loop keeps running.

use std::path::{Path, PathBuf};

use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};
use tuirealm::state::State;

use dessplay_core::net::AniDbSearchHit;
use dessplay_core::types::{Ed2kHash, ListEntryId, ListStatus, NextEpState, SeriesListEntry};

use super::components::plain;
use super::msg::Msg;
use super::theme;
use super::widgets::{
    Binding, CharOutcome, Form, FormEvent, FormModel, KeyPattern, Keymap, ListCursor, RowAction,
    TextField, render_list,
};
use crate::config::Settings;

/// Like `passive_component!` but without a focus field (modals are
/// always focused while open).
macro_rules! passive_modal {
    ($ty:ty) => {
        impl Component for $ty {
            fn view(&mut self, frame: &mut Frame, area: Rect) {
                self.render(frame, area);
            }
            fn query<'a>(&'a self, _attr: Attribute) -> Option<QueryResult<'a>> {
                None
            }
            fn attr(&mut self, _attr: Attribute, _value: AttrValue) {}
            fn state(&self) -> State {
                State::None
            }
            fn perform(&mut self, _cmd: Cmd) -> CmdResult {
                CmdResult::NoChange
            }
        }
    };
}

// The centered-overlay helper lives with the Form widget; re-exported so
// existing `modals::overlay` callers (and tests) keep their import path.
pub use super::widgets::overlay;

/// A one-line text editor for modal fields: a [`TextField`] plus the
/// modal commit protocol. Editing behavior (word motion, word kill,
/// scroll discipline) is the shared vocabulary — identical to the chat
/// input by construction.
struct FieldEditor {
    input: TextField,
}

impl FieldEditor {
    fn new(initial: &str) -> Self {
        Self {
            input: TextField::with_text(initial),
        }
    }

    fn text(&self) -> String {
        self.input.text()
    }

    /// Feed an event; `Some(committed)` when editing ends.
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<bool> {
        match plain(ev) {
            Some(Key::Enter) => return Some(true),
            Some(Key::Esc) => return Some(false),
            _ => {}
        }
        self.input.edit(ev);
        None
    }

    fn view(&mut self, frame: &mut Frame, area: Rect) {
        self.input.render(frame, area, true, false);
    }
}

/// Render a selectable list as a modal overlay.
fn render_modal_list(frame: &mut Frame, area: Rect, title: &str, items: Vec<ListItem>, sel: usize) {
    let area = overlay(area, 70, 70);
    frame.render_widget(Clear, area);
    let selected = (!items.is_empty()).then_some(sel);
    render_list(frame, area, title.to_string(), items, selected, true);
}

// ---- File browser ------------------------------------------------------

/// What the browser selects.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BrowseFor {
    /// A media file (playlist add). Carries the playlist anchor.
    File,
    /// A directory (settings media root).
    Directory,
    /// A replacement file for a missing playlist entry (manual map).
    /// Files in each directory are sorted by edit distance to the
    /// target filename (design.md, Manual File Mapping).
    Map {
        /// The playlist entry being mapped.
        file: Ed2kHash,
        /// The target filename, for the edit-distance sort.
        target: String,
    },
}

/// Distinguishes synthetic navigation rows from real filesystem entries.
enum RowKind {
    /// `[Select]` — confirm the current directory as a media root.
    Select,
    /// `..` — go up one level.
    Parent,
    /// A real file/directory at `path`.
    Entry,
}

struct DirRow {
    name: String,
    path: PathBuf,
    is_dir: bool,
    kind: RowKind,
}

/// Browse the media roots (or the whole filesystem for directory
/// selection).
pub struct FileBrowser {
    purpose: BrowseFor,
    /// Playlist anchor for file selection (`None` = append).
    pub after: Option<Ed2kHash>,
    roots: Vec<PathBuf>,
    /// `None` = listing the roots themselves.
    cwd: Option<PathBuf>,
    entries: Vec<DirRow>,
    cursor: ListCursor,
}

impl FileBrowser {
    /// Browse for a file to add to the playlist.
    pub fn for_file(roots: Vec<PathBuf>, after: Option<Ed2kHash>) -> Self {
        let mut browser = Self {
            purpose: BrowseFor::File,
            after,
            roots,
            cwd: None,
            entries: Vec::new(),
            cursor: ListCursor::default(),
        };
        browser.refresh();
        browser
    }

    /// Browse for a replacement file to map a missing entry to. Opens
    /// at `start` (the series' last-used directory if known, else the
    /// media roots); files are ranked by edit distance to `target`.
    pub fn for_mapping(
        roots: Vec<PathBuf>,
        file: Ed2kHash,
        target: String,
        start: Option<PathBuf>,
    ) -> Self {
        let mut browser = Self {
            purpose: BrowseFor::Map { file, target },
            after: None,
            roots,
            cwd: start,
            entries: Vec::new(),
            cursor: ListCursor::default(),
        };
        browser.refresh();
        browser
    }

    /// Browse for a directory (media-root picker). Starts at `$HOME`.
    pub fn for_directory() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut browser = Self {
            purpose: BrowseFor::Directory,
            after: None,
            roots: vec![home.clone()],
            cwd: Some(home),
            entries: Vec::new(),
            cursor: ListCursor::default(),
        };
        browser.refresh();
        browser
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.cursor.reset();
        match &self.cwd {
            None => {
                for root in &self.roots {
                    self.entries.push(DirRow {
                        name: root.display().to_string(),
                        path: root.clone(),
                        is_dir: true,
                        kind: RowKind::Entry,
                    });
                }
            }
            Some(dir) => {
                let mut rows: Vec<DirRow> = std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|entry| {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') {
                            return None;
                        }
                        // Follow symlinks: `DirEntry::file_type()` reports the
                        // link itself, so a symlinked directory would otherwise
                        // look like a non-dir. No cycle worry — this lists one
                        // level, it doesn't recurse.
                        let file_type = entry.file_type().ok()?;
                        let is_dir = if file_type.is_symlink() {
                            std::fs::metadata(entry.path())
                                .map(|m| m.is_dir())
                                .unwrap_or(false)
                        } else {
                            file_type.is_dir()
                        };
                        if matches!(self.purpose, BrowseFor::Directory) && !is_dir {
                            return None;
                        }
                        Some(DirRow {
                            name,
                            path: entry.path(),
                            is_dir,
                            kind: RowKind::Entry,
                        })
                    })
                    .collect();
                // Directories first, then by name — except in mapping
                // mode, where files are ranked by edit distance to the
                // target so the likely match floats to the top.
                match &self.purpose {
                    BrowseFor::Map { target, .. } => {
                        let target = target.clone();
                        rows.sort_by(|a, b| {
                            b.is_dir.cmp(&a.is_dir).then_with(|| {
                                if a.is_dir {
                                    a.name.cmp(&b.name)
                                } else {
                                    strsim::levenshtein(&a.name, &target)
                                        .cmp(&strsim::levenshtein(&b.name, &target))
                                        .then_with(|| a.name.cmp(&b.name))
                                }
                            })
                        });
                    }
                    _ => {
                        rows.sort_by(|a, b| {
                            b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name))
                        });
                    }
                }
                // In the media-root picker, surface "confirm this
                // directory" and "go up" as selectable rows at the top, so
                // they're discoverable beyond the bare `s` / Backspace keys.
                if matches!(self.purpose, BrowseFor::Directory) {
                    rows.insert(
                        0,
                        DirRow {
                            name: "..".to_string(),
                            path: dir.clone(),
                            is_dir: true,
                            kind: RowKind::Parent,
                        },
                    );
                    rows.insert(
                        0,
                        DirRow {
                            name: "[Select]".to_string(),
                            path: dir.clone(),
                            is_dir: false,
                            kind: RowKind::Select,
                        },
                    );
                }
                self.entries = rows;
            }
        }
    }

    fn ascend(&mut self) -> bool {
        match self.cwd.take() {
            None => false,
            Some(dir) => {
                if matches!(self.purpose, BrowseFor::Directory) {
                    // The media-root picker spans the whole filesystem: walk
                    // up to the real parent (so directories outside $HOME are
                    // reachable), stopping only at the filesystem root.
                    self.cwd = Some(dir.parent().map(Path::to_path_buf).unwrap_or(dir));
                } else if !self.roots.contains(&dir)
                    && let Some(parent) = dir.parent()
                {
                    // File/map browsers are confined to the media roots: leaving
                    // a root returns to the roots listing.
                    self.cwd = Some(parent.to_path_buf());
                }
                self.refresh();
                true
            }
        }
    }

    /// The active keymap (per purpose — labels differ, and `s` exists
    /// only in the directory picker).
    fn keymap(&self) -> &'static Keymap<FileBrowser, Msg> {
        match self.purpose {
            BrowseFor::File => &BROWSER_FILE_KEYMAP,
            BrowseFor::Map { .. } => &BROWSER_MAP_KEYMAP,
            BrowseFor::Directory => &BROWSER_DIR_KEYMAP,
        }
    }

    /// Keys for the keybinding bar (derived from the active keymap).
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        self.keymap().bar()
    }

    /// Enter: open a directory, act on a synthetic row, or choose a file.
    fn act_enter(&mut self) -> Option<Msg> {
        let row = self.entries.get(self.cursor.index())?;
        match row.kind {
            RowKind::Select => return self.cwd.clone().map(Msg::DirChosen),
            RowKind::Parent => {
                self.ascend();
                return Some(Msg::None);
            }
            RowKind::Entry => {}
        }
        if row.is_dir {
            self.cwd = Some(row.path.clone());
            self.refresh();
            return Some(Msg::None);
        }
        match &self.purpose {
            BrowseFor::Map { file, .. } => Some(Msg::FileMapped {
                file: *file,
                path: row.path.clone(),
            }),
            _ => Some(Msg::FileChosen {
                path: row.path.clone(),
                after: self.after,
            }),
        }
    }

    /// `s` (directory picker): confirm the current directory.
    fn act_select_here(&mut self) -> Option<Msg> {
        self.cwd.clone().map(Msg::DirChosen)
    }

    /// Backspace: up one level; from the roots listing, close.
    fn act_up(&mut self) -> Option<Msg> {
        if self.ascend() {
            Some(Msg::None)
        } else {
            Some(Msg::CloseModal)
        }
    }

    fn act_close(&mut self) -> Option<Msg> {
        Some(Msg::CloseModal)
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let title = match (&self.cwd, &self.purpose) {
            (None, _) => "Media roots".to_string(),
            (Some(dir), BrowseFor::File) => format!("Add file — {}", dir.display()),
            (Some(dir), BrowseFor::Directory) => format!("Pick directory — {}", dir.display()),
            (Some(dir), BrowseFor::Map { target, .. }) => {
                format!("Map “{target}” — {}", dir.display())
            }
        };
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|row| match row.kind {
                RowKind::Select | RowKind::Parent => ListItem::new(row.name.clone()),
                RowKind::Entry => {
                    let prefix = if row.is_dir { "▸ " } else { "  " };
                    ListItem::new(format!("{prefix}{}", row.name))
                }
            })
            .collect();
        render_modal_list(frame, area, &title, items, self.cursor.index());
    }
}

passive_modal!(FileBrowser);

impl AppComponent<Msg, NoUserEvent> for FileBrowser {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some(key) = plain(ev)
            && self.cursor.nav(key, self.entries.len())
        {
            return Some(Msg::None);
        }
        self.keymap().dispatch(self, ev)
    }
}

/// Playlist-add browser bindings.
static BROWSER_FILE_KEYMAP: Keymap<FileBrowser, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Open/Add")),
        action: FileBrowser::act_enter,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Backspace),
        bar: Some(("Bksp", "Up")),
        action: FileBrowser::act_up,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: Some(("Esc", "Cancel")),
        action: FileBrowser::act_close,
    },
]);

/// Manual-mapping browser bindings (same keys, mapping labels).
static BROWSER_MAP_KEYMAP: Keymap<FileBrowser, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Open/Map")),
        action: FileBrowser::act_enter,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Backspace),
        bar: Some(("Bksp", "Up")),
        action: FileBrowser::act_up,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: Some(("Esc", "Cancel")),
        action: FileBrowser::act_close,
    },
]);

/// Directory-picker bindings: adds `s` ("select here").
static BROWSER_DIR_KEYMAP: Keymap<FileBrowser, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Open")),
        action: FileBrowser::act_enter,
    },
    Binding {
        pattern: KeyPattern::Char('s'),
        bar: Some(("s", "Select here")),
        action: FileBrowser::act_select_here,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Backspace),
        bar: Some(("Bksp", "Up")),
        action: FileBrowser::act_up,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: Some(("Esc", "Cancel")),
        action: FileBrowser::act_close,
    },
]);

// ---- Settings ----------------------------------------------------------

/// Field index layout: fixed fields then one row per media root, then
/// the add-root row.
const FIELD_USERNAME: usize = 0;
const FIELD_SERVER: usize = 1;
const FIELD_PASSWORD: usize = 2;
const FIELD_READY: usize = 3;
const FIELD_SUBTITLE: usize = 4;
const FIELD_CACHE: usize = 5;
const FIELD_AUTO_DOWNLOAD: usize = 6;
const FIELD_IRC_ENABLED: usize = 7;
const FIELD_IRC_SERVER: usize = 8;
const FIELD_IRC_TLS: usize = 9;
const FIELD_IRC_CHANNEL: usize = 10;
const FIXED_FIELDS: usize = 11;

/// First-run and later settings editing: a [`Form`] over the working
/// settings and media roots. All form behavior (cursor, editor, save
/// keys) is the shared widget; this file only declares the rows.
pub struct SettingsModal {
    form: Form<SettingsForm>,
}

/// The settings form model: working copies, committed on save.
pub struct SettingsForm {
    /// The working copy.
    pub settings: Settings,
    /// Working media roots (position 0 is the download target).
    pub roots: Vec<PathBuf>,
}

impl SettingsModal {
    /// Open with current values.
    pub fn new(settings: Settings, roots: Vec<PathBuf>) -> Self {
        Self {
            form: Form::new(SettingsForm { settings, roots }),
        }
    }

    /// A directory was picked for a new media root.
    pub fn add_root(&mut self, root: PathBuf) {
        if !self.form.model.roots.contains(&root) {
            self.form.model.roots.push(root);
        }
    }

    /// Keys for the keybinding bar (derived from the Form).
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        self.form.bar()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.form.render(frame, area);
    }
}

impl SettingsForm {
    /// Index of the `[Add media root]` row.
    fn add_root_index(&self) -> usize {
        FIXED_FIELDS + self.roots.len()
    }

    /// Essentials still missing for a save, in display order. Empty == saveable.
    fn missing_essentials(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.settings.username.is_none() {
            missing.push("a username");
        }
        if self.settings.password.is_none() {
            missing.push("a password");
        }
        if self.roots.is_empty() {
            missing.push("a media root");
        }
        missing
    }

    /// Can the current working copy be saved? (username, password, ≥1 root)
    #[cfg(test)]
    fn can_save(&self) -> bool {
        self.missing_essentials().is_empty()
    }

    fn field_value(&self, index: usize) -> String {
        match index {
            FIELD_USERNAME => self.settings.username.clone().unwrap_or_default(),
            FIELD_SERVER => self.settings.server.clone(),
            FIELD_PASSWORD => self.settings.password.clone().unwrap_or_default(),
            FIELD_IRC_SERVER => self.settings.irc_server.clone(),
            FIELD_IRC_CHANNEL => self.settings.irc_channel.clone(),
            _ => String::new(),
        }
    }
}

impl FormModel for SettingsForm {
    type Out = Msg;

    fn title(&self) -> String {
        "Settings".to_string()
    }

    fn rows(&self) -> Vec<Line<'static>> {
        let mask = |s: &str| "*".repeat(s.chars().count());
        let yes_no = |b: bool| if b { "yes" } else { "no" };
        let mut lines: Vec<Line<'static>> = vec![
            format!("Username:  {}", self.field_value(FIELD_USERNAME)).into(),
            format!("Server:    {}", self.field_value(FIELD_SERVER)).into(),
            format!("Password:  {}", mask(&self.field_value(FIELD_PASSWORD))).into(),
            format!(
                "Ready on startup: {}",
                yes_no(self.settings.ready_on_startup)
            )
            .into(),
            format!("Subtitles: {}", self.settings.subtitle_mode.label()).into(),
            format!("Cache: {}", self.settings.cache_retention.label()).into(),
            format!("Auto-download: {}", yes_no(self.settings.auto_download)).into(),
            format!("IRC bridge: {}", yes_no(self.settings.irc_enabled)).into(),
            format!("IRC server:  {}", self.field_value(FIELD_IRC_SERVER)).into(),
            format!("IRC TLS:     {}", yes_no(self.settings.irc_tls)).into(),
            format!("IRC channel: {}", self.field_value(FIELD_IRC_CHANNEL)).into(),
        ];
        for (index, root) in self.roots.iter().enumerate() {
            let marker = if index == 0 { " (download target)" } else { "" };
            lines.push(Line::from(vec![
                Span::raw(format!("Media root: {}", root.display())),
                Span::styled(marker, theme::tone_style(super::props::Tone::Transfer)),
            ]));
        }
        lines.push(Line::from(Span::styled("[Add media root]", theme::dim())));
        lines
    }

    fn activate(&mut self, index: usize) -> RowAction<Msg> {
        match index {
            FIELD_USERNAME | FIELD_SERVER | FIELD_PASSWORD | FIELD_IRC_SERVER
            | FIELD_IRC_CHANNEL => RowAction::Edit {
                current: self.field_value(index),
            },
            FIELD_READY => {
                self.settings.ready_on_startup = !self.settings.ready_on_startup;
                RowAction::Handled
            }
            FIELD_IRC_ENABLED => {
                self.settings.irc_enabled = !self.settings.irc_enabled;
                RowAction::Handled
            }
            FIELD_IRC_TLS => {
                self.settings.irc_tls = !self.settings.irc_tls;
                RowAction::Handled
            }
            FIELD_SUBTITLE => {
                self.settings.subtitle_mode = self.settings.subtitle_mode.next();
                RowAction::Handled
            }
            FIELD_CACHE => {
                self.settings.cache_retention = self.settings.cache_retention.next();
                RowAction::Handled
            }
            FIELD_AUTO_DOWNLOAD => {
                self.settings.auto_download = !self.settings.auto_download;
                RowAction::Handled
            }
            index if index == self.add_root_index() => RowAction::Out(Msg::OpenDirPicker),
            _ => RowAction::Handled,
        }
    }

    fn commit(&mut self, index: usize, value: String) {
        let value = value.trim().to_string();
        match index {
            FIELD_USERNAME => {
                self.settings.username = (!value.is_empty()).then_some(value);
            }
            FIELD_SERVER if !value.is_empty() => self.settings.server = value,
            FIELD_PASSWORD => {
                self.settings.password = (!value.is_empty()).then_some(value);
            }
            FIELD_IRC_SERVER if !value.is_empty() => self.settings.irc_server = value,
            FIELD_IRC_CHANNEL if !value.is_empty() => self.settings.irc_channel = value,
            _ => {}
        }
    }

    /// `J`/`K` (and lowercase) reorder the selected media root, carrying the
    /// cursor with it; `d` removes it. Bare letters rather than
    /// Ctrl-J/Ctrl-K, which collide with control codes (Ctrl-J == LF) in
    /// terminals lacking the enhanced keyboard protocol.
    fn on_char(&mut self, index: usize, c: char) -> CharOutcome {
        if index < FIXED_FIELDS {
            return CharOutcome::Ignored;
        }
        let root = index - FIXED_FIELDS;
        match c {
            'j' | 'J' | 'k' | 'K' => {
                let down = matches!(c, 'j' | 'J');
                let target = if down { root + 1 } else { root.wrapping_sub(1) };
                if root < self.roots.len() && target < self.roots.len() {
                    self.roots.swap(root, target);
                    return CharOutcome::MoveTo(FIXED_FIELDS + target);
                }
                CharOutcome::Handled
            }
            'd' => {
                if root < self.roots.len() {
                    self.roots.remove(root);
                }
                CharOutcome::Handled
            }
            _ => CharOutcome::Ignored,
        }
    }

    fn enter_label(&self) -> &'static str {
        "Edit/Toggle"
    }

    fn extra_bar(&self) -> Vec<super::widgets::BarEntry> {
        vec![("d", "Remove root"), ("J/K", "Reorder")]
    }

    /// The "needs …" gate: drives both the save refusal and the `[Save]`
    /// row's hint, so a refused save explains itself.
    fn save_hint(&self) -> Option<String> {
        let missing = self.missing_essentials();
        (!missing.is_empty()).then(|| missing.join(", "))
    }

    fn save(&self) -> Msg {
        Msg::SettingsSaved(Box::new(self.settings.clone()), self.roots.clone())
    }
}

passive_modal!(SettingsModal);

impl AppComponent<Msg, NoUserEvent> for SettingsModal {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        match self.form.on(ev) {
            FormEvent::Handled => Some(Msg::None),
            FormEvent::Out(msg) => Some(msg),
            FormEvent::Cancelled => Some(Msg::CloseModal),
            FormEvent::Ignored => None,
        }
    }
}

// ---- Episode browser ---------------------------------------------------

/// One franchise member ("season").
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Season {
    /// Display title.
    pub title: String,
    /// Known files for this member.
    pub episodes: Vec<(Ed2kHash, String)>,
}

/// Browse a franchise's seasons and known episodes. Until Phases 8-9
/// fill in metadata and local files, this mostly shows structure.
pub struct EpisodeBrowser {
    title: String,
    seasons: Vec<Season>,
    /// `Some(index)` = episode view for that season.
    open: Option<usize>,
    cursor: ListCursor,
}

impl EpisodeBrowser {
    /// Open on a franchise. With exactly one season, jump straight to
    /// the episode list (the design's single-season shortcut).
    pub fn new(title: String, seasons: Vec<Season>) -> Self {
        let open = (seasons.len() == 1).then_some(0);
        Self {
            title,
            seasons,
            open,
            cursor: ListCursor::default(),
        }
    }

    /// Keys for the keybinding bar (derived from the keymap).
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        EPISODES_KEYMAP.bar()
    }

    /// Enter: open the selected season, or choose the selected episode
    /// (added to the playlist by hash — if we hold the file it resolves
    /// Ready; if not, it's added from the file catalog and downloads).
    fn act_enter(&mut self) -> Option<Msg> {
        match self.open {
            None => {
                if !self.seasons.is_empty() {
                    self.open = Some(self.cursor.index());
                    self.cursor.reset();
                }
                Some(Msg::None)
            }
            Some(index) => match self.seasons[index].episodes.get(self.cursor.index()) {
                Some((hash, _)) => Some(Msg::EpisodeChosen { hash: *hash }),
                None => Some(Msg::None),
            },
        }
    }

    /// Backspace: episodes -> seasons; from the seasons list, close.
    fn act_back(&mut self) -> Option<Msg> {
        if self.open.take().is_some() {
            self.cursor.reset();
            Some(Msg::None)
        } else {
            Some(Msg::CloseModal)
        }
    }

    fn act_close(&mut self) -> Option<Msg> {
        Some(Msg::CloseModal)
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let (title, items): (String, Vec<ListItem>) = match self.open {
            None => (
                self.title.clone(),
                self.seasons
                    .iter()
                    .map(|season| {
                        ListItem::new(format!(
                            "{} ({} known files)",
                            season.title,
                            season.episodes.len()
                        ))
                    })
                    .collect(),
            ),
            Some(index) => {
                let season = &self.seasons[index];
                (
                    format!("{} — {}", self.title, season.title),
                    if season.episodes.is_empty() {
                        vec![ListItem::new(Span::styled(
                            "no known files yet",
                            theme::dim(),
                        ))]
                    } else {
                        season
                            .episodes
                            .iter()
                            .map(|(_, name)| ListItem::new(name.clone()))
                            .collect()
                    },
                )
            }
        };
        render_modal_list(frame, area, &title, items, self.cursor.index());
    }

    fn len(&self) -> usize {
        match self.open {
            None => self.seasons.len(),
            Some(index) => self.seasons[index].episodes.len(),
        }
    }
}

passive_modal!(EpisodeBrowser);

impl AppComponent<Msg, NoUserEvent> for EpisodeBrowser {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some(key) = plain(ev)
            && self.cursor.nav(key, self.len())
        {
            return Some(Msg::None);
        }
        EPISODES_KEYMAP.dispatch(self, ev)
    }
}

/// Episode-browser bindings.
static EPISODES_KEYMAP: Keymap<EpisodeBrowser, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Open")),
        action: EpisodeBrowser::act_enter,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Backspace),
        bar: Some(("Bksp", "Back")),
        action: EpisodeBrowser::act_back,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: Some(("Esc", "Close")),
        action: EpisodeBrowser::act_close,
    },
]);

// ---- List entry editor -------------------------------------------------

const LIST_FIELDS: &[&str] = &[
    "Name",
    "Nero's name",
    "Genre",
    "Notes",
    "Recommender",
    "Status",
    "Status note",
    "Source",
    "Next ep",
    "Available",
];
const LIST_FIELD_STATUS: usize = 5;
const LIST_FIELD_NEXT_EP: usize = 8;
const LIST_FIELD_AVAILABLE: usize = 9;

/// Edit one List entry's fields (watchers are edited via import or a
/// later refinement): a [`Form`] over the entry plus its progress
/// register.
///
/// `next_ep`/`available` ([`NextEpState`]) live in a separate CRDT
/// register from the rest of the entry, so the server's EOF auto-advance
/// and a user's note edits never clobber each other. The form edits a
/// working copy and reports it on save only when it actually changed (see
/// `next_ep_change`), preserving that separation.
pub struct ListEditModal {
    form: Form<ListEditForm>,
}

/// The List-entry form model.
struct ListEditForm {
    /// The entry being edited.
    id: ListEntryId,
    entry: SeriesListEntry,
    /// Working copy of the progress register.
    next_ep: NextEpState,
    /// The progress register as loaded, for change detection on save.
    original_next_ep: NextEpState,
}

impl ListEditModal {
    /// Open on an entry plus its current progress register.
    pub fn new(id: ListEntryId, entry: SeriesListEntry, next_ep: NextEpState) -> Self {
        Self {
            form: Form::new(ListEditForm {
                id,
                entry,
                original_next_ep: next_ep.clone(),
                next_ep,
            }),
        }
    }

    /// Keys for the keybinding bar (derived from the Form).
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        self.form.bar()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.form.render(frame, area);
    }
}

impl ListEditForm {
    /// The edited progress register, or `None` if the user left both
    /// `next_ep` and `available` untouched (so saving an unrelated field
    /// never writes — and thus never clobbers — the shared register).
    fn next_ep_change(&self) -> Option<NextEpState> {
        (self.next_ep != self.original_next_ep).then(|| self.next_ep.clone())
    }

    fn field_value(&self, index: usize) -> String {
        match index {
            0 => self.entry.name.clone(),
            1 => self.entry.nero_name.clone().unwrap_or_default(),
            2 => self.entry.genre.clone().unwrap_or_default(),
            3 => self.entry.notes.join("; "),
            4 => self.entry.recommender.clone().unwrap_or_default(),
            5 => format!("{:?}", self.entry.status),
            6 => self.entry.status_note.clone().unwrap_or_default(),
            7 => self.entry.source.clone().unwrap_or_default(),
            LIST_FIELD_NEXT_EP => self.next_ep.next_ep.clone().unwrap_or_default(),
            LIST_FIELD_AVAILABLE => if self.next_ep.available { "yes" } else { "no" }.to_string(),
            _ => String::new(),
        }
    }

    fn cycle_status(&mut self) {
        use ListStatus::*;
        self.entry.status = match self.entry.status {
            ShortList => Planned,
            Planned => Active,
            Active => CurrentSeason,
            CurrentSeason => Waiting,
            Waiting => Hiatus,
            Hiatus => Finished,
            Finished => Dropped,
            Dropped => ShortList,
        };
    }
}

impl FormModel for ListEditForm {
    type Out = Msg;

    fn title(&self) -> String {
        format!("Edit — {}", self.entry.name)
    }

    fn rows(&self) -> Vec<Line<'static>> {
        LIST_FIELDS
            .iter()
            .enumerate()
            .map(|(index, label)| Line::raw(format!("{label:>12}: {}", self.field_value(index))))
            .collect()
    }

    fn activate(&mut self, index: usize) -> RowAction<Msg> {
        match index {
            LIST_FIELD_STATUS => {
                self.cycle_status();
                RowAction::Handled
            }
            LIST_FIELD_AVAILABLE => {
                self.next_ep.available = !self.next_ep.available;
                RowAction::Handled
            }
            index => RowAction::Edit {
                current: self.field_value(index),
            },
        }
    }

    fn commit(&mut self, index: usize, value: String) {
        let value = value.trim().to_string();
        let opt = (!value.is_empty()).then_some(value.clone());
        match index {
            0 if !value.is_empty() => self.entry.name = value,
            1 => self.entry.nero_name = opt,
            2 => self.entry.genre = opt,
            3 => {
                self.entry.notes = value
                    .split(';')
                    .map(|note| note.trim().to_string())
                    .filter(|note| !note.is_empty())
                    .collect();
            }
            4 => self.entry.recommender = opt,
            6 => self.entry.status_note = opt,
            7 => self.entry.source = opt,
            LIST_FIELD_NEXT_EP => self.next_ep.next_ep = opt,
            _ => {}
        }
    }

    fn save(&self) -> Msg {
        Msg::ListEntrySaved(
            self.id,
            Box::new(self.entry.clone()),
            self.next_ep_change().map(Box::new),
        )
    }

    fn enter_label(&self) -> &'static str {
        "Edit/Cycle"
    }

    fn overlay_percent(&self) -> (u16, u16) {
        (60, 60)
    }
}

/// Link a List entry to an AniDB series: type a (partial, informal)
/// name, Enter searches server-side over the titles dump, pick a
/// result, Enter links. Editing the query re-arms search.
pub struct AniDbSearchModal {
    /// The entry being linked.
    pub id: ListEntryId,
    entry_name: String,
    editor: FieldEditor,
    /// The query whose results are displayed (`None` = none yet).
    answered: Option<String>,
    results: Vec<AniDbSearchHit>,
    /// A search is in flight.
    searching: bool,
    cursor: ListCursor,
}

impl AniDbSearchModal {
    /// Open for an entry, prefilled with its name (the caller fires
    /// the initial search for it).
    pub fn new(id: ListEntryId, entry_name: String) -> Self {
        Self {
            id,
            editor: FieldEditor::new(&entry_name),
            entry_name,
            answered: None,
            results: Vec::new(),
            searching: true,
            cursor: ListCursor::default(),
        }
    }

    /// Deliver search results. Stale replies (for a query that is no
    /// longer the editor's text) are dropped.
    pub fn set_results(&mut self, query: &str, results: Vec<AniDbSearchHit>) {
        if query != self.editor.text() {
            return;
        }
        self.answered = Some(query.to_string());
        self.results = results;
        self.searching = false;
        self.cursor.reset();
    }

    /// Keys for the keybinding bar: the keymap's entries with the
    /// structural results-navigation entry slotted after Enter.
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        let mut items = SEARCH_KEYMAP.bar();
        items.insert(1, ("↑↓", "Pick"));
        items
    }

    /// Enter on fresh results links; otherwise it (re-)searches.
    fn act_enter(&mut self) -> Option<Msg> {
        let query = self.editor.text();
        if self.answered.as_deref() == Some(query.as_str())
            && let Some(hit) = self.results.get(self.cursor.index())
        {
            return Some(Msg::ListEntryLinked(self.id, hit.series));
        }
        if query.trim().is_empty() {
            return Some(Msg::None);
        }
        self.searching = true;
        self.answered = None;
        Some(Msg::AniDbSearchRequested(query))
    }

    fn act_close(&mut self) -> Option<Msg> {
        Some(Msg::CloseModal)
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let modal = overlay(area, 60, 60);
        frame.render_widget(Clear, modal);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border_style(true))
                .title(format!("Link to AniDB — {}", self.entry_name)),
            modal,
        );
        let input_area = Rect {
            x: modal.x + 2,
            y: modal.y + 1,
            width: modal.width.saturating_sub(4),
            height: 3,
        };
        self.editor.view(frame, input_area);

        let list_area = Rect {
            x: modal.x + 2,
            y: modal.y + 4,
            width: modal.width.saturating_sub(4),
            height: modal.height.saturating_sub(5),
        };
        if self.searching {
            frame.render_widget(
                tuirealm::ratatui::widgets::Paragraph::new("searching…"),
                list_area,
            );
            return;
        }
        if self.answered.is_some() && self.results.is_empty() {
            frame.render_widget(
                tuirealm::ratatui::widgets::Paragraph::new("no matches"),
                list_area,
            );
            return;
        }
        let items: Vec<ListItem> = self
            .results
            .iter()
            .map(|hit| {
                let mut spans = vec![Span::raw(hit.title.clone())];
                if hit.matched != hit.title {
                    spans.push(Span::styled(format!("  ({})", hit.matched), theme::dim()));
                }
                spans.push(Span::styled(format!("  a{}", hit.series.0), theme::dim()));
                ListItem::new(Line::from(spans))
            })
            .collect();
        let mut state = ListState::default();
        if !self.results.is_empty() {
            state.select(Some(self.cursor.index()));
        }
        frame.render_stateful_widget(
            List::new(items).highlight_style(theme::highlight_style()),
            list_area,
            &mut state,
        );
    }
}

passive_modal!(AniDbSearchModal);

impl AppComponent<Msg, NoUserEvent> for AniDbSearchModal {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some(key) = plain(ev)
            && self.cursor.nav(key, self.results.len())
        {
            return Some(Msg::None);
        }
        if let Some(msg) = SEARCH_KEYMAP.dispatch(self, ev) {
            return Some(msg);
        }
        // Everything else edits the query (which stales any results).
        self.editor.on(ev);
        Some(Msg::None)
    }
}

/// AniDB-search bindings (results navigation is the structural
/// ListCursor; query editing is the fall-through).
static SEARCH_KEYMAP: Keymap<AniDbSearchModal, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Search/Link")),
        action: AniDbSearchModal::act_enter,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: Some(("Esc", "Cancel")),
        action: AniDbSearchModal::act_close,
    },
]);

passive_modal!(ListEditModal);

impl AppComponent<Msg, NoUserEvent> for ListEditModal {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        match self.form.on(ev) {
            FormEvent::Handled => Some(Msg::None),
            FormEvent::Out(msg) => Some(msg),
            FormEvent::Cancelled => Some(Msg::CloseModal),
            FormEvent::Ignored => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use tuirealm::event::{KeyEvent, KeyModifiers};

    use super::*;

    fn enter() -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn down() -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn enter_cycles_cache_retention_field() {
        let mut modal = SettingsModal::new(crate::config::Settings::default(), vec![]);
        // Default retention is one week.
        assert_eq!(
            modal.form.model.settings.cache_retention,
            crate::config::CacheRetention::default()
        );
        // Move the cursor onto the Cache field and cycle it.
        for _ in 0..FIELD_CACHE {
            modal.on(&down());
        }
        modal.on(&enter());
        assert_eq!(
            modal.form.model.settings.cache_retention,
            crate::config::CacheRetention::default().next()
        );
    }

    #[test]
    fn enter_opens_season_then_chooses_episode() {
        let seasons = vec![
            Season {
                title: "S1".into(),
                episodes: vec![(hash(1), "ep1".into())],
            },
            Season {
                title: "S2".into(),
                episodes: vec![(hash(2), "ep2".into())],
            },
        ];
        let mut browser = EpisodeBrowser::new("Frieren".into(), seasons);
        // Two seasons: starts on the season list; Enter opens season 0.
        assert_eq!(browser.on(&enter()), Some(Msg::None));
        // Now on the episode list; Enter chooses the selected episode.
        assert_eq!(
            browser.on(&enter()),
            Some(Msg::EpisodeChosen { hash: hash(1) })
        );
    }

    #[test]
    fn directory_picker_prepends_select_and_parent() {
        // The media-root picker surfaces "[Select]" and ".." as the first
        // two rows, with the cursor on "[Select]".
        let browser = FileBrowser::for_directory();
        assert_eq!(browser.cursor.index(), 0);
        assert!(matches!(browser.entries[0].kind, RowKind::Select));
        assert_eq!(browser.entries[0].name, "[Select]");
        assert!(matches!(browser.entries[1].kind, RowKind::Parent));
        assert_eq!(browser.entries[1].name, "..");
        // Enter on "[Select]" confirms the current directory.
        let mut browser = FileBrowser::for_directory();
        assert!(matches!(browser.on(&enter()), Some(Msg::DirChosen(_))));
    }

    #[test]
    fn directory_picker_ascends_above_home() {
        // Regression: the media-root picker must reach directories outside
        // $HOME, so ".." walks up to the real filesystem parent rather than
        // stopping at the (home-only) roots boundary.
        let mut browser = FileBrowser::for_directory();
        let home = dirs::home_dir().unwrap();
        assert_eq!(browser.cwd.as_deref(), Some(home.as_path()));
        assert!(browser.ascend());
        assert_eq!(browser.cwd.as_deref(), home.parent());
    }

    #[cfg(unix)]
    #[test]
    fn directory_picker_follows_symlinked_directories() {
        // A symlink to a directory must list as a navigable directory, not a
        // dead non-dir row.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("real")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("link")).unwrap();

        let browser = FileBrowser::for_mapping(
            vec![tmp.path().to_path_buf()],
            hash(1),
            "target.mkv".into(),
            Some(tmp.path().to_path_buf()),
        );
        let link = browser.entries.iter().find(|r| r.name == "link").unwrap();
        assert!(link.is_dir, "symlinked directory must list as a directory");
    }

    #[test]
    fn single_season_chooses_episode_directly() {
        let seasons = vec![Season {
            title: "S1".into(),
            episodes: vec![(hash(7), "ep".into())],
        }];
        let mut browser = EpisodeBrowser::new("X".into(), seasons);
        // Single-season shortcut opens episodes immediately; Enter chooses.
        assert_eq!(
            browser.on(&enter()),
            Some(Msg::EpisodeChosen { hash: hash(7) })
        );
    }

    /// A SettingsModal with all the essentials filled in (saveable).
    fn saveable_settings() -> SettingsModal {
        let settings = Settings {
            username: Some("nero".into()),
            password: Some("hunter2".into()),
            ..Default::default()
        };
        SettingsModal::new(settings, vec![PathBuf::from("/anime")])
    }

    fn key(code: Key, modifiers: KeyModifiers) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent { code, modifiers })
    }

    fn is_save(msg: &Option<Msg>) -> bool {
        matches!(msg, Some(Msg::SettingsSaved(..)))
    }

    #[test]
    fn capital_s_saves_when_essentials_present() {
        // Capital `S` carries SHIFT and must save — the terminal-safe path
        // that replaces the Ctrl-S == XOFF trap.
        let mut modal = saveable_settings();
        assert!(is_save(
            &modal.on(&key(Key::Char('S'), KeyModifiers::SHIFT))
        ));
    }

    #[test]
    fn ctrl_s_still_saves() {
        // Ctrl-S is retained as an alias for terminals where it survives.
        let mut modal = saveable_settings();
        assert!(is_save(
            &modal.on(&key(Key::Char('s'), KeyModifiers::CONTROL))
        ));
    }

    #[test]
    fn enter_on_save_row_saves() {
        let mut modal = saveable_settings();
        modal.form.select(modal.form.save_index());
        assert!(is_save(&modal.on(&enter())));
    }

    #[test]
    fn missing_essentials_lists_each_gap() {
        // A blank modal is missing all three; the hint names them in order.
        let blank = SettingsModal::new(Settings::default(), vec![]);
        assert_eq!(
            blank.form.model.missing_essentials(),
            vec!["a username", "a password", "a media root"]
        );
        // Fill username + root; only the password remains.
        let settings = Settings {
            username: Some("nero".into()),
            ..Default::default()
        };
        let partial = SettingsModal::new(settings, vec![PathBuf::from("/anime")]);
        assert_eq!(partial.form.model.missing_essentials(), vec!["a password"]);
        // Fully populated: nothing missing, saveable.
        assert!(
            saveable_settings()
                .form
                .model
                .missing_essentials()
                .is_empty()
        );
    }

    fn sample_list_entry() -> SeriesListEntry {
        SeriesListEntry {
            name: "Frieren".into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status: ListStatus::Active,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
        }
    }

    #[test]
    fn list_edit_modal_saves_via_capital_s_and_save_row() {
        // Regression: the List edit modal must have a save path that does not
        // rely on Ctrl-S (eaten as XOFF on terminals without the enhanced
        // keyboard protocol), mirroring the SettingsModal. Capital `S`, the
        // `[Save]` row, and Ctrl-s (alias) all save.
        let mut modal =
            ListEditModal::new(ListEntryId(7), sample_list_entry(), NextEpState::default());
        assert!(matches!(
            modal.on(&key(Key::Char('S'), KeyModifiers::SHIFT)),
            Some(Msg::ListEntrySaved(ListEntryId(7), _, _))
        ));

        // The `[Save]` row (after the fields) saves on Enter.
        let mut modal =
            ListEditModal::new(ListEntryId(7), sample_list_entry(), NextEpState::default());
        modal.form.select(modal.form.save_index());
        assert!(matches!(
            modal.on(&enter()),
            Some(Msg::ListEntrySaved(ListEntryId(7), _, _))
        ));

        // Ctrl-s is retained as a working alias.
        let mut modal =
            ListEditModal::new(ListEntryId(7), sample_list_entry(), NextEpState::default());
        assert!(matches!(
            modal.on(&key(Key::Char('s'), KeyModifiers::CONTROL)),
            Some(Msg::ListEntrySaved(..))
        ));
    }

    #[test]
    fn media_roots_reorder_with_shift_jk() {
        // design.md and the code agree: media roots reorder with `J`/`K` (and
        // lowercase `j`/`k`), NOT Ctrl-J/Ctrl-K (which collide with LF in
        // terminals lacking the enhanced keyboard protocol). The cursor
        // follows the moved root, mirroring the playlist pane.
        let settings = Settings {
            username: Some("nero".into()),
            password: Some("hunter2".into()),
            ..Default::default()
        };
        let mut modal =
            SettingsModal::new(settings, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        // Put the cursor on the first media root.
        modal.form.select(FIXED_FIELDS);
        // `J` (shifted) moves it down; the cursor carries with it.
        assert_eq!(
            modal.on(&key(Key::Char('J'), KeyModifiers::SHIFT)),
            Some(Msg::None)
        );
        assert_eq!(
            modal.form.model.roots,
            vec![PathBuf::from("/b"), PathBuf::from("/a")]
        );
        assert_eq!(modal.form.selected(), FIXED_FIELDS + 1);
        // Lowercase `k` moves it back up.
        assert_eq!(
            modal.on(&key(Key::Char('k'), KeyModifiers::NONE)),
            Some(Msg::None)
        );
        assert_eq!(
            modal.form.model.roots,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert_eq!(modal.form.selected(), FIXED_FIELDS);
    }

    #[test]
    fn overlay_does_not_overflow_on_a_very_wide_terminal() {
        // Regression: the percent multiply must be widened past u16 before
        // dividing. On a very wide/tall terminal `area.width * percent / 100`
        // overflows u16 (panic in debug, garbage rect in release) — e.g.
        // 2000 cols * 70 = 140000 > u16::MAX. The result must stay clamped to
        // the frame.
        let area = Rect::new(0, 0, 2000, 2000);
        let rect = overlay(area, 70, 70);
        assert_eq!(rect.width, 1400);
        assert_eq!(rect.height, 1400);
        assert!(rect.width <= area.width && rect.height <= area.height);
        // Centered within the frame.
        assert_eq!(rect.x, (area.width - rect.width) / 2);
        assert_eq!(rect.y, (area.height - rect.height) / 2);
    }

    #[test]
    fn save_blocked_without_essentials() {
        // Missing password: every save path no-ops rather than emitting a save.
        let settings = Settings {
            username: Some("nero".into()),
            ..Default::default()
        };
        let mut modal = SettingsModal::new(settings, vec![PathBuf::from("/anime")]);
        let save_row = modal.form.save_index();
        assert!(!modal.form.model.can_save());
        assert_eq!(
            modal.on(&key(Key::Char('S'), KeyModifiers::SHIFT)),
            Some(Msg::None)
        );
        assert_eq!(
            modal.on(&key(Key::Char('s'), KeyModifiers::CONTROL)),
            Some(Msg::None)
        );
        modal.form.select(save_row);
        assert_eq!(modal.on(&enter()), Some(Msg::None));
    }
}
