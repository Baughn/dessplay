//! The main panes: hand-rendered ratatui widgets behind tui-realm's
//! `Component`/`AppComponent` traits, driven by typed props from
//! [`super::props`] and built on the shared interaction primitives in
//! [`super::widgets`]. (We use the component model but not tui-realm's
//! threaded event listener — see ui-architecture.md, Framework Choice.)

use std::collections::HashMap;

use dessplay_core::spoiler;
use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use tuirealm::state::State;

use super::msg::Msg;
use super::props::{
    ChatLine, FranchiseRow, HealthProps, ListGroup, PlaylistProps, SeriesSort, StatusProps, Tone,
    UsersProps,
};
use super::theme;
use super::widgets::{
    Align, Binding, Cell, KeyPattern, Keymap, LineBuffer, ListCursor, RenderedList, TextField,
    render_list, table_row, truncate_display,
};

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

// The key-event matchers live in widgets::keys (with the terminal
// compatibility policy); re-exported here so panes, modals, and the
// dispatcher keep one import path.
pub(crate) use super::widgets::{plain, typed};

// ---- Chat pane ---------------------------------------------------------

/// How many visual lines PgUp/PgDown move the chat view.
const CHAT_PAGE_STEP: usize = 5;
/// How many visual lines one mouse-wheel tick moves the chat view —
/// smaller than a page, matching the wheel's fine-grained feel.
const CHAT_WHEEL_STEP: usize = 3;
/// Indent applied to wrapped continuation lines in the chat log.
const CHAT_WRAP_INDENT: usize = 2;
/// Most command suggestions shown at once in the discoverability popup.
/// Sized to fit the whole command table on a bare `/` — bump it when a
/// command is added (see [`super::commands::SLASH_COMMANDS`]).
const CHAT_SUGGESTION_MAX: u16 = 12;

/// Frames of re-randomization a spoiler click plays before settling.
const SPOILER_FRAMES: u32 = 6;
/// Milliseconds per spoiler re-randomization frame.
const SPOILER_FRAME_MS: u64 = 100;
/// A second click within this window of the first reveals the spoiler.
const SPOILER_REVEAL_WINDOW_MS: u64 = 5000;

/// Stable identity of one `||spoiler||` run: which message (shared-clock
/// millis + sender, the same pair the chat interleave already treats as
/// identity, **plus** a hash of the message text) and which run within
/// it. Position-free, so scrolling between the two reveal clicks cannot
/// retarget the state.
///
/// The text hash is keying only — [`spoiler::seed`] stays text-free so
/// the mpv OSD reproduces the same letters from (stamp, sender, index).
/// Without it, two messages colliding on (millis, sender) — the IRC
/// bridge stamps two PRIVMSGs from one TCP read with the same local
/// millisecond — would share reveal state, and revealing one would
/// silently reveal the other.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SpoilerKey {
    millis: u64,
    sender: String,
    index: usize,
    text_hash: u64,
}

impl SpoilerKey {
    fn new(millis: u64, sender: &str, index: usize, message_text: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        message_text.hash(&mut hasher);
        Self {
            millis,
            sender: sender.to_string(),
            index,
            text_hash: hasher.finish(),
        }
    }
}

/// Per-spoiler reveal state (absent from the map = hidden, generation 0).
/// Generations are monotonic across re-teases so a repeat click shows
/// fresh letters instead of replaying the same frames.
#[derive(Clone, Debug)]
enum SpoilerState {
    /// First click landed: re-randomizing, one generation per frame.
    Animating {
        anim_started: u64,
        base_gen: u32,
        generation: u32,
        first_click: u64,
    },
    /// Frames done; a second click within the window reveals.
    Armed { generation: u32, first_click: u64 },
    /// Shown in the clear for the rest of the session.
    Revealed,
}

/// A clickable spoiler region within one visual chat line. Columns are
/// relative to the line's first cell in [`wrap_chat_line`]'s output and
/// absolute screen columns once recorded in [`RenderedChatLog`].
#[derive(Clone, Debug)]
struct SpoilerHit {
    cols: std::ops::Range<u16>,
    key: SpoilerKey,
}

/// The chat log viewport the last render actually drew: its rect plus,
/// per visible body row, the clickable spoiler ranges. The chat-pane
/// analogue of [`RenderedList`] — click mapping uses render-recorded
/// geometry, never a click-time re-derivation. Zero-default misses every
/// click until the first draw.
#[derive(Default)]
struct RenderedChatLog {
    area: Rect,
    rows: Vec<Vec<SpoilerHit>>,
}

impl RenderedChatLog {
    /// The spoiler under a screen position, if any (border cells miss).
    fn hit(&self, column: u16, row: u16) -> Option<&SpoilerKey> {
        let body_row = row.checked_sub(self.area.y + 1)? as usize;
        self.rows
            .get(body_row)?
            .iter()
            .find(|hit| hit.cols.contains(&column))
            .map(|hit| &hit.key)
    }
}

/// Chat log + always-visible input line.
pub struct ChatPane {
    lines: Vec<ChatLine>,
    input: TextField,
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
    /// Per-spoiler reveal state, keyed by message identity (per-client,
    /// session-lifetime — nothing here is synced).
    spoilers: HashMap<SpoilerKey, SpoilerState>,
    /// The log viewport the last render drew (spoiler click mapping).
    rendered: RenderedChatLog,
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
            input: TextField::new("say something…"),
            focused: false,
            scroll_offset: 0,
            sent_history: Vec::new(),
            history_pos: None,
            usernames: Vec::new(),
            me: String::new(),
            completion: None,
            spoilers: HashMap::new(),
            rendered: RenderedChatLog::default(),
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

    /// A left-click landed inside the chat pane: if it hit a spoiler,
    /// advance its reveal state machine. First click starts the
    /// re-randomization tease and arms a window; a second click within
    /// [`SPOILER_REVEAL_WINDOW_MS`] of the first reveals for the session;
    /// a click after the window lapses re-teases (with fresh letters —
    /// generations are monotonic). Clicks anywhere else are no-ops here
    /// (pane focusing lives in the dispatcher).
    pub(crate) fn click(&mut self, column: u16, row: u16, now_millis: u64) {
        let Some(key) = self.rendered.hit(column, row).cloned() else {
            return;
        };
        let next = match self.spoilers.get(&key) {
            None => SpoilerState::Animating {
                anim_started: now_millis,
                base_gen: 0,
                generation: 0,
                first_click: now_millis,
            },
            Some(
                SpoilerState::Animating {
                    first_click,
                    generation,
                    ..
                }
                | SpoilerState::Armed {
                    first_click,
                    generation,
                },
            ) => {
                if now_millis.saturating_sub(*first_click) <= SPOILER_REVEAL_WINDOW_MS {
                    SpoilerState::Revealed
                } else {
                    // Window lapsed: this click is a fresh first click.
                    SpoilerState::Animating {
                        anim_started: now_millis,
                        base_gen: *generation,
                        generation: *generation,
                        first_click: now_millis,
                    }
                }
            }
            Some(SpoilerState::Revealed) => return,
        };
        self.spoilers.insert(key, next);
    }

    /// Advance every animating spoiler to the frame `now_millis` implies
    /// (derived from wall time, never per-tick increments — the marquee
    /// discipline). Returns whether a redraw could change the screen.
    pub(crate) fn advance_spoilers(&mut self, now_millis: u64) -> bool {
        let mut changed = false;
        for state in self.spoilers.values_mut() {
            let SpoilerState::Animating {
                anim_started,
                base_gen,
                generation,
                first_click,
            } = *state
            else {
                continue;
            };
            let elapsed_frames = (now_millis.saturating_sub(anim_started) / SPOILER_FRAME_MS)
                .min(u64::from(SPOILER_FRAMES));
            let next_gen = base_gen + elapsed_frames as u32;
            if next_gen == generation {
                continue;
            }
            changed = true;
            *state = if next_gen >= base_gen + SPOILER_FRAMES {
                SpoilerState::Armed {
                    generation: next_gen,
                    first_click,
                }
            } else {
                SpoilerState::Animating {
                    anim_started,
                    base_gen,
                    generation: next_gen,
                    first_click,
                }
            };
        }
        changed
    }

    /// Whether any spoiler tease is mid-animation (drives the shell's
    /// fast tick).
    pub(crate) fn spoiler_animating(&self) -> bool {
        self.spoilers
            .values()
            .any(|s| matches!(s, SpoilerState::Animating { .. }))
    }

    /// `/reveal`: reveal the newest still-hidden spoiler in the visible
    /// log (bottom = newest; hidden runs are exactly the ones that emit
    /// hit ranges). Returns whether anything was revealed.
    pub(crate) fn reveal_newest_visible(&mut self) -> bool {
        // Hidden runs are the ones that emit hit ranges, but the recorded
        // rows only refresh on the next draw — skip keys already revealed
        // so a repeat between draws can't re-pick the same run.
        let Some(key) = self
            .rendered
            .rows
            .iter()
            .rev()
            .flat_map(|row| row.iter().rev())
            .map(|hit| &hit.key)
            .find(|key| !matches!(self.spoilers.get(key), Some(SpoilerState::Revealed)))
            .cloned()
        else {
            return false;
        };
        self.spoilers.insert(key, SpoilerState::Revealed);
        true
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
        self.input.text()
    }

    /// Clear the input line (cursor and scroll reset with it —
    /// [`LineBuffer`] guarantees a cleared field never renders from a
    /// stale column).
    fn clear(&mut self) {
        self.input.clear();
    }

    /// Insert pasted text at the cursor, as if typed (design.md #33) —
    /// which means control characters are dropped, the invariant every
    /// [`LineBuffer`]-backed field enforces (typing can never produce
    /// one, and a sent message syncs its bytes to every peer's
    /// terminal). Used for a bracketed paste that isn't a playlist-add
    /// path.
    pub(crate) fn insert_text(&mut self, text: &str) {
        self.history_pos = None;
        self.input.buffer_mut().insert_paste(text);
    }

    /// Keys shown in the keybinding bar (derived from the keymap).
    pub fn keybindings(&self) -> Vec<Keybinding> {
        CHAT_KEYMAP.bar()
    }

    /// Enter: send the input as chat or a `/command`. Declines on empty.
    fn act_send(&mut self) -> Option<Msg> {
        let text = self.text().trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.clear();
        self.sent_history.push(text.clone());
        self.history_pos = None;
        self.scroll_offset = 0; // jump to newest so you see it
        Some(if text.starts_with('/') {
            Msg::Command(text)
        } else {
            Msg::SendChat(text)
        })
    }

    /// Esc: clear the input (and drop out of history recall).
    fn act_clear(&mut self) -> Option<Msg> {
        self.clear();
        self.history_pos = None;
        Some(Msg::None)
    }

    fn act_scroll_up(&mut self) -> Option<Msg> {
        self.scroll_offset += CHAT_PAGE_STEP;
        Some(Msg::None)
    }

    /// Mouse wheel over the chat column: scroll the log. Render clamps
    /// the offset to the top of the history, so over-scrolling is safe.
    pub(crate) fn scroll_wheel(&mut self, up: bool) {
        if up {
            self.scroll_offset += CHAT_WHEEL_STEP;
        } else {
            self.scroll_offset = self.scroll_offset.saturating_sub(CHAT_WHEEL_STEP);
        }
    }

    fn act_scroll_down(&mut self) -> Option<Msg> {
        self.scroll_offset = self.scroll_offset.saturating_sub(CHAT_PAGE_STEP);
        Some(Msg::None)
    }

    /// Up: recall an older message I sent. Declines with no history.
    fn act_history_prev(&mut self) -> Option<Msg> {
        if self.sent_history.is_empty() {
            return None;
        }
        let pos = match self.history_pos {
            None => self.sent_history.len() - 1,
            Some(p) => p.saturating_sub(1),
        };
        self.history_pos = Some(pos);
        self.set_input(self.sent_history[pos].clone());
        Some(Msg::None)
    }

    /// Down: walk back toward the newest, then to an empty draft.
    /// Declines when not recalling.
    fn act_history_next(&mut self) -> Option<Msg> {
        let pos = self.history_pos?;
        if pos + 1 < self.sent_history.len() {
            self.history_pos = Some(pos + 1);
            self.set_input(self.sent_history[pos + 1].clone());
        } else {
            self.history_pos = None;
            self.clear();
        }
        Some(Msg::None)
    }

    /// Load `text` into the input and park the cursor at its end.
    fn set_input(&mut self, text: String) {
        self.input.set_text(&text);
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
        // Flatten every message into wrapped visual lines (paired with
        // their clickable spoiler ranges).
        let lines: Vec<(Line, Vec<SpoilerHit>)> = self
            .lines
            .iter()
            .flat_map(|line| wrap_chat_line(line, width, &self.usernames, &self.me, &self.spoilers))
            .collect();
        let visible = log_area.height.saturating_sub(2) as usize;
        // Clamp the scroll so it can never run past the top of the log.
        let max_offset = lines.len().saturating_sub(visible);
        self.scroll_offset = self.scroll_offset.min(max_offset);
        let end = lines.len().saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(visible);
        // Record the drawn viewport for spoiler click mapping: shift the
        // line-relative hit columns to absolute screen columns (past the
        // left border); body row i sits at log_area.y + 1 + i.
        self.rendered = RenderedChatLog {
            area: log_area,
            rows: lines[start..end]
                .iter()
                .map(|(_, hits)| {
                    hits.iter()
                        .map(|hit| SpoilerHit {
                            cols: hit.cols.start + log_area.x + 1..hit.cols.end + log_area.x + 1,
                            key: hit.key.clone(),
                        })
                        .collect()
                })
                .collect(),
        };
        let items: Vec<ListItem> = lines[start..end]
            .iter()
            .map(|(line, _)| ListItem::new(line.clone()))
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
        self.input.render(frame, input_area, self.focused, false);
    }
}

/// Greedy word-wrap over **display width** (terminal cells, via
/// `unicode-width`) — ratatui lays out by cell width, so double-width
/// CJK must consume two cells of budget, not one. The first visual line
/// gets `first_width` cells (the chat prefix eats into it); later lines
/// get `rest_width`. Breaks at spaces where possible, hard-breaks any
/// word wider than the available cells.
///
/// Each chunk is a contiguous char-slice of `text` (only boundary join
/// spaces are dropped); the second tuple element is the chunk's starting
/// **char offset** in `text` (identity, not geometry), which lets
/// callers map char ranges of the input (spoiler runs) onto the wrapped
/// lines.
fn wrap_body(text: &str, first_width: usize, rest_width: usize) -> Vec<(String, usize)> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let width_for = |idx: usize| if idx == 0 { first_width } else { rest_width }.max(1);
    let mut lines: Vec<(String, usize)> = Vec::new();
    let mut cur = String::new();
    let mut cur_start = 0;
    let mut width = width_for(0);
    let mut next_word_start = 0;
    for mut word in text.split(' ') {
        let mut word_start = next_word_start;
        next_word_start += word.chars().count() + 1;
        loop {
            let cur_cells = cur.width();
            let space = usize::from(!cur.is_empty());
            let word_cells = word.width();
            if cur_cells + space + word_cells <= width {
                if space == 1 {
                    cur.push(' ');
                } else {
                    // Chunk begins with this word.
                    cur_start = word_start;
                }
                cur.push_str(word);
                break;
            }
            if cur.is_empty() {
                // Word alone exceeds the line: hard-break it after the
                // last char that still fits `width` cells — but always
                // after at least one, so a single over-wide char cannot
                // stall the loop.
                let mut used = 0;
                let mut split_at = word.len();
                for (taken, (i, c)) in word.char_indices().enumerate() {
                    let cells = c.width().unwrap_or(0);
                    if taken > 0 && used + cells > width {
                        split_at = i;
                        break;
                    }
                    used += cells;
                }
                let (head, tail) = word.split_at(split_at);
                cur_start = word_start;
                cur.push_str(head);
                lines.push((std::mem::take(&mut cur), cur_start));
                width = width_for(lines.len());
                word_start += head.chars().count();
                word = tail;
            } else {
                // Flush and retry the word on a fresh line.
                lines.push((std::mem::take(&mut cur), cur_start));
                width = width_for(lines.len());
            }
        }
    }
    lines.push((cur, cur_start));
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
fn highlight_mentions(
    chunk: &str,
    usernames: &[String],
    me: &str,
    base: Style,
) -> Vec<Span<'static>> {
    use tuirealm::ratatui::style::Modifier;
    let mut spans: Vec<Span<'static>> = Vec::new();
    // Walk space-separated tokens, re-emitting the single spaces between them.
    let mut first = true;
    for token in chunk.split(' ') {
        if !first {
            spans.push(Span::styled(" ", base));
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
                    spans.push(Span::styled(punct.to_string(), base));
                }
            }
            None => spans.push(Span::styled(token.to_string(), base)),
        }
    }
    spans
}

/// A spoiler run's position within a composed display body: its char
/// range, identity, and — while hidden — the `(seed, generation)` that
/// scrambled it (`None` = revealed, shown in the clear).
struct SpoilerRun {
    chars: std::ops::Range<usize>,
    key: SpoilerKey,
    hidden: Option<(u64, u32)>,
}

/// Compose the display body for a chat message: `||spoiler||` runs
/// replaced by their scramble (or the original once revealed), bars
/// dropped, everything else verbatim. Returns the display string plus
/// each run's char range within it. The scramble is 1:1 in chars, so
/// run char ranges map exactly onto the display text; geometry (wrap
/// budgets, hit columns) is then measured in display cells over this
/// composed text — never the source, whose widths the scramble changes
/// (a double-width CJK char becomes one single-width ASCII letter).
fn compose_spoiler_body(
    line: &ChatLine,
    spoilers: &HashMap<SpoilerKey, SpoilerState>,
) -> (String, Vec<SpoilerRun>) {
    if !line.text.contains("||") {
        return (line.text.clone(), Vec::new());
    }
    let mut display = String::new();
    let mut chars = 0;
    let mut runs = Vec::new();
    let mut index = 0;
    for segment in spoiler::parse(&line.text) {
        match segment {
            spoiler::Segment::Text(t) => {
                display.push_str(t);
                chars += t.chars().count();
            }
            spoiler::Segment::Spoiler(s) => {
                let key = SpoilerKey::new(line.millis, &line.sender, index, &line.text);
                let hidden = match spoilers.get(&key) {
                    Some(SpoilerState::Revealed) => None,
                    Some(
                        SpoilerState::Animating { generation, .. }
                        | SpoilerState::Armed { generation, .. },
                    ) => Some((spoiler::seed(line.millis, &line.sender, index), *generation)),
                    None => Some((spoiler::seed(line.millis, &line.sender, index), 0)),
                };
                match hidden {
                    Some((seed, generation)) => {
                        display.push_str(&spoiler::scramble(s, seed, generation));
                    }
                    None => display.push_str(s),
                }
                let len = s.chars().count();
                runs.push(SpoilerRun {
                    chars: chars..chars + len,
                    key,
                    hidden,
                });
                chars += len;
                index += 1;
            }
        }
    }
    (display, runs)
}

/// Style one wrapped chunk of a composed body: mention highlighting
/// outside spoiler runs, zalgo'd scramble inside hidden ones, plain
/// `base` inside revealed ones. Returns the spans plus the hidden runs'
/// hit ranges, in **display cells** relative to the chunk's first cell —
/// ratatui advances by cell width, so hit geometry must too (the zalgo
/// marks are zero-width — added here, after wrapping, precisely so they
/// can't perturb the columns).
fn spoiler_chunk_spans(
    chunk: &str,
    chunk_start: usize,
    runs: &[SpoilerRun],
    usernames: &[String],
    me: &str,
    base: Style,
) -> (Vec<Span<'static>>, Vec<SpoilerHit>) {
    use unicode_width::UnicodeWidthChar;
    let chunk_chars: Vec<char> = chunk.chars().collect();
    let chunk_end = chunk_start + chunk_chars.len();
    // Slice by chunk-relative char positions.
    let slice = |a: usize, b: usize| -> String { chunk_chars[a..b].iter().collect() };
    // Display column of a chunk-relative char index: hits are screen
    // geometry, so they advance by cell width, not char count.
    let col_of = |idx: usize| -> usize {
        chunk_chars[..idx]
            .iter()
            .map(|c| c.width().unwrap_or(0))
            .sum()
    };
    let mut spans = Vec::new();
    let mut hits = Vec::new();
    let mut pos = chunk_start;
    for run in runs {
        let start = run.chars.start.max(chunk_start);
        let end = run.chars.end.min(chunk_end);
        if start >= end {
            continue;
        }
        if start > pos {
            spans.extend(highlight_mentions(
                &slice(pos - chunk_start, start - chunk_start),
                usernames,
                me,
                base,
            ));
        }
        let text = slice(start - chunk_start, end - chunk_start);
        match run.hidden {
            Some((seed, generation)) => {
                let corrupted = spoiler::zalgo(&text, seed, generation, start - run.chars.start);
                spans.push(Span::styled(corrupted, theme::spoiler()));
                hits.push(SpoilerHit {
                    cols: col_of(start - chunk_start) as u16..col_of(end - chunk_start) as u16,
                    key: run.key.clone(),
                });
            }
            None => spans.push(Span::styled(text, base)),
        }
        pos = end;
    }
    if pos < chunk_end {
        spans.extend(highlight_mentions(
            &slice(pos - chunk_start, chunk_end - chunk_start),
            usernames,
            me,
            base,
        ));
    }
    (spans, hits)
}

/// Render one chat message as one or more wrapped visual lines, each
/// paired with its clickable spoiler ranges (relative columns; empty for
/// the line kinds that never carry spoilers).
fn wrap_chat_line(
    line: &ChatLine,
    width: usize,
    usernames: &[String],
    me: &str,
    spoilers: &HashMap<SpoilerKey, SpoilerState>,
) -> Vec<(Line<'static>, Vec<SpoilerHit>)> {
    use tuirealm::ratatui::style::Modifier;
    use unicode_width::UnicodeWidthStr;
    let indent: String = " ".repeat(CHAT_WRAP_INDENT);
    if line.separator {
        // Render-time day divider: the date label centered between dashes.
        let label = format!(" {} ", line.text);
        let label_w = label.width();
        let total = width.max(label_w);
        let dashes = total - label_w;
        let left = dashes / 2;
        let bar = format!("{}{}{}", "─".repeat(left), label, "─".repeat(dashes - left));
        return vec![(Line::from(Span::styled(bar, theme::dim())), Vec::new())];
    }
    if line.subtitle {
        // Local subtitle (Intermixed mode): dim, no sender, "»" marker,
        // in-video timestamp.
        let time = format!("{} ", line.time);
        let prefix_width = time.width();
        let body = format!("» {}", line.text);
        let chunks = wrap_body(
            &body,
            width.saturating_sub(prefix_width),
            width.saturating_sub(CHAT_WRAP_INDENT),
        );
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, (chunk, _))| {
                let visual = if i == 0 {
                    Line::from(vec![
                        Span::styled(time.clone(), theme::dim()),
                        Span::styled(chunk, theme::dim()),
                    ])
                } else {
                    Line::from(Span::styled(format!("{indent}{chunk}"), theme::dim()))
                };
                (visual, Vec::new())
            })
            .collect()
    } else if line.system {
        // Local system notice: dim, no sender, "*" marker.
        let time = format!("{} ", line.time);
        let prefix_width = time.width();
        let body = format!("* {}", line.text);
        let chunks = wrap_body(
            &body,
            width.saturating_sub(prefix_width),
            width.saturating_sub(CHAT_WRAP_INDENT),
        );
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, (chunk, _))| {
                let visual = if i == 0 {
                    Line::from(vec![
                        Span::styled(time.clone(), theme::dim()),
                        Span::styled(chunk, theme::dim()),
                    ])
                } else {
                    Line::from(Span::styled(format!("{indent}{chunk}"), theme::dim()))
                };
                (visual, Vec::new())
            })
            .collect()
    } else if line.action {
        // IRC-style action: "* sender phrase", no colon. The "* " is dim,
        // the sender keeps its per-user color/bold, the phrase is raw. A
        // bridged IRC action also carries a dim "irc" tag.
        let time = format!("{} ", line.time);
        let tag = if line.irc { "irc " } else { "" };
        let marker = "* ";
        let sender = format!("{} ", line.sender);
        // Display cells, not chars: a CJK sender name is two cells wide
        // per char, and the hit columns offset by this prefix.
        let prefix_width = time.width() + tag.width() + marker.width() + sender.width();
        let (display, runs) = compose_spoiler_body(line, spoilers);
        let chunks = wrap_body(
            &display,
            width.saturating_sub(prefix_width),
            width.saturating_sub(CHAT_WRAP_INDENT),
        );
        let sender_style = theme::user_style(&line.sender).add_modifier(Modifier::BOLD);
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, (chunk, chunk_start))| {
                // The action phrase renders grey (#27) — the terminal has
                // no italics, so colour is what marks an emote. Mentions
                // still highlight through it.
                let (body, mut hits) =
                    spoiler_chunk_spans(&chunk, chunk_start, &runs, usernames, me, theme::dim());
                let offset = (if i == 0 {
                    prefix_width
                } else {
                    CHAT_WRAP_INDENT
                }) as u16;
                for hit in &mut hits {
                    hit.cols = hit.cols.start + offset..hit.cols.end + offset;
                }
                let visual = if i == 0 {
                    let mut spans = vec![Span::styled(time.clone(), theme::dim())];
                    if !tag.is_empty() {
                        spans.push(Span::styled(tag, theme::dim()));
                    }
                    spans.push(Span::styled(marker, theme::dim()));
                    spans.push(Span::styled(sender.clone(), sender_style));
                    spans.extend(body);
                    Line::from(spans)
                } else {
                    let mut spans = vec![Span::raw(indent.clone())];
                    spans.extend(body);
                    Line::from(spans)
                };
                (visual, hits)
            })
            .collect()
    } else {
        // Normal chat. A bridged IRC message is rendered identically
        // (colored sender, mention highlight) but with a dim "irc" tag so
        // it isn't mistaken for a dessplay peer.
        let time = format!("{} ", line.time);
        let tag = if line.irc { "irc " } else { "" };
        let sender = format!("{}: ", line.sender);
        // Display cells, not chars — see the action branch above.
        let prefix_width = time.width() + tag.width() + sender.width();
        let (display, runs) = compose_spoiler_body(line, spoilers);
        let chunks = wrap_body(
            &display,
            width.saturating_sub(prefix_width),
            width.saturating_sub(CHAT_WRAP_INDENT),
        );
        let sender_style = theme::user_style(&line.sender).add_modifier(Modifier::BOLD);
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, (chunk, chunk_start))| {
                let (body, mut hits) = spoiler_chunk_spans(
                    &chunk,
                    chunk_start,
                    &runs,
                    usernames,
                    me,
                    Style::default(),
                );
                let offset = (if i == 0 {
                    prefix_width
                } else {
                    CHAT_WRAP_INDENT
                }) as u16;
                for hit in &mut hits {
                    hit.cols = hit.cols.start + offset..hit.cols.end + offset;
                }
                let visual = if i == 0 {
                    let mut spans = vec![Span::styled(time.clone(), theme::dim())];
                    if !tag.is_empty() {
                        spans.push(Span::styled(tag, theme::dim()));
                    }
                    spans.push(Span::styled(sender.clone(), sender_style));
                    spans.extend(body);
                    Line::from(spans)
                } else {
                    let mut spans = vec![Span::raw(indent.clone())];
                    spans.extend(body);
                    Line::from(spans)
                };
                (visual, hits)
            })
            .collect()
    }
}

/// Test seams for the Ui-level clock-domain tests (`app.rs`), which
/// need a clickable spoiler without a real draw and a view of the
/// animation state `ChatPane` keeps private.
#[cfg(test)]
impl ChatPane {
    /// Pretend the last render drew one spoiler hit at columns 5..10 of
    /// body row 0 (screen row 1).
    pub(crate) fn test_install_spoiler_hit(&mut self) {
        self.rendered = RenderedChatLog {
            area: Rect::new(0, 0, 40, 10),
            rows: vec![vec![SpoilerHit {
                cols: 5..10,
                key: SpoilerKey::new(1_000, "kim", 0, "||x||"),
            }]],
        };
    }

    /// The animation generation of every tracked spoiler (revealed runs
    /// report `u32::MAX`), in map order.
    pub(crate) fn test_spoiler_generations(&self) -> Vec<u32> {
        self.spoilers
            .values()
            .map(|s| match s {
                SpoilerState::Animating { generation, .. }
                | SpoilerState::Armed { generation, .. } => *generation,
                SpoilerState::Revealed => u32::MAX,
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
        if let Some(c) = typed(ev) {
            // Typing detaches from history recall (shell behavior).
            self.history_pos = None;
            self.input.insert(c);
            return Some(Msg::None);
        }
        if let Some(msg) = CHAT_KEYMAP.dispatch(self, ev) {
            return Some(msg);
        }
        // Cursor motion, deletion, word ops — the vocabulary every text
        // field shares (widgets::LineBuffer::edit).
        if self.input.edit(ev) {
            return Some(Msg::None);
        }
        None
    }
}

/// Chat bindings: dispatch and the keybinding bar derive from this one
/// table, so the bar cannot lie about what the keys do.
static CHAT_KEYMAP: Keymap<ChatPane, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Send")),
        action: ChatPane::act_send,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::PageUp),
        bar: Some(("PgUp/Dn", "Scroll")),
        action: ChatPane::act_scroll_up,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::PageDown),
        bar: None,
        action: ChatPane::act_scroll_down,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Up),
        bar: Some(("↑↓", "History")),
        action: ChatPane::act_history_prev,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Down),
        bar: None,
        action: ChatPane::act_history_next,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: Some(("Esc", "Clear")),
        action: ChatPane::act_clear,
    },
]);

// ---- Users pane --------------------------------------------------------

/// Colored ready states + dim departed/seeder lines.
#[derive(Default)]
pub struct UsersPane {
    props: UsersProps,
    cursor: ListCursor,
    focused: bool,
    /// The viewport of the last render, for mouse hit-testing.
    rendered: RenderedList,
}

impl UsersPane {
    /// Replace props, clamping the selection (rows + selectable
    /// known-offline entries -- see `selectable_len`).
    pub fn set_props(&mut self, props: UsersProps) {
        self.props = props;
        self.cursor.clamp(self.selectable_len());
    }

    /// Keys shown in the keybinding bar: the structural list-navigation
    /// entry plus the keymap's own.
    pub fn keybindings(&self) -> Vec<Keybinding> {
        let mut items = vec![("↑↓", "Select")];
        items.extend(USERS_KEYMAP.bar());
        items
    }

    /// The selected username, whether it's a live `rows` entry or a
    /// selectable `known_offline` one (design.md #15 -- both are valid
    /// `a`/`n` targets).
    fn selected_username(&self) -> Option<String> {
        let index = self.cursor.index();
        if let Some(row) = self.props.rows.get(index) {
            return Some(row.name.clone());
        }
        self.props
            .known_offline
            .get(index - self.props.rows.len())
            .map(|row| row.name.clone())
    }

    /// `a`: mark the selected user Away (or clear an Away we set).
    fn act_away(&mut self) -> Option<Msg> {
        Some(Msg::ToggleAway(dessplay_core::types::UserId::new(
            self.selected_username()?,
        )))
    }

    /// `n`: mark the selected user NotWatching for the now-playing series
    /// (design.md #7/#13 — the "Kim tool": rule on someone's commitment
    /// without waiting for them to show up).
    fn act_not_watching(&mut self) -> Option<Msg> {
        Some(Msg::SetNotWatching(dessplay_core::types::UserId::new(
            self.selected_username()?,
        )))
    }

    /// Rows + known-offline entries are one selectable range; seeders never
    /// are.
    fn selectable_len(&self) -> usize {
        self.props.rows.len() + self.props.known_offline.len()
    }

    /// A left-click at (column, row): select the row under the pointer,
    /// per the recorded last-render viewport. The seeders line renders
    /// below the selectable range and is rejected as a target (it is
    /// display-only, exactly as for keyboard selection).
    pub(crate) fn click(&mut self, column: u16, row: u16) {
        if let Some(index) = self.rendered.hit(column, row)
            && index < self.selectable_len()
        {
            self.cursor.set(index);
        }
    }

    /// A mouse-wheel tick over the pane: move the selection like Up/Down.
    pub(crate) fn scroll_wheel(&mut self, up: bool) {
        let key = if up { Key::Up } else { Key::Down };
        self.cursor.nav(key, self.selectable_len());
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
        // Dim + italic, one row per known-offline user (design.md #15) --
        // selectable, unlike the seeders line below.
        items.extend(self.props.known_offline.iter().map(|row| {
            ListItem::new(Span::styled(
                format!("{} (last seen {})", row.name, row.last_seen_label),
                theme::tone_style(Tone::Muted)
                    .add_modifier(tuirealm::ratatui::style::Modifier::ITALIC),
            ))
        }));
        if !self.props.seeders.is_empty() {
            items.push(ListItem::new(Span::styled(
                format!("seeders: {}", self.props.seeders.join(", ")),
                theme::tone_style(Tone::Muted),
            )));
        }
        let selected = (self.focused && self.selectable_len() > 0).then(|| self.cursor.index());
        self.rendered = render_list(
            frame,
            area,
            "Users",
            items,
            selected,
            self.focused,
            Some(self.cursor.index()),
        );
    }
}

passive_component!(UsersPane);

impl AppComponent<Msg, NoUserEvent> for UsersPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some(key) = plain(ev)
            && self.cursor.nav(key, self.selectable_len())
        {
            return Some(Msg::None);
        }
        USERS_KEYMAP.dispatch(self, ev)
    }
}

/// Users-pane bindings.
static USERS_KEYMAP: Keymap<UsersPane, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Char('a'),
        bar: Some(("a", "Mark away")),
        action: UsersPane::act_away,
    },
    Binding {
        pattern: KeyPattern::Char('n'),
        bar: Some(("n", "Not watching")),
        action: UsersPane::act_not_watching,
    },
]);

// ---- Playlist pane -----------------------------------------------------

/// Shared playlist; trailing `[Add New]` row.
#[derive(Default)]
pub struct PlaylistPane {
    props: PlaylistProps,
    cursor: ListCursor,
    focused: bool,
    /// The viewport of the last render, for mouse hit-testing.
    rendered: RenderedList,
}

impl PlaylistPane {
    /// Replace props, clamping the selection (rows + the Add New row).
    pub fn set_props(&mut self, props: PlaylistProps) {
        self.props = props;
        self.cursor.clamp(self.props.rows.len() + 1);
    }

    /// The hash under the cursor, if it's a real row. `pub(crate)` so
    /// `Ui::handle`'s paste-add path (design.md #33) can anchor a pasted
    /// path after the same entry the `a` key would.
    pub(crate) fn selected_hash(&self) -> Option<dessplay_core::types::Ed2kHash> {
        self.props.rows.get(self.cursor.index()).map(|row| row.hash)
    }

    /// Keys shown in the keybinding bar (derived from the keymap).
    pub fn keybindings(&self) -> Vec<Keybinding> {
        PLAYLIST_KEYMAP.bar()
    }

    /// Enter: play the selected entry, or add on the [Add New] row.
    fn act_play(&mut self) -> Option<Msg> {
        Some(match self.selected_hash() {
            Some(hash) => Msg::PlaySelected(hash),
            None => Msg::AddFileAfter(None),
        })
    }

    fn act_add(&mut self) -> Option<Msg> {
        Some(Msg::AddFileAfter(self.selected_hash()))
    }

    fn act_nyaa(&mut self) -> Option<Msg> {
        Some(Msg::OpenNyaa(self.selected_hash()))
    }

    fn act_remove(&mut self) -> Option<Msg> {
        self.selected_hash().map(Msg::RemoveEntry)
    }

    /// `w`: cycle the entry's series watch state: Watching -> Maybe ->
    /// NotWatching -> ...
    fn act_watch(&mut self) -> Option<Msg> {
        self.selected_hash().map(Msg::CycleSeriesWatch)
    }

    /// `j`/`J`: move the selected entry down, carrying the cursor with it
    /// so repeated presses keep moving the same episode (the reorder is
    /// reflected via the forced UI refresh, so the cursor lands on the
    /// moved entry). Declines on the bottom row / [Add New].
    fn act_move_down(&mut self) -> Option<Msg> {
        let hash = self.selected_hash()?;
        let index = self.cursor.index();
        if index + 1 >= self.props.rows.len() {
            return None; // already the bottom row
        }
        self.cursor.set(index + 1);
        Some(Msg::MoveDown(hash))
    }

    /// `k`/`K`: move the selected entry up (see `act_move_down`).
    fn act_move_up(&mut self) -> Option<Msg> {
        let hash = self.selected_hash()?;
        let index = self.cursor.index();
        if index == 0 {
            return None; // already the top row
        }
        self.cursor.set(index - 1);
        Some(Msg::MoveUp(hash))
    }

    /// `M`: manually map the entry to a local file.
    fn act_map(&mut self) -> Option<Msg> {
        self.selected_hash().map(Msg::MapFile)
    }

    /// A left-click at (column, row): select the row under the pointer
    /// (the trailing [Add New] row included), per the recorded
    /// last-render viewport — which already embodies the pane's
    /// centering policy (now-playing while unfocused, the cursor while
    /// focused).
    pub(crate) fn click(&mut self, column: u16, row: u16) {
        if let Some(index) = self.rendered.hit(column, row) {
            self.cursor.set(index);
        }
    }

    /// A mouse-wheel tick over the pane: move the selection like Up/Down.
    pub(crate) fn scroll_wheel(&mut self, up: bool) {
        let key = if up { Key::Up } else { Key::Down };
        self.cursor.nav(key, self.props.rows.len() + 1);
    }

    /// `A`: archive — only cache-only ("temporary") rows.
    fn act_archive(&mut self) -> Option<Msg> {
        let row = self.props.rows.get(self.cursor.index())?;
        row.temporary.then_some(Msg::ArchiveFile(row.hash))
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let inner = area.width.saturating_sub(2) as usize;
        // Table columns after the title: an optional "temp" column (only
        // reserved when some row is cache-only) and the always-shown
        // watch state, sized to the widest tag on screen. The title cell
        // truncates — a long filename must not shove the columns off the
        // pane.
        let watch_tag = |row: &crate::ui::props::PlaylistRow| match row.watch {
            dessplay_core::types::SeriesWatchState::Watching => "watching",
            dessplay_core::types::SeriesWatchState::Maybe => "maybe",
            dessplay_core::types::SeriesWatchState::NotWatching => "not watching",
        };
        let show_temp = self.props.rows.iter().any(|row| row.temporary);
        // Our own background downloads surface here: a percentage column,
        // reserved only while something is actually downloading.
        let show_download = self.props.rows.iter().any(|row| row.download.is_some());
        let watch_width = self
            .props
            .rows
            .iter()
            .map(|row| watch_tag(row).len())
            .max()
            .unwrap_or(0);
        let mut items: Vec<ListItem> = self
            .props
            .rows
            .iter()
            .map(|row| {
                let marker = if row.is_now { "▶ " } else { "  " };
                let style = theme::tone_style(row.tone);
                let flex = vec![Span::styled(format!("{marker}{}", row.title), style)];
                let mut cells = Vec::new();
                if show_download {
                    let text = row
                        .download
                        .map(|bps| format!("{}%", bps / 100))
                        .unwrap_or_default();
                    cells.push(Cell::new(
                        text,
                        theme::tone_style(Tone::Transfer),
                        4,
                        Align::Right,
                    ));
                }
                if show_temp {
                    let text = if row.temporary { "temp" } else { "" };
                    cells.push(Cell::new(text, theme::dim(), 4, Align::Left));
                }
                cells.push(Cell::new(
                    watch_tag(row),
                    theme::dim(),
                    watch_width,
                    Align::Left,
                ));
                ListItem::new(table_row(inner, flex, cells))
            })
            .collect();
        items.push(ListItem::new(Span::styled("  [Add New]", theme::dim())));
        let selected = self.focused.then(|| self.cursor.index());
        let center = if self.focused {
            Some(self.cursor.index())
        } else {
            self.props.now_index
        };
        self.rendered = render_list(
            frame,
            area,
            "Playlist",
            items,
            selected,
            self.focused,
            center,
        );
    }
}

passive_component!(PlaylistPane);

impl AppComponent<Msg, NoUserEvent> for PlaylistPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        // Rows plus the trailing [Add New].
        if let Some(key) = plain(ev)
            && self.cursor.nav(key, self.props.rows.len() + 1)
        {
            return Some(Msg::None);
        }
        PLAYLIST_KEYMAP.dispatch(self, ev)
    }
}

/// Playlist bindings. Reorder and Map use bare letters rather than
/// Ctrl-J/Ctrl-K/Ctrl-M because those collide with control codes
/// (Ctrl-J == LF, Ctrl-M == Enter) in terminals lacking the enhanced
/// keyboard protocol.
static PLAYLIST_KEYMAP: Keymap<PlaylistPane, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Play")),
        action: PlaylistPane::act_play,
    },
    Binding {
        pattern: KeyPattern::Char('a'),
        bar: Some(("a", "Add")),
        action: PlaylistPane::act_add,
    },
    Binding {
        pattern: KeyPattern::Char('n'),
        bar: Some(("n", "Nyaa")),
        action: PlaylistPane::act_nyaa,
    },
    Binding {
        pattern: KeyPattern::Char('d'),
        bar: Some(("d", "Remove")),
        action: PlaylistPane::act_remove,
    },
    Binding {
        pattern: KeyPattern::Char('w'),
        bar: Some(("w", "Watch")),
        action: PlaylistPane::act_watch,
    },
    Binding {
        pattern: KeyPattern::Chars(&['j', 'J']),
        bar: Some(("J/K", "Move")),
        action: PlaylistPane::act_move_down,
    },
    Binding {
        pattern: KeyPattern::Chars(&['k', 'K']),
        bar: None,
        action: PlaylistPane::act_move_up,
    },
    Binding {
        pattern: KeyPattern::Char('M'),
        bar: Some(("M", "Map")),
        action: PlaylistPane::act_map,
    },
    Binding {
        pattern: KeyPattern::Char('A'),
        bar: Some(("A", "Archive")),
        action: PlaylistPane::act_archive,
    },
]);

// ---- Series pane -------------------------------------------------------

/// The pane's three modes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SeriesMode {
    /// Franchises by recency.
    Recent,
    /// Franchises alphabetical / by year.
    All,
    /// The List. Default mode (design.md, Adding Files to the Playlist):
    /// the spreadsheet view is the day-to-day "what are we watching"
    /// surface.
    #[default]
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
    /// default. Empty in The List mode. A full [`LineBuffer`], so the
    /// filter edits exactly like every other text field.
    filter: LineBuffer,
    /// Whether we're editing the filter. Gated behind `/` (rather than
    /// typing directly) so the bare `m` / `s` mode/sort keys stay live —
    /// and reliable: Ctrl-modified letters collide with control codes
    /// (Ctrl-M == Enter) in terminals without the enhanced keyboard
    /// protocol, so they can't be used for the binding.
    filtering: bool,
    cursor: ListCursor,
    focused: bool,
    /// The viewport of the last render, for mouse hit-testing.
    rendered: RenderedList,
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

    /// Seed the sort order (from the persisted setting at startup).
    pub fn set_sort(&mut self, sort: SeriesSort) {
        self.sort = sort;
    }

    /// Current type-to-filter text (Recent / All modes).
    pub fn filter(&self) -> String {
        self.filter.text()
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
        self.cursor.clamp(self.len());
    }

    /// The active keymap: per mode, with a dedicated one while the
    /// filter is being edited (letters must type, not bind).
    fn keymap(&self) -> &'static Keymap<SeriesPane, Msg> {
        if self.filtering {
            &SERIES_FILTERING_KEYMAP
        } else {
            match self.mode {
                SeriesMode::Recent => &SERIES_RECENT_KEYMAP,
                SeriesMode::All => &SERIES_ALL_KEYMAP,
                SeriesMode::TheList => &SERIES_LIST_KEYMAP,
            }
        }
    }

    /// Keys shown in the keybinding bar: derived from the active keymap,
    /// plus the structural "type to filter" entry while filtering (the
    /// edit fall-through exists exactly when `filtering` is set).
    pub fn keybindings(&self) -> Vec<Keybinding> {
        let mut items = if self.filtering {
            vec![("type", "Filter")]
        } else {
            Vec::new()
        };
        items.extend(self.keymap().bar());
        items
    }

    /// `m`: cycle Recent -> All -> The List.
    fn act_mode(&mut self) -> Option<Msg> {
        self.mode = match self.mode {
            SeriesMode::Recent => SeriesMode::All,
            SeriesMode::All => SeriesMode::TheList,
            SeriesMode::TheList => SeriesMode::Recent,
        };
        self.filter.clear();
        self.cursor.reset();
        Some(Msg::CycleSeriesMode)
    }

    /// `s` (All mode): toggle title/year sort.
    fn act_sort(&mut self) -> Option<Msg> {
        self.sort = match self.sort {
            SeriesSort::Title => SeriesSort::Year,
            SeriesSort::Year => SeriesSort::Title,
        };
        Some(Msg::ToggleSeriesSort)
    }

    /// `/`: begin editing the filter.
    fn act_filter_start(&mut self) -> Option<Msg> {
        self.filtering = true;
        Some(Msg::None)
    }

    /// Esc outside filter editing: clear a set filter. Declines when no
    /// filter is set.
    fn act_filter_clear(&mut self) -> Option<Msg> {
        if self.filter.is_empty() {
            return None;
        }
        self.filter.clear();
        self.cursor.reset();
        Some(Msg::SeriesFilterChanged)
    }

    /// Backspace while filtering: on an *empty* filter, exit filtering
    /// (the escape hatch alongside Esc). With text present it declines so
    /// the shared editor deletes a character instead.
    fn act_filter_backspace_exit(&mut self) -> Option<Msg> {
        if !self.filter.is_empty() {
            return None;
        }
        self.filtering = false;
        self.cursor.reset();
        Some(Msg::SeriesFilterChanged)
    }

    /// Esc while filtering: clear the filter and stop editing it.
    fn act_filter_esc(&mut self) -> Option<Msg> {
        self.filter.clear();
        self.filtering = false;
        self.cursor.reset();
        Some(Msg::SeriesFilterChanged)
    }

    /// Enter (Recent / All, filtering or not): browse the franchise.
    fn act_browse(&mut self) -> Option<Msg> {
        let row = self.franchises.get(self.cursor.index())?;
        Some(Msg::BrowseFranchise(row.key.clone()))
    }

    /// Enter (The List): toggle a heading, open a linked entry, or edit
    /// an unlinked one.
    fn act_list_enter(&mut self) -> Option<Msg> {
        match self.nav_rows().get(self.cursor.index())? {
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
                    None => Some(Msg::BrowseUnlinkedListEntry(entry.id)),
                }
            }
        }
    }

    /// `e` (The List): edit the selected entry.
    fn act_list_edit(&mut self) -> Option<Msg> {
        match self.nav_rows().get(self.cursor.index())? {
            ListNavRow::Entry(g, e) => Some(Msg::EditListEntry(self.groups[*g].rows[*e].id)),
            ListNavRow::Heading(_) => None,
        }
    }

    /// `l` (The List): link the selected entry to AniDB.
    fn act_list_link(&mut self) -> Option<Msg> {
        match self.nav_rows().get(self.cursor.index())? {
            ListNavRow::Entry(g, e) => Some(Msg::LinkListEntry(self.groups[*g].rows[*e].id)),
            ListNavRow::Heading(_) => None,
        }
    }

    /// A left-click at (column, row): select the row under the pointer
    /// (headings included — Enter toggles them, same as keyboard
    /// selection), per the recorded last-render viewport.
    pub(crate) fn click(&mut self, column: u16, row: u16) {
        if let Some(index) = self.rendered.hit(column, row) {
            self.cursor.set(index);
        }
    }

    /// A mouse-wheel tick over the pane: move the selection like Up/Down.
    pub(crate) fn scroll_wheel(&mut self, up: bool) {
        let key = if up { Key::Up } else { Key::Down };
        self.cursor.nav(key, self.len());
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let base = match self.mode {
            SeriesMode::Recent => "Recent Series",
            SeriesMode::All => "All Series",
            SeriesMode::TheList => "The List",
        };
        // Surface the filter so typing is visible (no silent state); show
        // the `/` cue the moment filtering starts, even before any text.
        // While editing, the filter's cursor renders as a reversed cell —
        // it is a full text field (word motion, Home/End), not append-only.
        let title: Line =
            if self.mode != SeriesMode::TheList && (self.filtering || !self.filter.is_empty()) {
                let mut spans = vec![Span::raw(format!("{base}  /"))];
                if self.filtering {
                    spans.extend(self.filter.cursor_spans());
                } else {
                    spans.push(Span::raw(self.filter.text()));
                }
                Line::from(spans)
            } else {
                Line::from(base)
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
                        // A plain table: episode #, out-this-week, and
                        // watchers are the spreadsheet's load-bearing
                        // columns, so they get fixed-width, aligned cells
                        // instead of drifting with the name's length
                        // (`table_row` truncates the name cell and keeps
                        // the columns put).
                        const EP_WIDTH: usize = 8;
                        const AVAIL_WIDTH: usize = 3;
                        const WATCHERS_WIDTH: usize = 10;
                        let inner = area.width.saturating_sub(2) as usize;

                        let entry = &self.groups[*g].rows[*e];
                        let mut flex = vec![Span::raw("  ")];
                        // A search that came up empty is a durable "AniDB
                        // doesn't have this" callout (design.md, Series
                        // Identity) -- distinct from an unlinked entry
                        // nobody's tried linking yet, which gets no marker.
                        if entry.series_id.is_none() && entry.anidb_unavailable {
                            flex.push(Span::styled("⊘ ", theme::dim()));
                        }
                        flex.push(Span::raw(entry.name.clone()));
                        if let Some(nero) = &entry.nero_name {
                            flex.push(Span::styled(format!(" “{nero}”"), theme::dim()));
                        }

                        let avail_text = if entry.next_ep.is_some() && entry.available {
                            "✓"
                        } else {
                            ""
                        };
                        let cells = vec![
                            Cell::new(
                                entry.next_ep.as_deref().unwrap_or(""),
                                theme::tone_style(Tone::Normal),
                                EP_WIDTH,
                                Align::Left,
                            ),
                            Cell::new(
                                avail_text,
                                theme::tone_style(Tone::Good),
                                AVAIL_WIDTH,
                                Align::Center,
                            ),
                            Cell::new(
                                entry.watchers.clone(),
                                theme::dim(),
                                WATCHERS_WIDTH,
                                Align::Left,
                            ),
                        ];
                        ListItem::new(table_row(inner, flex, cells))
                    }
                })
                .collect(),
        };
        let selected = (self.focused && !items.is_empty()).then(|| self.cursor.index());
        self.rendered = render_list(
            frame,
            area,
            title,
            items,
            selected,
            self.focused,
            Some(self.cursor.index()),
        );
    }
}

passive_component!(SeriesPane);

impl AppComponent<Msg, NoUserEvent> for SeriesPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some(key) = plain(ev)
            && self.cursor.nav(key, self.len())
        {
            return Some(Msg::None);
        }
        if let Some(msg) = self.keymap().dispatch(self, ev) {
            return Some(msg);
        }
        // While filtering, everything else edits the filter text — the
        // shared vocabulary (word ops included). Only a text *change*
        // re-filters and resets the selection; bare cursor motion inside
        // the filter keeps it.
        if self.filtering {
            let before = self.filter.text();
            if self.filter.edit(ev) {
                return Some(if self.filter.text() == before {
                    Msg::None
                } else {
                    self.cursor.reset();
                    Msg::SeriesFilterChanged
                });
            }
        }
        None
    }
}

/// While editing the filter: letters type (no Char bindings here), the
/// bindings below act. Mode and sort keys are deliberately absent so any
/// letter can be typed.
static SERIES_FILTERING_KEYMAP: Keymap<SeriesPane, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Backspace),
        bar: None,
        action: SeriesPane::act_filter_backspace_exit,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: Some(("Esc", "Clear")),
        action: SeriesPane::act_filter_esc,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Browse")),
        action: SeriesPane::act_browse,
    },
]);

/// Recent mode. Filtering is gated behind `/` so the bare `m`/`s` keys
/// stay live — and reliable: Ctrl-modified letters collide with control
/// codes (Ctrl-M == Enter) in terminals lacking the enhanced keyboard
/// protocol.
static SERIES_RECENT_KEYMAP: Keymap<SeriesPane, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Char('m'),
        bar: Some(("m", "Mode")),
        action: SeriesPane::act_mode,
    },
    Binding {
        pattern: KeyPattern::Char('/'),
        bar: Some(("/", "Filter")),
        action: SeriesPane::act_filter_start,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: None,
        action: SeriesPane::act_filter_clear,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Browse")),
        action: SeriesPane::act_browse,
    },
]);

/// All mode: Recent plus the sort toggle.
static SERIES_ALL_KEYMAP: Keymap<SeriesPane, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Char('m'),
        bar: Some(("m", "Mode")),
        action: SeriesPane::act_mode,
    },
    Binding {
        pattern: KeyPattern::Char('s'),
        bar: Some(("s", "Sort")),
        action: SeriesPane::act_sort,
    },
    Binding {
        pattern: KeyPattern::Char('/'),
        bar: Some(("/", "Filter")),
        action: SeriesPane::act_filter_start,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: None,
        action: SeriesPane::act_filter_clear,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Browse")),
        action: SeriesPane::act_browse,
    },
]);

/// The List mode: no filter (`/` deliberately unbound so it stays inert).
static SERIES_LIST_KEYMAP: Keymap<SeriesPane, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Char('m'),
        bar: Some(("m", "Mode")),
        action: SeriesPane::act_mode,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: Some(("Enter", "Open")),
        action: SeriesPane::act_list_enter,
    },
    Binding {
        pattern: KeyPattern::Char('e'),
        bar: Some(("e", "Edit")),
        action: SeriesPane::act_list_edit,
    },
    Binding {
        pattern: KeyPattern::Char('l'),
        bar: Some(("l", "Link")),
        action: SeriesPane::act_list_link,
    },
]);

// ---- Player status -----------------------------------------------------

/// The 3-line status block at the bottom.
#[derive(Default)]
pub struct StatusBar {
    props: StatusProps,
    focused: bool,
}

/// `MM:SS` formatting shared by the status line and the progress line.
fn mmss(millis: u64) -> String {
    let s = millis / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

impl StatusBar {
    /// Replace props.
    pub fn set_props(&mut self, props: StatusProps) {
        self.props = props;
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Anything but a healthy link replaces the play-state text: the
        // gating info is stale while offline, and a silent dead
        // handshake reads as a hang (design.md UI principles; the
        // 2026-07-06 post-wake IPv6 black hole).
        let state = match self.props.link {
            super::props::LinkStatus::Connecting { attempt } if attempt <= 1 => {
                Span::styled("⚡ connecting to server…", theme::tone_style(Tone::Paused))
            }
            super::props::LinkStatus::Connecting { attempt } => Span::styled(
                format!("⚡ connecting to server (attempt {attempt})…"),
                theme::tone_style(Tone::Paused),
            ),
            super::props::LinkStatus::Down => Span::styled(
                "⚡ connection lost — retrying…",
                theme::tone_style(Tone::Paused),
            ),
            super::props::LinkStatus::Connected => {
                if self.props.playing {
                    Span::styled("▶ playing", theme::tone_style(Tone::Good))
                } else if self.props.blockers.is_empty() {
                    Span::styled("⏸ paused", theme::dim())
                } else {
                    Span::styled(
                        format!("⏸ waiting on {}", self.props.blockers.join(", ")),
                        theme::tone_style(Tone::Blocked),
                    )
                }
            }
        };
        let now = match &self.props.title {
            Some(title) => format!("Now Playing: {title}"),
            None => "Nothing playing".to_string(),
        };
        let lines = vec![Line::from(vec![state]), Line::from(now)];
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(theme::dim()),
            ),
            area,
        );
    }

    /// The progress-bar + elapsed/total time text for the bottom line
    /// (design.md #6): its own row, so the variable-width "waiting
    /// on ..." state text on the main status line never shoves it
    /// sideways. It shares `self.props` with [`Self::render`] but is
    /// drawn by [`HealthLine`] (the left end of the terminal-wide
    /// bottom line), so this returns text instead of rendering.
    pub(crate) fn progress_text(&self) -> String {
        match (self.props.position_millis, self.props.duration_millis) {
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
        }
    }
}

passive_component!(StatusBar);

impl AppComponent<Msg, NoUserEvent> for StatusBar {
    fn on(&mut self, _ev: &Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}

// ---- Health line -------------------------------------------------------

/// The terminal-wide, borderless bottom line of the main area
/// (design.md #6 + Connection Health Line): the progress bar + time at
/// the left, connection-health metrics right-aligned, and the middle
/// space carrying the advisor's suggestion (and, someday, marquee
/// commentary), centered with at least two spaces of margin each side.
/// Dim when healthy; only the offending field turns yellow/red.
/// Rendered directly by [`super::app::Ui::draw`] — passive, captures no
/// input, and is not part of the focus ring or mouse hit-testing.
#[derive(Default)]
pub struct HealthLine {
    props: HealthProps,
}

impl HealthLine {
    /// Replace props.
    pub fn set_props(&mut self, props: HealthProps) {
        self.props = props;
    }

    /// Render the row. `marquee` is the live scroll frame — the text
    /// and its current cell offset — when one is animating; the slot
    /// precedence is **warning/critical suggestion > marquee > info
    /// suggestion > blank** (a health warning is the row's job; the
    /// commentary yields to it). Returns the middle slot's width in
    /// cells, so the caller can measure when the marquee has fully
    /// exited.
    pub(crate) fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        progress: &str,
        marquee: Option<(&str, usize)>,
    ) -> usize {
        use unicode_width::UnicodeWidthStr;
        let width = area.width as usize;

        // Right end: the health fragments, dim separators between.
        let mut health = Vec::new();
        for (text, tone) in super::props::health_fragments(&self.props) {
            if !health.is_empty() {
                health.push(Span::styled(" · ", theme::dim()));
            }
            health.push(Span::styled(text, theme::tone_style(tone)));
        }
        let health_width: usize = health.iter().map(|span| span.content.width()).sum();

        // Left end: the progress bar, truncated only if the terminal is
        // too narrow to hold both (the health metrics win — they are
        // the row's reason to exist).
        let progress_max = width.saturating_sub(health_width + 2);
        let (progress, progress_width) = truncate_display(progress, progress_max);

        // Middle: the suggestion / commentary slot, centered in the
        // leftover space with ≥2 spaces of margin toward each
        // neighbour; dropped entirely (rather than a lone ellipsis)
        // when the space is too tight for useful text.
        let free = width.saturating_sub(progress_width + health_width);
        let mut spans = vec![Span::raw(progress)];
        let suggestion = self.props.suggestion.as_ref().filter(|_| free >= 4 + 4);
        // A warning (or worse) owns the slot; the marquee scrolls only
        // over an empty or merely-informational slot. The ≥2-space
        // margins live inside the marquee window (its `free - 4`).
        let warning = suggestion.filter(|s| s.tone != super::props::Tone::Muted);
        let marquee_frame = warning
            .is_none()
            .then(|| {
                marquee
                    .filter(|_| free >= 4 + 4)
                    .and_then(|(text, offset)| super::props::marquee_window(text, free - 4, offset))
            })
            .flatten();
        match (warning.or(suggestion), marquee_frame) {
            (_, Some((text, window_pad))) => {
                let text_width = text.width();
                spans.push(Span::raw(" ".repeat(2 + window_pad)));
                spans.push(Span::raw(text.clone()));
                spans.push(Span::raw(
                    " ".repeat(free.saturating_sub(2 + window_pad + text_width)),
                ));
            }
            (Some(suggestion), None) => {
                let (text, text_width) = truncate_display(&suggestion.text, free - 4);
                let pad = free - text_width;
                let left_pad = pad / 2;
                spans.push(Span::raw(" ".repeat(left_pad)));
                spans.push(Span::styled(text, theme::tone_style(suggestion.tone)));
                spans.push(Span::raw(" ".repeat(pad - left_pad)));
            }
            (None, None) => spans.push(Span::raw(" ".repeat(free))),
        }
        spans.extend(health);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        free.saturating_sub(4)
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
    use crate::ui::widgets::list::PAGE_STEP;
    use tuirealm::event::{KeyEvent, KeyModifiers};
    use tuirealm::ratatui::Terminal;
    use tuirealm::ratatui::backend::TestBackend;

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
        assert_eq!(p.mode(), SeriesMode::TheList);

        // Bare `m` cycles mode when not filtering.
        p.on(&key(Key::Char('m')));
        assert_eq!(p.mode(), SeriesMode::Recent);

        // `/` starts filtering; now letters — including `m` and `s` —
        // build the filter instead of cycling/sorting.
        p.on(&key(Key::Char('/')));
        for c in ['m', 'o', 'n'] {
            p.on(&key(Key::Char(c)));
        }
        assert_eq!(p.filter(), "mon");
        assert_eq!(
            p.mode(),
            SeriesMode::Recent,
            "mode must not change while filtering"
        );

        // Backspace edits; Esc clears and exits filtering.
        p.on(&key(Key::Backspace));
        assert_eq!(p.filter(), "mo");
        p.on(&key(Key::Esc));
        assert_eq!(p.filter(), "");

        // After Esc, `m` cycles again.
        p.on(&key(Key::Char('m')));
        assert_eq!(p.mode(), SeriesMode::All);
    }

    /// Backspace deletes filter characters; once the filter is empty, a
    /// further Backspace exits filtering entirely (an escape hatch alongside
    /// Esc). Regression for filtering being a one-way trip via `/`
    /// (2026-06-15).
    #[test]
    fn backspace_on_empty_filter_exits_filtering() {
        // Filtering only applies in Recent/All (The List, the default,
        // doesn't filter — see `the_list_mode_does_not_filter`), so step
        // off the default mode first.
        let mut p = SeriesPane::default();
        p.on(&key(Key::Char('m')));
        assert_eq!(p.mode(), SeriesMode::Recent);

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

    /// Long list panes keep their stored cursor centered even while another
    /// pane has focus, without drawing the selection highlight.
    #[test]
    fn centers_long_list_around_unfocused_series_cursor() {
        let mut pane = SeriesPane::default();
        pane.on(&key(Key::Char('m'))); // The List -> Recent
        pane.set_franchises(
            (0..20)
                .map(|i| FranchiseRow {
                    key: dessplay_core::franchise::FranchiseKey::Name(i.to_string()),
                    title: format!("series-{i:02}"),
                    year: None,
                })
                .collect(),
        );
        pane.cursor.set(10);

        let mut terminal = Terminal::new(TestBackend::new(40, 9)).unwrap();
        let buffer = terminal
            .draw(|frame| pane.render(frame, frame.area()))
            .unwrap()
            .buffer
            .clone();
        let y = (0..buffer.area.height).find(|&y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains("series-10")
        });

        assert_eq!(y, Some(4));
    }

    /// PageUp/PageDown jump the selection by a page in both Recent/All and
    /// while filtering. Regression for the series browser ignoring the page
    /// keys (2026-06-15). Selection is observed through the franchise Enter
    /// resolves to.
    #[test]
    fn page_keys_jump_series_selection() {
        let mut p = SeriesPane::default();
        // Franchise-list paging is a Recent/All concern; step off the
        // default The List mode first.
        p.on(&key(Key::Char('m')));
        assert_eq!(p.mode(), SeriesMode::Recent);
        p.set_franchises(franchises(30));

        // From the top, PageDown lands a page in.
        p.on(&key(Key::PageDown));
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseFranchise(
                dessplay_core::franchise::FranchiseKey::Name(PAGE_STEP.to_string())
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
                dessplay_core::franchise::FranchiseKey::Name(PAGE_STEP.to_string())
            ))
        );
    }

    /// The List mode has no filter: `/` is inert and the bare letters keep
    /// their List bindings.
    #[test]
    fn the_list_mode_does_not_filter() {
        let mut p = SeriesPane::default();
        assert_eq!(p.mode(), SeriesMode::TheList);
        p.on(&key(Key::Char('/')));
        assert_eq!(p.filter(), "");
        // `m` still cycles (to Recent), proving `/` didn't start a filter.
        p.on(&key(Key::Char('m')));
        assert_eq!(p.mode(), SeriesMode::Recent);
    }

    fn list_row(
        id: u128,
        series_id: Option<dessplay_core::types::AniDbSeriesId>,
    ) -> crate::ui::props::ListRow {
        crate::ui::props::ListRow {
            id: dessplay_core::types::ListEntryId(id),
            name: "Some Show".into(),
            nero_name: None,
            next_ep: None,
            available: false,
            watchers: String::new(),
            series_id,
            anidb_unavailable: false,
        }
    }

    /// Enter on a linked entry browses its franchise; on an unlinked one it
    /// tries the candidate-ranked disambiguation view instead (design.md,
    /// Advancing next_ep) -- never straight to the plain editor anymore.
    #[test]
    fn list_enter_branches_on_whether_the_entry_is_linked() {
        let mut p = SeriesPane::default();
        assert_eq!(p.mode(), SeriesMode::TheList);
        p.set_groups(vec![ListGroup {
            heading: "Watching",
            rows: vec![
                list_row(1, Some(dessplay_core::types::AniDbSeriesId(7))),
                list_row(2, None),
            ],
            collapsed: false,
        }]);
        p.on(&key(Key::Down)); // heading -> first entry (linked)

        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseFranchise(
                dessplay_core::franchise::FranchiseKey::Series(
                    dessplay_core::types::AniDbSeriesId(7)
                )
            ))
        );
        p.on(&key(Key::Down));
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseUnlinkedListEntry(
                dessplay_core::types::ListEntryId(2)
            ))
        );
    }
}

#[cfg(test)]
mod playlist_pane_tests {
    use super::*;
    use crate::ui::props::PlaylistRow;
    use dessplay_core::types::Ed2kHash;
    use tuirealm::event::{KeyEvent, KeyModifiers};
    use tuirealm::ratatui::Terminal;
    use tuirealm::ratatui::backend::TestBackend;

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

    /// A char event the way a capital letter arrives from the terminal: the
    /// char is already uppercase, no modifier bit set (`typed` also accepts the
    /// SHIFT-flagged form).
    fn typed_char(c: char) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code: Key::Char(c),
            modifiers: KeyModifiers::NONE,
        })
    }

    fn row(hash: Ed2kHash) -> PlaylistRow {
        PlaylistRow {
            hash,
            title: "ep.mkv".to_string(),
            tone: Tone::Normal,
            is_now: false,
            temporary: false,
            download: None,
            watch: dessplay_core::types::SeriesWatchState::Maybe,
        }
    }

    fn pane_with_one_row() -> (PlaylistPane, Ed2kHash) {
        let hash = Ed2kHash([7u8; 16]);
        let mut p = PlaylistPane {
            focused: true,
            ..Default::default()
        };
        p.set_props(PlaylistProps {
            rows: vec![row(hash)],
            ..Default::default()
        });
        (p, hash)
    }

    fn pane_with_rows(n: u8) -> (PlaylistPane, Vec<Ed2kHash>) {
        let hashes: Vec<Ed2kHash> = (0..n).map(|i| Ed2kHash([i; 16])).collect();
        let mut p = PlaylistPane {
            focused: true,
            ..Default::default()
        };
        p.set_props(PlaylistProps {
            rows: hashes.iter().copied().map(row).collect(),
            ..Default::default()
        });
        (p, hashes)
    }

    fn render(pane: &mut PlaylistPane, height: u16) -> tuirealm::ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(40, height)).unwrap();
        terminal
            .draw(|frame| pane.render(frame, frame.area()))
            .unwrap()
            .buffer
            .clone()
    }

    fn row_y(buffer: &tuirealm::ratatui::buffer::Buffer, title: &str) -> Option<u16> {
        (0..buffer.area.height).find(|&y| {
            let line: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            line.contains(title)
        })
    }

    fn long_pane(focused: bool, target: usize) -> PlaylistPane {
        let mut pane = PlaylistPane {
            focused,
            ..Default::default()
        };
        pane.set_props(PlaylistProps {
            rows: (0..20)
                .map(|i| PlaylistRow {
                    title: format!("episode-{i:02}"),
                    is_now: i == target,
                    ..row(Ed2kHash([i as u8; 16]))
                })
                .collect(),
            now_index: Some(target),
        });
        pane.cursor.set(target);
        pane
    }

    /// A long focused playlist keeps context on both sides of the cursor
    /// instead of putting the selected row at the bottom of the viewport.
    #[test]
    fn focused_cursor_is_centered_in_long_playlist() {
        let mut pane = long_pane(true, 10);
        let buffer = render(&mut pane, 9); // seven rows inside the border

        assert_eq!(row_y(&buffer, "episode-10"), Some(4));
    }

    /// When another pane has focus, the playlist follows now-playing with
    /// the same centered context rather than jumping back to its first rows.
    #[test]
    fn unfocused_now_playing_is_centered_in_long_playlist() {
        let mut pane = long_pane(false, 10);
        let buffer = render(&mut pane, 9); // seven rows inside the border

        assert_eq!(row_y(&buffer, "episode-10"), Some(4));
    }

    /// Mimic the app applying a `MoveDown`/`MoveUp`: reorder the props to put
    /// `hash` at `new_index` and push them back, exactly as `apply_snapshot`
    /// does after the forced UI refresh. The component keeps its `sel`.
    fn apply_reorder(p: &mut PlaylistPane, hash: Ed2kHash, new_index: usize) {
        // All rows are identical except their hash, so drop the moved hash and
        // re-insert a fresh row for it at the target index (avoids an unwrap).
        let mut rows: Vec<PlaylistRow> = p
            .props
            .rows
            .iter()
            .filter(|r| r.hash != hash)
            .cloned()
            .collect();
        rows.insert(new_index, row(hash));
        p.set_props(PlaylistProps {
            rows,
            ..Default::default()
        });
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

    /// PgUp/PgDn page the playlist selection — the page keys work in every
    /// list by construction (widgets::ListCursor), where they previously
    /// existed in some panes and not others.
    #[test]
    fn page_keys_jump_playlist_selection() {
        use crate::ui::widgets::list::PAGE_STEP;
        let (mut p, hashes) = pane_with_rows(30);
        let page = |code| {
            Event::Keyboard(KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
            })
        };
        p.on(&page(Key::PageDown));
        assert_eq!(p.selected_hash(), Some(hashes[PAGE_STEP]));
        p.on(&page(Key::PageUp));
        assert_eq!(p.selected_hash(), Some(hashes[0]));
    }

    /// The cursor follows the moved entry across the reorder: after `J` advances
    /// the cursor and the app pushes the reordered props, it lands on the same
    /// entry, so repeated `J` keeps carrying it down.
    #[test]
    fn cursor_follows_moved_entry() {
        let (mut p, h) = pane_with_rows(3); // [0,1,2], sel=0
        assert_eq!(p.on(&typed_char('J')), Some(Msg::MoveDown(h[0])));
        apply_reorder(&mut p, h[0], 1); // -> [1,0,2]
        assert_eq!(p.cursor.index(), 1);
        assert_eq!(p.selected_hash(), Some(h[0])); // still on the moved entry
        assert_eq!(p.on(&typed_char('J')), Some(Msg::MoveDown(h[0])));
        apply_reorder(&mut p, h[0], 2); // -> [1,2,0]
        assert_eq!(p.cursor.index(), 2);
        assert_eq!(p.selected_hash(), Some(h[0]));
    }
}

#[cfg(test)]
mod users_pane_tests {
    use super::*;
    use crate::ui::props::{KnownOfflineRow, UserRow};
    use tuirealm::ratatui::Terminal;
    use tuirealm::ratatui::backend::TestBackend;

    fn present(name: &str) -> UserRow {
        UserRow {
            name: name.to_string(),
            label: "ready".to_string(),
            tone: Tone::Normal,
        }
    }

    fn offline(name: &str) -> KnownOfflineRow {
        KnownOfflineRow {
            name: name.to_string(),
            last_seen_label: "3d ago".to_string(),
        }
    }

    /// Selecting a known-offline row must survive a snapshot refresh with
    /// unchanged props -- `apply_snapshot` calls `set_props` on every
    /// incoming snapshot (presence, chat, position churn), not just when
    /// the rows actually change. Regression: `set_props` used to clamp to
    /// `rows.len()` only, snapping the selection off any known-offline row
    /// onto the last present user the moment any snapshot landed.
    #[test]
    fn selecting_a_known_offline_row_survives_a_snapshot_refresh() {
        let mut p = UsersPane {
            focused: true,
            ..Default::default()
        };
        let props = UsersProps {
            rows: vec![present("Baughn")],
            known_offline: vec![offline("Kim"), offline("Nero")],
            seeders: vec![],
        };
        p.set_props(props.clone());
        // Move onto the first known-offline row (index 1: past the one
        // present row).
        p.cursor.set(1);
        assert_eq!(p.selected_username(), Some("Kim".to_string()));

        // Simulate an unrelated snapshot refresh (props unchanged).
        p.set_props(props);
        assert_eq!(p.selected_username(), Some("Kim".to_string()));
    }

    #[test]
    fn centers_long_list_around_unfocused_users_cursor() {
        let mut pane = UsersPane::default();
        pane.set_props(UsersProps {
            rows: (0..20).map(|i| present(&format!("user-{i:02}"))).collect(),
            known_offline: vec![],
            seeders: vec![],
        });
        pane.cursor.set(10);

        let mut terminal = Terminal::new(TestBackend::new(40, 9)).unwrap();
        let buffer = terminal
            .draw(|frame| pane.render(frame, frame.area()))
            .unwrap()
            .buffer
            .clone();
        let y = (0..buffer.area.height).find(|&y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains("user-10")
        });

        assert_eq!(y, Some(4));
    }
}

#[cfg(test)]
mod chat_wrap_tests {
    use super::wrap_body;
    use proptest::prelude::*;

    /// Every chunk must be a contiguous char-slice of the input starting
    /// at its reported offset — the invariant spoiler hit-mapping needs.
    fn assert_offsets(text: &str, chunks: &[(String, usize)]) {
        let chars: Vec<char> = text.chars().collect();
        for (chunk, start) in chunks {
            let len = chunk.chars().count();
            let expected: String = chars[*start..*start + len].iter().collect();
            assert_eq!(chunk, &expected, "chunk not at reported offset {start}");
        }
    }

    #[test]
    fn breaks_at_spaces() {
        // first_width and rest_width both 10.
        let lines = wrap_body("the quick brown fox", 10, 10);
        for (line, _) in &lines {
            assert!(line.chars().count() <= 10, "line too wide: {line:?}");
        }
        // Reassembling with single spaces recovers the words in order.
        let joined = lines
            .iter()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            joined.split_whitespace().collect::<Vec<_>>(),
            ["the", "quick", "brown", "fox"]
        );
        assert_offsets("the quick brown fox", &lines);
    }

    #[test]
    fn hard_breaks_overlong_word() {
        let lines = wrap_body("supercalifragilistic", 5, 5);
        assert!(lines.len() > 1, "long word should be split");
        for (line, _) in &lines {
            assert!(line.chars().count() <= 5, "chunk too wide: {line:?}");
        }
        let concat: String = lines.iter().map(|(line, _)| line.as_str()).collect();
        assert_eq!(concat, "supercalifragilistic");
        assert_offsets("supercalifragilistic", &lines);
    }

    #[test]
    fn respects_narrower_first_line() {
        // First line only fits "ab"; the rest get width 10.
        let lines = wrap_body("ab cdefghij", 2, 10);
        assert_eq!(lines[0], ("ab".to_string(), 0));
        assert_eq!(lines[1], ("cdefghij".to_string(), 3));
    }

    #[test]
    fn exact_width_stays_on_one_line() {
        let lines = wrap_body("abcde", 5, 5);
        assert_eq!(lines, vec![("abcde".to_string(), 0)]);
    }

    #[test]
    fn zero_width_does_not_loop() {
        // Degenerate width is clamped to 1 internally; must terminate.
        let lines = wrap_body("hi there", 0, 0);
        assert!(!lines.is_empty());
    }

    #[test]
    fn offsets_survive_multiple_spaces() {
        // "a" at 0; the double space collapses at the boundary; "b" at 3.
        let lines = wrap_body("a  b", 1, 1);
        assert_offsets("a  b", &lines);
        assert_eq!(lines.last(), Some(&("b".to_string(), 3)));
    }

    proptest! {
        #[test]
        fn chunks_are_slices_at_their_offsets(
            text in "[a-c ]{0,40}",
            first in 1usize..12,
            rest in 1usize..12,
        ) {
            let lines = wrap_body(&text, first, rest);
            assert_offsets(&text, &lines);
        }
    }
}

#[cfg(test)]
mod chat_popup_tests {
    use super::CHAT_SUGGESTION_MAX;

    /// The popup cap must fit the whole command table, or a bare `/`
    /// silently drops rows.
    #[test]
    fn suggestion_cap_fits_command_table() {
        assert!(super::super::commands::SLASH_COMMANDS.len() <= CHAT_SUGGESTION_MAX as usize);
    }
}

#[cfg(test)]
mod chat_spoiler_tests {
    use super::*;
    use tuirealm::ratatui::layout::Rect;

    fn chat_line(text: &str) -> ChatLine {
        ChatLine {
            time: "12:00".to_string(),
            sender: "kim".to_string(),
            text: text.to_string(),
            system: false,
            subtitle: false,
            separator: false,
            action: false,
            irc: false,
            millis: 1_000,
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// The key `compose_spoiler_body` derives for run `index` of a
    /// [`chat_line`] with the given text.
    fn key_for(text: &str, index: usize) -> SpoilerKey {
        SpoilerKey::new(1_000, "kim", index, text)
    }

    /// A synthetic run identity for the click-state-machine tests, where
    /// only key consistency matters (no message text is in play).
    fn key(index: usize) -> SpoilerKey {
        key_for("synthetic", index)
    }

    /// Regression: two messages colliding on (millis, sender) — the
    /// realistic source is the IRC bridge, which stamps two PRIVMSGs
    /// arriving in one TCP read with the same local millisecond — must
    /// not share reveal state. Revealing A's spoiler must leave B's
    /// scrambled (the exact disclosure the feature exists to prevent).
    #[test]
    fn same_millisecond_messages_keep_separate_reveal_state() {
        let a = crate::ui::props::irc_line(1_000, "kim".into(), "one ||alpha|| here".into(), false);
        let b =
            crate::ui::props::irc_line(1_000, "kim".into(), "two ||omega|| there".into(), false);
        let key_a = wrap_chat_line(&a, 80, &[], "me", &HashMap::new())[0].1[0]
            .key
            .clone();
        let mut spoilers = HashMap::new();
        spoilers.insert(key_a, SpoilerState::Revealed);
        let wrapped_b = wrap_chat_line(&b, 80, &[], "me", &spoilers);
        let (visual, hits) = &wrapped_b[0];
        let text = line_text(visual);
        assert!(!text.contains("omega"), "revealing A leaked B: {text:?}");
        assert_eq!(hits.len(), 1, "B keeps its clickable hit range");
    }

    #[test]
    fn hidden_spoiler_is_scrambled_with_hit_range() {
        let line = chat_line("the ||secret|| word");
        let wrapped = wrap_chat_line(&line, 80, &[], "me", &HashMap::new());
        assert_eq!(wrapped.len(), 1);
        let (visual, hits) = &wrapped[0];
        let text = line_text(visual);
        assert!(!text.contains("secret"), "spoiler leaked: {text:?}");
        assert!(!text.contains('|'), "bars leaked: {text:?}");
        assert!(text.contains("the ") && text.contains(" word"));
        // Prefix "12:00 " (6) + "kim: " (5) = 11; run at display chars 4..10.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].cols, 15..21);
        assert_eq!(hits[0].key, key_for("the ||secret|| word", 0));
    }

    #[test]
    fn revealed_spoiler_shows_original_without_hits() {
        let line = chat_line("the ||secret|| word");
        let mut spoilers = HashMap::new();
        spoilers.insert(key_for("the ||secret|| word", 0), SpoilerState::Revealed);
        let wrapped = wrap_chat_line(&line, 80, &[], "me", &spoilers);
        let (visual, hits) = &wrapped[0];
        let text = line_text(visual);
        assert!(text.contains("the secret word"), "got {text:?}");
        assert!(hits.is_empty());
    }

    #[test]
    fn spoiler_spanning_wrap_boundary_hits_both_lines() {
        // Pane width 16: first chunk gets 16 - 11 = 5 cells, so the
        // nine-char run hard-breaks across two visual lines.
        let line = chat_line("||secretive|| end");
        let wrapped = wrap_chat_line(&line, 16, &[], "me", &HashMap::new());
        assert!(wrapped.len() >= 2, "expected a wrapped run");
        let first_hits = &wrapped[0].1;
        let second_hits = &wrapped[1].1;
        assert_eq!(first_hits.len(), 1);
        assert_eq!(second_hits.len(), 1);
        assert_eq!(first_hits[0].key, key_for("||secretive|| end", 0));
        assert_eq!(second_hits[0].key, key_for("||secretive|| end", 0));
        // First line: cols 11..16 (5 chars after the prefix); second:
        // the remaining 4 chars after the continuation indent.
        assert_eq!(first_hits[0].cols, 11..16);
        assert_eq!(second_hits[0].cols, 2..6);
        // The scrambled halves stay in sync with the unsplit scramble:
        // same seed, same char offsets (zalgo split-stability is
        // property-tested in dessplay-core).
        assert!(!line_text(&wrapped[0].0).contains("secre"));
    }

    /// Regression: hit columns are screen geometry, so they must advance
    /// by display width (ratatui lays out by cell width), not char count
    /// — double-width CJK before the run shifts its real cells right,
    /// and the scramble shrinks wide alphanumerics to single-width
    /// ASCII, so the run's own span shrinks too.
    #[test]
    fn hit_columns_use_display_width() {
        let line = chat_line("彼は||死ぬ||よ");
        let wrapped = wrap_chat_line(&line, 80, &[], "me", &HashMap::new());
        let (_, hits) = &wrapped[0];
        // Prefix "12:00 " (6) + "kim: " (5) = 11 cells; 彼は = 4 cells;
        // the run scrambles to two single-width ASCII letters → 15..17.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].cols, 15..17);
    }

    /// The same, end to end: a click at the run's *rendered* columns
    /// reveals it; a click one cell left (the "は" cell) does not.
    #[test]
    fn click_lands_on_wide_prefixed_spoiler_at_rendered_columns() {
        use tuirealm::ratatui::Terminal;
        use tuirealm::ratatui::backend::TestBackend;
        let mut pane = ChatPane::default();
        pane.set_lines(vec![chat_line("彼は||死ぬ||よ")]);
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|frame| pane.render(frame, frame.area()))
            .unwrap();
        // Left border (1) + prefix (11 cells) + 彼は (4 cells): the two
        // scrambled letters occupy screen columns 16..18 on body row 1.
        pane.click(15, 1, 1_000);
        assert!(pane.spoilers.is_empty(), "click left of the run must miss");
        pane.click(16, 1, 1_000);
        assert!(pane.spoiler_animating(), "click on the run must hit");
    }

    #[test]
    fn action_line_spoiler_is_scrambled() {
        let mut line = chat_line("reads ||the twist|| aloud");
        line.action = true;
        let wrapped = wrap_chat_line(&line, 80, &[], "me", &HashMap::new());
        let (visual, hits) = &wrapped[0];
        let text = line_text(visual);
        assert!(!text.contains("the twist"), "got {text:?}");
        assert!(!text.contains('|'));
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn mentions_highlight_outside_but_not_inside_hidden_runs() {
        let names = vec!["Nero".to_string()];
        let line = chat_line("Nero said ||Nero dies||");
        let wrapped = wrap_chat_line(&line, 80, &names, "me", &HashMap::new());
        let (visual, _) = &wrapped[0];
        // The mention outside the run keeps its user style...
        let styled: Vec<_> = visual
            .spans
            .iter()
            .filter(|s| s.content.as_ref() == "Nero")
            .collect();
        assert_eq!(styled.len(), 1, "exactly the outside mention: {visual:?}");
        assert_eq!(
            styled[0].style,
            theme::user_style("Nero").add_modifier(tuirealm::ratatui::style::Modifier::BOLD)
        );
        // ...and the hidden run contains no "Nero" at all (scrambled).
        let hidden: String = visual
            .spans
            .iter()
            .filter(|s| s.style == theme::spoiler())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!hidden.contains("Nero"));
    }

    /// A pane whose last "render" recorded the given hit rows (each row
    /// one hit at columns 5..10), so click tests can drive the state
    /// machine without a real draw. (ChatPane's fields are private, so
    /// default-then-assign is the only construction seam.)
    #[allow(clippy::field_reassign_with_default)]
    fn pane_with_hit_rows(count: usize) -> ChatPane {
        let mut pane = ChatPane::default();
        pane.rendered = RenderedChatLog {
            area: Rect::new(0, 0, 40, 10),
            rows: (0..count)
                .map(|i| {
                    vec![SpoilerHit {
                        cols: 5..10,
                        key: key(i),
                    }]
                })
                .collect(),
        };
        pane
    }

    #[test]
    fn click_tease_then_reveal_within_window() {
        let mut pane = pane_with_hit_rows(1);
        // Miss: border row.
        pane.click(6, 0, 1_000);
        assert!(pane.spoilers.is_empty());
        // First click: animation starts at generation 0.
        pane.click(6, 1, 1_000);
        assert!(pane.spoiler_animating());
        // Frames derive from wall time.
        assert!(pane.advance_spoilers(1_250));
        assert!(matches!(
            pane.spoilers.get(&key(0)),
            Some(SpoilerState::Animating { generation: 2, .. })
        ));
        // Settles into Armed after the last frame.
        assert!(pane.advance_spoilers(2_000));
        assert!(matches!(
            pane.spoilers.get(&key(0)),
            Some(SpoilerState::Armed { generation, .. }) if *generation == SPOILER_FRAMES
        ));
        assert!(!pane.spoiler_animating());
        // Second click within the 5s window reveals.
        pane.click(6, 1, 4_000);
        assert!(matches!(
            pane.spoilers.get(&key(0)),
            Some(SpoilerState::Revealed)
        ));
        // Further clicks are no-ops.
        pane.click(6, 1, 4_100);
        assert!(matches!(
            pane.spoilers.get(&key(0)),
            Some(SpoilerState::Revealed)
        ));
    }

    #[test]
    fn lapsed_window_reteases_with_fresh_generations() {
        let mut pane = pane_with_hit_rows(1);
        pane.click(6, 1, 0);
        pane.advance_spoilers(600);
        // Window lapsed: this is a fresh first click, continuing the
        // generation counter (fresh letters, not a replay).
        pane.click(6, 1, 6_000);
        assert!(matches!(
            pane.spoilers.get(&key(0)),
            Some(SpoilerState::Animating { base_gen, .. }) if *base_gen == SPOILER_FRAMES
        ));
        pane.advance_spoilers(6_600);
        // Second click within 5s of the *new* first click reveals.
        pane.click(6, 1, 7_000);
        assert!(matches!(
            pane.spoilers.get(&key(0)),
            Some(SpoilerState::Revealed)
        ));
    }

    #[test]
    fn reveal_newest_visible_walks_bottom_up() {
        let mut pane = pane_with_hit_rows(2);
        // Bottom row (newest) first.
        assert!(pane.reveal_newest_visible());
        assert!(matches!(
            pane.spoilers.get(&key(1)),
            Some(SpoilerState::Revealed)
        ));
        assert!(!matches!(
            pane.spoilers.get(&key(0)),
            Some(SpoilerState::Revealed)
        ));
        // Repeat (without a re-render) picks the remaining hidden one.
        assert!(pane.reveal_newest_visible());
        assert!(matches!(
            pane.spoilers.get(&key(0)),
            Some(SpoilerState::Revealed)
        ));
        // Nothing hidden left.
        assert!(!pane.reveal_newest_visible());
    }
}

#[cfg(test)]
mod chat_input_tests {
    use super::*;
    use tuirealm::event::{KeyEvent, KeyModifiers};

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

    /// Regression: pasted control characters must be dropped, like every
    /// other text field (`LineBuffer::insert_paste`). The chat input was
    /// the one editor that inserted `\n` / `\x1b` verbatim — and a sent
    /// message syncs those bytes to every peer's chat log, OSD, IRC, and
    /// the archive.
    #[test]
    fn pasted_control_characters_are_dropped() {
        let mut pane = focused_pane();
        pane.insert_text("one\ntwo\x1b[31m");
        assert_eq!(pane.text(), "onetwo[31m");
    }

    /// Regression: a scrolled input line must reset its horizontal scroll
    /// offset when sent, so the next line is not rendered from a stale column.
    /// (The reset now lives in `LineBuffer::clear`; this pins the chat-level
    /// wiring.)
    #[test]
    fn enter_resets_display_offset() {
        let mut pane = focused_pane();
        type_str(&mut pane, "a fairly long line that would scroll");
        // Rendering in a narrow window scrolls the buffer.
        pane.input.buffer_mut().scroll(12);
        assert!(pane.input.buffer().offset() > 0);
        let msg = pane.on(&key(Key::Enter));
        assert!(matches!(msg, Some(Msg::SendChat(_))));
        assert_eq!(pane.input.buffer().offset(), 0);
        assert_eq!(pane.text(), "");
    }

    #[test]
    fn esc_resets_display_offset() {
        let mut pane = focused_pane();
        type_str(&mut pane, "a fairly long line that would scroll");
        pane.input.buffer_mut().scroll(12);
        assert!(pane.input.buffer().offset() > 0);
        pane.on(&key(Key::Esc));
        assert_eq!(pane.input.buffer().offset(), 0);
        assert_eq!(pane.text(), "");
    }

    /// Backspacing the whole line away leaves nothing scrolled: the next
    /// render reconciliation snaps the window back to the start.
    #[test]
    fn backspace_to_empty_resets_display_offset() {
        let mut pane = focused_pane();
        type_str(&mut pane, "hello");
        pane.input.buffer_mut().scroll(3);
        for _ in 0.."hello".len() {
            pane.on(&key(Key::Backspace));
        }
        assert_eq!(pane.text(), "");
        assert_eq!(pane.input.buffer_mut().scroll(3), 0);
    }

    #[test]
    fn ctrl_left_moves_by_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        // Cursor parks at end (15). Word-left lands on the start of "brown".
        pane.on(&ctrl(Key::Left));
        assert_eq!(pane.input.buffer().cursor(), 10);
        pane.on(&ctrl(Key::Left));
        assert_eq!(pane.input.buffer().cursor(), 4); // start of "quick"
        pane.on(&ctrl(Key::Left));
        assert_eq!(pane.input.buffer().cursor(), 0); // start of "the"
        pane.on(&ctrl(Key::Left));
        assert_eq!(pane.input.buffer().cursor(), 0); // clamped
    }

    #[test]
    fn ctrl_right_moves_by_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        // Move cursor to the start first.
        pane.on(&key(Key::Home));
        assert_eq!(pane.input.buffer().cursor(), 0);
        pane.on(&ctrl(Key::Right));
        assert_eq!(pane.input.buffer().cursor(), 3); // end of "the"
        pane.on(&ctrl(Key::Right));
        assert_eq!(pane.input.buffer().cursor(), 9); // end of "quick"
        pane.on(&ctrl(Key::Right));
        assert_eq!(pane.input.buffer().cursor(), 15); // end of "brown"
        pane.on(&ctrl(Key::Right));
        assert_eq!(pane.input.buffer().cursor(), 15); // clamped
    }

    /// Alt-Left/Right move by word too — macOS terminals (ghostty) send Alt
    /// for Option-arrow, and some never send a usable Ctrl-arrow at all.
    #[test]
    fn alt_left_right_move_by_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        pane.on(&alt(Key::Left));
        assert_eq!(pane.input.buffer().cursor(), 10); // start of "brown"
        pane.on(&alt(Key::Left));
        assert_eq!(pane.input.buffer().cursor(), 4); // start of "quick"
        pane.on(&alt(Key::Right));
        assert_eq!(pane.input.buffer().cursor(), 9); // end of "quick"
    }

    /// macOS terminals (ghostty) emit Option-Left/Right as the readline
    /// word-motion bytes Alt-b / Alt-f, not Alt-arrow — those must move by word.
    #[test]
    fn alt_b_f_move_by_word() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        pane.on(&alt(Key::Char('b')));
        assert_eq!(pane.input.buffer().cursor(), 10); // start of "brown"
        pane.on(&alt(Key::Char('b')));
        assert_eq!(pane.input.buffer().cursor(), 4); // start of "quick"
        pane.on(&alt(Key::Char('f')));
        assert_eq!(pane.input.buffer().cursor(), 9); // end of "quick"
    }

    /// Ctrl-B / Ctrl-F are char-wise in readline, not word motion — they must
    /// not be hijacked into word jumps (and aren't typed into the buffer).
    #[test]
    fn ctrl_b_f_are_not_word_motion() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        let before = pane.input.buffer().cursor();
        pane.on(&ctrl(Key::Char('b')));
        pane.on(&ctrl(Key::Char('f')));
        assert_eq!(pane.input.buffer().cursor(), before);
        assert_eq!(pane.text(), "the quick brown");
    }

    /// Ctrl-A / Ctrl-E jump to the start / end of the line (emacs / readline
    /// habit), like Home / End. Ctrl-only — the letters must not be swallowed
    /// as word motion or typed into the buffer.
    #[test]
    fn ctrl_a_e_jump_to_line_ends() {
        let mut pane = focused_pane();
        type_str(&mut pane, "the quick brown");
        // Cursor parks at the end (15).
        pane.on(&ctrl(Key::Char('a')));
        assert_eq!(pane.input.buffer().cursor(), 0);
        pane.on(&ctrl(Key::Char('e')));
        assert_eq!(pane.input.buffer().cursor(), 15);
        // Neither was typed into the buffer.
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
        assert_eq!(pane.input.buffer().cursor(), 10); // start of "brown"
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

    /// Mid-line editing goes through the shared vocabulary: Delete removes
    /// forward, and typed characters land at the cursor, not the end.
    #[test]
    fn mid_line_insert_and_delete() {
        let mut pane = focused_pane();
        type_str(&mut pane, "helo world");
        pane.on(&key(Key::Home));
        pane.on(&key(Key::Right));
        pane.on(&key(Key::Right));
        pane.on(&key(Key::Char('l')));
        assert_eq!(pane.text(), "hello world");
        pane.on(&key(Key::End));
        pane.on(&ctrl(Key::Left));
        pane.on(&key(Key::Delete));
        assert_eq!(pane.text(), "hello orld");
    }
}

#[cfg(test)]
mod chat_completion_tests {
    use super::*;
    use tuirealm::event::{KeyEvent, KeyModifiers};
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
        let spans = highlight_mentions("just some words", &names(), "Baughn", Style::default());
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
        let spans = highlight_mentions("Baughn: hi", &names(), "Nero", Style::default());
        assert!(span_is_styled_user(&spans[0], "Baughn"));
        // The colon is a separate, unstyled span.
        assert_eq!(spans[1].content, ":");
        assert!(spans[1].style.fg.is_none());
    }

    #[test]
    fn mid_sentence_mention_only_styles_the_name() {
        let spans = highlight_mentions("ask Nero please", &names(), "Baughn", Style::default());
        let styled: Vec<_> = spans.iter().filter(|s| s.style.fg.is_some()).collect();
        assert_eq!(styled.len(), 1);
        assert!(span_is_styled_user(styled[0], "Nero"));
    }

    #[test]
    fn own_mention_is_additionally_reversed() {
        let spans = highlight_mentions("hi Baughn", &names(), "Baughn", Style::default());
        assert!(
            spans.iter().any(|s| s.content == "Baughn"
                && s.style.add_modifier.contains(Modifier::REVERSED)),
            "self-mention should be reversed"
        );
    }

    #[test]
    fn prefix_of_a_name_is_not_a_mention() {
        // "Bau" is a completion prefix but not an exact name — never styled.
        let spans = highlight_mentions("Bau is short", &names(), "Nero", Style::default());
        assert!(spans.iter().all(|s| s.style.fg.is_none()));
    }
}
