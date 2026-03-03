//! Application state machine: processes events, returns effects.
//!
//! `AppState` is a plain struct with no I/O. All input arrives via `AppEvent`;
//! all output is `Vec<AppEffect>`. This makes it trivially testable — inject
//! events, inspect state and returned effects.

use std::collections::HashMap;

use dessplay_core::protocol::{
    CrdtOp, CrdtSnapshot, GapFillRequest, GapFillResponse, LwwValue, PeerDatagram, PlaylistAction,
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
    SetNowPlaying { file_id: Option<FileId> },

    // --- Player ---
    PlayerPaused,
    PlayerUnpaused,
    PlayerSeeked { position_secs: f64 },
    PlayerPosition { position_secs: f64 },
    PlayerDuration { duration_secs: f64 },
    PlayerEof,
    PlayerCrashed,

    // --- Remote playback ---
    RemotePosition { from: PeerId, position_secs: f64 },
    RemoteSeek { from: PeerId, target_secs: f64 },

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
    /// Player control (Phase 7).
    PlayerPause,
    PlayerUnpause,
    PlayerSeek(f64),
    PlayerLoadFile(FileId),
    PlayerShowOsd(String),
    /// Mark a file as watched (85% rule).
    MarkWatched(FileId),
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

/// Crash recovery state.
#[derive(Clone, Debug)]
struct CrashState {
    /// Timestamp of the last crash.
    last_crash_time: Option<SharedTimestamp>,
    /// Number of consecutive crashes within 30 seconds.
    consecutive_crashes: u32,
}

/// Position broadcast interval during playback (100ms).
const POSITION_BROADCAST_PLAYING_MS: u64 = 100;
/// Position broadcast interval when paused (1s).
const POSITION_BROADCAST_PAUSED_MS: u64 = 1000;
/// Seek debounce window (1500ms) — only broadcast after user stops scrubbing.
const SEEK_DEBOUNCE_MS: u64 = 1500;
/// Sync tolerance — no seek triggered for drift smaller than this.
const SYNC_TOLERANCE_SECS: f64 = 3.0;
/// Crash window — 2nd crash within this triggers global pause.
const CRASH_WINDOW_MS: u64 = 30_000;

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

    // --- Player state ---
    /// Our current playback position in seconds.
    pub our_position_secs: f64,
    /// Duration of the currently loaded file.
    pub file_duration_secs: Option<f64>,
    /// Timestamp of the last position broadcast.
    last_position_broadcast: SharedTimestamp,
    /// Pending seek broadcast: (target_secs, first_seek_time).
    pending_seek_broadcast: Option<(f64, SharedTimestamp)>,
    /// Crash recovery state.
    crash_state: CrashState,
    /// The file currently loaded in the player (if any).
    player_loaded_file: Option<FileId>,
    /// Previous value of should_play (for transition detection).
    prev_should_play: bool,
    /// Remote peer positions (for UI display).
    pub remote_positions: HashMap<PeerId, f64>,
    /// Whether the "watched" marker has been sent for the current file.
    watched_marker_sent: bool,
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
            our_position_secs: 0.0,
            file_duration_secs: None,
            last_position_broadcast: 0,
            pending_seek_broadcast: None,
            crash_state: CrashState {
                last_crash_time: None,
                consecutive_crashes: 0,
            },
            player_loaded_file: None,
            prev_should_play: false,
            remote_positions: HashMap::new(),
            watched_marker_sent: false,
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
            our_position_secs: 0.0,
            file_duration_secs: None,
            last_position_broadcast: 0,
            pending_seek_broadcast: None,
            crash_state: CrashState {
                last_crash_time: None,
                consecutive_crashes: 0,
            },
            player_loaded_file: None,
            prev_should_play: false,
            remote_positions: HashMap::new(),
            watched_marker_sent: false,
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
            AppEvent::SetNowPlaying { file_id } => {
                self.handle_set_now_playing(file_id, now)
            }
            AppEvent::Tick => self.handle_tick(now),

            // Player events
            AppEvent::PlayerPaused => self.handle_player_paused(now),
            AppEvent::PlayerUnpaused => self.handle_player_unpaused(now),
            AppEvent::PlayerSeeked { position_secs } => {
                self.handle_player_seeked(position_secs, now)
            }
            AppEvent::PlayerPosition { position_secs } => {
                self.handle_player_position(position_secs, now)
            }
            AppEvent::PlayerDuration { duration_secs } => {
                self.handle_player_duration(duration_secs)
            }
            AppEvent::PlayerEof => self.handle_player_eof(now),
            AppEvent::PlayerCrashed => self.handle_player_crashed(now),

            // Remote playback
            AppEvent::RemotePosition {
                from,
                position_secs,
            } => self.handle_remote_position(from, position_secs),
            AppEvent::RemoteSeek {
                from: _,
                target_secs,
            } => self.handle_remote_seek(target_secs),
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
        // Check for incoming chat → OSD
        let osd_text = if let CrdtOp::ChatAppend {
            ref user_id,
            ref text,
            ..
        } = op
        {
            Some(format!("{}: {text}", user_id.0))
        } else {
            None
        };

        let actions = self.sync_engine.on_remote_op(from, op);
        let mut effects = Vec::new();
        effects.extend(self.playback_transition_effects());
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        if let Some(text) = osd_text {
            effects.push(AppEffect::PlayerShowOsd(text));
        }
        effects
    }

    fn handle_peer_connected(&mut self, peer_id: PeerId, username: String) -> Vec<AppEffect> {
        self.connected_peers.insert(peer_id, UserId(username));
        let actions = self.sync_engine.on_peer_connected(peer_id);
        let mut effects = Vec::new();
        effects.extend(self.playback_transition_effects());
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_peer_disconnected(&mut self, peer_id: PeerId) -> Vec<AppEffect> {
        self.connected_peers.remove(&peer_id);
        self.remote_positions.remove(&peer_id);
        let actions = self.sync_engine.on_peer_disconnected(peer_id);
        let mut effects = Vec::new();
        effects.extend(self.playback_transition_effects());
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
            let mut effects = Vec::new();
            effects.extend(self.playback_transition_effects());
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
            let mut effects = vec![AppEffect::Sync(actions)];
            effects.extend(self.playback_transition_effects());
            return effects;
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
        let mut effects = Vec::new();
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
        let mut effects = Vec::new();
        effects.extend(self.playback_transition_effects());
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
        let mut effects = Vec::new();
        effects.extend(self.playback_transition_effects());
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
        let was_empty = self
            .sync_engine
            .state()
            .now_playing
            .read(&())
            .and_then(|val| *val)
            .is_none();

        let op = CrdtOp::PlaylistOp {
            timestamp: now,
            action: PlaylistAction::Add { file_id, after },
        };
        let actions = self.sync_engine.apply_local_op(op);
        let mut effects = Vec::new();

        // Auto-select the first file added to an empty playlist
        if was_empty {
            let np_op = CrdtOp::LwwWrite {
                timestamp: now,
                value: LwwValue::NowPlaying(Some(file_id)),
            };
            let np_actions = self.sync_engine.apply_local_op(np_op);
            if !np_actions.is_empty() {
                effects.push(AppEffect::Sync(np_actions));
            }
        }

        effects.extend(self.playback_transition_effects());
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
        let mut effects = Vec::new();
        effects.extend(self.playback_transition_effects());
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
        let mut effects = Vec::new();
        effects.extend(self.playback_transition_effects());
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_set_now_playing(
        &mut self,
        file_id: Option<FileId>,
        now: SharedTimestamp,
    ) -> Vec<AppEffect> {
        let op = CrdtOp::LwwWrite {
            timestamp: now,
            value: LwwValue::NowPlaying(file_id),
        };
        let actions = self.sync_engine.apply_local_op(op);

        // Reset player state for new file
        self.our_position_secs = 0.0;
        self.file_duration_secs = None;
        self.watched_marker_sent = false;

        let mut effects = Vec::new();
        effects.extend(self.playback_transition_effects());
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_tick(&self, now: SharedTimestamp) -> Vec<AppEffect> {
        let _ = now; // used in future for periodic checks
        let actions = self.sync_engine.on_periodic_tick();
        vec![AppEffect::Sync(actions)]
    }

    // -----------------------------------------------------------------------
    // Player event handlers
    // -----------------------------------------------------------------------

    fn handle_player_paused(&mut self, now: SharedTimestamp) -> Vec<AppEffect> {
        // Player was paused (by the local user via mpv controls)
        let op = CrdtOp::LwwWrite {
            timestamp: now,
            value: LwwValue::UserState(self.our_user_id.clone(), UserState::Paused),
        };
        let actions = self.sync_engine.apply_local_op(op);
        let mut effects = Vec::new();
        effects.extend(self.playback_transition_effects());
        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_player_unpaused(&mut self, now: SharedTimestamp) -> Vec<AppEffect> {
        // Player was unpaused (user pressed play in mpv)
        // Set our state to Ready
        let op = CrdtOp::LwwWrite {
            timestamp: now,
            value: LwwValue::UserState(self.our_user_id.clone(), UserState::Ready),
        };
        let actions = self.sync_engine.apply_local_op(op);

        let mut effects = Vec::new();

        // If should_play is false, we must immediately re-pause
        // (The user tried to play, but others are blocking)
        if !self.playback.should_play {
            effects.push(AppEffect::PlayerPause);
        }

        // Don't add transition effects here — we handle the re-pause explicitly above
        self.recompute_playback();

        if !actions.is_empty() {
            effects.push(AppEffect::Sync(actions));
        }
        effects
    }

    fn handle_player_seeked(&mut self, position_secs: f64, now: SharedTimestamp) -> Vec<AppEffect> {
        self.our_position_secs = position_secs;

        // Set pending seek broadcast (debounced)
        if self.pending_seek_broadcast.is_none() {
            self.pending_seek_broadcast = Some((position_secs, now));
        } else {
            // Update target, keep original timestamp for debounce
            self.pending_seek_broadcast = self
                .pending_seek_broadcast
                .map(|(_, first_time)| (position_secs, first_time));
        }

        Vec::new()
    }

    fn handle_player_position(
        &mut self,
        position_secs: f64,
        now: SharedTimestamp,
    ) -> Vec<AppEffect> {
        self.our_position_secs = position_secs;

        let mut effects = Vec::new();

        // Watch tracking: emit MarkWatched at 85% through the file
        if !self.watched_marker_sent
            && let (Some(duration), Some(file_id)) =
                (self.file_duration_secs, self.playback.current_file)
            && duration > 0.0
            && position_secs / duration >= 0.85
        {
            self.watched_marker_sent = true;
            effects.push(AppEffect::MarkWatched(file_id));
        }

        // Check position broadcast timer
        let interval = if self.playback.should_play {
            POSITION_BROADCAST_PLAYING_MS
        } else {
            POSITION_BROADCAST_PAUSED_MS
        };

        if now.saturating_sub(self.last_position_broadcast) >= interval {
            self.last_position_broadcast = now;
            effects.push(AppEffect::Sync(vec![SyncAction::BroadcastDatagram {
                msg: PeerDatagram::Position {
                    timestamp: now,
                    position_secs,
                },
            }]));
        }

        // Check seek debounce
        if let Some((target, first_time)) = self.pending_seek_broadcast
            && now.saturating_sub(first_time) >= SEEK_DEBOUNCE_MS
        {
            self.pending_seek_broadcast = None;
            effects.push(AppEffect::Sync(vec![SyncAction::BroadcastDatagram {
                msg: PeerDatagram::Seek {
                    timestamp: now,
                    target_secs: target,
                },
            }]));
        }

        effects
    }

    fn handle_player_duration(&mut self, duration_secs: f64) -> Vec<AppEffect> {
        self.file_duration_secs = Some(duration_secs);
        Vec::new()
    }

    fn handle_player_eof(&mut self, now: SharedTimestamp) -> Vec<AppEffect> {
        let mut effects = Vec::new();

        if let Some(current) = self.playback.current_file {
            // Find the next file in the playlist after the current one
            let playlist = self.sync_engine.state().playlist.snapshot();
            let next_file = playlist
                .iter()
                .position(|fid| *fid == current)
                .and_then(|pos| playlist.get(pos + 1).copied());

            // Advance now_playing to the next file (or None if at end)
            let op = CrdtOp::LwwWrite {
                timestamp: now,
                value: LwwValue::NowPlaying(next_file),
            };
            let actions = self.sync_engine.apply_local_op(op);
            if !actions.is_empty() {
                effects.push(AppEffect::Sync(actions));
            }

            self.our_position_secs = 0.0;
            self.file_duration_secs = None;
            self.player_loaded_file = None;
            self.watched_marker_sent = false;

            // Recompute — if there's a next file, load it
            self.recompute_playback();
            if let Some(next_file) = self.playback.current_file {
                effects.push(AppEffect::PlayerLoadFile(next_file));
                self.player_loaded_file = Some(next_file);
            }
        }

        effects
    }

    fn handle_player_crashed(&mut self, now: SharedTimestamp) -> Vec<AppEffect> {
        let mut effects = Vec::new();

        let is_rapid_crash = self
            .crash_state
            .last_crash_time
            .is_some_and(|last| now.saturating_sub(last) < CRASH_WINDOW_MS);

        if is_rapid_crash {
            self.crash_state.consecutive_crashes += 1;
        } else {
            self.crash_state.consecutive_crashes = 1;
        }
        self.crash_state.last_crash_time = Some(now);

        if self.crash_state.consecutive_crashes >= 2 {
            // Second crash within window → global pause + system chat
            let op = CrdtOp::LwwWrite {
                timestamp: now,
                value: LwwValue::UserState(self.our_user_id.clone(), UserState::Paused),
            };
            let actions = self.sync_engine.apply_local_op(op);
            effects.extend(self.playback_transition_effects());
            if !actions.is_empty() {
                effects.push(AppEffect::Sync(actions));
            }

            let chat_text = format!(
                "[system] {}'s player crashed repeatedly — pausing",
                self.our_user_id.0
            );
            let seq = self.chat_seq;
            self.chat_seq += 1;
            let chat_op = CrdtOp::ChatAppend {
                user_id: self.our_user_id.clone(),
                seq,
                timestamp: now,
                text: chat_text,
            };
            let chat_actions = self.sync_engine.apply_local_op(chat_op);
            if !chat_actions.is_empty() {
                effects.push(AppEffect::Sync(chat_actions));
            }
        } else {
            // First crash → relaunch and seek to last position
            if let Some(file_id) = self.player_loaded_file {
                effects.push(AppEffect::PlayerLoadFile(file_id));
                if self.our_position_secs > 0.0 {
                    effects.push(AppEffect::PlayerSeek(self.our_position_secs));
                }
            }
        }

        effects
    }

    fn handle_remote_position(&mut self, from: PeerId, position_secs: f64) -> Vec<AppEffect> {
        self.remote_positions.insert(from, position_secs);
        // No playback effect — just store for UI
        vec![]
    }

    fn handle_remote_seek(&mut self, target_secs: f64) -> Vec<AppEffect> {
        if (self.our_position_secs - target_secs).abs() > SYNC_TOLERANCE_SECS {
            vec![AppEffect::PlayerSeek(target_secs)]
        } else {
            vec![]
        }
    }

    // -----------------------------------------------------------------------
    // Derived playback
    // -----------------------------------------------------------------------

    /// Recompute playback state and return transition effects.
    ///
    /// Call this instead of bare `recompute_playback()` whenever a handler
    /// may change `should_play` or `current_file`.
    fn playback_transition_effects(&mut self) -> Vec<AppEffect> {
        let old_should_play = self.prev_should_play;
        let old_file = self.player_loaded_file;

        self.recompute_playback();

        let new_should_play = self.playback.should_play;
        let new_file = self.playback.current_file;

        let mut effects = Vec::new();

        // File changed → load new file
        if new_file != old_file
            && let Some(file_id) = new_file
        {
            effects.push(AppEffect::PlayerLoadFile(file_id));
            self.player_loaded_file = Some(file_id);
            self.our_position_secs = 0.0;
            self.file_duration_secs = None;
            self.watched_marker_sent = false;
        }

        // should_play transitions
        if new_should_play && !old_should_play {
            effects.push(AppEffect::PlayerUnpause);
        } else if !new_should_play && old_should_play {
            effects.push(AppEffect::PlayerPause);
        }

        self.prev_should_play = new_should_play;

        effects
    }

    fn recompute_playback(&mut self) {
        let crdt = self.sync_engine.state();
        let current_file = crdt
            .now_playing
            .read(&())
            .and_then(|val| *val);

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
        let _effects = app.process_event(AppEvent::RemoteOp { from: peer(1), op }, 100);

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

    // -----------------------------------------------------------------------
    // Player event tests
    // -----------------------------------------------------------------------

    fn has_player_pause(effects: &[AppEffect]) -> bool {
        effects.iter().any(|e| matches!(e, AppEffect::PlayerPause))
    }

    fn has_player_unpause(effects: &[AppEffect]) -> bool {
        effects
            .iter()
            .any(|e| matches!(e, AppEffect::PlayerUnpause))
    }

    fn has_player_load(effects: &[AppEffect]) -> bool {
        effects
            .iter()
            .any(|e| matches!(e, AppEffect::PlayerLoadFile(_)))
    }

    fn has_player_seek(effects: &[AppEffect]) -> bool {
        effects
            .iter()
            .any(|e| matches!(e, AppEffect::PlayerSeek(_)))
    }

    fn has_player_osd(effects: &[AppEffect]) -> bool {
        effects
            .iter()
            .any(|e| matches!(e, AppEffect::PlayerShowOsd(_)))
    }

    #[test]
    fn player_paused_sets_user_state() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);

        let effects = app.process_event(AppEvent::PlayerPaused, 200);

        // Should set our user state to Paused
        let state = app
            .sync_engine
            .state()
            .user_states
            .read(&uid("alice"))
            .copied();
        assert_eq!(state, Some(UserState::Paused));
        assert!(has_sync_effect(&effects));
    }

    #[test]
    fn player_unpaused_sets_ready() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);

        // First pause
        app.process_event(AppEvent::PlayerPaused, 200);

        // Then unpause — but since we're the only blocker and we just went Ready,
        // should_play should become true
        let effects = app.process_event(AppEvent::PlayerUnpaused, 300);

        let state = app
            .sync_engine
            .state()
            .user_states
            .read(&uid("alice"))
            .copied();
        assert_eq!(state, Some(UserState::Ready));
        assert!(has_sync_effect(&effects));
    }

    #[test]
    fn player_unpaused_re_pauses_when_blocked() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        // Bob pauses → should_play is false
        set_remote_user_state(&mut app, "bob", UserState::Paused, 200);
        assert!(!app.playback.should_play);

        // We try to unpause → must be re-paused
        let effects = app.process_event(AppEvent::PlayerUnpaused, 300);
        assert!(has_player_pause(&effects));

        // But our state is still Ready (we tried!)
        let state = app
            .sync_engine
            .state()
            .user_states
            .read(&uid("alice"))
            .copied();
        assert_eq!(state, Some(UserState::Ready));
    }

    #[test]
    fn player_seeked_updates_position() {
        let mut app = make_app();
        app.process_event(AppEvent::PlayerSeeked { position_secs: 42.0 }, 100);
        assert!((app.our_position_secs - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn player_seeked_debounces_broadcast() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);

        // First seek sets pending
        app.process_event(AppEvent::PlayerSeeked { position_secs: 10.0 }, 1000);
        assert!(app.pending_seek_broadcast.is_some());

        // Position tick before debounce window → no broadcast
        let effects = app.process_event(AppEvent::PlayerPosition { position_secs: 10.0 }, 1100);
        let has_seek_datagram = effects.iter().any(|e| {
            if let AppEffect::Sync(actions) = e {
                actions.iter().any(|a| {
                    matches!(
                        a,
                        SyncAction::BroadcastDatagram {
                            msg: PeerDatagram::Seek { .. }
                        }
                    )
                })
            } else {
                false
            }
        });
        assert!(!has_seek_datagram);

        // After debounce window → broadcast
        let effects = app.process_event(AppEvent::PlayerPosition { position_secs: 10.0 }, 2600);
        let has_seek_datagram = effects.iter().any(|e| {
            if let AppEffect::Sync(actions) = e {
                actions.iter().any(|a| {
                    matches!(
                        a,
                        SyncAction::BroadcastDatagram {
                            msg: PeerDatagram::Seek { .. }
                        }
                    )
                })
            } else {
                false
            }
        });
        assert!(has_seek_datagram);
        assert!(app.pending_seek_broadcast.is_none());
    }

    #[test]
    fn player_position_broadcasts_periodically() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);

        // First position at t=1000 → broadcasts (never sent before)
        let effects = app.process_event(AppEvent::PlayerPosition { position_secs: 5.0 }, 1000);
        let has_pos_datagram = effects.iter().any(|e| {
            if let AppEffect::Sync(actions) = e {
                actions.iter().any(|a| {
                    matches!(
                        a,
                        SyncAction::BroadcastDatagram {
                            msg: PeerDatagram::Position { .. }
                        }
                    )
                })
            } else {
                false
            }
        });
        assert!(has_pos_datagram);

        // 50ms later → too soon, no broadcast
        let effects = app.process_event(AppEvent::PlayerPosition { position_secs: 5.1 }, 1050);
        let has_pos_datagram = effects.iter().any(|e| {
            if let AppEffect::Sync(actions) = e {
                actions.iter().any(|a| {
                    matches!(
                        a,
                        SyncAction::BroadcastDatagram {
                            msg: PeerDatagram::Position { .. }
                        }
                    )
                })
            } else {
                false
            }
        });
        assert!(!has_pos_datagram);
    }

    #[test]
    fn player_duration_stored() {
        let mut app = make_app();
        let _effects = app.process_event(AppEvent::PlayerDuration { duration_secs: 1440.0 }, 100);
        assert_eq!(app.file_duration_secs, Some(1440.0));
    }

    #[test]
    fn player_eof_advances_now_playing() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        add_file(&mut app, 2, 101);

        assert_eq!(app.playback.current_file, Some(fid(1)));

        let effects = app.process_event(AppEvent::PlayerEof, 200);
        // now_playing advances to file 2, file 1 stays in playlist
        assert_eq!(app.playback.current_file, Some(fid(2)));
        assert!(has_player_load(&effects));
        // File 1 is still in the playlist
        let playlist = app.sync_engine.state().playlist.snapshot();
        assert!(playlist.contains(&fid(1)), "file should stay in playlist after EOF");
        assert!(playlist.contains(&fid(2)));
    }

    #[test]
    fn player_eof_last_file_clears() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);

        app.process_event(AppEvent::PlayerEof, 200);
        // now_playing becomes None, but file stays in playlist
        assert!(app.playback.current_file.is_none());
        let playlist = app.sync_engine.state().playlist.snapshot();
        assert!(playlist.contains(&fid(1)), "file should stay in playlist after EOF");
    }

    #[test]
    fn player_crash_first_relaunches() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        app.player_loaded_file = Some(fid(1));
        app.our_position_secs = 42.0;

        let effects = app.process_event(AppEvent::PlayerCrashed, 1000);
        assert!(has_player_load(&effects));
        assert!(has_player_seek(&effects));
    }

    #[test]
    fn player_crash_second_pauses_globally() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        app.player_loaded_file = Some(fid(1));

        // First crash
        app.process_event(AppEvent::PlayerCrashed, 1000);

        // Second crash within 30s → global pause + chat
        let effects = app.process_event(AppEvent::PlayerCrashed, 1500);
        assert!(!has_player_load(&effects));

        // Should have set Paused
        let state = app
            .sync_engine
            .state()
            .user_states
            .read(&uid("alice"))
            .copied();
        assert_eq!(state, Some(UserState::Paused));
    }

    #[test]
    fn player_crash_resets_after_window() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        app.player_loaded_file = Some(fid(1));

        // First crash
        app.process_event(AppEvent::PlayerCrashed, 1000);

        // Second crash after 30s window → treated as first
        let effects = app.process_event(AppEvent::PlayerCrashed, 35_000);
        assert!(has_player_load(&effects));
    }

    #[test]
    fn remote_seek_within_tolerance_ignored() {
        let mut app = make_app();
        app.our_position_secs = 100.0;

        let effects = app.process_event(
            AppEvent::RemoteSeek {
                from: peer(1),
                target_secs: 101.0,
            },
            200,
        );
        assert!(!has_player_seek(&effects));
    }

    #[test]
    fn remote_seek_outside_tolerance_applies() {
        let mut app = make_app();
        app.our_position_secs = 100.0;

        let effects = app.process_event(
            AppEvent::RemoteSeek {
                from: peer(1),
                target_secs: 200.0,
            },
            200,
        );
        assert!(has_player_seek(&effects));
    }

    #[test]
    fn remote_position_stored() {
        let mut app = make_app();
        app.process_event(
            AppEvent::RemotePosition {
                from: peer(1),
                position_secs: 55.0,
            },
            100,
        );
        assert_eq!(app.remote_positions.get(&peer(1)), Some(&55.0));
    }

    #[test]
    fn remote_chat_emits_osd() {
        let mut app = make_app();
        connect_peer(&mut app, 1, "bob");

        let op = CrdtOp::ChatAppend {
            user_id: uid("bob"),
            seq: 0,
            timestamp: 200,
            text: "hello everyone".to_string(),
        };
        let effects = app.process_event(AppEvent::RemoteOp { from: peer(1), op }, 200);
        assert!(has_player_osd(&effects));
    }

    #[test]
    fn should_play_transition_emits_effects() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        // Initially should_play is true after adding file + peer
        // Sync prev_should_play
        app.prev_should_play = true;

        // Bob pauses → should emit PlayerPause
        let effects = set_remote_user_state_effects(&mut app, "bob", UserState::Paused, 200);
        assert!(has_player_pause(&effects));

        // Bob resumes → should emit PlayerUnpause
        let effects = set_remote_user_state_effects(&mut app, "bob", UserState::Ready, 300);
        assert!(has_player_unpause(&effects));
    }

    fn set_remote_user_state_effects(
        app: &mut AppState,
        user: &str,
        state: UserState,
        ts: u64,
    ) -> Vec<AppEffect> {
        let op = CrdtOp::LwwWrite {
            timestamp: ts,
            value: LwwValue::UserState(uid(user), state),
        };
        app.process_event(AppEvent::RemoteOp { from: peer(0), op }, ts)
    }

    // -----------------------------------------------------------------------
    // Watch tracking tests
    // -----------------------------------------------------------------------

    fn has_mark_watched(effects: &[AppEffect]) -> bool {
        effects
            .iter()
            .any(|e| matches!(e, AppEffect::MarkWatched(_)))
    }

    #[test]
    fn watch_84_percent_no_mark() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        app.file_duration_secs = Some(100.0);

        let effects = app.process_event(AppEvent::PlayerPosition { position_secs: 84.0 }, 1000);
        assert!(!has_mark_watched(&effects));
        assert!(!app.watched_marker_sent);
    }

    #[test]
    fn watch_85_percent_marks() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        app.file_duration_secs = Some(100.0);

        let effects = app.process_event(AppEvent::PlayerPosition { position_secs: 85.0 }, 1000);
        assert!(has_mark_watched(&effects));
        assert!(app.watched_marker_sent);
    }

    #[test]
    fn watch_only_emits_once() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        app.file_duration_secs = Some(100.0);

        // First at 85%
        let effects = app.process_event(AppEvent::PlayerPosition { position_secs: 85.0 }, 1000);
        assert!(has_mark_watched(&effects));

        // Second at 90% — no duplicate
        let effects = app.process_event(AppEvent::PlayerPosition { position_secs: 90.0 }, 1100);
        assert!(!has_mark_watched(&effects));
    }

    #[test]
    fn watch_resets_on_file_change() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        add_file(&mut app, 2, 101);
        app.file_duration_secs = Some(100.0);

        // Mark first file watched
        app.process_event(AppEvent::PlayerPosition { position_secs: 85.0 }, 1000);
        assert!(app.watched_marker_sent);

        // EOF advances to next file, should reset marker
        app.process_event(AppEvent::PlayerEof, 2000);
        assert!(!app.watched_marker_sent);
    }

    // -----------------------------------------------------------------------
    // Known/unknown series detection tests (9.4)
    // -----------------------------------------------------------------------

    #[test]
    fn set_file_state_missing_blocks_playback() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        assert!(app.playback.should_play);

        // Set our file state to Missing → should block
        app.process_event(
            AppEvent::SetFileState {
                file_id: fid(1),
                state: FileState::Missing,
            },
            200,
        );

        let our_fs = app
            .sync_engine
            .state()
            .file_states
            .read(&(uid("alice"), fid(1)))
            .cloned();
        assert_eq!(our_fs, Some(FileState::Missing));
        assert!(!app.playback.should_play);
        assert!(app.playback.blocking_users.contains(&uid("alice")));
    }

    #[test]
    fn set_user_state_not_watching_allows_playback() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        connect_peer(&mut app, 1, "bob");

        // Set alice to NotWatching → doesn't block (even with Missing file)
        app.process_event(
            AppEvent::SetFileState {
                file_id: fid(1),
                state: FileState::Missing,
            },
            200,
        );
        assert!(!app.playback.should_play);

        app.process_event(
            AppEvent::SetUserState {
                state: UserState::NotWatching,
            },
            201,
        );

        let our_us = app
            .sync_engine
            .state()
            .user_states
            .read(&uid("alice"))
            .copied();
        assert_eq!(our_us, Some(UserState::NotWatching));
        // NotWatching doesn't block, so playback should resume
        assert!(app.playback.should_play);
    }

    #[test]
    fn file_state_ready_clears_missing() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);

        // Set Missing
        app.process_event(
            AppEvent::SetFileState {
                file_id: fid(1),
                state: FileState::Missing,
            },
            200,
        );
        assert!(!app.playback.should_play);

        // Clear Missing → Ready
        app.process_event(
            AppEvent::SetFileState {
                file_id: fid(1),
                state: FileState::Ready,
            },
            300,
        );
        let our_fs = app
            .sync_engine
            .state()
            .file_states
            .read(&(uid("alice"), fid(1)))
            .cloned();
        assert_eq!(our_fs, Some(FileState::Ready));
        assert!(app.playback.should_play);
    }

    // -----------------------------------------------------------------------
    // Now-playing tests
    // -----------------------------------------------------------------------

    #[test]
    fn set_now_playing_changes_current() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        add_file(&mut app, 2, 101);
        assert_eq!(app.playback.current_file, Some(fid(1)));

        // Switch to file 2
        app.process_event(
            AppEvent::SetNowPlaying {
                file_id: Some(fid(2)),
            },
            200,
        );
        assert_eq!(app.playback.current_file, Some(fid(2)));
    }

    #[test]
    fn eof_advances_to_next() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        add_file(&mut app, 2, 101);
        add_file(&mut app, 3, 102);
        assert_eq!(app.playback.current_file, Some(fid(1)));

        // EOF on first → advances to second
        app.process_event(AppEvent::PlayerEof, 200);
        assert_eq!(app.playback.current_file, Some(fid(2)));

        // All three files still in playlist
        let playlist = app.sync_engine.state().playlist.snapshot();
        assert_eq!(playlist.len(), 3);
    }

    #[test]
    fn eof_on_last_clears() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        add_file(&mut app, 2, 101);

        // Jump to last file
        app.process_event(
            AppEvent::SetNowPlaying {
                file_id: Some(fid(2)),
            },
            150,
        );
        assert_eq!(app.playback.current_file, Some(fid(2)));

        // EOF on last → None
        app.process_event(AppEvent::PlayerEof, 200);
        assert!(app.playback.current_file.is_none());
    }

    #[test]
    fn first_add_auto_selects() {
        let mut app = make_app();
        // Empty playlist, now_playing is None
        assert!(app.playback.current_file.is_none());

        add_file(&mut app, 1, 100);
        // First add should auto-set now_playing
        assert_eq!(app.playback.current_file, Some(fid(1)));
    }

    #[test]
    fn second_add_no_change() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        assert_eq!(app.playback.current_file, Some(fid(1)));

        // Adding second file should NOT change now_playing
        add_file(&mut app, 2, 101);
        assert_eq!(app.playback.current_file, Some(fid(1)));
    }

    // Regression: playlist remove/move must return Sync effects so they
    // are broadcast to peers. Previously, the runner discarded these.
    #[test]
    fn playlist_remove_returns_sync_effects() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        let effects = app.process_event(AppEvent::RemoveFromPlaylist { file_id: fid(1) }, 200);
        assert!(has_sync_effect(&effects), "RemoveFromPlaylist must produce Sync effects");
    }

    #[test]
    fn playlist_move_returns_sync_effects() {
        let mut app = make_app();
        add_file(&mut app, 1, 100);
        add_file(&mut app, 2, 200);
        let effects = app.process_event(
            AppEvent::MoveInPlaylist {
                file_id: fid(1),
                after: Some(fid(2)),
            },
            300,
        );
        assert!(has_sync_effect(&effects), "MoveInPlaylist must produce Sync effects");
    }
}
