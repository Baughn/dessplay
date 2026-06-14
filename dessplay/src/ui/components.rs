//! The main panes: hand-rendered ratatui widgets behind tui-realm's
//! `Component`/`AppComponent` traits, driven by typed props from
//! [`super::props`]. (We use the component model and the stdlib's
//! `Input`, but not tui-realm's threaded event listener — see
//! ui-architecture.md, Framework Choice.)

use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use tuirealm::state::{State, StateValue};

use super::msg::Msg;
use super::props::{
    ChatLine, FranchiseRow, ListGroup, PlaylistProps, SeriesSort, StatusProps, Tone, UsersProps,
};
use super::theme;

/// A key the pane responds to, for the keybinding bar.
pub type Keybinding = (&'static str, &'static str);

/// Shared no-op implementations for the trait methods our panes don't
/// use (they're driven by typed props, not tui-realm attrs).
macro_rules! passive_component {
    ($ty:ty) => {
        impl Component for $ty {
            fn view(&mut self, frame: &mut Frame, area: Rect) {
                self.render(frame, area);
            }
            fn query<'a>(&'a self, _attr: Attribute) -> Option<QueryResult<'a>> {
                None
            }
            fn attr(&mut self, attr: Attribute, value: AttrValue) {
                if attr == Attribute::Focus
                    && let AttrValue::Flag(focused) = value
                {
                    self.focused = focused;
                }
            }
            fn state(&self) -> State {
                State::None
            }
            fn perform(&mut self, _cmd: Cmd) -> CmdResult {
                CmdResult::NoChange
            }
        }
    };
}

/// Match helper: a plain (modifier-less) key press.
pub(crate) fn plain(ev: &Event<NoUserEvent>) -> Option<Key> {
    match ev {
        Event::Keyboard(KeyEvent { code, modifiers }) if *modifiers == KeyModifiers::NONE => {
            Some(*code)
        }
        _ => None,
    }
}

/// Match helper: a Ctrl-modified key press.
pub(crate) fn ctrl(ev: &Event<NoUserEvent>) -> Option<Key> {
    match ev {
        Event::Keyboard(KeyEvent { code, modifiers }) if *modifiers == KeyModifiers::CONTROL => {
            Some(*code)
        }
        _ => None,
    }
}

/// Is this a typed character (with or without shift)?
pub(crate) fn typed(ev: &Event<NoUserEvent>) -> Option<char> {
    match ev {
        Event::Keyboard(KeyEvent {
            code: Key::Char(c),
            modifiers,
        }) if *modifiers == KeyModifiers::NONE || *modifiers == KeyModifiers::SHIFT => Some(*c),
        _ => None,
    }
}

/// Selection cursor over `len` rows.
fn step(sel: usize, len: usize, down: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if down {
        (sel + 1).min(len - 1)
    } else {
        sel.saturating_sub(1)
    }
}

// ---- Chat pane ---------------------------------------------------------

/// Chat log + always-visible input line.
pub struct ChatPane {
    lines: Vec<ChatLine>,
    input: tui_realm_stdlib::components::Input,
    focused: bool,
}

impl Default for ChatPane {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            input: tui_realm_stdlib::components::Input::default()
                .borders(tuirealm::props::Borders::default())
                .placeholder("say something…"),
            focused: false,
        }
    }
}

impl ChatPane {
    /// Replace the log.
    pub fn set_lines(&mut self, lines: Vec<ChatLine>) {
        self.lines = lines;
    }

    /// Current input text.
    fn text(&self) -> String {
        match self.input.state() {
            State::Single(StateValue::String(text)) => text,
            _ => String::new(),
        }
    }

    /// Clear the input line.
    fn clear(&mut self) {
        self.input
            .attr(Attribute::Value, AttrValue::String(String::new()));
    }

    /// Keys shown in the keybinding bar.
    pub fn keybindings(&self) -> Vec<Keybinding> {
        vec![("Enter", "Send"), ("Esc", "Clear")]
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [log_area, input_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(area);
        let visible = log_area.height.saturating_sub(2) as usize;
        let start = self.lines.len().saturating_sub(visible);
        let items: Vec<ListItem> = self.lines[start..]
            .iter()
            .map(|line| {
                if line.system {
                    // Local system notice: dim, no sender, "*" marker.
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{} ", line.time), theme::dim()),
                        Span::styled(format!("* {}", line.text), theme::dim()),
                    ]))
                } else {
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{} ", line.time), theme::dim()),
                        Span::styled(
                            format!("{}: ", line.sender),
                            Style::default().add_modifier(tuirealm::ratatui::style::Modifier::BOLD),
                        ),
                        Span::raw(line.text.clone()),
                    ]))
                }
            })
            .collect();
        frame.render_widget(
            List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border_style(self.focused))
                    .title("Chat"),
            ),
            log_area,
        );
        self.input.view(frame, input_area);
    }
}

passive_component!(ChatPane);

impl AppComponent<Msg, NoUserEvent> for ChatPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        let cmd = match typed(ev) {
            Some(c) => Cmd::Type(c),
            None => match plain(ev)? {
                Key::Enter => {
                    let text = self.text().trim().to_string();
                    if text.is_empty() {
                        return None;
                    }
                    self.clear();
                    return Some(if text.starts_with('/') {
                        Msg::Command(text)
                    } else {
                        Msg::SendChat(text)
                    });
                }
                Key::Esc => {
                    self.clear();
                    return Some(Msg::None);
                }
                Key::Backspace => Cmd::Delete, // stdlib: Delete = backspace
                Key::Delete => Cmd::Cancel,    // stdlib: Cancel = delete-forward
                Key::Left => Cmd::Move(Direction::Left),
                Key::Right => Cmd::Move(Direction::Right),
                Key::Home => Cmd::GoTo(Position::Begin),
                Key::End => Cmd::GoTo(Position::End),
                _ => return None,
            },
        };
        let _ = self.input.perform(cmd);
        Some(Msg::None)
    }
}

// ---- Users pane --------------------------------------------------------

/// Colored ready states + dim departed/seeder lines.
#[derive(Default)]
pub struct UsersPane {
    props: UsersProps,
    sel: usize,
    focused: bool,
}

impl UsersPane {
    /// Replace props, clamping the selection.
    pub fn set_props(&mut self, props: UsersProps) {
        self.props = props;
        self.sel = self.sel.min(self.props.rows.len().saturating_sub(1));
    }

    /// Keys shown in the keybinding bar.
    pub fn keybindings(&self) -> Vec<Keybinding> {
        vec![("↑↓", "Select"), ("a", "Mark away")]
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let mut items: Vec<ListItem> = self
            .props
            .rows
            .iter()
            .map(|row| {
                ListItem::new(Line::from(vec![
                    Span::raw(format!("{} ", row.name)),
                    Span::styled(format!("[{}]", row.label), theme::tone_style(row.tone)),
                ]))
            })
            .collect();
        if !self.props.departed.is_empty() {
            items.push(ListItem::new(Span::styled(
                format!("departed: {}", self.props.departed.join(", ")),
                theme::tone_style(Tone::Muted),
            )));
        }
        if !self.props.seeders.is_empty() {
            items.push(ListItem::new(Span::styled(
                format!("seeders: {}", self.props.seeders.join(", ")),
                theme::tone_style(Tone::Muted),
            )));
        }
        let mut state = ListState::default();
        if self.focused && !self.props.rows.is_empty() {
            state.select(Some(self.sel));
        }
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(theme::highlight_style())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme::border_style(self.focused))
                        .title("Users"),
                ),
            area,
            &mut state,
        );
    }
}

passive_component!(UsersPane);

impl AppComponent<Msg, NoUserEvent> for UsersPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        match plain(ev)? {
            Key::Up => {
                self.sel = step(self.sel, self.props.rows.len(), false);
                Some(Msg::None)
            }
            Key::Down => {
                self.sel = step(self.sel, self.props.rows.len(), true);
                Some(Msg::None)
            }
            Key::Char('a') => {
                let row = self.props.rows.get(self.sel)?;
                Some(Msg::ToggleAway(dessplay_core::types::UserId::new(
                    row.name.clone(),
                )))
            }
            _ => None,
        }
    }
}

// ---- Playlist pane -----------------------------------------------------

/// Shared playlist; trailing `[Add New]` row.
#[derive(Default)]
pub struct PlaylistPane {
    props: PlaylistProps,
    sel: usize,
    focused: bool,
}

impl PlaylistPane {
    /// Replace props, clamping the selection (rows + the Add New row).
    pub fn set_props(&mut self, props: PlaylistProps) {
        self.props = props;
        self.sel = self.sel.min(self.props.rows.len());
    }

    /// The hash under the cursor, if it's a real row.
    fn selected_hash(&self) -> Option<dessplay_core::types::Ed2kHash> {
        self.props.rows.get(self.sel).map(|row| row.hash)
    }

    /// Keys shown in the keybinding bar.
    pub fn keybindings(&self) -> Vec<Keybinding> {
        vec![
            ("Enter", "Play"),
            ("a", "Add"),
            ("d", "Remove"),
            ("C-j/k", "Move"),
            ("C-m", "Map"),
            ("A", "Archive"),
        ]
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let mut items: Vec<ListItem> = self
            .props
            .rows
            .iter()
            .map(|row| {
                let marker = if row.is_now { "▶ " } else { "  " };
                let style = theme::tone_style(row.tone);
                let left = format!("{marker}{}", row.title);
                if row.temporary {
                    // Cache-only file: dim "temporary" pushed to the right
                    // edge. Title clips before the tag when space is tight.
                    const TAG: &str = "temporary";
                    let inner = area.width.saturating_sub(2) as usize;
                    let pad = inner
                        .saturating_sub(left.chars().count() + TAG.len() + 1)
                        .max(1);
                    ListItem::new(Line::from(vec![
                        Span::styled(left, style),
                        Span::raw(" ".repeat(pad)),
                        Span::styled(TAG, theme::dim()),
                    ]))
                } else {
                    ListItem::new(Span::styled(left, style))
                }
            })
            .collect();
        items.push(ListItem::new(Span::styled("  [Add New]", theme::dim())));
        let mut state = ListState::default();
        if self.focused {
            state.select(Some(self.sel));
        }
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(theme::highlight_style())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme::border_style(self.focused))
                        .title("Playlist"),
                ),
            area,
            &mut state,
        );
    }
}

passive_component!(PlaylistPane);

impl AppComponent<Msg, NoUserEvent> for PlaylistPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some(code) = ctrl(ev) {
            let hash = self.selected_hash()?;
            return match code {
                Key::Char('j') => Some(Msg::MoveDown(hash)),
                Key::Char('k') => Some(Msg::MoveUp(hash)),
                Key::Char('m') => Some(Msg::MapFile(hash)),
                _ => None,
            };
        }
        // `A` (shift) archives; lowercase `a` adds. `typed` is the only
        // helper that sees a shifted char. Only cache-only ("temporary")
        // rows can be archived — anything else is a no-op.
        if typed(ev) == Some('A') {
            let row = self.props.rows.get(self.sel)?;
            return row.temporary.then_some(Msg::ArchiveFile(row.hash));
        }
        match plain(ev)? {
            Key::Up => {
                self.sel = step(self.sel, self.props.rows.len() + 1, false);
                Some(Msg::None)
            }
            Key::Down => {
                self.sel = step(self.sel, self.props.rows.len() + 1, true);
                Some(Msg::None)
            }
            Key::Enter => match self.selected_hash() {
                Some(hash) => Some(Msg::PlaySelected(hash)),
                // The [Add New] row: append.
                None => Some(Msg::AddFileAfter(None)),
            },
            Key::Char('a') => Some(Msg::AddFileAfter(self.selected_hash())),
            Key::Char('d') => self.selected_hash().map(Msg::RemoveEntry),
            _ => None,
        }
    }
}

// ---- Series pane -------------------------------------------------------

/// The pane's three modes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SeriesMode {
    /// Franchises by recency.
    #[default]
    Recent,
    /// Franchises alphabetical / by year.
    All,
    /// The List.
    TheList,
}

/// One navigable row in List mode.
enum ListNavRow {
    Heading(usize),
    Entry(usize, usize),
}

/// Series pane: Recent / All / The List.
#[derive(Default)]
pub struct SeriesPane {
    mode: SeriesMode,
    sort: SeriesSort,
    franchises: Vec<FranchiseRow>,
    groups: Vec<ListGroup>,
    /// Expanded-state override per group heading.
    expanded: std::collections::BTreeMap<&'static str, bool>,
    sel: usize,
    focused: bool,
}

impl SeriesPane {
    /// Current mode (the dispatcher rebuilds props on mode change).
    pub fn mode(&self) -> SeriesMode {
        self.mode
    }

    /// Current sort.
    pub fn sort(&self) -> SeriesSort {
        self.sort
    }

    /// Replace franchise rows (Recent / All modes).
    pub fn set_franchises(&mut self, rows: Vec<FranchiseRow>) {
        self.franchises = rows;
        self.clamp();
    }

    /// Replace List groups.
    pub fn set_groups(&mut self, groups: Vec<ListGroup>) {
        self.groups = groups;
        self.clamp();
    }

    fn expanded(&self, group: &ListGroup) -> bool {
        *self
            .expanded
            .get(group.heading)
            .unwrap_or(&!group.collapsed)
    }

    /// Rows in List mode, flattened for navigation.
    fn nav_rows(&self) -> Vec<ListNavRow> {
        let mut rows = Vec::new();
        for (g, group) in self.groups.iter().enumerate() {
            rows.push(ListNavRow::Heading(g));
            if self.expanded(group) {
                for e in 0..group.rows.len() {
                    rows.push(ListNavRow::Entry(g, e));
                }
            }
        }
        rows
    }

    fn len(&self) -> usize {
        match self.mode {
            SeriesMode::Recent | SeriesMode::All => self.franchises.len(),
            SeriesMode::TheList => self.nav_rows().len(),
        }
    }

    fn clamp(&mut self) {
        self.sel = self.sel.min(self.len().saturating_sub(1));
    }

    /// Keys shown in the keybinding bar.
    pub fn keybindings(&self) -> Vec<Keybinding> {
        match self.mode {
            SeriesMode::Recent => vec![("m", "Mode"), ("Enter", "Browse")],
            SeriesMode::All => vec![("m", "Mode"), ("s", "Sort"), ("Enter", "Browse")],
            SeriesMode::TheList => {
                vec![("m", "Mode"), ("Enter", "Open"), ("e", "Edit"), ("l", "Link")]
            }
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let title = match self.mode {
            SeriesMode::Recent => "Recent Series",
            SeriesMode::All => "All Series",
            SeriesMode::TheList => "The List",
        };
        let items: Vec<ListItem> = match self.mode {
            SeriesMode::Recent | SeriesMode::All => self
                .franchises
                .iter()
                .map(|row| {
                    let year = row.year.map(|y| format!(" ({y})")).unwrap_or_default();
                    ListItem::new(format!("{}{year}", row.title))
                })
                .collect(),
            SeriesMode::TheList => self
                .nav_rows()
                .iter()
                .map(|row| match row {
                    ListNavRow::Heading(g) => {
                        let group = &self.groups[*g];
                        let marker = if self.expanded(group) { "▾" } else { "▸" };
                        ListItem::new(Span::styled(
                            format!("{marker} {} ({})", group.heading, group.rows.len()),
                            theme::dim(),
                        ))
                    }
                    ListNavRow::Entry(g, e) => {
                        let entry = &self.groups[*g].rows[*e];
                        let mut spans = vec![Span::raw(format!("  {}", entry.name))];
                        if let Some(nero) = &entry.nero_name {
                            spans.push(Span::styled(format!(" “{nero}”"), theme::dim()));
                        }
                        if let Some(next) = &entry.next_ep {
                            let mark = if entry.available { "✓" } else { "" };
                            spans.push(Span::styled(
                                format!("  →{next}{mark}"),
                                theme::tone_style(if entry.available {
                                    Tone::Good
                                } else {
                                    Tone::Normal
                                }),
                            ));
                        }
                        if !entry.watchers.is_empty() {
                            spans.push(Span::styled(format!("  {}", entry.watchers), theme::dim()));
                        }
                        ListItem::new(Line::from(spans))
                    }
                })
                .collect(),
        };
        let mut state = ListState::default();
        if self.focused && !items.is_empty() {
            state.select(Some(self.sel));
        }
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(theme::highlight_style())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme::border_style(self.focused))
                        .title(title),
                ),
            area,
            &mut state,
        );
    }
}

passive_component!(SeriesPane);

impl AppComponent<Msg, NoUserEvent> for SeriesPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        match plain(ev)? {
            Key::Up => {
                self.sel = step(self.sel, self.len(), false);
                Some(Msg::None)
            }
            Key::Down => {
                self.sel = step(self.sel, self.len(), true);
                Some(Msg::None)
            }
            Key::Char('m') => {
                self.mode = match self.mode {
                    SeriesMode::Recent => SeriesMode::All,
                    SeriesMode::All => SeriesMode::TheList,
                    SeriesMode::TheList => SeriesMode::Recent,
                };
                self.sel = 0;
                Some(Msg::CycleSeriesMode)
            }
            Key::Char('s') if self.mode == SeriesMode::All => {
                self.sort = match self.sort {
                    SeriesSort::Title => SeriesSort::Year,
                    SeriesSort::Year => SeriesSort::Title,
                };
                Some(Msg::ToggleSeriesSort)
            }
            Key::Enter => match self.mode {
                SeriesMode::Recent | SeriesMode::All => {
                    let row = self.franchises.get(self.sel)?;
                    Some(Msg::BrowseFranchise(row.key.clone()))
                }
                SeriesMode::TheList => match self.nav_rows().get(self.sel)? {
                    ListNavRow::Heading(g) => {
                        let group = &self.groups[*g];
                        let now = self.expanded(group);
                        self.expanded.insert(group.heading, !now);
                        Some(Msg::None)
                    }
                    ListNavRow::Entry(g, e) => {
                        let entry = &self.groups[*g].rows[*e];
                        match entry.series_id {
                            Some(series) => Some(Msg::BrowseFranchise(
                                dessplay_core::franchise::FranchiseKey::Series(series),
                            )),
                            None => Some(Msg::EditListEntry(entry.id)),
                        }
                    }
                },
            },
            Key::Char('e') if self.mode == SeriesMode::TheList => {
                match self.nav_rows().get(self.sel)? {
                    ListNavRow::Entry(g, e) => {
                        Some(Msg::EditListEntry(self.groups[*g].rows[*e].id))
                    }
                    ListNavRow::Heading(_) => None,
                }
            }
            Key::Char('l') if self.mode == SeriesMode::TheList => {
                match self.nav_rows().get(self.sel)? {
                    ListNavRow::Entry(g, e) => {
                        Some(Msg::LinkListEntry(self.groups[*g].rows[*e].id))
                    }
                    ListNavRow::Heading(_) => None,
                }
            }
            _ => None,
        }
    }
}

// ---- Player status -----------------------------------------------------

/// The 3-line status block at the bottom.
#[derive(Default)]
pub struct StatusBar {
    props: StatusProps,
    focused: bool,
}

impl StatusBar {
    /// Replace props.
    pub fn set_props(&mut self, props: StatusProps) {
        self.props = props;
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        fn mmss(millis: u64) -> String {
            let s = millis / 1000;
            format!("{}:{:02}", s / 60, s % 60)
        }
        let progress = match (self.props.position_millis, self.props.duration_millis) {
            (Some(pos), Some(dur)) if dur > 0 => {
                let width = 30usize;
                let filled = ((pos as f64 / dur as f64) * width as f64) as usize;
                format!(
                    "[{}>{}] {} / {}",
                    "=".repeat(filled.min(width)),
                    " ".repeat(width - filled.min(width)),
                    mmss(pos),
                    mmss(dur),
                )
            }
            _ => String::new(),
        };
        let state = if self.props.playing {
            Span::styled("▶ playing", theme::tone_style(Tone::Good))
        } else if self.props.blockers.is_empty() {
            Span::styled("⏸ paused", theme::dim())
        } else {
            Span::styled(
                format!("⏸ waiting on {}", self.props.blockers.join(", ")),
                theme::tone_style(Tone::Blocked),
            )
        };
        let now = match &self.props.title {
            Some(title) => format!("Now Playing: {title}"),
            None => "Nothing playing".to_string(),
        };
        let lines = vec![
            Line::from(vec![state, Span::raw("  "), Span::raw(progress)]),
            Line::from(now),
        ];
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(theme::dim()),
            ),
            area,
        );
    }
}

passive_component!(StatusBar);

impl AppComponent<Msg, NoUserEvent> for StatusBar {
    fn on(&mut self, _ev: &Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}

// ---- Keybinding bar ----------------------------------------------------

/// The derived, context-sensitive bottom bar.
#[derive(Default)]
pub struct KeyBar {
    items: Vec<Keybinding>,
    focused: bool,
}

impl KeyBar {
    /// Replace bindings (focused pane's + globals).
    pub fn set_items(&mut self, items: Vec<Keybinding>) {
        self.items = items;
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let mut spans = Vec::new();
        for (key, label) in &self.items {
            if !spans.is_empty() {
                spans.push(Span::styled(" | ", theme::dim()));
            }
            spans.push(Span::styled(
                *key,
                Style::default().add_modifier(tuirealm::ratatui::style::Modifier::BOLD),
            ));
            spans.push(Span::raw(format!(" {label}")));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

passive_component!(KeyBar);

impl AppComponent<Msg, NoUserEvent> for KeyBar {
    fn on(&mut self, _ev: &Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}
