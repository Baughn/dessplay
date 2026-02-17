use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;
use tokio::time::Duration;

use super::clock::{ClockSyncService, SharedClock, SharedTimestamp};
use super::{ConnectionEvent, PeerId};

use crate::state::types::*;
use crate::state::SharedState;

// ── Sync Primitives ─────────────────────────────────────────────────────

/// Last-Writer-Wins register. Highest timestamp wins.
#[derive(Debug, Clone)]
pub struct LwwRegister<T: Clone> {
    pub value: T,
    pub timestamp: SharedTimestamp,
}

impl<T: Clone> LwwRegister<T> {
    pub fn new(value: T, timestamp: SharedTimestamp) -> Self {
        Self { value, timestamp }
    }

    /// Merge a remote value. Returns true if the remote value was adopted.
    pub fn merge(&mut self, value: T, timestamp: SharedTimestamp) -> bool {
        if timestamp > self.timestamp {
            self.value = value;
            self.timestamp = timestamp;
            true
        } else {
            false
        }
    }
}

/// A single entry in an append log.
#[derive(Debug, Clone)]
pub struct AppendLogEntry {
    pub seq: SequenceNumber,
    pub data: Vec<u8>,
    pub timestamp: SharedTimestamp,
}

/// Per-user append log with state vector tracking.
///
/// The state vector only advances through contiguous sequences —
/// out-of-order entries are stored but don't advance the watermark.
#[derive(Debug, Clone, Default)]
pub struct AppendLog {
    /// Entries per user, sorted by sequence number.
    entries: HashMap<PeerId, Vec<AppendLogEntry>>,
    /// Highest contiguous sequence number per user (watermark).
    state_vector: HashMap<PeerId, SequenceNumber>,
}

impl AppendLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a new entry from the local user. Returns the assigned sequence number.
    pub fn append_local(
        &mut self,
        user: &PeerId,
        data: Vec<u8>,
        timestamp: SharedTimestamp,
    ) -> SequenceNumber {
        let next_seq = self.state_vector.get(user).copied().unwrap_or(0) + 1;
        let entry = AppendLogEntry {
            seq: next_seq,
            data,
            timestamp,
        };
        self.entries.entry(user.clone()).or_default().push(entry);
        self.state_vector.insert(user.clone(), next_seq);
        next_seq
    }

    /// Insert a remote entry. Returns true if it was new (not a duplicate).
    pub fn insert(
        &mut self,
        user: &PeerId,
        seq: SequenceNumber,
        data: Vec<u8>,
        timestamp: SharedTimestamp,
    ) -> bool {
        let entries = self.entries.entry(user.clone()).or_default();

        // Check for duplicate
        if entries.iter().any(|e| e.seq == seq) {
            return false;
        }

        // Insert in sorted order
        let pos = entries.partition_point(|e| e.seq < seq);
        entries.insert(
            pos,
            AppendLogEntry {
                seq,
                data,
                timestamp,
            },
        );

        // Advance watermark through contiguous entries
        self.advance_watermark(user);
        true
    }

    /// Find gaps between our state vector and a remote state vector.
    /// Returns (user, from_seq, to_seq) tuples for missing ranges.
    pub fn find_gaps(
        &self,
        remote_vectors: &HashMap<PeerId, SequenceNumber>,
    ) -> Vec<(PeerId, SequenceNumber, SequenceNumber)> {
        let mut gaps = Vec::new();
        for (user, &remote_seq) in remote_vectors {
            let local_seq = self.state_vector.get(user).copied().unwrap_or(0);
            if remote_seq > local_seq {
                gaps.push((user.clone(), local_seq + 1, remote_seq));
            }
        }
        gaps
    }

    /// Get entries in a sequence range for a user (inclusive).
    pub fn get_range(
        &self,
        user: &PeerId,
        from_seq: SequenceNumber,
        to_seq: SequenceNumber,
    ) -> Vec<&AppendLogEntry> {
        self.entries
            .get(user)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.seq >= from_seq && e.seq <= to_seq)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all entries sorted by timestamp (for replay).
    pub fn all_entries_by_timestamp(&self) -> Vec<(&PeerId, &AppendLogEntry)> {
        let mut all: Vec<_> = self
            .entries
            .iter()
            .flat_map(|(user, entries)| entries.iter().map(move |e| (user, e)))
            .collect();
        all.sort_by_key(|(_, e)| e.timestamp);
        all
    }

    /// Get the current state vector.
    pub fn state_vector(&self) -> &HashMap<PeerId, SequenceNumber> {
        &self.state_vector
    }

    fn advance_watermark(&mut self, user: &PeerId) {
        let entries = match self.entries.get(user) {
            Some(e) => e,
            None => return,
        };
        let current = self.state_vector.get(user).copied().unwrap_or(0);
        let mut watermark = current;

        for entry in entries.iter().filter(|e| e.seq > current) {
            if entry.seq == watermark + 1 {
                watermark = entry.seq;
            } else {
                break;
            }
        }

        if watermark > current {
            self.state_vector.insert(user.clone(), watermark);
        }
    }
}

// ── Wire Types ──────────────────────────────────────────────────────────

/// Gossip message envelope. No version byte — WireMessage already has one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipMessage {
    pub origin: PeerId,
    pub timestamp: SharedTimestamp,
    pub payload: GossipPayload,
}

/// Full state snapshot data, sent periodically and on peer connect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshotData {
    pub file_register: FileRegister,
    pub position_register: PositionRegister,
    pub user_states: HashMap<PeerId, (UserState, SharedTimestamp)>,
    pub file_states: HashMap<PeerId, (FileState, SharedTimestamp)>,
    pub peer_generations: HashMap<PeerId, SharedTimestamp>,
    pub chat_vectors: HashMap<PeerId, SequenceNumber>,
    pub playlist_vectors: HashMap<PeerId, SequenceNumber>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipPayload {
    /// Periodic full state snapshot.
    StateSnapshot(Box<StateSnapshotData>),
    /// Eager-push of a new append log entry (via datagram).
    AppendEntry {
        log_type: LogType,
        user: PeerId,
        seq: SequenceNumber,
        data: Vec<u8>,
        timestamp: SharedTimestamp,
    },
    /// Request missing entries (via datagram).
    GapFillRequest {
        log_type: LogType,
        user: PeerId,
        from_seq: SequenceNumber,
        to_seq: SequenceNumber,
    },
    /// Response with missing entries (via reliable stream).
    GapFillResponse {
        log_type: LogType,
        user: PeerId,
        entries: Vec<(SequenceNumber, Vec<u8>, SharedTimestamp)>,
    },
}

// ── Local Events ────────────────────────────────────────────────────────

/// Events from the application layer into the sync engine.
pub enum LocalEvent {
    UserStateChanged(UserState),
    FileStateChanged(FileState),
    PositionUpdated { position: f64 },
    FileChanged { file_id: Option<ItemId> },
    ChatSent { text: String },
    PlaylistAction(PlaylistAction),
}

// ── Sync Engine ─────────────────────────────────────────────────────────

/// Mutable state inside the sync engine, protected by std::sync::Mutex.
/// Never held across .await points.
struct SyncInner {
    /// Our user state (LWW).
    local_user_state: LwwRegister<UserState>,
    /// Our file state (LWW).
    local_file_state: LwwRegister<FileState>,
    /// Chat append log.
    chat_log: AppendLog,
    /// Playlist append log.
    playlist_log: AppendLog,
    /// Debounce tracker for gap fill requests: (log_type, user) → last_request_time.
    gap_fill_debounce: HashMap<(LogType, PeerId), tokio::time::Instant>,
    /// Whether we're in burst mode (fast broadcast for 1s after state change).
    burst_until: Option<tokio::time::Instant>,
}

/// The sync engine: replicates state across peers via gossip.
///
/// Background tasks handle broadcasting, receiving, gap filling,
/// and connection events.
pub struct SyncEngine {
    clock_svc: Arc<ClockSyncService>,
    clock: SharedClock,
    local_peer: PeerId,
    state: Arc<SharedState>,
    inner: Mutex<SyncInner>,
    local_event_tx: mpsc::UnboundedSender<LocalEvent>,
    local_event_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<LocalEvent>>,
    broadcast_notify: Notify,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl SyncEngine {
    /// Create a new sync engine.
    pub fn new(
        clock_svc: Arc<ClockSyncService>,
        local_peer: PeerId,
        state: Arc<SharedState>,
    ) -> Arc<Self> {
        let clock = clock_svc.clock();
        let now = clock.now();
        let (local_event_tx, local_event_rx) = mpsc::unbounded_channel();

        // Register ourselves as a peer with our generation
        state.add_peer(local_peer.clone());
        state.update_peer_generation(&local_peer, now);
        state.set_user_state(&local_peer, UserState::Ready, now);
        state.set_file_state(&local_peer, FileState::Ready, now);

        Arc::new(Self {
            clock_svc,
            clock,
            local_peer,
            state,
            inner: Mutex::new(SyncInner {
                local_user_state: LwwRegister::new(UserState::Ready, now),
                local_file_state: LwwRegister::new(FileState::Ready, now),
                chat_log: AppendLog::new(),
                playlist_log: AppendLog::new(),
                gap_fill_debounce: HashMap::new(),
                burst_until: None,
            }),
            local_event_tx,
            local_event_rx: tokio::sync::Mutex::new(local_event_rx),
            broadcast_notify: Notify::new(),
            tasks: Mutex::new(Vec::new()),
        })
    }

    /// Get a reference to the shared state.
    pub fn shared_state(&self) -> &Arc<SharedState> {
        &self.state
    }

    /// Get a sender to push local events into the sync engine.
    pub fn local_event_sender(&self) -> mpsc::UnboundedSender<LocalEvent> {
        self.local_event_tx.clone()
    }

    /// Start background tasks. Call once after construction.
    pub fn start(self: &Arc<Self>) {
        let mut tasks = self.tasks.lock().unwrap();

        // 1. Gossip broadcaster
        tasks.push(tokio::spawn({
            let this = Arc::clone(self);
            async move { this.broadcast_loop().await }
        }));

        // 2. Datagram receiver
        tasks.push(tokio::spawn({
            let this = Arc::clone(self);
            async move { this.datagram_recv_loop().await }
        }));

        // 3. Reliable receiver
        tasks.push(tokio::spawn({
            let this = Arc::clone(self);
            async move { this.reliable_recv_loop().await }
        }));

        // 4. Connection event listener
        tasks.push(tokio::spawn({
            let this = Arc::clone(self);
            async move { this.connection_event_loop().await }
        }));

        // 5. Local event processor
        tasks.push(tokio::spawn({
            let this = Arc::clone(self);
            async move { this.local_event_loop().await }
        }));
    }

    // ── Background task: Gossip Broadcaster ─────────────────────────────

    async fn broadcast_loop(self: &Arc<Self>) {
        loop {
            let interval = self.compute_broadcast_interval();

            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = self.broadcast_notify.notified() => {}
            }

            self.broadcast_snapshot().await;
        }
    }

    fn compute_broadcast_interval(&self) -> Duration {
        let inner = self.inner.lock().unwrap();

        // Burst mode: 100ms for 1s after state change
        if let Some(until) = inner.burst_until
            && tokio::time::Instant::now() < until
        {
            return Duration::from_millis(100);
        }

        // Normal mode: 100ms playing, 1s paused
        let view = self.state.view();
        if view.is_playing {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(1)
        }
    }

    fn enter_burst_mode(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.burst_until = Some(tokio::time::Instant::now() + Duration::from_secs(1));
        drop(inner);
        self.broadcast_notify.notify_one();
    }

    async fn broadcast_snapshot(&self) {
        let msg = {
            let inner = self.inner.lock().unwrap();
            GossipMessage {
                origin: self.local_peer.clone(),
                timestamp: self.clock.now(),
                payload: GossipPayload::StateSnapshot(Box::new(StateSnapshotData {
                    file_register: self.state.file_register(),
                    position_register: self.state.position_register(),
                    user_states: self.state.raw_user_states(),
                    file_states: self.state.raw_file_states(),
                    peer_generations: self.state.peer_generations(),
                    chat_vectors: inner.chat_log.state_vector().clone(),
                    playlist_vectors: inner.playlist_log.state_vector().clone(),
                })),
            }
        };

        let data = match postcard::to_allocvec(&msg) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("failed to serialize gossip snapshot: {e}");
                return;
            }
        };

        let peers = self.clock_svc.connected_peers();
        for peer in peers {
            if peer == self.local_peer {
                continue;
            }
            if let Err(e) = self.clock_svc.send_app_datagram(&peer, &data).await {
                tracing::debug!(peer = %peer, "failed to send gossip snapshot: {e}");
            }
        }
    }

    // ── Background task: Datagram Receiver ──────────────────────────────

    async fn datagram_recv_loop(self: &Arc<Self>) {
        loop {
            let result = self.clock_svc.recv_app_datagram().await;
            match result {
                Ok((sender, data)) => {
                    match postcard::from_bytes::<GossipMessage>(&data) {
                        Ok(msg) => self.handle_gossip_message(sender, msg).await,
                        Err(e) => {
                            tracing::warn!("failed to deserialize gossip datagram: {e}");
                        }
                    }
                }
                Err(super::ConnectionError::Closed) => break,
                Err(e) => {
                    tracing::warn!("app datagram recv error: {e}");
                }
            }
        }
    }

    // ── Background task: Reliable Receiver ──────────────────────────────

    async fn reliable_recv_loop(self: &Arc<Self>) {
        loop {
            let result = self.clock_svc.recv_reliable().await;
            match result {
                Ok((sender, data)) => {
                    match postcard::from_bytes::<GossipMessage>(&data) {
                        Ok(msg) => self.handle_gossip_message(sender, msg).await,
                        Err(e) => {
                            tracing::warn!("failed to deserialize reliable message: {e}");
                        }
                    }
                }
                Err(super::ConnectionError::Closed) => break,
                Err(e) => {
                    tracing::warn!("reliable recv error: {e}");
                }
            }
        }
    }

    // ── Background task: Connection Events ──────────────────────────────

    async fn connection_event_loop(self: &Arc<Self>) {
        // Discover already-connected peers (events fired before start() are missed)
        for peer in self.clock_svc.connected_peers() {
            if peer != self.local_peer {
                self.state.add_peer(peer.clone());
                // Send immediate snapshot
                let msg = {
                    let inner = self.inner.lock().unwrap();
                    GossipMessage {
                        origin: self.local_peer.clone(),
                        timestamp: self.clock.now(),
                        payload: GossipPayload::StateSnapshot(Box::new(StateSnapshotData {
                            file_register: self.state.file_register(),
                            position_register: self.state.position_register(),
                            user_states: self.state.raw_user_states(),
                            file_states: self.state.raw_file_states(),
                            peer_generations: self.state.peer_generations(),
                            chat_vectors: inner.chat_log.state_vector().clone(),
                            playlist_vectors: inner.playlist_log.state_vector().clone(),
                        })),
                    }
                };
                if let Ok(data) = postcard::to_allocvec(&msg) {
                    let _ = self.clock_svc.send_app_datagram(&peer, &data).await;
                }
            }
        }

        let mut rx = self.clock_svc.subscribe();
        loop {
            match rx.recv().await {
                Ok(ConnectionEvent::PeerConnected(peer)) => {
                    if peer == self.local_peer {
                        continue;
                    }
                    tracing::info!(peer = %peer, "peer connected, sending snapshot");
                    self.state.add_peer(peer.clone());

                    // Send immediate snapshot to new peer
                    let msg = {
                        let inner = self.inner.lock().unwrap();
                        GossipMessage {
                            origin: self.local_peer.clone(),
                            timestamp: self.clock.now(),
                            payload: GossipPayload::StateSnapshot(Box::new(StateSnapshotData {
                                file_register: self.state.file_register(),
                                position_register: self.state.position_register(),
                                user_states: self.state.raw_user_states(),
                                file_states: self.state.raw_file_states(),
                                peer_generations: self.state.peer_generations(),
                                chat_vectors: inner.chat_log.state_vector().clone(),
                                playlist_vectors: inner.playlist_log.state_vector().clone(),
                            })),
                        }
                    };
                    if let Ok(data) = postcard::to_allocvec(&msg) {
                        let _ = self.clock_svc.send_app_datagram(&peer, &data).await;
                    }
                }
                Ok(ConnectionEvent::PeerDisconnected(peer)) => {
                    if peer == self.local_peer {
                        continue;
                    }
                    tracing::info!(peer = %peer, "peer disconnected");
                    self.state.remove_peer(&peer);
                    self.enter_burst_mode();
                }
                Ok(ConnectionEvent::ConnectionStateChanged { .. }) => {
                    // Informational only, no action needed
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("connection event listener lagged by {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    // ── Background task: Local Event Processor ──────────────────────────

    async fn local_event_loop(self: &Arc<Self>) {
        let mut rx = self.local_event_rx.lock().await;
        while let Some(event) = rx.recv().await {
            match event {
                LocalEvent::UserStateChanged(us) => {
                    let now = self.clock.now();
                    {
                        let mut inner = self.inner.lock().unwrap();
                        inner.local_user_state = LwwRegister::new(us, now);
                    }
                    self.state.set_user_state(&self.local_peer, us, now);
                    self.enter_burst_mode();
                }
                LocalEvent::FileStateChanged(fs) => {
                    let now = self.clock.now();
                    {
                        let mut inner = self.inner.lock().unwrap();
                        inner.local_file_state = LwwRegister::new(fs, now);
                    }
                    self.state.set_file_state(&self.local_peer, fs, now);
                    self.enter_burst_mode();
                }
                LocalEvent::PositionUpdated { position } => {
                    let now = self.clock.now();
                    let file_reg = self.state.file_register();
                    let pos_reg = PositionRegister {
                        position,
                        for_file: file_reg.file_id,
                        timestamp: now,
                        origin: self.local_peer.clone(),
                    };
                    self.state.update_position(pos_reg);
                }
                LocalEvent::FileChanged { file_id } => {
                    let now = self.clock.now();
                    let file_reg = FileRegister {
                        file_id,
                        timestamp: now,
                        origin: self.local_peer.clone(),
                    };
                    self.state.update_file_register(file_reg);
                    self.enter_burst_mode();
                }
                LocalEvent::ChatSent { text } => {
                    let now = self.clock.now();

                    // Peek at next seq (we're the only writer for our own peer)
                    let seq = {
                        let inner = self.inner.lock().unwrap();
                        inner.chat_log.state_vector()
                            .get(&self.local_peer).copied().unwrap_or(0) + 1
                    };

                    let msg = ChatMessage {
                        sender: self.local_peer.clone(),
                        text,
                        timestamp: now,
                        seq,
                    };
                    let data = postcard::to_allocvec(&msg).unwrap();

                    let actual_seq = {
                        let mut inner = self.inner.lock().unwrap();
                        inner.chat_log.append_local(&self.local_peer, data.clone(), now)
                    };
                    debug_assert_eq!(seq, actual_seq);

                    self.state.add_chat_message(msg);

                    // Eager push to all peers
                    self.eager_push(LogType::Chat, seq, data, now).await;
                }
                LocalEvent::PlaylistAction(action) => {
                    let now = self.clock.now();
                    let data = postcard::to_allocvec(&action).unwrap();

                    let seq = {
                        let mut inner = self.inner.lock().unwrap();
                        inner.playlist_log.append_local(&self.local_peer, data.clone(), now)
                    };

                    self.state.add_playlist_action(action, now);

                    // Eager push to all peers
                    self.eager_push(LogType::Playlist, seq, data, now).await;
                    self.enter_burst_mode();
                }
            }
        }
    }

    // ── Message Handling ────────────────────────────────────────────────

    async fn handle_gossip_message(&self, sender: PeerId, msg: GossipMessage) {
        let origin = msg.origin.clone();

        match msg.payload {
            GossipPayload::StateSnapshot(snapshot) => {
                let StateSnapshotData {
                    file_register,
                    position_register,
                    user_states,
                    file_states,
                    peer_generations,
                    chat_vectors,
                    playlist_vectors,
                } = *snapshot;
                let mut any_updated = false;

                // 1. Process peer generations first (gates per-peer state acceptance)
                for (peer, generation) in &peer_generations {
                    self.state.update_peer_generation(peer, *generation);
                }

                // 2. Merge file register BEFORE position register
                //    (position is conditional on file_id match)
                if self.state.update_file_register(file_register) {
                    any_updated = true;
                }

                // 3. Merge position register (rejected if for_file doesn't match)
                if self.state.update_position(position_register) {
                    any_updated = true;
                }

                // 4. Merge user states (per-peer LWW) — skip entries with stale generation
                for (peer, (us, ts)) in &user_states {
                    if let Some(known_gen) = self.state.peer_generation(peer)
                        && let Some(msg_gen) = peer_generations.get(peer)
                        && *msg_gen < known_gen
                    {
                        continue; // stale generation, skip
                    }
                    if self.state.set_user_state(peer, *us, *ts) {
                        any_updated = true;
                    }
                }

                // 5. Merge file states (per-peer LWW) — same generation check
                for (peer, (fs, ts)) in &file_states {
                    if let Some(known_gen) = self.state.peer_generation(peer)
                        && let Some(msg_gen) = peer_generations.get(peer)
                        && *msg_gen < known_gen
                    {
                        continue;
                    }
                    if self.state.set_file_state(peer, *fs, *ts) {
                        any_updated = true;
                    }
                }

                // Check for gaps in append logs
                let (chat_gaps, playlist_gaps) = {
                    let inner = self.inner.lock().unwrap();
                    (
                        inner.chat_log.find_gaps(&chat_vectors),
                        inner.playlist_log.find_gaps(&playlist_vectors),
                    )
                };

                // Request gap fills
                for (user, from_seq, to_seq) in chat_gaps {
                    self.request_gap_fill(
                        &sender,
                        &origin,
                        LogType::Chat,
                        &user,
                        from_seq,
                        to_seq,
                    )
                    .await;
                }
                for (user, from_seq, to_seq) in playlist_gaps {
                    self.request_gap_fill(
                        &sender,
                        &origin,
                        LogType::Playlist,
                        &user,
                        from_seq,
                        to_seq,
                    )
                    .await;
                }

                // Forward rule: if we adopted newer state, forward to peers - {sender, origin}
                if any_updated {
                    self.forward_snapshot(&sender, &origin).await;
                }
            }
            GossipPayload::AppendEntry {
                log_type,
                user,
                seq,
                data,
                timestamp,
            } => {
                let inserted = {
                    let mut inner = self.inner.lock().unwrap();
                    match log_type {
                        LogType::Chat => inner.chat_log.insert(&user, seq, data.clone(), timestamp),
                        LogType::Playlist => {
                            inner.playlist_log.insert(&user, seq, data.clone(), timestamp)
                        }
                    }
                };

                if inserted {
                    // Apply to shared state
                    match log_type {
                        LogType::Chat => {
                            if let Ok(msg) = postcard::from_bytes::<ChatMessage>(&data) {
                                self.state.add_chat_message(msg);
                            }
                        }
                        LogType::Playlist => {
                            if let Ok(action) = postcard::from_bytes::<PlaylistAction>(&data) {
                                self.state.add_playlist_action(action, timestamp);
                            }
                        }
                    }

                    // Forward to other peers
                    self.forward_append_entry(
                        &sender, &origin, log_type, &user, seq, data, timestamp,
                    )
                    .await;
                }
            }
            GossipPayload::GapFillRequest {
                log_type,
                user,
                from_seq,
                to_seq,
            } => {
                self.handle_gap_fill_request(&sender, log_type, &user, from_seq, to_seq)
                    .await;
            }
            GossipPayload::GapFillResponse {
                log_type,
                user,
                entries,
            } => {
                self.handle_gap_fill_response(log_type, &user, entries);
            }
        }
    }

    async fn request_gap_fill(
        &self,
        sender: &PeerId,
        origin: &PeerId,
        log_type: LogType,
        user: &PeerId,
        from_seq: SequenceNumber,
        to_seq: SequenceNumber,
    ) {
        // Debounce: 500ms per (log_type, user)
        let key = (log_type, user.clone());
        {
            let mut inner = self.inner.lock().unwrap();
            let now = tokio::time::Instant::now();
            if let Some(last) = inner.gap_fill_debounce.get(&key)
                && now.duration_since(*last) < Duration::from_millis(500)
            {
                return;
            }
            inner.gap_fill_debounce.insert(key, now);
        }

        let msg = GossipMessage {
            origin: self.local_peer.clone(),
            timestamp: self.clock.now(),
            payload: GossipPayload::GapFillRequest {
                log_type,
                user: user.clone(),
                from_seq,
                to_seq,
            },
        };

        let data = match postcard::to_allocvec(&msg) {
            Ok(d) => d,
            Err(_) => return,
        };

        // Prefer origin peer, fall back to sender
        let target = if self.clock_svc.connected_peers().contains(origin) && *origin != self.local_peer {
            origin
        } else {
            sender
        };

        let _ = self.clock_svc.send_app_datagram(target, &data).await;
    }

    async fn handle_gap_fill_request(
        &self,
        requester: &PeerId,
        log_type: LogType,
        user: &PeerId,
        from_seq: SequenceNumber,
        to_seq: SequenceNumber,
    ) {
        let entries = {
            let inner = self.inner.lock().unwrap();
            let log = match log_type {
                LogType::Chat => &inner.chat_log,
                LogType::Playlist => &inner.playlist_log,
            };
            log.get_range(user, from_seq, to_seq)
                .into_iter()
                .map(|e| (e.seq, e.data.clone(), e.timestamp))
                .collect::<Vec<_>>()
        };

        if entries.is_empty() {
            return;
        }

        let msg = GossipMessage {
            origin: self.local_peer.clone(),
            timestamp: self.clock.now(),
            payload: GossipPayload::GapFillResponse {
                log_type,
                user: user.clone(),
                entries,
            },
        };

        if let Ok(data) = postcard::to_allocvec(&msg) {
            // Gap fill responses use reliable stream
            if let Err(e) = self.clock_svc.send_reliable(requester, &data).await {
                tracing::debug!(
                    requester = %requester,
                    "failed to send gap fill response: {e}"
                );
            }
        }
    }

    fn handle_gap_fill_response(
        &self,
        log_type: LogType,
        user: &PeerId,
        entries: Vec<(SequenceNumber, Vec<u8>, SharedTimestamp)>,
    ) {
        let mut inner = self.inner.lock().unwrap();
        let log = match log_type {
            LogType::Chat => &mut inner.chat_log,
            LogType::Playlist => &mut inner.playlist_log,
        };

        for (seq, data, timestamp) in entries {
            if log.insert(user, seq, data.clone(), timestamp) {
                // Apply to shared state
                match log_type {
                    LogType::Chat => {
                        if let Ok(msg) = postcard::from_bytes::<ChatMessage>(&data) {
                            self.state.add_chat_message(msg);
                        }
                    }
                    LogType::Playlist => {
                        if let Ok(action) = postcard::from_bytes::<PlaylistAction>(&data) {
                            self.state.add_playlist_action(action, timestamp);
                        }
                    }
                }
            }
        }
    }

    // ── Forwarding ──────────────────────────────────────────────────────

    async fn forward_snapshot(&self, sender: &PeerId, origin: &PeerId) {
        let msg = {
            let inner = self.inner.lock().unwrap();
            GossipMessage {
                origin: origin.clone(),
                timestamp: self.clock.now(),
                payload: GossipPayload::StateSnapshot(Box::new(StateSnapshotData {
                    file_register: self.state.file_register(),
                    position_register: self.state.position_register(),
                    user_states: self.state.raw_user_states(),
                    file_states: self.state.raw_file_states(),
                    peer_generations: self.state.peer_generations(),
                    chat_vectors: inner.chat_log.state_vector().clone(),
                    playlist_vectors: inner.playlist_log.state_vector().clone(),
                })),
            }
        };

        let data = match postcard::to_allocvec(&msg) {
            Ok(d) => d,
            Err(_) => return,
        };

        let peers = self.clock_svc.connected_peers();
        for peer in peers {
            if peer == *sender || peer == *origin || peer == self.local_peer {
                continue;
            }
            let _ = self.clock_svc.send_app_datagram(&peer, &data).await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn forward_append_entry(
        &self,
        sender: &PeerId,
        origin: &PeerId,
        log_type: LogType,
        user: &PeerId,
        seq: SequenceNumber,
        data: Vec<u8>,
        timestamp: SharedTimestamp,
    ) {
        let msg = GossipMessage {
            origin: origin.clone(),
            timestamp: self.clock.now(),
            payload: GossipPayload::AppendEntry {
                log_type,
                user: user.clone(),
                seq,
                data,
                timestamp,
            },
        };

        let wire = match postcard::to_allocvec(&msg) {
            Ok(d) => d,
            Err(_) => return,
        };

        let peers = self.clock_svc.connected_peers();
        for peer in peers {
            if peer == *sender || peer == *origin || peer == self.local_peer {
                continue;
            }
            let _ = self.clock_svc.send_app_datagram(&peer, &wire).await;
        }
    }

    async fn eager_push(
        &self,
        log_type: LogType,
        seq: SequenceNumber,
        data: Vec<u8>,
        timestamp: SharedTimestamp,
    ) {
        let msg = GossipMessage {
            origin: self.local_peer.clone(),
            timestamp: self.clock.now(),
            payload: GossipPayload::AppendEntry {
                log_type,
                user: self.local_peer.clone(),
                seq,
                data,
                timestamp,
            },
        };

        let wire = match postcard::to_allocvec(&msg) {
            Ok(d) => d,
            Err(_) => return,
        };

        let peers = self.clock_svc.connected_peers();
        for peer in peers {
            if peer == self.local_peer {
                continue;
            }
            let _ = self.clock_svc.send_app_datagram(&peer, &wire).await;
        }
    }
}

impl Drop for SyncEngine {
    fn drop(&mut self) {
        let tasks = self.tasks.lock().unwrap();
        for task in tasks.iter() {
            task.abort();
        }
    }
}

// ── Unit Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(name: &str) -> PeerId {
        PeerId(name.to_string())
    }

    fn ts(us: i64) -> SharedTimestamp {
        SharedTimestamp(us)
    }

    #[test]
    fn lww_newer_wins() {
        let mut reg = LwwRegister::new(1, ts(100));
        assert!(reg.merge(2, ts(200)));
        assert_eq!(reg.value, 2);
    }

    #[test]
    fn lww_older_ignored() {
        let mut reg = LwwRegister::new(2, ts(200));
        assert!(!reg.merge(1, ts(100)));
        assert_eq!(reg.value, 2);
    }

    #[test]
    fn lww_equal_timestamp_ignored() {
        let mut reg = LwwRegister::new(1, ts(100));
        assert!(!reg.merge(2, ts(100)));
        assert_eq!(reg.value, 1);
    }

    #[test]
    fn append_log_local() {
        let mut log = AppendLog::new();
        let seq1 = log.append_local(&peer("alice"), vec![1], ts(100));
        let seq2 = log.append_local(&peer("alice"), vec![2], ts(200));
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_eq!(log.state_vector()[&peer("alice")], 2);
    }

    #[test]
    fn append_log_insert_in_order() {
        let mut log = AppendLog::new();
        assert!(log.insert(&peer("bob"), 1, vec![1], ts(100)));
        assert!(log.insert(&peer("bob"), 2, vec![2], ts(200)));
        assert_eq!(log.state_vector()[&peer("bob")], 2);
    }

    #[test]
    fn append_log_insert_out_of_order() {
        let mut log = AppendLog::new();
        // Insert seq 2 before seq 1
        assert!(log.insert(&peer("bob"), 2, vec![2], ts(200)));
        assert_eq!(log.state_vector().get(&peer("bob")).copied().unwrap_or(0), 0);

        // Now insert seq 1 — watermark should advance to 2
        assert!(log.insert(&peer("bob"), 1, vec![1], ts(100)));
        assert_eq!(log.state_vector()[&peer("bob")], 2);
    }

    #[test]
    fn append_log_duplicate_rejected() {
        let mut log = AppendLog::new();
        assert!(log.insert(&peer("bob"), 1, vec![1], ts(100)));
        assert!(!log.insert(&peer("bob"), 1, vec![1], ts(100)));
    }

    #[test]
    fn append_log_find_gaps() {
        let mut log = AppendLog::new();
        log.insert(&peer("alice"), 1, vec![1], ts(100));
        log.insert(&peer("alice"), 2, vec![2], ts(200));

        let remote = HashMap::from([
            (peer("alice"), 5u64),
            (peer("bob"), 3u64),
        ]);

        let gaps = log.find_gaps(&remote);
        assert_eq!(gaps.len(), 2);

        // alice: have up to 2, remote has 5 → gap 3..5
        let alice_gap = gaps.iter().find(|(p, _, _)| *p == peer("alice")).unwrap();
        assert_eq!(alice_gap.1, 3);
        assert_eq!(alice_gap.2, 5);

        // bob: have 0, remote has 3 → gap 1..3
        let bob_gap = gaps.iter().find(|(p, _, _)| *p == peer("bob")).unwrap();
        assert_eq!(bob_gap.1, 1);
        assert_eq!(bob_gap.2, 3);
    }

    #[test]
    fn append_log_get_range() {
        let mut log = AppendLog::new();
        for i in 1..=5 {
            log.insert(&peer("alice"), i, vec![i as u8], ts(i as i64 * 100));
        }
        let range = log.get_range(&peer("alice"), 2, 4);
        assert_eq!(range.len(), 3);
        assert_eq!(range[0].seq, 2);
        assert_eq!(range[2].seq, 4);
    }

    #[test]
    fn append_log_all_entries_sorted() {
        let mut log = AppendLog::new();
        log.insert(&peer("alice"), 1, vec![1], ts(200));
        log.insert(&peer("bob"), 1, vec![2], ts(100));
        log.insert(&peer("alice"), 2, vec![3], ts(300));

        let all = log.all_entries_by_timestamp();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].1.timestamp, ts(100));
        assert_eq!(all[1].1.timestamp, ts(200));
        assert_eq!(all[2].1.timestamp, ts(300));
    }

    #[test]
    fn gossip_message_round_trip() {
        let msg = GossipMessage {
            origin: peer("alice"),
            timestamp: ts(12345),
            payload: GossipPayload::StateSnapshot(Box::new(StateSnapshotData {
                file_register: FileRegister {
                    file_id: None,
                    timestamp: ts(12345),
                    origin: peer("alice"),
                },
                position_register: PositionRegister {
                    position: 42.5,
                    for_file: None,
                    timestamp: ts(12345),
                    origin: peer("alice"),
                },
                user_states: HashMap::from([(peer("alice"), (UserState::Ready, ts(100)))]),
                file_states: HashMap::from([(peer("alice"), (FileState::Ready, ts(100)))]),
                peer_generations: HashMap::from([(peer("alice"), ts(100))]),
                chat_vectors: HashMap::new(),
                playlist_vectors: HashMap::new(),
            })),
        };

        let encoded = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&encoded).unwrap();

        assert_eq!(decoded.origin, peer("alice"));
        assert_eq!(decoded.timestamp, ts(12345));
        match decoded.payload {
            GossipPayload::StateSnapshot(snapshot) => {
                assert_eq!(snapshot.position_register.position, 42.5);
            }
            _ => panic!("expected StateSnapshot"),
        }
    }
}
