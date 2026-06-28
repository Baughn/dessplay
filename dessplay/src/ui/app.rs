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
    AniDbSeriesId, Ed2kHash, ManualState, PlaybackIntent, SeriesWatchState, UserId, encode_action,
};
use tuirealm::component::AppComponent;
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout};
use tuirealm::ratatui::widgets::{Block, Borders};

use super::components::{
    ChatPane, KeyBar, PlaylistPane, SeriesMode, SeriesPane, StatusBar, UsersPane,
};
use super::modals::{
    AniDbSearchModal, EpisodeBrowser, FileBrowser, ListEditModal, Season, SettingsModal,
};
use super::msg::{Msg, UserAction};
use super::props;
use crate::actors::sync::Mutation;
use crate::config::{Settings, SubtitleMode};

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
    /// Local watch history: series (by AniDB id or filename-parsed name)
    /// -> last-watched millis (drives the Recent mode sort).
    pub recency: BTreeMap<crate::storage::SeriesKey, u64>,
    /// Hashes that live only in the local download cache (not in a media
    /// root). These render a dim "temporary" marker and are the only
    /// rows the archive action operates on. Local, not synced.
    pub cache_hashes: BTreeSet<Ed2kHash>,
}

/// Log one outgoing [`UserAction`] at debug. Mutations log their
/// variant name; `SaveSettings` deliberately logs no contents (the
/// settings carry the password).
fn log_action(action: &UserAction) {
    match action {
        UserAction::Mutate(mutation) => {
            tracing::debug!(mutation = mutation.name(), "user action: Mutate");
        }
        UserAction::HashAndAdd { path, .. } => {
            tracing::debug!(path = %path.display(), "user action: HashAndAdd");
        }
        UserAction::AddByHash { hash, .. } => {
            tracing::debug!(%hash, "user action: AddByHash");
        }
        UserAction::SaveSettings(..) => tracing::debug!("user action: SaveSettings"),
        UserAction::AniDbSearch { query } => {
            tracing::debug!(%query, "user action: AniDbSearch");
        }
        UserAction::MapFile { path, .. } => {
            tracing::debug!(path = %path.display(), "user action: MapFile");
        }
        UserAction::Archive { filename, .. } => {
            tracing::debug!(%filename, "user action: Archive");
        }
        UserAction::Notice(text) => tracing::debug!(%text, "user action: Notice"),
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

impl Focus {
    fn next(self) -> Self {
        match self {
            Focus::Chat => Focus::Series,
            Focus::Series => Focus::Users,
            Focus::Users => Focus::Playlist,
            Focus::Playlist => Focus::Chat,
        }
    }
}

/// An open modal.
enum Modal {
    Files(FileBrowser),
    Settings(SettingsModal),
    Episodes(EpisodeBrowser),
    ListEdit(ListEditModal),
    AniDbSearch(AniDbSearchModal),
}

impl Modal {
    fn as_component(&mut self) -> &mut dyn AppComponent<Msg, NoUserEvent> {
        match self {
            Modal::Files(modal) => modal,
            Modal::Settings(modal) => modal,
            Modal::Episodes(modal) => modal,
            Modal::ListEdit(modal) => modal,
            Modal::AniDbSearch(modal) => modal,
        }
    }

    fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            Modal::Files(modal) => modal.keybindings(),
            Modal::Settings(modal) => modal.keybindings(),
            Modal::Episodes(modal) => modal.keybindings(),
            Modal::ListEdit(modal) => modal.keybindings(),
            Modal::AniDbSearch(modal) => modal.keybindings(),
        }
    }

    /// The modal's name, for logging.
    fn name(&self) -> &'static str {
        match self {
            Modal::Files(_) => "Files",
            Modal::Settings(_) => "Settings",
            Modal::Episodes(_) => "Episodes",
            Modal::ListEdit(_) => "ListEdit",
            Modal::AniDbSearch(_) => "AniDbSearch",
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
    /// The ASS speaker/actor, if the cue carried one. Never displayed —
    /// only hashed to a color in separate-pane mode.
    speaker: Option<String>,
}

/// The whole TUI.
pub struct Ui {
    me: UserId,
    chat: ChatPane,
    series: SeriesPane,
    users: UsersPane,
    playlist: PlaylistPane,
    status: StatusBar,
    keybar: KeyBar,
    modals: Vec<Modal>,
    focus: Focus,
    subtitle_mode: SubtitleMode,
    /// Rolling log of the local player's subtitle lines (with in-video
    /// and arrival timestamps). Local only — never synced.
    subtitles: std::collections::VecDeque<SubtitleEntry>,
    /// In-flight playlist-add hashes: (filename, done, total). Drawn as
    /// a progress overlay while non-empty (the no-silent-work rule).
    hashing: Vec<(String, u64, u64)>,
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
            keybar: KeyBar::default(),
            modals: Vec::new(),
            focus: Focus::Chat,
            subtitle_mode: settings.subtitle_mode,
            subtitles: std::collections::VecDeque::new(),
            hashing: Vec::new(),
            system_log: Vec::new(),
            irc_log: Vec::new(),
            snapshot: UiSnapshot::default(),
            franchise_cache: franchise::FranchiseCache::default(),
            settings: settings.clone(),
            media_roots: media_roots.clone(),
            identity_locked,
        };
        ui.chat.set_me(ui.me.to_string());
        if open_settings {
            ui.push_modal(Modal::Settings(SettingsModal::new(settings, media_roots)));
        }
        ui.sync_focus_attr();
        ui.refresh_keybar();
        ui
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
        speaker: Option<String>,
    ) {
        let text = text.replace('\r', "").replace('\n', " ");
        if text.is_empty() {
            return;
        }
        // Classify the new text against the last entry: does it *extend* it
        // (same cue, fuller — a reveal or a neighbour appearing) or is it
        // *contained* in it (same cue, receding — a neighbour ending)? The
        // length test makes the two mutually exclusive; either end of the
        // join may carry the change, so both prefix and suffix count.
        let (extends, contained) = match self.subtitles.back() {
            Some(last) => (
                text.len() >= last.text.len()
                    && (text.starts_with(&last.text) || text.ends_with(&last.text)),
                text.len() < last.text.len()
                    && (last.text.starts_with(&text) || last.text.ends_with(&text)),
            ),
            None => (false, false),
        };
        if let Some(last) = self.subtitles.back_mut().filter(|_| extends) {
            // Same cue, fuller now: keep the original timestamps, track the
            // latest text and speaker.
            last.text = text;
            last.speaker = speaker;
        } else if contained {
            // Same cue receding (an overlapping neighbour ended); the fuller
            // text is already logged — drop the redundant re-show.
        } else {
            self.subtitles.push_back(SubtitleEntry {
                video_millis,
                arrival_millis,
                text,
                speaker,
            });
            while self.subtitles.len() > 100 {
                self.subtitles.pop_front();
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
        self.system_log.push(props::system_line(timestamp, text));
        while self.system_log.len() > 100 {
            self.system_log.remove(0);
        }
        self.refresh_chat();
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
            lines.extend(
                self.subtitles.iter().map(|s| {
                    props::subtitle_line(s.video_millis, s.arrival_millis, s.text.clone())
                }),
            );
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
    pub fn set_search_results(
        &mut self,
        query: &str,
        results: Vec<dessplay_core::net::AniDbSearchHit>,
    ) {
        if let Some(Modal::AniDbSearch(modal)) = self.modals.last_mut() {
            modal.set_results(query, results);
        }
    }

    /// Replace the snapshot and recompute every pane's props.
    pub fn apply_snapshot(&mut self, snapshot: UiSnapshot) {
        let chat = self.merged_chat(&snapshot.view);
        self.chat.set_lines(chat);
        self.chat
            .set_usernames(props::chat_usernames(&snapshot.peers));
        self.users
            .set_props(props::users_props(&snapshot.view, &snapshot.peers));
        self.playlist.set_props(props::playlist_props(
            &snapshot.view,
            &self.me,
            &snapshot.cache_hashes,
        ));
        self.status.set_props(props::status_props(
            &snapshot.view,
            &snapshot.peers,
            &self.me,
        ));
        self.snapshot = snapshot;
        self.refresh_series();
    }

    fn refresh_series(&mut self) {
        let recency = match self.series.mode() {
            SeriesMode::Recent => Some(&self.snapshot.recency),
            SeriesMode::All => None,
            SeriesMode::TheList => {
                let groups = props::list_groups(&self.snapshot.view);
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
            self.series.filter(),
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

    /// Route one input event; returns the actions it produced.
    pub fn handle(&mut self, ev: Event<NoUserEvent>) -> Vec<UserAction> {
        // SECURITY: never log keystroke contents while the settings
        // modal is open — the user may be typing the password.
        if matches!(self.modals.last(), Some(Modal::Settings(_))) {
            tracing::trace!("ui event (contents redacted: settings modal open)");
        } else {
            tracing::trace!(event = ?ev, "ui event");
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
        if self.modals.is_empty() {
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
            _ => {}
        }
        let action = msg.and_then(|msg| self.update(msg));
        if let Some(action) = &action {
            log_action(action);
        }
        self.refresh_keybar();
        action.into_iter().collect()
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
            && let Some(Some(metadata)) = view.anidb_metadata.get(&file)
            && let Some(series) = metadata.series_id
            && view.series_preference.get(&(me.clone(), series))
                == Some(&SeriesWatchState::NotWatching)
        {
            actions.push(UserAction::Mutate(Mutation::SetSeriesPreference {
                user: me,
                series,
                pref: SeriesWatchState::Maybe,
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
    /// Re-selecting the already-playing entry is not a transition, so it
    /// must not pause.
    fn play_selected(&self, hash: Ed2kHash) -> Vec<UserAction> {
        let mut actions = vec![UserAction::Mutate(Mutation::SetNowPlaying {
            file: Some(hash),
        })];
        if self.snapshot.view.now_playing != Some(hash) {
            actions.push(UserAction::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Paused,
            }));
        }
        actions
    }

    /// The Elm update: messages become internal changes or actions.
    fn update(&mut self, msg: Msg) -> Option<UserAction> {
        match msg {
            Msg::None => None,
            // `Msg::SendChat`, `Msg::Command`, and `Msg::PlaySelected` are
            // intercepted in `handle()` (they can each yield several
            // actions); they never reach `update()`.
            Msg::SendChat(_) | Msg::Command(_) | Msg::PlaySelected(_) => None,
            Msg::CycleSeriesMode | Msg::ToggleSeriesSort | Msg::SeriesFilterChanged => {
                self.refresh_series();
                None
            }
            Msg::BrowseFranchise(key) => {
                self.open_episode_browser(key);
                None
            }
            Msg::EditListEntry(id) => {
                let entry = self.snapshot.view.list_entries.get(&id)?.clone();
                self.push_modal(Modal::ListEdit(ListEditModal::new(id, entry)));
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
                let currently_away = matches!(
                    self.snapshot.view.manual_override.get(&user),
                    Some(Some(ManualState::Away { .. }))
                );
                let state = if currently_away {
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
                self.push_modal(Modal::Files(FileBrowser::for_file(
                    self.media_roots.clone(),
                    after,
                )));
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
                // Open the mapping browser at the media roots (the
                // per-series last-used directory is FileActor state, not
                // in the snapshot yet — edit-distance ranking surfaces
                // the right file regardless).
                let entry = self
                    .snapshot
                    .view
                    .playlist
                    .iter()
                    .find(|e| e.hash == hash)?;
                let target = entry.state.filename.clone();
                self.push_modal(Modal::Files(FileBrowser::for_mapping(
                    self.media_roots.clone(),
                    hash,
                    target,
                    None,
                )));
                None
            }
            Msg::FileMapped { file, path } => {
                self.pop_modal();
                self.sync_focus_attr();
                Some(UserAction::MapFile {
                    file,
                    path,
                    series: self.series_key_of(file),
                })
            }
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
            Msg::CycleSeriesWatch(hash) => {
                let view = &self.snapshot.view;
                // Resolve the entry's series; no id yet → local notice.
                let Some(series) = view
                    .anidb_metadata
                    .get(&hash)
                    .and_then(|m| m.as_ref())
                    .and_then(|m| m.series_id)
                else {
                    return Some(UserAction::Notice(
                        "watch: no series info for that file yet".to_string(),
                    ));
                };
                // Cycle Watching -> Maybe -> NotWatching -> Watching, with
                // an absent entry (the default) treated as Maybe.
                let current = view
                    .series_preference
                    .get(&(self.me.clone(), series))
                    .copied()
                    .unwrap_or(SeriesWatchState::Maybe);
                let next = match current {
                    SeriesWatchState::Watching => SeriesWatchState::Maybe,
                    SeriesWatchState::Maybe => SeriesWatchState::NotWatching,
                    SeriesWatchState::NotWatching => SeriesWatchState::Watching,
                };
                Some(UserAction::Mutate(Mutation::SetSeriesPreference {
                    user: self.me.clone(),
                    series,
                    pref: next,
                }))
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
                Some(UserAction::HashAndAdd { path, after })
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
            Msg::ListEntrySaved(id, entry) => {
                self.pop_modal();
                self.sync_focus_attr();
                Some(UserAction::Mutate(Mutation::PutListEntry {
                    id,
                    entry: *entry,
                }))
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
            "/skip" => self.set_now_playing_pref(SeriesWatchState::NotWatching, "/skip"),
            // Play past a committed-but-absent blocker of the current file.
            "/ack" => self.acknowledge_blockers(),
            other => vec![UserAction::Notice(format!(
                "Unknown command: {other} — type / to see commands"
            ))],
        }
    }

    /// The AniDB series id of the now-playing file, if known — the key the
    /// per-series watch commands write against.
    fn now_playing_series(&self) -> Option<AniDbSeriesId> {
        let view = &self.snapshot.view;
        let file = view.now_playing?;
        view.anidb_metadata.get(&file)?.as_ref()?.series_id
    }

    /// Set our watch preference for the now-playing file's series, or post
    /// a local notice when there is no series id yet.
    fn set_now_playing_pref(&self, pref: SeriesWatchState, cmd: &str) -> Vec<UserAction> {
        match self.now_playing_series() {
            Some(series) => vec![UserAction::Mutate(Mutation::SetSeriesPreference {
                user: self.me.clone(),
                series,
                pref,
            })],
            None => vec![UserAction::Notice(format!(
                "{cmd}: no series info for the current file yet"
            ))],
        }
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

    fn open_episode_browser(&mut self, key: FranchiseKey) {
        let view = &self.snapshot.view;
        let franchise = franchise::franchises(view)
            .into_iter()
            .find(|franchise| franchise.key == key);
        let Some(franchise) = franchise else { return };
        // Build a season's episode list, ordered topologically by AniDB
        // episode number (falling back to a natural parse of the label) --
        // the metadata map is keyed by ed2k hash, so it arrives unordered.
        let season_episodes = |hashes: Vec<Ed2kHash>| -> Vec<(Ed2kHash, String)> {
            let mut episodes: Vec<(Ed2kHash, String)> = hashes
                .into_iter()
                .map(|hash| (hash, super::props::episode_label(view, &hash)))
                .collect();
            episodes.sort_by(|a, b| {
                let key = |hash: &Ed2kHash, label: &str| {
                    let epno = view
                        .anidb_metadata
                        .get(hash)
                        .and_then(|metadata| metadata.as_ref())
                        .and_then(|metadata| metadata.episode_number.as_deref());
                    super::props::episode_sort_key(epno, label)
                };
                key(&a.0, &a.1).cmp(&key(&b.0, &b.1))
            });
            episodes
        };
        let seasons: Vec<Season> = if franchise.series.is_empty() {
            vec![Season {
                title: franchise.title.clone(),
                episodes: season_episodes(franchise.files.clone()),
            }]
        } else {
            franchise
                .series
                .iter()
                .map(|series| Season {
                    title: view
                        .series_relations
                        .get(series)
                        .map(|relations| relations.title.clone())
                        .unwrap_or_else(|| format!("anidb:{}", series.0)),
                    episodes: season_episodes(
                        view.anidb_metadata
                            .iter()
                            .filter_map(|(hash, metadata)| {
                                let metadata = metadata.as_ref()?;
                                (metadata.series_id == Some(*series)).then_some(*hash)
                            })
                            .collect(),
                    ),
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
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(main);
        let [series_area, users_area, playlist_area] = Layout::vertical([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .areas(right);

        if self.subtitle_mode == SubtitleMode::SeparatePane {
            let [chat_area, subs_area] =
                Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)])
                    .areas(left);
            self.chat.view(frame, chat_area);
            // The newest lines that fit, newest first (top) — the input box
            // sits just below, so the freshest line is closest to the eye.
            // Each line: a dim in-video timestamp, then the text colored by
            // its ASS speaker (reusing chat's name->color hash), so each
            // speaker is visually distinct. The speaker name itself is never
            // shown (spoilers).
            use tuirealm::ratatui::text::{Line, Span};
            let visible = (subs_area.height as usize).saturating_sub(2);
            let lines: Vec<Line> = self
                .subtitles
                .iter()
                .rev()
                .take(visible)
                .map(|entry| {
                    let text_style = match &entry.speaker {
                        Some(name) => super::theme::user_style(name),
                        None => tuirealm::ratatui::style::Style::default(),
                    };
                    Line::from(vec![
                        Span::styled(
                            format!("{}  ", props::mmss(entry.video_millis)),
                            super::theme::dim(),
                        ),
                        Span::styled(entry.text.clone(), text_style),
                    ])
                })
                .collect();
            frame.render_widget(
                tuirealm::ratatui::widgets::Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(super::theme::dim())
                        .title("Subtitles"),
                ),
                subs_area,
            );
        } else {
            // Off and Intermixed both use the full-width chat pane
            // (Intermixed shows subtitles inside the chat log).
            self.chat.view(frame, left);
        }
        self.series.view(frame, series_area);
        self.users.view(frame, users_area);
        self.playlist.view(frame, playlist_area);
        self.status.view(frame, status_area);
        self.keybar.view(frame, keybar_area);
        if let Some(modal) = self.modals.last_mut() {
            modal.as_component().view(frame, frame.area());
        }
        self.draw_hash_overlay(frame);
    }

    /// The hashing progress overlay: visually modal (centered, on top
    /// of everything), but it captures no input — chat keeps working
    /// while files hash. Design.md's no-silent-work rule.
    fn draw_hash_overlay(&self, frame: &mut Frame<'_>) {
        use tuirealm::ratatui::layout::Rect;
        use tuirealm::ratatui::widgets::{Clear, Paragraph};

        if self.hashing.is_empty() {
            return;
        }
        let area = frame.area();
        let height = (self.hashing.len() as u16 * 2 + 2).min(area.height);
        let width = (area.width * 3 / 5).clamp(20.min(area.width), area.width);
        let overlay = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, overlay);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title("Hashing for playlist"),
            overlay,
        );
        for (i, (filename, done, total)) in self.hashing.iter().enumerate() {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use dessplay_core::state::CrdtState;
    use dessplay_core::types::{
        ActorId, AniDbMetadata, AniDbSeriesId, MetadataSource, SharedTimestamp,
    };

    const A: ActorId = ActorId::SERVER;

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

    /// A now-playing file whose series (id 7) the given user marked
    /// NotWatching — the auto-set state with no other UI escape hatch.
    fn not_watching_state(user: &UserId) -> StateView {
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
        state.set_series_preference(
            A,
            SharedTimestamp(5),
            user.clone(),
            AniDbSeriesId(7),
            SeriesWatchState::NotWatching,
        );
        state.view()
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
        ui.push_subtitle(1000, 10, "H".into(), Some("Frieren".into()));
        ui.push_subtitle(1100, 11, "He".into(), Some("Frieren".into()));
        ui.push_subtitle(1200, 12, "Hello".into(), Some("Frieren".into()));
        assert_eq!(ui.subtitles.len(), 1);
        let entry = ui.subtitles.back().unwrap();
        assert_eq!(entry.text, "Hello");
        assert_eq!(entry.video_millis, 1000);
        assert_eq!(entry.arrival_millis, 10);
        // The speaker tracks the latest cue in the collapsed reveal.
        assert_eq!(entry.speaker.as_deref(), Some("Frieren"));
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

    #[test]
    fn separate_pane_renders_newest_on_top_colored_by_speaker() {
        use tuirealm::ratatui::Terminal;
        use tuirealm::ratatui::backend::TestBackend;

        let mut ui = ui_with_view(StateView::default());
        ui.subtitle_mode = SubtitleMode::SeparatePane;
        // Three distinct cues, oldest first; the third names a speaker.
        ui.push_subtitle(1000, 10, "oldest".into(), None);
        ui.push_subtitle(2000, 20, "middle".into(), None);
        ui.push_subtitle(3000, 30, "newest".into(), Some("Frieren".into()));

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

        // Feature 2: the speaker'd line's text is colored with the same
        // hash->palette color chat uses; the timestamp prefix stays dim.
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
                series: AniDbSeriesId(7),
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
    fn save_button_closes_settings_dialog() {
        // A valid save (Enter on the [Save] row) persists *and* dismisses
        // the modal — the dialog must not linger after saving.
        let settings = Settings {
            username: Some("nero".into()),
            password: Some("hunter2".into()),
            ..Default::default()
        };
        let mut ui = Ui::with_setup(me(), settings, vec![PathBuf::from("/anime")], true);
        assert!(matches!(ui.modals.last(), Some(Modal::Settings(_))));
        // Walk the cursor to the last row ([Save]) and activate it. Down
        // clamps at the last row, so any count past the field total lands
        // on [Save] regardless of how many setting rows exist.
        for _ in 0..30 {
            ui.handle(key(Key::Down));
        }
        let actions = ui.handle(key(Key::Enter));
        assert!(
            ui.modals.is_empty(),
            "save should close the settings dialog"
        );
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, UserAction::SaveSettings(..))),
            "save should emit a SaveSettings action",
        );
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
    fn unlocked_identity_follows_a_settings_save() {
        // No override: `me` matches the persisted username, so first-run
        // setup (confirming/editing the name) is free to update the
        // identity — the lock guard does not engage.
        let settings = Settings {
            username: Some("kim".into()),
            password: Some("hunter2".into()),
            ..Default::default()
        };
        let ui = Ui::with_setup(
            UserId::new("kim"),
            settings,
            vec![PathBuf::from("/anime")],
            true,
        );
        assert!(
            !ui.identity_locked,
            "without an override the identity is not locked"
        );
    }

    #[test]
    fn f3_opens_settings_and_esc_closes_it() {
        // A non-first-run UI: no modal is open initially.
        let mut ui = Ui::with_setup(me(), Settings::default(), vec![], false);
        assert!(ui.modals.is_empty());

        ui.handle(key(Key::Function(3)));
        assert!(matches!(ui.modals.last(), Some(Modal::Settings(_))));

        // Esc dismisses it back to the main screen.
        ui.handle(key(Key::Esc));
        assert!(ui.modals.is_empty());
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
        let mut ui = ui_with_view(state.view());
        let actions = ui.command("/skip");
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            UserAction::Mutate(Mutation::SetSeriesPreference {
                series: AniDbSeriesId(7),
                pref: SeriesWatchState::NotWatching,
                ..
            })
        ));
    }

    #[test]
    fn skip_without_series_info_notices() {
        // No now-playing file at all -> a single Notice, no mutation.
        let mut ui = ui_with_view(StateView::default());
        let actions = ui.command("/skip");
        assert!(matches!(actions.as_slice(), [UserAction::Notice(_)]));
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
                series: AniDbSeriesId(7),
                pref: SeriesWatchState::Maybe,
                ..
            }
        )));
    }

    #[test]
    fn watch_command_commits_now_playing_series() {
        let mut ui = ui_with_view(not_watching_state(&me()));
        let actions = ui.command("/watch");
        let muts = mutations(&actions);
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetSeriesPreference {
                series: AniDbSeriesId(7),
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
}
