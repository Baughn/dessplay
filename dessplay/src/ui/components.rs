//! The main panes: synchronous controllers and presentation models behind tui-realm's
//! `Component`/`AppComponent` traits, driven by typed props from
//! [`super::props`] and built on the shared interaction primitives in
//! [`super::widgets`]. (We use the component model but not tui-realm's
//! threaded event listener — see ui-architecture.md, Framework Choice.)

use std::collections::HashMap;

use dessplay_core::spoiler;
use dessplay_core::types::ListEntryId;
use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Rect, Size};
use tuirealm::ratatui::style::Style;
#[cfg(test)]
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::text::Span;
use tuirealm::state::State;

use super::msg::Msg;
use super::props::{
    ChatLine, FranchiseRow, HealthProps, ListGroup, ListSort, PlaylistProps, SeriesSort,
    StatusProps, Tone, UsersProps,
};
use super::theme;
use super::widgets::{Binding, KeyPattern, Keymap, LineBuffer, ListCursor, TextField};

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
#[cfg(test)]
const CHAT_WRAP_INDENT: usize = 2;

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
        Self {
            millis,
            sender: sender.to_string(),
            index,
            text_hash: text_hash(message_text),
        }
    }
}

/// The message-text hash both identity keys ([`SpoilerKey`],
/// [`LineKey`]) embed. Keying only — never a seed for anything
/// user-visible.
fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Identity of one chat line, position-free: the same
/// (millis, sender, text-hash) triple [`SpoilerKey`] is built on,
/// minus the run index — line-level, not run-level. Keys the *held*
/// drag-selection, so a log rebuild (every snapshot rebuilds
/// [`ChatPane::lines`] from scratch, and the merged log both shrinks —
/// server chat compaction — and shifts — saturated local rings, fresh
/// day separators) re-resolves the selection onto the same messages
/// instead of whatever now sits at the old indices.
#[derive(Clone, PartialEq, Debug)]
struct LineKey {
    millis: u64,
    sender: String,
    text_hash: u64,
}

impl LineKey {
    fn of(line: &ChatLine) -> Self {
        Self {
            millis: line.millis,
            sender: line.sender.clone(),
            text_hash: text_hash(&line.text),
        }
    }

    /// Whether `line` is the message this key identifies. Separators are
    /// never messages (their text is a render-time date label), so they
    /// never match.
    fn matches(&self, line: &ChatLine) -> bool {
        !line.separator
            && line.millis == self.millis
            && line.sender == self.sender
            && text_hash(&line.text) == self.text_hash
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

/// How long a finished drag-selection keeps its highlight before it
/// quietly disappears (the clipboard copy already happened on release).
const SELECTION_TTL_MS: u64 = 5_000;

/// One end of a selection drag, in **text** coordinates: which chat
/// line, and which chars of its display body the pointed cell covers.
/// Text space, not screen space, so scrolling mid-drag cannot smear the
/// selection. `floor..ceil` is the char under the pointer on a hit and
/// empty (`floor == ceil`) at a boundary — left of the body or past the
/// row's last char.
#[derive(Clone, Copy, PartialEq, Debug)]
struct SelPoint {
    /// Index into `ChatPane::lines`.
    line: usize,
    floor: usize,
    ceil: usize,
}

/// What a selection covers, in positional indices into the *current*
/// line list. The spec's invariant is structural: a selection is
/// *either* a char range within one line *or* whole lines — a partial
/// multi-line span is unrepresentable.
///
/// Positions are only ever computed and consumed against the same list:
/// an in-flight drag reads the render-recorded rows, and a *held*
/// selection is stored identity-keyed ([`HeldRange`]) and resolved to
/// this form at each use. A log rebuild during the hold window
/// therefore cannot smear the selection — it re-resolves onto the same
/// messages, or drops when they are gone.
#[derive(Clone, Copy, PartialEq, Debug)]
enum SelRange {
    /// Part of a single line: a char range of its display body.
    Partial {
        line: usize,
        start: usize,
        end: usize,
    },
    /// Whole lines, inclusive between the two ends. `anchor` is where
    /// the drag started; `focus` is the end Shift-Up/Down moves
    /// (gdocs-style, so extending can shrink back across the anchor).
    Lines { anchor: usize, focus: usize },
}

/// A held selection, keyed by message *identity* rather than position:
/// `set_lines` replaces the whole log at snapshot rate (~10 Hz during
/// playback) and the merged log shrinks and shifts (chat compaction,
/// saturated local rings, day separators), so a positional range held
/// for the 5 s window would silently retarget — or index out of
/// bounds. [`ChatPane::resolve_held`] maps this back to a positional
/// [`SelRange`] at each use (highlight, extend), yielding `None` when
/// an identified message has left the log.
#[derive(Clone, PartialEq, Debug)]
enum HeldRange {
    /// Part of a single line: a char range of its display body.
    Partial {
        line: LineKey,
        start: usize,
        end: usize,
    },
    /// Whole lines, inclusive between the two ends (see
    /// [`SelRange::Lines`]).
    Lines { anchor: LineKey, focus: LineKey },
}

/// Drag-selection state machine: press → drag → release copies & holds.
enum Selection {
    /// Button down on log text; `focus` stays `None` until the pointer
    /// moves, so a plain click never selects (or touches the clipboard).
    /// Positional (render-recorded rows *are* per-frame positions), and
    /// safe so: every consumer reads it through `.get()`-guarded paths
    /// within milliseconds of the recording frame.
    Dragging {
        anchor: SelPoint,
        focus: Option<SelPoint>,
    },
    /// Released: the copy has happened, the highlight lingers until
    /// `expires_at` (animator-clock millis — the shell's monotonic
    /// `Ui::clock` domain) or the next unrelated input. Identity-keyed —
    /// the 5 s hold spans many log rebuilds.
    Held { range: HeldRange, expires_at: u64 },
}

/// One visual row of the drawn chat log: its clickable spoiler ranges
/// plus the geometry the selection mapping needs — which chars of which
/// line's display body this row shows, starting at which screen column.
struct RowRecord {
    hits: Vec<SpoilerHit>,
    /// Index into `ChatPane::lines` of the message this row shows.
    line: usize,
    /// The display-body chunk drawn on this row.
    body: String,
    /// Char offset of `body` within the line's full display body.
    char_start: usize,
    /// Absolute screen column of `body`'s first cell.
    body_col: u16,
    /// Day separators are drawn but never selectable.
    selectable: bool,
}

/// The chat log viewport the last render actually drew: its rect plus a
/// [`RowRecord`] per visible body row. The chat-pane analogue of
/// [`super::layout::RenderedCollection`] — click and selection mapping use render-recorded
/// geometry, never a click-time re-derivation. Zero-default misses every
/// click until the first draw.
#[derive(Default)]
struct RenderedChatLog {
    area: Rect,
    rows: Vec<RowRecord>,
    /// Screen rects the last render drew inline images into. The
    /// renderer records these independently from text-theme painting
    /// (their fg/bg pairs *are* the picture).
    image_areas: Vec<Rect>,
    /// URL identities for successfully painted, visible image pixels.
    image_hits: Vec<(Rect, String)>,
}

impl RenderedChatLog {
    /// The spoiler under a screen position, if any (border cells miss).
    fn hit(&self, column: u16, row: u16) -> Option<&SpoilerKey> {
        if !self
            .area
            .contains(tuirealm::ratatui::layout::Position::new(column, row))
        {
            return None;
        }
        let body_row = row.checked_sub(self.area.y)? as usize;
        self.rows
            .get(body_row)?
            .hits
            .iter()
            .find(|hit| hit.cols.contains(&column))
            .map(|hit| &hit.key)
    }

    /// Map a screen position to a text point, exactly: misses (borders,
    /// the input line, separator rows) return `None`. Anchors a drag.
    fn point_at(&self, column: u16, row: u16) -> Option<SelPoint> {
        if !self
            .area
            .contains(tuirealm::ratatui::layout::Position::new(column, row))
        {
            return None;
        }
        let body_row = row.checked_sub(self.area.y)? as usize;
        let record = self.rows.get(body_row)?;
        record.selectable.then(|| Self::point_in(record, column))
    }

    /// Map a screen position to the *nearest* text point: rows clamp
    /// into the drawn log, a separator row resolves to the end of the
    /// nearest selectable row above it (or the start of the one below).
    /// Moves a drag's focus, so dragging past an edge or across a day
    /// divider keeps selecting instead of going dead.
    fn point_near(&self, column: u16, row: u16) -> Option<SelPoint> {
        if self.rows.is_empty() {
            return None;
        }
        let body_row = (row.saturating_sub(self.area.y) as usize).min(self.rows.len() - 1);
        let record = &self.rows[body_row];
        if record.selectable {
            return Some(Self::point_in(record, column));
        }
        if let Some(above) = self.rows[..body_row].iter().rev().find(|r| r.selectable) {
            let end = above.char_start + above.body.chars().count();
            return Some(SelPoint {
                line: above.line,
                floor: end,
                ceil: end,
            });
        }
        let below = self.rows[body_row + 1..].iter().find(|r| r.selectable)?;
        Some(SelPoint {
            line: below.line,
            floor: below.char_start,
            ceil: below.char_start,
        })
    }

    /// The text point a column hits within one row: the char under the
    /// cell, or an empty boundary left of the body / past its end.
    fn point_in(record: &RowRecord, column: u16) -> SelPoint {
        use unicode_width::UnicodeWidthChar;
        let mut col = record.body_col;
        let mut idx = record.char_start;
        if column >= col {
            for c in record.body.chars() {
                let cells = c.width().unwrap_or(0) as u16;
                if cells > 0 && column < col + cells {
                    return SelPoint {
                        line: record.line,
                        floor: idx,
                        ceil: idx + 1,
                    };
                }
                col += cells;
                idx += 1;
            }
        } else {
            idx = record.char_start;
        }
        SelPoint {
            line: record.line,
            floor: idx,
            ceil: idx,
        }
    }
}

/// Chat log + always-visible input line.
pub struct ChatPane {
    lines: Vec<ChatLine>,
    input: TextField,
    focused: bool,
    /// Visual lines scrolled up from the bottom (0 = pinned to newest).
    scroll_offset: usize,
    rendered_offset: usize,
    scroll_anchor: Option<(LineKey, usize, usize)>,
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
    /// Drag-selection state (design.md, Mouse support): in-flight drag
    /// or a held, already-copied highlight. Per-client, never synced.
    selection: Option<Selection>,
    /// Terminal image-protocol capability, detected once at startup.
    /// `None` (tests, or the query still pending) renders image URLs as
    /// the plain text they are.
    picker: Option<ratatui_image::picker::Picker>,
    /// Inline chat images, keyed by URL (design.md, Inline chat
    /// images). URL-keyed on purpose: the whole log is replaced from
    /// scratch ~10Hz, and a `StatefulProtocol`'s cached terminal encode
    /// must survive that. Pruned in [`ChatPane::sync_images`] to the
    /// URLs present in the current log window, so memory stays bounded
    /// by the log caps.
    images: HashMap<String, ImageSlot>,
    /// Set by `Ui::draw` while a modal is open: a graphics-protocol
    /// image ignores the cell z-order, so it could bleed through
    /// whatever is drawn on top. Suppressed images reserve no rows.
    suppress_images: bool,
}

/// Fetch/decode state for one inline chat image.
enum ImageSlot {
    /// Requested, no answer yet. Reserves no rows.
    Loading,
    /// Fetch or decode failed; the URL stays plain text. Kept so the
    /// URL is never re-requested while its message remains in the log.
    Failed,
    /// Decoded and ready to render.
    Ready {
        /// The decoded (pre-scaled) pixels — kept so the protocol can
        /// be re-encoded when the pane geometry changes.
        image: image::DynamicImage,
        /// The image encoded for a fitted cell size, sliceable by rows
        /// so scrolling crops the *fitted* image at the viewport edge
        /// (`Resize::Crop` would crop the unscaled source instead).
        /// `None` until the first draw; rebuilt when the fitted size
        /// changes (a pane resize).
        sliced: Option<(Size, ratatui_image::sliced::SlicedProtocol)>,
    },
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
            rendered_offset: 0,
            scroll_anchor: None,
            sent_history: Vec::new(),
            history_pos: None,
            usernames: Vec::new(),
            me: String::new(),
            completion: None,
            spoilers: HashMap::new(),
            rendered: RenderedChatLog::default(),
            selection: None,
            picker: None,
            images: HashMap::new(),
            suppress_images: false,
        }
    }
}

impl ChatPane {
    /// Replace the log.
    pub fn set_lines(&mut self, lines: Vec<ChatLine>) {
        self.lines = lines;
    }

    /// Set the detected image-protocol capability (production: once at
    /// terminal setup; tests inject `Picker::halfblocks()` for
    /// deterministic Unicode output).
    pub fn set_picker(&mut self, picker: ratatui_image::picker::Picker) {
        self.picker = Some(picker);
    }

    /// Reconcile the image store with the current log: prune entries
    /// whose message left the window, and (when inline images are
    /// enabled and a terminal protocol exists) mark unseen URLs as
    /// loading, returning them for the shell to fetch. A URL already
    /// tracked — loading, failed, or ready — is never re-requested.
    /// Disabling clears the store, freeing the decoded pixels; on
    /// re-enable everything re-fetches through the disk cache.
    pub fn sync_images(&mut self, enabled: bool) -> Vec<String> {
        if !enabled || self.picker.is_none() {
            self.images.clear();
            return Vec::new();
        }
        let wanted: std::collections::HashSet<&str> = self
            .lines
            .iter()
            .filter_map(|line| line.image_url.as_deref())
            .collect();
        self.images.retain(|url, _| wanted.contains(url.as_str()));
        let mut fetches = Vec::new();
        for url in wanted {
            if !self.images.contains_key(url) {
                self.images.insert(url.to_owned(), ImageSlot::Loading);
                fetches.push(url.to_owned());
            }
        }
        fetches
    }

    /// Open only pixels that survived the final overlay/clip pass. Keep an
    /// owned source so later chat compaction cannot remove the open image.
    pub(crate) fn image_at(&self, column: u16, row: u16) -> Option<super::modals::ImageModal> {
        let position = tuirealm::ratatui::layout::Position::new(column, row);
        let (_, url) = self
            .rendered
            .image_hits
            .iter()
            .find(|(area, _)| area.contains(position))?;
        let ImageSlot::Ready { image, .. } = self.images.get(url)? else {
            return None;
        };
        Some(super::modals::ImageModal::new(
            image.clone(),
            self.picker.clone()?,
        ))
    }

    /// Hide inline images this frame (a modal is open — graphics
    /// protocols ignore the cell z-order and would bleed through it).
    pub fn set_images_suppressed(&mut self, suppressed: bool) {
        self.suppress_images = suppressed;
    }

    /// The screen rects the last render drew images into; the theme's
    /// renderer exposes these alongside the text geometry in layout tools.
    pub(crate) fn image_areas(&self) -> &[Rect] {
        &self.rendered.image_areas
    }

    /// Deliver a fetch answer. Ignored when the URL's slot was pruned
    /// (its message scrolled out of the log window) or images were
    /// disabled meanwhile.
    pub fn set_image(&mut self, url: &str, result: Result<image::DynamicImage, String>) {
        if self.picker.is_none() {
            return;
        }
        let Some(slot) = self.images.get_mut(url) else {
            return;
        };
        *slot = match result {
            Ok(image) => ImageSlot::Ready {
                image,
                sliced: None,
            },
            Err(error) => {
                tracing::debug!(url, error, "chat image failed; leaving the link as text");
                ImageSlot::Failed
            }
        };
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
            .flat_map(|row| row.hits.iter().rev())
            .map(|hit| &hit.key)
            .find(|key| !matches!(self.spoilers.get(key), Some(SpoilerState::Revealed)))
            .cloned()
        else {
            return false;
        };
        self.spoilers.insert(key, SpoilerState::Revealed);
        true
    }

    // ---- Drag selection (design.md, Mouse support) -------------------

    /// Mouse button pressed in the chat column: any held highlight is
    /// already dismissed by the dispatcher; if the press landed on
    /// selectable log text, arm a potential drag. Selecting starts only
    /// on motion — a plain click still means "focus / spoiler", and
    /// never touches the clipboard.
    pub(crate) fn mouse_down(&mut self, column: u16, row: u16) {
        self.selection = self
            .rendered
            .point_at(column, row)
            .map(|anchor| Selection::Dragging {
                anchor,
                focus: None,
            });
    }

    /// Pointer moved with the button held: extend the drag. Coordinates
    /// clamp into the drawn log (the dispatcher grabs drags for us even
    /// when the pointer leaves the pane).
    pub(crate) fn mouse_drag(&mut self, column: u16, row: u16) {
        if let Some(Selection::Dragging { focus, .. }) = &mut self.selection
            && let Some(point) = self.rendered.point_near(column, row)
        {
            *focus = Some(point);
        }
    }

    /// Whether a drag is in progress (drag/release events route here by
    /// grab, not position).
    pub(crate) fn dragging(&self) -> bool {
        matches!(self.selection, Some(Selection::Dragging { .. }))
    }

    /// Button released: if the drag actually selected something, return
    /// the clipboard text and hold the highlight for
    /// [`SELECTION_TTL_MS`]. A motionless press releases into nothing.
    pub(crate) fn mouse_up(&mut self, now_millis: u64) -> Option<String> {
        if !self.dragging() {
            return None;
        }
        let range = self.selection_range();
        self.selection = None;
        let range = range?;
        let text = self.copy_text(range)?;
        // Re-key positional → identity for the hold: the log is rebuilt
        // out from under us at snapshot rate, and identity is what lets
        // the highlight follow its messages across rebuilds.
        let range = self.hold_range(range)?;
        self.selection = Some(Selection::Held {
            range,
            expires_at: now_millis + SELECTION_TTL_MS,
        });
        Some(text)
    }

    /// Whether a finished selection is being held (Shift-Up/Down extend
    /// it; any other key dismisses it).
    pub(crate) fn selection_held(&self) -> bool {
        matches!(self.selection, Some(Selection::Held { .. }))
    }

    /// Shift-Up/Down on a held selection: extend by one whole line. A
    /// partial selection first widens to its whole line (gdocs-style —
    /// the selection grows back to the start of the first selected
    /// line), then the focus end steps across lines, skipping day
    /// separators and shrinking when it re-crosses the anchor. Re-copies
    /// and re-arms the hold timer; returns the new clipboard text.
    pub(crate) fn extend_selection(&mut self, up: bool, now_millis: u64) -> Option<String> {
        let Some(Selection::Held { range, .. }) = &self.selection else {
            return None;
        };
        // Identity → current positions. The log may have been rebuilt
        // any number of times since the release; when a selected
        // message is gone (compaction, ring eviction), there is nothing
        // to extend — drop the stale highlight rather than guess.
        let Some(resolved) = self.resolve_held(range) else {
            self.selection = None;
            return None;
        };
        let (anchor, focus) = match resolved {
            SelRange::Partial { line, .. } => (line, line),
            SelRange::Lines { anchor, focus } => (anchor, focus),
        };
        let focus = self.step_line(focus, up).unwrap_or(focus);
        let range = SelRange::Lines { anchor, focus };
        let text = self.copy_text(range)?;
        let range = self.hold_range(range)?;
        self.selection = Some(Selection::Held {
            range,
            expires_at: now_millis + SELECTION_TTL_MS,
        });
        Some(text)
    }

    /// The nearest non-separator line index strictly beyond `from` in
    /// the given direction, if any. `from` is clamped into the list —
    /// callers pass freshly resolved indices, but no selection path may
    /// index `lines` on faith (belt and braces).
    fn step_line(&self, from: usize, up: bool) -> Option<usize> {
        if up {
            (0..from.min(self.lines.len()))
                .rev()
                .find(|&i| !self.lines[i].separator)
        } else {
            (from.saturating_add(1)..self.lines.len()).find(|&i| !self.lines[i].separator)
        }
    }

    /// The current index of the line `key` identifies, if it is still
    /// in the log. Linear over the merged log (bounded: synced
    /// `chat_keep` plus three 100-entry rings), and only ever run while
    /// a selection is held.
    fn line_index(&self, key: &LineKey) -> Option<usize> {
        self.lines.iter().position(|line| key.matches(line))
    }

    /// Positional → identity, keying `range` by the messages currently
    /// at its indices. `None` when an index is out of bounds (a drag
    /// recorded against a log that shrank before release — nothing
    /// sane to hold).
    fn hold_range(&self, range: SelRange) -> Option<HeldRange> {
        match range {
            SelRange::Partial { line, start, end } => Some(HeldRange::Partial {
                line: LineKey::of(self.lines.get(line)?),
                start,
                end,
            }),
            SelRange::Lines { anchor, focus } => Some(HeldRange::Lines {
                anchor: LineKey::of(self.lines.get(anchor)?),
                focus: LineKey::of(self.lines.get(focus)?),
            }),
        }
    }

    /// Identity → positional, against the *current* log. `None` when
    /// any identified message has left it — the caller treats that as
    /// "the selection is gone".
    fn resolve_held(&self, range: &HeldRange) -> Option<SelRange> {
        match range {
            HeldRange::Partial { line, start, end } => Some(SelRange::Partial {
                line: self.line_index(line)?,
                start: *start,
                end: *end,
            }),
            HeldRange::Lines { anchor, focus } => Some(SelRange::Lines {
                anchor: self.line_index(anchor)?,
                focus: self.line_index(focus)?,
            }),
        }
    }

    /// Drop any selection (a fresh click or an unrelated key).
    pub(crate) fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Expire a held highlight; returns whether the screen changed.
    pub(crate) fn expire_selection(&mut self, now_millis: u64) -> bool {
        if let Some(Selection::Held { expires_at, .. }) = &self.selection
            && now_millis >= *expires_at
        {
            self.selection = None;
            return true;
        }
        false
    }

    /// The range the current selection covers (in-flight drag or held),
    /// in current-log positions, if it covers anything at all. A held
    /// range whose messages have left the log resolves to `None` — the
    /// highlight simply stops drawing.
    fn selection_range(&self) -> Option<SelRange> {
        match self.selection.as_ref()? {
            Selection::Held { range, .. } => self.resolve_held(range),
            Selection::Dragging { anchor, focus } => {
                let focus = focus.as_ref()?;
                if anchor.line == focus.line {
                    let start = anchor.floor.min(focus.floor);
                    let end = anchor.ceil.max(focus.ceil);
                    (start < end).then_some(SelRange::Partial {
                        line: anchor.line,
                        start,
                        end,
                    })
                } else {
                    Some(SelRange::Lines {
                        anchor: anchor.line,
                        focus: focus.line,
                    })
                }
            }
        }
    }

    /// The clipboard text for a selection: a partial line verbatim (its
    /// display body, so a hidden spoiler copies as its scramble —
    /// WYSIWYG, no leak), whole lines in the irccloud log format
    /// (`HH:MM:SS <nick> text`). `None` when the range covers no
    /// copyable text.
    fn copy_text(&self, range: SelRange) -> Option<String> {
        match range {
            SelRange::Partial { line, start, end } => {
                let line = self.lines.get(line)?;
                let body = display_body(line, &self.spoilers);
                let text: String = body.chars().skip(start).take(end - start).collect();
                (!text.is_empty()).then_some(text)
            }
            SelRange::Lines { anchor, focus } => {
                let (lo, hi) = (anchor.min(focus), anchor.max(focus));
                let mut out = Vec::new();
                for line in self.lines.get(lo..=hi)? {
                    if line.separator {
                        continue;
                    }
                    let body = display_body(line, &self.spoilers);
                    out.push(if line.subtitle {
                        // The display body carries the "»"/"*" marker;
                        // subtitle timestamps are in-video positions, so
                        // keep them as displayed.
                        format!("{} {}", line.time, body)
                    } else if line.system {
                        format!("{} {}", super::props::hhmmss(line.millis), body)
                    } else if line.action {
                        format!(
                            "{} * {} {}",
                            super::props::hhmmss(line.millis),
                            line.sender,
                            body
                        )
                    } else {
                        format!(
                            "{} <{}> {}",
                            super::props::hhmmss(line.millis),
                            line.sender,
                            body
                        )
                    });
                }
                (!out.is_empty()).then(|| out.join("\n"))
            }
        }
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
    pub(crate) fn text(&self) -> String {
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

    /// Read-only live tail beneath the log viewer. Reuse chat wrapping and
    /// spoiler styling without changing the normal pane's scroll or draft.
    pub(crate) fn render_recent(
        &self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut super::layout::Renderer,
    ) {
        let Ok(scene) = renderer.arrange(
            "recent-chat",
            area,
            &super::layout::Presentation::default().text("title", "Recent chat"),
        ) else {
            return;
        };
        renderer.clear(frame, area);
        scene.paint_with_slots(frame, |name, frame, inner, style| {
            if name != "log" {
                return;
            }
            let mut messages = Vec::new();
            let mut used = 0usize;
            for line in self.lines.iter().rev() {
                let Ok((_, message)) = layout_chat_line(
                    line,
                    inner.width,
                    &self.usernames,
                    &self.me,
                    &self.spoilers,
                    scene.hanging_indent("log") as u16,
                    style,
                    "recent",
                    renderer,
                ) else {
                    continue;
                };
                used += usize::from(message.height());
                messages.push(message);
                if used >= usize::from(inner.height) {
                    break;
                }
            }
            let mut offset = -(used
                .saturating_sub(usize::from(inner.height))
                .min(i32::MAX as usize) as i32);
            for message in messages.iter().rev() {
                message.paint_scrolled(frame, inner, offset);
                offset += i32::from(message.height());
            }
        });
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        if let Ok(bundle) = super::layout::LayoutBundle::builtin() {
            self.render_layout(frame, area, &mut super::layout::Renderer::new(bundle));
        }
    }

    pub(crate) fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut super::layout::Renderer,
    ) {
        renderer.begin_chat();
        let suggestions = super::commands::matching(&self.text());
        let mut data = super::layout::Presentation::default()
            .text("title", "Chat")
            .boolean("suggesting", !suggestions.is_empty())
            .slot(
                "suggestions",
                0,
                suggestions.len().min(u16::MAX as usize) as u16,
            )
            .slot("input", 0, 1);
        if self.focused {
            data = data
                .state("chat-log-frame", "focus")
                .state("chat-input-frame", "focus");
        }
        let Ok(scene) = renderer.arrange("chat", area, &data) else {
            return;
        };
        renderer.record_controller("chat", &scene);
        let suggestion_rows = suggestions
            .iter()
            .map(|cmd| super::layout::PresentedRow {
                key: cmd.name.into(),
                data: super::layout::Presentation::default()
                    .text("signature", super::commands::signature(cmd))
                    .text("help", cmd.help),
                gap_after: false,
            })
            .collect::<Vec<_>>();
        self.rendered = RenderedChatLog::default();
        scene.paint_with_slots(frame, |name, frame, area, style| match name {
            "log" => self.render_log(
                frame,
                area,
                style,
                scene.hanging_indent("log") as u16,
                scene.has_overlay(),
                renderer,
            ),
            "input" => self.input.render_content_styled(
                frame,
                area,
                self.focused,
                false,
                style,
                renderer.color_depth(),
            ),
            "suggestions" => {
                let _ = renderer.paint_rows(
                    frame,
                    area,
                    "chat-suggestion",
                    &suggestion_rows,
                    None,
                    style,
                );
            }
            _ => {}
        });
        if !renderer.images_deferred() {
            self.paint_images(frame, renderer);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_log(
        &mut self,
        frame: &mut Frame,
        log_inner: Rect,
        style: Style,
        hanging_indent: u16,
        overlay: bool,
        renderer: &mut super::layout::Renderer,
    ) {
        let width = usize::from(log_inner.width);
        let visible = usize::from(log_inner.height);
        let max_image_rows = (visible / 3) as u16;
        if log_inner.is_empty() {
            return;
        }
        // Start at the live tail and instantiate only the required message window.
        // Source anchoring may extend that window to the retained scrollback item.
        let anchor = (self.scroll_offset > 0 && self.scroll_offset == self.rendered_offset)
            .then_some(self.scroll_anchor.as_ref())
            .flatten()
            .and_then(|(key, source, old_row)| {
                self.lines
                    .iter()
                    .position(|line| key.matches(line))
                    .map(|index| (index, *source, *old_row))
            });
        let mut first_images = HashMap::new();
        for (index, line) in self.lines.iter().enumerate() {
            if let Some(url) = &line.image_url {
                first_images.entry(url.as_str()).or_insert(index);
            }
        }
        let mut groups = Vec::new();
        let mut measured_rows = 0usize;
        let mut older_needed = None;
        for (idx, line) in self.lines.iter().enumerate().rev() {
            let Ok((message_rows, message)) = layout_chat_line(
                line,
                width as u16,
                &self.usernames,
                &self.me,
                &self.spoilers,
                hanging_indent,
                style,
                "chat",
                renderer,
            ) else {
                continue;
            };
            let attachment = if self.suppress_images || overlay || message.has_overlay() {
                None
            } else {
                line.image_url.as_ref().and_then(|url| {
                    if first_images.get(url.as_str()) != Some(&idx) {
                        return None;
                    }
                    let picker = self.picker.as_ref()?;
                    let ImageSlot::Ready { image, .. } = self.images.get(url)? else {
                        return None;
                    };
                    renderer
                        .attachment(
                            url,
                            &line.time,
                            Size::new(width as u16, max_image_rows),
                            image,
                            picker.font_size(),
                        )
                        .map(|attachment| (url.clone(), attachment))
                })
            };
            let height = message_rows.len()
                + attachment.as_ref().map_or(0, |(_, attachment)| {
                    usize::from(attachment.occupied_height().min(max_image_rows))
                });
            measured_rows += height;
            if let Some((target, source, old_row)) = anchor {
                if idx == target {
                    let at = message_rows
                        .iter()
                        .enumerate()
                        .filter(|(_, row)| row.selectable && row.char_start <= source)
                        .map(|(index, _)| index)
                        .next_back()
                        .unwrap_or(0);
                    older_needed = Some(old_row.saturating_sub(at));
                } else if let Some(remaining) = &mut older_needed {
                    *remaining = remaining.saturating_sub(height);
                }
            }
            groups.push((idx, message_rows, message, attachment));
            if older_needed == Some(0)
                || (anchor.is_none() && measured_rows >= self.scroll_offset.saturating_add(visible))
            {
                break;
            }
        }
        let mut rows: Vec<(usize, ChatRow)> = Vec::new();
        let mut image_bands = Vec::new();
        let mut message_bands = Vec::new();
        for (idx, message_rows, message, attachment) in groups.into_iter().rev() {
            message_bands.push((idx, rows.len(), message));
            rows.extend(message_rows.into_iter().map(|row| (idx, row)));
            if let Some((url, attachment)) = attachment {
                let height = attachment.occupied_height().min(max_image_rows);
                image_bands.push((url, rows.len(), attachment));
                for _ in 0..height {
                    rows.push((
                        idx,
                        ChatRow {
                            #[cfg(test)]
                            visual: Line::default(),
                            hits: Vec::new(),
                            body: String::new(),
                            char_start: 0,
                            body_col: 0,
                            selectable: false,
                        },
                    ));
                }
            }
        }
        // A stable source anchor retains context when wrapping, files, or history
        // changes. Explicit scroll input still chooses a new visual offset.
        if self.scroll_offset > 0
            && self.scroll_offset == self.rendered_offset
            && let Some((key, source, old_row)) = &self.scroll_anchor
            && let Some(line) = self.lines.iter().position(|line| key.matches(line))
            && let Some(at) = rows
                .iter()
                .enumerate()
                .filter(|(_, (index, row))| {
                    *index == line && row.selectable && row.char_start <= *source
                })
                .map(|(i, _)| i)
                .next_back()
        {
            let start = at.saturating_sub(*old_row);
            self.scroll_offset = rows.len().saturating_sub(start + visible);
        }
        // Clamp the scroll so it can never run past the top of the log.
        let max_offset = rows.len().saturating_sub(visible);
        self.scroll_offset = self.scroll_offset.min(max_offset);
        let end = rows.len().saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(visible);
        self.rendered_offset = self.scroll_offset;
        self.scroll_anchor = rows[start..end]
            .iter()
            .enumerate()
            .find(|(_, (_, row))| row.selectable)
            .map(|(position, (idx, row))| {
                (LineKey::of(&self.lines[*idx]), row.char_start, position)
            });
        // The bands intersecting the viewport, as (url, fitted size,
        // row offset relative to the viewport top — negative when the
        // band starts above it). The sliced widget crops the *fitted*
        // image by rows, so scrolling reveals it gradually.
        let mut image_draws = Vec::new();
        let mut image_areas = Vec::new();
        for (url, first_row, attachment) in image_bands {
            let band_top = first_row as i64 - start as i64;
            let band_bottom = band_top + i64::from(attachment.occupied_height());
            if band_bottom <= 0 || band_top >= visible as i64 {
                continue;
            }
            let pixels = attachment.scrolled_slot("image", log_inner, band_top as i32);
            if !pixels.is_empty() {
                image_areas.push(pixels);
            }
            image_draws.push((url, attachment, band_top as i32));
        }
        // Record the drawn viewport for spoiler-click and selection
        // mapping: shift the row-relative columns to absolute screen
        // columns in the arranged log viewport.
        self.rendered = RenderedChatLog {
            area: log_inner,
            rows: rows
                .into_iter()
                .skip(start)
                .take(end - start)
                .map(|(idx, mut row)| {
                    for hit in &mut row.hits {
                        hit.cols.start += log_inner.x;
                        hit.cols.end += log_inner.x;
                    }
                    RowRecord {
                        hits: row.hits,
                        line: idx,
                        body: row.body,
                        char_start: row.char_start,
                        body_col: row.body_col + log_inner.x,
                        selectable: row.selectable,
                    }
                })
                .collect(),
            image_areas,
            image_hits: Vec::new(),
        };
        let selection = self.selection_range();
        for (idx, first, message) in message_bands {
            let offset = first as i64 - start as i64;
            if offset + i64::from(message.height()) <= 0 || offset >= visible as i64 {
                continue;
            }
            message.paint_scrolled(frame, log_inner, offset as i32);
            match selection {
                Some(SelRange::Lines { anchor, focus })
                    if (anchor.min(focus)..=anchor.max(focus)).contains(&idx)
                        && !self.lines[idx].separator =>
                {
                    message.highlight_text(
                        frame,
                        log_inner,
                        offset as i32,
                        None,
                        None,
                        "",
                        theme::highlight_style(),
                    );
                }
                Some(SelRange::Partial { line, start, end }) if line == idx => {
                    message.highlight_text(
                        frame,
                        log_inner,
                        offset as i32,
                        Some("body"),
                        Some(start..end),
                        &display_body(&self.lines[idx], &self.spoilers),
                        theme::highlight_style(),
                    );
                }
                _ => {}
            }
        }
        for (url, attachment, offset) in image_draws {
            renderer.queue_image(url, attachment, log_inner, offset);
        }
    }

    pub(crate) fn paint_images(
        &mut self,
        frame: &mut Frame,
        renderer: &mut super::layout::Renderer,
    ) {
        self.rendered.image_areas.clear();
        self.rendered.image_hits.clear();
        let mut broken = Vec::new();
        for operation in renderer.take_image_operations() {
            let url = operation.url;
            let attachment = operation.attachment;
            let band_top = operation.offset;
            let log_inner = operation.viewport;
            let pixels = attachment.scrolled_slot("image", log_inner, band_top);
            if !pixels.is_empty() {
                self.rendered.image_areas.push(pixels);
            }
            attachment.paint_scrolled(frame, log_inner, band_top);
            let interior = attachment.slot("image");
            let size = Size::new(interior.width, interior.height);
            let y_offset = (band_top + i32::from(interior.y)) as i16;

            let Some(picker) = &self.picker else {
                break;
            };
            let Some(ImageSlot::Ready { image, sliced }) = self.images.get_mut(&url) else {
                continue;
            };
            if sliced.as_ref().map(|(encoded, _)| *encoded) != Some(size) {
                match ratatui_image::sliced::SlicedProtocol::new_with_resize(
                    picker,
                    image.clone(),
                    size,
                    ratatui_image::Resize::Fit(None),
                ) {
                    Ok(protocol) => *sliced = Some((size, protocol)),
                    Err(error) => {
                        tracing::debug!(url, %error, "encoding chat image failed");
                        broken.push(url);
                        continue;
                    }
                }
            }
            if let Some((_, protocol)) = sliced {
                if !pixels.is_empty() {
                    self.rendered.image_hits.push((pixels, url.clone()));
                }
                frame.render_widget(
                    ratatui_image::sliced::SlicedImage::new(
                        protocol,
                        ratatui_image::sliced::SignedPosition {
                            x: interior.x as i16,
                            y: y_offset,
                        },
                    ),
                    log_inner,
                );
            }
        }
        for url in broken {
            self.images.insert(url, ImageSlot::Failed);
        }
    }
}

#[cfg(test)]
pub(crate) use super::layout::wrap_body;

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
#[cfg(test)]
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

/// One wrapped visual row of a chat line: the drawn spans, its spoiler
/// hit ranges, and the selection geometry — which chars of the line's
/// display body this row shows, starting at which relative column.
struct ChatRow {
    #[cfg(test)]
    visual: Line<'static>,
    hits: Vec<SpoilerHit>,
    /// The display-body chunk drawn on this row (empty for separators).
    body: String,
    /// Char offset of `body` within the line's full display body.
    char_start: usize,
    /// Column (relative to the pane's first text cell) of `body`'s
    /// first cell.
    body_col: u16,
    /// Day separators are drawn but never selectable.
    selectable: bool,
}

/// The display body of a chat line — exactly the text [`wrap_chat_line`]
/// wraps (markers included, spoilers as currently shown), so selection
/// char offsets recorded at render time index straight into it.
fn display_body(line: &ChatLine, spoilers: &HashMap<SpoilerKey, SpoilerState>) -> String {
    if line.separator {
        String::new()
    } else if line.subtitle {
        format!("» {}", line.text)
    } else if line.system {
        format!("* {}", line.text)
    } else {
        compose_spoiler_body(line, spoilers).0
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_chat_line(
    line: &ChatLine,
    width: u16,
    usernames: &[String],
    me: &str,
    spoilers: &HashMap<SpoilerKey, SpoilerState>,
    indent: u16,
    inherited: Style,
    projection: &str,
    renderer: &mut super::layout::Renderer,
) -> Result<(Vec<ChatRow>, super::layout::RenderedScene), super::layout::Diagnostic> {
    use super::layout::{Presentation, RichSpan};
    use tuirealm::ratatui::style::Modifier;
    let key = format!("{projection}/{:?}", LineKey::of(line));
    let (display, runs) = if line.system || line.subtitle || line.separator {
        (display_body(line, spoilers), Vec::new())
    } else {
        compose_spoiler_body(line, spoilers)
    };
    let base = if line.system || line.subtitle || line.action {
        theme::dim()
    } else {
        Style::default()
    };
    let mut rich = Vec::new();
    let plain = |text: &str| -> Vec<RichSpan> {
        if line.system || line.subtitle {
            vec![RichSpan {
                text: text.into(),
                style: base,
                ..Default::default()
            }]
        } else {
            highlight_mentions(text, usernames, me, base)
                .into_iter()
                .map(|span| RichSpan {
                    text: span.content.into_owned(),
                    style: span.style,
                    ..Default::default()
                })
                .collect()
        }
    };
    let chars: Vec<_> = display.chars().collect();
    let mut position = 0;
    for run in &runs {
        if run.chars.start > position {
            rich.extend(plain(
                &chars[position..run.chars.start].iter().collect::<String>(),
            ));
        }
        let text: String = chars[run.chars.clone()].iter().collect();
        if let Some((seed, generation)) = run.hidden {
            let marks = text
                .chars()
                .enumerate()
                .filter_map(|(index, ch)| {
                    let marked = spoiler::zalgo(&ch.to_string(), seed, generation, index);
                    let marks: String = marked.chars().skip(1).collect();
                    (!marks.is_empty()).then_some((index, marks))
                })
                .collect();
            rich.push(RichSpan {
                text,
                style: theme::spoiler(),
                action: Some(run.key.index.to_string()),
                marks,
            });
        } else {
            rich.push(RichSpan {
                text,
                style: base,
                ..Default::default()
            });
        }
        position = run.chars.end;
    }
    if position < chars.len() {
        rich.extend(plain(&chars[position..].iter().collect::<String>()));
    }
    let data = Presentation::default()
        .inherit(inherited, indent)
        .text("timestamp", &line.time)
        .style("timestamp", theme::dim())
        .text("origin", "irc")
        .style("origin", theme::dim())
        .text("action-marker", "*")
        .style("action-marker", theme::dim())
        .text("sender", &line.sender)
        .style(
            "sender",
            theme::user_style(&line.sender).add_modifier(Modifier::BOLD),
        )
        .text("sender-delimiter", if line.action { "" } else { ":" })
        .style(
            "sender-delimiter",
            theme::user_style(&line.sender).add_modifier(Modifier::BOLD),
        )
        .boolean("external", line.irc && !line.system && !line.subtitle)
        .boolean("action", line.action && !line.system && !line.subtitle)
        .boolean("named", !line.system && !line.subtitle)
        .rich("body", rich)
        .text("label", &line.text);
    let scene = renderer.measure_content(
        if line.separator {
            "chat-separator"
        } else {
            "chat-message"
        },
        &key,
        width,
        &data,
    )?;
    let rows = (0..scene.height())
        .map(|y| {
            let regions: Vec<_> = scene
                .text_regions
                .iter()
                .filter(|region| region.binding == "body" && region.bounds.y == y)
                .collect();
            let start = regions
                .iter()
                .map(|region| region.source.start)
                .min()
                .unwrap_or(0);
            let end = regions
                .iter()
                .map(|region| region.source.end)
                .max()
                .unwrap_or(start);
            let body_col = regions
                .iter()
                .map(|region| region.bounds.x)
                .min()
                .unwrap_or(0);
            let hits = regions
                .iter()
                .filter_map(|region| {
                    let index = region.action.as_ref()?.parse::<usize>().ok()?;
                    let run = runs.iter().find(|run| run.key.index == index)?;
                    let bounds = region.bounds.intersection(region.clip);
                    (!bounds.is_empty()).then(|| SpoilerHit {
                        cols: bounds.x..bounds.right(),
                        key: run.key.clone(),
                    })
                })
                .collect();
            ChatRow {
                #[cfg(test)]
                visual: Line::default(),
                hits,
                body: chars[start..end].iter().collect(),
                char_start: start,
                body_col,
                selectable: !line.separator && (!regions.is_empty() || display.is_empty()),
            }
        })
        .collect();
    Ok((rows, scene))
}

/// Render one chat message as one or more wrapped visual rows, each
/// carrying its clickable spoiler ranges (relative columns; empty for
/// the line kinds that never carry spoilers) and selection geometry.
#[cfg(test)]
fn wrap_chat_line(
    line: &ChatLine,
    width: usize,
    usernames: &[String],
    me: &str,
    spoilers: &HashMap<SpoilerKey, SpoilerState>,
) -> Vec<ChatRow> {
    wrap_chat_line_with_indent(line, width, usernames, me, spoilers, CHAT_WRAP_INDENT)
}

#[cfg(test)]
fn wrap_chat_line_with_indent(
    line: &ChatLine,
    width: usize,
    usernames: &[String],
    me: &str,
    spoilers: &HashMap<SpoilerKey, SpoilerState>,
    continuation_indent: usize,
) -> Vec<ChatRow> {
    let continuation_indent = continuation_indent.min(width.saturating_sub(1));
    use tuirealm::ratatui::style::Modifier;
    use unicode_width::UnicodeWidthStr;
    let indent: String = " ".repeat(continuation_indent);
    if line.separator {
        // Render-time day divider: the date label centered between
        // dashes. Drawn but never selectable — it is not a message.
        let label = format!(" {} ", line.text);
        let label_w = label.width();
        let total = width.max(label_w);
        let dashes = total - label_w;
        let left = dashes / 2;
        let bar = format!("{}{}{}", "─".repeat(left), label, "─".repeat(dashes - left));
        return vec![ChatRow {
            visual: Line::from(Span::styled(bar, theme::dim())),
            hits: Vec::new(),
            body: String::new(),
            char_start: 0,
            body_col: 0,
            selectable: false,
        }];
    }
    if line.subtitle || line.system {
        // Local subtitle (Intermixed mode) / system notice: dim, no
        // sender, "»" / "*" marker (part of the display body).
        let time = format!("{} ", line.time);
        let prefix_width = time.width();
        let body = display_body(line, spoilers);
        let chunks = wrap_body(
            &body,
            width.saturating_sub(prefix_width),
            width.saturating_sub(continuation_indent),
        );
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, (chunk, chunk_start))| {
                let (visual, body_col) = if i == 0 {
                    (
                        Line::from(vec![
                            Span::styled(time.clone(), theme::dim()),
                            Span::styled(chunk.clone(), theme::dim()),
                        ]),
                        prefix_width as u16,
                    )
                } else {
                    (
                        Line::from(Span::styled(format!("{indent}{chunk}"), theme::dim())),
                        continuation_indent as u16,
                    )
                };
                ChatRow {
                    visual,
                    hits: Vec::new(),
                    body: chunk,
                    char_start: chunk_start,
                    body_col,
                    selectable: true,
                }
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
            width.saturating_sub(continuation_indent),
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
                    continuation_indent
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
                ChatRow {
                    visual,
                    hits,
                    body: chunk,
                    char_start: chunk_start,
                    body_col: offset,
                    selectable: true,
                }
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
            width.saturating_sub(continuation_indent),
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
                    continuation_indent
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
                ChatRow {
                    visual,
                    hits,
                    body: chunk,
                    char_start: chunk_start,
                    body_col: offset,
                    selectable: true,
                }
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
            area: Rect::new(1, 1, 38, 8),
            rows: vec![RowRecord {
                hits: vec![SpoilerHit {
                    cols: 5..10,
                    key: SpoilerKey::new(1_000, "kim", 0, "||x||"),
                }],
                line: 0,
                body: String::new(),
                char_start: 0,
                body_col: 1,
                selectable: true,
            }],
            image_areas: Vec::new(),
            image_hits: Vec::new(),
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
    rendered: super::layout::RenderedCollection,
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
        if let Ok(bundle) = super::layout::LayoutBundle::builtin() {
            self.render_layout(frame, area, &mut super::layout::Renderer::new(bundle));
        }
    }
    pub(crate) fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut super::layout::Renderer,
    ) {
        use super::layout::{Presentation, PresentedRow};
        let mut data = Presentation::default().text("title", "Users");
        if self.focused {
            data = data.state("users-frame", "focus");
        }
        let mut rows = self
            .props
            .rows
            .iter()
            .map(|row| PresentedRow {
                key: format!("user:{}", row.name),
                data: Presentation::default()
                    .text("name", &row.name)
                    .text("status", format!("[{}]", row.label))
                    .boolean("online", true)
                    .style("name", theme::user_style(&row.name))
                    .style("status", theme::tone_style(row.tone)),
                gap_after: false,
            })
            .collect::<Vec<_>>();
        let offline =
            theme::tone_style(Tone::Muted).add_modifier(tuirealm::ratatui::style::Modifier::ITALIC);
        rows.extend(self.props.known_offline.iter().map(|row| {
            PresentedRow {
                key: format!("user:{}", row.name),
                data: Presentation::default()
                    .text("name", &row.name)
                    .text("last-seen", format!("(last seen {})", row.last_seen_label))
                    .boolean("offline", true)
                    .style("name", offline)
                    .style("last-seen", offline),
                gap_after: false,
            }
        }));
        if !self.props.seeders.is_empty() {
            rows.push(PresentedRow {
                key: "seeders".into(),
                data: Presentation::default()
                    .text("name", "seeders:")
                    .text("holders", self.props.seeders.join(", "))
                    .boolean("seeders", true)
                    .style("name", theme::dim())
                    .style("holders", theme::dim()),
                gap_after: false,
            });
        }
        let selected = (self.focused && self.selectable_len() > 0).then(|| self.cursor.index());
        let data = data.collection("rows", &rows, selected, Some(self.cursor.index()));
        let Ok(scene) = renderer.arrange("users", area, &data) else {
            return;
        };
        renderer.record_controller("users", &scene);
        self.rendered = scene.collection("rows", &rows);
        scene.paint_with_slots(frame, |name, frame, area, style| {
            if name == "body" {
                self.rendered = renderer
                    .paint_collection(
                        frame,
                        area,
                        "user-row",
                        &rows,
                        selected,
                        Some(self.cursor.index()),
                        style,
                    )
                    .unwrap_or_default();
            }
        });
        self.cursor
            .reconcile_visible(self.selectable_len(), self.rendered.hidden());
    }
}

passive_component!(UsersPane);

impl AppComponent<Msg, NoUserEvent> for UsersPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some(key) = plain(ev)
            && self
                .cursor
                .nav_visible(key, self.selectable_len(), self.rendered.hidden())
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
    rendered: super::layout::RenderedCollection,
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

    /// `w`: cycle the entry's series watch state: Maybe -> Watching ->
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
        if let Ok(bundle) = super::layout::LayoutBundle::builtin() {
            self.render_layout(frame, area, &mut super::layout::Renderer::new(bundle));
        }
    }
    pub(crate) fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut super::layout::Renderer,
    ) {
        use super::layout::{Presentation, PresentedRow};
        let mut data = Presentation::default().text("title", "Playlist");
        if self.focused {
            data = data.state("playlist-frame", "focus");
        }
        let watch_tag = |row: &crate::ui::props::PlaylistRow| match row.watch {
            dessplay_core::types::SeriesWatchState::Watching => "watching",
            dessplay_core::types::SeriesWatchState::Maybe => "maybe",
            dessplay_core::types::SeriesWatchState::NotWatching => "not watching",
        };
        let show_temp = self.props.rows.iter().any(|row| row.temporary);
        let show_download = self.props.rows.iter().any(|row| row.download.is_some());
        let watch_width = self
            .props
            .rows
            .iter()
            .map(|row| watch_tag(row).len())
            .max()
            .unwrap_or(0) as u16;
        let mut rows = self
            .props
            .rows
            .iter()
            .map(|row| PresentedRow {
                key: row.hash.to_string(),
                data: Presentation::default()
                    .text("marker", if row.is_now { "▶" } else { "" })
                    .text("title", &row.title)
                    .text(
                        "download",
                        row.download
                            .map(|bps| format!("{}%", bps / 100))
                            .unwrap_or_default(),
                    )
                    .text("temporary", if row.temporary { "temp" } else { "" })
                    .text("watch", watch_tag(row))
                    .intrinsic_width("watch", watch_width)
                    .boolean("entry", true)
                    .boolean("show-download", show_download)
                    .boolean("show-temporary", show_temp)
                    .style("marker", theme::tone_style(row.tone))
                    .style("title", theme::tone_style(row.tone))
                    .style("download", theme::tone_style(Tone::Transfer))
                    .style("temporary", theme::dim())
                    .style("watch", theme::dim()),
                gap_after: false,
            })
            .collect::<Vec<_>>();
        rows.push(PresentedRow {
            key: "add".into(),
            data: Presentation::default()
                .text("title", "[Add New]")
                .style("title", theme::dim()),
            gap_after: false,
        });
        let selected = self.focused.then(|| self.cursor.index());
        let center = if self.focused {
            Some(self.cursor.index())
        } else {
            self.props.now_index
        };
        let data = data.collection("rows", &rows, selected, center);
        let Ok(scene) = renderer.arrange("playlist", area, &data) else {
            return;
        };
        renderer.record_controller("playlist", &scene);
        self.rendered = scene.collection("rows", &rows);
        scene.paint_with_slots(frame, |name, frame, area, style| {
            if name == "body" {
                self.rendered = renderer
                    .paint_collection(frame, area, "playlist-row", &rows, selected, center, style)
                    .unwrap_or_default();
            }
        });
        self.cursor
            .reconcile_visible(self.props.rows.len() + 1, self.rendered.hidden());
    }
}

passive_component!(PlaylistPane);

impl AppComponent<Msg, NoUserEvent> for PlaylistPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        // Rows plus the trailing [Add New].
        if let Some(key) = plain(ev)
            && self
                .cursor
                .nav_visible(key, self.props.rows.len() + 1, self.rendered.hidden())
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

/// What the List cursor was aimed at, by identity — a group heading, or
/// an entry id within a heading (an entry can appear in several per-user
/// groups, so the heading disambiguates which occurrence). Captured
/// before a `set_groups` rebuild and resolved against the new groups, so
/// the ~10 Hz snapshot churn (peers connecting/committing insert whole
/// groups; availability flips resort Recency) can't silently retarget
/// `n`/`e`/`l`/Enter — which write destructive, synced state — onto a
/// neighbouring row.
enum ListAnchor {
    Heading(String),
    Entry { heading: String, id: ListEntryId },
}

/// Series pane: Recent / All / The List.
#[derive(Default)]
pub struct SeriesPane {
    mode: SeriesMode,
    sort: SeriesSort,
    /// Sort order for The List mode (independent of `sort`, which is the
    /// All-Series toggle; the two modes answer different questions).
    list_sort: ListSort,
    franchises: Vec<FranchiseRow>,
    groups: Vec<ListGroup>,
    /// Expanded-state override per group heading.
    expanded: std::collections::BTreeMap<String, bool>,
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
    rendered: super::layout::RenderedCollection,
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

    /// Current List-mode sort.
    pub fn list_sort(&self) -> ListSort {
        self.list_sort
    }

    /// Seed the List sort order (from the persisted setting at startup).
    pub fn set_list_sort(&mut self, sort: ListSort) {
        self.list_sort = sort;
    }

    /// Current type-to-filter text (Recent / All modes).
    pub fn filter(&self) -> String {
        self.filter.text()
    }

    /// Replace franchise rows (Recent / All modes).
    ///
    /// No identity re-anchoring here, unlike [`Self::set_groups`]: the
    /// franchise order only moves on rare events (a watch completing, a
    /// filter edit — which resets the cursor anyway — or metadata
    /// arriving), and the sole row action is the non-destructive
    /// `BrowseFranchise`, so a shifted row can't mis-aim a synced write.
    pub fn set_franchises(&mut self, rows: Vec<FranchiseRow>) {
        self.franchises = rows;
        self.clamp();
    }

    /// Replace List groups, keeping the cursor aimed at the same thing.
    ///
    /// The groups are rebuilt per snapshot and their order is volatile by
    /// design (per-user groups come and go with peers; Recency resorts on
    /// availability/watched flips), while `n`/`e`/`l`/Enter resolve
    /// positionally through [`Self::nav_rows`] — so the cursor re-anchors
    /// by identity ([`ListAnchor`]) rather than riding its bare row
    /// index, and falls back to the plain clamp only when the anchored
    /// row disappeared. Identical groups (the memoized derivation's
    /// common case) return without touching anything.
    pub fn set_groups(&mut self, groups: &[ListGroup]) {
        if self.groups.as_slice() == groups {
            return;
        }
        let anchor = self
            .nav_rows()
            .nth(self.cursor.index())
            .map(|row| match row {
                ListNavRow::Heading(g) => ListAnchor::Heading(self.groups[g].heading.clone()),
                ListNavRow::Entry(g, e) => ListAnchor::Entry {
                    heading: self.groups[g].heading.clone(),
                    id: self.groups[g].rows[e].id,
                },
            });
        self.groups = groups.to_vec();
        match anchor.and_then(|anchor| self.anchor_position(&anchor)) {
            Some(position) => self.cursor.set(position),
            None => self.clamp(),
        }
    }

    /// Where `anchor` sits in the current nav space: an exact
    /// (heading, id) match first — an entry can appear in several
    /// per-user groups — then the id under any heading, then the bare
    /// heading. `None` when it vanished entirely.
    fn anchor_position(&self, anchor: &ListAnchor) -> Option<usize> {
        let find = |pred: &dyn Fn(ListNavRow) -> bool| self.nav_rows().position(pred);
        match anchor {
            ListAnchor::Heading(heading) => find(&|row| {
                matches!(row, ListNavRow::Heading(g) if self.groups[g].heading == *heading)
            }),
            ListAnchor::Entry { heading, id } => find(&|row| {
                matches!(row, ListNavRow::Entry(g, e)
                    if self.groups[g].rows[e].id == *id
                        && self.groups[g].heading == *heading)
            })
            .or_else(|| {
                find(&|row| matches!(row, ListNavRow::Entry(g, e) if self.groups[g].rows[e].id == *id))
            })
            .or_else(|| {
                find(&|row| {
                    matches!(row, ListNavRow::Heading(g) if self.groups[g].heading == *heading)
                })
            }),
        }
    }

    /// Test-only: the cursor's position in the current nav space.
    #[cfg(test)]
    fn cursor_index(&self) -> usize {
        self.cursor.index()
    }

    fn expanded(&self, group: &ListGroup) -> bool {
        *self
            .expanded
            .get(&group.heading)
            .unwrap_or(&!group.collapsed)
    }

    /// Rows in List mode, flattened for navigation.
    fn nav_rows(&self) -> impl Iterator<Item = ListNavRow> + '_ {
        self.groups.iter().enumerate().flat_map(|(g, group)| {
            let entries = if self.expanded(group) {
                group.rows.len()
            } else {
                0
            };
            std::iter::once(ListNavRow::Heading(g))
                .chain((0..entries).map(move |e| ListNavRow::Entry(g, e)))
        })
    }

    fn len(&self) -> usize {
        match self.mode {
            SeriesMode::Recent | SeriesMode::All => self.franchises.len(),
            SeriesMode::TheList => self.nav_rows().count(),
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

    /// `s` (The List): toggle recency/alphabetical sort.
    fn act_list_sort(&mut self) -> Option<Msg> {
        self.list_sort = self.list_sort.toggled();
        Some(Msg::ToggleListSort)
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
        let row = self.nav_rows().nth(self.cursor.index())?;
        match row {
            ListNavRow::Heading(g) => {
                let group = &self.groups[g];
                let now = self.expanded(group);
                let heading = group.heading.clone();
                self.expanded.insert(heading, !now);
                Some(Msg::None)
            }
            ListNavRow::Entry(g, e) => {
                // Linked or not, the dispatcher resolves what opening the
                // entry means (episode browser / candidate view / editor)
                // — it has the view; this pane only has the row.
                Some(Msg::BrowseListEntry(self.groups[g].rows[e].id))
            }
        }
    }

    /// `e` (The List): edit the selected entry.
    fn act_list_edit(&mut self) -> Option<Msg> {
        match self.nav_rows().nth(self.cursor.index())? {
            ListNavRow::Entry(g, e) => Some(Msg::EditListEntry(self.groups[g].rows[e].id)),
            ListNavRow::Heading(_) => None,
        }
    }

    /// `n` (The List): the minimal `nero_name` editor for the selected
    /// entry — the fast path for the group's renaming culture.
    fn act_list_nero(&mut self) -> Option<Msg> {
        match self.nav_rows().nth(self.cursor.index())? {
            ListNavRow::Entry(g, e) => Some(Msg::EditNeroName(self.groups[g].rows[e].id)),
            ListNavRow::Heading(_) => None,
        }
    }

    /// `l` (The List): link the selected entry to AniDB.
    fn act_list_link(&mut self) -> Option<Msg> {
        match self.nav_rows().nth(self.cursor.index())? {
            ListNavRow::Entry(g, e) => Some(Msg::LinkListEntry(self.groups[g].rows[e].id)),
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
        if let Ok(bundle) = super::layout::LayoutBundle::builtin() {
            self.render_layout(frame, area, &mut super::layout::Renderer::new(bundle));
        }
    }
    pub(crate) fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut super::layout::Renderer,
    ) {
        use super::layout::{Presentation, PresentedRow, RichSpan};
        let base = match self.mode {
            SeriesMode::Recent => "Recent Series",
            SeriesMode::All => "All Series",
            SeriesMode::TheList => "The List",
        };
        let filter = if self.filtering {
            self.filter.cursor_spans()
        } else {
            vec![Span::raw(self.filter.text())]
        };
        let mut data = Presentation::default()
            .text("title", base)
            .text("filter-label", "  /")
            .boolean(
                "filter-visible",
                self.mode != SeriesMode::TheList && (self.filtering || !self.filter.is_empty()),
            )
            .rich(
                "filter",
                filter
                    .into_iter()
                    .map(|span| RichSpan {
                        text: span.content.into_owned(),
                        style: span.style,
                        ..Default::default()
                    })
                    .collect(),
            );
        if self.focused {
            data = data.state("series-frame", "focus");
        }
        let rows: Vec<PresentedRow> = match self.mode {
            SeriesMode::Recent | SeriesMode::All => self
                .franchises
                .iter()
                .map(|row| PresentedRow {
                    key: format!("franchise/{:?}", row.key),
                    data: Presentation::default()
                        .boolean("franchise", true)
                        .text("title", &row.title)
                        .boolean("has-year", row.year.is_some())
                        .text(
                            "year",
                            row.year.map(|year| format!("({year})")).unwrap_or_default(),
                        ),
                    gap_after: false,
                })
                .collect(),
            SeriesMode::TheList => self
                .nav_rows()
                .map(|row| match row {
                    ListNavRow::Heading(g) => {
                        let group = &self.groups[g];
                        PresentedRow {
                            key: format!("heading/{:?}", group.heading),
                            data: Presentation::default()
                                .boolean("heading", true)
                                .text("marker", if self.expanded(group) { "▾" } else { "▸" })
                                .text("title", &group.heading)
                                .text("count", format!("({})", group.rows.len()))
                                .style("marker", theme::dim())
                                .style("title", theme::dim())
                                .style("count", theme::dim()),
                            gap_after: false,
                        }
                    }
                    ListNavRow::Entry(g, e) => {
                        let group = &self.groups[g];
                        let entry = &group.rows[e];
                        let name_style = if entry.dimmed {
                            theme::dim()
                        } else {
                            Style::default()
                        };
                        PresentedRow {
                            key: format!("entry/{:?}/{:?}", group.heading, entry.id),
                            data: Presentation::default()
                                .boolean("entry", true)
                                .boolean(
                                    "unlinked",
                                    entry.series_id.is_none() && entry.anidb_unavailable,
                                )
                                .text("unavailable", "⊘")
                                .style("unavailable", theme::dim())
                                .text("name", &entry.name)
                                .style("name", name_style)
                                .boolean("has-nero", entry.nero_name.is_some())
                                .text(
                                    "nero",
                                    entry
                                        .nero_name
                                        .as_ref()
                                        .map(|name| format!("“{name}”"))
                                        .unwrap_or_default(),
                                )
                                .style("nero", theme::dim())
                                .text("episode", entry.next_ep.as_deref().unwrap_or(""))
                                .style("episode", name_style)
                                .text(
                                    "available",
                                    if entry.next_ep.is_some() && entry.available {
                                        "✓"
                                    } else {
                                        ""
                                    },
                                )
                                .style("available", theme::tone_style(Tone::Good))
                                .text("watchers", &entry.watchers)
                                .style("watchers", theme::dim()),
                            gap_after: false,
                        }
                    }
                })
                .collect(),
        };
        let data = data.collection(
            "rows",
            &rows,
            self.focused.then(|| self.cursor.index()),
            Some(self.cursor.index()),
        );
        let Ok(scene) = renderer.arrange("series", area, &data) else {
            return;
        };
        renderer.record_controller("series", &scene);
        self.rendered = scene.collection("rows", &rows);
        scene.paint_with_slots(frame, |name, frame, area, style| {
            if name == "body" {
                self.rendered = renderer
                    .paint_collection(
                        frame,
                        area,
                        "series-row",
                        &rows,
                        self.focused.then(|| self.cursor.index()),
                        Some(self.cursor.index()),
                        style,
                    )
                    .unwrap_or_default();
            }
        });
        self.cursor
            .reconcile_visible(self.len(), self.rendered.hidden());
    }
}

passive_component!(SeriesPane);

impl AppComponent<Msg, NoUserEvent> for SeriesPane {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some(key) = plain(ev)
            && self
                .cursor
                .nav_visible(key, self.len(), self.rendered.hidden())
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
        pattern: KeyPattern::Char('s'),
        bar: Some(("s", "Sort")),
        action: SeriesPane::act_list_sort,
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
        pattern: KeyPattern::Char('n'),
        bar: Some(("n", "Nero name")),
        action: SeriesPane::act_list_nero,
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
        if let Ok(bundle) = super::layout::LayoutBundle::builtin() {
            self.render_layout(frame, area, &mut super::layout::Renderer::new(bundle));
        }
    }
    pub(crate) fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut super::layout::Renderer,
    ) {
        use super::props::LinkStatus;
        let (marker, state, tone, blocked) = match self.props.link {
            LinkStatus::Connecting { attempt } if attempt <= 1 => {
                ("⚡", "connecting to server…".into(), Tone::Paused, false)
            }
            LinkStatus::Connecting { attempt } => (
                "⚡",
                format!("connecting to server (attempt {attempt})…"),
                Tone::Paused,
                false,
            ),
            LinkStatus::Down => (
                "⚡",
                "connection lost — retrying…".into(),
                Tone::Paused,
                false,
            ),
            LinkStatus::Connected if self.props.playing => {
                ("▶", "playing".into(), Tone::Good, false)
            }
            LinkStatus::Connected if self.props.blockers.is_empty() => {
                ("⏸", "paused".into(), Tone::Muted, false)
            }
            LinkStatus::Connected => ("⏸", "waiting on".into(), Tone::Blocked, true),
        };
        let style = theme::tone_style(tone);
        let data = super::layout::Presentation::default()
            .text("marker", marker)
            .text("state", state)
            .text("blockers", self.props.blockers.join(", "))
            .boolean("blocked", blocked)
            .text(
                "now-label",
                if self.props.title.is_some() {
                    "Now Playing:"
                } else {
                    "Nothing playing"
                },
            )
            .text("title", self.props.title.as_deref().unwrap_or(""))
            .boolean("has-title", self.props.title.is_some())
            .style("marker", style)
            .style("state", style)
            .style("blockers", style);
        if let Ok(scene) = renderer.arrange("status", area, &data) {
            scene.paint_with_slots(frame, |_, _, _, _| {});
        }
    }

    /// Semantic progress fields; the fill is a specialized terminal primitive.
    pub(crate) fn progress_presentation(&self) -> super::layout::Presentation {
        let mut data = super::layout::Presentation::default();
        if let (Some(pos), Some(dur)) = (self.props.position_millis, self.props.duration_millis) {
            let filled = ((pos as f64 / dur.get() as f64) * 30.0).min(30.0) as usize;
            data = data
                .boolean("available", true)
                .text("open", "[")
                .text("close", "]")
                .text(
                    "fill",
                    format!("{}>{}", "=".repeat(filled), " ".repeat(30 - filled)),
                )
                .text("elapsed", mmss(pos))
                .text("duration", mmss(dur.get()))
                .text("slash", "/");
        }
        data
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

    /// Render semantic metrics with the shared health content-priority policy.
    pub(crate) fn render_layout(
        &self,
        frame: &mut Frame,
        area: Rect,
        progress: &super::layout::Presentation,
        marquee: Option<(&str, usize)>,
        renderer: &mut super::layout::Renderer,
    ) -> usize {
        use super::layout::{Presentation, PresentedItem};
        use super::props::{HealthMetric, Tone};
        let metrics = super::props::health_metrics(&self.props)
            .into_iter()
            .enumerate()
            .map(|(index, metric)| {
                let (key, label, value, tone, bandwidth, suffix) = match metric {
                    HealthMetric::Link(state, tone) => ("link", "link:", state, tone, None, false),
                    HealthMetric::Bandwidth(up, down) => (
                        "bandwidth",
                        "",
                        String::new(),
                        Tone::Muted,
                        Some((up, down)),
                        false,
                    ),
                    HealthMetric::RoundTrip(rtt, tone) => {
                        ("rtt", "rtt", format!("{rtt}ms"), tone, None, false)
                    }
                    HealthMetric::Sync(state, tone) => ("sync", "sync", state, tone, None, false),
                    HealthMetric::Probes(lost, tone) => {
                        ("probes", "probes lost", lost.to_string(), tone, None, true)
                    }
                };
                let mut data = Presentation::default()
                    .boolean("bandwidth", bandwidth.is_some())
                    .boolean("ordinary", bandwidth.is_none() && !suffix)
                    .boolean("suffix", suffix)
                    .boolean("separated", index > 0)
                    .text("separator", "·")
                    .text("label", label)
                    .text("value", value)
                    .component_style("health-metric", theme::tone_style(tone))
                    .style("separator", theme::dim());
                if let Some((up, down)) = bandwidth {
                    data = data
                        .text("up-marker", "▲")
                        .text("down-marker", "▼")
                        .text("upload", up)
                        .text("download", down);
                }
                PresentedItem {
                    key: key.into(),
                    data,
                }
            })
            .collect();
        let data = Presentation::default().list("metrics", metrics);
        let suggestion = self.props.suggestion.as_ref();
        let warning = suggestion.filter(|s| s.tone != Tone::Muted);
        let middle = warning
            .map(|s| s.text.as_str())
            .or_else(|| marquee.map(|(text, _)| text))
            .or_else(|| suggestion.map(|s| s.text.as_str()));
        let tone = if warning.is_some() || marquee.is_none() {
            suggestion.map_or(Tone::Normal, |s| s.tone)
        } else {
            Tone::Normal
        };
        renderer
            .paint_health(
                frame,
                area,
                progress,
                &data,
                middle.unwrap_or(""),
                theme::tone_style(tone),
                warning
                    .is_none()
                    .then(|| marquee.map(|(_, offset)| offset))
                    .flatten(),
            )
            .unwrap_or(0)
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
        if let Ok(bundle) = super::layout::LayoutBundle::builtin() {
            self.render_layout(frame, area, &mut super::layout::Renderer::new(bundle));
        }
    }
    pub(crate) fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut super::layout::Renderer,
    ) {
        use super::layout::{Presentation, PresentedItem};
        let mut occurrences = std::collections::BTreeMap::new();
        let items = self
            .items
            .iter()
            .enumerate()
            .map(|(index, (key, label))| {
                let occurrence = occurrences.entry((*key, *label)).or_insert(0usize);
                let identity = format!("{key:?}/{label:?}/{occurrence}");
                *occurrence += 1;
                PresentedItem {
                    key: identity,
                    data: Presentation::default()
                        .text("separator", "|")
                        .boolean("separated", index > 0)
                        .text("key", *key)
                        .text("label", *label)
                        .style("separator", theme::dim())
                        .style(
                            "key",
                            Style::default().add_modifier(tuirealm::ratatui::style::Modifier::BOLD),
                        ),
                }
            })
            .collect();
        if let Ok(scene) = renderer.arrange(
            "keybar",
            area,
            &Presentation::default().list("bindings", items),
        ) {
            scene.paint_with_slots(frame, |_, _, _, _| {});
        }
    }
}

passive_component!(KeyBar);

impl AppComponent<Msg, NoUserEvent> for KeyBar {
    fn on(&mut self, _ev: &Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}

#[cfg(test)]
mod chrome_layout_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::ui::layout::{LayoutBundle, Renderer};
    use tuirealm::ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn files_reorder_status_and_keybinding_fields() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("templates")).unwrap();
        std::fs::write(directory.path().join("templates/chrome.xml"), r#"<templates version="1"><template name="status"><column><text bind="title"/><flow><text bind="state"/><text bind="blockers"/></flow></column></template><template name="keybinding"><flow><text bind="label"/><text bind="key"/></flow></template></templates>"#).unwrap();
        let mut renderer = Renderer::new(LayoutBundle::load(directory.path()).unwrap());
        let mut status = StatusBar::default();
        status.set_props(StatusProps {
            link: super::super::props::LinkStatus::Connected,
            title: Some("episode.mkv".into()),
            blockers: vec!["kim (paused)".into()],
            ..Default::default()
        });
        let mut keys = KeyBar::default();
        keys.set_items(vec![("F3", "Settings")]);
        let mut terminal = Terminal::new(TestBackend::new(50, 10)).unwrap();
        terminal
            .draw(|frame| {
                status.render_layout(frame, Rect::new(3, 2, 40, 3), &mut renderer);
                keys.render_layout(frame, Rect::new(3, 6, 40, 1), &mut renderer);
            })
            .unwrap();
        let row = |y| {
            (3..43)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
        };
        assert!(row(2).starts_with("episode.mkv"));
        assert!(row(3).starts_with("waiting on kim (paused)"));
        assert!(row(6).starts_with("Settings F3"));
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
            dimmed: false,
        }
    }

    /// Enter on any List entry — linked or not — emits the one
    /// `BrowseListEntry` message; the dispatcher (which has the view)
    /// decides between episode browser, candidate view, and editor. The
    /// pane must not pre-resolve a franchise key: it only has the linked
    /// id, which is usually not the franchise's component root.
    #[test]
    fn list_enter_opens_the_entry_linked_or_not() {
        let mut p = SeriesPane::default();
        assert_eq!(p.mode(), SeriesMode::TheList);
        p.set_groups(&[ListGroup {
            heading: "Watching".to_string(),
            rows: vec![
                list_row(1, Some(dessplay_core::types::AniDbSeriesId(7))),
                list_row(2, None),
            ],
            collapsed: false,
        }]);
        p.on(&key(Key::Down)); // heading -> first entry (linked)

        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseListEntry(dessplay_core::types::ListEntryId(1)))
        );
        p.on(&key(Key::Down));
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseListEntry(dessplay_core::types::ListEntryId(2)))
        );
    }

    /// `n` on a List entry opens the minimal `nero_name` editor (the
    /// third pane-local meaning of `n`, after Users' NotWatching and
    /// Playlist's Nyaa); on a heading it does nothing.
    #[test]
    fn n_opens_the_nero_name_editor_on_entries_only() {
        let mut p = SeriesPane::default();
        assert_eq!(p.mode(), SeriesMode::TheList);
        p.set_groups(&[ListGroup {
            heading: "Watching".to_string(),
            rows: vec![list_row(1, None)],
            collapsed: false,
        }]);
        // Cursor starts on the heading: no message.
        assert_eq!(p.on(&key(Key::Char('n'))), None);
        p.on(&key(Key::Down));
        assert_eq!(
            p.on(&key(Key::Char('n'))),
            Some(Msg::EditNeroName(dessplay_core::types::ListEntryId(1)))
        );
    }

    fn group(heading: &str, ids: &[u128]) -> ListGroup {
        ListGroup {
            heading: heading.to_string(),
            rows: ids.iter().map(|id| list_row(*id, None)).collect(),
            collapsed: false,
        }
    }

    /// A snapshot that inserts a whole group above the cursor (a peer
    /// connecting, aging out of known-offline, or newly committing does
    /// exactly this) must not shift the cursor onto a neighbouring row:
    /// `n`/`e`/`l`/Enter all write through the cursor, so it re-anchors
    /// on the entry's identity, not its old row index. Regression for
    /// the 2026-08-20 review's mis-aimed-keystroke finding.
    #[test]
    fn set_groups_reanchors_the_cursor_on_the_same_entry() {
        let mut p = SeriesPane::default();
        p.set_groups(&[group("Watching — kim", &[1, 2])]);
        p.on(&key(Key::Down));
        p.on(&key(Key::Down)); // heading -> entry 1 -> entry 2
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseListEntry(dessplay_core::types::ListEntryId(2)))
        );
        // A freshly committed peer inserts a whole group above.
        p.set_groups(&[
            group("Watching — amu", &[9]),
            group("Watching — kim", &[1, 2]),
        ]);
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseListEntry(dessplay_core::types::ListEntryId(2)))
        );
    }

    /// The same re-anchoring for a cursor sitting on a group heading:
    /// it follows the heading, not the row index.
    #[test]
    fn set_groups_reanchors_a_heading_by_identity() {
        let mut p = SeriesPane::default();
        p.set_groups(&[group("Watching — kim", &[1])]);
        // Cursor starts on the kim heading.
        p.set_groups(&[group("Watching — amu", &[9]), group("Watching — kim", &[1])]);
        // Still on the kim heading: Down+Enter opens kim's entry, not
        // amu's.
        p.on(&key(Key::Down));
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseListEntry(dessplay_core::types::ListEntryId(1)))
        );
    }

    /// An entry can appear in several per-user groups; the anchor
    /// prefers the occurrence in the same group over the first
    /// occurrence anywhere.
    #[test]
    fn set_groups_prefers_the_same_group_for_a_shared_entry() {
        let mut p = SeriesPane::default();
        p.set_groups(&[
            group("Watching — kim", &[1]),
            group("Watching — nero", &[1]),
        ]);
        for _ in 0..3 {
            p.on(&key(Key::Down));
        }
        // nav: [kim, E1, nero, E1] — cursor on nero's copy.
        assert_eq!(p.cursor_index(), 3);
        p.set_groups(&[
            group("Watching — kim", &[7, 1]),
            group("Watching — nero", &[1]),
        ]);
        // nav: [kim, E7, E1, nero, E1] — still nero's copy.
        assert_eq!(p.cursor_index(), 4);
    }

    /// When the anchored entry vanished, the cursor falls back to its
    /// group's heading; when that is gone too, to the plain clamp — in
    /// range, no panic, still aimable either way.
    #[test]
    fn set_groups_falls_back_when_the_anchored_entry_disappears() {
        let mut p = SeriesPane::default();
        p.set_groups(&[group("Watching", &[1, 2, 3])]);
        for _ in 0..3 {
            p.on(&key(Key::Down)); // onto entry 3
        }
        // Entry gone, heading still there: land on the heading.
        p.set_groups(&[group("Watching", &[1])]);
        assert_eq!(p.cursor_index(), 0);
        p.on(&key(Key::Down));
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseListEntry(dessplay_core::types::ListEntryId(1)))
        );
        // Heading gone too: clamp into the new range, still aimable.
        p.set_groups(&[group("Planned", &[9])]);
        assert_eq!(p.cursor_index(), 1);
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::BrowseListEntry(dessplay_core::types::ListEntryId(9)))
        );
    }

    /// `s` toggles the List sort (Recency <-> Alphabetical) and emits the
    /// persistence message, mirroring the All-mode pattern.
    #[test]
    fn s_toggles_the_list_sort() {
        let mut p = SeriesPane::default();
        assert_eq!(p.mode(), SeriesMode::TheList);
        assert_eq!(p.list_sort(), ListSort::Recency);
        assert_eq!(p.on(&key(Key::Char('s'))), Some(Msg::ToggleListSort));
        assert_eq!(p.list_sort(), ListSort::Alphabetical);
        assert_eq!(p.on(&key(Key::Char('s'))), Some(Msg::ToggleListSort));
        assert_eq!(p.list_sort(), ListSort::Recency);
        // The All-mode sort is untouched.
        assert_eq!(p.sort(), SeriesSort::Title);
    }

    /// A List fixture exercised through the real `list_groups` derivation:
    /// two committed users (multi-membership), a residual shared Watching
    /// entry, an SnEnn-rendered next_ep, a dimmed nothing-to-watch row,
    /// and the shared status groups below.
    fn list_fixture() -> dessplay_core::CrdtState {
        use dessplay_core::types::{
            ActorId, AniDbMetadata, AniDbSeriesId, ListEntryId, ListStatus, MetadataSource,
            NextEpState, RelationKind, SeriesListEntry, SeriesRelation, SeriesRelations,
            SeriesWatchState, SharedTimestamp,
        };
        let a = ActorId::SERVER;
        let ts = SharedTimestamp;
        let mut state = dessplay_core::CrdtState::new();
        let entry = |name: &str, status: ListStatus, series: Option<u32>| SeriesListEntry {
            name: name.into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: series.map(AniDbSeriesId),
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        };
        // Frieren S2 (id 20, prequel id 10): kim + nero watch, ep 5 out.
        // Named exactly as the relations title (the auto-seeded case),
        // so the curated short title below substitutes in the render.
        state.put_list_entry(
            a,
            ts(1),
            ListEntryId(1),
            entry("Frieren S2", ListStatus::CurrentSeason, Some(20)),
        );
        // Akira: only kim watches; recently watched but nothing left
        // unwatched -> dims and sinks in Recency order despite leading
        // alphabetically.
        state.put_list_entry(
            a,
            ts(2),
            ListEntryId(2),
            entry("Akira", ListStatus::Active, None),
        );
        // Uncommitted: the residual shared Watching group.
        state.put_list_entry(
            a,
            ts(3),
            ListEntryId(3),
            entry("Adrift", ListStatus::Active, None),
        );
        state.put_list_entry(
            a,
            ts(4),
            ListEntryId(4),
            entry("Someday", ListStatus::Planned, None),
        );
        state.put_list_entry(
            a,
            ts(5),
            ListEntryId(5),
            entry("Old Favorite", ListStatus::Finished, None),
        );
        for (t, user, id) in [(6, "kim", 1), (7, "nero", 1), (8, "kim", 2)] {
            state.set_series_preference(
                a,
                ts(t),
                dessplay_core::types::UserId::new(user),
                ListEntryId(id),
                SeriesWatchState::Watching,
                None,
            );
        }
        state.set_next_ep(
            a,
            ts(9),
            ListEntryId(1),
            NextEpState {
                next_ep: Some("5".into()),
                available: true,
            },
        );
        state.set_next_ep(
            a,
            ts(10),
            ListEntryId(2),
            NextEpState {
                next_ep: Some("movie?".into()),
                available: false,
            },
        );
        state.set_series_relations(
            a,
            ts(11),
            AniDbSeriesId(20),
            SeriesRelations {
                title: "Frieren S2".into(),
                year: None,
                episode_count: None,
                relations: [SeriesRelation {
                    kind: RelationKind::Prequel,
                    target: AniDbSeriesId(10),
                }]
                .into_iter()
                .collect(),
                // Differs from the entry's auto-seeded name ("Frieren
                // S2") so the snapshots show the curated title winning
                // the row.
                short_titles: vec!["Frieren 2".into()],
            },
        );
        // "Adrift" holds an unwatched file (name-resolved, advertised by
        // a client), so it stays bright and floats in Recency order.
        state.set_anidb_metadata(
            a,
            ts(12),
            dessplay_core::types::Ed2kHash([3; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::FilenameDerived,
                series_name: "Adrift".into(),
                series_id: None,
                episode_number: None,
            }),
        );
        state.set_file_availability(
            a,
            ts(13),
            dessplay_core::types::UserId::new("kim"),
            dessplay_core::types::Ed2kHash([3; 16]),
            dessplay_core::types::FileAvailability::Ready,
        );
        state
    }

    fn render_list_pane(p: &mut SeriesPane) -> String {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        tuirealm::testing::buffer_to_string(
            &terminal
                .draw(|frame| p.render(frame, frame.area()))
                .unwrap()
                .buffer
                .clone(),
        )
    }

    fn fixture_groups(sort: ListSort) -> Vec<ListGroup> {
        use dessplay_core::types::UserId;
        crate::ui::props::list_groups(
            &list_fixture().view(),
            &UserId::new("kim"),
            &[UserId::new("kim"), UserId::new("nero")],
            sort,
            &[(crate::storage::SeriesKey::Name("Akira".into()), 500)]
                .into_iter()
                .collect(),
            &Default::default(),
        )
    }

    #[test]
    fn the_list_snapshot_recency() {
        let mut p = SeriesPane::default();
        p.set_groups(&fixture_groups(ListSort::Recency));
        insta::assert_snapshot!(render_list_pane(&mut p));
    }

    #[test]
    fn the_list_snapshot_alphabetical() {
        let mut p = SeriesPane::default();
        p.set_list_sort(ListSort::Alphabetical);
        p.set_groups(&fixture_groups(ListSort::Alphabetical));
        insta::assert_snapshot!(render_list_pane(&mut p));
    }
    #[test]
    #[allow(clippy::unwrap_used)]
    fn series_template_reorders_fields_and_keeps_the_selected_entry_on_reload() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("templates")).unwrap();
        std::fs::write(directory.path().join("templates/series-row.xml"), r#"<templates version="1"><template name="series-row"><column><text bind="title" if="heading"/><column if="entry"><text bind="episode"/><text bind="name" style="white-space: normal"/></column></column></template></templates>"#).unwrap();
        let mut renderer = super::super::layout::Renderer::new(
            super::super::layout::LayoutBundle::load(directory.path()).unwrap(),
        );
        let mut pane = SeriesPane {
            focused: true,
            ..Default::default()
        };
        pane.set_groups(&fixture_groups(ListSort::Recency));
        let expected = pane.groups[0].rows[0].id;
        let mut terminal = Terminal::new(TestBackend::new(30, 15)).unwrap();
        terminal
            .draw(|frame| pane.render_layout(frame, Rect::new(3, 2, 24, 11), &mut renderer))
            .unwrap();
        // First entry's episode and name occupy separate rows after the heading.
        pane.click(4, 5);
        assert_eq!(pane.act_list_edit(), Some(Msg::EditListEntry(expected)));
        renderer.install(super::super::layout::LayoutBundle::builtin().unwrap());
        terminal
            .draw(|frame| pane.render_layout(frame, Rect::new(3, 2, 24, 11), &mut renderer))
            .unwrap();
        assert_eq!(pane.act_list_edit(), Some(Msg::EditListEntry(expected)));
    }
    #[test]
    #[allow(clippy::unwrap_used)]
    fn series_filter_cursor_is_painted_in_the_measured_caption() {
        let mut pane = SeriesPane {
            mode: SeriesMode::All,
            focused: true,
            ..Default::default()
        };
        pane.act_filter_start();
        pane.filter.set_text("filter");
        let mut renderer = super::super::layout::Renderer::new(
            super::super::layout::LayoutBundle::builtin().unwrap(),
        );
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|frame| pane.render_layout(frame, Rect::new(2, 3, 35, 8), &mut renderer))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(3, 3)].symbol(), "A");
        assert!(
            buffer[(22, 3)]
                .modifier
                .contains(tuirealm::ratatui::style::Modifier::REVERSED)
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

    #[test]
    #[allow(clippy::unwrap_used)]
    fn file_only_columns_and_wrapped_rows_keep_clicks_on_the_painted_episode() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("templates")).unwrap();
        std::fs::write(dir.path().join("templates/playlist.xml"), r#"<templates version="1"><template name="playlist-row"><row id="playlist-row"><text class="playlist-watch" bind="watch" if="entry"/><text class="playlist-marker" bind="marker"/><text class="playlist-title" bind="title"/></row></template></templates>"#).unwrap();
        std::fs::write(
            dir.path().join("style.css"),
            "#playlist-frame { border: 0; } .playlist-title { white-space: normal; }",
        )
        .unwrap();
        let mut renderer = super::super::layout::Renderer::new(
            super::super::layout::LayoutBundle::load(dir.path()).unwrap(),
        );
        let (mut pane, hashes) = pane_with_rows(2);
        pane.props.rows[0].title = "one two 界界 three four five six seven".into();
        pane.props.rows[1].title = "second.mkv".into();
        let area = Rect::new(3, 2, 25, 12);
        let mut terminal = Terminal::new(TestBackend::new(40, 18)).unwrap();
        let frame = terminal
            .draw(|f| pane.render_layout(f, area, &mut renderer))
            .unwrap();
        let first: String = (3..28).map(|x| frame.buffer[(x, 2)].symbol()).collect();
        assert!(
            first.find("maybe").unwrap() < first.find("one").unwrap(),
            "{first}"
        );
        assert_eq!(
            pane.rendered.hit(4, 3),
            Some(0),
            "continuation row still belongs to the first episode"
        );
        let second = (2..14)
            .find(|y| pane.rendered.hit(4, *y) == Some(1))
            .unwrap();
        assert!(
            second > 3,
            "the next item follows the measured wrapped rows"
        );
        let text: String = (3..28)
            .map(|x| frame.buffer[(x, second)].symbol())
            .collect();
        assert!(text.contains("second.mkv"));
        pane.click(4, second);
        assert_eq!(pane.act_play(), Some(Msg::PlaySelected(hashes[1])));
        renderer.install(super::super::layout::LayoutBundle::builtin().unwrap());
        terminal
            .draw(|f| pane.render_layout(f, area, &mut renderer))
            .unwrap();
        assert_eq!(
            pane.selected_hash(),
            Some(hashes[1]),
            "reload retains controller selection"
        );
    }

    /// A long focused playlist keeps context on both sides of the cursor
    /// instead of putting the selected row at the bottom of the viewport.
    #[test]
    fn focused_cursor_is_centered_in_long_playlist() {
        let mut pane = long_pane(true, 10);
        let buffer = render(&mut pane, 9); // seven rows inside the border

        assert_eq!(row_y(&buffer, "episode-10"), Some(4));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn hidden_collection_items_leave_navigation_while_offscreen_items_remain() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("templates")).unwrap();
        std::fs::write(directory.path().join("templates/rows.xml"), r#"<templates version="1"><template name="playlist-row"><text bind="title" if="entry"/></template></templates>"#).unwrap();
        let mut renderer = super::super::layout::Renderer::new(
            super::super::layout::LayoutBundle::load(directory.path()).unwrap(),
        );
        let mut pane = long_pane(true, 0);
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        for _ in 0..100 {
            pane.on(&Event::Keyboard(KeyEvent {
                code: Key::Down,
                modifiers: KeyModifiers::NONE,
            }));
        }
        assert_eq!(
            pane.cursor.index(),
            pane.props.rows.len() - 1,
            "hidden Add New must be skipped even beyond the viewport"
        );
        pane.on(&Event::Keyboard(KeyEvent {
            code: Key::Up,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(pane.cursor.index(), pane.props.rows.len() - 2);
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
            image_url: None,
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
        let key_a = wrap_chat_line(&a, 80, &[], "me", &HashMap::new())[0].hits[0]
            .key
            .clone();
        let mut spoilers = HashMap::new();
        spoilers.insert(key_a, SpoilerState::Revealed);
        let wrapped_b = wrap_chat_line(&b, 80, &[], "me", &spoilers);
        let ChatRow { visual, hits, .. } = &wrapped_b[0];
        let text = line_text(visual);
        assert!(!text.contains("omega"), "revealing A leaked B: {text:?}");
        assert_eq!(hits.len(), 1, "B keeps its clickable hit range");
    }

    #[test]
    fn hidden_spoiler_is_scrambled_with_hit_range() {
        let line = chat_line("the ||secret|| word");
        let wrapped = wrap_chat_line(&line, 80, &[], "me", &HashMap::new());
        assert_eq!(wrapped.len(), 1);
        let ChatRow { visual, hits, .. } = &wrapped[0];
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
        let ChatRow { visual, hits, .. } = &wrapped[0];
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
        let first_hits = &wrapped[0].hits;
        let second_hits = &wrapped[1].hits;
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
        assert!(!line_text(&wrapped[0].visual).contains("secre"));
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
        let ChatRow { hits, .. } = &wrapped[0];
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
        let ChatRow { visual, hits, .. } = &wrapped[0];
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
        let ChatRow { visual, .. } = &wrapped[0];
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
            area: Rect::new(1, 1, 38, 8),
            rows: (0..count)
                .map(|i| RowRecord {
                    hits: vec![SpoilerHit {
                        cols: 5..10,
                        key: key(i),
                    }],
                    line: i,
                    body: String::new(),
                    char_start: 0,
                    body_col: 1,
                    selectable: true,
                })
                .collect(),
            image_areas: Vec::new(),
            image_hits: Vec::new(),
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
mod chat_selection_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use proptest::prelude::*;

    fn arb_line() -> impl Strategy<Value = ChatLine> {
        (0u64..8, "[ab]", "[xy]{1,3}", any::<bool>()).prop_map(
            |(millis, sender, text, separator)| ChatLine {
                time: "12:00".into(),
                sender,
                text,
                system: false,
                subtitle: false,
                separator,
                action: false,
                irc: false,
                millis,
                image_url: None,
            },
        )
    }

    /// An initial log plus the index of a non-separator line to select.
    fn arb_log_and_selection() -> impl Strategy<Value = (Vec<ChatLine>, usize)> {
        proptest::collection::vec(arb_line(), 1..8).prop_flat_map(|lines| {
            let len = lines.len();
            (Just(lines), 0..len).prop_map(|(mut lines, idx)| {
                lines[idx].separator = false;
                (lines, idx)
            })
        })
    }

    proptest! {
        /// Regression (audit 2026-08-20): the log is rebuilt from
        /// scratch on every snapshot and is neither append-only nor
        /// monotonic in length — an arbitrary `set_lines` can land
        /// between the release and the Shift-Up/Down. Extending must
        /// never panic, and must either follow the selected message's
        /// identity (highlighting and copying from *it*) or drop the
        /// selection when the message is gone.
        #[test]
        fn extend_after_arbitrary_set_lines_never_panics_and_follows_identity(
            (initial, idx) in arb_log_and_selection(),
            replacement in proptest::collection::vec(arb_line(), 0..8),
            up in any::<bool>(),
        ) {
            let mut pane = ChatPane::default();
            pane.set_lines(initial.clone());
            // The drag anchors on line `idx` exactly as `point_at`
            // would produce it: its first char.
            pane.selection = Some(Selection::Dragging {
                anchor: SelPoint { line: idx, floor: 0, ceil: 1 },
                focus: Some(SelPoint { line: idx, floor: 0, ceil: 1 }),
            });
            prop_assert!(pane.mouse_up(0).is_some(), "release must copy");
            pane.set_lines(replacement.clone());
            let key = LineKey::of(&initial[idx]);
            // Must not panic, whatever the replacement log looks like.
            let copied = pane.extend_selection(up, 0);
            if replacement.iter().any(|line| key.matches(line)) {
                // The selected message survived the rebuild: the
                // extension must anchor on *it* — never a neighbour.
                prop_assert!(copied.is_some(), "extension must copy");
                let Some(SelRange::Lines { anchor, .. }) = pane.selection_range() else {
                    return Err(TestCaseError::fail("extension must hold whole lines"));
                };
                prop_assert!(
                    key.matches(&pane.lines[anchor]),
                    "the highlight must anchor on the selected message"
                );
            } else {
                // Gone: the selection is dropped, nothing is copied.
                prop_assert!(copied.is_none(), "nothing sane to extend");
                prop_assert!(!pane.selection_held(), "stale hold must drop");
            }
        }
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

#[cfg(test)]
mod chat_image_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use tuirealm::ratatui::Terminal;
    use tuirealm::ratatui::backend::TestBackend;

    const URL: &str = "https://x.example/shot.png";

    fn image_line(millis: u64, text: &str) -> ChatLine {
        ChatLine {
            time: "12:00".to_string(),
            sender: "dagger".to_string(),
            text: text.to_string(),
            system: false,
            subtitle: false,
            separator: false,
            action: false,
            irc: true,
            millis,
            image_url: Some(URL.to_string()),
        }
    }

    /// A tall test image: 100×400px is 10×20 cells under the halfblocks
    /// picker's fixed 10×20 font, so it always overflows the ⅓ cap.
    /// Striped so adjacent pixel rows differ after downscaling — a
    /// uniform color would encode as plain background-colored spaces,
    /// while stripes force visible `▀` half-block glyphs.
    fn tall_image() -> image::DynamicImage {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(100, 400, |_, y| {
            if (y / 25).is_multiple_of(2) {
                image::Rgba([200, 40, 40, 255])
            } else {
                image::Rgba([40, 40, 200, 255])
            }
        }))
    }

    /// A ready-to-render pane: halfblocks picker, one image message, the
    /// image delivered.
    fn ready_pane() -> ChatPane {
        let mut pane = ChatPane::default();
        pane.set_picker(ratatui_image::picker::Picker::halfblocks());
        pane.set_lines(vec![image_line(1_000, "look at this")]);
        assert_eq!(pane.sync_images(true), vec![URL.to_string()]);
        pane.set_image(URL, Ok(tall_image()));
        pane
    }

    fn draw(pane: &mut ChatPane, width: u16, height: u16) -> tuirealm::ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| pane.render(frame, frame.area()))
            .unwrap()
            .buffer
            .clone()
    }

    #[test]
    fn attachment_border_follows_timestamp_and_is_included_in_height_cap() {
        let mut pane = ready_pane();
        let buffer = draw(&mut pane, 40, 30);
        let pixels = pane.rendered.image_areas[0];
        assert_eq!(
            pixels.x, 8,
            "log border + measured timestamp/separator + attachment border"
        );
        assert_eq!(
            pixels.height, 6,
            "eight-row budget includes two border rows"
        );
        assert_eq!(buffer[(pixels.x - 1, pixels.y - 1)].symbol(), "┌");
        assert_eq!(buffer[(pixels.x - 1, pixels.bottom())].symbol(), "└");
    }

    #[test]
    fn file_only_attachment_styles_change_frame_and_alignment() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("style.css"),
            "#timestamp-gutter { display: none; } #chat-attachment-image { border: 0; }",
        )
        .unwrap();
        let mut renderer = super::super::layout::Renderer::new(
            super::super::layout::LayoutBundle::load(dir.path()).unwrap(),
        );
        let mut pane = ready_pane();
        let mut terminal = Terminal::new(TestBackend::new(40, 30)).unwrap();
        terminal
            .draw(|f| pane.render_layout(f, f.area(), &mut renderer))
            .unwrap();
        assert_eq!(pane.rendered.image_areas[0].x, 1);
        assert_eq!(pane.rendered.image_areas[0].height, 8);
    }

    #[test]
    fn attachment_spacing_stays_within_the_frame_budget() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("style.css"),
            "#chat-attachment-image { padding: 1ch; margin: 1ch; }",
        )
        .unwrap();
        let mut renderer = super::super::layout::Renderer::new(
            super::super::layout::LayoutBundle::load(dir.path()).unwrap(),
        );
        let mut pane = ready_pane();
        let mut terminal = Terminal::new(TestBackend::new(40, 30)).unwrap();
        terminal
            .draw(|f| pane.render_layout(f, f.area(), &mut renderer))
            .unwrap();
        assert_eq!(
            pane.rendered.image_areas[0].height, 2,
            "two borders + padding + margins consume six of eight rows"
        );
        assert_eq!(
            pane.rendered.rows.iter().filter(|r| !r.selectable).count(),
            8
        );
    }

    #[test]
    fn cropped_attachment_keeps_original_pixels_size_and_border_edges() {
        let mut pane = ready_pane();
        let full = draw(&mut pane, 40, 30);
        let pixels = pane.rendered.image_areas[0];
        for n in 0..20 {
            let mut line = image_line(2000 + n, "later");
            line.image_url = None;
            pane.lines.push(line);
        }
        let cropped = draw(&mut pane, 40, 30);
        let clipped = pane.rendered.image_areas[0];
        assert_eq!(clipped.y, 1);
        assert_eq!(clipped.height, 4);
        for y in 0..clipped.height {
            for x in 0..pixels.width {
                assert_eq!(
                    cropped[(clipped.x + x, clipped.y + y)],
                    full[(pixels.x + x, pixels.y + y + 2)],
                    "crop must sample the original fitted image"
                );
            }
        }
        assert_eq!(
            cropped[(clipped.x - 1, 1)].symbol(),
            "│",
            "do not draw a new top border at the crop"
        );
        assert!(
            matches!(&pane.images[URL], ImageSlot::Ready { sliced: Some((size, _)), .. } if size.height == 6)
        );
    }

    #[test]
    fn no_usable_bordered_interior_leaves_the_original_link() {
        let mut pane = ready_pane();
        pane.lines[0].text = URL.into();
        let buffer = draw(&mut pane, 80, 10);
        assert!(pane.rendered.image_areas.is_empty());
        assert!(tuirealm::testing::buffer_to_string(&buffer).contains(URL));
        draw(&mut pane, 8, 30);
        assert!(pane.rendered.image_areas.is_empty());
    }

    #[test]
    fn resizing_and_new_messages_preserve_scrolled_source_context() {
        let mut pane = ChatPane {
            scroll_offset: 20,
            ..Default::default()
        };
        pane.set_lines(
            (0..30)
                .map(|i| {
                    let mut line = image_line(
                        i,
                        "Unicode 界界界 one two three four five six seven eight nine ten",
                    );
                    line.image_url = None;
                    line
                })
                .collect(),
        );
        draw(&mut pane, 50, 20);
        let (key, source, _) = pane.scroll_anchor.clone().unwrap();
        draw(&mut pane, 25, 20);
        let first = &pane.rendered.rows[0];
        assert!(key.matches(&pane.lines[first.line]));
        assert!(first.char_start <= source);
        assert!(first.char_start + first.body.chars().count() >= source);
        pane.lines.push(image_line(100, "incoming message"));
        draw(&mut pane, 25, 20);
        assert!(key.matches(&pane.lines[pane.rendered.rows[0].line]));
    }

    #[test]
    fn borderless_chat_and_custom_indentation_share_painted_hit_geometry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("style.css"),
            "#chat-log-frame { border: 0; hanging-indent: 7ch; }",
        )
        .unwrap();
        let mut renderer = super::super::layout::Renderer::new(
            super::super::layout::LayoutBundle::load(dir.path()).unwrap(),
        );
        let mut pane = ChatPane::default();
        pane.lines.push(image_line(
            1,
            "one two three four five six seven eight nine ten",
        ));
        let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
        terminal
            .draw(|f| pane.render_layout(f, Rect::new(3, 2, 30, 16), &mut renderer))
            .unwrap();
        assert_eq!(pane.rendered.area.x, 3);
        assert_eq!(pane.rendered.area.y, 2);
        assert!(pane.rendered.point_at(3, 2).is_some());
        assert!(pane.rendered.point_at(2, 2).is_none());
        let continuation = &pane.rendered.rows[1];
        assert_eq!(continuation.body_col, 10);
        assert_eq!(
            pane.rendered.point_at(10, 3).unwrap().floor,
            continuation.char_start
        );
    }

    /// The reserved band is capped at a third of the log viewport and its
    /// rows are never selectable, so clicks and drags resolve to real text.
    #[test]
    fn image_rows_cap_at_a_third_and_are_not_selectable() {
        let mut pane = ready_pane();
        // 30-row pane: log = 30 - 3 (input) = 27, minus borders = 25
        // visible rows; the cap is 25 / 3 = 8.
        draw(&mut pane, 40, 30);
        let image_rows: Vec<&RowRecord> = pane
            .rendered
            .rows
            .iter()
            .filter(|row| !row.selectable && row.body.is_empty())
            .collect();
        assert_eq!(image_rows.len(), 8, "reserved rows");
        assert_eq!(pane.rendered.image_areas.len(), 1);
        assert_eq!(pane.rendered.image_areas[0].height, 6);
        // A click on an image row maps to no exact point, and the
        // nearest point resolves to the message text above.
        let rect = pane.rendered.image_areas[0];
        assert_eq!(pane.rendered.point_at(rect.x + 1, rect.y + 1), None);
        let near = pane.rendered.point_near(rect.x + 1, rect.y + 1).unwrap();
        assert_eq!(near.line, 0);
    }

    /// Scrolling counts image rows like any other visual rows, and the
    /// clamp keeps the log in range with the band included.
    #[test]
    fn scroll_clamp_counts_image_rows() {
        let mut pane = ready_pane();
        pane.scroll_offset = usize::MAX;
        draw(&mut pane, 40, 30);
        // 1 text row + 8 image rows, all visible in a 25-row viewport:
        // nothing to scroll.
        assert_eq!(pane.scroll_offset, 0);
    }

    /// Half-block pixels actually land in the reserved band.
    #[test]
    fn halfblock_cells_render_in_the_reserved_band() {
        let mut pane = ready_pane();
        let buffer = draw(&mut pane, 40, 30);
        let rect = pane.rendered.image_areas[0];
        let band: String = (rect.y..rect.y + rect.height)
            .flat_map(|y| (rect.x..rect.x + rect.width).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol().to_string())
            .collect();
        assert!(
            band.contains('▀'),
            "no half-block cells in rect {rect:?} band {band:?} full {:?}",
            tuirealm::testing::buffer_to_string(&buffer)
        );
    }

    /// Without a picker (plain tests, or a failed protocol query) the
    /// URL renders as text only — no reserved rows, no fetches.
    #[test]
    fn no_picker_means_no_reservation_and_no_fetches() {
        let mut pane = ChatPane::default();
        pane.set_lines(vec![image_line(1_000, "look at this")]);
        assert!(pane.sync_images(true).is_empty());
        draw(&mut pane, 40, 30);
        assert!(pane.rendered.image_areas.is_empty());
        assert!(pane.rendered.rows.iter().all(|row| row.selectable));
    }

    /// Suppression (a modal is open) hides the band without disturbing
    /// the store; the next unsuppressed frame brings it back.
    #[test]
    fn suppression_hides_the_band() {
        let mut pane = ready_pane();
        pane.set_images_suppressed(true);
        draw(&mut pane, 40, 30);
        assert!(pane.rendered.image_areas.is_empty());
        pane.set_images_suppressed(false);
        draw(&mut pane, 40, 30);
        assert_eq!(pane.rendered.image_areas.len(), 1);
    }

    /// The same URL posted twice renders under its first occurrence only
    /// (one StatefulProtocol must not draw at two rects per frame), and
    /// sync_images requests it once.
    #[test]
    fn repeated_url_reserves_once_and_fetches_once() {
        let mut pane = ChatPane::default();
        pane.set_picker(ratatui_image::picker::Picker::halfblocks());
        pane.set_lines(vec![
            image_line(1_000, "first post"),
            image_line(2_000, "same link again"),
        ]);
        assert_eq!(pane.sync_images(true), vec![URL.to_string()]);
        assert!(pane.sync_images(true).is_empty(), "no re-request");
        pane.set_image(URL, Ok(tall_image()));
        draw(&mut pane, 40, 30);
        assert_eq!(pane.rendered.image_areas.len(), 1, "one band");
    }

    /// Disabling the setting clears the store; re-enabling re-requests.
    #[test]
    fn disabling_clears_and_reenabling_refetches() {
        let mut pane = ready_pane();
        assert!(pane.sync_images(false).is_empty());
        draw(&mut pane, 40, 30);
        assert!(pane.rendered.image_areas.is_empty());
        assert_eq!(pane.sync_images(true), vec![URL.to_string()]);
    }

    /// A failed fetch reserves nothing and is never re-requested while
    /// its message stays in the log.
    #[test]
    fn failed_fetch_reserves_nothing() {
        let mut pane = ChatPane::default();
        pane.set_picker(ratatui_image::picker::Picker::halfblocks());
        pane.set_lines(vec![image_line(1_000, "dead link")]);
        assert_eq!(pane.sync_images(true), vec![URL.to_string()]);
        pane.set_image(URL, Err("404".to_string()));
        draw(&mut pane, 40, 30);
        assert!(pane.rendered.image_areas.is_empty());
        assert!(pane.sync_images(true).is_empty());
    }

    /// Pruning: when the message leaves the log window, the slot goes
    /// with it (bounded memory), and a later answer for it is ignored.
    #[test]
    fn slots_prune_with_their_messages() {
        let mut pane = ChatPane::default();
        pane.set_picker(ratatui_image::picker::Picker::halfblocks());
        pane.set_lines(vec![image_line(1_000, "going away")]);
        assert_eq!(pane.sync_images(true), vec![URL.to_string()]);
        pane.set_lines(Vec::new());
        assert!(pane.sync_images(true).is_empty());
        pane.set_image(URL, Ok(tall_image()));
        draw(&mut pane, 40, 30);
        assert!(pane.rendered.image_areas.is_empty());
    }
}

#[cfg(test)]
mod chat_layout_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::ui::layout::{LayoutBundle, Renderer};
    use tuirealm::ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn controller_contents_survive_authored_overlay_slots() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("templates")).unwrap();
        std::fs::write(directory.path().join("templates/layers.xml"), r#"<templates version="1">
          <template name="chat"><box><overlay placement="modal" style="width: 100%; height: 100%"><scroll><slot name="log"/></scroll><slot name="input" style="height: 1lh; flex-shrink: 0"/></overlay></box></template>
          <template name="recent-chat"><box><overlay placement="modal" style="width: 100%; height: 100%"><scroll><slot name="log"/></scroll></overlay></box></template>
          <template name="playlist"><box><overlay placement="modal" style="width: 100%; height: 100%"><scroll><slot name="body"/></scroll></overlay></box></template>
        </templates>"#).unwrap();
        let mut renderer = Renderer::new(LayoutBundle::load(directory.path()).unwrap());
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let mut chat = ChatPane::default();
        chat.set_lines(vec![message(1, "visible message")]);
        for recent in [false, true] {
            terminal
                .draw(|frame| {
                    if recent {
                        chat.render_recent(frame, frame.area(), &mut renderer);
                    } else {
                        chat.render_layout(frame, frame.area(), &mut renderer);
                    }
                })
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("visible message"), "recent={recent}: {text}");
        }
        let mut playlist = PlaylistPane::default();
        terminal
            .draw(|frame| playlist.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("[Add New]"), "{text}");
    }

    fn message(millis: u64, text: &str) -> ChatLine {
        ChatLine {
            time: "12:00".into(),
            sender: "kim".into(),
            text: text.into(),
            system: false,
            subtitle: false,
            separator: false,
            action: false,
            irc: false,
            millis,
            image_url: None,
        }
    }
    #[test]
    fn live_tail_instantiates_visible_messages_and_reuses_their_scenes() {
        let mut pane = ChatPane::default();
        pane.set_lines((0..5000).map(|id| message(id, "one line")).collect());
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        let first = renderer.arrangement_count();
        assert!(
            first < 40,
            "only the viewport should need layout, got {first} trees"
        );
        terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        assert_eq!(renderer.arrangement_count(), first);
        pane.lines.push(message(5001, "new line"));
        terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        assert_eq!(renderer.arrangement_count(), first + 1);
    }
    #[test]
    fn spoiler_animation_reuses_measured_geometry() {
        let mut pane = ChatPane::default();
        pane.set_lines(vec![message(1, "before ||secret words|| after")]);
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let mut terminal = Terminal::new(TestBackend::new(60, 15)).unwrap();
        terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        let (row, hit) = pane
            .rendered
            .rows
            .iter()
            .enumerate()
            .find_map(|(row, record)| record.hits.first().map(|hit| (row, hit)))
            .unwrap();
        pane.click(hit.cols.start, pane.rendered.area.y + row as u16, 100);
        terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        let measured = renderer.arrangement_count();
        assert!(pane.advance_spoilers(100 + SPOILER_FRAME_MS));
        terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        assert_eq!(renderer.arrangement_count(), measured);
    }
    #[test]
    fn file_only_message_reordering_retains_draft_selection_and_spoiler_actions() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("templates")).unwrap();
        std::fs::write(directory.path().join("templates/chat-message.xml"), r#"<templates version="1"><template name="chat-message"><column><row style="gap: 1ch"><text bind="sender"/><text bind="timestamp"/></row><rich bind="body" style="white-space: normal; margin: 0 0 0 3ch"/></column></template></templates>"#).unwrap();
        let mut renderer = Renderer::new(LayoutBundle::load(directory.path()).unwrap());
        let mut pane = ChatPane::default();
        pane.set_input("unsent draft".into());
        pane.set_lines(vec![message(1, "before ||hidden words|| after 界")]);
        let mut terminal = Terminal::new(TestBackend::new(60, 15)).unwrap();
        let frame = terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        let text: String = frame
            .buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("kim 12:00"));
        let (row, record) = pane
            .rendered
            .rows
            .iter()
            .enumerate()
            .find(|(_, record)| record.body.starts_with("before"))
            .unwrap();
        assert_eq!(record.body_col, pane.rendered.area.x + 3);
        let (x, y) = (record.body_col, pane.rendered.area.y + row as u16);
        pane.mouse_down(x, y);
        pane.mouse_drag(x + 5, y);
        assert_eq!(pane.mouse_up(0).as_deref(), Some("before"));
        renderer.install(LayoutBundle::builtin().unwrap());
        terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        assert!(pane.selection_held());
        assert_eq!(pane.text(), "unsent draft");
        assert_eq!(
            pane.selection_range(),
            Some(SelRange::Partial {
                line: 0,
                start: 0,
                end: 6
            })
        );
        let (row, hit) = pane
            .rendered
            .rows
            .iter()
            .enumerate()
            .find_map(|(row, record)| record.hits.first().map(|hit| (row, hit)))
            .unwrap();
        let (x, y) = (hit.cols.start, pane.rendered.area.y + row as u16);
        pane.click(x, y, 100);
        pane.click(x, y, 150);
        let frame = terminal
            .draw(|frame| pane.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        let text: String = frame
            .buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("hidden words"));
        assert!(!pane.reveal_newest_visible());
    }
}
