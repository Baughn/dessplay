//! The rendezvous server's connection handling and authoritative logic.
//!
//! Phase 5 scope: accept, authenticate, answer time-sync probes, sync
//! state, track presence (Present -> Lost -> Departed), own the EOF ->
//! next-file transition, and compact state on schedule. File-transfer
//! relay arrives in Phase 9.
//!
//! Generic over [`Listener`] so tests run it over the simulated
//! transport and production over QUIC.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dessplay_core::derive::{self, DerivedUserState};
use dessplay_core::net::{
    Listener, PeerInfo, Presence, Role, ServerControl, Transport, TransportEvent, WireMessage,
};
use dessplay_core::types::{
    ActorId, Ed2kHash, Epoch, PlaybackIntent, SeekAuthority, SharedTimestamp, UserId,
};
use dessplay_core::wire;
use dessplay_core::{CrdtOp, CrdtState, StateSnapshot};

use crate::storage::ServerStorage;

/// How the server reads its clock (unix millis — this *is* the shared
/// clock everyone syncs to). Injected for paused-time tests.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// A clock backed by the system time. The production choice.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    })
}

/// When compaction runs (see docs/sync-state.md, Compaction).
#[derive(Clone, Copy, Debug)]
pub enum CompactionSchedule {
    /// Daily at the given UTC wall-clock time. The production setting;
    /// pick an hour far from watch-party time.
    DailyUtc {
        /// Hour, 0-23.
        hour: u8,
        /// Minute, 0-59.
        minute: u8,
    },
    /// Fixed period. For tests (paused time makes it exact).
    Every(Duration),
    /// Never compact automatically.
    Disabled,
}

/// Server configuration.
pub struct ServerConfig {
    /// The shared room password.
    pub password: String,
    /// How long a connection may sit unauthenticated before being cut.
    pub auth_timeout: Duration,
    /// Compaction schedule.
    pub compaction: CompactionSchedule,
    /// How many chat messages survive compaction (the rest are archived
    /// to SQLite first).
    pub chat_keep: usize,
}

impl ServerConfig {
    /// Defaults with the given password: daily compaction at 12:00 UTC,
    /// 100 chat messages kept.
    pub fn new(password: impl Into<String>) -> Self {
        Self {
            password: password.into(),
            auth_timeout: Duration::from_secs(10),
            compaction: CompactionSchedule::DailyUtc {
                hour: 12,
                minute: 0,
            },
            chat_keep: 100,
        }
    }
}

struct PeerEntry<T> {
    /// Id of the connection backing this entry (live or last).
    conn_id: u64,
    /// The live connection; `None` once Lost or Departed.
    conn: Option<Arc<T>>,
    info: PeerInfo,
    /// Shared-clock millis when the connection died (drives the
    /// Lost -> Departed promotion).
    lost_at: Option<u64>,
}

struct Registry<T> {
    peers: HashMap<UserId, PeerEntry<T>>,
}

impl<T> Registry<T> {
    fn peer_infos(&self) -> Vec<PeerInfo> {
        let mut infos: Vec<PeerInfo> = self.peers.values().map(|p| p.info.clone()).collect();
        infos.sort_by(|a, b| a.username.cmp(&b.username));
        infos
    }
}

/// State shared by all connection tasks.
struct Shared<T> {
    registry: Mutex<Registry<T>>,
    state: Mutex<CrdtState>,
    /// Read or written only while holding the `state` lock, so epoch
    /// and state are always observed as a consistent pair.
    epoch: AtomicU64,
    /// Monotonic-stamp bookkeeping for server-authored ops.
    last_issued: AtomicU64,
    dirty: AtomicBool,
    storage: Mutex<Option<ServerStorage>>,
    clock: Clock,
}

impl<T: Transport> Shared<T> {
    /// A consistent (epoch, state) snapshot.
    fn snapshot(&self) -> StateSnapshot {
        let state = lock(&self.state);
        StateSnapshot {
            epoch: Epoch(self.epoch.load(Ordering::SeqCst)),
            state: state.clone(),
        }
    }

    /// Lamport-monotonic shared-clock stamp for server-authored ops.
    /// Server writes are LWW like everyone else's; the floor is raised
    /// by [`Self::observe`] for every client timestamp seen, so a
    /// server write issued causally after a client's (forced pause
    /// after a play press, in the same millisecond) always dominates.
    fn stamp(&self) -> SharedTimestamp {
        let now = (self.clock)();
        let prev = self
            .last_issued
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
                Some(now.max(last + 1))
            })
            .unwrap_or(0);
        SharedTimestamp(now.max(prev + 1))
    }

    /// Raise the Lamport floor to an observed client timestamp.
    fn observe(&self, ts: Option<SharedTimestamp>) {
        if let Some(ts) = ts {
            self.last_issued.fetch_max(ts.0, Ordering::SeqCst);
        }
    }

    fn flush(&self) {
        if !self.dirty.swap(false, Ordering::SeqCst) {
            return;
        }
        let snapshot = self.snapshot();
        let now = (self.clock)() as i64;
        if let Some(storage) = &*lock(&self.storage)
            && let Err(e) = storage.save_state(&snapshot, now)
        {
            tracing::error!("server state flush failed: {e}");
            self.dirty.store(true, Ordering::SeqCst);
        }
    }

    /// All live connections, optionally excluding one.
    fn live_conns(&self, skip: Option<u64>) -> Vec<Arc<T>> {
        lock(&self.registry)
            .peers
            .values()
            .filter(|p| Some(p.conn_id) != skip)
            .filter_map(|p| p.conn.clone())
            .collect()
    }

    /// Broadcast one message to every live connection: reliable, plus
    /// an eager datagram copy when it fits.
    async fn broadcast_op(&self, msg: ServerControl, skip: Option<u64>) {
        let Ok(frame) = wire::encode(&WireMessage::Control(msg)) else {
            return;
        };
        for conn in self.live_conns(skip) {
            let _ = conn.send_control(&frame).await;
            if conn
                .max_datagram_size()
                .is_some_and(|max| frame.len() <= max)
            {
                let _ = conn.send_datagram(&frame).await;
            }
        }
    }

    /// Apply a server-authored mutation and broadcast it to everyone.
    async fn server_write(
        &self,
        mutate: impl FnOnce(&mut CrdtState, ActorId, SharedTimestamp) -> CrdtOp,
    ) {
        let ts = self.stamp();
        let (epoch, op) = {
            let mut state = lock(&self.state);
            let op = mutate(&mut state, ActorId::SERVER, ts);
            (Epoch(self.epoch.load(Ordering::SeqCst)), op)
        };
        self.dirty.store(true, Ordering::SeqCst);
        self.broadcast_op(ServerControl::StateOp { epoch, op }, None)
            .await;
    }

    /// Force the playback-intent latch to Paused (lost connection,
    /// graceful quit, departure, EOF). Idempotent in effect.
    async fn force_pause(&self) {
        self.server_write(|state, actor, ts| {
            state.set_playback_intent(actor, ts, PlaybackIntent::Paused)
        })
        .await;
    }

    /// If `user` holds seek authority, the server takes it (so nobody
    /// syncs to a ghost).
    async fn take_authority_from(&self, user: &UserId) {
        let holds = {
            let state = lock(&self.state);
            dessplay_core::resolve_value(&state.seek_authority)
                == Some(SeekAuthority::User(user.clone()))
        };
        if holds {
            self.server_write(|state, actor, ts| {
                state.set_seek_authority(actor, ts, SeekAuthority::Server)
            })
            .await;
        }
    }
}

/// Cadence of StateHash broadcasts and storage flushes.
const HASH_INTERVAL: Duration = Duration::from_secs(30);
const FLUSH_INTERVAL: Duration = Duration::from_secs(30);
/// How often the presence sweeper looks for Lost -> Departed promotions.
const SWEEP_INTERVAL: Duration = Duration::from_secs(1);
/// How long after the connection died (~30s of silence, the QUIC idle
/// timeout) a Lost peer becomes Departed — 60s of silence total, per
/// docs/design.md (Presence).
const DEPART_AFTER_MILLIS: u64 = 30_000;

/// Run the accept loop until the listener fails. Each connection gets
/// its own task. State is loaded from `storage` if present; a fresh
/// server starts at epoch 1 (clients persist real epochs, so epoch 0
/// always reads as stale and gets a snapshot).
pub async fn run<L: Listener>(
    listener: L,
    config: ServerConfig,
    clock: Clock,
    storage: Option<ServerStorage>,
) where
    L::Conn: Transport,
{
    let config = Arc::new(config);
    let initial = storage
        .as_ref()
        .and_then(|s| s.load_state().ok().flatten())
        .unwrap_or(StateSnapshot {
            epoch: Epoch(1),
            state: CrdtState::new(),
        });
    // Lamport floor from stored state: a restart must not re-issue
    // stamps the previous incarnation already spent.
    let last_issued = initial.state.max_lww_timestamp().0;
    let shared = Arc::new(Shared::<L::Conn> {
        registry: Mutex::new(Registry {
            peers: HashMap::new(),
        }),
        state: Mutex::new(initial.state),
        epoch: AtomicU64::new(initial.epoch.0),
        last_issued: AtomicU64::new(last_issued),
        dirty: AtomicBool::new(false),
        storage: Mutex::new(storage),
        clock: Arc::clone(&clock),
    });
    let mut next_conn_id: u64 = 0;

    // Divergence-alarm hashes.
    {
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(HASH_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            tick.tick().await; // skip the immediate tick
            loop {
                tick.tick().await;
                let msg = {
                    let state = lock(&shared.state);
                    ServerControl::StateHash {
                        epoch: Epoch(shared.epoch.load(Ordering::SeqCst)),
                        hash: state.view_hash(),
                    }
                };
                for conn in shared.live_conns(None) {
                    send_control(&*conn, &msg).await;
                }
            }
        });
    }
    // Periodic persistence.
    {
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(FLUSH_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            tick.tick().await;
            loop {
                tick.tick().await;
                shared.flush();
            }
        });
    }
    // Presence sweeper: Lost -> Departed promotions.
    {
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SWEEP_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                sweep_departed(&shared).await;
            }
        });
    }
    // Scheduled compaction.
    match config.compaction {
        CompactionSchedule::Disabled => {}
        CompactionSchedule::Every(period) => {
            let shared = Arc::clone(&shared);
            let chat_keep = config.chat_keep;
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(period).await;
                    run_compaction(&shared, chat_keep).await;
                }
            });
        }
        CompactionSchedule::DailyUtc { hour, minute } => {
            let shared = Arc::clone(&shared);
            let chat_keep = config.chat_keep;
            tokio::spawn(async move {
                loop {
                    let wait = millis_until_daily_utc((shared.clock)(), hour, minute);
                    tokio::time::sleep(Duration::from_millis(wait)).await;
                    run_compaction(&shared, chat_keep).await;
                }
            });
        }
    }

    loop {
        let (conn, remote) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::info!("listener stopped: {e}");
                shared.flush();
                return;
            }
        };
        let conn_id = next_conn_id;
        next_conn_id += 1;
        let config = Arc::clone(&config);
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            serve_connection(Arc::new(conn), conn_id, remote, config, shared).await;
        });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Milliseconds from `now` (unix millis) until the next daily UTC
/// occurrence of `hour:minute`. Never returns 0 — an exact hit waits a
/// full day, which keeps the schedule loop from spinning.
fn millis_until_daily_utc(now: u64, hour: u8, minute: u8) -> u64 {
    const DAY: u64 = 24 * 60 * 60 * 1000;
    let target = (hour as u64 * 60 + minute as u64) * 60 * 1000;
    let into_day = now % DAY;
    let wait = (target + DAY - into_day) % DAY;
    if wait == 0 { DAY } else { wait }
}

async fn send_control<T: Transport>(conn: &T, msg: &ServerControl) -> bool {
    let Ok(frame) = wire::encode(&WireMessage::Control(msg.clone())) else {
        return false;
    };
    conn.send_control(&frame).await.is_ok()
}

/// Push the current peer list (all presence stages) to every live
/// connection.
async fn broadcast_peer_list<T: Transport>(shared: &Shared<T>) {
    let peers = lock(&shared.registry).peer_infos();
    let msg = ServerControl::PeerList { peers };
    for conn in shared.live_conns(None) {
        send_control(&*conn, &msg).await;
    }
}

/// Promote Lost peers past the departure threshold, then apply the
/// departure consequences (peer-list push, forced pause, seek-authority
/// rescue).
async fn sweep_departed<T: Transport>(shared: &Shared<T>) {
    let now = (shared.clock)();
    let promoted: Vec<(UserId, Role)> = {
        let mut registry = lock(&shared.registry);
        registry
            .peers
            .iter_mut()
            .filter(|(_, entry)| {
                entry.info.presence == Presence::Lost
                    && entry
                        .lost_at
                        .is_some_and(|t| now.saturating_sub(t) >= DEPART_AFTER_MILLIS)
            })
            .map(|(user, entry)| {
                entry.info.presence = Presence::Departed;
                (user.clone(), entry.info.role)
            })
            .collect()
    };
    if promoted.is_empty() {
        return;
    }
    broadcast_peer_list(shared).await;
    for (user, role) in promoted {
        tracing::info!("{user:?} departed");
        if role == Role::Interactive {
            shared.force_pause().await;
            shared.take_authority_from(&user).await;
        }
    }
}

/// How an authenticated connection ended.
enum AuthedEnd {
    /// The client said Goodbye: remove immediately, no Lost stage.
    Goodbye,
    /// The connection died: the user becomes Lost.
    Lost(String),
}

async fn serve_connection<T: Transport>(
    conn: Arc<T>,
    conn_id: u64,
    remote: SocketAddr,
    config: Arc<ServerConfig>,
    shared: Arc<Shared<T>>,
) {
    let clock = Arc::clone(&shared.clock);
    // ---- Await Auth (bounded).
    let auth = tokio::time::timeout(config.auth_timeout, async {
        loop {
            match conn.recv().await {
                Ok(TransportEvent::Control(bytes)) => {
                    match wire::decode::<WireMessage>(&bytes) {
                        Ok(WireMessage::Control(ServerControl::Auth {
                            username,
                            password,
                            role,
                            epoch,
                        })) => return Some((username, password, role, epoch)),
                        Ok(_) | Err(_) => return None, // protocol violation
                    }
                }
                Ok(TransportEvent::Closed { .. }) | Err(_) => return None,
                Ok(_) => continue, // ignore datagrams/streams pre-auth
            }
        }
    })
    .await;

    let Ok(Some((username, password, role, client_epoch))) = auth else {
        conn.close("authentication required").await;
        return;
    };
    if password != config.password {
        send_control(&*conn, &ServerControl::AuthFailed).await;
        conn.close("bad password").await;
        return;
    }

    // ---- Register, superseding any existing entry for this user (a
    // reconnecting client — possibly one whose old connection hasn't
    // died yet, possibly one currently Lost or Departed).
    let superseded: Option<Arc<T>> = lock(&shared.registry)
        .peers
        .insert(
            username.clone(),
            PeerEntry {
                conn_id,
                conn: Some(Arc::clone(&conn)),
                info: PeerInfo {
                    username: username.clone(),
                    role,
                    presence: Presence::Present,
                    addresses: vec![remote],
                    connected_since: clock(),
                },
                lost_at: None,
            },
        )
        .and_then(|old| old.conn);
    if let Some(old) = superseded {
        old.close("superseded by a new connection").await;
    }

    send_control(
        &*conn,
        &ServerControl::AuthOk {
            observed_addr: remote,
        },
    )
    .await;
    broadcast_peer_list(&shared).await;

    // ---- Initial state sync: merge for a current epoch, snapshot for
    // a stale one (see sync-state.md, Sync Flow).
    let snapshot = shared.snapshot();
    let initial = if client_epoch == snapshot.epoch {
        ServerControl::StateMerge(snapshot)
    } else {
        ServerControl::StateSnapshot(snapshot)
    };
    send_control(&*conn, &initial).await;
    tracing::info!("{username:?} connected from {remote} as {role:?}");

    // ---- Serve until the connection ends.
    let end = serve_authed(&*conn, conn_id, &username, role, &shared).await;

    // A newer connection may have superseded us while we were closing;
    // in that case the registry entry is no longer ours to touch.
    let still_ours = {
        let mut registry = lock(&shared.registry);
        match registry.peers.get_mut(&username) {
            Some(entry) if entry.conn_id == conn_id => {
                match end {
                    AuthedEnd::Goodbye => {
                        registry.peers.remove(&username);
                    }
                    AuthedEnd::Lost(_) => {
                        entry.conn = None;
                        entry.info.presence = Presence::Lost;
                        entry.lost_at = Some(clock());
                    }
                }
                true
            }
            _ => false,
        }
    };
    if !still_ours {
        return;
    }

    match end {
        AuthedEnd::Goodbye => {
            tracing::info!("{username:?} quit");
            conn.close("goodbye").await;
            broadcast_peer_list(&shared).await;
            if role == Role::Interactive {
                shared.force_pause().await;
                shared.take_authority_from(&username).await;
            }
        }
        AuthedEnd::Lost(reason) => {
            tracing::info!("{username:?} lost: {reason}");
            broadcast_peer_list(&shared).await;
            if role == Role::Interactive {
                shared.force_pause().await;
            }
        }
    }
}

/// The post-auth message loop.
async fn serve_authed<T: Transport>(
    conn: &T,
    conn_id: u64,
    username: &UserId,
    role: Role,
    shared: &Shared<T>,
) -> AuthedEnd {
    let clock = &shared.clock;
    loop {
        let event = match conn.recv().await {
            Ok(event) => event,
            Err(e) => return AuthedEnd::Lost(e.to_string()),
        };
        let (payload, via_datagram) = match event {
            TransportEvent::Control(bytes) => (bytes, false),
            TransportEvent::Datagram(bytes) => (bytes, true),
            TransportEvent::IncomingStream(_) => continue, // Phase 9
            TransportEvent::Closed { reason } => return AuthedEnd::Lost(reason),
        };
        let msg: WireMessage = match wire::decode(&payload) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::warn!("undecodable client message: {e}");
                continue;
            }
        };
        let WireMessage::Control(msg) = msg;
        match msg {
            ServerControl::Goodbye => return AuthedEnd::Goodbye,
            ServerControl::TimeSyncRequest { client_send } => {
                let server_recv = clock();
                let response = ServerControl::TimeSyncResponse {
                    client_send,
                    server_recv,
                    server_send: clock(),
                };
                // Answer on the channel the probe used, so stream
                // retransmissions never pollute datagram RTTs.
                let Ok(frame) = wire::encode(&WireMessage::Control(response)) else {
                    continue;
                };
                let sent = if via_datagram {
                    conn.send_datagram(&frame).await.is_ok()
                } else {
                    conn.send_control(&frame).await.is_ok()
                };
                if !sent && conn.send_control(&frame).await.is_err() {
                    return AuthedEnd::Lost("send failed".into());
                }
            }
            ServerControl::StateOp { epoch, op } => {
                shared.observe(op.lww_timestamp());
                let applied = {
                    let mut state = lock(&shared.state);
                    if epoch != Epoch(shared.epoch.load(Ordering::SeqCst)) {
                        // Op from across a compaction boundary: applying
                        // it would pollute the rebuilt state's dot
                        // sequences. The client adopts the new snapshot
                        // and pushes a merge if it had anything to say.
                        tracing::warn!("dropping stale-epoch op from {username:?}");
                        false
                    } else if via_datagram {
                        // Unordered path: refuse anything that could
                        // mask a per-origin gap.
                        state.apply_if_orderly(op.clone())
                    } else {
                        state.apply(op.clone());
                        true
                    }
                };
                if !applied {
                    continue;
                }
                shared.dirty.store(true, Ordering::SeqCst);
                let now_playing_changed = matches!(op, CrdtOp::NowPlaying(_)) && !via_datagram;
                shared
                    .broadcast_op(ServerControl::StateOp { epoch, op }, Some(conn_id))
                    .await;
                // File change: the server takes seek authority so the
                // transition has one position source (everyone resets
                // to 0; a real user seek will take it right back).
                if now_playing_changed {
                    shared
                        .server_write(|state, actor, ts| {
                            state.set_seek_authority(actor, ts, SeekAuthority::Server)
                        })
                        .await;
                }
            }
            ServerControl::EofReached { file } => {
                handle_eof(shared, username, role, file).await;
            }
            ServerControl::RequestMerge => {
                tracing::warn!("client requested a divergence-heal merge");
                send_control(conn, &ServerControl::StateMerge(shared.snapshot())).await;
            }
            ServerControl::StateMerge(client_state) => {
                // The reconnect handshake's upward half: the client
                // pushes its full state so ops that died with its old
                // connection still land. If it taught us anything,
                // everyone gets a merge broadcast.
                let now_playing_changed = {
                    let mut state = lock(&shared.state);
                    let server_epoch = Epoch(shared.epoch.load(Ordering::SeqCst));
                    if client_state.epoch != server_epoch {
                        tracing::warn!(
                            "ignoring client merge with stale epoch {:?}",
                            client_state.epoch
                        );
                        continue;
                    }
                    let before = dessplay_core::resolve_value(&state.now_playing).flatten();
                    state.merge(client_state.state);
                    shared.observe(Some(state.max_lww_timestamp()));
                    before != dessplay_core::resolve_value(&state.now_playing).flatten()
                };
                shared.dirty.store(true, Ordering::SeqCst);
                // Always rebroadcast: cheap (reconnects are rare), and
                // any change-detection that ignores playback positions
                // (as view_hash must) would fail to propagate a
                // recovered position.
                let merge = ServerControl::StateMerge(shared.snapshot());
                for peer in shared.live_conns(None) {
                    send_control(&*peer, &merge).await;
                }
                if now_playing_changed {
                    shared
                        .server_write(|state, actor, ts| {
                            state.set_seek_authority(actor, ts, SeekAuthority::Server)
                        })
                        .await;
                }
            }
            other => {
                tracing::debug!("ignoring unexpected client message: {other:?}");
            }
        }
    }
}

/// The EOF -> next-file transition (docs/design.md, Playback Rules).
/// The first report matching now-playing from a present, watching,
/// interactive user wins; later duplicates no longer match now-playing
/// and fall through, making the transition idempotent.
async fn handle_eof<T: Transport>(
    shared: &Shared<T>,
    reporter: &UserId,
    role: Role,
    file: Ed2kHash,
) {
    if role != Role::Interactive {
        return; // seeders don't watch
    }
    let ops = {
        let mut state = lock(&shared.state);
        let view = state.view();
        if view.now_playing != Some(file) {
            return; // duplicate or stale report
        }
        match derive::user_state(&view, reporter) {
            DerivedUserState::NotWatching | DerivedUserState::Away { .. } => return,
            DerivedUserState::Ready | DerivedUserState::Paused => {}
        }
        let next = view
            .playlist
            .iter()
            .skip_while(|entry| entry.hash != file)
            .nth(1)
            .map(|entry| entry.hash);
        let epoch = Epoch(shared.epoch.load(Ordering::SeqCst));
        [
            state.set_watched(ActorId::SERVER, shared.stamp(), file, true),
            state.set_now_playing(ActorId::SERVER, shared.stamp(), next),
            // The next episode loads paused; anyone presses play.
            state.set_playback_intent(ActorId::SERVER, shared.stamp(), PlaybackIntent::Paused),
            state.set_seek_authority(ActorId::SERVER, shared.stamp(), SeekAuthority::Server),
        ]
        .map(|op| ServerControl::StateOp { epoch, op })
    };
    shared.dirty.store(true, Ordering::SeqCst);
    tracing::info!("EOF on {file:?} reported by {reporter:?}; advancing");
    for op in ops {
        shared.broadcast_op(op, None).await;
    }
}

/// Compact the state and broadcast the fresh snapshot to every live
/// connection.
async fn run_compaction<T: Transport>(shared: &Shared<T>, chat_keep: usize) {
    let snapshot = compact_state(shared, chat_keep);
    tracing::info!("compacted state; new epoch {:?}", snapshot.epoch);
    let msg = ServerControl::StateSnapshot(snapshot);
    for conn in shared.live_conns(None) {
        send_control(&*conn, &msg).await;
    }
    shared.flush();
}

/// Rebuild the state from its resolved view under the server actor and
/// bump the epoch (docs/sync-state.md, Compaction). The rebuild itself
/// is the pure, property-tested [`dessplay_core::compact::rebuild`];
/// this wrapper supplies the server's Lamport stamps, swaps the state
/// in atomically with the epoch bump, and archives the chat.
fn compact_state<T: Transport>(shared: &Shared<T>, chat_keep: usize) -> StateSnapshot {
    let view = {
        let mut guard = lock(&shared.state);
        let view = guard.view();
        let fresh =
            dessplay_core::compact::rebuild(&view, ActorId::SERVER, chat_keep, || shared.stamp());
        // Replace state and bump the epoch while holding the lock, so
        // concurrent snapshot() calls never see a torn pair.
        *guard = fresh;
        shared.epoch.fetch_add(1, Ordering::SeqCst);
        view
    };

    // Archive the full pre-compaction chat (idempotent: the table is
    // unique on (timestamp, sender, text)).
    if let Some(storage) = &mut *lock(&shared.storage)
        && let Err(e) = storage.archive_chat(&view.chat)
    {
        tracing::error!("chat archive failed (continuing with trim): {e}");
    }
    shared.dirty.store(true, Ordering::SeqCst);
    shared.snapshot()
}
