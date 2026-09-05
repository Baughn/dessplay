//! The UI dispatcher: focus ring, modal stack, event routing, and the
//! Elm `update()` — the part of tui-realm's `Application` we replaced
//! with synchronous code so whole-app tests are deterministic
//! (ui-architecture.md, Framework Choice).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use dessplay_core::StateView;
use dessplay_core::derive::{self, DerivedUserState};
use dessplay_core::franchise::{self, FranchiseKey};
use dessplay_core::net::PeerInfo;
use dessplay_core::types::{
    Ed2kHash, ListEntryId, ManualState, NextEpState, PlaybackIntent, SeriesListEntry,
    SeriesWatchState, SharedTimestamp, UserId, encode_action,
};
use tuirealm::component::AppComponent;
use tuirealm::event::{
    Event, Key, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind, NoUserEvent,
};
use tuirealm::props::{AttrValue, Attribute};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Position, Rect};
use tuirealm::ratatui::widgets::{Block, Borders};

use super::components::{
    ChatPane, HealthLine, KeyBar, PlaylistPane, SeriesMode, SeriesPane, StatusBar, UsersPane,
};
use super::modals::{
    AniDbSearchModal, BrowserLibrary, ChangelogModal, ConfirmModal, EpisodeBrowser, FileBrowser,
    ListEditModal, LocalCopyOfferModal, NeroNameModal, NyaaActiveImport, NyaaSearchModal, Season,
    SettingsModal,
};
use super::msg::{BrowseRequest, Msg, UserAction};
use super::props;
use super::speaker_colors::SpeakerColors;
use super::theme::ColorDepth;
use crate::actors::sync::Mutation;
use crate::config::{MarqueeMode, PaneLayout, Settings, SubtitleMode, SubtitleSpeakerOverflow};
use crate::player::SpeakerName;

/// Everything the UI renders from, refreshed on every state/peer
/// change.
#[derive(Clone, Debug, Default)]
pub struct UiSnapshot {
    /// The resolved CRDT view. `Arc`-wrapped so the per-tick fan-out --
    /// the run loop's diff baseline plus the copy handed to this UI thread
    /// -- shares one allocation (a refcount bump) instead of deep-cloning
    /// the whole view every playback-position tick.
    pub view: std::sync::Arc<StateView>,
    /// The latest peer list.
    pub peers: Vec<PeerInfo>,
    /// Known usernames not currently in `peers` (design.md #15) — valid
    /// targets for `n` / `/skip <name>` even before they show up.
    pub known_offline: Vec<dessplay_core::net::KnownUser>,
    /// **Wall-clock** millis when this snapshot was built — the "now"
    /// the Users pane's "last seen Nd ago" labels are relative to, and
    /// the clock domain day separators deliberately use (design.md,
    /// System Messages: local time, per client). Threaded explicitly
    /// (not read from the system clock inside `props`) so the mapping
    /// stays a pure, testable function of its inputs.
    pub now: u64,
    /// **Shared-clock** millis when this snapshot was built (`now` plus
    /// the time-sync offset). This is the domain synced LWW stamps live
    /// in, so anything comparing against a stamp — the marquee's
    /// staleness guard — must use this, never `now`: a wall clock
    /// leading the group would otherwise read fresh stamps as stale
    /// (and a lagging one would replay last night's leftovers).
    pub shared_now: u64,
    /// Local watch history: series (by AniDB id or filename-parsed name)
    /// -> last-watched millis (drives the Recent mode sort).
    pub recency: BTreeMap<crate::storage::SeriesKey, u64>,
    /// Hashes that live only in the local download cache (not in a media
    /// root). These render a dim "temporary" marker and are the only
    /// rows the archive action operates on. Local, not synced.
    pub cache_hashes: BTreeSet<Ed2kHash>,
    /// Personal watch history (85% rule): hash -> watched-at millis.
    /// Feeds the episode browser's muting alongside the group watched
    /// flag (design.md #11), and — via the timestamps — which copy of a
    /// multi-copy episode the group has actually been playing
    /// (`props::opening_row`). Local, not synced.
    pub personal_watched: BTreeMap<Ed2kHash, i64>,
    /// Server-link state, from the network actor's connect lifecycle.
    /// The status bar shows it whenever it isn't `Connected`.
    pub link: props::LinkStatus,
    /// Connection/sync health for the borderless status field under the
    /// playlist (design.md, Connection Health Line). Run-loop state like
    /// `link`, not CRDT state.
    pub health: props::HealthProps,
}

/// Log one outgoing [`UserAction`] at debug. Mutations log their
/// variant name; `SaveSettings` deliberately logs no contents (the
/// settings carry the password).
fn log_action(action: &UserAction) {
    match action {
        UserAction::Mutate(Mutation::SetSeriesPreference {
            user,
            entry,
            pref,
            set_by,
        }) => {
            let actor = set_by.as_ref().unwrap_or(user);
            tracing::info!(
                %user,
                entry = entry.0,
                ?pref,
                set_by = %actor,
                reason = "user action",
                "requesting series preference change"
            );
        }
        UserAction::Mutate(mutation) => {
            tracing::debug!(mutation = mutation.name(), "user action: Mutate");
        }
        UserAction::HashAndAdd { path, .. } => {
            tracing::debug!(path = %path.display(), "user action: HashAndAdd");
        }
        UserAction::AddByHash { hash, .. } => {
            tracing::debug!(%hash, "user action: AddByHash");
        }
        UserAction::SearchNyaa { query } => {
            tracing::debug!(%query, "user action: SearchNyaa");
        }
        UserAction::StartNyaaImport { id, result, .. } => {
            tracing::debug!(import = id.0, title = %result.title, "user action: StartNyaaImport");
        }
        UserAction::CancelNyaaImport { id } => {
            tracing::debug!(import = id.0, "user action: CancelNyaaImport");
        }
        UserAction::SaveSettings(..) => tracing::debug!("user action: SaveSettings"),
        UserAction::AniDbSearch { query } => {
            tracing::debug!(%query, "user action: AniDbSearch");
        }
        UserAction::MarkWatched { file, watched } => {
            tracing::debug!(hash = %file, watched, "user action: MarkWatched");
        }
        UserAction::MapFile { path, .. } => {
            tracing::debug!(path = %path.display(), "user action: MapFile");
        }
        UserAction::LocalCopyOfferDismissed { file } => {
            tracing::debug!(%file, "user action: LocalCopyOfferDismissed");
        }
        UserAction::ChangelogSeen { marker } => {
            tracing::debug!(%marker, "user action: ChangelogSeen");
        }
        UserAction::Archive { filename, .. } => {
            tracing::debug!(%filename, "user action: Archive");
        }
        UserAction::Browse(request) => {
            let kind = match request {
                crate::ui::msg::BrowseRequest::Add { .. } => "Add",
                crate::ui::msg::BrowseRequest::Map { .. } => "Map",
            };
            tracing::debug!(kind, "user action: Browse");
        }
        // Info, not debug: a user-visible state change (the replica is
        // discarded and re-adopted).
        UserAction::ResetSyncedState => tracing::info!("user action: ResetSyncedState"),
        UserAction::Notice(text) => tracing::debug!(%text, "user action: Notice"),
        UserAction::Summon(absent) => {
            tracing::debug!(count = absent.len(), "user action: Summon");
        }
        // Contents deliberately not logged: chat may hold spoilers or
        // anything else the users consider theirs.
        UserAction::CopyToClipboard(text) => {
            tracing::debug!(chars = text.chars().count(), "user action: CopyToClipboard");
        }
        UserAction::Quit => tracing::debug!("user action: Quit"),
    }
}

/// Pane focus ring: Chat -> Series -> Users -> Playlist.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    Chat,
    Series,
    Users,
    Playlist,
}

/// Screen rectangles of the four panes as of the last draw, for mouse
/// hit-testing. Zero-sized until the first draw, so every click misses
/// — no `Option` dance needed. `chat` spans the whole left column
/// (log, input, progress line, and the subtitle pane when shown): a
/// click anywhere in it means "the chat side". `subs` overlaps it and
/// only matters for the wheel, which is hit-tested against it first.
#[derive(Clone, Copy, Default)]
struct PaneRects {
    /// The whole region the panes divide (everything above the
    /// progress line) — the denominator for the column splitter.
    area: Rect,
    chat: Rect,
    /// The separate subtitle pane; zero-sized unless it was drawn.
    subs: Rect,
    series: Rect,
    users: Rect,
    playlist: Rect,
}

/// The four draggable pane boundaries (design.md, Mouse support:
/// resizable panes). Each is the pair of adjacent border cells the two
/// panes draw against each other — two cells wide, so the grab is
/// forgiving — and each maps to exactly one `PaneLayout` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Splitter {
    /// Between the chat column and the right column.
    Column,
    /// Between the chat log and the separate subtitle pane.
    ChatSubs,
    /// Between the Series and Users panes.
    SeriesUsers,
    /// Between the Users and Playlist panes.
    UsersPlaylist,
}

impl PaneRects {
    /// Which splitter, if any, the pointer is over. Checked before the
    /// pane hit-test, since the strips overlap the panes' border cells.
    fn splitter_at(&self, position: Position) -> Option<Splitter> {
        // The two border columns/rows that meet at the boundary.
        let vertical_strip = |left: Rect, right: Rect| {
            Rect::new(
                right.x.saturating_sub(1),
                left.y,
                2,
                left.height.max(right.height),
            )
        };
        let horizontal_strip = |top: Rect, bottom: Rect| {
            Rect::new(
                top.x,
                bottom.y.saturating_sub(1),
                top.width.max(bottom.width),
                2,
            )
        };
        let candidates = [
            (Splitter::Column, vertical_strip(self.chat, self.series)),
            (
                Splitter::SeriesUsers,
                horizontal_strip(self.series, self.users),
            ),
            (
                Splitter::UsersPlaylist,
                horizontal_strip(self.users, self.playlist),
            ),
            (
                Splitter::ChatSubs,
                // Zero-sized (never hit) while the subtitle pane is hidden.
                if self.subs.height == 0 {
                    Rect::default()
                } else {
                    Rect::new(
                        self.chat.x,
                        self.subs.y.saturating_sub(1),
                        self.chat.width,
                        2,
                    )
                },
            ),
        ];
        candidates
            .into_iter()
            .find_map(|(splitter, rect)| rect.contains(position).then_some(splitter))
    }

    /// The layout that puts `splitter` under the pointer: the pointer's
    /// offset into the divided region, as a whole percentage of it.
    /// Out-of-range results are clamped by `PaneLayout::clamped`, so a
    /// drag past the edge just pins the pane at its minimum.
    fn layout_for_drag(
        &self,
        splitter: Splitter,
        position: Position,
        mut layout: PaneLayout,
    ) -> PaneLayout {
        fn percent(offset: u16, extent: u16) -> u8 {
            if extent == 0 {
                return 0;
            }
            (u32::from(offset) * 100 / u32::from(extent)).min(100) as u8
        }
        let right = Rect::new(
            self.series.x,
            self.series.y,
            self.series.width,
            self.series.height + self.users.height + self.playlist.height,
        );
        match splitter {
            Splitter::Column => {
                layout.chat_width =
                    percent(position.x.saturating_sub(self.area.x), self.area.width);
            }
            Splitter::ChatSubs => {
                // The subtitle pane is the *bottom* share of the column.
                let from_bottom = self.chat.bottom().saturating_sub(position.y);
                layout.subtitle_height = percent(from_bottom, self.chat.height);
            }
            Splitter::SeriesUsers => {
                // Keep the users/playlist boundary where it is: moving
                // this splitter trades rows between series and users
                // only, so the series share is clamped against that
                // boundary here (not by `clamped`, which would move it).
                let users_end = layout.series_height.saturating_add(layout.users_height);
                let series = percent(position.y.saturating_sub(right.y), right.height)
                    .clamp(PaneLayout::MIN, users_end.saturating_sub(PaneLayout::MIN));
                layout.series_height = series;
                layout.users_height = users_end - series;
            }
            Splitter::UsersPlaylist => {
                let users_end = percent(position.y.saturating_sub(right.y), right.height);
                layout.users_height = users_end.saturating_sub(layout.series_height);
            }
        }
        layout.clamped()
    }
}

impl Focus {
    /// Clockwise: Chat -> Series -> Users -> Playlist -> Chat.
    fn next(self) -> Self {
        match self {
            Focus::Chat => Focus::Series,
            Focus::Series => Focus::Users,
            Focus::Users => Focus::Playlist,
            Focus::Playlist => Focus::Chat,
        }
    }

    /// The inverse of `next` (Shift-Tab).
    fn prev(self) -> Self {
        match self {
            Focus::Chat => Focus::Playlist,
            Focus::Series => Focus::Chat,
            Focus::Users => Focus::Series,
            Focus::Playlist => Focus::Users,
        }
    }
}

/// An open modal.
enum Modal {
    Files(FileBrowser),
    Settings(SettingsModal),
    Episodes(EpisodeBrowser),
    ListEdit(ListEditModal),
    NeroName(NeroNameModal),
    AniDbSearch(AniDbSearchModal),
    NyaaSearch(NyaaSearchModal),
    LocalCopyOffer(LocalCopyOfferModal),
    Changelog(ChangelogModal),
    Logs(super::modals::LogModal),
    Confirm(ConfirmModal),
}

impl Modal {
    fn as_component(&mut self) -> &mut dyn AppComponent<Msg, NoUserEvent> {
        match self {
            Modal::Files(modal) => modal,
            Modal::Settings(modal) => modal,
            Modal::Episodes(modal) => modal,
            Modal::ListEdit(modal) => modal,
            Modal::NeroName(modal) => modal,
            Modal::AniDbSearch(modal) => modal,
            Modal::NyaaSearch(modal) => modal,
            Modal::LocalCopyOffer(modal) => modal,
            Modal::Changelog(modal) => modal,
            Modal::Logs(modal) => modal,
            Modal::Confirm(modal) => modal,
        }
    }

    fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            Modal::Files(modal) => modal.keybindings(),
            Modal::Settings(modal) => modal.keybindings(),
            Modal::Episodes(modal) => modal.keybindings(),
            Modal::ListEdit(modal) => modal.keybindings(),
            Modal::NeroName(modal) => modal.keybindings(),
            Modal::AniDbSearch(modal) => modal.keybindings(),
            Modal::NyaaSearch(modal) => modal.keybindings(),
            Modal::LocalCopyOffer(modal) => modal.keybindings(),
            Modal::Changelog(modal) => modal.keybindings(),
            Modal::Logs(modal) => modal.keybindings(),
            Modal::Confirm(modal) => modal.keybindings(),
        }
    }

    /// The modal's name, for logging.
    fn name(&self) -> &'static str {
        match self {
            Modal::Files(_) => "Files",
            Modal::Settings(_) => "Settings",
            Modal::Episodes(_) => "Episodes",
            Modal::ListEdit(_) => "ListEdit",
            Modal::NeroName(_) => "NeroName",
            Modal::AniDbSearch(_) => "AniDbSearch",
            Modal::NyaaSearch(_) => "NyaaSearch",
            Modal::LocalCopyOffer(_) => "LocalCopyOffer",
            Modal::Changelog(_) => "Changelog",
            Modal::Logs(_) => "Logs",
            Modal::Confirm(_) => "Confirm",
        }
    }
}

/// One subtitle line in the rolling log. `video_millis` is the in-video
/// position (the displayed timestamp); `arrival_millis` is the
/// wall-clock arrival used to interleave with chat in Intermixed mode.
#[derive(Clone, Debug)]
struct SubtitleEntry {
    video_millis: u64,
    arrival_millis: u64,
    text: String,
    /// The ASS speaker/actor, if the cue carried one. Used for optional name
    /// display in both text modes and for coloring in separate-pane mode.
    speaker: Option<SpeakerName>,
    /// Stable color slot assigned while this speaker was active. Kept on
    /// the entry so a sparse, older visible line does not change color when
    /// the rolling activity tracker later recycles its slot.
    speaker_slot: Option<usize>,
}

/// One marquee pass in progress. The offset derives from wall millis
/// (jitter-proof); `done` latches once the text has fully exited the
/// last-measured slot, and the pass never replays for the same key.
struct MarqueeAnim {
    /// The LWW stamp of the message being scrolled — the identity that
    /// decides "new pass" vs "same pass".
    key: SharedTimestamp,
    text: String,
    /// Display width of `text`, cells (fixed; computed once).
    text_width: usize,
    /// Wall millis when this pass started.
    started_at_millis: u64,
    /// The last computed cell offset (draws render this).
    offset: usize,
    /// The middle slot's width from the last draw; `None` until a draw
    /// has measured it. The done-check waits for a real measurement
    /// rather than guess — but a *measured* zero (a narrow terminal
    /// while a file plays) terminates the pass at once, or the 10 Hz
    /// animation tick would pin the shell forever.
    slot_width: Option<usize>,
    /// The pass has fully exited; the slot reverts to the suggestion.
    done: bool,
}

/// The whole TUI.
pub struct Ui {
    me: UserId,
    chat: ChatPane,
    series: SeriesPane,
    users: UsersPane,
    playlist: PlaylistPane,
    status: StatusBar,
    /// The borderless connection-health row under the playlist.
    health: HealthLine,
    keybar: KeyBar,
    modals: Vec<Modal>,
    logging: Option<crate::logging::LiveLogging>,
    focus: Focus,
    /// Where the panes landed in the last draw (mouse hit-testing).
    panes: PaneRects,
    /// A pane splitter being dragged, with the layout as it was at the
    /// press — released unchanged means nothing to persist. Like the
    /// chat's selection drag, drag/release events route here by grab,
    /// not position.
    splitter_drag: Option<(Splitter, PaneLayout)>,
    subtitle_mode: SubtitleMode,
    /// Terminal color capability, detected by the production shell and
    /// injected by rendering tests. Limited is the deterministic default.
    color_depth: ColorDepth,
    /// Rolling log of the local player's subtitle lines (with in-video
    /// and arrival timestamps). Local only — never synced.
    subtitles: std::collections::VecDeque<SubtitleEntry>,
    /// How many entries back from the newest the separate subtitle
    /// pane is scrolled (0 = live). Mouse-only: the pane is not
    /// focusable. Clamped at render, reset whenever the pane is hidden.
    subtitle_scroll: usize,
    /// Named speakers active within the last five wall-clock minutes.
    speaker_colors: SpeakerColors,
    /// The scrolling state of the synced marquee line, keyed by its LWW
    /// stamp: one full off-screen-right to off-screen-left pass per
    /// update, never restarted for the same key (design.md, AI
    /// Commentary).
    marquee: Option<MarqueeAnim>,
    /// Shared-clock millis of the first applied snapshot — this
    /// session's birth. A marquee stamped before it is a previous
    /// session's leftover (the register persists in synced state until
    /// compaction) and is adopted already-done instead of played.
    startup_shared_millis: Option<u64>,
    /// The local animator clock: **monotonic** millis fed exclusively
    /// by the shell's ticks (`Instant`-derived, never wall time).
    /// Every consumer measures elapsed time between local events —
    /// marquee frames, the spoiler tease and double-click window, the
    /// selection TTL — so the absolute value is meaningless and wall or
    /// shared stamps must never be merged in: both of those domains can
    /// step backward (a local NTP correction; a ClockSync shrinking the
    /// offset behind `shared_now`), and the old max-merge latched at
    /// the historic maximum, freezing every animator for the size of
    /// the step (2026-08-20 review). `Ui` itself never touches a system
    /// clock, so tests stay deterministic.
    clock: u64,
    /// In-flight playlist-add hashes: (filename, done, total). Drawn as
    /// a progress overlay while non-empty (the no-silent-work rule).
    hashing: Vec<(String, u64, u64)>,
    /// Pending user-selected torrent imports, local until hashing discovers
    /// their playlist identity.
    nyaa_imports: BTreeMap<crate::torrent::engine::TorrentImportId, NyaaActiveImport>,
    next_nyaa_import_id: u64,
    /// Local-only system chat lines (archive results, etc.), merged into
    /// the chat log by timestamp. Never synced.
    system_log: Vec<props::ChatLine>,
    /// Local-only chat lines from external IRC users (the IRC bridge),
    /// merged into the chat log by timestamp. Never synced — each client
    /// runs its own bridge.
    irc_log: Vec<props::ChatLine>,
    snapshot: UiSnapshot,
    /// Memoizes the franchise grouping so it is not rebuilt on every 10Hz
    /// playback-position snapshot (was ~⅓ of normal-play CPU). Reachable
    /// only via `get`, so it can never be read without its freshness check.
    franchise_cache: franchise::FranchiseCache,
    /// The same memoization for The List's grouping — the Series pane's
    /// *default* mode, O(held files × entries) computed fresh.
    list_groups_cache: props::ListGroupsCache,
    settings: Settings,
    media_roots: Vec<PathBuf>,
    /// True when `me` is a runtime `--username` override that differs from
    /// the persisted `settings.username` (set at construction). While set,
    /// a settings save must NOT move `self.me` onto the settings-screen
    /// value — the override stays the identity, and the stored name is left
    /// untouched (run.rs keeps it out of the persisted settings). Without
    /// this, opening F3 and saving in an overridden session would key our
    /// own writes under the stored name and our readiness would stop
    /// showing (the 2026-06-14 identity-agreement invariant).
    identity_locked: bool,
}

impl Ui {
    /// Supply a logging controller (production defaults to the process runtime).
    pub fn set_logging(&mut self, logging: crate::logging::LiveLogging) {
        self.logging = Some(logging);
    }

    /// Build the UI. Opens the settings modal when the *given* settings
    /// need setup. Callers that prefill values (the `$USER` username,
    /// the `.env` password) but still want first-run confirmation use
    /// [`Ui::with_setup`].
    pub fn new(me: UserId, settings: Settings, media_roots: Vec<PathBuf>) -> Self {
        let open_settings = settings.needs_setup() || media_roots.is_empty();
        Self::with_setup(me, settings, media_roots, open_settings)
    }

    /// Build the UI, opening the settings modal iff `open_settings`
    /// (prefilled values appear as editable defaults).
    pub fn with_setup(
        me: UserId,
        settings: Settings,
        media_roots: Vec<PathBuf>,
        open_settings: bool,
    ) -> Self {
        // Identity is locked when a runtime override (a `--username` flag)
        // gives us a `me` that differs from the persisted username; a
        // settings save then must not move the identity (see the field doc
        // and run.rs's matching `identity_locked`).
        let identity_locked = settings
            .username
            .as_deref()
            .is_some_and(|stored| me.0 != stored);
        let mut ui = Self {
            me,
            chat: ChatPane::default(),
            series: SeriesPane::default(),
            users: UsersPane::default(),
            playlist: PlaylistPane::default(),
            status: StatusBar::default(),
            health: HealthLine::default(),
            keybar: KeyBar::default(),
            modals: Vec::new(),
            logging: crate::logging::runtime(),
            focus: Focus::Chat,
            panes: PaneRects::default(),
            splitter_drag: None,
            subtitle_mode: settings.subtitle_mode,
            color_depth: ColorDepth::Limited,
            subtitles: std::collections::VecDeque::new(),
            subtitle_scroll: 0,
            speaker_colors: SpeakerColors::default(),
            marquee: None,
            startup_shared_millis: None,
            clock: 0,
            hashing: Vec::new(),
            nyaa_imports: BTreeMap::new(),
            next_nyaa_import_id: 1,
            system_log: Vec::new(),
            irc_log: Vec::new(),
            snapshot: UiSnapshot::default(),
            franchise_cache: franchise::FranchiseCache::default(),
            list_groups_cache: props::ListGroupsCache::default(),
            settings: settings.clone(),
            media_roots: media_roots.clone(),
            identity_locked,
        };
        ui.chat.set_me(ui.me.to_string());
        ui.series.set_sort(settings.series_sort);
        ui.series.set_list_sort(settings.list_sort);
        if open_settings {
            ui.push_modal(Modal::Settings(SettingsModal::new(settings, media_roots)));
        }
        ui.sync_focus_attr();
        ui.refresh_keybar();
        ui
    }

    /// Set the terminal color capability before the first draw. Production
    /// calls this once during terminal setup; tests use it as an injection
    /// seam for true-color rendering.
    pub fn set_color_depth(&mut self, color_depth: ColorDepth) {
        self.color_depth = color_depth;
    }

    /// Advance every animator (marquee pass, spoiler tease, selection
    /// TTL) to the given **monotonic** millis, independently of session
    /// traffic. Returns whether a redraw could change what's on screen.
    ///
    /// The shell calls this with `Instant`-derived millis before
    /// dispatching every input and on every idle tick; animation starts
    /// and TTL stamps read the resulting `Ui::clock`, so stamps and
    /// frames share one domain that cannot step backward. The `max` is
    /// only a guard against out-of-order synthetic inputs in tests.
    /// Speaker-color leases live in the shared-clock domain of subtitle
    /// arrival stamps, so a tick advances them by the locally *elapsed*
    /// millis (domain-free) rather than the absolute value; arrivals
    /// and snapshots provide their absolute anchors.
    pub fn advance_clock(&mut self, now_millis: u64) -> bool {
        let elapsed = now_millis.saturating_sub(self.clock);
        self.clock = self.clock.max(now_millis);
        let now = self.clock;
        let speakers = self.speaker_colors.tick(elapsed);
        let marquee = self.advance_marquee(now);
        let spoilers = self.chat.advance_spoilers(now);
        let selection = self.chat.expire_selection(now);
        let logs = matches!(self.modals.last(), Some(Modal::Logs(modal)) if modal.refresh_needed());
        speakers || marquee || spoilers || selection || logs
    }

    /// How soon the shell should tick again: fast while a marquee pass
    /// or a spoiler re-randomization tease is animating, the lazy 1s
    /// otherwise. The idle discipline is preserved either way — a tick
    /// only repaints when [`Self::advance_clock`] reports a change.
    pub(crate) fn next_tick_hint(&self) -> std::time::Duration {
        let marquee_live = matches!(&self.marquee, Some(anim) if !anim.done);
        if marquee_live || self.chat.spoiler_animating() {
            std::time::Duration::from_millis(100)
        } else {
            std::time::Duration::from_secs(1)
        }
    }

    /// Recompute the marquee offset from wall time; returns whether
    /// anything changed. Done latches when the text has fully exited the
    /// slot — but only against a *measured* slot width (a pass that has
    /// never been drawn keeps animating conservatively; the first draw
    /// measures). A measured width of **zero** is a real measurement,
    /// not "not yet measured": the pass ends immediately, since no cell
    /// of it can ever be shown.
    fn advance_marquee(&mut self, now_millis: u64) -> bool {
        let Some(anim) = &mut self.marquee else {
            return false;
        };
        if anim.done {
            return false;
        }
        let elapsed = now_millis.saturating_sub(anim.started_at_millis);
        let offset = (elapsed * props::MARQUEE_CELLS_PER_SEC / 1000) as usize;
        let moved = offset != anim.offset;
        anim.offset = offset;
        if let Some(width) = anim.slot_width
            && (width == 0 || offset >= width + anim.text_width)
        {
            anim.done = true;
            return true;
        }
        moved
    }

    /// Append a subtitle line to the rolling log (empty lines are
    /// clears — the previous cue just stopped displaying; skip them).
    ///
    /// The on-screen cue-set evolves over time, and mpv re-emits the whole
    /// (joined) value on every change, so consecutive pushes are often the
    /// *same* utterance growing or shrinking rather than a new line:
    ///
    /// - **Incremental reveal**: some ASS subs reveal a line letter-by-letter
    ///   as rapid-fire cues, each a longer prefix of the last.
    /// - **Overlapping cues**: when two ASS events display simultaneously mpv
    ///   joins them (`parse_ass_full` separates with a space); as one ends the
    ///   combined text shrinks back. The disappearing event can sit at either
    ///   end of the join (mpv's order is not fixed), so the shrink leaves a
    ///   prefix *or* a suffix of what was shown.
    ///
    /// We collapse any such prefix/suffix relationship between the previous
    /// and the new text into one log entry: a growth replaces it in place
    /// (keeping the original cue's timestamps and tracking the latest
    /// speaker); a shrink-back is dropped as a redundant re-show — otherwise
    /// it reads as a duplicate (SL2_Episode-141 at ~03:19, where "…glory."
    /// re-appeared after the overlapping "Coming!" cleared). An exact repeat
    /// is the degenerate case and collapses too. Only strictly-distinct text,
    /// with no containment either way, starts a new line.
    ///
    /// A multi-line cue arrives newline-separated; we render one line, so
    /// newlines become spaces (otherwise the join reads as "youdemons"
    /// instead of "you demons"). Normalizing also keeps the prefix/suffix
    /// test working as a two-line cue grows past its first line.
    pub fn push_subtitle(
        &mut self,
        video_millis: u64,
        arrival_millis: u64,
        text: String,
        speaker: Option<SpeakerName>,
    ) {
        let text = text.replace('\r', "").replace('\n', " ");
        if text.is_empty() {
            self.speaker_colors.advance(arrival_millis);
            return;
        }
        // Observe every incoming cue before collapse: a contained overlap
        // can be dropped from the log while its named speaker still counts
        // toward the five-minute active set.
        let speaker_slot = self
            .speaker_colors
            .observe(speaker.as_ref(), arrival_millis);
        // Classify the new text against the last entry (the shared
        // reveal/overlap collapse — `props::subtitle_collapse`, also
        // used by the advisor's context ring).
        let collapse =
            props::subtitle_collapse(self.subtitles.back().map(|last| last.text.as_str()), &text);
        match collapse {
            props::SubtitleCollapse::Extends => {
                if let Some(last) = self.subtitles.back_mut() {
                    // Same cue, fuller now: keep the original timestamps,
                    // track the latest text and speaker.
                    last.text = text;
                    last.speaker = speaker;
                    last.speaker_slot = speaker_slot;
                }
            }
            props::SubtitleCollapse::Contained => {
                // Same cue receding (an overlapping neighbour ended); the
                // fuller text is already logged — drop the redundant
                // re-show.
            }
            props::SubtitleCollapse::Distinct => {
                self.subtitles.push_back(SubtitleEntry {
                    video_millis,
                    arrival_millis,
                    text,
                    speaker,
                    speaker_slot,
                });
                while self.subtitles.len() > 100 {
                    self.subtitles.pop_front();
                }
            }
        }
        // In Intermixed mode subtitles live in the chat log, so a new
        // line must rebuild it (the separate pane re-reads every draw).
        if self.subtitle_mode == SubtitleMode::Intermixed {
            self.refresh_chat();
        }
    }

    /// Append a local system chat line (e.g. an archive result) and
    /// refresh the chat pane. Not synced — local to this client.
    pub fn push_system(&mut self, timestamp: u64, text: String) {
        self.log_system_line(timestamp, text);
        self.refresh_chat();
    }

    /// Append to the system log without rebuilding the chat pane — for
    /// callers that rebuild it themselves right after (apply_snapshot).
    fn log_system_line(&mut self, timestamp: u64, text: String) {
        self.system_log.push(props::system_line(timestamp, text));
        while self.system_log.len() > 100 {
            self.system_log.remove(0);
        }
    }

    /// Append a local chat line from an external IRC user and refresh the
    /// chat pane. Rendered like normal chat (colored nick, mention
    /// highlight) but with a dim `irc` tag, and never synced — each
    /// client's bridge surfaces these independently.
    pub fn push_irc(&mut self, timestamp: u64, sender: String, text: String, action: bool) {
        self.irc_log
            .push(props::irc_line(timestamp, sender, text, action));
        while self.irc_log.len() > 100 {
            self.irc_log.remove(0);
        }
        self.refresh_chat();
    }

    /// Mouse wheel over the separate subtitle pane: scroll back through
    /// the log (up = older). Render clamps to the oldest entry, so
    /// over-scrolling is safe.
    fn scroll_subtitles(&mut self, up: bool) {
        const STEP: usize = 3;
        if up {
            self.subtitle_scroll += STEP;
        } else {
            self.subtitle_scroll = self.subtitle_scroll.saturating_sub(STEP);
        }
        tracing::debug!(offset = self.subtitle_scroll, "subtitle pane scrolled");
    }

    /// Cycle the subtitle mode (Off -> Intermixed -> Separate -> Off),
    /// persist it (user chose F2-persistence), and rebuild chat so
    /// Intermixed lines appear/disappear at once. Returns the persist
    /// action for the caller to forward.
    fn cycle_subtitle_mode(&mut self) -> UserAction {
        self.subtitle_mode = self.subtitle_mode.next();
        self.settings.subtitle_mode = self.subtitle_mode;
        tracing::debug!(mode = ?self.subtitle_mode, "subtitle mode cycled");
        self.refresh_chat();
        UserAction::SaveSettings(Box::new(self.settings.clone()), self.media_roots.clone())
    }

    /// Rebuild the chat pane from the current snapshot, system log, and
    /// (in Intermixed mode) subtitle log.
    fn refresh_chat(&mut self) {
        let chat = self.merged_chat(&self.snapshot.view);
        self.chat.set_lines(chat);
    }

    /// The synced chat log merged with local system lines — and, in
    /// Intermixed mode, subtitle lines — ordered by shared-clock millis.
    /// Stable sort: at an equal millis, synced messages sort before
    /// system/subtitle lines (which are pushed afterward).
    fn merged_chat(&self, view: &StateView) -> Vec<props::ChatLine> {
        let mut lines = props::chat_lines(view);
        lines.extend(self.system_log.iter().cloned());
        lines.extend(self.irc_log.iter().cloned());
        if self.subtitle_mode == SubtitleMode::Intermixed {
            lines.extend(self.subtitles.iter().map(|s| {
                props::subtitle_line(
                    s.video_millis,
                    s.arrival_millis,
                    s.text.clone(),
                    s.speaker.as_ref(),
                    self.settings.subtitle_speaker_names,
                )
            }));
        }
        lines.sort_by_key(|line| line.millis);
        Self::insert_day_separators(lines)
    }

    /// Insert a biblical-day separator (09:00 boundary) between adjacent
    /// lines whose day differs. A render-time view concern: computed from
    /// the (already sorted) line timestamps, never stored or synced, so it
    /// is recomputed every draw and is visible to late joiners too.
    fn insert_day_separators(lines: Vec<props::ChatLine>) -> Vec<props::ChatLine> {
        let mut out = Vec::with_capacity(lines.len());
        let mut prev_day = None;
        for line in lines {
            let day = props::biblical_date(line.millis);
            if let (Some(today), Some(prev)) = (day, prev_day)
                && today != prev
            {
                out.push(props::day_separator(line.millis));
            }
            if day.is_some() {
                prev_day = day;
            }
            out.push(line);
        }
        out
    }

    /// Track playlist-add hashing progress (drawn as an overlay).
    pub fn set_hash_progress(&mut self, filename: String, done: u64, total: u64, finished: bool) {
        if finished {
            self.hashing.retain(|(name, _, _)| *name != filename);
            return;
        }
        match self
            .hashing
            .iter_mut()
            .find(|(name, _, _)| *name == filename)
        {
            Some(row) => {
                row.1 = done;
                row.2 = total;
            }
            None => self.hashing.push((filename, done, total)),
        }
    }

    /// Deliver AniDB search results to the search modal, if it's open
    /// (stale results for a superseded query are dropped by the modal).
    ///
    /// A real "zero matches" answer marks the entry `anidb_unavailable`
    /// (design.md, Series Identity: distinguishes "confirmed not on
    /// AniDB" from "nobody's checked yet"); a real answer *with* hits
    /// clears a stale marker, even without linking -- a better query
    /// proved AniDB does know the show after all. No-op if the flag
    /// already matches (nothing to write).
    pub fn set_search_results(
        &mut self,
        query: &str,
        results: Vec<dessplay_core::net::AniDbSearchHit>,
    ) -> Vec<UserAction> {
        let Some(Modal::AniDbSearch(modal)) = self.modals.last_mut() else {
            return Vec::new();
        };
        modal.set_results(query, results);
        let id = modal.id;
        let unavailable = if modal.search_answered_empty() {
            true
        } else if modal.search_answered_with_hits() {
            false
        } else {
            return Vec::new(); // still in flight, or a stale reply was dropped
        };
        let Some(entry) = self.snapshot.view.list_entries.get(&id) else {
            return Vec::new();
        };
        if entry.anidb_unavailable == unavailable {
            return Vec::new();
        }
        let mut entry = entry.clone();
        entry.anidb_unavailable = unavailable;
        vec![UserAction::Mutate(Mutation::PutListEntry { id, entry })]
    }

    /// Deliver a Nyaa browse answer to the open modal. The modal rejects
    /// stale queries and answers that arrive after it switched to active
    /// imports.
    pub fn set_nyaa_results(
        &mut self,
        query: &str,
        result: Result<Vec<crate::torrent::nyaa::NyaaBrowseResult>, String>,
    ) {
        if let Some(Modal::NyaaSearch(modal)) = self.modals.last_mut() {
            modal.set_results(query, result);
        }
    }

    /// Update the local pending-import model used by both the progress
    /// overlay and the Nyaa modal's active list.
    pub fn set_nyaa_import_progress(
        &mut self,
        id: crate::torrent::engine::TorrentImportId,
        filename: String,
        stage: crate::actors::file::NyaaImportStage,
        done_bytes: u64,
        total_bytes: u64,
    ) {
        self.nyaa_imports.insert(
            id,
            NyaaActiveImport {
                id,
                filename,
                stage,
                done_bytes,
                total_bytes,
            },
        );
        self.refresh_nyaa_modal_active();
    }

    /// Remove a completed, failed, or cancelled pending import.
    pub fn finish_nyaa_import(&mut self, id: crate::torrent::engine::TorrentImportId) {
        self.nyaa_imports.remove(&id);
        self.refresh_nyaa_modal_active();
    }

    fn refresh_nyaa_modal_active(&mut self) {
        if let Some(Modal::NyaaSearch(modal)) = self.modals.last_mut() {
            modal.set_active(self.nyaa_imports.values().cloned().collect());
        }
    }

    /// Open a file browser: the main loop's answer to
    /// [`UserAction::Browse`], carrying the library index (every indexed
    /// `(path, hash)` under a media root), the personally-watched hashes,
    /// and — for the mapping browser — the series' last-used directory.
    /// Group-watched flags are unioned in here (they live in the synced
    /// view, which the main loop doesn't re-read for this).
    pub fn open_file_browser(
        &mut self,
        request: BrowseRequest,
        files: Vec<(PathBuf, Ed2kHash, i64)>,
        mut watched: BTreeSet<Ed2kHash>,
        start: Option<PathBuf>,
    ) {
        watched.extend(
            self.snapshot
                .view
                .watched
                .iter()
                .filter(|(_, flag)| **flag)
                .map(|(hash, _)| *hash),
        );
        let library = BrowserLibrary::new(&self.media_roots, files, watched);
        let mut browser = match request {
            BrowseRequest::Add { after } => {
                FileBrowser::for_file(self.media_roots.clone(), after, library)
            }
            BrowseRequest::Map { file, target, .. } => {
                FileBrowser::for_mapping(self.media_roots.clone(), file, target, start, library)
            }
        };
        browser.set_sort(self.settings.file_browser_sort);
        // A double-press races two requests; the second answer replaces
        // the first browser instead of stacking on it.
        if matches!(self.modals.last(), Some(Modal::Files(_))) {
            self.pop_modal();
        }
        self.push_modal(Modal::Files(browser));
    }

    /// Open the local-copy offer modal (proposal
    /// 2026-08-31-local-copy-offer): the main loop found plausible local
    /// copies for a missing now-playing file. Pushed on top of whatever
    /// is open — the group may be waiting on this user, so the offer must
    /// not sit hidden behind e.g. the settings screen. A duplicate for a
    /// file already offered anywhere on the stack is dropped.
    pub fn offer_local_copies(
        &mut self,
        file: Ed2kHash,
        filename: String,
        candidates: Vec<dessplay_core::local_copy::CopyCandidate>,
    ) {
        if self
            .modals
            .iter()
            .any(|modal| matches!(modal, Modal::LocalCopyOffer(offer) if offer.file == file))
        {
            return;
        }
        self.push_modal(Modal::LocalCopyOffer(LocalCopyOfferModal::new(
            file, filename, candidates,
        )));
        self.sync_focus_attr();
    }

    /// Push the "What's new" changelog modal (design.md, Changelog).
    /// Called by run.rs between construction and the UI thread's start,
    /// when unseen entries exist and this is not the first run; `marker`
    /// records the **whole** embedded changelog as seen and is persisted
    /// when the user dismisses the modal.
    pub fn show_changelog(
        &mut self,
        days: Vec<crate::changelog::ChangelogDay>,
        marker: crate::changelog::SeenMarker,
    ) {
        if days.is_empty() {
            return;
        }
        self.push_modal(Modal::Changelog(ChangelogModal::new(days, marker, true)));
        self.sync_focus_attr();
        self.refresh_keybar();
    }

    /// Open the full changelog viewer (`/changelog`).
    fn open_changelog(&mut self) -> Vec<UserAction> {
        let days = crate::changelog::entries();
        let Some(marker) = crate::changelog::latest_marker(days) else {
            // Unreachable in a build that passed the test suite (the
            // embedded changelog is validated non-empty), but degrade
            // honestly rather than open an empty modal.
            return vec![UserAction::Notice("/changelog: no entries".to_string())];
        };
        self.push_modal(Modal::Changelog(ChangelogModal::new(
            days.to_vec(),
            marker,
            false,
        )));
        self.sync_focus_attr();
        Vec::new()
    }

    /// Replace the snapshot and recompute every pane's props.
    pub fn apply_snapshot(&mut self, snapshot: UiSnapshot) {
        // `Ui::clock` is deliberately NOT advanced here: the shell
        // freshens it (monotonic millis) before dispatching every
        // input, this snapshot included. The snapshot's stamps live in
        // rewindable domains — wall time steps on NTP corrections,
        // `shared_now`'s offset shrinks on a later ClockSync — and the
        // old max-merge latched the animator clock at the historic
        // maximum, freezing every animator for the size of a backward
        // step (2026-08-20 review). Wall/shared time is for display and
        // message identity only. Speaker-color leases are the one
        // shared-domain clock consumer: their stamps are shared-clock
        // subtitle arrivals, so the window advances on `shared_now`.
        self.speaker_colors.advance(snapshot.shared_now);
        // Marquee lifecycle: a new LWW stamp starts a fresh pass (even
        // for identical text — a rewrite replays); the same stamp never
        // restarts, including after `done`; a cleared register drops the
        // animation. Snapshots arrive frequently during playback, so
        // they advance the offset too. A stamp from before this
        // session's first snapshot is a previous session's leftover and
        // is adopted with `done` latched, so it never plays. The
        // marquee-mode setting chooses the presentation: Chat folds the
        // update into the chat log instead of scrolling (which is why
        // this runs before the chat merge below), Off shows nothing —
        // both still adopt the stamp, so flipping back to Marquee never
        // replays an old message.
        // Seeded from the snapshot's *shared* clock — the stamp it
        // gates lives in that domain (2026-08-12 review: seeding from
        // the wall-clock `now` made a leading clock suppress fresh
        // comments and a lagging one replay stale ones).
        let startup = *self
            .startup_shared_millis
            .get_or_insert(snapshot.shared_now);
        match &snapshot.view.marquee {
            Some((stamp, message)) => {
                if self.marquee.as_ref().map(|anim| anim.key) != Some(*stamp) {
                    use unicode_width::UnicodeWidthStr;
                    let stale = stamp.as_millis() < startup;
                    let mode = self.settings.marquee_mode;
                    self.marquee = Some(MarqueeAnim {
                        key: *stamp,
                        text_width: message.text.width(),
                        text: message.text.clone(),
                        // The animator clock, not snapshot.now: the
                        // pass must start at elapsed 0 in the domain
                        // that advances it, or a wall-ahead clock skips
                        // its entry.
                        started_at_millis: self.clock,
                        offset: 0,
                        slot_width: None,
                        done: stale || mode != MarqueeMode::Marquee,
                    });
                    if !stale && mode == MarqueeMode::Chat {
                        self.log_system_line(stamp.as_millis(), message.text.clone());
                    }
                }
            }
            None => self.marquee = None,
        }
        self.advance_marquee(self.clock);
        // Snapshots arrive at ~10Hz during playback, so they advance a
        // running spoiler tease too (same policy as the marquee above).
        self.chat.advance_spoilers(self.clock);
        let chat = self.merged_chat(&snapshot.view);
        self.chat.set_lines(chat);
        self.chat
            .set_usernames(props::chat_usernames(&snapshot.peers));
        self.users.set_props(props::users_props(
            &snapshot.view,
            &snapshot.peers,
            &snapshot.known_offline,
            snapshot.now,
        ));
        self.playlist.set_props(props::playlist_props(
            &snapshot.view,
            &self.me,
            &snapshot.cache_hashes,
        ));
        self.status.set_props(props::status_props(
            &snapshot.view,
            &snapshot.peers,
            &self.me,
            snapshot.link,
        ));
        self.health.set_props(snapshot.health.clone());
        self.snapshot = snapshot;
        self.refresh_series();
        // Open modals render derived state too: refresh episode
        // browsers (anywhere in the stack) in place, so `w`'s
        // round-trip — and other clients' watched toggles and download
        // completions — shows immediately instead of on reopen.
        for modal in &mut self.modals {
            if let Modal::Episodes(browser) = modal {
                browser.refresh(&self.snapshot.view, &self.snapshot.personal_watched);
            }
        }
    }

    fn refresh_series(&mut self) {
        let recency = match self.series.mode() {
            SeriesMode::Recent => Some(&self.snapshot.recency),
            SeriesMode::All => None,
            SeriesMode::TheList => {
                // Per-user Watching groups draw from everyone the client
                // can name: connected peers plus the known-offline roster
                // (design.md #15) — a committed user's group must not
                // vanish with their connection.
                let users: Vec<UserId> = self
                    .snapshot
                    .peers
                    .iter()
                    .map(|peer| peer.username.clone())
                    .chain(
                        self.snapshot
                            .known_offline
                            .iter()
                            .map(|user| user.username.clone()),
                    )
                    .collect();
                // Through the cache: position ticks (and anything else
                // that leaves the derivation's inputs alone) reuse the
                // grouping, and `set_groups` skips identical groups
                // outright.
                let groups = self.list_groups_cache.get(
                    &self.snapshot.view,
                    &self.me,
                    &users,
                    self.series.list_sort(),
                    &self.snapshot.recency,
                    &self.snapshot.personal_watched,
                );
                self.series.set_groups(groups);
                return;
            }
        };
        // The grouping comes through the cache, which recomputes only when
        // the metadata/relations maps change -- not on every position tick.
        let franchises = self.franchise_cache.get(&self.snapshot.view);
        let rows = props::franchise_rows_from(
            franchises,
            self.series.sort(),
            recency,
            &self.series.filter(),
        );
        self.series.set_franchises(rows);
    }

    fn sync_focus_attr(&mut self) {
        use tuirealm::component::Component;
        for (pane, focused) in [
            (
                &mut self.chat as &mut dyn Component,
                self.focus == Focus::Chat,
            ),
            (
                &mut self.series as &mut dyn Component,
                self.focus == Focus::Series,
            ),
            (
                &mut self.users as &mut dyn Component,
                self.focus == Focus::Users,
            ),
            (
                &mut self.playlist as &mut dyn Component,
                self.focus == Focus::Playlist,
            ),
        ] {
            pane.attr(
                Attribute::Focus,
                AttrValue::Flag(focused && self.modals.is_empty()),
            );
        }
    }

    fn refresh_keybar(&mut self) {
        let mut items: Vec<(&'static str, &'static str)> = match self.modals.last() {
            Some(modal) => modal.keybindings(),
            None => {
                let mut items = match self.focus {
                    Focus::Chat => self.chat.keybindings(),
                    Focus::Series => self.series.keybindings(),
                    Focus::Users => self.users.keybindings(),
                    Focus::Playlist => self.playlist.keybindings(),
                };
                items.insert(0, ("Tab", "Next pane"));
                // F2 cycles subtitle mode; F3 opens settings. Only shown
                // when no modal is up.
                items.push((
                    "F2",
                    match self.subtitle_mode {
                        SubtitleMode::Off => "Subs: off",
                        SubtitleMode::Intermixed => "Subs: intermixed",
                        SubtitleMode::SeparatePane => "Subs: separate",
                    },
                ));
                items.push(("F3", "Settings"));
                items
            }
        };
        // Globals, always available (handled before pane/modal routing).
        items.push(("F11", "Logs"));
        items.push(("Ctrl-r", "Ready"));
        items.push(("Ctrl-c", "Quit"));
        self.keybar.set_items(items);
    }

    fn push_modal(&mut self, modal: Modal) {
        tracing::debug!(modal = modal.name(), "modal opened");
        self.modals.push(modal);
    }

    /// Open the settings modal from the UI's current settings + roots.
    /// Reachable any time via F3 / `/settings` (the first-run path opens
    /// it through [`Ui::with_setup`] instead).
    fn open_settings(&mut self) {
        self.push_modal(Modal::Settings(SettingsModal::new(
            self.settings.clone(),
            self.media_roots.clone(),
        )));
        self.sync_focus_attr();
    }

    fn pop_modal(&mut self) {
        if let Some(modal) = self.modals.pop() {
            tracing::debug!(modal = modal.name(), "modal closed");
        }
    }

    /// Route a mouse event (design.md, Mouse support): a left-click
    /// focuses the pane under the pointer and moves the pane's selection
    /// to the clicked row; the wheel scrolls the pane under the pointer
    /// only when it is *already focused* — touchpads emit wheel events by
    /// accident, and an unfocused pane scrolling invisibly (or stealing
    /// focus) would turn each graze into a surprise. Ignored while a
    /// modal is open — modals capture all input and none of them speak
    /// mouse yet — and before the first draw (the stored rects are
    /// zero-sized, so every hit-test misses).
    fn handle_mouse(&mut self, mouse: MouseEvent) -> Vec<UserAction> {
        if !self.modals.is_empty() {
            return Vec::new();
        }
        let position = Position::new(mouse.column, mouse.row);
        // Drag/release route to whichever drag is in progress — a
        // splitter resize or a chat selection — wherever the pointer
        // is: a grab, so leaving the pane mid-drag keeps going
        // (clamped) instead of going dead.
        match mouse.kind {
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((splitter, _)) = self.splitter_drag {
                    self.settings.pane_layout =
                        self.panes
                            .layout_for_drag(splitter, position, self.settings.pane_layout);
                } else if self.chat.dragging() {
                    self.chat.mouse_drag(mouse.column, mouse.row);
                }
                return Vec::new();
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some((splitter, at_press)) = self.splitter_drag.take() {
                    if self.settings.pane_layout == at_press {
                        return Vec::new();
                    }
                    tracing::info!(?splitter, layout = %self.settings.pane_layout.as_string(), "panes resized");
                    return vec![UserAction::SaveSettings(
                        Box::new(self.settings.clone()),
                        self.media_roots.clone(),
                    )];
                }
                if let Some(text) = self.chat.mouse_up(self.clock) {
                    tracing::info!(chars = text.chars().count(), "chat selection copied");
                    return vec![UserAction::CopyToClipboard(text)];
                }
                return Vec::new();
            }
            _ => {}
        }
        // Any fresh press dismisses a held selection highlight, even
        // when it lands outside the chat (same rule as unrelated keys).
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            self.chat.clear_selection();
            // A press on a pane boundary arms a resize drag instead of
            // a click: the border cells are misses for every pane's
            // own click handling anyway, so nothing is shadowed.
            if let Some(splitter) = self.panes.splitter_at(position) {
                tracing::debug!(
                    ?splitter,
                    column = mouse.column,
                    row = mouse.row,
                    "splitter grabbed"
                );
                self.splitter_drag = Some((splitter, self.settings.pane_layout));
                return Vec::new();
            }
        }
        // The separate subtitle pane is not focusable, so the wheel
        // works over it regardless of focus (its scroll is visible, so
        // the accidental-graze objection doesn't apply). Checked before
        // the chat column, which it overlaps.
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) && self.panes.subs.contains(position)
        {
            self.scroll_subtitles(mouse.kind == MouseEventKind::ScrollUp);
            return Vec::new();
        }
        let target = [
            (self.panes.chat, Focus::Chat),
            (self.panes.series, Focus::Series),
            (self.panes.users, Focus::Users),
            (self.panes.playlist, Focus::Playlist),
        ]
        .into_iter()
        .find_map(|(rect, focus)| rect.contains(position).then_some(focus));
        let Some(target) = target else {
            return Vec::new(); // status bar, keybar, or off-layout
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                tracing::debug!(?target, column = mouse.column, row = mouse.row, "click");
                // The panes hit-test against the viewport they recorded
                // at render time, so the click lands on the row the
                // user saw regardless of focus or centering policy.
                match target {
                    // Chat has no row selection; a click there drives the
                    // spoiler reveal state machine (and focuses, below) —
                    // and arms a potential selection drag.
                    Focus::Chat => {
                        self.chat.click(mouse.column, mouse.row, self.clock);
                        self.chat.mouse_down(mouse.column, mouse.row);
                    }
                    Focus::Series => self.series.click(mouse.column, mouse.row),
                    Focus::Users => self.users.click(mouse.column, mouse.row),
                    Focus::Playlist => self.playlist.click(mouse.column, mouse.row),
                }
                if self.focus != target {
                    self.focus = target;
                    tracing::debug!(focus = ?self.focus, "focus changed (click)");
                }
                self.sync_focus_attr();
                self.refresh_keybar();
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                if target != self.focus {
                    return Vec::new();
                }
                let up = mouse.kind == MouseEventKind::ScrollUp;
                match target {
                    Focus::Chat => self.chat.scroll_wheel(up),
                    Focus::Series => self.series.scroll_wheel(up),
                    Focus::Users => self.users.scroll_wheel(up),
                    Focus::Playlist => self.playlist.scroll_wheel(up),
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// Route one input event; returns the actions it produced.
    pub fn handle(&mut self, ev: Event<NoUserEvent>) -> Vec<UserAction> {
        // SECURITY: never log keystroke contents while the settings
        // modal is open — the user may be typing the password.
        if matches!(self.modals.last(), Some(Modal::Settings(_))) {
            tracing::trace!("ui event (contents redacted: settings modal open)");
        } else {
            tracing::trace!(event = ?ev, "ui event");
        }
        // Mouse events route by position, not focus.
        if let Event::Mouse(mouse) = &ev {
            return self.handle_mouse(*mouse);
        }
        // A held chat drag-selection owns Shift-Up/Down (extend by one
        // whole line, re-copy); any other key dismisses the highlight
        // and then does its normal job. Deliberately no copy binding —
        // the release already copied.
        if let Event::Keyboard(KeyEvent { code, modifiers }) = &ev {
            let extend = match code {
                Key::Up if *modifiers == KeyModifiers::SHIFT => Some(true),
                Key::Down if *modifiers == KeyModifiers::SHIFT => Some(false),
                _ => None,
            };
            match extend {
                Some(up) if self.chat.selection_held() => {
                    tracing::debug!(up, "user action: extend chat selection");
                    return match self.chat.extend_selection(up, self.clock) {
                        Some(text) => vec![UserAction::CopyToClipboard(text)],
                        None => Vec::new(),
                    };
                }
                _ => self.chat.clear_selection(),
            }
        }
        // F11 also works over another modal, restoring it on close.
        if super::components::plain(&ev) == Some(Key::Function(11)) {
            tracing::debug!("user action: toggle logs (F11)");
            if let Some(index) = self
                .modals
                .iter()
                .position(|modal| matches!(modal, Modal::Logs(_)))
            {
                self.modals.remove(index);
                self.sync_focus_attr();
                self.refresh_keybar();
            } else {
                self.push_modal(Modal::Logs(super::modals::LogModal::new(
                    self.logging.clone(),
                )));
                self.sync_focus_attr();
                self.refresh_keybar();
            }
            return Vec::new();
        }
        // Globals first.
        if let Event::Keyboard(KeyEvent {
            code: Key::Char('c'),
            modifiers,
        }) = &ev
            && *modifiers == KeyModifiers::CONTROL
        {
            tracing::debug!("user action: Quit (Ctrl-C)");
            return vec![UserAction::Quit];
        }
        if let Event::Keyboard(KeyEvent {
            code: Key::Char('r'),
            modifiers,
        }) = &ev
            && *modifiers == KeyModifiers::CONTROL
        {
            tracing::debug!("user action: ToggleSelfReady (Ctrl-R)");
            let actions = self.toggle_self_ready();
            for action in &actions {
                log_action(action);
            }
            return actions;
        }
        // Bracketed paste (design.md #33). A pasted single existing-file
        // path — dragged in from anywhere, whichever pane is focused —
        // becomes a playlist add, exactly like picking it in the file
        // browser (there is no use for posting a file path to chat); any
        // other paste (not a real file, multi-line) lands in the chat
        // input as plain text, as if typed. While a modal is open the
        // event falls through to the modal, whose active text editor
        // accepts it (LineBuffer::edit handles Event::Paste).
        if let Event::Paste(text) = &ev
            && self.modals.is_empty()
        {
            if let Some(path) = pasted_file_path(text) {
                let msg = Msg::FileChosen {
                    path,
                    after: self.playlist.selected_hash(),
                };
                let action = self.update(msg);
                if let Some(action) = &action {
                    log_action(action);
                }
                self.refresh_keybar();
                return action.into_iter().collect();
            }
            self.chat.insert_text(text);
            self.refresh_keybar();
            return Vec::new();
        }

        if self.modals.is_empty() {
            // Shift-Tab cycles panes backwards. Crossterm reports it as
            // `BackTab`, normally with the SHIFT modifier attached but on
            // some terminals bare; either way it is the same key, and
            // (unlike Tab) it never tries username completion.
            if let Event::Keyboard(KeyEvent {
                code: Key::BackTab,
                modifiers,
            }) = &ev
                && (*modifiers == KeyModifiers::NONE || *modifiers == KeyModifiers::SHIFT)
            {
                self.focus = self.focus.prev();
                tracing::debug!(focus = ?self.focus, "focus changed (reverse)");
                self.sync_focus_attr();
                self.refresh_keybar();
                return Vec::new();
            }
            match super::components::plain(&ev) {
                Some(Key::Tab) => {
                    // In the chat pane, Tab first tries to complete a username
                    // at the end of the input; only if nothing matches does it
                    // fall through to cycling panes.
                    if self.focus == Focus::Chat && self.chat.try_tab_complete() {
                        self.refresh_keybar();
                        return Vec::new();
                    }
                    self.focus = self.focus.next();
                    tracing::debug!(focus = ?self.focus, "focus changed");
                    self.sync_focus_attr();
                    self.refresh_keybar();
                    return Vec::new();
                }
                Some(Key::Function(2)) => {
                    let action = self.cycle_subtitle_mode();
                    self.refresh_keybar();
                    return vec![action];
                }
                Some(Key::Function(3)) => {
                    tracing::debug!("user action: open settings (F3)");
                    self.open_settings();
                    self.refresh_keybar();
                    return Vec::new();
                }
                _ => {}
            }
        }

        let msg = match self.modals.last_mut() {
            Some(modal) => modal.as_component().on(&ev),
            None => match self.focus {
                Focus::Chat => self.chat.on(&ev),
                Focus::Series => self.series.on(&ev),
                Focus::Users => self.users.on(&ev),
                Focus::Playlist => self.playlist.on(&ev),
            },
        };
        if let Some(msg) = &msg {
            tracing::trace!(msg = msg.name(), "msg produced");
        }
        // A confirmed question: close the confirm modal and route the
        // wrapped message exactly as if its originating component had
        // just produced it (it may be one of the multi-action ones
        // below).
        let msg = match msg {
            Some(Msg::Confirmed(inner)) => {
                tracing::debug!(msg = inner.name(), "user action: confirmed");
                self.pop_modal();
                self.sync_focus_attr();
                Some(*inner)
            }
            other => other,
        };
        // A couple of messages yield several actions; route them straight
        // to their handlers (like the Ctrl-R global above) rather than
        // through the single-action `update()`.
        match &msg {
            Some(Msg::Command(cmd)) => {
                let actions = self.command(cmd);
                for action in &actions {
                    log_action(action);
                }
                self.refresh_keybar();
                return actions;
            }
            Some(Msg::SendChat(text)) => {
                let actions = self.send_chat(text);
                for action in &actions {
                    log_action(action);
                }
                self.refresh_keybar();
                return actions;
            }
            Some(Msg::PlaySelected(hash)) => {
                let actions = self.play_selected(*hash);
                for action in &actions {
                    log_action(action);
                }
                self.refresh_keybar();
                return actions;
            }
            Some(Msg::ListEntrySaved(id, entry, next_ep)) => {
                let actions =
                    self.save_list_entry(*id, (**entry).clone(), next_ep.as_deref().cloned());
                for action in &actions {
                    log_action(action);
                }
                self.refresh_keybar();
                return actions;
            }
            Some(Msg::CycleSeriesWatch(hash)) => {
                let actions = self.cycle_series_watch(*hash);
                for action in &actions {
                    log_action(action);
                }
                self.refresh_keybar();
                return actions;
            }
            Some(Msg::SetNotWatching(user)) => {
                let actions =
                    self.set_others_pref(user.clone(), SeriesWatchState::NotWatching, "n");
                for action in &actions {
                    log_action(action);
                }
                self.refresh_keybar();
                return actions;
            }
            Some(Msg::ToggleEpisodeWatched { hashes }) => {
                let actions = self.toggle_episode_watched(hashes);
                for action in &actions {
                    log_action(action);
                }
                self.refresh_keybar();
                return actions;
            }
            Some(Msg::SetEpisodesWatched { hashes, watched }) => {
                let actions = self.set_episodes_watched(hashes, *watched);
                for action in &actions {
                    log_action(action);
                }
                self.refresh_keybar();
                return actions;
            }
            _ => {}
        }
        let action = msg.and_then(|msg| self.update(msg));
        if let Some(action) = &action {
            log_action(action);
        }
        self.refresh_keybar();
        action.into_iter().collect()
    }

    /// `w` in the episode browser: toggle the group watched flag for
    /// the given copies as one episode. The direction follows the
    /// any-copy rule the display uses — any copy flagged means the
    /// episode is watched, so the toggle unmarks every flagged copy;
    /// none flagged marks them all. Only flags that actually change are
    /// written (no junk `false` rows for copies already unmarked).
    fn toggle_episode_watched(&self, hashes: &[Ed2kHash]) -> Vec<UserAction> {
        let flagged = |hash: &Ed2kHash| self.snapshot.view.watched.get(hash) == Some(&true);
        let watched = !hashes.iter().any(flagged);
        self.set_episodes_watched(hashes, watched)
    }

    /// Set the group watched flag on every file in `hashes` to `watched`,
    /// writing only the ones that actually change (the season-row `w`,
    /// and the direction-resolved half of the episode toggle).
    fn set_episodes_watched(&self, hashes: &[Ed2kHash], watched: bool) -> Vec<UserAction> {
        let flagged = |hash: &Ed2kHash| self.snapshot.view.watched.get(hash) == Some(&true);
        hashes
            .iter()
            .filter(|hash| flagged(hash) != watched)
            .map(|hash| UserAction::MarkWatched {
                file: *hash,
                watched,
            })
            .collect()
    }

    /// Ctrl-R: toggle our own readiness. Mirrors the player's
    /// pause/unpause writes (session.rs `on_player`) and doubles as the
    /// only way to mark yourself *watching* again — clearing a series
    /// NotWatching that was auto-set (or chosen) for the now-playing
    /// show, which has no other UI path.
    fn toggle_self_ready(&self) -> Vec<UserAction> {
        let current = derive::user_state(&self.snapshot.view, &self.me);
        tracing::debug!(?current, "Ctrl-R: toggling self readiness");
        // Ready (committed) and Maybe (default) are both "participating"
        // states — Ctrl-R from either pauses; from Paused/Away/NotWatching
        // it becomes ready.
        if matches!(current, DerivedUserState::Ready | DerivedUserState::Maybe) {
            self.become_unready()
        } else {
            self.become_ready()
        }
    }

    /// Become unready: like pressing pause. Sets the manual override (so
    /// others see *who* is blocking) and latches intent Paused. Shared by
    /// Ctrl-R and `/pause`.
    fn become_unready(&self) -> Vec<UserAction> {
        vec![
            UserAction::Mutate(Mutation::SetManualOverride {
                user: self.me.clone(),
                state: Some(ManualState::Paused),
            }),
            UserAction::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Paused,
            }),
        ]
    }

    /// Become ready: like pressing play ("I'm ready, go"). Clears any
    /// manual override, latches Playing, and — if the now-playing series
    /// is marked NotWatching for us — flips it back to **Maybe** (the
    /// default), enough to reach a non-blocking state while present. It
    /// deliberately does *not* commit to Watching; that is `/watch`.
    /// Shared by Ctrl-R and `/ready`.
    fn become_ready(&self) -> Vec<UserAction> {
        let view = &self.snapshot.view;
        let me = self.me.clone();
        let mut actions = vec![
            UserAction::Mutate(Mutation::SetManualOverride {
                user: me.clone(),
                state: None,
            }),
            UserAction::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Playing,
            }),
        ];
        if let Some(file) = view.now_playing
            && let Some(entry) =
                dessplay_core::series_identity::resolve_series_entry_for_file(view, file)
            && view
                .series_preference
                .get(&(me.clone(), entry))
                .map(|pref| pref.state)
                == Some(SeriesWatchState::NotWatching)
        {
            actions.push(UserAction::Mutate(Mutation::SetSeriesPreference {
                user: me,
                entry,
                pref: SeriesWatchState::Maybe,
                set_by: None,
            }));
        }
        actions
    }

    /// Enter on a playlist entry: make it now-playing. A real file change
    /// loads a fresh episode, so -- exactly like an EOF advance
    /// (`dessplay-rendezvous` `handle_eof`) -- it also latches intent
    /// Paused, so the new file comes up paused at the start and the group
    /// presses play when ready. (The server resets seek authority to
    /// Server on the now-playing op, the other half of the transition.)
    ///
    /// Re-selecting the already-playing entry is not a transition, so it is
    /// a true no-op: emitting even a same-value `SetNowPlaying` would make
    /// the server reset seek authority back to Server (it resets on *any*
    /// non-datagram NowPlaying op, value-change or not), yanking it from
    /// whoever just seeked.
    fn play_selected(&self, hash: Ed2kHash) -> Vec<UserAction> {
        if self.snapshot.view.now_playing == Some(hash) {
            return Vec::new();
        }
        vec![
            UserAction::Mutate(Mutation::SetNowPlaying { file: Some(hash) }),
            UserAction::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Paused,
            }),
        ]
    }

    /// Save a List edit: write the entry, and -- only when the user
    /// actually changed `next_ep`/`available` -- the separate progress
    /// register. Keeping the `SetNextEp` write conditional preserves the
    /// reason the register is split out (design.md, The List): a note edit
    /// must not clobber a concurrent server EOF auto-advance, and vice
    /// versa. Two mutations, so this routes through `handle()` rather than
    /// the single-action `update()`.
    fn save_list_entry(
        &mut self,
        id: ListEntryId,
        entry: SeriesListEntry,
        next_ep: Option<NextEpState>,
    ) -> Vec<UserAction> {
        self.pop_modal();
        self.sync_focus_attr();
        let mut actions = vec![UserAction::Mutate(Mutation::PutListEntry { id, entry })];
        if let Some(next_ep) = next_ep {
            actions.push(UserAction::Mutate(Mutation::SetNextEp { id, next_ep }));
        }
        actions
    }

    /// The Elm update: messages become internal changes or actions.
    fn update(&mut self, msg: Msg) -> Option<UserAction> {
        match msg {
            Msg::None => None,
            // `Msg::SendChat`, `Msg::Command`, `Msg::PlaySelected`,
            // `Msg::ListEntrySaved`, `Msg::CycleSeriesWatch`,
            // `Msg::SetNotWatching`, and `Msg::ToggleEpisodeWatched` are
            // intercepted in `handle()` (they can each yield several
            // actions); they never reach `update()`.
            Msg::SendChat(_)
            | Msg::Command(_)
            | Msg::PlaySelected(_)
            | Msg::ListEntrySaved(..)
            | Msg::CycleSeriesWatch(_)
            | Msg::SetNotWatching(_)
            | Msg::ToggleEpisodeWatched { .. }
            | Msg::SetEpisodesWatched { .. }
            | Msg::Confirmed(_) => None,
            Msg::Confirm { prompt, then } => {
                self.push_modal(Modal::Confirm(ConfirmModal::new(prompt, *then)));
                self.sync_focus_attr();
                None
            }
            Msg::CycleSeriesMode | Msg::SeriesFilterChanged => {
                self.refresh_series();
                None
            }
            // The series pane already flipped its own sort; mirror it into the
            // persisted settings and save, so it survives a restart (design.md,
            // Adding Files to the Playlist: "Sort mode for All Series is
            // persisted across sessions").
            Msg::ToggleSeriesSort => {
                self.settings.series_sort = self.series.sort();
                self.refresh_series();
                Some(UserAction::SaveSettings(
                    Box::new(self.settings.clone()),
                    self.media_roots.clone(),
                ))
            }
            // The List pane already flipped its own sort; mirror and save,
            // same pattern as `ToggleSeriesSort` above.
            Msg::ToggleListSort => {
                self.settings.list_sort = self.series.list_sort();
                self.refresh_series();
                Some(UserAction::SaveSettings(
                    Box::new(self.settings.clone()),
                    self.media_roots.clone(),
                ))
            }
            // The file browser already flipped its own sort; mirror it into
            // settings and save (design.md #8), same pattern as
            // `ToggleSeriesSort` above.
            Msg::ToggleBrowserSort => {
                if let Some(Modal::Files(browser)) = self.modals.last() {
                    self.settings.file_browser_sort = browser.sort();
                }
                Some(UserAction::SaveSettings(
                    Box::new(self.settings.clone()),
                    self.media_roots.clone(),
                ))
            }
            Msg::BrowseFranchise(key) => {
                self.open_episode_browser(key);
                None
            }
            Msg::EditListEntry(id) => {
                let entry = self.snapshot.view.list_entries.get(&id)?.clone();
                let next_ep = self
                    .snapshot
                    .view
                    .list_next_ep
                    .get(&id)
                    .cloned()
                    .unwrap_or_default();
                self.push_modal(Modal::ListEdit(ListEditModal::new(id, entry, next_ep)));
                None
            }
            Msg::EditNeroName(id) => {
                let entry = self.snapshot.view.list_entries.get(&id)?;
                self.push_modal(Modal::NeroName(NeroNameModal::new(
                    id,
                    entry.name.clone(),
                    entry.nero_name.as_deref(),
                )));
                None
            }
            Msg::NeroNameSaved(id, nero_name) => {
                self.pop_modal();
                self.sync_focus_attr();
                let mut entry = self.snapshot.view.list_entries.get(&id)?.clone();
                if entry.nero_name == nero_name {
                    return None;
                }
                entry.nero_name = nero_name;
                Some(UserAction::Mutate(Mutation::PutListEntry { id, entry }))
            }
            Msg::BrowseListEntry(id) => {
                let view = &self.snapshot.view;
                let entry = view.list_entries.get(&id)?.clone();
                // A linked entry whose franchise holds files opens the
                // episode browser. The franchise is matched by full
                // component membership: the linked season is usually not
                // the component root (the franchise's key), and may
                // itself hold no files — an exact-key lookup here
                // silently missed both (2026-08-17).
                if let Some(series) = entry.anidb_series_id
                    && let Some(key) = franchise::franchises(view)
                        .into_iter()
                        .find(|franchise| franchise.members.contains(&series))
                        .map(|franchise| franchise.key)
                {
                    self.open_episode_browser(key);
                    return None;
                }
                // Unlinked — or linked but nothing held anywhere: the
                // candidate view, then the editor. Never a silent no-op.
                let view = &self.snapshot.view;
                let next_ep = view.list_next_ep.get(&id);
                let rows = props::candidate_rows(view, &entry, next_ep);
                if rows.is_empty() {
                    // Nothing to disambiguate: fall back to the plain editor.
                    return self.update(Msg::EditListEntry(id));
                }
                let season = Season {
                    title: entry.name.clone(),
                    // Row 0 is always the synthetic Header; the best-ranked
                    // candidate (index 1) is where the cursor should open.
                    first_unwatched: Some(1),
                    opening_row: 1,
                    episodes: rows,
                    depth: 0,
                    watchable: true,
                };
                self.push_modal(Modal::Episodes(EpisodeBrowser::new(
                    entry.name,
                    vec![season],
                )));
                None
            }
            Msg::LinkListEntry(id) => {
                let entry = self.snapshot.view.list_entries.get(&id)?;
                let name = entry.name.clone();
                self.push_modal(Modal::AniDbSearch(AniDbSearchModal::new(id, name.clone())));
                // Fire the initial search for the entry's name.
                Some(UserAction::AniDbSearch { query: name })
            }
            Msg::AniDbSearchRequested(query) => Some(UserAction::AniDbSearch { query }),
            Msg::ListEntryLinked(id, series) => {
                self.pop_modal();
                self.sync_focus_attr();
                let mut entry = self.snapshot.view.list_entries.get(&id)?.clone();
                entry.anidb_series_id = Some(series);
                Some(UserAction::Mutate(Mutation::PutListEntry { id, entry }))
            }
            Msg::ToggleAway(user) => {
                // The `a` toggle only clears an Away *we* set (design.md,
                // Keyboard Shortcuts: "clear an Away you set"). An Away set by
                // someone else is cleared by the marked user's own "I'm here"
                // action -- pressing `a` on it instead (re-)marks them Away by
                // us rather than silently overriding the other setter.
                let cleared_by_me = matches!(
                    self.snapshot.view.manual_override.get(&user),
                    Some(Some(ManualState::Away { set_by })) if *set_by == self.me
                );
                let state = if cleared_by_me {
                    None
                } else {
                    Some(ManualState::Away {
                        set_by: self.me.clone(),
                    })
                };
                Some(UserAction::Mutate(Mutation::SetManualOverride {
                    user,
                    state,
                }))
            }
            Msg::AddFileAfter(after) => {
                // The browser wants the library index (recursive search,
                // watched greying, cursor placement), which lives in
                // storage — ask the main loop; the answer arrives as
                // [`UiInput::Browse`] and opens the modal.
                Some(UserAction::Browse(BrowseRequest::Add { after }))
            }
            Msg::OpenNyaa(after) => {
                if !self.settings.torrent_enabled {
                    return Some(UserAction::Notice(
                        "BitTorrent downloads are disabled; enable them in settings and restart."
                            .to_string(),
                    ));
                }
                let after =
                    after.or_else(|| self.snapshot.view.playlist.last().map(|entry| entry.hash));
                let active = self.nyaa_imports.values().cloned().collect();
                self.push_modal(Modal::NyaaSearch(NyaaSearchModal::new(after, active)));
                None
            }
            Msg::MoveUp(hash) => {
                let index = self.playlist_index(hash)?;
                if index == 0 {
                    return None;
                }
                let anchor = (index >= 2).then(|| self.snapshot.view.playlist[index - 2].hash);
                Some(UserAction::Mutate(Mutation::MovePlaylistAfter {
                    hash,
                    anchor,
                }))
            }
            Msg::MoveDown(hash) => {
                let index = self.playlist_index(hash)?;
                let anchor = self.snapshot.view.playlist.get(index + 1)?.hash;
                Some(UserAction::Mutate(Mutation::MovePlaylistAfter {
                    hash,
                    anchor: Some(anchor),
                }))
            }
            Msg::RemoveEntry(hash) => Some(UserAction::Mutate(Mutation::RemovePlaylist { hash })),
            Msg::MapFile(hash) => {
                let entry = self
                    .snapshot
                    .view
                    .playlist
                    .iter()
                    .find(|e| e.hash == hash)?;
                let target = entry.state.filename.clone();
                // The series key lets the main loop start the browser at
                // the series' last-used mapping directory.
                Some(UserAction::Browse(BrowseRequest::Map {
                    file: hash,
                    target,
                    series: self.series_key_of(hash),
                }))
            }
            Msg::FileMapped { file, path } => {
                self.pop_modal();
                self.sync_focus_attr();
                Some(UserAction::MapFile {
                    file,
                    // A mapping is a durable row; never persist a
                    // cwd-dependent path.
                    path: canonical_or_original(path),
                    series: self.series_key_of(file),
                })
            }
            Msg::LocalCopyOfferDismissed(file) => {
                self.pop_modal();
                self.sync_focus_attr();
                Some(UserAction::LocalCopyOfferDismissed { file })
            }
            Msg::ChangelogDismissed(marker) => {
                self.pop_modal();
                self.sync_focus_attr();
                Some(UserAction::ChangelogSeen { marker })
            }
            Msg::NyaaSearchRequested(query) => Some(UserAction::SearchNyaa { query }),
            Msg::NyaaResultChosen { result, after } => {
                self.pop_modal();
                self.sync_focus_attr();
                let id = crate::torrent::engine::TorrentImportId(self.next_nyaa_import_id);
                self.next_nyaa_import_id = self.next_nyaa_import_id.saturating_add(1);
                Some(UserAction::StartNyaaImport { id, result, after })
            }
            Msg::CancelNyaaImport(id) => Some(UserAction::CancelNyaaImport { id }),
            Msg::NewNyaaSearch => None,
            Msg::ArchiveFile(hash) => {
                let entry = self
                    .snapshot
                    .view
                    .playlist
                    .iter()
                    .find(|e| e.hash == hash)?;
                let filename = entry.state.filename.clone();
                let series_name = self
                    .snapshot
                    .view
                    .anidb_metadata
                    .get(&hash)
                    .and_then(|m| m.as_ref())
                    .map(|m| m.series_name.clone());
                Some(UserAction::Archive {
                    file: hash,
                    series_name,
                    filename,
                })
            }
            Msg::CloseModal => {
                self.pop_modal();
                self.sync_focus_attr();
                None
            }
            Msg::FileChosen { path, after } => {
                self.pop_modal();
                self.sync_focus_attr();
                // `None` (from [Add New]) appends.
                let after =
                    after.or_else(|| self.snapshot.view.playlist.last().map(|entry| entry.hash));
                Some(UserAction::HashAndAdd {
                    path: canonical_or_original(path),
                    after,
                })
            }
            Msg::EpisodeChosen { hash } => {
                self.pop_modal();
                self.sync_focus_attr();
                // Already queued? Don't re-add (it would just reorder).
                if self.snapshot.view.playlist.iter().any(|e| e.hash == hash) {
                    return None;
                }
                let after = self.snapshot.view.playlist.last().map(|entry| entry.hash);
                Some(UserAction::AddByHash { hash, after })
            }
            Msg::OpenDirPicker => {
                self.push_modal(Modal::Files(FileBrowser::for_directory()));
                None
            }
            // The Settings action row's path to the same reset `/resync`
            // performs; the modal stays open (like OpenDirPicker's flow).
            Msg::ResetSyncedState => Some(UserAction::ResetSyncedState),
            Msg::DirChosen(path) => {
                self.pop_modal();
                if let Some(Modal::Settings(settings)) = self.modals.last_mut() {
                    settings.add_root(path);
                }
                None
            }
            Msg::SettingsSaved(settings, roots) => {
                self.pop_modal();
                self.sync_focus_attr();
                self.settings = (*settings).clone();
                self.subtitle_mode = settings.subtitle_mode;
                self.media_roots = roots.clone();
                // A marquee pass mid-scroll stops when the display moves
                // off the bottom line; the stamp stays adopted, so it
                // never replays if the setting flips back.
                if self.settings.marquee_mode != MarqueeMode::Marquee
                    && let Some(anim) = &mut self.marquee
                {
                    anim.done = true;
                }
                // The mode may have flipped Intermixed membership.
                self.refresh_chat();
                // First-run setup may have changed who we are — adopt the
                // saved username. But when the identity is locked to a
                // runtime `--username` override, the saved (persisted) name
                // is deliberately the *stored* value, not our identity, so
                // leave `self.me` on the override (see `identity_locked`).
                if !self.identity_locked
                    && let Some(name) = &settings.username
                {
                    self.me = UserId::new(name.clone());
                }
                Some(UserAction::SaveSettings(settings, roots))
            }
            Msg::FocusNext => {
                self.focus = self.focus.next();
                tracing::debug!(focus = ?self.focus, "focus changed");
                self.sync_focus_attr();
                None
            }
            Msg::CycleSubtitleMode => Some(self.cycle_subtitle_mode()),
            Msg::Quit => Some(UserAction::Quit),
        }
    }

    fn playlist_index(&self, hash: Ed2kHash) -> Option<usize> {
        self.snapshot
            .view
            .playlist
            .iter()
            .position(|entry| entry.hash == hash)
    }

    /// The series key for a file, for remembering its mapping directory:
    /// AniDB id when metadata has one, else the parsed series name, else
    /// `None` (no metadata yet).
    fn series_key_of(&self, hash: Ed2kHash) -> Option<crate::storage::SeriesKey> {
        let metadata = self.snapshot.view.anidb_metadata.get(&hash)?.as_ref()?;
        Some(match metadata.series_id {
            Some(id) => crate::storage::SeriesKey::AniDb(id),
            None => crate::storage::SeriesKey::Name(metadata.series_name.clone()),
        })
    }

    /// Send a chat message. Sending a line counts as activity from this
    /// client, so it clears an [`ManualState::Away`] on us (someone
    /// stepped off and is now back) — but never a deliberate
    /// [`ManualState::Paused`], and merely *typing* (without Enter) does
    /// nothing. Mirrors the unpause-clears-Away path in
    /// `session.rs::on_player`.
    fn send_chat(&self, text: &str) -> Vec<UserAction> {
        let mut actions = Vec::new();
        if matches!(
            self.snapshot.view.manual_override.get(&self.me),
            Some(Some(ManualState::Away { .. }))
        ) {
            actions.push(UserAction::Mutate(Mutation::SetManualOverride {
                user: self.me.clone(),
                state: None,
            }));
        }
        actions.push(UserAction::Mutate(Mutation::Chat {
            text: text.to_string(),
        }));
        actions
    }

    /// `/commands` from the chat input. Returns the actions a command
    /// produces (often several); an empty vec means "handled, nothing to
    /// send" (e.g. `/settings`). Unknown or unusable commands return a
    /// single [`UserAction::Notice`] for local chat feedback. See
    /// [`super::commands`] for the discoverability table.
    fn command(&mut self, command: &str) -> Vec<UserAction> {
        let mut parts = command.split_whitespace();
        let Some(verb) = parts.next() else {
            return Vec::new();
        };
        match verb {
            "/quit" | "/exit" | "/q" => vec![UserAction::Quit],
            "/settings" => {
                self.open_settings();
                Vec::new()
            }
            "/changelog" => self.open_changelog(),
            "/ready" => self.become_ready(),
            "/pause" => self.become_unready(),
            // `/me <action>` emotes an IRC-style action. It is an ordinary
            // (synced) chat message carrying a CTCP-encoded body, routed
            // through `send_chat` so it also clears your own Away — sending
            // any chat line is an "I'm here" action. The raw remainder is
            // kept verbatim (internal spacing preserved).
            "/me" => {
                let action = command.strip_prefix("/me").map_or("", str::trim_start);
                if action.is_empty() {
                    vec![UserAction::Notice(
                        "/me: describe an action, e.g. /me waves".to_string(),
                    )]
                } else {
                    self.send_chat(&encode_action(action))
                }
            }
            // `/away` marks yourself by default; an optional name targets
            // another user. `/afk` is a name-taking alias (legacy spelling).
            "/away" | "/afk" => {
                let user = parts
                    .next()
                    .map(UserId::new)
                    .unwrap_or_else(|| self.me.clone());
                vec![UserAction::Mutate(Mutation::SetManualOverride {
                    user,
                    state: Some(ManualState::Away {
                        set_by: self.me.clone(),
                    }),
                })]
            }
            // Per-series watch state for the now-playing file. Each needs
            // an AniDB series id to key the preference on (Phase 9A).
            // `/watch` commits (the group waits for you even when absent);
            // `/maybe` is the opportunistic default; `/skip` opts out.
            "/watch" => self.set_now_playing_pref(SeriesWatchState::Watching, "/watch"),
            "/maybe" => self.set_now_playing_pref(SeriesWatchState::Maybe, "/maybe"),
            // `/skip` marks yourself by default; an optional name targets
            // another user (design.md #7/#13) — mirrors `/away <name>`'s
            // no-validation `UserId::new`.
            "/skip" => match parts.next() {
                Some(name) => {
                    self.set_others_pref(UserId::new(name), SeriesWatchState::NotWatching, "/skip")
                }
                None => self.set_now_playing_pref(SeriesWatchState::NotWatching, "/skip"),
            },
            // Play past a committed-but-absent blocker of the current file.
            "/ack" => self.acknowledge_blockers(),
            // `/reveal`: keyboard path for the spoiler mouse flow
            // (design.md: every mouse action has a key equivalent).
            // Reveals the newest still-hidden spoiler in the visible
            // log directly — no click tease.
            "/reveal" => {
                if self.chat.reveal_newest_visible() {
                    Vec::new()
                } else {
                    vec![UserAction::Notice(
                        "/reveal: no hidden spoiler on screen".to_string(),
                    )]
                }
            }
            // Ping absent known users on IRC (design.md #4).
            "/summon" => self.command_summon(),
            // `/resync`: clear the local synced state and restart the
            // client (clear-and-re-exec; the restart's connect handshake
            // re-adopts the server's copy) — the manual remedy for
            // persistent divergence (docs/sync-state.md, Manual Reset).
            // No confirm modal: typing the command is the deliberate
            // act, and the state is losslessly recoverable from the
            // server.
            "/resync" => vec![
                UserAction::ResetSyncedState,
                UserAction::Notice(
                    "/resync: clearing local synced state and restarting".to_string(),
                ),
            ],
            other => vec![UserAction::Notice(format!(
                "Unknown command: {other} — type / to see commands"
            ))],
        }
    }

    /// The List entry claiming the now-playing file, if resolvable — the
    /// key the per-series watch commands write against (design.md, Series
    /// Identity). Auto-creates one (via a `PutListEntry` action to send
    /// alongside) if nothing claims it yet; `None` only when nothing is
    /// playing or the file has no metadata at all.
    fn now_playing_entry(&self) -> Option<(ListEntryId, Vec<UserAction>)> {
        let view = &self.snapshot.view;
        let file = view.now_playing?;
        let (entry, create) = crate::session::resolve_or_create_series_entry(view, file)?;
        Some((entry, create.into_iter().map(UserAction::Mutate).collect()))
    }

    /// Set our watch preference for the now-playing file's series, or post
    /// a local notice when there is no series info yet.
    fn set_now_playing_pref(&self, pref: SeriesWatchState, cmd: &str) -> Vec<UserAction> {
        match self.now_playing_entry() {
            Some((entry, mut actions)) => {
                actions.push(UserAction::Mutate(Mutation::SetSeriesPreference {
                    user: self.me.clone(),
                    entry,
                    pref,
                    set_by: None,
                }));
                actions
            }
            None => vec![UserAction::Notice(format!(
                "{cmd}: no series info for the current file yet"
            ))],
        }
    }

    /// Set another user's watch preference for the now-playing file's
    /// series, attributed to us (design.md #7/#13: `n` on the Users pane,
    /// `/skip <name>`). A local notice when there is no series info yet —
    /// the mirror of [`Self::set_now_playing_pref`], which the self-directed
    /// commands still use unattributed (`set_by: None`).
    fn set_others_pref(&self, user: UserId, pref: SeriesWatchState, cmd: &str) -> Vec<UserAction> {
        match self.now_playing_entry() {
            Some((entry, mut actions)) => {
                actions.push(UserAction::Mutate(Mutation::SetSeriesPreference {
                    user,
                    entry,
                    pref,
                    set_by: Some(self.me.clone()),
                }));
                actions
            }
            None => vec![UserAction::Notice(format!(
                "{cmd}: no series info for the current file yet"
            ))],
        }
    }

    /// Playlist `w`: cycle the given file's watch state Maybe -> Watching
    /// -> NotWatching -> Maybe (absent = Maybe). Commit comes first: the
    /// common press happens mid-show, so the first `w` from the default
    /// must mean "I'm watching this", not skip it. Auto-creates a List
    /// entry (design.md, Series Identity) if nothing claims the file yet,
    /// so this works even for a series AniDB doesn't know about -- unlike
    /// `update()`'s single-action return, this can yield the create *and*
    /// the preference write, so it is intercepted in `handle()`.
    fn cycle_series_watch(&self, hash: Ed2kHash) -> Vec<UserAction> {
        let view = &self.snapshot.view;
        let Some((entry, create)) = crate::session::resolve_or_create_series_entry(view, hash)
        else {
            return vec![UserAction::Notice(
                "watch: no series info for that file yet".to_string(),
            )];
        };
        let mut actions: Vec<UserAction> = create.into_iter().map(UserAction::Mutate).collect();
        let current = view
            .series_preference
            .get(&(self.me.clone(), entry))
            .map(|pref| pref.state)
            .unwrap_or(SeriesWatchState::Maybe);
        let next = match current {
            SeriesWatchState::Maybe => SeriesWatchState::Watching,
            SeriesWatchState::Watching => SeriesWatchState::NotWatching,
            SeriesWatchState::NotWatching => SeriesWatchState::Maybe,
        };
        actions.push(UserAction::Mutate(Mutation::SetSeriesPreference {
            user: self.me.clone(),
            entry,
            pref: next,
            set_by: None,
        }));
        actions
    }

    /// `/ack`: acknowledge every committed-but-absent blocker of the
    /// now-playing file (a per-file one-shot) and latch Playing — "play
    /// anyway". A no-op notice when nothing is playing or nobody is a
    /// committed-absent blocker.
    fn acknowledge_blockers(&self) -> Vec<UserAction> {
        let view = &self.snapshot.view;
        let Some(file) = view.now_playing else {
            return vec![UserAction::Notice("/ack: nothing is playing".to_string())];
        };
        let blockers: Vec<UserId> = derive::playback_blockers(view, &self.snapshot.peers)
            .into_iter()
            .filter(|blocker| blocker.reason == derive::BlockReason::CommittedAbsent)
            .map(|blocker| blocker.user)
            .collect();
        if blockers.is_empty() {
            return vec![UserAction::Notice(
                "/ack: no committed-but-absent blockers right now".to_string(),
            )];
        }
        let mut actions: Vec<UserAction> = blockers
            .into_iter()
            .map(|user| UserAction::Mutate(Mutation::AcknowledgeAbsent { file, user }))
            .collect();
        actions.push(UserAction::Mutate(Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        }));
        actions
    }

    /// `/summon`: ping every known-but-offline user (design.md #15's
    /// registry, already "present peers excluded") on IRC. The
    /// bridge-disabled and nobody-absent cases are decided here (no round
    /// trip needed — both are already in `self.snapshot`/`self.settings`);
    /// everything requiring live channel membership (matching a username to
    /// a nick, sending the PRIVMSG) happens in the IRC actor and reports
    /// back through `IrcEvent::Summoned`.
    fn command_summon(&self) -> Vec<UserAction> {
        if !self.settings.irc_enabled {
            return vec![UserAction::Notice(
                "/summon: IRC bridge disabled".to_string(),
            )];
        }
        let absent: Vec<UserId> = self
            .snapshot
            .known_offline
            .iter()
            .map(|user| user.username.clone())
            .collect();
        if absent.is_empty() {
            return vec![UserAction::Notice("/summon: everyone's here".to_string())];
        }
        vec![UserAction::Summon(absent)]
    }

    fn open_episode_browser(&mut self, key: FranchiseKey) {
        let view = &self.snapshot.view;
        let franchise = franchise::franchises(view)
            .into_iter()
            .find(|franchise| franchise.key == key);
        let Some(franchise) = franchise else { return };
        // Group a season's known files into rows (design.md #31/#11):
        // sorted and grouped by AniDB episode identity, muted by group
        // flag or personal history, with the browser's opening cursor on
        // the first unwatched row.
        let personal_watched = &self.snapshot.personal_watched;
        let build_season = |title: String, hashes: Vec<Ed2kHash>| -> Season {
            let episodes = props::episode_rows(view, &hashes, personal_watched);
            Season::new(title, episodes, personal_watched, view)
        };
        let seasons: Vec<Season> = if franchise.series.is_empty() {
            vec![build_season(
                franchise.title.clone(),
                franchise.files.clone(),
            )]
        } else {
            // Prequel-chain order with side branches indented under
            // their parent (proposal 2026-08-28).
            props::season_tree(view, &franchise)
                .into_iter()
                .map(|(series, depth)| {
                    let title = view
                        .series_relations
                        .get(&series)
                        .map(|relations| relations.title.clone())
                        .unwrap_or_else(|| format!("anidb:{}", series.0));
                    let hashes = view
                        .anidb_metadata
                        .iter()
                        .filter_map(|(hash, metadata)| {
                            let metadata = metadata.as_ref()?;
                            (metadata.series_id == Some(series)).then_some(*hash)
                        })
                        .collect();
                    build_season(title, hashes).at_depth(depth)
                })
                .collect()
        };
        self.push_modal(Modal::Episodes(EpisodeBrowser::new(
            franchise.title,
            seasons,
        )));
    }

    /// Render the whole screen (design.md, TUI Layout).
    pub fn draw(&mut self, frame: &mut Frame) {
        use tuirealm::component::Component;
        let [main, status_area, keybar_area] = Layout::vertical([
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        // The main area's last row is one terminal-wide, borderless
        // bottom line: progress bar + time on the left (design.md #6 —
        // its own row, never sharing with the status bar's
        // variable-width blocker text), connection-health metrics
        // right-aligned, and the middle space reserved for the
        // suggestion / future marquee commentary (design.md, Connection
        // Health Line). Reserving it before the column split also puts
        // the playlist's bottom border level with the chat input's.
        let [panes_area, bottom_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(main);
        // Splitter shares come from settings (mouse-draggable, persisted);
        // the defaults reproduce the original 50/50 and 34/33/33 split.
        let layout = self.settings.pane_layout.clamped();
        let [left, right] = Layout::horizontal([
            Constraint::Percentage(layout.chat_width.into()),
            Constraint::Percentage((100 - layout.chat_width).into()),
        ])
        .areas(panes_area);
        let [series_area, users_area, playlist_area] = Layout::vertical([
            Constraint::Percentage(layout.series_height.into()),
            Constraint::Percentage(layout.users_height.into()),
            Constraint::Percentage(layout.playlist_height().into()),
        ])
        .areas(right);
        // Remember where the panes landed for mouse hit-testing.
        self.panes = PaneRects {
            area: panes_area,
            chat: left,
            subs: Rect::default(),
            series: series_area,
            users: users_area,
            playlist: playlist_area,
        };

        if self.subtitle_mode == SubtitleMode::SeparatePane {
            let [chat_area, subs_area] = Layout::vertical([
                Constraint::Percentage((100 - layout.subtitle_height).into()),
                Constraint::Percentage(layout.subtitle_height.into()),
            ])
            .areas(left);
            self.panes.subs = subs_area;
            self.chat.view(frame, chat_area);
            // The newest lines that fit, newest first (top) — the input box
            // sits just below, so the freshest line is closest to the eye.
            // A wheel scroll-back skips `subtitle_scroll` newest entries,
            // clamped so the oldest entry never rises above the bottom.
            // Each line: a dim in-video timestamp, then text colored by its
            // ASS speaker. Limited terminals preserve the existing name hash
            // into the app palette; RGB terminals use the stable
            // activity-window slot to generate another perceptually spaced
            // color as needed. Speaker names are opt-in and formatted by the
            // same helper used for Intermixed mode.
            use tuirealm::ratatui::text::{Line, Span};
            let visible = (subs_area.height as usize).saturating_sub(2);
            self.subtitle_scroll = self
                .subtitle_scroll
                .min(self.subtitles.len().saturating_sub(visible));
            let limited_palette_overflow = self.color_depth == ColorDepth::Limited
                && self.speaker_colors.len() > super::theme::LIMITED_SPEAKER_CAPACITY;
            let speaker_colors_enabled = self.settings.subtitle_speaker_colors
                && !(limited_palette_overflow
                    && self.settings.subtitle_speaker_overflow
                        == SubtitleSpeakerOverflow::DisableColors);
            let lines: Vec<Line> = self
                .subtitles
                .iter()
                .rev()
                .skip(self.subtitle_scroll)
                .take(visible)
                .map(|entry| {
                    let text = props::subtitle_text(
                        &entry.text,
                        entry.speaker.as_ref(),
                        self.settings.subtitle_speaker_names,
                    );
                    let text_style = if !speaker_colors_enabled {
                        super::theme::dim()
                    } else {
                        match entry.speaker_slot {
                            Some(slot) if self.color_depth == ColorDepth::TrueColor => {
                                super::theme::speaker_truecolor(slot)
                            }
                            _ => match &entry.speaker {
                                Some(name) => super::theme::user_style(name),
                                None => tuirealm::ratatui::style::Style::default(),
                            },
                        }
                    };
                    Line::from(vec![
                        Span::styled(
                            format!("{}  ", props::mmss(entry.video_millis)),
                            super::theme::dim(),
                        ),
                        Span::styled(text, text_style),
                    ])
                })
                .collect();
            frame.render_widget(
                tuirealm::ratatui::widgets::Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(super::theme::dim())
                        .title(if self.subtitle_scroll == 0 {
                            "Subtitles".to_string()
                        } else {
                            format!("Subtitles (-{})", self.subtitle_scroll)
                        }),
                ),
                subs_area,
            );
        } else {
            // Hidden pane: forget any scroll-back so it comes back live.
            self.subtitle_scroll = 0;
            // Off and Intermixed both use the full-height chat pane
            // (Intermixed shows subtitles inside the chat log).
            self.chat.view(frame, left);
        }
        self.series.view(frame, series_area);
        self.users.view(frame, users_area);
        self.playlist.view(frame, playlist_area);
        let progress = self.status.progress_text();
        let marquee_frame = self
            .marquee
            .as_ref()
            .filter(|anim| !anim.done)
            .map(|anim| (anim.text.as_str(), anim.offset));
        let slot_width = self
            .health
            .render(frame, bottom_area, &progress, marquee_frame);
        if let Some(anim) = &mut self.marquee {
            // Measure the slot every draw (even while a warning owns
            // it), so the done-check tracks the real width.
            anim.slot_width = Some(slot_width);
        }
        self.status.view(frame, status_area);
        self.keybar.view(frame, keybar_area);
        if matches!(self.modals.last(), Some(Modal::Logs(_))) {
            let area = frame.area();
            let y = super::modals::LogModal::area(area).bottom();
            self.chat.render_recent(
                frame,
                Rect {
                    y,
                    height: keybar_area.y.saturating_sub(y),
                    ..area
                },
            );
        }
        if let Some(modal) = self.modals.last_mut() {
            modal.as_component().view(frame, frame.area());
        }
        if !matches!(self.modals.last(), Some(Modal::Logs(_))) {
            self.draw_work_overlay(frame);
        }
        super::theme::apply_color_depth(frame.buffer_mut(), self.color_depth);
    }

    /// The hashing progress overlay: visually modal (centered, on top
    /// of everything), but it captures no input — chat keeps working
    /// while files hash. Design.md's no-silent-work rule.
    fn draw_work_overlay(&self, frame: &mut Frame<'_>) {
        use tuirealm::ratatui::layout::Rect;
        use tuirealm::ratatui::widgets::{Clear, Paragraph};

        // The Nyaa modal itself shows active imports and their cancellation
        // controls; do not paint the passive overlay over those controls.
        if matches!(self.modals.last(), Some(Modal::NyaaSearch(_)))
            || (self.hashing.is_empty() && self.nyaa_imports.is_empty())
        {
            return;
        }
        let has_nyaa = !self.nyaa_imports.is_empty();
        let mut rows: Vec<(String, u64, u64)> = self
            .hashing
            .iter()
            .map(|(filename, done, total)| {
                let label = if has_nyaa {
                    format!("Hashing {filename}")
                } else {
                    filename.clone()
                };
                (label, *done, *total)
            })
            .collect();
        rows.extend(self.nyaa_imports.values().map(|row| {
            let stage = match row.stage {
                crate::actors::file::NyaaImportStage::Downloading => "Downloading",
                crate::actors::file::NyaaImportStage::Hashing => "Hashing",
            };
            (
                format!("{stage} {}", row.filename),
                row.done_bytes,
                row.total_bytes,
            )
        }));
        let overlay = hash_overlay_rect(frame.area(), rows.len());
        frame.render_widget(Clear, overlay);
        frame.render_widget(
            Block::default().borders(Borders::ALL).title(if has_nyaa {
                "Adding to playlist"
            } else {
                "Hashing for playlist"
            }),
            overlay,
        );
        for (i, (filename, done, total)) in rows.iter().enumerate() {
            let y = overlay.y + 1 + (i as u16) * 2;
            if y + 1 >= overlay.y + overlay.height {
                break;
            }
            let inner_x = overlay.x + 1;
            let inner_w = overlay.width.saturating_sub(2);
            frame.render_widget(
                Paragraph::new(filename.as_str()),
                Rect {
                    x: inner_x,
                    y,
                    width: inner_w,
                    height: 1,
                },
            );
            let ratio = if *total > 0 {
                (*done as f64 / *total as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            // A classic [####    ] bar — fill length is easier to track
            // at a glance than a number.
            let slots = inner_w.saturating_sub(2) as usize;
            let filled = (ratio * slots as f64).round() as usize;
            let bar = format!("[{}{}]", "#".repeat(filled), " ".repeat(slots - filled));
            frame.render_widget(
                Paragraph::new(bar),
                Rect {
                    x: inner_x,
                    y: y + 1,
                    width: inner_w,
                    height: 1,
                },
            );
        }
    }

    /// Is a modal open? (Tests and the shell.)
    pub fn modal_open(&self) -> bool {
        !self.modals.is_empty()
    }

    /// Current settings (the shell persists them on save).
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
}

/// The centered rect for the playlist-add hashing overlay: 60% of the
/// frame width (min 20), two rows per in-flight file plus a border.
///
/// The arithmetic is done in u32: `area.width * 3` overflows u16 on an
/// extremely wide terminal (panic in debug, garbage rect in release) — the
/// same class of bug fixed in `modals::overlay`. Both dimensions are
/// clamped back under the frame, so the final `as u16` is always in range.
fn hash_overlay_rect(
    area: tuirealm::ratatui::layout::Rect,
    n_hashing: usize,
) -> tuirealm::ratatui::layout::Rect {
    let aw = u32::from(area.width);
    let ah = u32::from(area.height);
    let height = ((n_hashing as u32) * 2 + 2).min(ah) as u16;
    let width = (aw * 3 / 5).clamp(20u32.min(aw), aw) as u16;
    tuirealm::ratatui::layout::Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Interpret pasted text as a dragged-in file path. Terminals hand a
/// drag to the app as pasted text in one of a few shapes — the bare
/// path, a shell-escaped path (`/a/My\ Show/ep.mkv`), a quoted path, or
/// a `file://` URL — so each reading is tried against the filesystem
/// and the first that names an existing file wins (design.md #33).
/// Multi-line pastes and anything that names no existing file are not
/// paths (the caller sends those to the chat input instead). Existence
/// is the arbiter, so text that merely *looks* escaped or quoted still
/// adds fine when the literal path exists.
fn pasted_file_path(text: &str) -> Option<PathBuf> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }
    let mut candidates: Vec<String> = vec![trimmed.to_string()];
    // Wrapped in matching quotes (Windows Terminal, some Linux terminals).
    for quote in ['\'', '"'] {
        if trimmed.len() >= 2 && trimmed.starts_with(quote) && trimmed.ends_with(quote) {
            candidates.push(trimmed[1..trimmed.len() - 1].to_string());
        }
    }
    // Shell backslash-escapes (`My\ Show`) — the macOS Terminal drag form.
    if trimmed.contains('\\') {
        let mut unescaped = String::with_capacity(trimmed.len());
        let mut chars = trimmed.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(next) = chars.next() {
                        unescaped.push(next);
                    }
                }
                c => unescaped.push(c),
            }
        }
        candidates.push(unescaped);
    }
    // file:// URLs (percent-encoded), with an optional host part
    // (`file:///path` or `file://localhost/path`).
    if let Some(rest) = trimmed.strip_prefix("file://") {
        let path = if rest.starts_with('/') {
            rest
        } else {
            rest.find('/').map(|i| &rest[i..]).unwrap_or(rest)
        };
        candidates.push(
            percent_encoding::percent_decode_str(path)
                .decode_utf8_lossy()
                .into_owned(),
        );
    }
    candidates
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .map(canonical_or_original)
}

/// Canonicalize a user-supplied file path at the boundary. Paths picked
/// or pasted here can become *durable* registrations (an out-of-root add
/// persists a manual mapping), so a relative path accepted verbatim
/// turns into a permanent cwd-dependent row that names nothing after any
/// cwd change (2026-08-20 review). Canonicalization failure (the file
/// vanished between the check and here) falls back to the original.
fn canonical_or_original(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use dessplay_core::state::CrdtState;
    use dessplay_core::types::{
        ActorId, AniDbMetadata, AniDbSeriesId, ListStatus, MetadataSource, SharedTimestamp,
    };
    use std::num::NonZeroU64;

    const A: ActorId = ActorId::SERVER;

    /// Link a List entry to `series` so preference writes/gating resolve
    /// through it (design.md, Series Identity).
    fn link_series(
        state: &mut CrdtState,
        ts: SharedTimestamp,
        series: AniDbSeriesId,
    ) -> ListEntryId {
        let id = ListEntryId(series.0 as u128);
        state.put_list_entry(
            A,
            ts,
            id,
            SeriesListEntry {
                name: "Show".into(),
                nero_name: None,
                genre: None,
                notes: Vec::new(),
                recommender: None,
                status: ListStatus::Active,
                status_note: None,
                source: None,
                watchers: Default::default(),
                anidb_series_id: Some(series),
                local_aliases: Default::default(),
                manual_files: Default::default(),
                anidb_unavailable: false,
            },
        );
        id
    }

    /// Regression: the hashing-overlay rect must not overflow u16 when the
    /// terminal is extremely wide. `area.width * 3` overflowed u16 (panic in
    /// debug, garbage rect in release) — e.g. 30000 * 3 = 90000 > u16::MAX.
    /// The result must stay clamped to the frame.
    #[test]
    fn hash_overlay_rect_does_not_overflow_on_a_very_wide_terminal() {
        use tuirealm::ratatui::layout::Rect;
        let area = Rect::new(0, 0, 30000, 30000);
        let rect = hash_overlay_rect(area, 3);
        assert_eq!(rect.width, 18000); // 30000 * 3 / 5
        assert_eq!(rect.height, 8); // 3 files * 2 + 2
        assert!(rect.width <= area.width && rect.height <= area.height);
        assert_eq!(rect.x, (area.width - rect.width) / 2);
        assert_eq!(rect.y, (area.height - rect.height) / 2);
    }

    fn me() -> UserId {
        UserId::new("kim")
    }

    fn ui_with_view(view: StateView) -> Ui {
        let mut ui = Ui::with_setup(me(), Settings::default(), vec![], false);
        ui.snapshot.view = std::sync::Arc::new(view);
        ui
    }

    fn mutations(actions: &[UserAction]) -> Vec<&Mutation> {
        actions
            .iter()
            .filter_map(|a| match a {
                UserAction::Mutate(m) => Some(m),
                _ => None,
            })
            .collect()
    }

    /// A now-playing file (series 7) with the given explicit series
    /// preference for `user`, or none (absent = the Maybe default).
    fn now_playing_state_with_pref(user: &UserId, pref: Option<SeriesWatchState>) -> StateView {
        let mut state = CrdtState::new();
        state.push_playlist_entry(
            A,
            SharedTimestamp(1),
            dessplay_core::playlist::NewPlaylistEntry {
                hash: Ed2kHash([1; 16]),
                added_by: UserId::new("baughn"),
                filename: "ep1.mkv".into(),
                size_bytes: 1,
                duration_millis: None,
            },
        );
        state.set_now_playing(A, SharedTimestamp(2), Some(Ed2kHash([1; 16])));
        state.set_playback_intent(A, SharedTimestamp(3), PlaybackIntent::Paused);
        state.set_anidb_metadata(
            A,
            SharedTimestamp(4),
            Ed2kHash([1; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Show".into(),
                series_id: Some(AniDbSeriesId(7)),
                episode_number: Some("1".into()),
            }),
        );
        let entry = link_series(&mut state, SharedTimestamp(4), AniDbSeriesId(7));
        if let Some(pref) = pref {
            state.set_series_preference(A, SharedTimestamp(5), user.clone(), entry, pref, None);
        }
        state.view()
    }

    /// A now-playing file whose series (id 7) the given user marked
    /// NotWatching — the auto-set state with no other UI escape hatch.
    fn not_watching_state(user: &UserId) -> StateView {
        now_playing_state_with_pref(user, Some(SeriesWatchState::NotWatching))
    }

    /// A now-playing file (series 7) that `user` is **committed** (Watching)
    /// to — so an absent `user` is a committed-absent blocker of it.
    fn committed_now_playing_state(user: &UserId) -> StateView {
        now_playing_state_with_pref(user, Some(SeriesWatchState::Watching))
    }

    fn peer_info(name: &str, presence: dessplay_core::net::Presence) -> PeerInfo {
        PeerInfo {
            username: UserId::new(name),
            role: dessplay_core::net::Role::Interactive,
            presence,
            addresses: vec![],
            connected_since: 0,
        }
    }

    /// `/ack` on a committed-but-absent blocker acknowledges each such
    /// blocker (a per-file one-shot) and latches playback intent Playing.
    #[test]
    fn ack_acknowledges_committed_absent_blockers_and_latches_playing() {
        use dessplay_core::net::Presence;
        let baughn = UserId::new("baughn");
        let mut ui = ui_with_view(committed_now_playing_state(&baughn));
        // baughn is committed to the now-playing series but has departed.
        ui.snapshot.peers = vec![
            peer_info("kim", Presence::Present),
            peer_info("baughn", Presence::Departed),
        ];
        let actions = ui.command("/ack");
        assert_eq!(
            mutations(&actions),
            vec![
                &Mutation::AcknowledgeAbsent {
                    file: Ed2kHash([1; 16]),
                    user: baughn,
                },
                &Mutation::SetPlaybackIntent {
                    intent: PlaybackIntent::Playing,
                },
            ]
        );
    }

    /// `/ack` when no one is a committed-absent blocker (everyone present) is
    /// a local notice with no mutations.
    #[test]
    fn ack_with_no_committed_absent_blockers_is_a_notice() {
        use dessplay_core::net::Presence;
        let baughn = UserId::new("baughn");
        let mut ui = ui_with_view(committed_now_playing_state(&baughn));
        ui.snapshot.peers = vec![
            peer_info("kim", Presence::Present),
            peer_info("baughn", Presence::Present),
        ];
        let actions = ui.command("/ack");
        assert!(
            mutations(&actions).is_empty(),
            "a present committed user is not a committed-absent blocker"
        );
        assert!(matches!(actions.as_slice(), [UserAction::Notice(_)]));
    }

    /// `/resync` emits the reset action plus a local feedback notice —
    /// no confirm modal, no mutation (the reset is not a CRDT write).
    #[test]
    fn resync_command_emits_reset_and_notice() {
        let mut ui = ui_with_view(StateView::default());
        let actions = ui.command("/resync");
        assert!(mutations(&actions).is_empty());
        assert!(
            matches!(
                actions.as_slice(),
                [UserAction::ResetSyncedState, UserAction::Notice(text)]
                    if text.contains("/resync")
            ),
            "expected reset + feedback notice, got {actions:?}"
        );
    }

    /// `/ack` with nothing playing is a local notice.
    #[test]
    fn ack_with_nothing_playing_is_a_notice() {
        let mut ui = ui_with_view(StateView::default());
        let actions = ui.command("/ack");
        assert!(mutations(&actions).is_empty());
        assert!(matches!(actions.as_slice(), [UserAction::Notice(_)]));
    }

    fn known_user(name: &str, last_seen: u64) -> dessplay_core::net::KnownUser {
        dessplay_core::net::KnownUser {
            username: UserId::new(name),
            last_seen,
        }
    }

    /// `/summon` with nobody known-offline is a local notice — nobody to
    /// page, no actor round trip.
    #[test]
    fn summon_with_nobody_offline_is_a_notice() {
        let mut ui = ui_with_view(StateView::default());
        ui.snapshot.known_offline = Vec::new();
        let actions = ui.command("/summon");
        assert!(matches!(actions.as_slice(), [UserAction::Notice(_)]));
    }

    /// `/summon` with the IRC bridge disabled is a local notice, decided
    /// entirely from settings — no `UserAction::Summon` reaches the actor.
    #[test]
    fn summon_with_irc_disabled_is_a_notice() {
        let settings = Settings {
            irc_enabled: false,
            ..Settings::default()
        };
        let mut ui = Ui::with_setup(me(), settings, vec![], false);
        ui.snapshot.view = std::sync::Arc::new(StateView::default());
        ui.snapshot.known_offline = vec![known_user("nero", 0)];
        let actions = ui.command("/summon");
        assert!(matches!(actions.as_slice(), [UserAction::Notice(_)]));
    }

    /// `/summon` with known-offline users emits `UserAction::Summon`
    /// carrying every one of their usernames — the IRC actor resolves the
    /// rest (design.md #4).
    #[test]
    fn summon_with_absent_users_emits_summon_action() {
        let mut ui = ui_with_view(StateView::default());
        ui.snapshot.known_offline = vec![known_user("nero", 0), known_user("kim", 0)];
        let actions = ui.command("/summon");
        assert_eq!(
            actions,
            vec![UserAction::Summon(vec![
                UserId::new("nero"),
                UserId::new("kim")
            ])]
        );
    }

    fn chat_msg(t: u64, who: &str, text: &str) -> dessplay_core::types::ChatMessage {
        dessplay_core::types::ChatMessage {
            timestamp: SharedTimestamp(t),
            sender: UserId::new(who),
            text: text.into(),
        }
    }

    fn intermixed_ui() -> Ui {
        let mut ui = ui_with_view(StateView::default());
        ui.subtitle_mode = SubtitleMode::Intermixed;
        ui
    }

    #[test]
    fn series_sort_initializes_from_settings() {
        // Regression: the All-Series sort must be seeded from the persisted
        // setting at startup, not always reset to Title (design.md: "Sort mode
        // for All Series is persisted across sessions").
        let settings = Settings {
            series_sort: props::SeriesSort::Year,
            ..Settings::default()
        };
        let ui = Ui::with_setup(me(), settings, vec![], false);
        assert_eq!(ui.series.sort(), props::SeriesSort::Year);
    }

    #[test]
    fn subtitle_pane_scrolls_back_with_wheel_only_over_it() {
        // Separate pane, more lines than fit. Wheel-up over the pane
        // reveals older lines (mouse-only: the pane is unfocusable);
        // wheel-down returns to live; over-scroll clamps at the oldest.
        let mut ui = ui_with_view(StateView::default());
        ui.subtitle_mode = SubtitleMode::SeparatePane;
        for i in 0..40u64 {
            ui.push_subtitle(1_000 * i, i, format!("line{i:02}"), None);
        }
        let buffer = render_test_buffer(&mut ui);
        let subs = ui.panes.subs;
        assert!(subs.height > 2, "pane was drawn");
        assert!(buffer_contains(&buffer, "line39"), "live view shows newest");
        let wheel = |kind, column, row| {
            Event::Mouse(MouseEvent {
                kind,
                modifiers: KeyModifiers::NONE,
                column,
                row,
            })
        };
        // Wheel over the chat column above the pane: chat's business,
        // the subtitle pane stays live.
        ui.handle(wheel(
            MouseEventKind::ScrollUp,
            subs.x + 2,
            ui.panes.chat.y + 1,
        ));
        assert_eq!(ui.subtitle_scroll, 0);
        // Wheel over the pane: scrolls back regardless of focus.
        ui.handle(wheel(MouseEventKind::ScrollUp, subs.x + 2, subs.y + 1));
        let buffer = render_test_buffer(&mut ui);
        assert!(!buffer_contains(&buffer, "line39"), "newest scrolled off");
        assert!(buffer_contains(&buffer, "line36"), "older line revealed");
        assert!(
            buffer_contains(&buffer, "Subtitles (-3)"),
            "title shows offset"
        );
        // Over-scroll clamps to the oldest entry.
        for _ in 0..100 {
            ui.handle(wheel(MouseEventKind::ScrollUp, subs.x + 2, subs.y + 1));
        }
        let buffer = render_test_buffer(&mut ui);
        assert!(buffer_contains(&buffer, "line00"), "oldest reachable");
        let visible = subs.height as usize - 2;
        assert_eq!(ui.subtitle_scroll, 40 - visible);
        // Wheel-down all the way returns to live.
        for _ in 0..100 {
            ui.handle(wheel(MouseEventKind::ScrollDown, subs.x + 2, subs.y + 1));
        }
        let buffer = render_test_buffer(&mut ui);
        assert!(buffer_contains(&buffer, "line39"));
        assert_eq!(ui.subtitle_scroll, 0);
        // Hiding the pane forgets the scroll-back.
        ui.handle(wheel(MouseEventKind::ScrollUp, subs.x + 2, subs.y + 1));
        ui.subtitle_mode = SubtitleMode::Off;
        render_test_buffer(&mut ui);
        assert_eq!(ui.subtitle_scroll, 0);
    }

    #[test]
    fn subtitle_empty_line_is_skipped() {
        let mut ui = intermixed_ui();
        ui.push_subtitle(1000, 5, String::new(), None);
        assert!(ui.subtitles.is_empty());
    }

    #[test]
    fn subtitle_incremental_reveal_collapses_to_one() {
        let mut ui = intermixed_ui();
        // Each cue is a longer prefix of the next; the first cue's
        // timestamps win.
        ui.push_subtitle(1000, 10, "H".into(), SpeakerName::new("Frieren"));
        ui.push_subtitle(1100, 11, "He".into(), SpeakerName::new("Frieren"));
        ui.push_subtitle(1200, 12, "Hello".into(), SpeakerName::new("Frieren"));
        assert_eq!(ui.subtitles.len(), 1);
        let entry = ui.subtitles.back().unwrap();
        assert_eq!(entry.text, "Hello");
        assert_eq!(entry.video_millis, 1000);
        assert_eq!(entry.arrival_millis, 10);
        // The speaker tracks the latest cue in the collapsed reveal.
        assert_eq!(entry.speaker.as_deref(), Some("Frieren"));
    }

    #[test]
    fn collapsed_subtitle_updates_still_track_every_named_speaker() {
        let mut ui = intermixed_ui();
        ui.push_subtitle(1_000, 10, "H".into(), SpeakerName::new("First"));
        ui.push_subtitle(1_100, 11, "He".into(), SpeakerName::new("Second"));

        assert_eq!(ui.subtitles.len(), 1, "the reveal still collapses");
        assert_eq!(ui.speaker_colors.len(), 2);
        assert_eq!(ui.subtitles.back().unwrap().speaker_slot, Some(1));
    }

    #[test]
    fn subtitle_exact_duplicate_collapses() {
        let mut ui = intermixed_ui();
        ui.push_subtitle(1000, 10, "same".into(), None);
        ui.push_subtitle(2000, 20, "same".into(), None);
        assert_eq!(ui.subtitles.len(), 1);
        assert_eq!(ui.subtitles.back().unwrap().video_millis, 1000);
    }

    #[test]
    fn subtitle_non_prefix_appends() {
        let mut ui = intermixed_ui();
        ui.push_subtitle(1000, 10, "Hello".into(), None);
        ui.push_subtitle(2000, 20, "World".into(), None);
        assert_eq!(ui.subtitles.len(), 2);
    }

    #[test]
    fn subtitle_multiline_cue_joins_with_a_space() {
        // mpv joins a two-line cue with a newline; we render one line, so
        // it must read "you demons", not "youdemons" (the reported bug).
        let mut ui = intermixed_ui();
        ui.push_subtitle(
            1000,
            10,
            "I won't let you\ndemons have your way".into(),
            None,
        );
        assert_eq!(ui.subtitles.len(), 1);
        assert_eq!(
            ui.subtitles.back().unwrap().text,
            "I won't let you demons have your way"
        );
    }

    #[test]
    fn subtitle_second_line_growth_collapses_with_a_space() {
        // A two-line cue revealed incrementally: first line completes,
        // then the second grows. Newline-normalization keeps the prefix
        // relation intact and inserts the space.
        let mut ui = intermixed_ui();
        ui.push_subtitle(1000, 10, "I won't let you".into(), None);
        ui.push_subtitle(1100, 11, "I won't let you\nd".into(), None);
        ui.push_subtitle(1200, 12, "I won't let you\ndemons".into(), None);
        assert_eq!(ui.subtitles.len(), 1);
        assert_eq!(ui.subtitles.back().unwrap().text, "I won't let you demons");
    }

    #[test]
    fn subtitle_overlapping_cue_shrink_does_not_duplicate() {
        // Reproduces SL2_Episode-141 at ~03:19: a stable line is on screen
        // when a brief interjection ("Coming!") overlaps it. mpv reports the
        // two simultaneous ASS events as one combined cue; when the
        // interjection ends it re-emits the stable line *alone*, so the
        // combined text shrinks back to a prefix of itself. That shrink is
        // the same cue receding -- it must not append a duplicate line.
        let mut ui = intermixed_ui();
        let main = "that your greatest wish was to reforge Tang Sect's glory.";
        let combined = "that your greatest wish was to reforge Tang Sect's glory. Coming!";
        ui.push_subtitle(1000, 10, main.into(), None); // cue 67 alone
        ui.push_subtitle(1100, 11, combined.into(), None); // 67 + "Coming!"
        ui.push_subtitle(1200, 12, main.into(), None); // "Coming!" gone, 67 alone
        assert_eq!(ui.subtitles.len(), 1, "shrink-back must not duplicate");
        // The fullest text seen is retained as the single history line.
        assert_eq!(ui.subtitles.back().unwrap().text, combined);
        assert_eq!(ui.subtitles.back().unwrap().video_millis, 1000);
    }

    #[test]
    fn subtitle_overlapping_cue_shrink_from_front_does_not_duplicate() {
        // The mirror of the above: mpv can order overlapping events either
        // way, so the disappearing neighbour may sit at the *front* of the
        // joined text. The shrink then leaves a suffix of what was shown;
        // still the same cue receding, still no new line.
        let mut ui = intermixed_ui();
        let main = "that your greatest wish";
        let combined = "Coming! that your greatest wish";
        ui.push_subtitle(1000, 10, "Coming!".into(), None);
        ui.push_subtitle(1100, 11, combined.into(), None);
        ui.push_subtitle(1200, 12, main.into(), None);
        assert_eq!(ui.subtitles.len(), 1, "suffix shrink must not duplicate");
        assert_eq!(ui.subtitles.back().unwrap().text, combined);
    }

    #[test]
    fn subtitle_log_caps_at_100() {
        let mut ui = intermixed_ui();
        for i in 0..150 {
            ui.push_subtitle(i, i, format!("line {i}"), None);
        }
        assert_eq!(ui.subtitles.len(), 100);
        // Oldest dropped: the front is line 50.
        assert_eq!(ui.subtitles.front().unwrap().text, "line 50");
    }

    // ---- Marquee (design.md, AI Commentary) ----------------------------

    /// Regression (2026-08-12 review, retargeted 2026-08-20): clicks
    /// are stamped from `Ui::clock`, so spoiler frames must advance on
    /// that same clock. The original bug fed animators one domain and
    /// stamped clicks from another; today a single monotonic animator
    /// domain makes that unrepresentable, and the residual guard is
    /// that an out-of-order (stale) tick value must never rewind a
    /// running tease's generation to 0 — generation-0 letters look
    /// unclicked.
    #[test]
    fn spoiler_tease_ignores_a_stale_tick() {
        let mut ui = Ui::with_setup(me(), Settings::default(), vec![], false);
        // The shell freshens the clock before every input.
        ui.advance_clock(30_000);
        ui.apply_snapshot(UiSnapshot {
            now: 30_000,
            ..UiSnapshot::default()
        });
        ui.chat.test_install_spoiler_hit();
        ui.chat.click(6, 1, ui.clock); // the dispatcher's exact stamping
        assert_eq!(ui.chat.test_spoiler_generations(), vec![0]);
        // The next pre-input freshen advances the tease (snapshots
        // arrive at ~10Hz during playback).
        ui.advance_clock(30_250);
        ui.apply_snapshot(UiSnapshot {
            now: 30_250,
            ..UiSnapshot::default()
        });
        assert_eq!(ui.chat.test_spoiler_generations(), vec![2]);
        // A stale tick value must not rewind the animation.
        ui.advance_clock(300);
        assert_eq!(ui.chat.test_spoiler_generations(), vec![2]);
    }

    /// Regression (2026-08-20 review): `Ui::clock` merged the wall and
    /// shared clock domains with `max` and so latched at the historic
    /// maximum — but both domains can step backward (a local NTP
    /// correction; a later ClockSync shrinking the offset behind
    /// `shared_now`). After a backward step every animator computed an
    /// unchanged "now" and froze for exactly the size of the step.
    /// Animators run on the shell's monotonic clock; no snapshot stamp
    /// may drag it.
    #[test]
    fn spoiler_tease_survives_a_backward_shared_clock_step() {
        let mut ui = Ui::with_setup(me(), Settings::default(), vec![], false);
        // Shell tick, then a snapshot whose shared clock leads wall by
        // 90 s (a large positive clock offset).
        ui.advance_clock(10_000);
        ui.apply_snapshot(UiSnapshot {
            now: 10_000,
            shared_now: 100_000,
            ..UiSnapshot::default()
        });
        ui.chat.test_install_spoiler_hit();
        ui.chat.click(6, 1, ui.clock); // the dispatcher's exact stamping
        assert_eq!(ui.chat.test_spoiler_generations(), vec![0]);
        // ClockSync corrects the offset: the next snapshot's shared
        // clock is back at wall.
        ui.advance_clock(10_100);
        ui.apply_snapshot(UiSnapshot {
            now: 10_100,
            shared_now: 10_100,
            ..UiSnapshot::default()
        });
        // The tick 250 ms after the click: the running tease must still
        // advance.
        assert!(ui.advance_clock(10_250), "tease frame wants a repaint");
        assert_eq!(ui.chat.test_spoiler_generations(), vec![2]);
    }

    /// The marquee variant of the backward-step regression: a pass
    /// started while `shared_now` led wall must keep scrolling after
    /// the offset collapses.
    #[test]
    fn marquee_pass_survives_a_backward_shared_clock_step() {
        let mut ui = ui_with_view(StateView::default());
        ui.advance_clock(10_000);
        let mut snapshot = marquee_snapshot("<Amu> Whaaaat?", 100_000, 10_000);
        snapshot.shared_now = 100_000; // fresh in the leading shared domain
        ui.apply_snapshot(snapshot);
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_millis(100));
        // Offset correction: shared falls back to wall. Same stamp — no
        // restart.
        ui.advance_clock(10_100);
        let mut snapshot = marquee_snapshot("<Amu> Whaaaat?", 100_000, 10_100);
        snapshot.shared_now = 10_100;
        ui.apply_snapshot(snapshot);
        // One second into the pass the text must have moved.
        assert!(ui.advance_clock(11_000), "the pass still animates");
    }

    fn marquee_snapshot(text: &str, stamp: u64, now: u64) -> UiSnapshot {
        let view = StateView {
            marquee: Some((
                SharedTimestamp(stamp),
                dessplay_core::types::MarqueeMessage {
                    text: text.into(),
                    set_by: None,
                },
            )),
            ..StateView::default()
        };
        UiSnapshot {
            view: std::sync::Arc::new(view),
            now,
            // The two domains coincide unless a test separates them.
            shared_now: now,
            ..UiSnapshot::default()
        }
    }

    /// Does any row of the rendered buffer contain `needle`?
    fn buffer_contains(buffer: &tuirealm::ratatui::buffer::Buffer, needle: &str) -> bool {
        (0..buffer.area.height).any(|y| {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            row.contains(needle)
        })
    }

    fn bottom_row(buffer: &tuirealm::ratatui::buffer::Buffer) -> String {
        // 100x30 test terminal: status bar 3 + keybar 1 leave rows 0..=25
        // for the main area; the terminal-wide bottom line is row 25.
        (0..buffer.area.width)
            .map(|x| buffer[(x, 25)].symbol())
            .collect()
    }

    #[test]
    fn marquee_scrolls_in_from_off_screen_and_runs_once_per_stamp() {
        let mut ui = ui_with_view(StateView::default());
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 1_000, 1_000));
        assert_eq!(
            ui.next_tick_hint(),
            std::time::Duration::from_millis(100),
            "an animating marquee wants the fast tick"
        );
        // Offset 0: entirely off-screen right — the entry delay is the
        // "glance down" affordance.
        let buffer = render_test_buffer(&mut ui);
        assert!(!bottom_row(&buffer).contains("Whaaaat?"), "not yet visible");

        // 1s in (15 cells): fully entered, hugging the right of the slot.
        assert!(ui.advance_clock(2_000), "movement wants a repaint");
        let row = bottom_row(&render_test_buffer(&mut ui));
        assert!(row.contains("<Amu> Whaaaat?"), "entered: {row}");
        let early = row.find("<Amu>").unwrap();

        // Another second: strictly further left.
        assert!(ui.advance_clock(3_000));
        let row = bottom_row(&render_test_buffer(&mut ui));
        let later = row.find("<Amu>").unwrap();
        assert!(later < early, "scrolls right-to-left ({early} -> {later})");

        // Long after: fully exited, done latches, tick relaxes, and the
        // same stamp never replays — even across fresh snapshots.
        assert!(ui.advance_clock(60_000));
        let row = bottom_row(&render_test_buffer(&mut ui));
        assert!(!row.contains("Whaaaat?"), "exited: {row}");
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_secs(1));
        assert!(!ui.advance_clock(61_000), "a done pass never repaints");
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 1_000, 62_000));
        assert_eq!(
            ui.next_tick_hint(),
            std::time::Duration::from_secs(1),
            "same stamp: no replay"
        );

        // A new stamp — even with identical text — replays from the top.
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 62_000, 62_000));
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_millis(100));

        // A cleared register drops the animation outright.
        ui.apply_snapshot(UiSnapshot::default());
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_secs(1));
    }

    #[test]
    fn marquee_stamped_before_startup_never_plays() {
        // The marquee register survives in synced state across sessions
        // (cleared only at compaction), so the first snapshot after
        // startup can carry last night's comment. A stamp from before
        // this session's first snapshot must not replay; a fresh write
        // still does.
        let mut ui = ui_with_view(StateView::default());
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 500, 3_600_000));
        assert_eq!(
            ui.next_tick_hint(),
            std::time::Duration::from_secs(1),
            "a pre-startup stamp never animates"
        );
        assert!(!ui.advance_clock(3_601_000), "nothing to animate");
        let row = bottom_row(&render_test_buffer(&mut ui));
        assert!(!row.contains("Whaaaat?"), "stale comment stays off: {row}");

        // A fresh write this session plays normally.
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 3_602_000, 3_602_000));
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_millis(100));
    }

    /// Regression (2026-08-12 review): the staleness guard compares the
    /// marquee's **shared-clock** LWW stamp against the first snapshot's
    /// "now" — which used to be raw wall-clock. A wall clock leading the
    /// group by N suppressed every marquee written in the first N ms; a
    /// lagging one replayed last night's final comment. The guard must
    /// seed from the snapshot's shared clock.
    #[test]
    fn marquee_staleness_guard_uses_the_shared_clock_not_wall() {
        // Wall clock 95s ahead of the group: a comment stamped "now" in
        // shared time sits behind snapshot.now, but it is fresh and
        // must play.
        let mut ui = ui_with_view(StateView::default());
        let mut snapshot = marquee_snapshot("<Amu> Whaaaat?", 5_000, 100_000);
        snapshot.shared_now = 4_000;
        ui.apply_snapshot(snapshot);
        assert_eq!(
            ui.next_tick_hint(),
            std::time::Duration::from_millis(100),
            "a stamp fresh by the shared clock plays, whatever the wall says"
        );

        // The true-stale case still never plays: a stamp from before
        // this session's first snapshot (in shared time) is last
        // night's leftover.
        let mut ui = ui_with_view(StateView::default());
        let mut snapshot = marquee_snapshot("<Amu> Whaaaat?", 500, 100_000);
        snapshot.shared_now = 3_600_000;
        ui.apply_snapshot(snapshot);
        assert_eq!(
            ui.next_tick_hint(),
            std::time::Duration::from_secs(1),
            "a pre-startup stamp stays stale under the shared clock too"
        );
    }

    #[test]
    fn marquee_chat_mode_logs_one_chat_line_instead_of_scrolling() {
        let mut ui = ui_with_view(StateView::default());
        ui.settings.marquee_mode = MarqueeMode::Chat;
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 1_000, 1_000));
        assert_eq!(
            ui.next_tick_hint(),
            std::time::Duration::from_secs(1),
            "chat mode never animates"
        );
        assert!(!ui.advance_clock(2_000), "nothing to animate");
        let buffer = render_test_buffer(&mut ui);
        assert!(!bottom_row(&buffer).contains("Whaaaat?"), "no scroll pass");
        let all: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        assert!(
            all.contains("<Amu> Whaaaat?"),
            "the line is in the chat log"
        );

        // The same stamp re-arriving (snapshots are frequent) does not
        // duplicate the line; a fresh stamp logs again.
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 1_000, 2_000));
        assert_eq!(ui.system_log.len(), 1, "same stamp logs once");
        ui.apply_snapshot(marquee_snapshot("<Amu> Weeeell.", 3_000, 3_000));
        assert_eq!(ui.system_log.len(), 2, "a new stamp logs a new line");
    }

    #[test]
    fn marquee_chat_mode_skips_pre_startup_leftovers() {
        let mut ui = ui_with_view(StateView::default());
        ui.settings.marquee_mode = MarqueeMode::Chat;
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 500, 3_600_000));
        assert!(
            ui.system_log.is_empty(),
            "last night's comment is not replayed into chat"
        );
    }

    #[test]
    fn marquee_off_mode_shows_nothing_and_never_replays() {
        let mut ui = ui_with_view(StateView::default());
        ui.settings.marquee_mode = MarqueeMode::Off;
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 1_000, 1_000));
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_secs(1));
        assert!(ui.system_log.is_empty(), "no chat line either");
        let buffer = render_test_buffer(&mut ui);
        assert!(!bottom_row(&buffer).contains("Whaaaat?"));

        // Flipping back to Marquee does not replay the adopted stamp —
        // only a fresh write plays.
        ui.settings.marquee_mode = MarqueeMode::Marquee;
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 1_000, 2_000));
        assert_eq!(
            ui.next_tick_hint(),
            std::time::Duration::from_secs(1),
            "adopted stamp stays done"
        );
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 3_000, 3_000));
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_millis(100));
    }

    /// A playing snapshot with a live marquee: now-playing with a
    /// duration, our own position sample (the 47-cell progress text),
    /// link Connected with a health sample (~32 cells of metrics on
    /// the right).
    fn playing_marquee_snapshot() -> UiSnapshot {
        use crate::ui::props::{HealthProps, HealthSample};
        use dessplay_core::types::PlaybackPosition;

        let hash = Ed2kHash([1; 16]);
        let mut state = CrdtState::new();
        state.push_playlist_entry(
            A,
            SharedTimestamp(1),
            dessplay_core::playlist::NewPlaylistEntry {
                hash,
                added_by: UserId::new("baughn"),
                filename: "ep1.mkv".into(),
                size_bytes: 1,
                duration_millis: NonZeroU64::new(1_440_000),
            },
        );
        state.set_now_playing(A, SharedTimestamp(2), Some(hash));
        state.set_playback_position(
            A,
            SharedTimestamp(3),
            me(),
            PlaybackPosition {
                position_millis: 754_000,
                timestamp: SharedTimestamp(3),
                file: hash,
            },
        );
        let mut view = state.view();
        view.marquee = Some((
            SharedTimestamp(1_000),
            dessplay_core::types::MarqueeMessage {
                text: "<Amu> Whaaaat?".into(),
                set_by: None,
            },
        ));
        let sample = HealthSample {
            rtt_millis: Some(89),
            unanswered_probes: 0,
            server_silence_millis: 0,
            up_bps: 1_200_000,
            down_bps: 340_000,
        };
        UiSnapshot {
            view: std::sync::Arc::new(view),
            now: 1_000,
            link: props::LinkStatus::Connected,
            health: HealthProps {
                link: props::LinkStatus::Connected,
                sample: Some(sample),
                ..HealthProps::default()
            },
            ..UiSnapshot::default()
        }
    }

    /// Regression (2026-08-12 review, spec-drift): the progress bar
    /// used to reserve everything but two cells, so at 80 columns with
    /// a file playing the marquee never got a single cell — design.md's
    /// truncation order (health > progress > slot) says the *bar*
    /// yields first. The pass must scroll through a real window while
    /// the bar renders truncated beside it.
    #[test]
    fn marquee_gets_scroll_space_over_a_full_progress_bar() {
        let mut ui = Ui::with_setup(me(), Settings::default(), vec![], false);
        // The shell freshens the clock before every input.
        ui.advance_clock(1_000);
        ui.apply_snapshot(playing_marquee_snapshot());
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_millis(100));

        // 1s in (15 cells): the line has entered the slot.
        assert!(ui.advance_clock(2_000));
        let buffer = render_buffer_at(&mut ui, 80, 30);
        let bottom: String = (0..buffer.area.width)
            .map(|x| buffer[(x, 25)].symbol())
            .collect();
        assert!(bottom.contains("sync ok"), "health metrics drawn: {bottom}");
        assert!(bottom.contains("Whaaa"), "marquee cells visible: {bottom}");
        assert!(
            !bottom.contains("12:34"),
            "the bar truncated to make room: {bottom}"
        );
        let slot = ui.marquee.as_ref().unwrap().slot_width;
        assert!(
            slot.is_some_and(|width| width > 0),
            "a real scroll window was measured: {slot:?}"
        );
    }

    /// Regression (2026-08-12 review): on a terminal too narrow to give
    /// the slot its 8-cell minimum even after the progress bar yields
    /// (health ~32 cells; 36 columns leave 4), the middle slot is
    /// genuinely zero-width — a real measurement, not "not yet
    /// measured". The done-latch used to require `slot_width > 0`, so
    /// the pass never terminated and `next_tick_hint()` pinned the
    /// shell to 10 Hz full-screen repaints until compaction or a
    /// restart.
    #[test]
    fn marquee_pass_terminates_on_a_zero_width_slot() {
        let mut ui = Ui::with_setup(me(), Settings::default(), vec![], false);
        ui.apply_snapshot(playing_marquee_snapshot());
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_millis(100));

        // Draw at 36 columns: the slot measures as zero. Sanity-check
        // the collapse arithmetic so this test can't pass vacuously on
        // a roomy slot.
        let buffer = render_buffer_at(&mut ui, 36, 30);
        let bottom: String = (0..buffer.area.width)
            .map(|x| buffer[(x, 25)].symbol())
            .collect();
        assert!(bottom.contains("sync ok"), "health metrics drawn: {bottom}");
        assert!(
            !bottom.contains("Whaaa"),
            "no room for any marquee cell: {bottom}"
        );
        assert_eq!(
            ui.marquee.as_ref().unwrap().slot_width,
            Some(0),
            "the zero slot is a real measurement"
        );

        // A minute later the pass is long over; the tick hint must be
        // back to the lazy 1s even though the text never fit on screen.
        assert!(ui.advance_clock(61_000));
        assert_eq!(
            ui.next_tick_hint(),
            std::time::Duration::from_secs(1),
            "a zero-width slot must still terminate the pass"
        );
    }

    #[test]
    fn saving_a_non_marquee_mode_stops_a_pass_mid_scroll() {
        let mut ui = ui_with_view(StateView::default());
        ui.apply_snapshot(marquee_snapshot("<Amu> Whaaaat?", 1_000, 1_000));
        assert_eq!(ui.next_tick_hint(), std::time::Duration::from_millis(100));
        let mut settings = ui.settings.clone();
        settings.marquee_mode = MarqueeMode::Off;
        let roots = ui.media_roots.clone();
        ui.update(Msg::SettingsSaved(Box::new(settings), roots));
        assert_eq!(
            ui.next_tick_hint(),
            std::time::Duration::from_secs(1),
            "the pass stops at once"
        );
        let row = bottom_row(&render_test_buffer(&mut ui));
        assert!(!row.contains("Whaaaat?"), "slot reverts: {row}");
    }

    #[test]
    fn health_warning_owns_the_slot_over_a_live_marquee() {
        use crate::ui::props::{SuggestionProps, Tone};

        let mut ui = ui_with_view(StateView::default());
        let mut snapshot = marquee_snapshot("<Amu> Whaaaat?", 1_000, 1_000);
        snapshot.health.suggestion = Some(SuggestionProps {
            text: "high latency — disable BitTorrent (F3, applies immediately)".into(),
            tone: Tone::Paused,
        });
        ui.apply_snapshot(snapshot);
        ui.advance_clock(3_000); // marquee would be mid-slot by now
        let row = bottom_row(&render_test_buffer(&mut ui));
        assert!(
            row.contains("high latency"),
            "the warning owns the slot: {row}"
        );
        assert!(!row.contains("Whaaaat?"), "marquee yields to warnings");

        // An Info suggestion yields to the marquee instead.
        let mut snapshot = marquee_snapshot("<Amu> Whaaaat?", 1_000, 1_000);
        snapshot.health.suggestion = Some(SuggestionProps {
            text: "state diverged — resyncing".into(),
            tone: Tone::Muted,
        });
        let mut ui = ui_with_view(StateView::default());
        ui.apply_snapshot(snapshot);
        ui.advance_clock(3_000);
        let row = bottom_row(&render_test_buffer(&mut ui));
        assert!(row.contains("Whaaaat?"), "marquee over an Info note: {row}");
    }

    fn render_test_buffer(ui: &mut Ui) -> tuirealm::ratatui::buffer::Buffer {
        render_buffer_at(ui, 100, 30)
    }

    fn render_buffer_at(ui: &mut Ui, width: u16, height: u16) -> tuirealm::ratatui::buffer::Buffer {
        use tuirealm::ratatui::Terminal;
        use tuirealm::ratatui::backend::TestBackend;

        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui.draw(frame))
            .unwrap()
            .buffer
            .clone()
    }

    fn rendered_text_color(
        buffer: &tuirealm::ratatui::buffer::Buffer,
        needle: &str,
    ) -> tuirealm::ratatui::style::Color {
        let first = needle.chars().next().expect("non-empty test needle");
        for y in 0..buffer.area.height {
            let line: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            if line.contains(needle) {
                return (0..buffer.area.width)
                    .map(|x| &buffer[(x, y)])
                    .find(|cell| cell.symbol().starts_with(first))
                    .expect("subtitle text cell")
                    .fg;
            }
        }
        panic!("{needle:?} not found in render");
    }

    fn rendered_text_style(
        buffer: &tuirealm::ratatui::buffer::Buffer,
        needle: &str,
    ) -> (
        tuirealm::ratatui::style::Color,
        tuirealm::ratatui::style::Modifier,
    ) {
        let first = needle.chars().next().expect("non-empty test needle");
        for y in 0..buffer.area.height {
            let line: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            if line.contains(needle) {
                let cell = (0..buffer.area.width)
                    .map(|x| &buffer[(x, y)])
                    .find(|cell| cell.symbol().starts_with(first))
                    .expect("rendered text cell");
                return (cell.fg, cell.modifier);
            }
        }
        panic!("{needle:?} not found in render");
    }

    /// Regression: VTE-based terminals can fail to visibly apply SGR 2 to
    /// explicit RGB colors. A watched playlist row in true-color mode must
    /// therefore leave the completed frame with a concrete muted foreground,
    /// not a terminal-dependent DIM attribute.
    #[test]
    fn truecolor_watched_playlist_row_uses_explicit_muted_foreground() {
        use dessplay_core::playlist::NewPlaylistEntry;
        use tuirealm::ratatui::style::Modifier;

        let file = Ed2kHash([42; 16]);
        let mut state = CrdtState::new();
        state.push_playlist_entry(
            A,
            SharedTimestamp(1),
            NewPlaylistEntry {
                hash: file,
                added_by: UserId::new("baughn"),
                filename: "watched-regression.mkv".into(),
                size_bytes: 1,
                duration_millis: None,
            },
        );
        state.set_watched(A, SharedTimestamp(2), file, true);

        let mut ui = Ui::with_setup(me(), Settings::default(), vec![], false);
        ui.apply_snapshot(UiSnapshot {
            view: std::sync::Arc::new(state.view()),
            ..Default::default()
        });
        ui.set_color_depth(ColorDepth::TrueColor);

        let buffer = render_test_buffer(&mut ui);
        let (foreground, modifier) = rendered_text_style(&buffer, "watched-regression.mkv");

        assert_eq!(foreground, crate::ui::theme::TRUECOLOR_MUTED_FOREGROUND);
        assert!(!modifier.contains(Modifier::DIM));
    }

    /// Regression: once RGB is available, every cell belongs to DessPlay's
    /// own dark theme instead of inheriting an arbitrary terminal background.
    #[test]
    fn truecolor_renders_the_entire_app_in_dark_mode() {
        let mut ui = ui_with_view(StateView::default());
        ui.set_color_depth(ColorDepth::TrueColor);

        let buffer = render_test_buffer(&mut ui);

        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| cell.bg == crate::ui::theme::TRUECOLOR_BACKGROUND),
            "every true-color cell should use the app background"
        );
    }

    /// Regression: an RGB terminal is not constrained by the finite ANSI
    /// speaker palette. Even after that many active speakers, visible cues
    /// receive distinct generated RGB colors.
    #[test]
    fn truecolor_speaker_colors_continue_past_the_limited_palette() {
        let mut ui = ui_with_view(StateView::default());
        ui.subtitle_mode = SubtitleMode::SeparatePane;
        ui.set_color_depth(ColorDepth::TrueColor);
        let count = crate::ui::theme::LIMITED_SPEAKER_CAPACITY + 5;
        for index in 0..count {
            let label = char::from(b'A' + index as u8);
            ui.push_subtitle(
                index as u64,
                index as u64,
                format!("{label} utterance"),
                SpeakerName::new(format!("speaker-{index}")),
            );
        }

        let buffer = render_test_buffer(&mut ui);
        let visible = (count - 5..count)
            .map(|index| {
                let label = char::from(b'A' + index as u8);
                rendered_text_color(&buffer, &format!("{label} utterance"))
            })
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(visible.len(), 5, "visible speakers should not share colors");
        assert!(
            visible
                .iter()
                .all(|color| matches!(color, tuirealm::ratatui::style::Color::Rgb(..))),
            "true-color speakers should use generated RGB colors: {visible:?}"
        );
    }

    /// Regression: limited terminals may remove speaker identity when their
    /// finite palette overflows. The active set is an inclusive rolling five
    /// minutes: the boundary cue is uncolored, then colors return once every
    /// original speaker is more than five minutes old.
    #[test]
    fn limited_overflow_disable_colors_uses_a_five_minute_window() {
        use crate::config::SubtitleSpeakerOverflow;

        let settings = Settings {
            subtitle_mode: SubtitleMode::SeparatePane,
            subtitle_speaker_overflow: SubtitleSpeakerOverflow::DisableColors,
            ..Settings::default()
        };
        let mut ui = Ui::with_setup(me(), settings, vec![], false);
        ui.subtitle_mode = SubtitleMode::SeparatePane;
        let base = 1_000;
        for index in 0..crate::ui::theme::LIMITED_SPEAKER_CAPACITY {
            ui.push_subtitle(
                index as u64,
                base + index as u64,
                format!("{index} original"),
                SpeakerName::new(format!("speaker-{index}")),
            );
        }
        let boundary = base + crate::ui::theme::SPEAKER_WINDOW_MILLIS;
        ui.push_subtitle(
            20_000,
            boundary,
            "Z boundary".into(),
            SpeakerName::new("boundary"),
        );

        let overflow = render_test_buffer(&mut ui);
        assert_eq!(
            rendered_text_color(&overflow, "Z boundary"),
            crate::ui::theme::dim().fg.unwrap(),
            "speaker identity should be removed while the palette is over capacity"
        );

        let after_expiry = base
            + crate::ui::theme::LIMITED_SPEAKER_CAPACITY as u64
            + crate::ui::theme::SPEAKER_WINDOW_MILLIS;
        assert!(ui.advance_clock(after_expiry));
        let recovered = render_test_buffer(&mut ui);
        assert_ne!(
            rendered_text_color(&recovered, "Z boundary"),
            crate::ui::theme::dim().fg.unwrap(),
            "speaker colors should return during a quiet scene once the old active set expires"
        );
    }

    #[test]
    fn limited_overflow_reuse_colors_preserves_colored_speaker_identity() {
        let settings = Settings {
            subtitle_mode: SubtitleMode::SeparatePane,
            subtitle_speaker_overflow: SubtitleSpeakerOverflow::ReuseColors,
            ..Settings::default()
        };
        let mut ui = Ui::with_setup(me(), settings, vec![], false);
        ui.subtitle_mode = SubtitleMode::SeparatePane;
        for index in 0..=crate::ui::theme::LIMITED_SPEAKER_CAPACITY {
            ui.push_subtitle(
                index as u64,
                index as u64,
                format!("R{index} reuse"),
                SpeakerName::new(format!("speaker-{index}")),
            );
        }

        let buffer = render_test_buffer(&mut ui);
        assert_eq!(
            rendered_text_color(
                &buffer,
                &format!("R{} reuse", crate::ui::theme::LIMITED_SPEAKER_CAPACITY),
            ),
            crate::ui::theme::user_style(&format!(
                "speaker-{}",
                crate::ui::theme::LIMITED_SPEAKER_CAPACITY
            ))
            .fg
            .unwrap()
        );
    }

    #[test]
    fn separate_pane_renders_newest_on_top_colored_by_speaker() {
        use tuirealm::ratatui::Terminal;
        use tuirealm::ratatui::backend::TestBackend;

        let mut ui = ui_with_view(StateView::default());
        ui.subtitle_mode = SubtitleMode::SeparatePane;
        // Three distinct cues, oldest first; the third names a speaker.
        ui.push_subtitle(1000, 10, "oldest".into(), None);
        ui.push_subtitle(2000, 20, "middle".into(), None);
        ui.push_subtitle(3000, 30, "newest".into(), SpeakerName::new("Frieren"));

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let buffer = terminal
            .draw(|frame| ui.draw(frame))
            .unwrap()
            .buffer
            .clone();

        // Find each line's row by scanning for its text; the pane is the
        // lower-left quarter of the frame.
        let row_of = |needle: &str| -> u16 {
            for y in 0..buffer.area.height {
                let mut line = String::new();
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                if line.contains(needle) {
                    return y;
                }
            }
            panic!("{needle:?} not found in render");
        };
        let (newest, middle, oldest) = (row_of("newest"), row_of("middle"), row_of("oldest"));
        // Feature 1: newest is on top (smallest y), oldest at the bottom.
        assert!(
            newest < middle && middle < oldest,
            "expected newest-on-top order, got newest={newest} middle={middle} oldest={oldest}"
        );

        // Feature 2: a limited terminal retains the existing deterministic
        // name hash into the app palette; the timestamp prefix stays dim.
        let want = crate::ui::theme::user_style("Frieren").fg.unwrap();
        let y = newest;
        let n_cell = (0..buffer.area.width)
            .map(|x| &buffer[(x, y)])
            .find(|c| c.symbol() == "n")
            .expect("subtitle text cell");
        assert_eq!(n_cell.fg, want, "subtitle text should be speaker-colored");
        // A digit from the MM:SS prefix is dim, not the speaker color.
        let prefix_cell = (0..buffer.area.width)
            .map(|x| &buffer[(x, y)])
            .find(|c| c.symbol() == "0")
            .expect("timestamp digit cell");
        assert_eq!(prefix_cell.fg, crate::ui::theme::dim().fg.unwrap());
    }

    /// `subtitle_speaker_colors = false` (design.md #22): every
    /// separate-pane line renders uniformly dim, even one with a known
    /// speaker.
    #[test]
    fn separate_pane_speaker_colors_off_renders_uniformly_dim() {
        use tuirealm::ratatui::Terminal;
        use tuirealm::ratatui::backend::TestBackend;

        let mut ui = ui_with_view(StateView::default());
        ui.subtitle_mode = SubtitleMode::SeparatePane;
        ui.settings.subtitle_speaker_colors = false;
        ui.push_subtitle(1000, 10, "newest".into(), SpeakerName::new("Frieren"));

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let buffer = terminal
            .draw(|frame| ui.draw(frame))
            .unwrap()
            .buffer
            .clone();

        let row_of = |needle: &str| -> u16 {
            for y in 0..buffer.area.height {
                let mut line = String::new();
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                if line.contains(needle) {
                    return y;
                }
            }
            panic!("{needle:?} not found in render");
        };
        let y = row_of("newest");
        let n_cell = (0..buffer.area.width)
            .map(|x| &buffer[(x, y)])
            .find(|c| c.symbol() == "n")
            .expect("subtitle text cell");
        assert_eq!(
            n_cell.fg,
            crate::ui::theme::dim().fg.unwrap(),
            "speaker color must be suppressed when the setting is off"
        );
    }

    proptest::proptest! {
        /// Any sequence of growing prefixes collapses to a single entry
        /// whose text is the last (longest) cue.
        #[test]
        fn prefix_sequence_collapses_to_last(seed in "[a-z ]{1,40}") {
            let mut ui = intermixed_ui();
            let chars: Vec<char> = seed.chars().collect();
            for i in 1..=chars.len() {
                let prefix: String = chars[..i].iter().collect();
                ui.push_subtitle(i as u64, i as u64, prefix, None);
            }
            proptest::prop_assert_eq!(ui.subtitles.len(), 1);
            proptest::prop_assert_eq!(ui.subtitles.back().unwrap().text.as_str(), seed.as_str());
        }
    }

    #[test]
    fn intermixed_interleaves_subtitles_with_chat_by_arrival() {
        let mut state = CrdtState::new();
        state.append_chat(chat_msg(100, "kim", "before"));
        state.append_chat(chat_msg(300, "kim", "after"));
        let mut ui = ui_with_view(state.view());
        ui.subtitle_mode = SubtitleMode::Intermixed;
        // Arrival 200 sits between the two chat messages; the in-video
        // position (65s) is unrelated to the interleave order.
        ui.push_subtitle(65_000, 200, "sub".into(), None);

        let lines = ui.merged_chat(&ui.snapshot.view);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["before", "sub", "after"]);
        // The subtitle is dim, marker-prefixed, with an in-video MM:SS.
        let sub = lines.iter().find(|l| l.subtitle).unwrap();
        assert_eq!(sub.time, "01:05");
        assert!(sub.sender.is_empty());
    }

    #[test]
    fn speaker_names_apply_live_to_both_subtitle_text_modes() {
        let mut ui = ui_with_view(StateView::default());
        ui.subtitle_mode = SubtitleMode::Intermixed;
        ui.push_subtitle(65_000, 200, "Hello".into(), SpeakerName::new("Frieren"));

        let line = ui
            .merged_chat(&ui.snapshot.view)
            .into_iter()
            .find(|line| line.subtitle)
            .unwrap();
        assert_eq!(line.text, "Hello", "names default to hidden");

        ui.settings.subtitle_speaker_names = true;
        let line = ui
            .merged_chat(&ui.snapshot.view)
            .into_iter()
            .find(|line| line.subtitle)
            .unwrap();
        assert_eq!(line.text, "Frieren: Hello");

        // Settings save calls this after replacing the working settings.
        ui.refresh_chat();
        let buffer = render_test_buffer(&mut ui);
        assert_eq!(
            rendered_text_color(&buffer, "Frieren: Hello"),
            crate::ui::theme::dim().fg.unwrap(),
            "Intermixed speaker names remain uniformly dim"
        );

        ui.subtitle_mode = SubtitleMode::SeparatePane;
        ui.refresh_chat();
        let buffer = render_test_buffer(&mut ui);
        let rendered: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(rendered.contains("Frieren: Hello"), "{rendered}");
        assert_eq!(
            rendered_text_color(&buffer, "Frieren: Hello"),
            crate::ui::theme::user_style("Frieren").fg.unwrap(),
            "Separate-pane names retain the cue's speaker color"
        );
    }

    #[test]
    fn off_and_separate_keep_subtitles_out_of_chat() {
        let mut state = CrdtState::new();
        state.append_chat(chat_msg(100, "kim", "hi"));
        let mut ui = ui_with_view(state.view());
        ui.push_subtitle(1000, 50, "sub".into(), None);

        for mode in [SubtitleMode::Off, SubtitleMode::SeparatePane] {
            ui.subtitle_mode = mode;
            let lines = ui.merged_chat(&ui.snapshot.view);
            assert!(!lines.iter().any(|l| l.subtitle), "mode {mode:?}");
        }
    }

    #[test]
    fn f2_cycles_subtitle_mode_and_persists() {
        let mut ui = ui_with_view(StateView::default());
        assert_eq!(ui.subtitle_mode, SubtitleMode::Off);
        for expected in [
            SubtitleMode::Intermixed,
            SubtitleMode::SeparatePane,
            SubtitleMode::Off,
        ] {
            let action = ui.cycle_subtitle_mode();
            assert_eq!(ui.subtitle_mode, expected);
            assert_eq!(ui.settings.subtitle_mode, expected);
            // Each cycle persists the choice (F2-persistence).
            assert!(
                matches!(action, UserAction::SaveSettings(s, _) if s.subtitle_mode == expected)
            );
        }
    }

    #[test]
    fn ctrl_r_readies_to_maybe_when_not_watching() {
        let ui = ui_with_view(not_watching_state(&me()));
        let actions = ui.toggle_self_ready();
        let muts = mutations(&actions);
        // Clears the manual override...
        assert!(
            muts.iter()
                .any(|m| matches!(m, Mutation::SetManualOverride { state: None, .. }))
        );
        // ...latches Playing...
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Playing
            }
        )));
        // ...and flips the series back to Maybe (the escape hatch — NOT a
        // commit to Watching, which is now a deliberate `/watch`).
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetSeriesPreference {
                entry: ListEntryId(7),
                pref: SeriesWatchState::Maybe,
                ..
            }
        )));
    }

    #[test]
    fn ctrl_r_pauses_when_ready() {
        // No override and no NotWatching pref -> derived state is Ready.
        let mut state = CrdtState::new();
        state.set_now_playing(A, SharedTimestamp(1), Some(Ed2kHash([1; 16])));
        state.set_playback_intent(A, SharedTimestamp(2), PlaybackIntent::Playing);
        let ui = ui_with_view(state.view());
        let actions = ui.toggle_self_ready();
        let muts = mutations(&actions);
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetManualOverride {
                state: Some(ManualState::Paused),
                ..
            }
        )));
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Paused
            }
        )));
        // Ready -> unready never touches series preferences.
        assert!(
            !muts
                .iter()
                .any(|m| matches!(m, Mutation::SetSeriesPreference { .. }))
        );
    }

    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn locked_identity_is_not_moved_by_a_settings_save() {
        // `me` ("foo") is a runtime `--username` override that differs from
        // the persisted username ("real"): a settings save must keep the
        // identity on the override and must persist the stored name, never
        // the override (the run.rs side keeps it out of `settings`).
        let settings = Settings {
            username: Some("real".into()),
            password: Some("hunter2".into()),
            ..Default::default()
        };
        let mut ui = Ui::with_setup(
            UserId::new("foo"),
            settings,
            vec![PathBuf::from("/anime")],
            true,
        );
        assert!(ui.identity_locked, "an override should lock the identity");

        // Walk to the [Save] row and activate it.
        for _ in 0..30 {
            ui.handle(key(Key::Down));
        }
        let actions = ui.handle(key(Key::Enter));

        // The identity stays the override...
        assert_eq!(
            ui.me,
            UserId::new("foo"),
            "a settings save must not move a locked identity"
        );
        // ...and what gets persisted is the stored username, not the override.
        let saved = actions
            .iter()
            .find_map(|a| match a {
                UserAction::SaveSettings(s, _) => Some(s),
                _ => None,
            })
            .expect("a SaveSettings action");
        assert_eq!(saved.username, Some("real".into()));
    }

    #[test]
    fn settings_chat_command_opens_settings() {
        let mut ui = Ui::with_setup(me(), Settings::default(), vec![], false);
        assert!(ui.modals.is_empty());

        let actions = ui.command("/settings");
        assert!(actions.is_empty());
        assert!(matches!(ui.modals.last(), Some(Modal::Settings(_))));
    }

    #[test]
    fn chat_send_clears_my_away_but_keeps_chat() {
        // I was marked Away by someone else; sending a chat line is
        // activity from my client, so it clears the Away and still sends.
        let mut state = CrdtState::new();
        state.set_manual_override(
            A,
            SharedTimestamp(1),
            me(),
            Some(ManualState::Away {
                set_by: UserId::new("baughn"),
            }),
        );
        let ui = ui_with_view(state.view());
        let actions = ui.send_chat("i'm back");
        // Clears the override...
        assert!(actions.iter().any(|a| matches!(
            a,
            UserAction::Mutate(Mutation::SetManualOverride { state: None, user })
                if *user == me()
        )));
        // ...and still sends the message.
        assert!(actions.iter().any(|a| matches!(
            a,
            UserAction::Mutate(Mutation::Chat { text }) if text == "i'm back"
        )));
    }

    #[test]
    fn chat_send_keeps_my_pause() {
        // A deliberate Paused override is NOT cleared by chatting.
        let mut state = CrdtState::new();
        state.set_manual_override(A, SharedTimestamp(1), me(), Some(ManualState::Paused));
        let ui = ui_with_view(state.view());
        let actions = ui.send_chat("hi");
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, UserAction::Mutate(Mutation::SetManualOverride { .. })))
        );
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn chat_send_without_away_just_chats() {
        let ui = ui_with_view(StateView::default());
        let actions = ui.send_chat("hello");
        assert!(matches!(
            actions.as_slice(),
            [UserAction::Mutate(Mutation::Chat { text })] if text == "hello"
        ));
    }

    #[test]
    fn skip_marks_now_playing_series_not_watching() {
        // A now-playing file with series id 7, no preference yet.
        let mut state = CrdtState::new();
        state.set_now_playing(A, SharedTimestamp(1), Some(Ed2kHash([1; 16])));
        state.set_anidb_metadata(
            A,
            SharedTimestamp(2),
            Ed2kHash([1; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Show".into(),
                series_id: Some(AniDbSeriesId(7)),
                episode_number: Some("1".into()),
            }),
        );
        link_series(&mut state, SharedTimestamp(3), AniDbSeriesId(7));
        let mut ui = ui_with_view(state.view());
        let actions = ui.command("/skip");
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            UserAction::Mutate(Mutation::SetSeriesPreference {
                entry: ListEntryId(7),
                pref: SeriesWatchState::NotWatching,
                ..
            })
        ));
    }

    /// An unlinked List entry, matching `link_series`'s shape but with no
    /// AniDB series id.
    fn unlinked_entry(id: ListEntryId, name: &str) -> (ListEntryId, SeriesListEntry) {
        (
            id,
            SeriesListEntry {
                name: name.into(),
                nero_name: None,
                genre: None,
                notes: Vec::new(),
                recommender: None,
                status: ListStatus::Active,
                status_note: None,
                source: None,
                watchers: Default::default(),
                anidb_series_id: None,
                local_aliases: Default::default(),
                manual_files: Default::default(),
                anidb_unavailable: false,
            },
        )
    }

    #[test]
    fn empty_search_marks_the_entry_anidb_unavailable() {
        let mut state = CrdtState::new();
        let (id, entry) = unlinked_entry(ListEntryId(1), "Some Obscure Show");
        state.put_list_entry(A, SharedTimestamp(1), id, entry);
        let mut ui = ui_with_view(state.view());
        ui.update(Msg::LinkListEntry(id));
        assert!(matches!(ui.modals.last(), Some(Modal::AniDbSearch(_))));

        let actions = ui.set_search_results("Some Obscure Show", vec![]);
        assert_eq!(
            actions,
            vec![UserAction::Mutate(Mutation::PutListEntry {
                id,
                entry: SeriesListEntry {
                    anidb_unavailable: true,
                    ..unlinked_entry(id, "Some Obscure Show").1
                },
            })]
        );
    }

    #[test]
    fn empty_search_is_a_no_op_when_already_marked_unavailable() {
        let mut state = CrdtState::new();
        let (id, mut entry) = unlinked_entry(ListEntryId(1), "Some Obscure Show");
        entry.anidb_unavailable = true;
        state.put_list_entry(A, SharedTimestamp(1), id, entry);
        let mut ui = ui_with_view(state.view());
        ui.update(Msg::LinkListEntry(id));
        assert_eq!(ui.set_search_results("Some Obscure Show", vec![]), vec![]);
    }

    #[test]
    fn a_search_with_hits_clears_a_stale_unavailable_marker() {
        let mut state = CrdtState::new();
        let (id, mut entry) = unlinked_entry(ListEntryId(1), "Some Obscure Show");
        entry.anidb_unavailable = true;
        state.put_list_entry(A, SharedTimestamp(1), id, entry);
        let mut ui = ui_with_view(state.view());
        ui.update(Msg::LinkListEntry(id));

        let hit = dessplay_core::net::AniDbSearchHit {
            series: AniDbSeriesId(99),
            title: "Some Obscure Show".into(),
            matched: "Some Obscure Show".into(),
        };
        let actions = ui.set_search_results("Some Obscure Show", vec![hit]);
        assert_eq!(
            actions,
            vec![UserAction::Mutate(Mutation::PutListEntry {
                id,
                entry: SeriesListEntry {
                    anidb_unavailable: false,
                    ..unlinked_entry(id, "Some Obscure Show").1
                },
            })]
        );
    }

    #[test]
    fn stale_search_reply_is_ignored() {
        let mut state = CrdtState::new();
        let (id, entry) = unlinked_entry(ListEntryId(1), "Some Obscure Show");
        state.put_list_entry(A, SharedTimestamp(1), id, entry);
        let mut ui = ui_with_view(state.view());
        ui.update(Msg::LinkListEntry(id));
        // A reply for a query that's no longer the editor's text (the
        // user has since typed something else) must not write anything.
        assert_eq!(
            ui.set_search_results("some old query nobody sees anymore", vec![]),
            vec![]
        );
    }

    #[test]
    fn browse_unlinked_list_entry_opens_candidate_browser_when_candidates_exist() {
        let id = ListEntryId(1);
        let mut state = CrdtState::new();
        let (id, entry) = unlinked_entry(id, "Some Obscure Show");
        state.put_list_entry(A, SharedTimestamp(1), id, entry);
        // A library file whose derived name matches the entry -- a
        // candidate for its next episode (design.md, Advancing next_ep).
        state.set_anidb_metadata(
            A,
            SharedTimestamp(2),
            Ed2kHash([1; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::FilenameDerived,
                series_name: "Some Obscure Show".into(),
                series_id: None,
                episode_number: None,
            }),
        );
        state.set_file_catalog(
            A,
            SharedTimestamp(3),
            Ed2kHash([1; 16]),
            dessplay_core::types::FileCatalogEntry {
                filename: "Some Obscure Show - 01.mkv".into(),
                size_bytes: 1,
                duration_millis: None,
            },
        );
        let mut ui = ui_with_view(state.view());
        ui.update(Msg::BrowseListEntry(id));
        assert!(
            matches!(ui.modals.last(), Some(Modal::Episodes(_))),
            "a candidate should open the disambiguation browser, not the editor"
        );
    }

    #[test]
    fn browse_unlinked_list_entry_falls_back_to_editor_when_no_candidates() {
        let id = ListEntryId(1);
        let mut state = CrdtState::new();
        let (id, entry) = unlinked_entry(id, "Some Obscure Show");
        state.put_list_entry(A, SharedTimestamp(1), id, entry);
        // No files in the library at all -- nothing to disambiguate.
        let mut ui = ui_with_view(state.view());
        ui.update(Msg::BrowseListEntry(id));
        assert!(
            matches!(ui.modals.last(), Some(Modal::ListEdit(_))),
            "no candidates should fall back to the plain editor"
        );
    }

    /// Regression (2026-08-17, "Enter did nothing on Hyakunin no
    /// Kanojo"): an entry linked to a *non-root* season — S2 here; the
    /// franchise's key is the component root S1 — must still open the
    /// episode browser on its franchise. The old exact-key lookup
    /// (`FranchiseKey::Series(linked_id)`) missed and silently returned.
    #[test]
    fn browse_linked_entry_opens_the_franchise_even_when_not_the_component_root() {
        let mut state = CrdtState::new();
        state.set_series_relations(
            A,
            SharedTimestamp(1),
            AniDbSeriesId(10),
            dessplay_core::types::SeriesRelations {
                title: "Show".into(),
                year: Some(2024),
                episode_count: None,
                relations: [dessplay_core::types::SeriesRelation {
                    kind: dessplay_core::types::RelationKind::Sequel,
                    target: AniDbSeriesId(20),
                }]
                .into_iter()
                .collect(),
                short_titles: vec![],
            },
        );
        state.set_series_relations(
            A,
            SharedTimestamp(2),
            AniDbSeriesId(20),
            dessplay_core::types::SeriesRelations {
                title: "Show S2".into(),
                year: Some(2026),
                episode_count: None,
                relations: [dessplay_core::types::SeriesRelation {
                    kind: dessplay_core::types::RelationKind::Prequel,
                    target: AniDbSeriesId(10),
                }]
                .into_iter()
                .collect(),
                short_titles: vec![],
            },
        );
        // A held file for the linked season, so the franchise exists.
        state.set_anidb_metadata(
            A,
            SharedTimestamp(3),
            Ed2kHash([1; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Show S2".into(),
                series_id: Some(AniDbSeriesId(20)),
                episode_number: Some("1".into()),
            }),
        );
        let id = ListEntryId(1);
        let (id, entry) = unlinked_entry(id, "Show S2");
        let entry = SeriesListEntry {
            anidb_series_id: Some(AniDbSeriesId(20)),
            ..entry
        };
        state.put_list_entry(A, SharedTimestamp(4), id, entry);

        let mut ui = ui_with_view(state.view());
        ui.update(Msg::BrowseListEntry(id));
        assert!(
            matches!(ui.modals.last(), Some(Modal::Episodes(_))),
            "a linked entry with held franchise files must open the episode browser"
        );
    }

    /// `w` on a whole episode follows the any-copy rule in both
    /// directions: none of the copies flagged -> mark all; any flagged
    /// -> unmark, writing only the flags that actually change (no junk
    /// `false` rows for copies never marked).
    #[test]
    fn toggling_an_episode_marks_all_copies_and_unmarks_only_the_flagged() {
        let mut state = CrdtState::new();
        let mut ui = ui_with_view(state.view());
        let hashes = vec![Ed2kHash([1; 16]), Ed2kHash([2; 16])];

        // Nothing flagged: both copies get marked.
        assert_eq!(
            ui.toggle_episode_watched(&hashes),
            vec![
                UserAction::MarkWatched {
                    file: Ed2kHash([1; 16]),
                    watched: true,
                },
                UserAction::MarkWatched {
                    file: Ed2kHash([2; 16]),
                    watched: true,
                },
            ]
        );

        // One copy flagged: the episode is watched, so `w` unmarks — and
        // touches only the copy that carries a flag.
        state.set_watched(A, SharedTimestamp(1), Ed2kHash([1; 16]), true);
        ui.snapshot.view = std::sync::Arc::new(state.view());
        assert_eq!(
            ui.toggle_episode_watched(&hashes),
            vec![UserAction::MarkWatched {
                file: Ed2kHash([1; 16]),
                watched: false,
            }]
        );
    }

    /// Regression (2026-08-17): `w` in the episode browser writes the
    /// group watched flag, but the open modal froze its rows at open
    /// time — the mark only appeared after closing and reopening. A
    /// fresh snapshot (the `w` round-trip, or another client's toggle)
    /// must update the open browser in place: watched marks, holder
    /// lists, and the season's first-unwatched marker.
    #[test]
    fn episode_browser_updates_watched_marks_from_fresh_snapshots() {
        let mut state = CrdtState::new();
        state.set_anidb_metadata(
            A,
            SharedTimestamp(1),
            Ed2kHash([1; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Show".into(),
                series_id: Some(AniDbSeriesId(7)),
                episode_number: Some("1".into()),
            }),
        );
        let mut ui = ui_with_view(state.view());
        ui.update(Msg::BrowseFranchise(FranchiseKey::Series(AniDbSeriesId(7))));
        let Some(Modal::Episodes(browser)) = ui.modals.last() else {
            panic!("the episode browser should be open");
        };
        assert!(!browser.seasons()[0].episodes[0].watched());

        // The flag lands (our own `w`, echoed through the sync actor).
        state.set_watched(A, SharedTimestamp(2), Ed2kHash([1; 16]), true);
        ui.apply_snapshot(UiSnapshot {
            view: std::sync::Arc::new(state.view()),
            ..Default::default()
        });
        let Some(Modal::Episodes(browser)) = ui.modals.last() else {
            panic!("the episode browser should still be open");
        };
        assert!(
            browser.seasons()[0].episodes[0].watched(),
            "a fresh snapshot must mute the watched episode without reopening"
        );
        assert_eq!(
            browser.seasons()[0].first_unwatched,
            None,
            "the first-unwatched marker follows the fresh flags too"
        );
    }

    /// Regression (same investigation): a linked entry whose franchise
    /// holds no files anywhere must not be a silent no-op — it takes the
    /// unlinked path (candidates, then the editor).
    #[test]
    fn browse_linked_entry_with_no_held_files_falls_back_instead_of_doing_nothing() {
        let mut state = CrdtState::new();
        let id = ListEntryId(1);
        let (id, entry) = unlinked_entry(id, "Vaporware");
        let entry = SeriesListEntry {
            anidb_series_id: Some(AniDbSeriesId(99)),
            ..entry
        };
        state.put_list_entry(A, SharedTimestamp(1), id, entry);

        let mut ui = ui_with_view(state.view());
        ui.update(Msg::BrowseListEntry(id));
        assert!(
            matches!(ui.modals.last(), Some(Modal::ListEdit(_))),
            "no held files and no candidates: the editor, never silence; got {:?}",
            ui.modals.last().map(std::mem::discriminant)
        );
    }

    #[test]
    fn skip_without_series_info_notices() {
        // No now-playing file at all -> a single Notice, no mutation.
        let mut ui = ui_with_view(StateView::default());
        let actions = ui.command("/skip");
        assert!(matches!(actions.as_slice(), [UserAction::Notice(_)]));
    }

    #[test]
    fn skip_with_a_name_targets_that_user_attributed_to_self() {
        // design.md #7/#13: `/skip <name>` marks *another* user
        // NotWatching, attributed to the local user (`me`, i.e. "kim").
        let mut state = CrdtState::new();
        state.set_now_playing(A, SharedTimestamp(1), Some(Ed2kHash([1; 16])));
        state.set_anidb_metadata(
            A,
            SharedTimestamp(2),
            Ed2kHash([1; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Show".into(),
                series_id: Some(AniDbSeriesId(7)),
                episode_number: Some("1".into()),
            }),
        );
        link_series(&mut state, SharedTimestamp(3), AniDbSeriesId(7));
        let mut ui = ui_with_view(state.view());
        let actions = ui.command("/skip baughn");
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            UserAction::Mutate(Mutation::SetSeriesPreference {
                user,
                entry: ListEntryId(7),
                pref: SeriesWatchState::NotWatching,
                set_by: Some(setter),
            }) if *user == UserId::new("baughn") && *setter == me()
        ));
    }

    #[test]
    fn users_pane_n_marks_the_selected_user_not_watching_attributed_to_self() {
        // design.md #7/#13: `n` on the Users pane mirrors `/skip <name>`
        // but is driven by row selection instead of a typed name.
        let mut state = CrdtState::new();
        state.set_now_playing(A, SharedTimestamp(1), Some(Ed2kHash([1; 16])));
        state.set_anidb_metadata(
            A,
            SharedTimestamp(2),
            Ed2kHash([1; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Show".into(),
                series_id: Some(AniDbSeriesId(7)),
                episode_number: Some("1".into()),
            }),
        );
        link_series(&mut state, SharedTimestamp(3), AniDbSeriesId(7));
        let ui = ui_with_view(state.view());
        // `Msg::SetNotWatching` is intercepted in `handle()` (it can yield
        // several actions, e.g. an auto-created List entry), never reaching
        // `update()` -- exercise the underlying handler directly.
        let action = ui
            .set_others_pref(UserId::new("baughn"), SeriesWatchState::NotWatching, "n")
            .into_iter()
            .next();
        assert!(matches!(
            action,
            Some(UserAction::Mutate(Mutation::SetSeriesPreference {
                user,
                entry: ListEntryId(7),
                pref: SeriesWatchState::NotWatching,
                set_by: Some(setter),
            })) if user == UserId::new("baughn") && setter == me()
        ));
    }

    #[test]
    fn me_command_sends_encoded_action() {
        use dessplay_core::types::decode_action;
        let mut ui = ui_with_view(StateView::default());
        let actions = ui.command("/me waves at Nero");
        // A single synced chat message whose body decodes to the action.
        assert!(matches!(
            actions.as_slice(),
            [UserAction::Mutate(Mutation::Chat { text })]
                if decode_action(text) == Some("waves at Nero")
        ));
    }

    #[test]
    fn me_command_clears_away() {
        // Like sending a normal chat line, `/me` is an "I'm here" action.
        let mut state = CrdtState::new();
        state.set_manual_override(
            A,
            SharedTimestamp(1),
            me(),
            Some(ManualState::Away {
                set_by: UserId::new("baughn"),
            }),
        );
        let mut ui = ui_with_view(state.view());
        let actions = ui.command("/me waves");
        assert!(actions.iter().any(|a| matches!(
            a,
            UserAction::Mutate(Mutation::SetManualOverride { state: None, user })
                if *user == me()
        )));
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, UserAction::Mutate(Mutation::Chat { .. })))
        );
    }

    #[test]
    fn me_command_without_action_notices() {
        let mut ui = ui_with_view(StateView::default());
        assert!(matches!(
            ui.command("/me").as_slice(),
            [UserAction::Notice(_)]
        ));
        assert!(matches!(
            ui.command("/me    ").as_slice(),
            [UserAction::Notice(_)]
        ));
    }

    #[test]
    fn ready_command_readies_and_clears_not_watching_to_maybe() {
        let mut ui = ui_with_view(not_watching_state(&me()));
        let actions = ui.command("/ready");
        let muts = mutations(&actions);
        assert!(
            muts.iter()
                .any(|m| matches!(m, Mutation::SetManualOverride { state: None, .. }))
        );
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Playing
            }
        )));
        // Clears NotWatching to Maybe (the default), not a Watching commit.
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetSeriesPreference {
                entry: ListEntryId(7),
                pref: SeriesWatchState::Maybe,
                ..
            }
        )));
    }

    /// Playlist `w` cycles Maybe -> Watching -> NotWatching -> Maybe. The
    /// order matters: the common first press (from the absent/Maybe
    /// default) happens mid-show and must commit the user, not skip the
    /// series (the old Maybe -> NotWatching order did exactly that).
    #[test]
    fn w_cycle_commits_first_from_the_maybe_default() {
        for (current, expected) in [
            (None, SeriesWatchState::Watching),
            (
                Some(SeriesWatchState::Watching),
                SeriesWatchState::NotWatching,
            ),
            (Some(SeriesWatchState::NotWatching), SeriesWatchState::Maybe),
        ] {
            let ui = ui_with_view(now_playing_state_with_pref(&me(), current));
            let actions = ui.cycle_series_watch(Ed2kHash([1; 16]));
            let muts = mutations(&actions);
            assert!(
                muts.iter().any(|m| matches!(
                    m,
                    Mutation::SetSeriesPreference { pref, .. } if *pref == expected
                )),
                "from {current:?}: expected a write of {expected:?}, got {muts:?}"
            );
        }
    }

    #[test]
    fn watch_command_commits_now_playing_series() {
        let mut ui = ui_with_view(not_watching_state(&me()));
        let actions = ui.command("/watch");
        let muts = mutations(&actions);
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetSeriesPreference {
                entry: ListEntryId(7),
                pref: SeriesWatchState::Watching,
                ..
            }
        )));
    }

    #[test]
    fn pause_command_pauses() {
        let mut ui = ui_with_view(StateView::default());
        let actions = ui.command("/pause");
        let muts = mutations(&actions);
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetManualOverride {
                state: Some(ManualState::Paused),
                ..
            }
        )));
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Paused
            }
        )));
    }

    #[test]
    fn away_command_marks_self_then_named() {
        let mut ui = ui_with_view(StateView::default());
        // No argument -> marks myself away, set_by myself.
        let actions = ui.command("/away");
        assert!(matches!(
            actions.as_slice(),
            [UserAction::Mutate(Mutation::SetManualOverride {
                user,
                state: Some(ManualState::Away { set_by }),
            })] if *user == me() && *set_by == me()
        ));
        // With a name -> targets that user, still set_by me.
        let actions = ui.command("/away nero");
        assert!(matches!(
            actions.as_slice(),
            [UserAction::Mutate(Mutation::SetManualOverride {
                user,
                state: Some(ManualState::Away { set_by }),
            })] if *user == UserId::new("nero") && *set_by == me()
        ));
    }

    #[test]
    fn unknown_command_notices_without_acting() {
        let mut ui = ui_with_view(StateView::default());
        let actions = ui.command("/frobnicate");
        assert!(matches!(actions.as_slice(), [UserAction::Notice(_)]));
        assert!(!actions.iter().any(|a| matches!(a, UserAction::Quit)));
    }

    // ---- pasted_file_path: the drag-in normalization (Phase 31).

    /// A temp dir holding `My Show/ep 1.mkv` — a path with spaces, the
    /// case every terminal escapes or quotes on drag. The base is
    /// canonicalized so the expected path survives the boundary's own
    /// canonicalization (e.g. a symlinked system temp dir).
    fn spaced_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let sub = base.join("My Show");
        std::fs::create_dir(&sub).unwrap();
        let path = sub.join("ep 1.mkv");
        std::fs::write(&path, b"x").unwrap();
        (dir, path)
    }

    #[test]
    fn pasted_bare_path_is_accepted() {
        let (_dir, path) = spaced_file();
        assert_eq!(
            pasted_file_path(&path.display().to_string()),
            Some(path.clone())
        );
        // Terminals often append a trailing newline to a drag.
        assert_eq!(
            pasted_file_path(&format!("{}\n", path.display())),
            Some(path)
        );
    }

    #[test]
    fn pasted_backslash_escaped_path_is_unescaped() {
        let (_dir, path) = spaced_file();
        let escaped = path.display().to_string().replace(' ', "\\ ");
        assert_ne!(escaped, path.display().to_string());
        assert_eq!(pasted_file_path(&escaped), Some(path));
    }

    #[test]
    fn pasted_quoted_paths_are_unquoted() {
        let (_dir, path) = spaced_file();
        assert_eq!(
            pasted_file_path(&format!("'{}'", path.display())),
            Some(path.clone())
        );
        assert_eq!(
            pasted_file_path(&format!("\"{}\"", path.display())),
            Some(path)
        );
    }

    #[test]
    fn pasted_file_url_is_decoded() {
        let (_dir, path) = spaced_file();
        let url = format!("file://{}", path.display().to_string().replace(' ', "%20"));
        assert_eq!(pasted_file_path(&url), Some(path.clone()));
        // With a host part, as some platforms produce.
        let url = format!(
            "file://localhost{}",
            path.display().to_string().replace(' ', "%20")
        );
        assert_eq!(pasted_file_path(&url), Some(path));
    }

    #[test]
    fn pasted_non_paths_are_rejected() {
        let (_dir, path) = spaced_file();
        // A directory is not an addable file.
        assert_eq!(
            pasted_file_path(&path.parent().unwrap().display().to_string()),
            None
        );
        // Multi-line text is never a path, even when one line is real.
        assert_eq!(
            pasted_file_path(&format!("{}\nand more", path.display())),
            None
        );
        assert_eq!(pasted_file_path("no-such-file.mkv"), None);
        assert_eq!(pasted_file_path(""), None);
    }

    /// Regression (2026-08-20 review): a pasted *relative* path was
    /// accepted verbatim and flowed all the way into a durable manual
    /// mapping — a permanent cwd-dependent registration that names
    /// nothing after any cwd change (exactly the dead-mapping input that
    /// wedged resolution). The boundary must canonicalize.
    #[test]
    fn pasted_relative_path_comes_back_absolute() {
        let (_dir, path) = spaced_file();
        // Build a relative path from the cwd to the temp file.
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        let mut relative = PathBuf::new();
        for _ in cwd.components().filter(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        }) {
            relative.push("..");
        }
        for component in path.components().skip(1) {
            relative.push(component);
        }
        assert!(relative.is_relative());
        assert!(relative.is_file(), "test setup: relative path must exist");

        let accepted = pasted_file_path(&relative.display().to_string())
            .expect("an existing relative path must be accepted");
        assert!(
            accepted.is_absolute(),
            "a pasted relative path must come back absolute, got: {}",
            accepted.display()
        );
        assert_eq!(accepted, path);
    }

    // ---- Chat drag-selection hold timer (design.md, Mouse support) ----

    /// Whether the buffer cell at (x, y) renders reversed — the
    /// selection highlight.
    fn cell_reversed(buffer: &tuirealm::ratatui::buffer::Buffer, x: u16, y: u16) -> bool {
        buffer[(x, y)]
            .modifier
            .contains(tuirealm::ratatui::style::Modifier::REVERSED)
    }

    fn mouse_at(kind: MouseEventKind, column: u16, row: u16) -> Event<NoUserEvent> {
        Event::Mouse(MouseEvent {
            kind,
            modifiers: KeyModifiers::NONE,
            column,
            row,
        })
    }

    /// Dragging the column splitter moves the chat/right boundary to
    /// the pointer, the release persists the new layout, and the next
    /// draw lays the panes out accordingly (design.md, Mouse support:
    /// resizable panes).
    #[test]
    fn dragging_the_column_splitter_resizes_and_persists() {
        let mut ui = ui_with_view(StateView::default());
        render_test_buffer(&mut ui); // 100x30: chat is columns 0..50
        assert_eq!(ui.panes.chat.width, 50);
        let boundary = ui.panes.series.x; // 50; strip is columns 49..=50
        let row = ui.panes.chat.y + 2;
        assert!(
            ui.handle(mouse_at(
                MouseEventKind::Down(MouseButton::Left),
                boundary,
                row
            ))
            .is_empty()
        );
        assert!(
            ui.handle(mouse_at(MouseEventKind::Drag(MouseButton::Left), 70, row))
                .is_empty(),
            "a drag in progress emits nothing"
        );
        assert_eq!(ui.settings.pane_layout.chat_width, 70);
        let actions = ui.handle(mouse_at(MouseEventKind::Up(MouseButton::Left), 70, row));
        assert!(
            matches!(&actions[..], [UserAction::SaveSettings(s, _)] if s.pane_layout.chat_width == 70),
            "release persists the layout: {actions:?}"
        );
        render_test_buffer(&mut ui);
        assert_eq!(ui.panes.chat.width, 70);
        assert_eq!(ui.panes.series.x, 70);
        // A grab-and-release without movement has nothing to save.
        ui.handle(mouse_at(MouseEventKind::Down(MouseButton::Left), 70, row));
        assert!(
            ui.handle(mouse_at(MouseEventKind::Up(MouseButton::Left), 70, row))
                .is_empty()
        );
    }

    /// A drag past the edge pins the pane at its minimum share rather
    /// than collapsing it; the next splitter down keeps its place.
    #[test]
    fn splitter_drags_clamp_to_minimum_pane_sizes() {
        let mut ui = ui_with_view(StateView::default());
        render_test_buffer(&mut ui);
        // Series/Users boundary: drag it to the top of the screen.
        let column = ui.panes.series.x + 5;
        let boundary = ui.panes.users.y;
        let users_end_before = ui.panes.playlist.y;
        ui.handle(mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            column,
            boundary,
        ));
        ui.handle(mouse_at(MouseEventKind::Drag(MouseButton::Left), column, 0));
        ui.handle(mouse_at(MouseEventKind::Up(MouseButton::Left), column, 0));
        let layout = ui.settings.pane_layout;
        assert_eq!(layout.series_height, PaneLayout::MIN);
        assert_eq!(
            layout.series_height + layout.users_height,
            34 + 33,
            "users end held"
        );
        render_test_buffer(&mut ui);
        assert_eq!(ui.panes.playlist.y, users_end_before);
        // Column splitter dragged off the right edge: chat pinned at MAX.
        let row = ui.panes.chat.y + 2;
        ui.handle(mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            ui.panes.series.x,
            row,
        ));
        ui.handle(mouse_at(MouseEventKind::Drag(MouseButton::Left), 99, row));
        ui.handle(mouse_at(MouseEventKind::Up(MouseButton::Left), 99, row));
        assert_eq!(ui.settings.pane_layout.chat_width, PaneLayout::MAX);
        render_test_buffer(&mut ui);
        assert!(
            ui.panes.playlist.height >= 3,
            "every pane keeps a usable height"
        );
    }

    /// The subtitle splitter only exists while the pane is shown; a
    /// press where it would be is an ordinary chat click otherwise.
    #[test]
    fn subtitle_splitter_only_exists_in_separate_pane_mode() {
        let mut ui = ui_with_view(StateView::default());
        render_test_buffer(&mut ui);
        assert!(ui.panes.splitter_at(Position::new(5, 20)).is_none());
        ui.subtitle_mode = SubtitleMode::SeparatePane;
        render_test_buffer(&mut ui);
        let boundary = ui.panes.subs.y;
        assert_eq!(
            ui.panes.splitter_at(Position::new(5, boundary)),
            Some(Splitter::ChatSubs)
        );
        ui.handle(mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            5,
            boundary,
        ));
        ui.handle(mouse_at(
            MouseEventKind::Drag(MouseButton::Left),
            5,
            boundary - 5,
        ));
        assert!(
            ui.settings.pane_layout.subtitle_height > 30,
            "dragging up grows the pane"
        );
        ui.handle(mouse_at(
            MouseEventKind::Up(MouseButton::Left),
            5,
            boundary - 5,
        ));
        render_test_buffer(&mut ui);
        assert_eq!(ui.panes.subs.y, boundary - 5);
    }

    /// The held highlight expires SELECTION_TTL_MS (5s) after the
    /// release: the expiry tick reports a repaint, the reverse-video
    /// cells disappear, and Shift-Down has nothing left to extend. The
    /// clipboard copy itself happened on release and is untouched.
    #[test]
    fn held_selection_highlight_expires_after_its_ttl() {
        let mut state = CrdtState::new();
        state.append_chat(dessplay_core::types::ChatMessage {
            timestamp: SharedTimestamp(1_000),
            sender: dessplay_core::types::UserId::new("amu"),
            text: "hello world".into(),
        });
        let mut ui = ui_with_view(StateView::default());
        // The shell freshens the clock before every input.
        ui.advance_clock(10_000);
        ui.apply_snapshot(UiSnapshot {
            view: std::sync::Arc::new(state.view()),
            now: 10_000,
            shared_now: 10_000,
            ..UiSnapshot::default()
        });
        // First draw records the log geometry. Body starts past the
        // border (1) and the "HH:MM amu: " prefix (11): 'w' of "world"
        // sits at column 18, body row 1.
        render_test_buffer(&mut ui);
        let mouse = |kind, column| {
            Event::Mouse(MouseEvent {
                kind,
                modifiers: KeyModifiers::NONE,
                column,
                row: 1,
            })
        };
        ui.handle(mouse(MouseEventKind::Down(MouseButton::Left), 18));
        ui.handle(mouse(MouseEventKind::Drag(MouseButton::Left), 22));
        assert_eq!(
            ui.handle(mouse(MouseEventKind::Up(MouseButton::Left), 22)),
            vec![UserAction::CopyToClipboard("world".into())]
        );
        let buffer = render_test_buffer(&mut ui);
        assert!(cell_reversed(&buffer, 18, 1), "held selection highlights");
        assert!(!cell_reversed(&buffer, 12, 1), "unselected text does not");

        // One millisecond short of the TTL: nothing changes.
        assert!(!ui.advance_clock(14_999));
        // Crossing it wants a repaint, and the highlight is gone.
        assert!(ui.advance_clock(15_000), "expiry wants a repaint");
        let buffer = render_test_buffer(&mut ui);
        assert!(!cell_reversed(&buffer, 18, 1), "highlight expired");
        assert!(
            ui.handle(Event::Keyboard(KeyEvent {
                code: Key::Down,
                modifiers: KeyModifiers::SHIFT,
            }))
            .is_empty(),
            "an expired selection cannot be extended"
        );
    }

    /// The selection-TTL variant of the backward-step regression
    /// (2026-08-20 review): a hold stamped while `shared_now` led wall
    /// must still expire 5 s of real time after release, not 5 s after
    /// the latched maximum.
    #[test]
    fn held_selection_ttl_survives_a_backward_shared_clock_step() {
        let mut state = CrdtState::new();
        state.append_chat(dessplay_core::types::ChatMessage {
            timestamp: SharedTimestamp(1_000),
            sender: dessplay_core::types::UserId::new("amu"),
            text: "hello world".into(),
        });
        let view = std::sync::Arc::new(state.view());
        let mut ui = ui_with_view(StateView::default());
        // Shell tick, then a snapshot whose shared clock leads wall by
        // 90 s.
        ui.advance_clock(10_000);
        ui.apply_snapshot(UiSnapshot {
            view: view.clone(),
            now: 10_000,
            shared_now: 100_000,
            ..UiSnapshot::default()
        });
        render_test_buffer(&mut ui);
        let mouse = |kind, column| {
            Event::Mouse(MouseEvent {
                kind,
                modifiers: KeyModifiers::NONE,
                column,
                row: 1,
            })
        };
        ui.handle(mouse(MouseEventKind::Down(MouseButton::Left), 18));
        ui.handle(mouse(MouseEventKind::Drag(MouseButton::Left), 22));
        assert_eq!(
            ui.handle(mouse(MouseEventKind::Up(MouseButton::Left), 22)),
            vec![UserAction::CopyToClipboard("world".into())]
        );
        // ClockSync corrects the offset: shared falls back to wall.
        ui.advance_clock(10_100);
        ui.apply_snapshot(UiSnapshot {
            view,
            now: 10_100,
            shared_now: 10_100,
            ..UiSnapshot::default()
        });
        // 5 s of real time after the release the highlight must be gone.
        assert!(ui.advance_clock(15_100), "expiry wants a repaint");
        let buffer = render_test_buffer(&mut ui);
        assert!(!cell_reversed(&buffer, 18, 1), "highlight expired");
    }

    /// Regression (audit 2026-08-20): server chat compaction keeps only
    /// the trailing `chat_keep` messages and broadcasts the shrunk log.
    /// A selection held across that shrink kept its stale line index;
    /// Shift-Up then indexed `lines[10]` of a 2-line log and panicked
    /// the UI thread (no catch_unwind — the whole TUI exited). The held
    /// selection is keyed by message identity now: Shift-Up after the
    /// shrink must not panic, and must extend from the surviving
    /// selected message — never from a line that no longer exists.
    #[test]
    fn shift_up_on_a_selection_held_across_a_chat_compaction_does_not_panic() {
        let chat_view = |from: u64| {
            let mut state = CrdtState::new();
            for i in from..=12 {
                state.append_chat(dessplay_core::types::ChatMessage {
                    timestamp: SharedTimestamp(i * 1_000),
                    sender: dessplay_core::types::UserId::new("amu"),
                    text: format!("message-{i:02}"),
                });
            }
            state.view()
        };
        let snap = |view| UiSnapshot {
            view: std::sync::Arc::new(view),
            now: 10_000,
            shared_now: 10_000,
            ..UiSnapshot::default()
        };
        let mut ui = ui_with_view(StateView::default());
        ui.apply_snapshot(snap(chat_view(1)));
        // First draw records the log geometry: 12 one-row messages on
        // body rows y=1..=12, bodies starting past "HH:MM amu: " and the
        // border at column 12.
        render_test_buffer(&mut ui);
        let mouse = |kind, column, row| {
            Event::Mouse(MouseEvent {
                kind,
                modifiers: KeyModifiers::NONE,
                column,
                row,
            })
        };
        // Drag-select part of the last message.
        ui.handle(mouse(MouseEventKind::Down(MouseButton::Left), 12, 12));
        ui.handle(mouse(MouseEventKind::Drag(MouseButton::Left), 18, 12));
        let copied = ui.handle(mouse(MouseEventKind::Up(MouseButton::Left), 18, 12));
        assert!(
            matches!(copied.as_slice(), [UserAction::CopyToClipboard(_)]),
            "release must copy the dragged text: {copied:?}"
        );
        // Compaction: a snapshot whose chat holds only the trailing 2.
        ui.apply_snapshot(snap(chat_view(11)));
        let actions = ui.handle(Event::Keyboard(KeyEvent {
            code: Key::Up,
            modifiers: KeyModifiers::SHIFT,
        }));
        // Sane outcomes only: the selection re-anchored onto its message
        // (the copy covers the survivors) or was dropped — never a copy
        // of unrelated lines, never a panic.
        match actions.as_slice() {
            [UserAction::CopyToClipboard(text)] => {
                assert!(
                    text.contains("message-12") && text.contains("message-11"),
                    "extension must cover the selected message and its \
                     surviving neighbour: {text:?}"
                );
            }
            [] => {}
            other => panic!("unexpected actions: {other:?}"),
        }
    }

    /// Regression (audit 2026-08-20): the merged log is not append-only
    /// — a line sorting *before* the selection (a saturated local ring
    /// evicting an entry, a system line older than recent chat) shifts
    /// every later index down, and a positionally-held selection then
    /// highlighted and re-copied a neighbouring message the user never
    /// selected. The highlight must follow the selected message's
    /// identity across the rebuild, and Shift-Down must extend from it.
    #[test]
    fn held_selection_follows_its_message_when_the_merged_log_shifts() {
        let mut state = CrdtState::new();
        for (i, text) in ["alpha", "bravo", "charlie"].iter().enumerate() {
            state.append_chat(dessplay_core::types::ChatMessage {
                timestamp: SharedTimestamp((i as u64 + 5) * 1_000),
                sender: dessplay_core::types::UserId::new("amu"),
                text: (*text).into(),
            });
        }
        let view = state.view();
        let snap = || UiSnapshot {
            view: std::sync::Arc::new(view.clone()),
            now: 10_000,
            shared_now: 10_000,
            ..UiSnapshot::default()
        };
        let mut ui = ui_with_view(StateView::default());
        ui.apply_snapshot(snap());
        render_test_buffer(&mut ui);
        let mouse = |kind, column, row| {
            Event::Mouse(MouseEvent {
                kind,
                modifiers: KeyModifiers::NONE,
                column,
                row,
            })
        };
        // Select all of "bravo" (body row y=2, body chars at column 12).
        ui.handle(mouse(MouseEventKind::Down(MouseButton::Left), 12, 2));
        ui.handle(mouse(MouseEventKind::Drag(MouseButton::Left), 16, 2));
        assert_eq!(
            ui.handle(mouse(MouseEventKind::Up(MouseButton::Left), 16, 2)),
            vec![UserAction::CopyToClipboard("bravo".into())]
        );
        // Shift the merged log under the held selection: a system line
        // older than every chat message sorts to the top on the next
        // rebuild, moving every chat index down by one.
        ui.log_system_line(1_000, "the system speaks".into());
        ui.apply_snapshot(snap());
        // Rows now: y=1 system, y=2 alpha, y=3 bravo, y=4 charlie.
        let buffer = render_test_buffer(&mut ui);
        assert!(
            cell_reversed(&buffer, 12, 3),
            "the highlight must follow bravo to its new row"
        );
        assert!(
            !cell_reversed(&buffer, 12, 2),
            "alpha was never selected and must not highlight"
        );
        // Shift-Down extends from bravo: bravo + charlie, never alpha.
        let actions = ui.handle(Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: KeyModifiers::SHIFT,
        }));
        match actions.as_slice() {
            [UserAction::CopyToClipboard(text)] => {
                assert!(
                    text.contains("bravo") && text.contains("charlie"),
                    "extension must cover the selected message: {text:?}"
                );
                assert!(
                    !text.contains("alpha") && !text.contains("system"),
                    "extension must not copy lines the user never \
                     selected: {text:?}"
                );
            }
            other => panic!("Shift-Down must extend the selection: {other:?}"),
        }
    }
}
