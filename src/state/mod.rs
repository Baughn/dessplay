pub mod playlist;
pub mod types;

use std::collections::HashMap;
use std::sync::RwLock;

use tokio::sync::watch;

use crate::network::clock::SharedTimestamp;
use crate::network::PeerId;

use self::playlist::{replay_playlist, TimestampedAction};
use self::types::*;

/// A consistent snapshot of the shared state for rendering.
///
/// Produced by `SharedState::view()`. Contains resolved/derived data
/// ready for display.
#[derive(Debug, Clone)]
pub struct StateView {
    /// Resolved playlist (replayed from action log).
    pub playlist: Vec<PlaylistItem>,
    /// Currently loaded file.
    pub current_file: Option<ItemId>,
    /// Playback position in seconds.
    pub position: f64,
    /// Position timestamp (for LWW comparison).
    pub position_timestamp: SharedTimestamp,
    /// Per-peer user states.
    pub user_states: HashMap<PeerId, UserState>,
    /// Per-peer file states.
    pub file_states: HashMap<PeerId, FileState>,
    /// Derived ready states for UI display.
    pub ready_states: HashMap<PeerId, ReadyState>,
    /// Whether playback should be active (derived from all user/file states).
    pub is_playing: bool,
    /// Chat messages sorted by timestamp.
    pub chat_messages: Vec<ChatMessage>,
    /// Connected peers.
    pub peers: Vec<PeerId>,
}

/// Inner mutable state behind the RwLock.
struct SharedStateInner {
    /// Player state (LWW).
    player_state: PlayerStateSnapshot,
    /// Per-peer user states with timestamps (LWW).
    user_states: HashMap<PeerId, (UserState, SharedTimestamp)>,
    /// Per-peer file states with timestamps (LWW).
    file_states: HashMap<PeerId, (FileState, SharedTimestamp)>,
    /// All playlist actions from all users.
    playlist_actions: Vec<TimestampedAction>,
    /// All chat messages from all users.
    chat_messages: Vec<ChatMessage>,
    /// Connected peers.
    peers: Vec<PeerId>,
    /// Monotonic version counter for change detection.
    version: u64,
}

/// Canonical application state shared between the sync engine and the TUI.
///
/// The sync engine writes to this; the TUI reads via `view()`.
/// Change notification is via a `watch` channel carrying a version counter.
pub struct SharedState {
    inner: RwLock<SharedStateInner>,
    version_tx: watch::Sender<u64>,
}

impl SharedState {
    /// Create a new shared state and return the change notification receiver.
    pub fn new() -> (Self, watch::Receiver<u64>) {
        let (version_tx, version_rx) = watch::channel(0u64);
        let state = Self {
            inner: RwLock::new(SharedStateInner {
                player_state: PlayerStateSnapshot {
                    file_id: None,
                    position: PositionRegister {
                        position: 0.0,
                        timestamp: SharedTimestamp(0),
                    },
                },
                user_states: HashMap::new(),
                file_states: HashMap::new(),
                playlist_actions: Vec::new(),
                chat_messages: Vec::new(),
                peers: Vec::new(),
                version: 0,
            }),
            version_tx,
        };
        (state, version_rx)
    }

    /// Produce a consistent snapshot for rendering.
    pub fn view(&self) -> StateView {
        let inner = self.inner.read().unwrap();

        let playlist = replay_playlist(&inner.playlist_actions);

        let user_states: HashMap<PeerId, UserState> = inner
            .user_states
            .iter()
            .map(|(k, (v, _))| (k.clone(), *v))
            .collect();

        let file_states: HashMap<PeerId, FileState> = inner
            .file_states
            .iter()
            .map(|(k, (v, _))| (k.clone(), *v))
            .collect();

        let ready_states: HashMap<PeerId, ReadyState> = inner
            .peers
            .iter()
            .map(|peer| {
                let us = user_states
                    .get(peer)
                    .copied()
                    .unwrap_or(UserState::Ready);
                let fs = file_states
                    .get(peer)
                    .copied()
                    .unwrap_or(FileState::Ready);
                (peer.clone(), ReadyState::derive(us, fs))
            })
            .collect();

        let is_playing = compute_is_playing(&inner.peers, &user_states, &file_states);

        let mut chat_messages = inner.chat_messages.clone();
        // Sort by timestamp, breaking ties by sender for determinism.
        chat_messages.sort_by(|a, b| {
            a.timestamp.cmp(&b.timestamp)
                .then_with(|| a.sender.0.cmp(&b.sender.0))
        });

        StateView {
            playlist,
            current_file: inner.player_state.file_id.clone(),
            position: inner.player_state.position.position,
            position_timestamp: inner.player_state.position.timestamp,
            user_states,
            file_states,
            ready_states,
            is_playing,
            chat_messages,
            peers: inner.peers.clone(),
        }
    }

    /// Update the player state (file + position) if the new timestamp is newer or equal.
    /// Returns true if the state was updated.
    pub fn update_player_state(&self, state: PlayerStateSnapshot) -> bool {
        let mut inner = self.inner.write().unwrap();
        if state.position.timestamp >= inner.player_state.position.timestamp {
            inner.player_state = state;
            self.bump_version(&mut inner);
            true
        } else {
            false
        }
    }

    /// Set a peer's user state if the timestamp is newer or equal.
    /// Returns true if the state was updated.
    pub fn set_user_state(
        &self,
        peer: &PeerId,
        state: UserState,
        timestamp: SharedTimestamp,
    ) -> bool {
        let mut inner = self.inner.write().unwrap();
        let entry = inner.user_states.entry(peer.clone()).or_insert((
            UserState::Ready,
            SharedTimestamp(0),
        ));
        if timestamp >= entry.1 {
            *entry = (state, timestamp);
            self.bump_version(&mut inner);
            true
        } else {
            false
        }
    }

    /// Set a peer's file state if the timestamp is newer or equal.
    /// Returns true if the state was updated.
    pub fn set_file_state(
        &self,
        peer: &PeerId,
        state: FileState,
        timestamp: SharedTimestamp,
    ) -> bool {
        let mut inner = self.inner.write().unwrap();
        let entry = inner.file_states.entry(peer.clone()).or_insert((
            FileState::Ready,
            SharedTimestamp(0),
        ));
        if timestamp >= entry.1 {
            *entry = (state, timestamp);
            self.bump_version(&mut inner);
            true
        } else {
            false
        }
    }

    /// Add a chat message (idempotent — duplicates by sender+timestamp are ignored).
    pub fn add_chat_message(&self, msg: ChatMessage) {
        let mut inner = self.inner.write().unwrap();
        // Deduplicate by sender + timestamp
        let exists = inner.chat_messages.iter().any(|m| {
            m.sender == msg.sender && m.timestamp == msg.timestamp
        });
        if !exists {
            inner.chat_messages.push(msg);
            self.bump_version(&mut inner);
        }
    }

    /// Add a playlist action with its timestamp.
    pub fn add_playlist_action(&self, action: PlaylistAction, timestamp: SharedTimestamp) {
        let mut inner = self.inner.write().unwrap();
        inner.playlist_actions.push(TimestampedAction {
            action,
            timestamp,
        });
        self.bump_version(&mut inner);
    }

    /// Register a new peer.
    pub fn add_peer(&self, peer: PeerId) {
        let mut inner = self.inner.write().unwrap();
        if !inner.peers.contains(&peer) {
            inner.peers.push(peer);
            self.bump_version(&mut inner);
        }
    }

    /// Remove a peer and clean up their state.
    pub fn remove_peer(&self, peer: &PeerId) {
        let mut inner = self.inner.write().unwrap();
        inner.peers.retain(|p| p != peer);
        inner.user_states.remove(peer);
        inner.file_states.remove(peer);
        self.bump_version(&mut inner);
    }

    /// Get raw user states with timestamps (for sync engine snapshot building).
    pub fn raw_user_states(&self) -> HashMap<PeerId, (UserState, SharedTimestamp)> {
        self.inner.read().unwrap().user_states.clone()
    }

    /// Get raw file states with timestamps (for sync engine snapshot building).
    pub fn raw_file_states(&self) -> HashMap<PeerId, (FileState, SharedTimestamp)> {
        self.inner.read().unwrap().file_states.clone()
    }

    /// Get the current player state snapshot (for sync engine snapshot building).
    pub fn player_state(&self) -> PlayerStateSnapshot {
        self.inner.read().unwrap().player_state.clone()
    }

    fn bump_version(&self, inner: &mut SharedStateInner) {
        inner.version += 1;
        let _ = self.version_tx.send(inner.version);
    }
}

/// Derived pause logic: play iff every peer is Ready/NotWatching AND
/// their file state permits playback.
fn compute_is_playing(
    peers: &[PeerId],
    user_states: &HashMap<PeerId, UserState>,
    file_states: &HashMap<PeerId, FileState>,
) -> bool {
    if peers.is_empty() {
        return false;
    }

    peers.iter().all(|peer| {
        let us = user_states.get(peer).copied().unwrap_or(UserState::Ready);
        let fs = file_states.get(peer).copied().unwrap_or(FileState::Ready);

        match us {
            UserState::NotWatching => true,
            UserState::Paused => false,
            UserState::Ready => fs.permits_playback(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(name: &str) -> PeerId {
        PeerId(name.to_string())
    }

    #[test]
    fn empty_state_not_playing() {
        let (state, _rx) = SharedState::new();
        let view = state.view();
        assert!(!view.is_playing);
    }

    #[test]
    fn single_ready_peer_is_playing() {
        let (state, _rx) = SharedState::new();
        state.add_peer(peer("alice"));
        state.set_user_state(&peer("alice"), UserState::Ready, SharedTimestamp(1));
        state.set_file_state(&peer("alice"), FileState::Ready, SharedTimestamp(1));
        let view = state.view();
        assert!(view.is_playing);
    }

    #[test]
    fn paused_peer_stops_playback() {
        let (state, _rx) = SharedState::new();
        state.add_peer(peer("alice"));
        state.add_peer(peer("bob"));
        state.set_user_state(&peer("alice"), UserState::Ready, SharedTimestamp(1));
        state.set_user_state(&peer("bob"), UserState::Paused, SharedTimestamp(1));
        let view = state.view();
        assert!(!view.is_playing);
    }

    #[test]
    fn not_watching_excluded_from_pause() {
        let (state, _rx) = SharedState::new();
        state.add_peer(peer("alice"));
        state.add_peer(peer("bob"));
        state.set_user_state(&peer("alice"), UserState::Ready, SharedTimestamp(1));
        state.set_user_state(&peer("bob"), UserState::NotWatching, SharedTimestamp(1));
        let view = state.view();
        assert!(view.is_playing);
    }

    #[test]
    fn lww_newer_timestamp_wins() {
        let (state, _rx) = SharedState::new();
        state.add_peer(peer("alice"));
        state.set_user_state(&peer("alice"), UserState::Ready, SharedTimestamp(100));
        state.set_user_state(&peer("alice"), UserState::Paused, SharedTimestamp(200));
        let view = state.view();
        assert_eq!(view.user_states[&peer("alice")], UserState::Paused);
    }

    #[test]
    fn lww_older_timestamp_ignored() {
        let (state, _rx) = SharedState::new();
        state.add_peer(peer("alice"));
        state.set_user_state(&peer("alice"), UserState::Paused, SharedTimestamp(200));
        state.set_user_state(&peer("alice"), UserState::Ready, SharedTimestamp(100));
        let view = state.view();
        assert_eq!(view.user_states[&peer("alice")], UserState::Paused);
    }

    #[test]
    fn version_increments_on_mutation() {
        let (state, mut rx) = SharedState::new();
        assert_eq!(*rx.borrow(), 0);
        state.add_peer(peer("alice"));
        assert_eq!(*rx.borrow_and_update(), 1);
        state.set_user_state(&peer("alice"), UserState::Ready, SharedTimestamp(1));
        assert_eq!(*rx.borrow_and_update(), 2);
    }

    #[test]
    fn missing_file_blocks_playback() {
        let (state, _rx) = SharedState::new();
        state.add_peer(peer("alice"));
        state.set_user_state(&peer("alice"), UserState::Ready, SharedTimestamp(1));
        state.set_file_state(&peer("alice"), FileState::Missing, SharedTimestamp(1));
        let view = state.view();
        assert!(!view.is_playing);
    }
}
