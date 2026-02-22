//! Application state machine: processes events, returns effects.
//!
//! `AppState` is a plain struct with no I/O. All input arrives via `AppEvent`;
//! all output is `Vec<AppEffect>`. This makes it trivially testable — inject
//! events, inspect state and returned effects.

use std::collections::HashMap;

use dessplay_core::protocol::{
    CrdtOp, CrdtSnapshot, GapFillRequest, GapFillResponse, LwwValue, PlaylistAction,
    VersionVectors,
};
use dessplay_core::sync_engine::{SyncAction, SyncEngine};
use dessplay_core::types::{FileId, FileState, PeerId, SharedTimestamp, UserId, UserState};

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Events that drive the AppState machine.
#[derive(Debug, Clone)]
pub enum AppEvent {
    // --- Network ---
    RemoteOp { from: PeerId, op: CrdtOp },
    PeerConnected { peer_id: PeerId, username: String },
    PeerDisconnected { peer_id: PeerId },
    StateSummary {
        from: PeerId,
        epoch: u64,
        versions: VersionVectors,
    },
    StateSnapshot {
        epoch: u64,
        snapshot: CrdtSnapshot,
    },
    GapFillResponse {
        from: PeerId,
        response: GapFillResponse,
    },

    // --- User actions ---
    SendChat { text: String },
    SetUserState { state: UserState },
    SetFileState { file_id: FileId, state: FileState },
    AddToPlaylist {
        file_id: FileId,
        after: Option<FileId>,
    },
    RemoveFromPlaylist { file_id: FileId },
    MoveInPlaylist {
        file_id: FileId,
        after: Option<FileId>,
    },

    // --- Player (Phase 7 stubs) ---
    PlayerPaused,
    PlayerUnpaused,
    PlayerSeeked { position_secs: f64 },
    PlayerPosition { position_secs: f64 },
    PlayerEof,
    PlayerCrashed,

    // --- Timer ---
    Tick,
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

/// Effects AppState requests the runtime to execute.
#[derive(Debug, Clone)]
pub enum AppEffect {
    /// Dispatch these sync actions to the network/storage layers.
    Sync(Vec<SyncAction>),
    /// Signal the TUI to redraw (Phase 6).
    Redraw,
    /// Player control (Phase 7).
    PlayerPause,
    PlayerUnpause,
    PlayerSeek(f64),
    PlayerLoadFile(FileId),
    PlayerShowOsd(String),
}

// ---------------------------------------------------------------------------
// Playback state
// ---------------------------------------------------------------------------

/// Derived playback state, recomputed on every state change.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackState {
    /// Whether playback should proceed (all users permit it).
    pub should_play: bool,
    /// Users who are blocking playback.
    pub blocking_users: Vec<UserId>,
    /// The current file (first item in the playlist).
    pub current_file: Option<FileId>,
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// The central application state machine.
pub struct AppState {
    /// Our own user identity.
    pub our_user_id: UserId,
    /// The sync engine (owns CrdtState).
    pub sync_engine: SyncEngine,
    /// Maps PeerId -> UserId for connected peers.
    pub connected_peers: HashMap<PeerId, UserId>,
    /// Cached playback state, updated by `recompute_playback`.
    pub playback: PlaybackState,
    /// Next chat sequence number for our messages.
    chat_seq: u64,
}

impl AppState {
    /// Create a new AppState with empty CRDT state.
    pub fn new(our_user_id: UserId) -> Self {
        let mut state = Self {
            our_user_id,
            sync_engine: SyncEngine::new(),
            connected_peers: HashMap::new(),
            playback: PlaybackState {
                should_play: false,
                blocking_users: Vec::new(),
                current_file: None,
            },
            chat_seq: 0,
        };
        state.recompute_playback();
        state
    }

    /// Construct from persisted state (loaded from SQLite at startup).
    pub fn from_persisted(our_user_id: UserId, engine: SyncEngine) -> Self {
        // Initialize chat_seq from existing messages for our user
        let chat_seq = engine
            .state()
            .chat
            .entries_since(&our_user_id, None)
            .last()
            .map(|e| e.seq + 1)
            .unwrap_or(0);

        let mut state = Self {
            our_user_id,
            sync_engine: engine,
            connected_peers: HashMap::new(),
            playback: PlaybackState {
                should_play: false,
                blocking_users: Vec::new(),
                current_file: None,
            },
            chat_seq,
        };
        state.recompute_playback();
        state
    }

    // -----------------------------------------------------------------------
    // Main dispatch
    // -----------------------------------------------------------------------

    /// Process one event, returning effects for the runtime to execute.
    pub fn process_event(&mut self, event: AppEvent, now: SharedTimestamp) -> Vec<AppEffect> {
        match event {
            AppEvent::RemoteOp { from, op } => self.handle_remote_op(from, op),
            AppEvent::PeerConnected { peer_id, username } => {
                self.handle_peer_connected(peer_id, username)
            }
            AppEvent::PeerDisconnected { peer_id } => self.handle_peer_disconnected(peer_id),
            AppEvent::StateSummary {
                from,
                epoch,
                versions,
            } => self.handle_state_summary(from, epoch, versions),
            AppEvent::StateSnapshot { epoch, snapshot } => {
                self.handle_state_snapshot(epoch, snapshot)
            }
            AppEvent::GapFillResponse { from, response } => {
                self.handle_gap_fill_response(from, response)
            }
            AppEvent::SendChat { text } => self.handle_send_chat(text, now),
            AppEvent::SetUserState { state } => self.handle_set_user_state(state, now),
            AppEvent::SetFileState { file_id, state } => {
                self.handle_set_file_state(file_id, state, now)
            }
            AppEvent::AddToPlaylist { file_id, after } => {
                self.handle_add_to_playlist(file_id, after, now)
            }
            AppEvent::RemoveFromPlaylist { file_id } => {
                self.handle_remove_from_playlist(file_id, now)
            }
            AppEvent::MoveInPlaylist { file_id, after } => {
                self.handle_move_in_playlist(file_id, after, now)
            }
            AppEvent::Tick => self.handle_tick(),
            // Phase 7 stubs
            AppEvent::PlayerPaused
            | AppEvent::PlayerUnpaused
            | AppEvent::PlayerSeeked { .. }
            | AppEvent::PlayerPosition { .. }
            | AppEvent::PlayerEof
            | AppEvent::PlayerCrashed => vec![],
        }
    }

    /// Handle an incoming gap fill request (read-only, for incoming streams).
    pub fn on_gap_fill_request(&self, request: &GapFillRequest) -> GapFillResponse {
        self.sync_engine.on_gap_fill_request(request)
    }

    // -----------------------------------------------------------------------
    // Event handlers
    // -----------------------------------------------------------------------

    fn handle_remote_op(&mut self, from: PeerId, op: CrdtOp) -> Vec<AppEffect> {
        let actions = self.sync_engine.on_remote_op(from, op);
        self.recompute_playback();
        let mut effects = vec![AppEffect::Redraw];
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_peer_connected(&mut self, peer_id: PeerId, username: String) -> Vec<AppEffect> {
        self.connected_peers.insert(peer_id, UserId(username));
        let actions = self.sync_engine.on_peer_connected(peer_id);
        self.recompute_playback();
        let mut effects = vec![AppEffect::Redraw];
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_peer_disconnected(&mut self, peer_id: PeerId) -> Vec<AppEffect> {
        self.connected_peers.remove(&peer_id);
        let actions = self.sync_engine.on_peer_disconnected(peer_id);
        self.recompute_playback();
        let mut effects = vec![AppEffect::Redraw];
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_state_summary(
        &mut self,
        from: PeerId,
        epoch: u64,
        versions: VersionVectors,
    ) -> Vec<AppEffect> {
        let actions = self.sync_engine.on_state_summary(from, epoch, versions);
        sync_to_effects(actions)
    }

    fn handle_state_snapshot(&mut self, epoch: u64, snapshot: CrdtSnapshot) -> Vec<AppEffect> {
        let actions = self.sync_engine.on_state_snapshot(epoch, snapshot);
        if !actions.is_empty() {
            self.recompute_playback();
            let mut effects = vec![AppEffect::Redraw];
            effects.push(AppEffect::Sync(actions));
            return effects;
        }
        vec![]
    }

    fn handle_gap_fill_response(
        &mut self,
        from: PeerId,
        response: GapFillResponse,
    ) -> Vec<AppEffect> {
        let actions = self.sync_engine.on_gap_fill_response(from, response);
        if !actions.is_empty() {
            self.recompute_playback();
            return vec![AppEffect::Sync(actions), AppEffect::Redraw];
        }
        vec![]
    }

    fn handle_send_chat(&mut self, text: String, now: SharedTimestamp) -> Vec<AppEffect> {
        let seq = self.chat_seq;
        self.chat_seq += 1;
        let op = CrdtOp::ChatAppend {
            user_id: self.our_user_id.clone(),
            seq,
            timestamp: now,
            text,
        };
        let actions = self.sync_engine.apply_local_op(op);
        let mut effects = vec![AppEffect::Redraw];
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_set_user_state(&mut self, state: UserState, now: SharedTimestamp) -> Vec<AppEffect> {
        let op = CrdtOp::LwwWrite {
            timestamp: now,
            value: LwwValue::UserState(self.our_user_id.clone(), state),
        };
        let actions = self.sync_engine.apply_local_op(op);
        self.recompute_playback();
        let mut effects = vec![AppEffect::Redraw];
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_set_file_state(
        &mut self,
        file_id: FileId,
        state: FileState,
        now: SharedTimestamp,
    ) -> Vec<AppEffect> {
        let op = CrdtOp::LwwWrite {
            timestamp: now,
            value: LwwValue::FileState(self.our_user_id.clone(), file_id, state),
        };
        let actions = self.sync_engine.apply_local_op(op);
        self.recompute_playback();
        let mut effects = vec![AppEffect::Redraw];
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_add_to_playlist(
        &mut self,
        file_id: FileId,
        after: Option<FileId>,
        now: SharedTimestamp,
    ) -> Vec<AppEffect> {
        let op = CrdtOp::PlaylistOp {
            timestamp: now,
            action: PlaylistAction::Add { file_id, after },
        };
        let actions = self.sync_engine.apply_local_op(op);
        self.recompute_playback();
        let mut effects = vec![AppEffect::Redraw];
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_remove_from_playlist(
        &mut self,
        file_id: FileId,
        now: SharedTimestamp,
    ) -> Vec<AppEffect> {
        let op = CrdtOp::PlaylistOp {
            timestamp: now,
            action: PlaylistAction::Remove { file_id },
        };
        let actions = self.sync_engine.apply_local_op(op);
        self.recompute_playback();
        let mut effects = vec![AppEffect::Redraw];
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_move_in_playlist(
        &mut self,
        file_id: FileId,
        after: Option<FileId>,
        now: SharedTimestamp,
    ) -> Vec<AppEffect> {
        let op = CrdtOp::PlaylistOp {
            timestamp: now,
            action: PlaylistAction::Move { file_id, after },
        };
        let actions = self.sync_engine.apply_local_op(op);
        self.recompute_playback();
        let mut effects = vec![AppEffect::Redraw];
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_tick(&self) -> Vec<AppEffect> {
        let actions = self.sync_engine.on_periodic_tick();
        vec![AppEffect::Sync(actions)]
    }

    // -----------------------------------------------------------------------
    // Derived playback
    // -----------------------------------------------------------------------

    fn recompute_playback(&mut self) {
        let crdt = self.sync_engine.state();
        let playlist_snapshot = crdt.playlist.snapshot();
        let current_file = playlist_snapshot.into_iter().next();

        let mut blocking = Vec::new();
        let mut should_play = current_file.is_some();

        // Check our own state
        self.check_user_playback(
            &self.our_user_id.clone(),
            current_file,
            &mut blocking,
            &mut should_play,
        );

        // Check connected peers
        for user_id in self.connected_peers.values() {
            self.check_user_playback(user_id, current_file, &mut blocking, &mut should_play);
        }

        self.playback = PlaybackState {
            should_play,
            blocking_users: blocking,
            current_file,
        };
    }

    fn check_user_playback(
        &self,
        user_id: &UserId,
        current_file: Option<FileId>,
        blocking: &mut Vec<UserId>,
        should_play: &mut bool,
    ) {
        let crdt = self.sync_engine.state();
        let user_state = crdt
            .user_states
            .read(user_id)
            .copied()
            .unwrap_or(UserState::Ready);

        match user_state {
            UserState::NotWatching => {} // doesn't block
            UserState::Paused => {
                blocking.push(user_id.clone());
                *should_play = false;
            }
            UserState::Ready => {
                if let Some(fid) = current_file {
                    let file_state = crdt
                        .file_states
                        .read(&(user_id.clone(), fid))
                        .cloned()
                        .unwrap_or(FileState::Ready);
                    if !file_state_permits_play(&file_state) {
                        blocking.push(user_id.clone());
                        *should_play = false;
                    }
                }
            }
        }
    }
}

fn file_state_permits_play(fs: &FileState) -> bool {
    match fs {
        FileState::Ready => true,
        FileState::Missing => false,
        FileState::Downloading { progress } => *progress >= 0.20,
    }
}

fn sync_to_effects(actions: Vec<SyncAction>) -> Vec<AppEffect> {
    if actions.is_empty() {
        vec![]
    } else {
        vec![AppEffect::Sync(actions)]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use dessplay_core::protocol::PeerControl;

    fn uid(s: &str) -> UserId {
        UserId(s.to_string())
    }

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    fn peer(n: u64) -> PeerId {
        PeerId(n)
    }

    fn make_app() -> AppState {
        AppState::new(uid("alice"))
    }

    fn connect_peer(app: &mut AppState, peer_id: u64, username: &str) {
        app.process_event(
            AppEvent::PeerConnected {
                peer_id: peer(peer_id),
                username: username.to_string(),
            },
            100,
        );
    }

    fn add_file(app: &mut AppState, file: u8, now: u64) {
        app.process_event(
            AppEvent::AddToPlaylist {
                file_id: fid(file),
                after: None,
            },
            now,
        );
    }

    fn set_remote_user_state(app: &mut AppState, user: &str, state: UserState, ts: u64) {
        let op = CrdtOp::LwwWrite {
            timestamp: ts,
            value: LwwValue::UserState(uid(user), state),
        };
        app.process_event(AppEvent::RemoteOp { from: peer(0), op }, ts);
    }

    fn set_remote_file_state(
        app: &mut AppState,
        user: &str,
        file: u8,
        state: FileState,
        ts: u64,
    ) {
        let op = CrdtOp::LwwWrite {
            timestamp: ts,
            value: LwwValue::FileState(uid(user), fid(file), state),
        };
        app.process_event(AppEvent::RemoteOp { from: peer(0), op }, ts);
    }

    fn has_sync_effect(effects: &[AppEffect]) -> bool {
        effects.iter().any(|e| matches!(e, AppEffect::Sync(_)))
    }

    fn has_redraw_effect(effects: &[AppEffect]) -> bool {
        effects.iter().any(|e| matches!(e, AppEffect::Redraw))
    }

    // -----------------------------------------------------------------------
    // Derived playback tests
    // -----------------------------------------------------------------------

    #[test]
    fn playback_no_file_no_play() {
        let app = make_app();
        assert!(!app.playback.should_play);
        assert!(app.playback.current_file.is_none());
    }

    #[test]
    fn playback_all_ready() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        // alice defaults to Ready (no explicit state), bob defaults to Ready
        assert!(app.playback.should_play);
        assert!(app.playback.blocking_users.is_empty());
        assert_eq!(app.playback.current_file, Some(fid(1)));
    }

    #[test]
    fn playback_one_paused_blocks() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        // Bob pauses
        set_remote_user_state(&mut app, "bob", UserState::Paused, 200);

        assert!(!app.playback.should_play);
        assert!(app.playback.blocking_users.contains(&uid("bob")));
    }

    #[test]
    fn playback_not_watching_allowed() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        // Bob is not watching
        set_remote_user_state(&mut app, "bob", UserState::NotWatching, 200);

        assert!(app.playback.should_play);
        assert!(app.playback.blocking_users.is_empty());
    }

    #[test]
    fn playback_missing_file_blocks() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        // Bob is Ready but his file is Missing
        set_remote_user_state(&mut app, "bob", UserState::Ready, 200);
        set_remote_file_state(&mut app, "bob", 1, FileState::Missing, 201);

        assert!(!app.playback.should_play);
        assert!(app.playback.blocking_users.contains(&uid("bob")));
    }

    #[test]
    fn playback_downloading_insufficient() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        set_remote_user_state(&mut app, "bob", UserState::Ready, 200);
        set_remote_file_state(
            &mut app,
            "bob",
            1,
            FileState::Downloading { progress: 0.10 },
            201,
        );

        assert!(!app.playback.should_play);
    }

    #[test]
    fn playback_downloading_sufficient() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        set_remote_user_state(&mut app, "bob", UserState::Ready, 200);
        set_remote_file_state(
            &mut app,
            "bob",
            1,
            FileState::Downloading { progress: 0.25 },
            201,
        );

        assert!(app.playback.should_play);
    }

    #[test]
    fn playback_own_state_checked() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);

        // Set ourselves to Paused
        app.process_event(
            AppEvent::SetUserState {
                state: UserState::Paused,
            },
            200,
        );

        assert!(!app.playback.should_play);
        assert!(app.playback.blocking_users.contains(&uid("alice")));
    }

    // -----------------------------------------------------------------------
    // Event processing tests
    // -----------------------------------------------------------------------

    #[test]
    fn send_chat_creates_op() {
        let mut app = make_app();
        let effects = app.process_event(
            AppEvent::SendChat {
                text: "hello".to_string(),
            },
            100,
        );

        assert!(has_sync_effect(&effects));
        assert!(has_redraw_effect(&effects));

        // Verify the chat was applied
        let view = app.sync_engine.state().chat.merged_view();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].1.text, "hello");

        // Second chat increments seq
        app.process_event(
            AppEvent::SendChat {
                text: "world".to_string(),
            },
            101,
        );
        let view = app.sync_engine.state().chat.merged_view();
        assert_eq!(view.len(), 2);
        assert_eq!(view[1].1.seq, 1);
    }

    #[test]
    fn set_user_state_creates_op() {
        let mut app = make_app();
        let effects = app.process_event(
            AppEvent::SetUserState {
                state: UserState::Paused,
            },
            100,
        );

        assert!(has_sync_effect(&effects));

        let state = app
            .sync_engine
            .state()
            .user_states
            .read(&uid("alice"))
            .copied();
        assert_eq!(state, Some(UserState::Paused));
    }

    #[test]
    fn remote_op_updates_state() {
        let mut app = make_app();
        let op = CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::UserState(uid("bob"), UserState::Paused),
        };
        let effects = app.process_event(AppEvent::RemoteOp { from: peer(1), op }, 100);

        assert!(has_redraw_effect(&effects));

        let state = app
            .sync_engine
            .state()
            .user_states
            .read(&uid("bob"))
            .copied();
        assert_eq!(state, Some(UserState::Paused));
    }

    #[test]
    fn peer_connected_tracked() {
        let mut app = make_app();
        app.process_event(
            AppEvent::PeerConnected {
                peer_id: peer(1),
                username: "bob".to_string(),
            },
            100,
        );

        assert_eq!(app.connected_peers.get(&peer(1)), Some(&uid("bob")));
    }

    #[test]
    fn peer_disconnected_removed() {
        let mut app = make_app();
        connect_peer(&mut app, 1, "bob");
        assert!(app.connected_peers.contains_key(&peer(1)));

        app.process_event(AppEvent::PeerDisconnected { peer_id: peer(1) }, 200);

        assert!(!app.connected_peers.contains_key(&peer(1)));
    }

    #[test]
    fn tick_broadcasts_summary() {
        let mut app = make_app();
        let effects = app.process_event(AppEvent::Tick, 100);

        assert!(has_sync_effect(&effects));
        // Verify it contains a BroadcastControl
        for effect in &effects {
            if let AppEffect::Sync(actions) = effect {
                let has_broadcast = actions.iter().any(|a| {
                    matches!(
                        a,
                        SyncAction::BroadcastControl {
                            msg: PeerControl::StateSummary { .. },
                            ..
                        }
                    )
                });
                assert!(has_broadcast, "tick must produce BroadcastControl");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Playback toggle tests
    // -----------------------------------------------------------------------

    #[test]
    fn peer_pause_toggles_playback() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");
        assert!(app.playback.should_play);

        // Bob pauses -> should_play flips to false
        set_remote_user_state(&mut app, "bob", UserState::Paused, 200);
        assert!(!app.playback.should_play);
    }

    #[test]
    fn peer_resume_toggles_playback() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        // Bob pauses
        set_remote_user_state(&mut app, "bob", UserState::Paused, 200);
        assert!(!app.playback.should_play);

        // Bob resumes
        set_remote_user_state(&mut app, "bob", UserState::Ready, 300);
        assert!(app.playback.should_play);
    }
}
