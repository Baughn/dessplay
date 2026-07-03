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
use dessplay_core::net::framing::{read_frame, write_frame};
use dessplay_core::net::{
    AniDbSearchHit, BiStream, KnownUser, Listener, PROTOCOL_VERSION, PeerInfo, Presence,
    RelayEnvelope, Role, ServerControl, Transport, TransportEvent, WireMessage,
};
use dessplay_core::state::StateView;
use dessplay_core::types::{
    ActorId, AniDbMetadata, AniDbSeriesId, Ed2kHash, Epoch, FileCatalogEntry, NextEpState,
    PlaybackIntent, SeekAuthority, SeriesRelations, SharedTimestamp, UserId,
};
use dessplay_core::wire;
use dessplay_core::{CrdtOp, CrdtState, StateSnapshot};

use crate::anidb::client::AniDbApi;
use crate::anidb::titles::TitlesSource;
use crate::anidb::worker::{self, AniDbHost};
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

/// AniDB integration: the (rate-limited) API client and the titles-dump
/// source. Both are trait objects so tests inject canned data — no test
/// may ever touch the real API.
#[derive(Clone)]
pub struct AniDbConfig {
    /// FILE/ANIME lookups.
    pub api: Arc<dyn AniDbApi>,
    /// The anime-titles dump (name search).
    pub titles: Arc<dyn TitlesSource>,
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
    /// AniDB integration; `None` disables it (no credentials).
    pub anidb: Option<AniDbConfig>,
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
            anidb: None,
        }
    }
}

/// The write half of a peer's relay stream, shared so any forwarder can
/// deliver to it. A `tokio::sync::Mutex` because writes are `.await`ed
/// (a frame can span several poll-writes) and held across them.
type RelayTx = Arc<tokio::sync::Mutex<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>>;

/// Which channel a relayed op leaves the server on.
#[derive(Clone, Copy, Debug)]
enum RelayTransport {
    /// Reliable control stream plus an eager datagram copy when it fits.
    /// The default: ordinary ops, server-authored writes, and the 1s
    /// reliable playback-position tick.
    Eager,
    /// Datagram only, best-effort — no reliable copy.
    DatagramOnly,
}

/// The relay transport for `op`, given how it *arrived*.
///
/// A `PlaybackPosition` that came in on the 100ms datagram fast path is
/// relayed datagram-only: re-fanning every stale position out reliably to
/// all peers is exactly the head-of-line blocking the datagram-only
/// position transport exists to avoid (docs/network-design.md, "Exception
/// -- playback position"; docs/sync-state.md, Playback Position). The rule
/// mirrors the inbound transport — a position that arrived reliably (the
/// 1s catch-up tick, sent eager by the client) and every other op type
/// relay [`RelayTransport::Eager`].
fn relay_transport(op: &CrdtOp, via_datagram: bool) -> RelayTransport {
    if via_datagram && matches!(op, CrdtOp::PlaybackPosition(_)) {
        RelayTransport::DatagramOnly
    } else {
        RelayTransport::Eager
    }
}

struct PeerEntry<T> {
    /// Id of the connection backing this entry (live or last).
    conn_id: u64,
    /// The live connection; `None` once Lost or Departed.
    conn: Option<Arc<T>>,
    /// Write half of the peer's relay stream (file transfer), if it has
    /// opened one. Cleared with `conn` on disconnect.
    relay_tx: Option<RelayTx>,
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

    /// Broadcast one message to every live connection, on the channel
    /// [`RelayTransport`] selects.
    ///
    /// [`RelayTransport::Eager`] (ordinary ops and server-authored
    /// writes): reliable control stream, plus an eager datagram copy when
    /// it fits. [`RelayTransport::DatagramOnly`]: datagram only,
    /// best-effort — used to mirror the 100ms position fast path so stale
    /// positions never queue on the control stream (see
    /// [`relay_transport`]).
    async fn broadcast_op(&self, msg: ServerControl, skip: Option<u64>, transport: RelayTransport) {
        let name = msg.variant_name();
        let Ok(frame) = wire::encode(&WireMessage::Control(msg)) else {
            return;
        };
        let conns = self.live_conns(skip);
        tracing::trace!(
            msg = name,
            bytes = frame.len(),
            recipients = conns.len(),
            ?transport,
            "broadcast"
        );
        for conn in conns {
            let fits_datagram = conn
                .max_datagram_size()
                .is_some_and(|max| frame.len() <= max);
            if matches!(transport, RelayTransport::Eager) {
                let _ = conn.send_control(&frame).await;
            }
            // Eager: an extra datagram copy (pure latency win, receivers
            // dedup). DatagramOnly: the *only* copy — and if it won't fit
            // a datagram we drop it rather than fall back to the control
            // stream (the next position, or the 1s reliable tick,
            // supersedes it), since that fallback is the very head-of-line
            // blocking this path exists to avoid.
            if fits_datagram {
                let _ = conn.send_datagram(&frame).await;
            }
        }
    }

    /// Forward a peer message to `to`, wrapped as `Forwarded { from }`.
    /// Dropped silently if the target has no live relay stream (absent
    /// or not yet opened) — the design's "drop envelopes addressed to
    /// non-Present peers". Returns whether it was delivered.
    async fn forward(&self, from: &UserId, to: &UserId, message: Vec<u8>) -> bool {
        let relay_tx = {
            let registry = lock(&self.registry);
            registry.peers.get(to).and_then(|p| p.relay_tx.clone())
        };
        let Some(relay_tx) = relay_tx else {
            tracing::trace!(%from, %to, "relay: target has no stream; dropping");
            return false;
        };
        let envelope = RelayEnvelope::Forwarded {
            from: from.clone(),
            message,
        };
        let Ok(frame) = wire::encode(&envelope) else {
            return false;
        };
        let mut guard = relay_tx.lock().await;
        match write_frame(&mut *guard, &frame).await {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!(%to, "relay write failed: {e}");
                false
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
        self.broadcast_op(
            ServerControl::StateOp { epoch, op },
            None,
            RelayTransport::Eager,
        )
        .await;
    }

    /// Force the playback-intent latch to Paused (lost connection,
    /// graceful quit, departure, EOF). Idempotent in effect.
    async fn force_pause(&self) {
        tracing::debug!("forcing playback intent to Paused");
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
            tracing::debug!(user = %user.0, "rescuing seek authority from a gone user");
            self.server_write(|state, actor, ts| {
                state.set_seek_authority(actor, ts, SeekAuthority::Server)
            })
            .await;
        }
    }
}

/// The AniDB worker's view of the server: the shared clock, the
/// resolved state, server-authored writes, and the queue storage.
struct SharedHost<T>(Arc<Shared<T>>);

impl<T: Transport> AniDbHost for SharedHost<T> {
    fn now(&self) -> u64 {
        (self.0.clock)()
    }

    fn view(&self) -> StateView {
        lock(&self.0.state).view()
    }

    async fn write_metadata(&self, hash: Ed2kHash, metadata: AniDbMetadata) {
        self.0
            .server_write(move |state, actor, ts| {
                state.set_anidb_metadata(actor, ts, hash, Some(metadata))
            })
            .await;
    }

    async fn write_relations(&self, series: AniDbSeriesId, relations: SeriesRelations) {
        self.0
            .server_write(move |state, actor, ts| {
                state.set_series_relations(actor, ts, series, relations)
            })
            .await;
    }

    async fn write_catalog(&self, hash: Ed2kHash, entry: FileCatalogEntry) {
        self.0
            .server_write(move |state, actor, ts| state.set_file_catalog(actor, ts, hash, entry))
            .await;
    }

    fn with_storage<R>(&self, f: impl FnOnce(&mut ServerStorage) -> R) -> Option<R> {
        lock(&self.0.storage).as_mut().map(f)
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
/// How long a known user stays visible in `known_offline` after their last
/// connect/disconnect, per docs/design.md #15 ("hidden after 30 days").
const KNOWN_USER_RETENTION_MILLIS: u64 = 30 * 24 * 60 * 60 * 1000;

/// Resolve the snapshot the server starts from.
///
/// - `None` storage, or `Ok(None)` (no stored row) — a genuine first run:
///   fresh epoch-1 state.
/// - `Ok(Some(snapshot))` — the stored snapshot, used as-is.
/// - `Err(e)` — a load failure (a corrupt/truncated blob, a layout the
///   forward-compat [`CrdtState::decode_snapshot`] cannot read, or a
///   SQLite read error). This is **fatal**: the server is authoritative
///   and cannot re-sync its lost state from anyone, so it refuses to
///   start rather than silently discard the playlist/List/metadata/...
///   and reset the epoch to 1. An operator can then investigate or
///   restore the database.
fn initial_snapshot(storage: Option<&ServerStorage>) -> Result<StateSnapshot, String> {
    let fresh = || StateSnapshot {
        epoch: Epoch(1),
        state: CrdtState::new(),
    };
    match storage.map(ServerStorage::load_state) {
        // No storage, or no stored row: a genuine first run.
        None | Some(Ok(None)) => Ok(fresh()),
        // A stored snapshot: use it.
        Some(Ok(Some(snapshot))) => Ok(snapshot),
        // A load failure must never be collapsed into "first run": doing
        // so silently discards authoritative state and resets the epoch.
        Some(Err(e)) => Err(format!(
            "refusing to start: cannot load the authoritative state snapshot from \
             the server database ({e}); the snapshot may be corrupt or from an \
             incompatible version — investigate or restore the database rather \
             than losing server state (playlist, the List, watched flags, AniDB \
             metadata, file catalog, relations, chat) and resetting the epoch"
        )),
    }
}

/// Run the accept loop until the listener fails. Each connection gets
/// its own task. State is loaded from `storage` if present; a fresh
/// server starts at epoch 1 (clients persist real epochs, so epoch 0
/// always reads as stale and gets a snapshot).
///
/// Returns `Err` (refusing to start) if the stored snapshot exists but
/// cannot be loaded — see [`initial_snapshot`]. The server is
/// authoritative and cannot re-sync its lost state from anyone, so a
/// load failure must never be silently treated as a fresh start.
pub async fn run<L: Listener>(
    listener: L,
    config: ServerConfig,
    clock: Clock,
    storage: Option<ServerStorage>,
) -> Result<(), String>
where
    L::Conn: Transport,
{
    let config = Arc::new(config);
    let initial = initial_snapshot(storage.as_ref())?;
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
    // The AniDB worker (needs storage for its queues).
    if let Some(anidb) = config.anidb.clone() {
        if lock(&shared.storage).is_some() {
            let host = SharedHost(Arc::clone(&shared));
            tokio::spawn(worker::run(host, anidb.api, anidb.titles));
        } else {
            tracing::warn!("AniDB configured but the server has no storage; disabled");
        }
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
                return Ok(());
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

/// Send a terminal rejection, then wait (bounded) for the client to act
/// on it before closing: closing immediately can discard the unflushed
/// frame, leaving the client with a generic connection loss it would
/// retry forever (the Goodbye pattern, server-side).
async fn reject_and_close<T: Transport>(conn: &T, msg: &ServerControl, reason: &str) {
    send_control(conn, msg).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match conn.recv().await {
                Ok(TransportEvent::Closed { .. }) | Err(_) => break,
                Ok(_) => continue,
            }
        }
    })
    .await;
    conn.close(reason).await;
}

/// Every known user (design.md #15) not currently **Present**, within the
/// retention window — the persisted counterpart to the in-memory registry,
/// so a user who hasn't connected this session (possibly not since a
/// server restart) can still be named and acted on.
///
/// Only `Presence::Present` peers are excluded here, not Lost/Departed
/// ones: those still hold a `registry.peers` entry (it is never removed,
/// only its presence flips), but they are *not* "fully represented"
/// on-screen the way a Present peer is -- a Departed peer's only client-side
/// display today is the plain dim line this field replaces. The finer
/// dedup (Lost peers already get a row; a committed-absent Departed peer
/// gets a blocker row) happens client-side against `rows`, which is built
/// from the full peer list and knows about that gating state; this
/// function only needs to avoid listing someone who is plainly, currently
/// here.
fn known_offline<T>(shared: &Shared<T>, peers: &[PeerInfo]) -> Vec<KnownUser> {
    let Some(storage) = &*lock(&shared.storage) else {
        return Vec::new();
    };
    let cutoff = (shared.clock)().saturating_sub(KNOWN_USER_RETENTION_MILLIS) as i64;
    let known = match storage.known_users(cutoff) {
        Ok(known) => known,
        Err(e) => {
            tracing::error!("loading known_users: {e}");
            return Vec::new();
        }
    };
    known
        .into_iter()
        .filter(|(username, _)| {
            !peers
                .iter()
                .any(|p| &p.username == username && p.presence == Presence::Present)
        })
        .map(|(username, last_seen)| KnownUser {
            username,
            last_seen: last_seen as u64,
        })
        .collect()
}

/// Record `username`'s last-seen timestamp (design.md #15), on connect and
/// disconnect alike. Best-effort: a storage error only logs, since presence
/// tracking must not depend on it.
fn record_seen<T>(shared: &Shared<T>, username: &UserId, at: u64) {
    if let Some(storage) = &*lock(&shared.storage)
        && let Err(e) = storage.record_seen(username, at as i64)
    {
        tracing::error!(user = %username.0, "recording known_users last_seen: {e}");
    }
}

/// Push the current peer list (all presence stages) to every live
/// connection.
async fn broadcast_peer_list<T: Transport>(shared: &Shared<T>) {
    let peers = lock(&shared.registry).peer_infos();
    let known_offline = known_offline(shared, &peers);
    let msg = ServerControl::PeerList {
        peers,
        known_offline,
    };
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
            // No force_pause here. A timeout-ladder departure was already
            // force-paused at its Lost transition 30s earlier; the peer is
            // only promoted here from Presence::Lost. Re-pausing would
            // clobber a legitimate resume that the *present* users made
            // during the Lost window -- an absent Maybe/NotWatching/Away
            // peer is non-blocking, so pressing play then is valid -- with
            // a strictly-later Lamport stamp that wins the LWW. Departed
            // only changes gating. (The graceful-quit path force-pauses on
            // its own immediate-departure arm, which never enters Lost.)
            // We still reclaim seek authority from a departed holder.
            shared.take_authority_from(&user).await;
        }
    }
}

/// How an authenticated connection ended.
enum AuthedEnd {
    /// The client said Goodbye: depart immediately (Present -> Departed),
    /// skipping the Lost stage but staying listed.
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
    enum AuthOutcome {
        Auth {
            username: UserId,
            password: String,
            role: Role,
            epoch: Epoch,
            protocol_version: u32,
        },
        /// A control frame that does not decode as `Auth` — the
        /// signature of a pre-versioning client, whose `Auth` is a
        /// strict prefix of the current shape (or of plain garbage).
        Undecodable,
        /// Closed, or a decodable-but-wrong first message.
        Dead,
    }
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
                            protocol_version,
                        })) => {
                            return AuthOutcome::Auth {
                                username,
                                password,
                                role,
                                epoch,
                                protocol_version,
                            };
                        }
                        Ok(_) => return AuthOutcome::Dead, // protocol violation
                        Err(_) => return AuthOutcome::Undecodable,
                    }
                }
                Ok(TransportEvent::Closed { .. }) | Err(_) => return AuthOutcome::Dead,
                Ok(_) => continue, // ignore datagrams/streams pre-auth
            }
        }
    })
    .await;

    let (username, password, role, client_epoch) = match auth {
        Err(_) | Ok(AuthOutcome::Dead) => {
            tracing::debug!(%remote, "connection closed before authenticating");
            conn.close("authentication required").await;
            return;
        }
        Ok(AuthOutcome::Undecodable) => {
            // An old binary cannot decode ProtocolMismatch; AuthFailed
            // has kept its discriminant since v0, so it at least fails
            // fast with a readable (if generic) refusal.
            tracing::warn!(%remote, "undecodable Auth: pre-versioning client (or garbage)");
            reject_and_close(
                &*conn,
                &ServerControl::AuthFailed,
                "protocol version too old",
            )
            .await;
            return;
        }
        Ok(AuthOutcome::Auth {
            username,
            password,
            role,
            epoch,
            protocol_version,
        }) => {
            // Version before password: a mismatched client should hear
            // "update" even when its stored password is also stale.
            if protocol_version != PROTOCOL_VERSION {
                tracing::warn!(
                    user = %username.0,
                    %remote,
                    client_version = protocol_version,
                    server_version = PROTOCOL_VERSION,
                    "auth refused: protocol version mismatch"
                );
                reject_and_close(
                    &*conn,
                    &ServerControl::ProtocolMismatch {
                        server_version: PROTOCOL_VERSION,
                    },
                    "protocol version mismatch",
                )
                .await;
                return;
            }
            (username, password, role, epoch)
        }
    };
    if password != config.password {
        tracing::warn!(user = %username.0, %remote, "auth failed: bad password");
        reject_and_close(&*conn, &ServerControl::AuthFailed, "bad password").await;
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
                relay_tx: None,
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
        tracing::debug!(user = %username.0, "superseding the user's old connection");
        old.close("superseded by a new connection").await;
    }
    record_seen(&shared, &username, clock());

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
    let server_epoch = snapshot.epoch;
    let initial = if client_epoch == snapshot.epoch {
        ServerControl::StateMerge(snapshot)
    } else {
        ServerControl::StateSnapshot(snapshot)
    };
    tracing::debug!(
        user = %username.0,
        client_epoch = client_epoch.0,
        server_epoch = server_epoch.0,
        decision = initial.variant_name(),
        "initial sync"
    );
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
                    // A clean quit is an *immediate departure*, not a
                    // registry removal: the peer stays listed as Departed
                    // (the dim "left" line), and a committed (Watching)
                    // quitter keeps gating until acknowledged — design.md
                    // (User States) waits for a committed user even when
                    // absent, "Lost, Departed, or quit". Skipping the Lost
                    // stage is the only thing a Goodbye buys over a timeout.
                    AuthedEnd::Goodbye => {
                        entry.conn = None;
                        entry.relay_tx = None;
                        entry.info.presence = Presence::Departed;
                        entry.lost_at = Some(clock());
                    }
                    AuthedEnd::Lost(_) => {
                        entry.conn = None;
                        entry.relay_tx = None;
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

    record_seen(&shared, &username, clock());
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

/// Read a peer's relay stream, forwarding each `Forward` envelope to its
/// target. Exits when the stream closes (connection gone). Lives as a
/// task for the connection's lifetime; clients only send `Forward`, so
/// a stray `Forwarded` is ignored.
async fn relay_reader<T: Transport>(
    mut recv: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    from: UserId,
    shared: Arc<Shared<T>>,
) {
    loop {
        let frame = match read_frame(&mut recv).await {
            Ok(frame) => frame,
            Err(_) => break,
        };
        match wire::decode::<RelayEnvelope>(&frame) {
            // The stream registered on accept_bi before this reader even
            // started; Hello exists only to trigger that, so ignore it.
            Ok(RelayEnvelope::Hello) => {}
            Ok(RelayEnvelope::Forward { to, message }) => {
                shared.forward(&from, &to, message).await;
            }
            Ok(RelayEnvelope::Forwarded { .. }) => {
                tracing::warn!(%from, "client sent a Forwarded envelope; ignoring");
            }
            Err(e) => tracing::warn!(%from, "undecodable relay envelope: {e}"),
        }
    }
    tracing::trace!(%from, "relay reader exiting");
}

/// The post-auth message loop.
async fn serve_authed<T: Transport>(
    conn: &T,
    conn_id: u64,
    username: &UserId,
    role: Role,
    shared: &Arc<Shared<T>>,
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
            TransportEvent::IncomingStream(stream) => {
                // The peer's relay stream (file transfer). Register the
                // write half so other peers can forward to it, and read
                // its Forward envelopes in a task.
                let BiStream { send, recv } = stream;
                let relay_tx: RelayTx = Arc::new(tokio::sync::Mutex::new(send));
                {
                    let mut registry = lock(&shared.registry);
                    if let Some(entry) = registry.peers.get_mut(username)
                        && entry.conn_id == conn_id
                    {
                        entry.relay_tx = Some(Arc::clone(&relay_tx));
                    }
                }
                tracing::debug!(user = %username.0, "relay stream opened");
                tokio::spawn(relay_reader(recv, username.clone(), Arc::clone(shared)));
                continue;
            }
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
        tracing::trace!(
            user = %username.0,
            msg = msg.variant_name(),
            bytes = payload.len(),
            via_datagram,
            "recv"
        );
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
                    } else {
                        // Apply and decide whether to re-fan-out. Every
                        // eager op arrives twice (reliable + datagram); only
                        // the copy that actually advances state is
                        // rebroadcast, so we don't broadcast each op twice.
                        // The datagram path also drops out-of-sequence map
                        // ops (its reliable copy fills the gap).
                        state.apply_for_broadcast(op.clone(), via_datagram)
                    }
                };
                if !applied {
                    continue;
                }
                shared.dirty.store(true, Ordering::SeqCst);
                // `applied` already means "this copy changed now-playing":
                // both transports route through change-detecting applies, so
                // whichever copy effected the change (and only that one)
                // reaches here, taking seek authority exactly once.
                let now_playing_changed = matches!(op, CrdtOp::NowPlaying(_));
                // Mirror the inbound transport: a 100ms datagram position
                // is relayed datagram-only, never re-fanned-out reliably.
                let transport = relay_transport(&op, via_datagram);
                shared
                    .broadcast_op(
                        ServerControl::StateOp { epoch, op },
                        Some(conn_id),
                        transport,
                    )
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
            ServerControl::MarkWatched { file, watched } => {
                handle_mark_watched(shared, file, watched).await;
            }
            ServerControl::AniDbSearch { query } => {
                // A LIKE scan over the whole titles table (~1M rows,
                // tens of ms). Searches are manual and rare; fine to
                // answer inline.
                let results = {
                    let storage = lock(&shared.storage);
                    storage.as_ref().map_or_else(Vec::new, |storage| {
                        storage
                            .search_titles(&query, 20)
                            .map_err(|e| tracing::error!("title search failed: {e}"))
                            .unwrap_or_default()
                            .into_iter()
                            .map(|hit| AniDbSearchHit {
                                series: hit.series,
                                title: hit.title,
                                matched: hit.matched,
                            })
                            .collect()
                    })
                };
                tracing::debug!(user = %username.0, %query, hits = results.len(), "anidb search");
                send_control(conn, &ServerControl::AniDbSearchResults { query, results }).await;
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
                tracing::debug!(user = %username.0, "client merge applied; rebroadcasting");
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
            // Only a present *watching* reporter advances the group —
            // committed (Ready) or opportunistic (Maybe), per
            // docs/design.md, Playback Rules / EOF. A manually-Paused
            // reporter is not actively watching, and NotWatching/Away never
            // gate, so none of them advance now-playing.
            DerivedUserState::Ready | DerivedUserState::Maybe => {}
            DerivedUserState::Paused
            | DerivedUserState::NotWatching
            | DerivedUserState::Away { .. } => return,
        }
        let next = view
            .playlist
            .iter()
            .skip_while(|entry| entry.hash != file)
            .nth(1)
            .map(|entry| entry.hash);
        let epoch = Epoch(shared.epoch.load(Ordering::SeqCst));
        let mut ops = vec![
            state.set_watched(ActorId::SERVER, shared.stamp(), file, true),
            state.set_now_playing(ActorId::SERVER, shared.stamp(), next),
            // The next episode loads paused; anyone presses play.
            state.set_playback_intent(ActorId::SERVER, shared.stamp(), PlaybackIntent::Paused),
            state.set_seek_authority(ActorId::SERVER, shared.stamp(), SeekAuthority::Server),
        ];
        // The List: auto-advance next_ep for linked entries whose
        // numeric next_ep matches the episode that just finished
        // (docs/design.md, The List). `available` resets — the new
        // next episode is presumably not out yet.
        for (id, new_state) in list_advances(&view, file) {
            tracing::info!(next_ep = ?new_state.next_ep, "advancing List entry");
            ops.push(state.set_next_ep(ActorId::SERVER, shared.stamp(), id, new_state));
        }
        ops.into_iter()
            .map(|op| ServerControl::StateOp { epoch, op })
            .collect::<Vec<_>>()
    };
    shared.dirty.store(true, Ordering::SeqCst);
    tracing::info!("EOF on {file:?} reported by {reporter:?}; advancing");
    for op in ops {
        shared.broadcast_op(op, None, RelayTransport::Eager).await;
    }
}

/// Manual mark-watched from the episode browser (docs/design.md #10):
/// unlike `handle_eof` this is not scoped to now-playing and touches no
/// playback register -- just the watched flag, plus (when setting `true`)
/// the same List `next_ep` auto-advance the EOF path gets. A request that
/// would not change the flag is a no-op, making repeats idempotent.
async fn handle_mark_watched<T: Transport>(shared: &Shared<T>, file: Ed2kHash, watched: bool) {
    let ops = {
        let mut state = lock(&shared.state);
        let view = state.view();
        if view.watched.get(&file) == Some(&watched) {
            return; // already at the requested value
        }
        let epoch = Epoch(shared.epoch.load(Ordering::SeqCst));
        let mut ops = vec![state.set_watched(ActorId::SERVER, shared.stamp(), file, watched)];
        if watched {
            for (id, new_state) in list_advances(&view, file) {
                tracing::info!(next_ep = ?new_state.next_ep, "advancing List entry");
                ops.push(state.set_next_ep(ActorId::SERVER, shared.stamp(), id, new_state));
            }
        }
        ops.into_iter()
            .map(|op| ServerControl::StateOp { epoch, op })
            .collect::<Vec<_>>()
    };
    shared.dirty.store(true, Ordering::SeqCst);
    tracing::info!(watched, "manual mark-watched on {file:?}");
    for op in ops {
        shared.broadcast_op(op, None, RelayTransport::Eager).await;
    }
}

/// List entries to auto-advance when `file` finishes: linked to the
/// file's series, with a numeric `next_ep` equal to the file's numeric
/// episode number. Returns the new progress states.
fn list_advances(
    view: &StateView,
    file: Ed2kHash,
) -> Vec<(dessplay_core::types::ListEntryId, NextEpState)> {
    let Some(Some(metadata)) = view.anidb_metadata.get(&file) else {
        return Vec::new();
    };
    let (Some(series), Some(episode)) = (
        metadata.series_id,
        metadata
            .episode_number
            .as_deref()
            .and_then(|ep| ep.trim().parse::<u32>().ok()),
    ) else {
        return Vec::new(); // unlinked file or special episode ("S1")
    };
    view.list_entries
        .iter()
        .filter(|(_, entry)| entry.anidb_series_id == Some(series))
        .filter_map(|(id, _)| {
            let progress = view.list_next_ep.get(id)?;
            let next: u32 = progress.next_ep.as_deref()?.trim().parse().ok()?;
            (next == episode).then(|| {
                (
                    *id,
                    NextEpState {
                        next_ep: Some((episode + 1).to_string()),
                        available: false,
                    },
                )
            })
        })
        .collect()
}

/// Compact the state and broadcast the fresh snapshot to every live
/// connection.
async fn run_compaction<T: Transport>(shared: &Shared<T>, chat_keep: usize) {
    let started = std::time::Instant::now();
    let snapshot = compact_state(shared, chat_keep);
    tracing::info!(
        epoch = snapshot.epoch.0,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "compacted state"
    );
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
    tracing::debug!(
        playlist_entries = view.playlist.len(),
        chat_before = view.chat.len(),
        chat_after = view.chat.len().min(chat_keep),
        "compaction rebuilt the state"
    );

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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use dessplay_core::test_support::{ClusterEvent, ScriptOp, run_cluster};

    use super::*;

    /// A small snapshot with one playlist entry, for the load tests.
    fn sample_snapshot(epoch: u64) -> StateSnapshot {
        let cluster = run_cluster(&[ClusterEvent::ServerOp {
            ts: 1,
            op: ScriptOp::AddPlaylist {
                file: 1,
                after: None,
            },
        }]);
        StateSnapshot {
            epoch: Epoch(epoch),
            state: cluster.server,
        }
    }

    /// No storage at all (e.g. an ephemeral server) is a genuine first
    /// run: fresh epoch-1 state.
    #[test]
    fn initial_snapshot_without_storage_is_fresh_epoch_1() {
        let snapshot = initial_snapshot(None).expect("no storage must be a fresh start");
        assert_eq!(snapshot.epoch, Epoch(1));
        assert_eq!(snapshot.state, CrdtState::new());
    }

    /// Storage with no stored row is also a genuine first run.
    #[test]
    fn initial_snapshot_with_empty_storage_is_fresh_epoch_1() {
        let storage = ServerStorage::open_in_memory().unwrap();
        let snapshot = initial_snapshot(Some(&storage)).expect("empty storage must be fresh");
        assert_eq!(snapshot.epoch, Epoch(1));
        assert_eq!(snapshot.state, CrdtState::new());
    }

    /// A valid stored blob loads as-is (epoch and state preserved).
    #[test]
    fn initial_snapshot_loads_a_valid_stored_blob() {
        let storage = ServerStorage::open_in_memory().unwrap();
        let snapshot = sample_snapshot(9);
        storage.save_state(&snapshot, 1000).unwrap();
        let loaded = initial_snapshot(Some(&storage)).expect("valid blob must load");
        assert_eq!(loaded, snapshot);
    }

    /// Regression (2026-06-27): a corrupt/undecodable stored blob must
    /// **abort startup**, not be silently treated as "first run" — which
    /// would discard the authoritative playlist/List/metadata/... and
    /// reset the epoch to 1. The server cannot re-sync this from anyone.
    #[test]
    fn initial_snapshot_aborts_on_a_corrupt_blob() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rendezvous.db");

        // Persist a real snapshot, then overwrite its blob with bytes that
        // decode as neither the current layout nor the CrdtStateV1 prefix.
        {
            let storage = ServerStorage::open(&path).unwrap();
            storage.save_state(&sample_snapshot(5), 1000).unwrap();
        }
        {
            let raw = rusqlite::Connection::open(&path).unwrap();
            raw.execute(
                "UPDATE crdt_state SET state = ?1 WHERE room = 'default'",
                rusqlite::params![vec![0xFF_u8; 64]],
            )
            .unwrap();
        }

        let storage = ServerStorage::open(&path).unwrap();
        // Sanity: the storage layer itself reports the decode failure.
        assert!(
            storage.load_state().is_err(),
            "load_state must surface the corrupt blob as an error"
        );

        // The startup resolver must propagate it, not swallow it.
        assert!(
            initial_snapshot(Some(&storage)).is_err(),
            "a corrupt snapshot must abort startup, not silently reset to epoch 1"
        );
    }
}
