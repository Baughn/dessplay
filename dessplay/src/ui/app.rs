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
    Ed2kHash, ManualState, PlaybackIntent, SeriesWatchState, UserId,
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
use super::modals::{AniDbSearchModal, EpisodeBrowser, FileBrowser, ListEditModal, Season, SettingsModal};
use super::msg::{Msg, UserAction};
use super::props;
use crate::actors::sync::Mutation;
use crate::config::Settings;

/// Everything the UI renders from, refreshed on every state/peer
/// change.
#[derive(Clone, Debug, Default)]
pub struct UiSnapshot {
    /// The resolved CRDT view.
    pub view: StateView,
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
    subtitle_pane: bool,
    /// Rolling log of the local player's subtitle lines.
    subtitles: std::collections::VecDeque<String>,
    /// In-flight playlist-add hashes: (filename, done, total). Drawn as
    /// a progress overlay while non-empty (the no-silent-work rule).
    hashing: Vec<(String, u64, u64)>,
    /// Local-only system chat lines (archive results, etc.), merged into
    /// the chat log by timestamp. Never synced.
    system_log: Vec<props::ChatLine>,
    snapshot: UiSnapshot,
    settings: Settings,
    media_roots: Vec<PathBuf>,
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
            subtitle_pane: settings.subtitle_pane,
            subtitles: std::collections::VecDeque::new(),
            hashing: Vec::new(),
            system_log: Vec::new(),
            snapshot: UiSnapshot::default(),
            settings: settings.clone(),
            media_roots: media_roots.clone(),
        };
        if open_settings {
            ui.push_modal(Modal::Settings(SettingsModal::new(settings, media_roots)));
        }
        ui.sync_focus_attr();
        ui.refresh_keybar();
        ui
    }

    /// Append a subtitle line to the rolling log (empty lines are
    /// clears — the previous line just stopped displaying; skip them).
    pub fn push_subtitle(&mut self, line: String) {
        if line.is_empty() {
            return;
        }
        // mpv re-reports multi-line cues; collapse exact repeats.
        if self.subtitles.back() == Some(&line) {
            return;
        }
        self.subtitles.push_back(line);
        while self.subtitles.len() > 100 {
            self.subtitles.pop_front();
        }
    }

    /// Append a local system chat line (e.g. an archive result) and
    /// refresh the chat pane. Not synced — local to this client.
    pub fn push_system(&mut self, timestamp: u64, text: String) {
        self.system_log.push(props::system_line(timestamp, text));
        while self.system_log.len() > 100 {
            self.system_log.remove(0);
        }
        let chat = self.merged_chat(&self.snapshot.view);
        self.chat.set_lines(chat);
    }

    /// The synced chat log merged with local system lines, ordered by
    /// shared-clock millis (stable: synced messages sort before a system
    /// line stamped at the same millisecond).
    fn merged_chat(&self, view: &StateView) -> Vec<props::ChatLine> {
        let mut lines = props::chat_lines(view);
        lines.extend(self.system_log.iter().cloned());
        lines.sort_by_key(|line| line.millis);
        lines
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
        match self.series.mode() {
            SeriesMode::Recent => self.series.set_franchises(props::franchise_rows(
                &self.snapshot.view,
                self.series.sort(),
                Some(&self.snapshot.recency),
                self.series.filter(),
            )),
            SeriesMode::All => self.series.set_franchises(props::franchise_rows(
                &self.snapshot.view,
                self.series.sort(),
                None,
                self.series.filter(),
            )),
            SeriesMode::TheList => self
                .series
                .set_groups(props::list_groups(&self.snapshot.view)),
        }
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
                // F3 opens settings, but only when no modal is up.
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
                    self.focus = self.focus.next();
                    tracing::debug!(focus = ?self.focus, "focus changed");
                    self.sync_focus_attr();
                    self.refresh_keybar();
                    return Vec::new();
                }
                Some(Key::Function(2)) => {
                    self.subtitle_pane = !self.subtitle_pane;
                    tracing::debug!(enabled = self.subtitle_pane, "subtitle pane toggled");
                    return Vec::new();
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
        let view = &self.snapshot.view;
        let me = self.me.clone();
        let current = derive::user_state(view, &me);
        tracing::debug!(?current, "Ctrl-R: toggling self readiness");
        if current == DerivedUserState::Ready {
            // Become unready: like pressing pause.
            return vec![
                UserAction::Mutate(Mutation::SetManualOverride {
                    user: me,
                    state: Some(ManualState::Paused),
                }),
                UserAction::Mutate(Mutation::SetPlaybackIntent {
                    intent: PlaybackIntent::Paused,
                }),
            ];
        }
        // Become ready: like pressing play ("I'm ready, go"). Clear any
        // manual override and latch Playing.
        let mut actions = vec![
            UserAction::Mutate(Mutation::SetManualOverride {
                user: me.clone(),
                state: None,
            }),
            UserAction::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Playing,
            }),
        ];
        // If the now-playing series is marked NotWatching for us, flip it
        // back to Watching so the derived state actually reaches Ready.
        if let Some(file) = view.now_playing
            && let Some(Some(metadata)) = view.anidb_metadata.get(&file)
            && let Some(series) = metadata.series_id
            && view.series_preference.get(&(me.clone(), series))
                == Some(&SeriesWatchState::NotWatching)
        {
            actions.push(UserAction::Mutate(Mutation::SetSeriesPreference {
                user: me,
                series,
                pref: SeriesWatchState::Watching,
            }));
        }
        actions
    }

    /// The Elm update: messages become internal changes or actions.
    fn update(&mut self, msg: Msg) -> Option<UserAction> {
        match msg {
            Msg::None => None,
            Msg::SendChat(text) => Some(UserAction::Mutate(Mutation::Chat { text })),
            Msg::Command(command) => self.command(&command),
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
            Msg::PlaySelected(hash) => Some(UserAction::Mutate(Mutation::SetNowPlaying {
                file: Some(hash),
            })),
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
                let entry = self.snapshot.view.playlist.iter().find(|e| e.hash == hash)?;
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
                let entry = self.snapshot.view.playlist.iter().find(|e| e.hash == hash)?;
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
                self.subtitle_pane = settings.subtitle_pane;
                self.media_roots = roots.clone();
                // First-run setup may have changed who we are.
                if let Some(name) = &settings.username {
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
            Msg::ToggleSubtitlePane => {
                self.subtitle_pane = !self.subtitle_pane;
                None
            }
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

    /// `/commands` from the chat input.
    fn command(&mut self, command: &str) -> Option<UserAction> {
        let mut parts = command.split_whitespace();
        match parts.next()? {
            "/quit" | "/exit" | "/q" => Some(UserAction::Quit),
            "/settings" => {
                self.open_settings();
                None
            }
            "/afk" => {
                let name = parts.next()?;
                Some(UserAction::Mutate(Mutation::SetManualOverride {
                    user: UserId::new(name),
                    state: Some(ManualState::Away {
                        set_by: self.me.clone(),
                    }),
                }))
            }
            _ => None,
        }
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

        if self.subtitle_pane {
            let [chat_area, subs_area] =
                Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)])
                    .areas(left);
            self.chat.view(frame, chat_area);
            // The newest lines that fit, oldest first.
            let visible = (subs_area.height as usize).saturating_sub(2);
            let lines: Vec<tuirealm::ratatui::text::Line> = self
                .subtitles
                .iter()
                .rev()
                .take(visible)
                .rev()
                .map(|line| tuirealm::ratatui::text::Line::from(line.as_str()))
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
        ui.snapshot.view = view;
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

    #[test]
    fn ctrl_r_marks_watching_and_readies_when_not_watching() {
        let ui = ui_with_view(not_watching_state(&me()));
        let actions = ui.toggle_self_ready();
        let muts = mutations(&actions);
        // Clears the manual override...
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetManualOverride { state: None, .. }
        )));
        // ...latches Playing...
        assert!(muts.iter().any(|m| matches!(
            m,
            Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Playing
            }
        )));
        // ...and flips the series back to Watching (the escape hatch).
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

        let action = ui.update(Msg::Command("/settings".into()));
        assert!(action.is_none());
        assert!(matches!(ui.modals.last(), Some(Modal::Settings(_))));
    }
}
