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
    /// Currently loaded file (LWW with origin tiebreaker).
    file_register: FileRegister,
    /// Playback position (LWW, conditional on file_id match).
    position_register: PositionRegister,
    /// Per-peer user states with timestamps (LWW).
    user_states: HashMap<PeerId, (UserState, SharedTimestamp)>,
    /// Per-peer file states with timestamps (LWW).
    file_states: HashMap<PeerId, (FileState, SharedTimestamp)>,
    /// Per-peer join generation timestamps. NOT removed on disconnect —
    /// used to reject stale state from old sessions after reconnect.
    peer_generations: HashMap<PeerId, SharedTimestamp>,
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
        let sentinel = PeerId(String::new());
        let state = Self {
            inner: RwLock::new(SharedStateInner {
                file_register: FileRegister {
                    file_id: None,
                    timestamp: SharedTimestamp(0),
                    origin: sentinel.clone(),
                },
                position_register: PositionRegister {
                    position: 0.0,
                    for_file: None,
                    timestamp: SharedTimestamp(0),
                    origin: sentinel,
                },
                user_states: HashMap::new(),
                file_states: HashMap::new(),
                peer_generations: HashMap::new(),
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
            current_file: inner.file_register.file_id.clone(),
            position: inner.position_register.position,
            position_timestamp: inner.position_register.timestamp,
            user_states,
            file_states,
            ready_states,
            is_playing,
            chat_messages,
            peers: inner.peers.clone(),
        }
    }

    // ── Global registers (deterministic tiebreaker) ─────────────────────

    /// Update the file register (LWW with origin tiebreaker).
    /// When accepted, also resets the position register to 0.0 for the new file.
    /// Returns true if the state was updated.
    pub fn update_file_register(&self, reg: FileRegister) -> bool {
        let mut inner = self.inner.write().unwrap();
        if (reg.timestamp, &reg.origin)
            > (inner.file_register.timestamp, &inner.file_register.origin)
        {
            // Reset position when file changes
            inner.position_register = PositionRegister {
                position: 0.0,
                for_file: reg.file_id.clone(),
                timestamp: reg.timestamp,
                origin: reg.origin.clone(),
            };
            inner.file_register = reg;
            self.bump_version(&mut inner);
            true
        } else {
            false
        }
    }

    /// Update the position register (LWW with origin tiebreaker).
    /// Rejects if `for_file` doesn't match the current file register.
    /// Returns true if the state was updated.
    pub fn update_position(&self, reg: PositionRegister) -> bool {
        let mut inner = self.inner.write().unwrap();
        // Reject position for a different file
        if reg.for_file != inner.file_register.file_id {
            return false;
        }
        if (reg.timestamp, &reg.origin)
            > (inner.position_register.timestamp, &inner.position_register.origin)
        {
            inner.position_register = reg;
            self.bump_version(&mut inner);
            true
        } else {
            false
        }
    }

    /// Get the current file register (for sync engine snapshot building).
    pub fn file_register(&self) -> FileRegister {
        self.inner.read().unwrap().file_register.clone()
    }

    /// Get the current position register (for sync engine snapshot building).
    pub fn position_register(&self) -> PositionRegister {
        self.inner.read().unwrap().position_register.clone()
    }

    // ── Per-peer registers (>= comparison, single writer) ───────────────

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

    // ── Generation counter ──────────────────────────────────────────────

    /// Update a peer's generation (join timestamp).
    /// Returns true if the generation was accepted (>= known).
    /// If strictly newer, clears old user/file state for that peer.
    pub fn update_peer_generation(&self, peer: &PeerId, generation: SharedTimestamp) -> bool {
        let mut inner = self.inner.write().unwrap();
        let current = inner.peer_generations.get(peer).copied().unwrap_or(SharedTimestamp(0));
        if generation >= current {
            if generation > current {
                // New session — clear old state
                inner.user_states.remove(peer);
                inner.file_states.remove(peer);
            }
            inner.peer_generations.insert(peer.clone(), generation);
            self.bump_version(&mut inner);
            true
        } else {
            false
        }
    }

    /// Get the known generation for a peer, if any.
    pub fn peer_generation(&self, peer: &PeerId) -> Option<SharedTimestamp> {
        self.inner.read().unwrap().peer_generations.get(peer).copied()
    }

    /// Get all peer generations (for snapshot building).
    pub fn peer_generations(&self) -> HashMap<PeerId, SharedTimestamp> {
        self.inner.read().unwrap().peer_generations.clone()
    }

    // ── Append log state ────────────────────────────────────────────────

    /// Add a chat message (idempotent — duplicates by sender+seq are ignored).
    pub fn add_chat_message(&self, msg: ChatMessage) {
        let mut inner = self.inner.write().unwrap();
        // Deduplicate by sender + seq
        let exists = inner.chat_messages.iter().any(|m| {
            m.sender == msg.sender && m.seq == msg.seq
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

    // ── Peer management ─────────────────────────────────────────────────

    /// Register a new peer.
    pub fn add_peer(&self, peer: PeerId) {
        let mut inner = self.inner.write().unwrap();
        if !inner.peers.contains(&peer) {
            inner.peers.push(peer);
            self.bump_version(&mut inner);
        }
    }

    /// Remove a peer and clean up their state.
    /// Note: peer_generations is NOT removed — needed to reject stale state on reconnect.
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

    fn item_id(user: &str, seq: u64) -> ItemId {
        ItemId {
            user: peer(user),
            seq,
        }
    }

    #[test]
    fn file_register_lww_tiebreaker() {
        // At equal timestamps, the higher PeerId wins deterministically.
        let (state, _rx) = SharedState::new();

        state.update_file_register(FileRegister {
            file_id: Some(item_id("alice", 1)),
            timestamp: SharedTimestamp(100),
            origin: peer("alice"),
        });
        // "bob" > "alice" lexicographically, so bob wins at the same timestamp.
        state.update_file_register(FileRegister {
            file_id: Some(item_id("bob", 1)),
            timestamp: SharedTimestamp(100),
            origin: peer("bob"),
        });

        let view = state.view();
        assert_eq!(view.current_file.unwrap().user, peer("bob"));

        // Verify the reverse order also gives the same result.
        let (state2, _rx2) = SharedState::new();
        state2.update_file_register(FileRegister {
            file_id: Some(item_id("bob", 1)),
            timestamp: SharedTimestamp(100),
            origin: peer("bob"),
        });
        state2.update_file_register(FileRegister {
            file_id: Some(item_id("alice", 1)),
            timestamp: SharedTimestamp(100),
            origin: peer("alice"),
        });
        let view2 = state2.view();
        assert_eq!(view2.current_file.unwrap().user, peer("bob"));
    }

    #[test]
    fn position_rejected_for_wrong_file() {
        let (state, _rx) = SharedState::new();

        state.update_file_register(FileRegister {
            file_id: Some(item_id("alice", 1)),
            timestamp: SharedTimestamp(100),
            origin: peer("alice"),
        });

        // Position for a different file should be rejected.
        let accepted = state.update_position(PositionRegister {
            position: 50.0,
            for_file: Some(item_id("alice", 2)), // wrong file
            timestamp: SharedTimestamp(200),
            origin: peer("alice"),
        });
        assert!(!accepted);
        assert_eq!(state.view().position, 0.0);
    }

    #[test]
    fn position_accepted_for_correct_file() {
        let (state, _rx) = SharedState::new();

        let file = Some(item_id("alice", 1));
        state.update_file_register(FileRegister {
            file_id: file.clone(),
            timestamp: SharedTimestamp(100),
            origin: peer("alice"),
        });

        let accepted = state.update_position(PositionRegister {
            position: 42.5,
            for_file: file,
            timestamp: SharedTimestamp(200),
            origin: peer("alice"),
        });
        assert!(accepted);
        assert_eq!(state.view().position, 42.5);
    }

    #[test]
    fn file_register_change_resets_position() {
        let (state, _rx) = SharedState::new();

        let file1 = Some(item_id("alice", 1));
        state.update_file_register(FileRegister {
            file_id: file1.clone(),
            timestamp: SharedTimestamp(100),
            origin: peer("alice"),
        });

        // Seek to 300s in file 1.
        state.update_position(PositionRegister {
            position: 300.0,
            for_file: file1,
            timestamp: SharedTimestamp(200),
            origin: peer("alice"),
        });
        assert_eq!(state.view().position, 300.0);

        // Switch to file 2 — position should reset to 0.
        state.update_file_register(FileRegister {
            file_id: Some(item_id("alice", 2)),
            timestamp: SharedTimestamp(300),
            origin: peer("alice"),
        });
        assert_eq!(state.view().position, 0.0);
    }

    #[test]
    fn generation_counter_rejects_stale_state() {
        let (state, _rx) = SharedState::new();
        let alice = peer("alice");
        state.add_peer(alice.clone());

        // First generation — alice joins and sets state.
        state.update_peer_generation(&alice, SharedTimestamp(100));
        state.set_user_state(&alice, UserState::Paused, SharedTimestamp(150));
        assert_eq!(state.view().user_states[&alice], UserState::Paused);

        // New generation (alice reconnects) — old state should be cleared.
        state.update_peer_generation(&alice, SharedTimestamp(200));
        // User state should be gone (alice hasn't set it in the new session yet).
        assert!(!state.view().user_states.contains_key(&alice));
    }

    #[test]
    fn generation_not_removed_on_disconnect() {
        let (state, _rx) = SharedState::new();
        let alice = peer("alice");
        state.add_peer(alice.clone());
        state.update_peer_generation(&alice, SharedTimestamp(100));

        // Disconnect alice.
        state.remove_peer(&alice);

        // Generation should still be there to reject stale state.
        assert_eq!(state.peer_generation(&alice), Some(SharedTimestamp(100)));
    }

    #[test]
    fn stale_generation_rejected() {
        let (state, _rx) = SharedState::new();
        let alice = peer("alice");
        state.add_peer(alice.clone());

        // Set generation to 200 (current session).
        state.update_peer_generation(&alice, SharedTimestamp(200));

        // Try to set an older generation — should be rejected.
        let accepted = state.update_peer_generation(&alice, SharedTimestamp(100));
        assert!(!accepted);
        assert_eq!(state.peer_generation(&alice), Some(SharedTimestamp(200)));
    }

    #[test]
    fn chat_dedup_by_sender_seq() {
        let (state, _rx) = SharedState::new();

        let msg1 = ChatMessage {
            sender: peer("alice"),
            text: "hello".into(),
            timestamp: SharedTimestamp(100),
            seq: 1,
        };
        // Same sender+seq but different text and timestamp — should be deduplicated.
        let msg2 = ChatMessage {
            sender: peer("alice"),
            text: "different text".into(),
            timestamp: SharedTimestamp(200),
            seq: 1,
        };
        state.add_chat_message(msg1);
        state.add_chat_message(msg2);

        let view = state.view();
        assert_eq!(view.chat_messages.len(), 1);
        assert_eq!(view.chat_messages[0].text, "hello");
    }

    #[test]
    fn chat_different_seq_accepted() {
        let (state, _rx) = SharedState::new();

        state.add_chat_message(ChatMessage {
            sender: peer("alice"),
            text: "first".into(),
            timestamp: SharedTimestamp(100),
            seq: 1,
        });
        state.add_chat_message(ChatMessage {
            sender: peer("alice"),
            text: "second".into(),
            timestamp: SharedTimestamp(100), // same timestamp
            seq: 2, // different seq
        });

        let view = state.view();
        assert_eq!(view.chat_messages.len(), 2);
    }
}
