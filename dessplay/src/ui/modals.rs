//! Modal components: centered overlays that capture input while open
//! (ui-architecture.md, Modals). The dispatcher keeps a modal stack;
//! the background keeps rendering and the event loop keeps running.

use std::path::{Path, PathBuf};

use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};
use tuirealm::state::{State, StateValue};

use dessplay_core::net::AniDbSearchHit;
use dessplay_core::types::{Ed2kHash, ListEntryId, ListStatus, SeriesListEntry};

use super::components::{ctrl, plain, step_by, typed, LIST_PAGE_STEP};
use super::msg::Msg;
use super::theme;
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

/// The centered overlay area: `percent` of the frame, clamped.
pub fn overlay(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let width = (area.width * percent_x / 100).max(20).min(area.width);
    let height = (area.height * percent_y / 100).max(8).min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// A one-line text editor for modal fields, backed by the stdlib Input.
struct FieldEditor {
    input: tui_realm_stdlib::components::Input,
}

impl FieldEditor {
    fn new(initial: &str) -> Self {
        let mut input = tui_realm_stdlib::components::Input::default()
            .borders(tuirealm::props::Borders::default());
        input.attr(Attribute::Value, AttrValue::String(initial.to_string()));
        input.attr(Attribute::Focus, AttrValue::Flag(true));
        Self { input }
    }

    fn text(&self) -> String {
        match self.input.state() {
            State::Single(StateValue::String(text)) => text,
            _ => String::new(),
        }
    }

    /// Feed an event; `Some(committed)` when editing ends.
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<bool> {
        if let Some(c) = typed(ev) {
            let _ = self.input.perform(Cmd::Type(c));
            return None;
        }
        match plain(ev)? {
            Key::Enter => Some(true),
            Key::Esc => Some(false),
            Key::Backspace => {
                let _ = self.input.perform(Cmd::Delete);
                None
            }
            Key::Delete => {
                let _ = self.input.perform(Cmd::Cancel);
                None
            }
            Key::Left => {
                let _ = self.input.perform(Cmd::Move(Direction::Left));
                None
            }
            Key::Right => {
                let _ = self.input.perform(Cmd::Move(Direction::Right));
                None
            }
            Key::Home => {
                let _ = self.input.perform(Cmd::GoTo(Position::Begin));
                None
            }
            Key::End => {
                let _ = self.input.perform(Cmd::GoTo(Position::End));
                None
            }
            _ => None,
        }
    }

    fn view(&mut self, frame: &mut Frame, area: Rect) {
        self.input.view(frame, area);
    }
}

/// Render a selectable list as a modal overlay.
fn render_modal_list(frame: &mut Frame, area: Rect, title: &str, items: Vec<ListItem>, sel: usize) {
    let area = overlay(area, 70, 70);
    frame.render_widget(Clear, area);
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(sel));
    }
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(theme::highlight_style())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border_style(true))
                    .title(title.to_string()),
            ),
        area,
        &mut state,
    );
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
    sel: usize,
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
            sel: 0,
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
            sel: 0,
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
            sel: 0,
        };
        browser.refresh();
        browser
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.sel = 0;
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

    /// Keys for the keybinding bar.
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        match self.purpose {
            BrowseFor::File => vec![("Enter", "Open/Add"), ("Bksp", "Up"), ("Esc", "Cancel")],
            BrowseFor::Map { .. } => {
                vec![("Enter", "Open/Map"), ("Bksp", "Up"), ("Esc", "Cancel")]
            }
            BrowseFor::Directory => {
                vec![
                    ("Enter", "Open"),
                    ("s", "Select here"),
                    ("Bksp", "Up"),
                    ("Esc", "Cancel"),
                ]
            }
        }
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
        render_modal_list(frame, area, &title, items, self.sel);
    }
}

passive_modal!(FileBrowser);

impl AppComponent<Msg, NoUserEvent> for FileBrowser {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        match plain(ev)? {
            Key::Up => {
                self.sel = step_by(self.sel, self.entries.len(), false, 1);
                Some(Msg::None)
            }
            Key::Down => {
                self.sel = step_by(self.sel, self.entries.len(), true, 1);
                Some(Msg::None)
            }
            Key::PageUp => {
                self.sel = step_by(self.sel, self.entries.len(), false, LIST_PAGE_STEP);
                Some(Msg::None)
            }
            Key::PageDown => {
                self.sel = step_by(self.sel, self.entries.len(), true, LIST_PAGE_STEP);
                Some(Msg::None)
            }
            Key::Enter => {
                let row = self.entries.get(self.sel)?;
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
            Key::Char('s') if matches!(self.purpose, BrowseFor::Directory) => {
                self.cwd.clone().map(Msg::DirChosen)
            }
            Key::Backspace => {
                if self.ascend() {
                    Some(Msg::None)
                } else {
                    Some(Msg::CloseModal)
                }
            }
            Key::Esc => Some(Msg::CloseModal),
            _ => None,
        }
    }
}

// ---- Settings ----------------------------------------------------------

/// Field index layout: fixed fields then one row per media root, then
/// the add-root row.
const FIELD_USERNAME: usize = 0;
const FIELD_SERVER: usize = 1;
const FIELD_PASSWORD: usize = 2;
const FIELD_READY: usize = 3;
const FIELD_SUBTITLE: usize = 4;
const FIELD_CACHE: usize = 5;
const FIXED_FIELDS: usize = 6;

/// First-run and later settings editing.
pub struct SettingsModal {
    /// The working copy.
    pub settings: Settings,
    /// Working media roots (position 0 is the download target).
    pub roots: Vec<PathBuf>,
    sel: usize,
    editor: Option<(usize, FieldEditor)>,
}

impl SettingsModal {
    /// Open with current values.
    pub fn new(settings: Settings, roots: Vec<PathBuf>) -> Self {
        Self {
            settings,
            roots,
            sel: 0,
            editor: None,
        }
    }

    /// A directory was picked for a new media root.
    pub fn add_root(&mut self, root: PathBuf) {
        if !self.roots.contains(&root) {
            self.roots.push(root);
        }
    }

    fn field_count(&self) -> usize {
        FIXED_FIELDS + self.roots.len() + 2
    }

    /// Index of the `[Add media root]` row.
    fn add_root_index(&self) -> usize {
        FIXED_FIELDS + self.roots.len()
    }

    /// Index of the `[Save]` row (the last row).
    fn save_index(&self) -> usize {
        FIXED_FIELDS + self.roots.len() + 1
    }

    /// Essentials still missing for a save, in display order. Empty == saveable.
    /// Drives both the save gate and the `[Save]` row's "needs …" hint, so the
    /// UI explains *why* it can't save rather than silently refusing.
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
    fn can_save(&self) -> bool {
        self.missing_essentials().is_empty()
    }

    /// Build the save message if the essentials are present, else `None`.
    fn try_save(&self) -> Option<Msg> {
        self.can_save().then(|| {
            Msg::SettingsSaved(Box::new(self.settings.clone()), self.roots.clone())
        })
    }

    fn field_value(&self, index: usize) -> String {
        match index {
            FIELD_USERNAME => self.settings.username.clone().unwrap_or_default(),
            FIELD_SERVER => self.settings.server.clone(),
            FIELD_PASSWORD => self.settings.password.clone().unwrap_or_default(),
            _ => String::new(),
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
            _ => {}
        }
    }

    /// Keys for the keybinding bar.
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("Enter", "Edit/Toggle"),
            ("d", "Remove root"),
            ("Ctrl-j/k", "Reorder"),
            ("S", "Save"),
            ("Esc", "Cancel"),
        ]
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let modal = overlay(area, 70, 70);
        frame.render_widget(Clear, modal);
        let mut lines: Vec<ListItem> = Vec::new();
        let mask = |s: &str| "*".repeat(s.chars().count());
        let rows: Vec<String> = vec![
            format!("Username:  {}", self.field_value(FIELD_USERNAME)),
            format!("Server:    {}", self.field_value(FIELD_SERVER)),
            format!("Password:  {}", mask(&self.field_value(FIELD_PASSWORD))),
            format!(
                "Ready on startup: {}",
                if self.settings.ready_on_startup {
                    "yes"
                } else {
                    "no"
                }
            ),
            format!("Subtitles: {}", self.settings.subtitle_mode.label()),
            format!("Cache: {}", self.settings.cache_retention.label()),
        ];
        for row in rows {
            lines.push(ListItem::new(row));
        }
        for (index, root) in self.roots.iter().enumerate() {
            let marker = if index == 0 { " (download target)" } else { "" };
            lines.push(ListItem::new(Line::from(vec![
                Span::raw(format!("Media root: {}", root.display())),
                Span::styled(marker, theme::tone_style(super::props::Tone::Transfer)),
            ])));
        }
        lines.push(ListItem::new(Span::styled(
            "[Add media root]",
            theme::dim(),
        )));
        // Saving needs no Ctrl combo: a plain `[Save]` row, alongside the
        // capital-`S` key (avoids the Ctrl-S == XOFF terminal trap). When the
        // essentials are missing the row is dim and spells out *what* is
        // needed, so a refused save explains itself instead of doing nothing.
        // The List's own highlight handles the selection cursor, so the
        // enabled row is just normal text.
        let missing = self.missing_essentials();
        let (save_label, save_style) = if missing.is_empty() {
            ("[Save]".to_string(), tuirealm::props::Style::default())
        } else {
            (format!("[Save] — needs {}", missing.join(", ")), theme::dim())
        };
        lines.push(ListItem::new(Span::styled(save_label, save_style)));

        let mut state = ListState::default();
        state.select(Some(self.sel));
        frame.render_stateful_widget(
            List::new(lines)
                .highlight_style(theme::highlight_style())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme::border_style(true))
                        .title("Settings"),
                ),
            modal,
            &mut state,
        );
        if let Some((_, editor)) = &mut self.editor {
            let edit_area = Rect {
                x: modal.x + 2,
                y: modal.y + modal.height.saturating_sub(4),
                width: modal.width.saturating_sub(4),
                height: 3,
            };
            frame.render_widget(Clear, edit_area);
            editor.view(frame, edit_area);
        }
    }
}

passive_modal!(SettingsModal);

impl AppComponent<Msg, NoUserEvent> for SettingsModal {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        // An active text editor swallows everything.
        if let Some((index, editor)) = &mut self.editor {
            if let Some(commit) = editor.on(ev) {
                let (index, editor) = (*index, self.editor.take()?.1);
                if commit {
                    self.commit(index, editor.text());
                }
            }
            return Some(Msg::None);
        }
        if let Some(code) = ctrl(ev) {
            match code {
                // Ctrl-S is kept as an alias for terminals where it isn't
                // eaten as XOFF; capital `S` and the `[Save]` row are the
                // reliable paths.
                Key::Char('s') => return Some(self.try_save().unwrap_or(Msg::None)),
                Key::Char('j') | Key::Char('k') if self.sel >= FIXED_FIELDS => {
                    let index = self.sel - FIXED_FIELDS;
                    let down = code == Key::Char('j');
                    let target = if down {
                        index + 1
                    } else {
                        index.wrapping_sub(1)
                    };
                    if index < self.roots.len() && target < self.roots.len() {
                        self.roots.swap(index, target);
                        self.sel = FIXED_FIELDS + target;
                    }
                    return Some(Msg::None);
                }
                _ => return None,
            }
        }
        // Capital `S` saves. `typed` is the only helper that sees a shifted
        // char; `plain` below requires no modifiers, so this must come first.
        if let Some('S') = typed(ev) {
            return Some(self.try_save().unwrap_or(Msg::None));
        }
        match plain(ev)? {
            Key::Up => {
                self.sel = self.sel.saturating_sub(1);
                Some(Msg::None)
            }
            Key::Down => {
                self.sel = (self.sel + 1).min(self.field_count() - 1);
                Some(Msg::None)
            }
            Key::Enter => {
                match self.sel {
                    FIELD_USERNAME | FIELD_SERVER | FIELD_PASSWORD => {
                        self.editor =
                            Some((self.sel, FieldEditor::new(&self.field_value(self.sel))));
                    }
                    FIELD_READY => {
                        self.settings.ready_on_startup = !self.settings.ready_on_startup;
                    }
                    FIELD_SUBTITLE => {
                        self.settings.subtitle_mode = self.settings.subtitle_mode.next();
                    }
                    FIELD_CACHE => {
                        self.settings.cache_retention = self.settings.cache_retention.next();
                    }
                    index if index == self.add_root_index() => {
                        return Some(Msg::OpenDirPicker);
                    }
                    index if index == self.save_index() => {
                        return Some(self.try_save().unwrap_or(Msg::None));
                    }
                    _ => {}
                }
                Some(Msg::None)
            }
            Key::Char('d') if self.sel >= FIXED_FIELDS => {
                let index = self.sel - FIXED_FIELDS;
                if index < self.roots.len() {
                    self.roots.remove(index);
                    self.sel = self.sel.min(self.field_count() - 1);
                }
                Some(Msg::None)
            }
            Key::Esc => Some(Msg::CloseModal),
            _ => None,
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
    sel: usize,
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
            sel: 0,
        }
    }

    /// Keys for the keybinding bar.
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        vec![("Enter", "Open"), ("Bksp", "Back"), ("Esc", "Close")]
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
        render_modal_list(frame, area, &title, items, self.sel);
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
        match plain(ev)? {
            Key::Up => {
                self.sel = step_by(self.sel, self.len(), false, 1);
                Some(Msg::None)
            }
            Key::Down => {
                self.sel = step_by(self.sel, self.len(), true, 1);
                Some(Msg::None)
            }
            Key::PageUp => {
                self.sel = step_by(self.sel, self.len(), false, LIST_PAGE_STEP);
                Some(Msg::None)
            }
            Key::PageDown => {
                self.sel = step_by(self.sel, self.len(), true, LIST_PAGE_STEP);
                Some(Msg::None)
            }
            Key::Enter => {
                match self.open {
                    // On the season list: open the selected season.
                    None => {
                        if !self.seasons.is_empty() {
                            self.open = Some(self.sel);
                            self.sel = 0;
                        }
                        Some(Msg::None)
                    }
                    // On an episode: add it to the playlist by hash. If we
                    // hold the file it resolves Ready; if not, it's added
                    // from the file catalog and downloads.
                    Some(index) => match self.seasons[index].episodes.get(self.sel) {
                        Some((hash, _)) => Some(Msg::EpisodeChosen { hash: *hash }),
                        None => Some(Msg::None),
                    },
                }
            }
            Key::Backspace => {
                if self.open.take().is_some() {
                    self.sel = 0;
                    Some(Msg::None)
                } else {
                    Some(Msg::CloseModal)
                }
            }
            Key::Esc => Some(Msg::CloseModal),
            _ => None,
        }
    }
}

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
];
const LIST_FIELD_STATUS: usize = 5;

/// Edit one List entry's fields (watchers are edited via import or a
/// later refinement).
pub struct ListEditModal {
    /// The entry being edited.
    pub id: ListEntryId,
    entry: SeriesListEntry,
    sel: usize,
    editor: Option<(usize, FieldEditor)>,
}

impl ListEditModal {
    /// Open on an entry.
    pub fn new(id: ListEntryId, entry: SeriesListEntry) -> Self {
        Self {
            id,
            entry,
            sel: 0,
            editor: None,
        }
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
            _ => String::new(),
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
            _ => {}
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

    /// Keys for the keybinding bar.
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        vec![("Enter", "Edit/Cycle"), ("Ctrl-s", "Save"), ("Esc", "Cancel")]
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let modal = overlay(area, 60, 60);
        frame.render_widget(Clear, modal);
        let items: Vec<ListItem> = LIST_FIELDS
            .iter()
            .enumerate()
            .map(|(index, label)| {
                ListItem::new(format!("{label:>12}: {}", self.field_value(index)))
            })
            .collect();
        let mut state = ListState::default();
        state.select(Some(self.sel));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(theme::highlight_style())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme::border_style(true))
                        .title(format!("Edit — {}", self.entry.name)),
                ),
            modal,
            &mut state,
        );
        if let Some((_, editor)) = &mut self.editor {
            let edit_area = Rect {
                x: modal.x + 2,
                y: modal.y + modal.height.saturating_sub(4),
                width: modal.width.saturating_sub(4),
                height: 3,
            };
            frame.render_widget(Clear, edit_area);
            editor.view(frame, edit_area);
        }
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
    sel: usize,
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
            sel: 0,
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
        self.sel = 0;
    }

    /// Keys for the keybinding bar.
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        vec![("Enter", "Search/Link"), ("↑↓", "Pick"), ("Esc", "Cancel")]
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
            state.select(Some(self.sel));
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
        match plain(ev) {
            Some(Key::Esc) => return Some(Msg::CloseModal),
            Some(Key::Up) => {
                self.sel = self.sel.saturating_sub(1);
                return Some(Msg::None);
            }
            Some(Key::Down) => {
                self.sel = (self.sel + 1).min(self.results.len().saturating_sub(1));
                return Some(Msg::None);
            }
            Some(Key::Enter) => {
                let query = self.editor.text();
                // Enter on fresh results links; otherwise it searches.
                if self.answered.as_deref() == Some(query.as_str())
                    && let Some(hit) = self.results.get(self.sel)
                {
                    return Some(Msg::ListEntryLinked(self.id, hit.series));
                }
                if query.trim().is_empty() {
                    return Some(Msg::None);
                }
                self.searching = true;
                self.answered = None;
                return Some(Msg::AniDbSearchRequested(query));
            }
            _ => {}
        }
        // Everything else edits the query (which stales any results).
        self.editor.on(ev);
        Some(Msg::None)
    }
}

passive_modal!(ListEditModal);

impl AppComponent<Msg, NoUserEvent> for ListEditModal {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some((index, editor)) = &mut self.editor {
            if let Some(commit) = editor.on(ev) {
                let (index, editor) = (*index, self.editor.take()?.1);
                if commit {
                    self.commit(index, editor.text());
                }
            }
            return Some(Msg::None);
        }
        if ctrl(ev) == Some(Key::Char('s')) {
            return Some(Msg::ListEntrySaved(self.id, Box::new(self.entry.clone())));
        }
        match plain(ev)? {
            Key::Up => {
                self.sel = self.sel.saturating_sub(1);
                Some(Msg::None)
            }
            Key::Down => {
                self.sel = (self.sel + 1).min(LIST_FIELDS.len() - 1);
                Some(Msg::None)
            }
            Key::Enter => {
                if self.sel == LIST_FIELD_STATUS {
                    self.cycle_status();
                } else {
                    self.editor = Some((self.sel, FieldEditor::new(&self.field_value(self.sel))));
                }
                Some(Msg::None)
            }
            Key::Esc => Some(Msg::CloseModal),
            _ => None,
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
            modal.settings.cache_retention,
            crate::config::CacheRetention::default()
        );
        // Move the cursor onto the Cache field and cycle it.
        for _ in 0..FIELD_CACHE {
            modal.on(&down());
        }
        modal.on(&enter());
        assert_eq!(
            modal.settings.cache_retention,
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
        assert_eq!(browser.sel, 0);
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
        let link = browser
            .entries
            .iter()
            .find(|r| r.name == "link")
            .unwrap();
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
        let mut settings = Settings::default();
        settings.username = Some("nero".into());
        settings.password = Some("hunter2".into());
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
        assert!(is_save(&modal.on(&key(Key::Char('S'), KeyModifiers::SHIFT))));
    }

    #[test]
    fn ctrl_s_still_saves() {
        // Ctrl-S is retained as an alias for terminals where it survives.
        let mut modal = saveable_settings();
        assert!(is_save(&modal.on(&key(Key::Char('s'), KeyModifiers::CONTROL))));
    }

    #[test]
    fn enter_on_save_row_saves() {
        let mut modal = saveable_settings();
        modal.sel = modal.save_index();
        assert!(is_save(&modal.on(&enter())));
    }

    #[test]
    fn missing_essentials_lists_each_gap() {
        // A blank modal is missing all three; the hint names them in order.
        let blank = SettingsModal::new(Settings::default(), vec![]);
        assert_eq!(
            blank.missing_essentials(),
            vec!["a username", "a password", "a media root"]
        );
        // Fill username + root; only the password remains.
        let mut settings = Settings::default();
        settings.username = Some("nero".into());
        let partial = SettingsModal::new(settings, vec![PathBuf::from("/anime")]);
        assert_eq!(partial.missing_essentials(), vec!["a password"]);
        // Fully populated: nothing missing, saveable.
        assert!(saveable_settings().missing_essentials().is_empty());
    }

    #[test]
    fn save_blocked_without_essentials() {
        // Missing password: every save path no-ops rather than emitting a save.
        let mut settings = Settings::default();
        settings.username = Some("nero".into());
        let mut modal = SettingsModal::new(settings, vec![PathBuf::from("/anime")]);
        let save_row = modal.save_index();
        assert!(!modal.can_save());
        assert_eq!(modal.on(&key(Key::Char('S'), KeyModifiers::SHIFT)), Some(Msg::None));
        assert_eq!(modal.on(&key(Key::Char('s'), KeyModifiers::CONTROL)), Some(Msg::None));
        modal.sel = save_row;
        assert_eq!(modal.on(&enter()), Some(Msg::None));
    }
}
