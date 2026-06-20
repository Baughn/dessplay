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

/// Match helper for word navigation/deletion: a key carrying *either* the
/// Ctrl or the Alt modifier. Desktop terminals send Ctrl for Ctrl-arrow;
/// macOS terminals (ghostty) send Alt for Option-arrow and are unreliable
/// about Ctrl-arrow — accepting both makes word motion work everywhere, and
/// matches the Alt-arrow/Alt-Backspace muscle memory from macOS line editing.
/// `.contains` (rather than `==`) also tolerates the extra modifier bits the
/// kitty keyboard protocol can set alongside Ctrl. The modifiers are returned
/// so callers can keep a binding Ctrl-only where Alt would collide (e.g. `w`).
pub(crate) fn word_mod(ev: &Event<NoUserEvent>) -> Option<(Key, KeyModifiers)> {
    match ev {
        Event::Keyboard(KeyEvent { code, modifiers })
            if modifiers.contains(KeyModifiers::CONTROL)
                || modifiers.contains(KeyModifiers::ALT) =>
        {
            Some((*code, *modifiers))
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
/// Sized to fit the whole command table on a bare `/` with a little
/// headroom (see [`super::commands::SLASH_COMMANDS`]).
const CHAT_SUGGESTION_MAX: u16 = 11;

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
    /// Online usernames (present + lost interactive peers; seeders and
    /// departed excluded), used for Tab-completion and mention highlighting.
    /// Stored with original casing; matching is case-insensitive.
    usernames: Vec<String>,
    /// The local user's name, so their own mentions can be emphasized.
    me: String,
    /// Tab-completion cycling state (see [`ChatPane::try_tab_complete`]).
    completion: Option<CompletionState>,
}

/// In-flight Tab-completion cycle. While the input still equals `produced`,
/// repeated Tab walks `candidates`; any other edit drops this state so the
/// next Tab recomputes from the buffer.
struct CompletionState {
    /// Buffer text before the completed trailing word.
    head: String,
    /// Matching usernames, in the order Tab cycles through them.
    candidates: Vec<String>,
    /// Index of the candidate currently in the buffer.
    index: usize,
    /// Trailing text added after the name (`": "` for a whole-buffer
    /// completion, else empty).
    suffix: &'static str,
    /// Exact text last written, so we can tell a continued cycle from a
    /// fresh completion.
    produced: String,
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
            usernames: Vec::new(),
            me: String::new(),
            completion: None,
        }
    }
}

impl ChatPane {
    /// Replace the log.
    pub fn set_lines(&mut self, lines: Vec<ChatLine>) {
        self.lines = lines;
    }

    /// Set the online-username set used for completion and highlighting.
    pub fn set_usernames(&mut self, names: Vec<String>) {
        self.usernames = names;
    }

    /// Set the local user's name (for self-mention emphasis).
    pub fn set_me(&mut self, me: String) {
        self.me = me;
    }

    /// Try to Tab-complete a username at the end of the input. Returns
    /// `true` if it completed (the caller should *not* cycle panes), `false`
    /// if the trailing word matches no online username (Tab falls through to
    /// its normal pane-cycling job). Repeated Tab without an intervening edit
    /// cycles through multiple matches.
    pub fn try_tab_complete(&mut self) -> bool {
        let text = self.text();
        // Continue an in-flight cycle iff the buffer is still exactly what we
        // last wrote.
        if let Some(state) = &mut self.completion
            && state.produced == text
        {
            state.index = (state.index + 1) % state.candidates.len();
            let chosen = &state.candidates[state.index];
            let produced = format!("{}{}{}", state.head, chosen, state.suffix);
            state.produced = produced.clone();
            self.set_input(produced);
            return true;
        }
        // Fresh completion.
        let Some((head, matches)) = mention_completion_candidates(&text, &self.usernames) else {
            self.completion = None;
            return false;
        };
        let suffix = if head.is_empty() { ": " } else { "" };
        let candidates: Vec<String> = matches.into_iter().cloned().collect();
        let chosen = &candidates[0];
        let produced = format!("{head}{chosen}{suffix}");
        self.set_input(produced.clone());
        self.completion = Some(CompletionState {
            head,
            candidates,
            index: 0,
            suffix,
            produced,
        });
        true
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
            .flat_map(|line| wrap_chat_line(line, width, &self.usernames, &self.me))
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

/// Trailing punctuation stripped off a word before testing it against the
/// username set (so "Baughn:" and "Nero," still match), and used to bound
/// the completable trailing word.
const MENTION_PUNCT: &[char] = &[':', ',', '.', '!', '?', ';', ')', '('];

/// Compute a Tab-completion for the trailing word of `text`.
///
/// The trailing word is the run after the last space. If it is a non-empty,
/// case-insensitive prefix of one or more `usernames`, returns the buffer
/// text *before* that word (`head`) and the matching usernames, sorted
/// case-insensitively for deterministic cycling. Returns `None` when the
/// trailing word is empty or matches nothing (Tab then keeps its normal job).
fn mention_completion_candidates<'a>(
    text: &str,
    usernames: &'a [String],
) -> Option<(String, Vec<&'a String>)> {
    let trail_start = text.rfind(' ').map(|i| i + 1).unwrap_or(0);
    let head = &text[..trail_start];
    let prefix = &text[trail_start..];
    if prefix.is_empty() {
        return None;
    }
    let lower = prefix.to_lowercase();
    let mut matches: Vec<&String> = usernames
        .iter()
        .filter(|u| u.to_lowercase().starts_with(&lower))
        .collect();
    if matches.is_empty() {
        return None;
    }
    matches.sort_by_key(|u| u.to_lowercase());
    Some((head.to_string(), matches))
}

/// Split one wrapped body chunk into spans, coloring any whitespace-delimited
/// word that matches an online username (trailing punctuation stripped before
/// matching but kept as plain text). Mentions of `me` are additionally
/// reversed so a ping stands out. Spacing is preserved verbatim.
fn highlight_mentions(chunk: &str, usernames: &[String], me: &str) -> Vec<Span<'static>> {
    use tuirealm::ratatui::style::Modifier;
    let mut spans: Vec<Span<'static>> = Vec::new();
    // Walk space-separated tokens, re-emitting the single spaces between them.
    let mut first = true;
    for token in chunk.split(' ') {
        if !first {
            spans.push(Span::raw(" "));
        }
        first = false;
        if token.is_empty() {
            continue;
        }
        // Split the candidate word from any trailing punctuation.
        let candidate = token.trim_end_matches(MENTION_PUNCT);
        let punct = &token[candidate.len()..];
        let canonical = (!candidate.is_empty())
            .then(|| usernames.iter().find(|u| u.eq_ignore_ascii_case(candidate)))
            .flatten();
        match canonical {
            Some(name) => {
                let mut style = theme::user_style(name).add_modifier(Modifier::BOLD);
                if name.eq_ignore_ascii_case(me) {
                    style = style.patch(theme::highlight_style());
                }
                spans.push(Span::styled(candidate.to_string(), style));
                if !punct.is_empty() {
                    spans.push(Span::raw(punct.to_string()));
                }
            }
            None => spans.push(Span::raw(token.to_string())),
        }
    }
    spans
}

/// Render one chat message as one or more wrapped visual lines.
fn wrap_chat_line(
    line: &ChatLine,
    width: usize,
    usernames: &[String],
    me: &str,
) -> Vec<Line<'static>> {
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
    } else if line.action {
        // IRC-style action: "* sender phrase", no colon. The "* " is dim,
        // the sender keeps its per-user color/bold, the phrase is raw.
        let time = format!("{} ", line.time);
        let marker = "* ";
        let sender = format!("{} ", line.sender);
        let prefix_width = time.chars().count() + marker.chars().count() + sender.chars().count();
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
                let body = highlight_mentions(&chunk, usernames, me);
                if i == 0 {
                    let mut spans = vec![
                        Span::styled(time.clone(), theme::dim()),
                        Span::styled(marker, theme::dim()),
                        Span::styled(sender.clone(), sender_style),
                    ];
                    spans.extend(body);
                    Line::from(spans)
                } else {
                    let mut spans = vec![Span::raw(indent.clone())];
                    spans.extend(body);
                    Line::from(spans)
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
                let body = highlight_mentions(&chunk, usernames, me);
                if i == 0 {
                    let mut spans = vec![
                        Span::styled(time.clone(), theme::dim()),
                        Span::styled(sender.clone(), sender_style),
                    ];
                    spans.extend(body);
                    Line::from(spans)
                } else {
                    let mut spans = vec![Span::raw(indent.clone())];
                    spans.extend(body);
                    Line::from(spans)
                }
            })
            .collect()
    }
}

passive_component!(ChatPane);

impl AppComponent<Msg, NoUserEvent> for ChatPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        // Tab is intercepted in `Ui::handle` (it drives completion / pane
        // cycling), so every event reaching this method is a non-Tab key:
        // any of them ends an in-flight completion cycle.
        self.completion = None;
        let cmd = match typed(ev) {
            Some(c) => {
                // Editing detaches from history recall (shell behavior).
                self.history_pos = None;
                Cmd::Type(c)
            }
            None => {
                if let Some((key, mods)) = word_mod(ev) {
                    match key {
                        Key::Left => self.move_word_left(),
                        Key::Right => self.move_word_right(),
                        // macOS terminals (ghostty) don't send Alt-arrow for
                        // Option-Left/Right — they emit the readline word-motion
                        // bytes Alt-b / Alt-f. Bind those (Alt-only; Ctrl-B /
                        // Ctrl-F are char-wise motion in readline, not ours).
                        Key::Char('b') if mods.contains(KeyModifiers::ALT) => self.move_word_left(),
                        Key::Char('f') if mods.contains(KeyModifiers::ALT) => {
                            self.move_word_right()
                        }
                        // Ctrl-Backspace / Alt-Backspace kill the previous word
                        // (Alt-Backspace is the macOS delete-word habit).
                        Key::Backspace => self.kill_word_left(),
                        // Ctrl-W also kills, but only under Ctrl — Alt-W is a
                        // typed character on macOS, not a word kill.
                        Key::Char('w') if mods.contains(KeyModifiers::CONTROL) => {
                            self.kill_word_left()
                        }
                        // Any other Ctrl/Alt combo isn't ours; fall through to
                        // the plain-key match (which rejects modified keys).
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
            ("w", "Watch"),
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
                // Right-aligned dim tags: the always-shown watch state,
                // prefixed by "temporary" for a cache-only copy. Title
                // clips before the tags when space is tight.
                let watch_tag = match row.watch {
                    dessplay_core::types::SeriesWatchState::Watching => "watching",
                    dessplay_core::types::SeriesWatchState::Maybe => "maybe",
                    dessplay_core::types::SeriesWatchState::NotWatching => "not watching",
                };
                let right = if row.temporary {
                    format!("temporary  {watch_tag}")
                } else {
                    watch_tag.to_string()
                };
                let inner = area.width.saturating_sub(2) as usize;
                let pad = inner
                    .saturating_sub(left.chars().count() + right.chars().count() + 1)
                    .max(1);
                ListItem::new(Line::from(vec![
                    Span::styled(left, style),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(right, theme::dim()),
                ]))
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
            // Cycle this entry's series watch state: Watching -> Maybe ->
            // NotWatching -> ...
            Key::Char('w') => self.selected_hash().map(Msg::CycleSeriesWatch),
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
                watch: dessplay_core::types::SeriesWatchState::Maybe,
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

    fn alt(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::ALT,
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

    /// Alt-Left/Right move by word too — macOS terminals (ghostty) send Alt
    /// for Option-arrow, and some never send a usable Ctrl-arrow at all.
    #[test]
    fn alt_left_right_move_by_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        pane.on(&alt(Key::Left));
        assert_eq!(pane.input.states.cursor, 10); // start of "brown"
        pane.on(&alt(Key::Left));
        assert_eq!(pane.input.states.cursor, 4); // start of "quick"
        pane.on(&alt(Key::Right));
        assert_eq!(pane.input.states.cursor, 9); // end of "quick"
    }

    /// macOS terminals (ghostty) emit Option-Left/Right as the readline
    /// word-motion bytes Alt-b / Alt-f, not Alt-arrow — those must move by word.
    #[test]
    fn alt_b_f_move_by_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        pane.on(&alt(Key::Char('b')));
        assert_eq!(pane.input.states.cursor, 10); // start of "brown"
        pane.on(&alt(Key::Char('b')));
        assert_eq!(pane.input.states.cursor, 4); // start of "quick"
        pane.on(&alt(Key::Char('f')));
        assert_eq!(pane.input.states.cursor, 9); // end of "quick"
    }

    /// Ctrl-B / Ctrl-F are char-wise in readline, not word motion — they must
    /// not be hijacked into word jumps (and aren't typed into the buffer).
    #[test]
    fn ctrl_b_f_are_not_word_motion() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        let before = pane.input.states.cursor;
        pane.on(&ctrl(Key::Char('b')));
        pane.on(&ctrl(Key::Char('f')));
        assert_eq!(pane.input.states.cursor, before);
        assert_eq!(pane.text(), "the quick brown");
    }

    /// Alt-Backspace deletes the previous word (macOS line-editing habit).
    #[test]
    fn alt_backspace_kills_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        pane.on(&alt(Key::Backspace));
        assert_eq!(pane.text(), "the quick ");
    }

    /// Alt-W is a typed character on macOS, not a word kill — only Ctrl-W
    /// kills. (Here it simply does nothing, not delete a word.)
    #[test]
    fn alt_w_does_not_kill_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        pane.on(&alt(Key::Char('w')));
        assert_eq!(pane.text(), "the quick brown");
    }

    /// The kitty keyboard protocol can report Ctrl-arrow with extra modifier
    /// bits set; word motion must still trigger (we match on `contains`, not
    /// equality). This is the most likely cause of "Ctrl-Left does nothing on
    /// the laptop but works on the desktop".
    #[test]
    fn ctrl_left_with_extra_modifier_bits_moves_by_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        let ev = Event::Keyboard(KeyEvent {
            code: Key::Left,
            modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        });
        pane.on(&ev);
        assert_eq!(pane.input.states.cursor, 10); // start of "brown"
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

#[cfg(test)]
mod chat_completion_tests {
    use super::*;
    use tuirealm::ratatui::style::Modifier;

    fn names() -> Vec<String> {
        ["Baughn", "Nero", "Dagger", "Danny"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    fn pane(names: Vec<String>, me: &str) -> ChatPane {
        let mut p = ChatPane::default();
        p.set_usernames(names);
        p.set_me(me.to_string());
        p
    }

    #[test]
    fn no_completion_for_empty_or_trailing_space() {
        let mut p = pane(names(), "");
        // Empty buffer: Tab does not complete (falls through to pane cycle).
        assert!(!p.try_tab_complete());
        // Trailing space: the word is empty, so no completion.
        p.set_input("hello ".to_string());
        assert!(!p.try_tab_complete());
        assert_eq!(p.text(), "hello ");
    }

    #[test]
    fn whole_buffer_prefix_gets_colon_suffix() {
        let mut p = pane(names(), "");
        p.set_input("bau".to_string());
        assert!(p.try_tab_complete());
        assert_eq!(p.text(), "Baughn: ");
    }

    #[test]
    fn mid_sentence_completes_without_colon_or_space() {
        let mut p = pane(names(), "");
        p.set_input("hey ner".to_string());
        assert!(p.try_tab_complete());
        assert_eq!(p.text(), "hey Nero");
    }

    #[test]
    fn matching_is_case_insensitive_canonical_casing_wins() {
        let mut p = pane(names(), "");
        p.set_input("BAU".to_string());
        assert!(p.try_tab_complete());
        assert_eq!(p.text(), "Baughn: ");
    }

    #[test]
    fn non_prefix_does_not_complete() {
        let mut p = pane(names(), "");
        p.set_input("xyz".to_string());
        assert!(!p.try_tab_complete());
        assert_eq!(p.text(), "xyz");
    }

    #[test]
    fn repeated_tab_cycles_through_matches_and_wraps() {
        // "da" matches both Dagger and Danny (sorted case-insensitively).
        let mut p = pane(names(), "");
        p.set_input("da".to_string());
        assert!(p.try_tab_complete());
        assert_eq!(p.text(), "Dagger: ");
        // Tab again (no edit) advances to the next match.
        assert!(p.try_tab_complete());
        assert_eq!(p.text(), "Danny: ");
        // And wraps back around.
        assert!(p.try_tab_complete());
        assert_eq!(p.text(), "Dagger: ");
    }

    #[test]
    fn editing_resets_the_cycle() {
        let mut p = pane(names(), "");
        p.set_input("da".to_string());
        assert!(p.try_tab_complete());
        assert_eq!(p.text(), "Dagger: ");
        // An edit (any key reaching `on`) drops the cycle state; the buffer
        // no longer equals `produced`, so the next Tab recomputes fresh.
        p.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('x'),
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(p.text(), "Dagger: x");
        // "x" alone matches nothing, so Tab no longer completes.
        assert!(!p.try_tab_complete());
    }

    fn span_is_styled_user(span: &Span, name: &str) -> bool {
        span.content == name
            && span.style.fg == theme::user_style(name).fg
            && span.style.add_modifier.contains(Modifier::BOLD)
    }

    #[test]
    fn plain_text_has_no_styled_mentions() {
        let spans = highlight_mentions("just some words", &names(), "Baughn");
        assert!(
            spans.iter().all(|s| s.style.fg.is_none()),
            "no word should be colored"
        );
        // Rejoining the span contents reproduces the chunk verbatim.
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "just some words");
    }

    #[test]
    fn leading_mention_with_colon_is_highlighted_punct_plain() {
        let spans = highlight_mentions("Baughn: hi", &names(), "Nero");
        assert!(span_is_styled_user(&spans[0], "Baughn"));
        // The colon is a separate, unstyled span.
        assert_eq!(spans[1].content, ":");
        assert!(spans[1].style.fg.is_none());
    }

    #[test]
    fn mid_sentence_mention_only_styles_the_name() {
        let spans = highlight_mentions("ask Nero please", &names(), "Baughn");
        let styled: Vec<_> = spans.iter().filter(|s| s.style.fg.is_some()).collect();
        assert_eq!(styled.len(), 1);
        assert!(span_is_styled_user(styled[0], "Nero"));
    }

    #[test]
    fn own_mention_is_additionally_reversed() {
        let spans = highlight_mentions("hi Baughn", &names(), "Baughn");
        assert!(
            spans.iter().any(|s| s.content == "Baughn"
                && s.style.add_modifier.contains(Modifier::REVERSED)),
            "self-mention should be reversed"
        );
    }

    #[test]
    fn prefix_of_a_name_is_not_a_mention() {
        // "Bau" is a completion prefix but not an exact name — never styled.
        let spans = highlight_mentions("Bau is short", &names(), "Nero");
        assert!(spans.iter().all(|s| s.style.fg.is_none()));
    }
}
