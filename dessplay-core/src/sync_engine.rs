//! State sync engine: coordinates CRDT replication across peers.
//!
//! The `SyncEngine` wraps a `CrdtState` and produces `SyncAction`s that the
//! caller must dispatch to the network and storage layers. It performs no I/O
//! itself, keeping it pure and trivially testable.

use std::collections::HashSet;

use crate::crdt::CrdtState;
use crate::crdt::version::detect_gaps;
use crate::protocol::{
    CrdtOp, CrdtSnapshot, GapFillRequest, GapFillResponse, PeerControl, PeerDatagram,
    VersionVectors,
};
use crate::types::PeerId;

/// An action the caller must execute after calling a `SyncEngine` method.
#[derive(Clone, Debug)]
pub enum SyncAction {
    /// Send a reliable control message to a specific peer.
    SendControl { peer: PeerId, msg: PeerControl },
    /// Send an unreliable datagram to a specific peer.
    SendDatagram { peer: PeerId, msg: PeerDatagram },
    /// Broadcast a reliable control message to all connected peers.
    BroadcastControl { msg: PeerControl },
    /// Broadcast an unreliable datagram to all connected peers.
    BroadcastDatagram { msg: PeerDatagram },
    /// Open a gap fill stream to the peer and send this request.
    RequestGapFill {
        peer: PeerId,
        request: GapFillRequest,
    },
    /// Persist this operation to storage.
    PersistOp { op: CrdtOp },
    /// Persist a full snapshot (after epoch change).
    PersistSnapshot {
        epoch: u64,
        snapshot: CrdtSnapshot,
    },
}

/// The state sync engine.
///
/// Wraps `CrdtState` and coordinates replication. All methods are synchronous
/// and return `Vec<SyncAction>` describing what the caller should do.
pub struct SyncEngine {
    state: CrdtState,
    /// Peers for which we have an outstanding gap fill request.
    pending_gap_fills: HashSet<PeerId>,
}

impl SyncEngine {
    /// Create a new engine with empty state.
    pub fn new() -> Self {
        Self {
            state: CrdtState::new(),
            pending_gap_fills: HashSet::new(),
        }
    }

    /// Create from persisted state (loaded from SQLite at startup).
    pub fn from_persisted(epoch: u64, state: CrdtState, _stored_epoch: u64) -> Self {
        let _ = epoch; // epoch is embedded in CrdtState
        let _ = _stored_epoch;
        Self {
            state,
            pending_gap_fills: HashSet::new(),
        }
    }

    /// Apply a locally generated operation.
    ///
    /// Broadcasts to all peers (control + datagram) and requests persistence.
    pub fn apply_local_op(&mut self, op: CrdtOp) -> Vec<SyncAction> {
        if !self.state.apply_op(&op) {
            return vec![];
        }
        vec![
            SyncAction::BroadcastControl {
                msg: PeerControl::StateOp { op: op.clone() },
            },
            SyncAction::BroadcastDatagram {
                msg: PeerDatagram::StateOp { op: op.clone() },
            },
            SyncAction::PersistOp { op },
        ]
    }

    /// Handle a new peer connection.
    ///
    /// Sends our current state summary so the peer can detect gaps.
    pub fn on_peer_connected(&mut self, peer: PeerId) -> Vec<SyncAction> {
        vec![SyncAction::SendControl {
            peer,
            msg: PeerControl::StateSummary {
                epoch: self.state.epoch(),
                versions: self.state.version_vectors(),
            },
        }]
    }

    /// Handle a peer disconnection.
    pub fn on_peer_disconnected(&mut self, peer: PeerId) -> Vec<SyncAction> {
        self.pending_gap_fills.remove(&peer);
        vec![]
    }

    /// Handle a state operation received from a peer.
    ///
    /// Applies the op and persists it if it changed state. Deduplication is
    /// handled by `CrdtState::apply_op` returning false for duplicates.
    pub fn on_remote_op(&mut self, _from: PeerId, op: CrdtOp) -> Vec<SyncAction> {
        if !self.state.apply_op(&op) {
            return vec![];
        }
        vec![SyncAction::PersistOp { op }]
    }

    /// Handle a state summary received from a peer.
    ///
    /// Compares version vectors and either requests a gap fill, handles epoch
    /// mismatch, or does nothing if already up to date.
    pub fn on_state_summary(
        &mut self,
        from: PeerId,
        remote_epoch: u64,
        remote_versions: VersionVectors,
    ) -> Vec<SyncAction> {
        let local_epoch = self.state.epoch();

        if remote_epoch > local_epoch {
            // Remote has a newer epoch — we need a full snapshot from them.
            // Send our summary so they see our stale epoch and send a snapshot.
            return vec![SyncAction::SendControl {
                peer: from,
                msg: PeerControl::StateSummary {
                    epoch: local_epoch,
                    versions: self.state.version_vectors(),
                },
            }];
        }

        if remote_epoch < local_epoch {
            // Remote is behind — send them our snapshot so they can upgrade.
            return vec![SyncAction::SendControl {
                peer: from,
                msg: PeerControl::StateSnapshot {
                    epoch: local_epoch,
                    crdts: self.state.snapshot(),
                },
            }];
        }

        // Same epoch — check for gaps in both directions
        let local_vv = self.state.version_vectors();

        let mut actions = Vec::new();

        // Check if remote has data we don't → request gap fill
        if let Some(gap_request) = detect_gaps(&local_vv, &remote_versions)
            && !self.pending_gap_fills.contains(&from)
        {
            self.pending_gap_fills.insert(from);
            actions.push(SyncAction::RequestGapFill {
                peer: from,
                request: gap_request,
            });
        }

        // Check if we have data the remote doesn't → proactively send missing ops.
        // This is essential for the server case where gap fill via streams isn't
        // supported, but also benefits P2P by reducing round trips.
        if let Some(their_gaps) = detect_gaps(&remote_versions, &local_vv) {
            let ops = self.on_gap_fill_request(&their_gaps).ops;
            for op in ops {
                actions.push(SyncAction::SendControl {
                    peer: from,
                    msg: PeerControl::StateOp { op },
                });
            }
        }

        actions
    }

    /// Handle a state snapshot received from a peer (epoch upgrade).
    ///
    /// Replaces local state if the snapshot's epoch is newer.
    pub fn on_state_snapshot(&mut self, epoch: u64, snapshot: CrdtSnapshot) -> Vec<SyncAction> {
        if epoch <= self.state.epoch() {
            return vec![];
        }

        self.state.load_snapshot(epoch, snapshot.clone());
        self.pending_gap_fills.clear();

        vec![SyncAction::PersistSnapshot { epoch, snapshot }]
    }

    /// Handle an incoming gap fill request from a peer.
    ///
    /// Returns the ops the requester is missing.
    pub fn on_gap_fill_request(&self, request: &GapFillRequest) -> GapFillResponse {
        // Build a synthetic VersionVectors from the request's known state,
        // so we can use ops_since() to find what they're missing.
        let mut remote_vv = VersionVectors::new(self.state.epoch());

        for (reg, ts) in &request.lww_needed {
            remote_vv.lww_versions.insert(reg.clone(), *ts);
        }
        for (uid, hash) in &request.chat_needed {
            remote_vv.chat_versions.insert(uid.clone(), *hash);
        }
        if let Some(pv) = request.playlist_after {
            remote_vv.playlist_version = pv;
        }

        let ops = self.state.ops_since(&remote_vv);
        GapFillResponse { ops }
    }

    /// Handle a gap fill response (ops from a peer we requested).
    pub fn on_gap_fill_response(
        &mut self,
        from: PeerId,
        response: GapFillResponse,
    ) -> Vec<SyncAction> {
        self.pending_gap_fills.remove(&from);

        let mut actions = Vec::new();
        for op in response.ops {
            if self.state.apply_op(&op) {
                actions.push(SyncAction::PersistOp { op });
            }
        }
        actions
    }

    /// Called every 1 second by the event loop timer.
    ///
    /// Broadcasts our current state summary to all peers.
    pub fn on_periodic_tick(&self) -> Vec<SyncAction> {
        vec![SyncAction::BroadcastControl {
            msg: PeerControl::StateSummary {
                epoch: self.state.epoch(),
                versions: self.state.version_vectors(),
            },
        }]
    }

    /// Read-only access to the current CRDT state.
    pub fn state(&self) -> &CrdtState {
        &self.state
    }

    /// Current epoch.
    pub fn epoch(&self) -> u64 {
        self.state.epoch()
    }

    /// Current version vectors.
    pub fn version_vectors(&self) -> VersionVectors {
        self.state.version_vectors()
    }

    /// Compact in-memory state into a snapshot and increment the epoch.
    ///
    /// Returns the new epoch and the snapshot. The caller is responsible for
    /// persisting the snapshot and cleaning up old data in storage.
    pub fn compact(&mut self) -> (u64, CrdtSnapshot) {
        let new_epoch = self.state.epoch() + 1;
        self.state.set_epoch(new_epoch);
        let snapshot = self.state.snapshot();
        self.pending_gap_fills.clear();
        (new_epoch, snapshot)
    }
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::protocol::{LwwValue, PlaylistAction, RegisterId};
    use crate::types::{FileId, FileState, UserId, UserState};

    fn uid(name: &str) -> UserId {
        UserId(name.to_string())
    }

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    fn peer(n: u64) -> PeerId {
        PeerId(n)
    }

    fn chat_op(user: &str, seq: u64, ts: u64, text: &str) -> CrdtOp {
        CrdtOp::ChatAppend {
            user_id: uid(user),
            seq,
            timestamp: ts,
            text: text.to_string(),
        }
    }

    fn user_state_op(user: &str, state: UserState, ts: u64) -> CrdtOp {
        CrdtOp::LwwWrite {
            timestamp: ts,
            value: LwwValue::UserState(uid(user), state),
        }
    }

    fn playlist_add_op(file: u8, ts: u64) -> CrdtOp {
        CrdtOp::PlaylistOp {
            timestamp: ts,
            action: PlaylistAction::Add {
                file_id: fid(file),
                after: None,
            },
        }
    }

    // --- Helper to count specific action types ---

    fn count_broadcast_control(actions: &[SyncAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, SyncAction::BroadcastControl { .. }))
            .count()
    }

    fn count_broadcast_datagram(actions: &[SyncAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, SyncAction::BroadcastDatagram { .. }))
            .count()
    }

    fn count_persist_op(actions: &[SyncAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, SyncAction::PersistOp { .. }))
            .count()
    }

    fn count_persist_snapshot(actions: &[SyncAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, SyncAction::PersistSnapshot { .. }))
            .count()
    }

    fn count_send_control(actions: &[SyncAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, SyncAction::SendControl { .. }))
            .count()
    }

    fn count_gap_fill_request(actions: &[SyncAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, SyncAction::RequestGapFill { .. }))
            .count()
    }

    // -----------------------------------------------------------------------
    // apply_local_op
    // -----------------------------------------------------------------------

    #[test]
    fn apply_local_op_broadcasts_and_persists() {
        let mut engine = SyncEngine::new();
        let op = user_state_op("alice", UserState::Ready, 100);
        let actions = engine.apply_local_op(op);

        assert_eq!(count_broadcast_control(&actions), 1);
        assert_eq!(count_broadcast_datagram(&actions), 1);
        assert_eq!(count_persist_op(&actions), 1);
        assert_eq!(actions.len(), 3);
    }

    #[test]
    fn apply_local_op_duplicate_returns_empty() {
        let mut engine = SyncEngine::new();
        let op = user_state_op("alice", UserState::Ready, 100);
        engine.apply_local_op(op.clone());

        // Same op again — no state change
        let actions = engine.apply_local_op(op);
        assert!(actions.is_empty());
    }

    #[test]
    fn apply_local_op_invalid_returns_empty() {
        let mut engine = SyncEngine::new();
        // Timestamp 0 is rejected by validation
        let op = user_state_op("alice", UserState::Ready, 0);
        let actions = engine.apply_local_op(op);
        assert!(actions.is_empty());
    }

    // -----------------------------------------------------------------------
    // on_peer_connected
    // -----------------------------------------------------------------------

    #[test]
    fn on_peer_connected_sends_summary() {
        let mut engine = SyncEngine::new();
        let actions = engine.on_peer_connected(peer(1));

        assert_eq!(actions.len(), 1);
        assert_eq!(count_send_control(&actions), 1);

        if let SyncAction::SendControl { peer: p, msg } = &actions[0] {
            assert_eq!(*p, peer(1));
            assert!(matches!(msg, PeerControl::StateSummary { .. }));
        } else {
            panic!("expected SendControl");
        }
    }

    // -----------------------------------------------------------------------
    // on_peer_disconnected
    // -----------------------------------------------------------------------

    #[test]
    fn on_peer_disconnected_clears_pending_gap_fill() {
        let mut engine = SyncEngine::new();
        // Simulate having a pending gap fill
        engine.pending_gap_fills.insert(peer(1));

        let actions = engine.on_peer_disconnected(peer(1));
        assert!(actions.is_empty());
        assert!(!engine.pending_gap_fills.contains(&peer(1)));
    }

    // -----------------------------------------------------------------------
    // on_remote_op
    // -----------------------------------------------------------------------

    #[test]
    fn on_remote_op_applies_and_persists() {
        let mut engine = SyncEngine::new();
        let op = user_state_op("alice", UserState::Paused, 100);
        let actions = engine.on_remote_op(peer(1), op);

        assert_eq!(count_persist_op(&actions), 1);
        assert_eq!(actions.len(), 1);

        // Verify state changed
        assert_eq!(
            engine.state().user_states.read(&uid("alice")),
            Some(&UserState::Paused)
        );
    }

    #[test]
    fn on_remote_op_duplicate_returns_empty() {
        let mut engine = SyncEngine::new();
        let op = user_state_op("alice", UserState::Ready, 100);
        engine.on_remote_op(peer(1), op.clone());

        let actions = engine.on_remote_op(peer(2), op);
        assert!(actions.is_empty());
    }

    #[test]
    fn on_remote_op_does_not_rebroadcast() {
        let mut engine = SyncEngine::new();
        let op = user_state_op("alice", UserState::Ready, 100);
        let actions = engine.on_remote_op(peer(1), op);

        // Should only persist, never broadcast
        assert_eq!(count_broadcast_control(&actions), 0);
        assert_eq!(count_broadcast_datagram(&actions), 0);
    }

    // -----------------------------------------------------------------------
    // on_state_summary — same epoch
    // -----------------------------------------------------------------------

    #[test]
    fn on_state_summary_no_gaps_returns_empty() {
        let mut engine = SyncEngine::new();
        let vv = engine.version_vectors();
        let actions = engine.on_state_summary(peer(1), 0, vv);
        assert!(actions.is_empty());
    }

    #[test]
    fn on_state_summary_with_gaps_requests_gap_fill() {
        let mut engine = SyncEngine::new();

        // Remote has state we don't
        let mut remote_vv = VersionVectors::new(0);
        remote_vv
            .lww_versions
            .insert(RegisterId::UserState(uid("alice")), 100);

        let actions = engine.on_state_summary(peer(1), 0, remote_vv);

        assert_eq!(count_gap_fill_request(&actions), 1);
        if let SyncAction::RequestGapFill { peer: p, .. } = &actions[0] {
            assert_eq!(*p, peer(1));
        }
    }

    #[test]
    fn on_state_summary_deduplicates_gap_fill_requests() {
        let mut engine = SyncEngine::new();

        let mut remote_vv = VersionVectors::new(0);
        remote_vv
            .lww_versions
            .insert(RegisterId::UserState(uid("alice")), 100);

        // First request goes through
        let actions1 = engine.on_state_summary(peer(1), 0, remote_vv.clone());
        assert_eq!(count_gap_fill_request(&actions1), 1);

        // Second request to same peer is deduplicated
        let actions2 = engine.on_state_summary(peer(1), 0, remote_vv);
        assert!(actions2.is_empty());
    }

    // -----------------------------------------------------------------------
    // on_state_summary — epoch mismatch
    // -----------------------------------------------------------------------

    #[test]
    fn on_state_summary_higher_remote_epoch_sends_our_summary() {
        let mut engine = SyncEngine::new();
        let remote_vv = VersionVectors::new(5);

        let actions = engine.on_state_summary(peer(1), 5, remote_vv);

        // We should send our summary so the remote sees our stale epoch
        assert_eq!(count_send_control(&actions), 1);
        if let SyncAction::SendControl { msg: PeerControl::StateSummary { epoch, .. }, .. } =
            &actions[0]
        {
            assert_eq!(*epoch, 0); // our epoch
        } else {
            panic!("expected StateSummary");
        }
    }

    #[test]
    fn on_state_summary_lower_remote_epoch_sends_snapshot() {
        let mut engine = SyncEngine::new();
        // Give our engine some state at epoch 3
        let snap = CrdtSnapshot {
            user_states: Default::default(),
            file_states: Default::default(),
            anidb: Default::default(),
            playlist_ops: vec![],
            chat: Default::default(),
        };
        engine.state.load_snapshot(3, snap);

        let remote_vv = VersionVectors::new(1);
        let actions = engine.on_state_summary(peer(1), 1, remote_vv);

        assert_eq!(count_send_control(&actions), 1);
        if let SyncAction::SendControl {
            msg: PeerControl::StateSnapshot { epoch, .. },
            ..
        } = &actions[0]
        {
            assert_eq!(*epoch, 3);
        } else {
            panic!("expected StateSnapshot");
        }
    }

    // -----------------------------------------------------------------------
    // on_state_snapshot
    // -----------------------------------------------------------------------

    #[test]
    fn on_state_snapshot_upgrades_epoch() {
        let mut engine = SyncEngine::new();
        let snap = CrdtSnapshot {
            user_states: Default::default(),
            file_states: Default::default(),
            anidb: Default::default(),
            playlist_ops: vec![],
            chat: Default::default(),
        };

        let actions = engine.on_state_snapshot(5, snap);

        assert_eq!(engine.epoch(), 5);
        assert_eq!(count_persist_snapshot(&actions), 1);
    }

    #[test]
    fn on_state_snapshot_ignores_stale() {
        let mut engine = SyncEngine::new();
        // Set epoch to 5
        let snap = CrdtSnapshot {
            user_states: Default::default(),
            file_states: Default::default(),
            anidb: Default::default(),
            playlist_ops: vec![],
            chat: Default::default(),
        };
        engine.state.load_snapshot(5, snap);

        // Try to load epoch 3 — should be ignored
        let old_snap = CrdtSnapshot {
            user_states: Default::default(),
            file_states: Default::default(),
            anidb: Default::default(),
            playlist_ops: vec![],
            chat: Default::default(),
        };
        let actions = engine.on_state_snapshot(3, old_snap);

        assert!(actions.is_empty());
        assert_eq!(engine.epoch(), 5);
    }

    #[test]
    fn on_state_snapshot_same_epoch_ignored() {
        let mut engine = SyncEngine::new();
        let snap = CrdtSnapshot {
            user_states: Default::default(),
            file_states: Default::default(),
            anidb: Default::default(),
            playlist_ops: vec![],
            chat: Default::default(),
        };

        // epoch 0 snapshot on epoch 0 engine — not newer
        let actions = engine.on_state_snapshot(0, snap);
        assert!(actions.is_empty());
    }

    #[test]
    fn on_state_snapshot_clears_pending_gap_fills() {
        let mut engine = SyncEngine::new();
        engine.pending_gap_fills.insert(peer(1));
        engine.pending_gap_fills.insert(peer(2));

        let snap = CrdtSnapshot {
            user_states: Default::default(),
            file_states: Default::default(),
            anidb: Default::default(),
            playlist_ops: vec![],
            chat: Default::default(),
        };
        engine.on_state_snapshot(1, snap);

        assert!(engine.pending_gap_fills.is_empty());
    }

    // -----------------------------------------------------------------------
    // on_gap_fill_request / on_gap_fill_response
    // -----------------------------------------------------------------------

    #[test]
    fn gap_fill_round_trip() {
        // Provider has state
        let mut provider = SyncEngine::new();
        provider.apply_local_op(user_state_op("alice", UserState::Ready, 100));
        provider.apply_local_op(chat_op("bob", 0, 200, "hello"));
        provider.apply_local_op(playlist_add_op(1, 300));

        // Requester has nothing
        let requester = SyncEngine::new();
        let provider_vv = provider.version_vectors();
        let requester_vv = requester.version_vectors();

        // Detect gaps
        let gap = detect_gaps(&requester_vv, &provider_vv).unwrap();

        // Provider responds
        let response = provider.on_gap_fill_request(&gap);
        assert!(!response.ops.is_empty());

        // Requester applies response
        let mut requester = requester;
        let actions = requester.on_gap_fill_response(peer(1), response);

        // All ops should be persisted
        assert_eq!(count_persist_op(&actions), 3);

        // State should match
        assert_eq!(provider.state().snapshot(), requester.state().snapshot());
    }

    #[test]
    fn gap_fill_response_clears_pending() {
        let mut engine = SyncEngine::new();
        engine.pending_gap_fills.insert(peer(1));

        let response = GapFillResponse { ops: vec![] };
        engine.on_gap_fill_response(peer(1), response);

        assert!(!engine.pending_gap_fills.contains(&peer(1)));
    }

    #[test]
    fn gap_fill_response_deduplicates_ops() {
        let mut engine = SyncEngine::new();
        let op = user_state_op("alice", UserState::Ready, 100);
        engine.apply_local_op(op.clone());

        // Response contains an op we already have
        let response = GapFillResponse {
            ops: vec![op],
        };
        let actions = engine.on_gap_fill_response(peer(1), response);

        // No persist action for the duplicate
        assert!(actions.is_empty());
    }

    // -----------------------------------------------------------------------
    // on_periodic_tick
    // -----------------------------------------------------------------------

    #[test]
    fn on_periodic_tick_broadcasts_summary() {
        let engine = SyncEngine::new();
        let actions = engine.on_periodic_tick();

        assert_eq!(actions.len(), 1);
        assert_eq!(count_broadcast_control(&actions), 1);

        if let SyncAction::BroadcastControl { msg } = &actions[0] {
            assert!(matches!(msg, PeerControl::StateSummary { .. }));
        } else {
            panic!("expected BroadcastControl with StateSummary");
        }
    }

    #[test]
    fn on_periodic_tick_reflects_current_state() {
        let mut engine = SyncEngine::new();
        engine.apply_local_op(user_state_op("alice", UserState::Paused, 42));

        let actions = engine.on_periodic_tick();
        if let SyncAction::BroadcastControl {
            msg: PeerControl::StateSummary { versions, .. },
        } = &actions[0]
        {
            assert!(versions
                .lww_versions
                .contains_key(&RegisterId::UserState(uid("alice"))));
        } else {
            panic!("expected StateSummary");
        }
    }

    // -----------------------------------------------------------------------
    // End-to-end scenarios
    // -----------------------------------------------------------------------

    #[test]
    fn two_engines_converge_via_ops() {
        let mut engine_a = SyncEngine::new();
        let mut engine_b = SyncEngine::new();

        // A generates an op
        let op = chat_op("alice", 0, 100, "hello from A");
        let actions = engine_a.apply_local_op(op.clone());

        // Simulate B receiving the op via control stream
        for action in &actions {
            if let SyncAction::BroadcastControl {
                msg: PeerControl::StateOp { op },
            } = action
            {
                engine_b.on_remote_op(peer(1), op.clone());
            }
        }

        assert_eq!(engine_a.state().snapshot(), engine_b.state().snapshot());
    }

    #[test]
    fn two_engines_converge_via_gap_fill() {
        let mut engine_a = SyncEngine::new();
        let mut engine_b = SyncEngine::new();

        // A has state that B doesn't
        engine_a.apply_local_op(user_state_op("alice", UserState::Ready, 100));
        engine_a.apply_local_op(chat_op("alice", 0, 200, "msg"));

        // B receives A's state summary and detects gaps
        let a_vv = engine_a.version_vectors();
        let b_actions = engine_b.on_state_summary(peer(1), 0, a_vv);
        assert_eq!(count_gap_fill_request(&b_actions), 1);

        // Extract the gap fill request
        let request = match &b_actions[0] {
            SyncAction::RequestGapFill { request, .. } => request.clone(),
            _ => panic!("expected gap fill request"),
        };

        // A responds
        let response = engine_a.on_gap_fill_request(&request);

        // B applies the response
        engine_b.on_gap_fill_response(peer(1), response);

        assert_eq!(engine_a.state().snapshot(), engine_b.state().snapshot());
    }

    #[test]
    fn epoch_upgrade_replaces_state() {
        let mut engine = SyncEngine::new();
        engine.apply_local_op(user_state_op("old_user", UserState::Ready, 1));

        // Receive a snapshot with new epoch and different state
        let mut new_state = CrdtState::new();
        new_state.apply_op(&user_state_op("new_user", UserState::Paused, 50));
        let snap = new_state.snapshot();

        engine.on_state_snapshot(2, snap);

        assert_eq!(engine.epoch(), 2);
        // Old state should be gone
        assert_eq!(engine.state().user_states.read(&uid("old_user")), None);
        // New state should be present
        assert_eq!(
            engine.state().user_states.read(&uid("new_user")),
            Some(&UserState::Paused)
        );
    }

    #[test]
    fn multiple_op_types_converge() {
        let mut engine_a = SyncEngine::new();
        let mut engine_b = SyncEngine::new();

        let ops = vec![
            user_state_op("alice", UserState::Ready, 10),
            CrdtOp::LwwWrite {
                timestamp: 20,
                value: LwwValue::FileState(uid("alice"), fid(1), FileState::Missing),
            },
            playlist_add_op(1, 30),
            playlist_add_op(2, 40),
            chat_op("alice", 0, 50, "hi"),
            chat_op("bob", 0, 60, "hey"),
        ];

        // A generates all ops
        for op in &ops {
            engine_a.apply_local_op(op.clone());
        }

        // B receives them as remote ops
        for op in &ops {
            engine_b.on_remote_op(peer(1), op.clone());
        }

        assert_eq!(engine_a.state().snapshot(), engine_b.state().snapshot());
    }

    // -----------------------------------------------------------------------
    // compact
    // -----------------------------------------------------------------------

    #[test]
    fn compact_increments_epoch() {
        let mut engine = SyncEngine::new();
        engine.apply_local_op(user_state_op("alice", UserState::Ready, 100));
        assert_eq!(engine.epoch(), 0);

        let (new_epoch, _) = engine.compact();
        assert_eq!(new_epoch, 1);
        assert_eq!(engine.epoch(), 1);
    }

    #[test]
    fn compact_preserves_state() {
        let mut engine = SyncEngine::new();
        engine.apply_local_op(user_state_op("alice", UserState::Paused, 100));
        engine.apply_local_op(playlist_add_op(1, 200));
        engine.apply_local_op(chat_op("alice", 0, 300, "hi"));

        let snapshot_before = engine.state().snapshot();
        let (_, snapshot_after) = engine.compact();

        assert_eq!(snapshot_before, snapshot_after);
    }

    #[test]
    fn compact_clears_pending_gap_fills() {
        let mut engine = SyncEngine::new();
        engine.pending_gap_fills.insert(peer(1));
        engine.pending_gap_fills.insert(peer(2));
        engine.compact();
        assert!(engine.pending_gap_fills.is_empty());
    }

    #[test]
    fn compact_round_trip() {
        // Server side: build up some state and compact
        let mut server = SyncEngine::new();
        server.apply_local_op(user_state_op("alice", UserState::Paused, 100));
        server.apply_local_op(playlist_add_op(1, 200));
        server.apply_local_op(chat_op("alice", 0, 300, "hi"));

        let (new_epoch, _snapshot) = server.compact();
        assert_eq!(new_epoch, 1);

        // Client with stale epoch 0
        let mut client = SyncEngine::new();
        assert_eq!(client.epoch(), 0);

        // Client sends its summary; server detects stale epoch and responds
        let server_actions =
            server.on_state_summary(peer(1), 0, client.version_vectors());

        // Server must send a StateSnapshot to the stale client
        let snapshot_action = server_actions.iter().find(|a| {
            matches!(
                a,
                SyncAction::SendControl {
                    msg: PeerControl::StateSnapshot { .. },
                    ..
                }
            )
        });
        assert!(
            snapshot_action.is_some(),
            "server must send snapshot to stale client"
        );

        // Client applies the snapshot
        if let Some(SyncAction::SendControl {
            msg: PeerControl::StateSnapshot { epoch, crdts },
            ..
        }) = snapshot_action
        {
            let actions = client.on_state_snapshot(*epoch, crdts.clone());
            assert!(!actions.is_empty(), "must produce PersistSnapshot");
            assert_eq!(client.epoch(), 1);
            assert_eq!(client.state().snapshot(), server.state().snapshot());
        }
    }
}
