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

/// How many rows PgUp/PgDown jump in list panes.
pub(crate) const LIST_PAGE_STEP: usize = 10;

/// Selection cursor over `len` rows.
fn step(sel: usize, len: usize, down: bool) -> usize {
    step_by(sel, len, down, 1)
}

/// First index at or left of `cursor` that starts the word being left:
/// skip any whitespace to the left, then skip the word to the left.
fn word_boundary_left(chars: &[char], cursor: usize) -> usize {
    let mut i = cursor.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// First index at or right of `cursor` past the next word: skip any
/// whitespace to the right, then skip the word to the right.
fn word_boundary_right(chars: &[char], cursor: usize) -> usize {
    let n = chars.len();
    let mut i = cursor.min(n);
    while i < n && chars[i].is_whitespace() {
        i += 1;
    }
    while i < n && !chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Selection cursor over `len` rows, moved by `delta` (used for PgUp/PgDown).
pub(crate) fn step_by(sel: usize, len: usize, down: bool, delta: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if down {
        (sel + delta).min(len - 1)
    } else {
        sel.saturating_sub(delta)
    }
}

// ---- Chat pane ---------------------------------------------------------

/// How many visual lines PgUp/PgDown move the chat view.
const CHAT_PAGE_STEP: usize = 5;
/// Indent applied to wrapped continuation lines in the chat log.
const CHAT_WRAP_INDENT: usize = 2;
/// Most command suggestions shown at once in the discoverability popup.
const CHAT_SUGGESTION_MAX: u16 = 6;

/// Chat log + always-visible input line.
pub struct ChatPane {
    lines: Vec<ChatLine>,
    input: tui_realm_stdlib::components::Input,
    focused: bool,
    /// Visual lines scrolled up from the bottom (0 = pinned to newest).
    scroll_offset: usize,
    /// Text of every message this client has sent this session, for
    /// shell-style Up/Down recall. Never touches the synced chat.
    sent_history: Vec<String>,
    /// Position while walking `sent_history` (None = editing a fresh draft).
    history_pos: Option<usize>,
}

impl Default for ChatPane {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            input: tui_realm_stdlib::components::Input::default()
                .borders(tuirealm::props::Borders::default())
                .placeholder("say something…"),
            focused: false,
            scroll_offset: 0,
            sent_history: Vec::new(),
            history_pos: None,
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
        // Setting Value resets the cursor but NOT the stdlib Input's
        // horizontal scroll offset; without this a previously-scrolled line
        // would render the next line from a stale column. GoTo(Begin) runs
        // cursor_at_begin(), which zeroes the offset. (Same trick set_input
        // uses with GoTo(End).)
        let _ = self.input.perform(Cmd::GoTo(Position::Begin));
    }

    /// Keys shown in the keybinding bar.
    pub fn keybindings(&self) -> Vec<Keybinding> {
        vec![
            ("Enter", "Send"),
            ("PgUp/Dn", "Scroll"),
            ("↑↓", "History"),
            ("Esc", "Clear"),
        ]
    }

    /// Load `text` into the input and park the cursor at its end.
    fn set_input(&mut self, text: String) {
        self.input.attr(Attribute::Value, AttrValue::String(text));
        let _ = self.input.perform(Cmd::GoTo(Position::End));
    }

    /// Move the cursor left by one word. Driven through single-step Moves so
    /// the stdlib's horizontal-scroll bookkeeping stays correct.
    fn move_word_left(&mut self) {
        let cursor = self.input.states.cursor;
        let target = word_boundary_left(&self.input.states.input, cursor);
        for _ in target..cursor {
            let _ = self.input.perform(Cmd::Move(Direction::Left));
        }
    }

    /// Move the cursor right by one word.
    fn move_word_right(&mut self) {
        let cursor = self.input.states.cursor;
        let target = word_boundary_right(&self.input.states.input, cursor);
        for _ in cursor..target {
            let _ = self.input.perform(Cmd::Move(Direction::Right));
        }
    }

    /// Delete the word before the cursor (Ctrl-W / Ctrl-Backspace). Driven
    /// through stdlib backspaces so the scroll offset tracks down with it.
    fn kill_word_left(&mut self) {
        let cursor = self.input.states.cursor;
        let target = word_boundary_left(&self.input.states.input, cursor);
        for _ in target..cursor {
            let _ = self.input.perform(Cmd::Delete);
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [log_area, input_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(area);
        // When the input starts with `/`, carve the bottom of the log
        // area for a grey, filtered list of matching commands — pure
        // discoverability, captures no input. Collapses when the input
        // no longer matches anything.
        let suggestions = super::commands::matching(&self.text());
        let log_area = if suggestions.is_empty() {
            log_area
        } else {
            let height = (suggestions.len() as u16).min(CHAT_SUGGESTION_MAX);
            let [log_area, sugg_area] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(height)]).areas(log_area);
            // Tabulate: every help string starts at the same column. The
            // "name args" width is measured across the whole command table
            // (not just the filtered subset) so the column is stable as
            // the list narrows.
            let name_col = super::commands::SLASH_COMMANDS
                .iter()
                .map(|cmd| super::commands::signature(cmd).chars().count())
                .max()
                .unwrap_or(0);
            let items: Vec<ListItem> = suggestions
                .iter()
                .take(height as usize)
                .map(|cmd| {
                    let label = format!(
                        "{:<width$}   {}",
                        super::commands::signature(cmd),
                        cmd.help,
                        width = name_col,
                    );
                    ListItem::new(Span::styled(label, theme::dim()))
                })
                .collect();
            frame.render_widget(List::new(items), sugg_area);
            log_area
        };
        let width = log_area.width.saturating_sub(2) as usize;
        // Flatten every message into wrapped visual lines.
        let lines: Vec<Line> = self
            .lines
            .iter()
            .flat_map(|line| wrap_chat_line(line, width))
            .collect();
        let visible = log_area.height.saturating_sub(2) as usize;
        // Clamp the scroll so it can never run past the top of the log.
        let max_offset = lines.len().saturating_sub(visible);
        self.scroll_offset = self.scroll_offset.min(max_offset);
        let end = lines.len().saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(visible);
        let items: Vec<ListItem> = lines[start..end]
            .iter()
            .cloned()
            .map(ListItem::new)
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
        // Match the input border to the rest of the pane's focus color. The
        // stdlib Input uses its `Borders` color only when active and its
        // `UnfocusedBorderStyle` otherwise, so set both and forward focus
        // (which also makes the text cursor visible while typing).
        self.input.attr(
            Attribute::Borders,
            AttrValue::Borders(
                tuirealm::props::Borders::default().color(theme::border_color(true)),
            ),
        );
        self.input.attr(
            Attribute::UnfocusedBorderStyle,
            AttrValue::Style(Style::default().fg(theme::border_color(false))),
        );
        self.input
            .attr(Attribute::Focus, AttrValue::Flag(self.focused));
        self.input.view(frame, input_area);
    }
}

/// Greedy word-wrap. The first visual line gets `first_width` (the chat
/// prefix eats into it); later lines get `rest_width`. Breaks at spaces
/// where possible, hard-breaks any word longer than the available width.
fn wrap_body(text: &str, first_width: usize, rest_width: usize) -> Vec<String> {
    let width_for = |idx: usize| if idx == 0 { first_width } else { rest_width }.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut width = width_for(0);
    for mut word in text.split(' ') {
        loop {
            let cur_len = cur.chars().count();
            let space = usize::from(!cur.is_empty());
            let wlen = word.chars().count();
            if cur_len + space + wlen <= width {
                if space == 1 {
                    cur.push(' ');
                }
                cur.push_str(word);
                break;
            }
            if cur.is_empty() {
                // Word alone exceeds the line: hard-break it.
                let split_at = word
                    .char_indices()
                    .nth(width)
                    .map(|(i, _)| i)
                    .unwrap_or(word.len());
                let (head, tail) = word.split_at(split_at);
                cur.push_str(head);
                lines.push(std::mem::take(&mut cur));
                width = width_for(lines.len());
                word = tail;
            } else {
                // Flush and retry the word on a fresh line.
                lines.push(std::mem::take(&mut cur));
                width = width_for(lines.len());
            }
        }
    }
    lines.push(cur);
    lines
}

/// Render one chat message as one or more wrapped visual lines.
fn wrap_chat_line(line: &ChatLine, width: usize) -> Vec<Line<'static>> {
    use tuirealm::ratatui::style::Modifier;
    let indent: String = " ".repeat(CHAT_WRAP_INDENT);
    if line.separator {
        // Render-time day divider: the date label centered between dashes.
        let label = format!(" {} ", line.text);
        let label_w = label.chars().count();
        let total = width.max(label_w);
        let dashes = total - label_w;
        let left = dashes / 2;
        let bar = format!("{}{}{}", "─".repeat(left), label, "─".repeat(dashes - left));
        return vec![Line::from(Span::styled(bar, theme::dim()))];
    }
    if line.subtitle {
        // Local subtitle (Intermixed mode): dim, no sender, "»" marker,
        // in-video timestamp.
        let time = format!("{} ", line.time);
        let prefix_width = time.chars().count();
        let body = format!("» {}", line.text);
        let chunks = wrap_body(
            &body,
            width.saturating_sub(prefix_width),
            width.saturating_sub(CHAT_WRAP_INDENT),
        );
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, chunk)| {
                if i == 0 {
                    Line::from(vec![
                        Span::styled(time.clone(), theme::dim()),
                        Span::styled(chunk, theme::dim()),
                    ])
                } else {
                    Line::from(Span::styled(format!("{indent}{chunk}"), theme::dim()))
                }
            })
            .collect()
    } else if line.system {
        // Local system notice: dim, no sender, "*" marker.
        let time = format!("{} ", line.time);
        let prefix_width = time.chars().count();
        let body = format!("* {}", line.text);
        let chunks = wrap_body(
            &body,
            width.saturating_sub(prefix_width),
            width.saturating_sub(CHAT_WRAP_INDENT),
        );
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, chunk)| {
                if i == 0 {
                    Line::from(vec![
                        Span::styled(time.clone(), theme::dim()),
                        Span::styled(chunk, theme::dim()),
                    ])
                } else {
                    Line::from(Span::styled(format!("{indent}{chunk}"), theme::dim()))
                }
            })
            .collect()
    } else {
        let time = format!("{} ", line.time);
        let sender = format!("{}: ", line.sender);
        let prefix_width = time.chars().count() + sender.chars().count();
        let chunks = wrap_body(
            &line.text,
            width.saturating_sub(prefix_width),
            width.saturating_sub(CHAT_WRAP_INDENT),
        );
        let sender_style = theme::user_style(&line.sender).add_modifier(Modifier::BOLD);
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, chunk)| {
                if i == 0 {
                    Line::from(vec![
                        Span::styled(time.clone(), theme::dim()),
                        Span::styled(sender.clone(), sender_style),
                        Span::raw(chunk),
                    ])
                } else {
                    Line::from(Span::raw(format!("{indent}{chunk}")))
                }
            })
            .collect()
    }
}

passive_component!(ChatPane);

impl AppComponent<Msg, NoUserEvent> for ChatPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        let cmd = match typed(ev) {
            Some(c) => {
                // Editing detaches from history recall (shell behavior).
                self.history_pos = None;
                Cmd::Type(c)
            }
            None => {
                if let Some(key) = ctrl(ev) {
                    match key {
                        Key::Left => self.move_word_left(),
                        Key::Right => self.move_word_right(),
                        // Ctrl-W and Ctrl-Backspace both kill the previous word.
                        Key::Char('w') | Key::Backspace => self.kill_word_left(),
                        _ => return None,
                    }
                    return Some(Msg::None);
                }
                match plain(ev)? {
                    Key::Enter => {
                        let text = self.text().trim().to_string();
                        if text.is_empty() {
                            return None;
                        }
                        self.clear();
                        self.sent_history.push(text.clone());
                        self.history_pos = None;
                        self.scroll_offset = 0; // jump to newest so you see it
                        return Some(if text.starts_with('/') {
                            Msg::Command(text)
                        } else {
                            Msg::SendChat(text)
                        });
                    }
                    Key::Esc => {
                        self.clear();
                        self.history_pos = None;
                        return Some(Msg::None);
                    }
                    Key::PageUp => {
                        self.scroll_offset += CHAT_PAGE_STEP;
                        return Some(Msg::None);
                    }
                    Key::PageDown => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(CHAT_PAGE_STEP);
                        return Some(Msg::None);
                    }
                    Key::Up => {
                        // Recall an older message I sent into the input.
                        if self.sent_history.is_empty() {
                            return None;
                        }
                        let pos = match self.history_pos {
                            None => self.sent_history.len() - 1,
                            Some(p) => p.saturating_sub(1),
                        };
                        self.history_pos = Some(pos);
                        self.set_input(self.sent_history[pos].clone());
                        return Some(Msg::None);
                    }
                    Key::Down => {
                        // Walk back toward the newest, then to an empty draft.
                        let pos = self.history_pos?;
                        if pos + 1 < self.sent_history.len() {
                            self.history_pos = Some(pos + 1);
                            self.set_input(self.sent_history[pos + 1].clone());
                        } else {
                            self.history_pos = None;
                            self.clear();
                        }
                        return Some(Msg::None);
                    }
                    Key::Backspace => Cmd::Delete, // stdlib: Delete = backspace
                    Key::Delete => Cmd::Cancel,    // stdlib: Cancel = delete-forward
                    Key::Left => Cmd::Move(Direction::Left),
                    Key::Right => Cmd::Move(Direction::Right),
                    Key::Home => Cmd::GoTo(Position::Begin),
                    Key::End => Cmd::GoTo(Position::End),
                    _ => return None,
                }
            }
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
                    Span::styled(format!("{} ", row.name), theme::user_style(&row.name)),
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
            ("Ctrl-j/k", "Move"),
            ("M", "Map"),
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
                _ => None,
            };
        }
        // Shifted letters: `A` archives, `M` maps. `typed` is the only helper
        // that sees a shifted char. Map uses `M` rather than Ctrl-M because
        // Ctrl-M == Enter in terminals lacking the enhanced keyboard protocol.
        // Only cache-only ("temporary") rows can be archived.
        match typed(ev) {
            Some('A') => {
                let row = self.props.rows.get(self.sel)?;
                return row.temporary.then_some(Msg::ArchiveFile(row.hash));
            }
            Some('M') => return self.selected_hash().map(Msg::MapFile),
            _ => {}
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
    /// Filter text for Recent / All modes (case-insensitive substring on
    /// title). A non-empty filter also drops Recent's watched-only
    /// default. Empty in The List mode.
    filter: String,
    /// Whether we're editing the filter. Gated behind `/` (rather than
    /// typing directly) so the bare `m` / `s` mode/sort keys stay live —
    /// and reliable: Ctrl-modified letters collide with control codes
    /// (Ctrl-M == Enter) in terminals without the enhanced keyboard
    /// protocol, so they can't be used for the binding.
    filtering: bool,
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

    /// Current type-to-filter text (Recent / All modes).
    pub fn filter(&self) -> &str {
        &self.filter
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
        // While editing the filter, printable keys go to the filter; only
        // navigation / Esc / Enter act.
        if self.filtering {
            return vec![("type", "Filter"), ("Esc", "Clear"), ("Enter", "Browse")];
        }
        match self.mode {
            SeriesMode::Recent => vec![("m", "Mode"), ("/", "Filter"), ("Enter", "Browse")],
            SeriesMode::All => vec![
                ("m", "Mode"),
                ("s", "Sort"),
                ("/", "Filter"),
                ("Enter", "Browse"),
            ],
            SeriesMode::TheList => {
                vec![
                    ("m", "Mode"),
                    ("Enter", "Open"),
                    ("e", "Edit"),
                    ("l", "Link"),
                ]
            }
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let base = match self.mode {
            SeriesMode::Recent => "Recent Series",
            SeriesMode::All => "All Series",
            SeriesMode::TheList => "The List",
        };
        // Surface the filter so typing is visible (no silent state); show
        // the `/` cue the moment filtering starts, even before any text.
        let title =
            if self.mode != SeriesMode::TheList && (self.filtering || !self.filter.is_empty()) {
                format!("{base}  /{}", self.filter)
            } else {
                base.to_string()
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
        // Filter editing (entered with `/`): printable keys narrow the
        // list, Backspace deletes, Esc clears and exits. Up/Down and Enter
        // still navigate / select the filtered list. Mode and sort keys
        // are intentionally inert here so any letter can be typed.
        if self.filtering {
            if let Some(c) = typed(ev) {
                self.filter.push(c);
                self.sel = 0;
                return Some(Msg::SeriesFilterChanged);
            }
            return match plain(ev)? {
                // Backspace deletes a filter character; on an empty filter it
                // exits filtering entirely (the escape hatch, alongside Esc).
                Key::Backspace => {
                    if self.filter.pop().is_none() {
                        self.filtering = false;
                    }
                    self.sel = 0;
                    Some(Msg::SeriesFilterChanged)
                }
                Key::Esc => {
                    self.filter.clear();
                    self.filtering = false;
                    self.sel = 0;
                    Some(Msg::SeriesFilterChanged)
                }
                Key::Up => {
                    self.sel = step(self.sel, self.len(), false);
                    Some(Msg::None)
                }
                Key::Down => {
                    self.sel = step(self.sel, self.len(), true);
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
                    let row = self.franchises.get(self.sel)?;
                    Some(Msg::BrowseFranchise(row.key.clone()))
                }
                _ => None,
            };
        }
        match plain(ev)? {
            Key::Up => {
                self.sel = step(self.sel, self.len(), false);
                Some(Msg::None)
            }
            Key::Down => {
                self.sel = step(self.sel, self.len(), true);
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
            Key::Char('m') => {
                self.mode = match self.mode {
                    SeriesMode::Recent => SeriesMode::All,
                    SeriesMode::All => SeriesMode::TheList,
                    SeriesMode::TheList => SeriesMode::Recent,
                };
                self.filter.clear();
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
            // `/` begins filtering (Recent / All only).
            Key::Char('/') if self.mode != SeriesMode::TheList => {
                self.filtering = true;
                Some(Msg::None)
            }
            // A set-but-not-editing filter: Esc clears it.
            Key::Esc if self.mode != SeriesMode::TheList && !self.filter.is_empty() => {
                self.filter.clear();
                self.sel = 0;
                Some(Msg::SeriesFilterChanged)
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

#[cfg(test)]
mod series_pane_tests {
    use super::*;

    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// Mode/sort live on the bare `m` / `s` keys (reliable across
    /// terminals); filtering is gated behind `/` so those letters can be
    /// typed into the filter without cycling the mode. Regression for
    /// Ctrl-m being indistinguishable from Enter in konsole (2026-06-14).
    #[test]
    fn slash_gated_filter_leaves_mode_keys_live() {
        let mut p = SeriesPane::default();
        assert_eq!(p.mode(), SeriesMode::Recent);

        // Bare `m` cycles mode when not filtering.
        p.on(&key(Key::Char('m')));
        assert_eq!(p.mode(), SeriesMode::All);

        // `/` starts filtering; now letters — including `m` and `s` —
        // build the filter instead of cycling/sorting.
        p.on(&key(Key::Char('/')));
        for c in ['m', 'o', 'n'] {
            p.on(&key(Key::Char(c)));
        }
        assert_eq!(p.filter(), "mon");
        assert_eq!(
            p.mode(),
            SeriesMode::All,
            "mode must not change while filtering"
        );

        // Backspace edits; Esc clears and exits filtering.
        p.on(&key(Key::Backspace));
        assert_eq!(p.filter(), "mo");
        p.on(&key(Key::Esc));
        assert_eq!(p.filter(), "");

        // After Esc, `m` cycles again.
        p.on(&key(Key::Char('m')));
        assert_eq!(p.mode(), SeriesMode::TheList);
    }

    /// Backspace deletes filter characters; once the filter is empty, a
    /// further Backspace exits filtering entirely (an escape hatch alongside
    /// Esc). Regression for filtering being a one-way trip via `/`
    /// (2026-06-15).
    #[test]
    fn backspace_on_empty_filter_exits_filtering() {
        let mut p = SeriesPane::default();
        p.on(&key(Key::Char('/')));
        p.on(&key(Key::Char('a')));
        assert_eq!(p.filter(), "a");

        // First Backspace empties the filter (still filtering).
        p.on(&key(Key::Backspace));
        assert_eq!(p.filter(), "");
        // Proof we're still in filter mode: `m` types, it does not cycle.
        p.on(&key(Key::Char('m')));
        assert_eq!(p.filter(), "m");
        assert_eq!(p.mode(), SeriesMode::Recent);

        // Empty it again, then Backspace once more to leave filtering.
        p.on(&key(Key::Backspace)); // "" again
        p.on(&key(Key::Backspace)); // exits filtering
        assert_eq!(p.filter(), "");
        // Now `m` cycles the mode again — filtering really ended.
        p.on(&key(Key::Char('m')));
        assert_eq!(p.mode(), SeriesMode::All);
    }

    fn franchises(n: usize) -> Vec<FranchiseRow> {
        (0..n)
            .map(|i| FranchiseRow {
                key: dessplay_core::franchise::FranchiseKey::Name(i.to_string()),
                title: i.to_string(),
                year: None,
            })
            .collect()
    }

    /// PageUp/PageDown jump the selection by a page in both Recent/All and
    /// while filtering. Regression for the series browser ignoring the page
    /// keys (2026-06-15). Selection is observed through the franchise Enter
    /// resolves to.
    #[test]
    fn page_keys_jump_series_selection() {
        let mut p = SeriesPane::default();
        p.set_franchises(franchises(30));

        // From the top, PageDown lands a page in.
        p.on(&key(Key::PageDown));
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseFranchise(
                dessplay_core::franchise::FranchiseKey::Name(LIST_PAGE_STEP.to_string())
            ))
        );

        // PageUp returns to the top.
        p.on(&key(Key::PageUp));
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseFranchise(
                dessplay_core::franchise::FranchiseKey::Name("0".to_string())
            ))
        );

        // The page keys also work while filtering (filter empty = all rows).
        p.on(&key(Key::Char('/')));
        p.on(&key(Key::PageDown));
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseFranchise(
                dessplay_core::franchise::FranchiseKey::Name(LIST_PAGE_STEP.to_string())
            ))
        );
    }

    /// The List mode has no filter: `/` is inert and the bare letters keep
    /// their List bindings.
    #[test]
    fn the_list_mode_does_not_filter() {
        let mut p = SeriesPane::default();
        p.on(&key(Key::Char('m'))); // All
        p.on(&key(Key::Char('m'))); // The List
        assert_eq!(p.mode(), SeriesMode::TheList);
        p.on(&key(Key::Char('/')));
        assert_eq!(p.filter(), "");
        // `m` still cycles (back to Recent), proving `/` didn't start a filter.
        p.on(&key(Key::Char('m')));
        assert_eq!(p.mode(), SeriesMode::Recent);
    }
}

#[cfg(test)]
mod playlist_pane_tests {
    use super::*;
    use crate::ui::props::PlaylistRow;
    use dessplay_core::types::Ed2kHash;

    fn shifted(c: char) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code: Key::Char(c),
            modifiers: KeyModifiers::NONE,
        })
    }

    fn ctrl_key(c: char) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code: Key::Char(c),
            modifiers: KeyModifiers::CONTROL,
        })
    }

    fn pane_with_one_row() -> (PlaylistPane, Ed2kHash) {
        let hash = Ed2kHash([7u8; 16]);
        let mut p = PlaylistPane {
            focused: true,
            ..Default::default()
        };
        p.set_props(PlaylistProps {
            rows: vec![PlaylistRow {
                hash,
                title: "ep.mkv".to_string(),
                tone: Tone::Normal,
                is_now: false,
                temporary: false,
            }],
            ..Default::default()
        });
        (p, hash)
    }

    /// Manual mapping moved from Ctrl-m to capital `M`: Ctrl-M is
    /// indistinguishable from Enter in terminals without the enhanced
    /// keyboard protocol. Regression (2026-06-15).
    #[test]
    fn capital_m_maps_and_ctrl_m_does_not() {
        let (mut p, hash) = pane_with_one_row();
        assert_eq!(p.on(&shifted('M')), Some(Msg::MapFile(hash)));
        // The old Ctrl-m binding is gone (it now reads as Enter elsewhere).
        assert_eq!(p.on(&ctrl_key('m')), None);
    }
}

#[cfg(test)]
mod chat_wrap_tests {
    use super::wrap_body;

    #[test]
    fn breaks_at_spaces() {
        // first_width and rest_width both 10.
        let lines = wrap_body("the quick brown fox", 10, 10);
        for line in &lines {
            assert!(line.chars().count() <= 10, "line too wide: {line:?}");
        }
        // Reassembling with single spaces recovers the words in order.
        assert_eq!(
            lines.join(" ").split_whitespace().collect::<Vec<_>>(),
            ["the", "quick", "brown", "fox"]
        );
    }

    #[test]
    fn hard_breaks_overlong_word() {
        let lines = wrap_body("supercalifragilistic", 5, 5);
        assert!(lines.len() > 1, "long word should be split");
        for line in &lines {
            assert!(line.chars().count() <= 5, "chunk too wide: {line:?}");
        }
        assert_eq!(lines.concat(), "supercalifragilistic");
    }

    #[test]
    fn respects_narrower_first_line() {
        // First line only fits "ab"; the rest get width 10.
        let lines = wrap_body("ab cdefghij", 2, 10);
        assert_eq!(lines[0], "ab");
        assert_eq!(lines[1], "cdefghij");
    }

    #[test]
    fn exact_width_stays_on_one_line() {
        let lines = wrap_body("abcde", 5, 5);
        assert_eq!(lines, vec!["abcde".to_string()]);
    }

    #[test]
    fn zero_width_does_not_loop() {
        // Degenerate width is clamped to 1 internally; must terminate.
        let lines = wrap_body("hi there", 0, 0);
        assert!(!lines.is_empty());
    }
}

#[cfg(test)]
mod chat_input_tests {
    use super::*;

    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn ctrl(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
        })
    }

    fn focused_pane() -> ChatPane {
        ChatPane {
            focused: true,
            ..Default::default()
        }
    }

    fn type_str(pane: &mut ChatPane, text: &str) {
        for c in text.chars() {
            pane.on(&key(Key::Char(c)));
        }
    }

    /// Regression: a scrolled input line must reset its horizontal scroll
    /// offset when sent, so the next line is not rendered from a stale column.
    #[test]
    fn enter_resets_display_offset() {
        let mut pane = focused_pane();
        type_str(&mut pane, "a fairly long line that would scroll");
        // `display_offset` only grows during rendering (last_width is unset in
        // tests); simulate a scrolled line directly.
        pane.input.states.display_offset = 12;
        let msg = pane.on(&key(Key::Enter));
        assert!(matches!(msg, Some(Msg::SendChat(_))));
        assert_eq!(pane.input.states.display_offset, 0);
        assert_eq!(pane.text(), "");
    }

    #[test]
    fn esc_resets_display_offset() {
        let mut pane = focused_pane();
        type_str(&mut pane, "some text");
        pane.input.states.display_offset = 5;
        pane.on(&key(Key::Esc));
        assert_eq!(pane.input.states.display_offset, 0);
        assert_eq!(pane.text(), "");
    }

    /// Backspacing the whole line away leaves the offset at zero (relies on
    /// stdlib `backspace()` tracking the offset down with the cursor).
    #[test]
    fn backspace_to_empty_resets_display_offset() {
        let mut pane = focused_pane();
        type_str(&mut pane, "hello");
        for _ in 0.."hello".len() {
            pane.on(&key(Key::Backspace));
        }
        assert_eq!(pane.text(), "");
        assert_eq!(pane.input.states.display_offset, 0);
    }

    #[test]
    fn ctrl_left_moves_by_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        // Cursor parks at end (15). Word-left lands on the start of "brown".
        pane.on(&ctrl(Key::Left));
        assert_eq!(pane.input.states.cursor, 10);
        pane.on(&ctrl(Key::Left));
        assert_eq!(pane.input.states.cursor, 4); // start of "quick"
        pane.on(&ctrl(Key::Left));
        assert_eq!(pane.input.states.cursor, 0); // start of "the"
        pane.on(&ctrl(Key::Left));
        assert_eq!(pane.input.states.cursor, 0); // clamped
    }

    #[test]
    fn ctrl_right_moves_by_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        // Move cursor to the start first.
        pane.on(&key(Key::Home));
        assert_eq!(pane.input.states.cursor, 0);
        pane.on(&ctrl(Key::Right));
        assert_eq!(pane.input.states.cursor, 3); // end of "the"
        pane.on(&ctrl(Key::Right));
        assert_eq!(pane.input.states.cursor, 9); // end of "quick"
        pane.on(&ctrl(Key::Right));
        assert_eq!(pane.input.states.cursor, 15); // end of "brown"
        pane.on(&ctrl(Key::Right));
        assert_eq!(pane.input.states.cursor, 15); // clamped
    }

    #[test]
    fn ctrl_w_kills_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        pane.on(&ctrl(Key::Char('w')));
        assert_eq!(pane.text(), "the quick ");
        // Second kill skips the trailing space, then removes "quick".
        pane.on(&ctrl(Key::Char('w')));
        assert_eq!(pane.text(), "the ");
    }

    #[test]
    fn ctrl_backspace_kills_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        pane.on(&ctrl(Key::Backspace));
        assert_eq!(pane.text(), "the quick ");
    }

    /// Killing across trailing whitespace removes the spaces and the word.
    #[test]
    fn ctrl_w_skips_trailing_whitespace() {
        let mut pane = focused_pane();
        type_str(&mut pane, "hello   ");
        pane.on(&ctrl(Key::Char('w')));
        assert_eq!(pane.text(), "");
    }

    #[test]
    fn word_boundary_left_cases() {
        let chars: Vec<char> = "the quick".chars().collect();
        assert_eq!(word_boundary_left(&chars, 9), 4); // from end → start of "quick"
        assert_eq!(word_boundary_left(&chars, 4), 0); // from start of "quick" → "the"
        assert_eq!(word_boundary_left(&chars, 0), 0); // clamped
        // Mid-word.
        assert_eq!(word_boundary_left(&chars, 6), 4);
        // Leading spaces: word-left from the end stops at the word start.
        let spaced: Vec<char> = "   hi".chars().collect();
        assert_eq!(word_boundary_left(&spaced, 5), 3);
        // From the word start, skipping the leading spaces reaches 0.
        assert_eq!(word_boundary_left(&spaced, 3), 0);
        // Empty.
        assert_eq!(word_boundary_left(&[], 0), 0);
    }

    #[test]
    fn word_boundary_right_cases() {
        let chars: Vec<char> = "the quick".chars().collect();
        assert_eq!(word_boundary_right(&chars, 0), 3); // → end of "the"
        assert_eq!(word_boundary_right(&chars, 3), 9); // → end of "quick"
        assert_eq!(word_boundary_right(&chars, 9), 9); // clamped
        // Mid-word.
        assert_eq!(word_boundary_right(&chars, 1), 3);
        // Trailing spaces.
        let spaced: Vec<char> = "hi   ".chars().collect();
        assert_eq!(word_boundary_right(&spaced, 0), 2);
        // Empty.
        assert_eq!(word_boundary_right(&[], 0), 0);
    }
}
