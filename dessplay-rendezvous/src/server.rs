//! The rendezvous server's connection handling.
//!
//! Phase 3 scope: accept, authenticate, answer time-sync probes, and
//! push peer lists on join/leave. State sync, presence staging
//! (Lost/Departed), compaction, and relay arrive in Phases 4-5.
//!
//! Generic over [`Listener`] so tests run it over the simulated
//! transport and production over QUIC.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dessplay_core::net::{
    Listener, PeerInfo, Presence, ServerControl, Transport, TransportEvent, WireMessage,
};
use dessplay_core::types::{Epoch, UserId};
use dessplay_core::wire;
use dessplay_core::{CrdtState, StateSnapshot};

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

/// Server configuration.
pub struct ServerConfig {
    /// The shared room password.
    pub password: String,
    /// How long a connection may sit unauthenticated before being cut.
    pub auth_timeout: Duration,
}

impl ServerConfig {
    /// Defaults with the given password.
    pub fn new(password: impl Into<String>) -> Self {
        Self {
            password: password.into(),
            auth_timeout: Duration::from_secs(10),
        }
    }
}

struct PeerEntry<T> {
    conn_id: u64,
    conn: Arc<T>,
    info: PeerInfo,
}

struct Registry<T> {
    peers: HashMap<UserId, PeerEntry<T>>,
}

/// State shared by all connection tasks.
struct Shared<T> {
    registry: Mutex<Registry<T>>,
    state: Mutex<CrdtState>,
    epoch: AtomicU64,
    dirty: AtomicBool,
    storage: Mutex<Option<ServerStorage>>,
    clock: Clock,
}

impl<T: Transport> Shared<T> {
    fn snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            epoch: Epoch(self.epoch.load(Ordering::SeqCst)),
            state: lock(&self.state).clone(),
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

    /// All present connections except `skip`.
    fn other_conns(&self, skip: u64) -> Vec<Arc<T>> {
        lock(&self.registry)
            .peers
            .values()
            .filter(|p| p.conn_id != skip)
            .map(|p| Arc::clone(&p.conn))
            .collect()
    }
}

impl<T: Transport> Registry<T> {
    fn peer_infos(&self) -> Vec<PeerInfo> {
        let mut infos: Vec<PeerInfo> = self.peers.values().map(|p| p.info.clone()).collect();
        infos.sort_by(|a, b| a.username.cmp(&b.username));
        infos
    }
}

/// Cadence of StateHash broadcasts and storage flushes.
const HASH_INTERVAL: Duration = Duration::from_secs(30);
const FLUSH_INTERVAL: Duration = Duration::from_secs(30);

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
    let shared = Arc::new(Shared::<L::Conn> {
        registry: Mutex::new(Registry {
            peers: HashMap::new(),
        }),
        state: Mutex::new(initial.state),
        epoch: AtomicU64::new(initial.epoch.0),
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
                let msg = ServerControl::StateHash {
                    epoch: Epoch(shared.epoch.load(Ordering::SeqCst)),
                    hash: lock(&shared.state).view_hash(),
                };
                for conn in shared.other_conns(u64::MAX) {
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

async fn send_control<T: Transport>(conn: &T, msg: &ServerControl) -> bool {
    let Ok(frame) = wire::encode(&WireMessage::Control(msg.clone())) else {
        return false;
    };
    conn.send_control(&frame).await.is_ok()
}

/// Push the current peer list to everyone.
async fn broadcast_peer_list<T: Transport>(registry: &Mutex<Registry<T>>) {
    let (peers, conns): (Vec<PeerInfo>, Vec<Arc<T>>) = {
        let registry = lock(registry);
        (
            registry.peer_infos(),
            registry
                .peers
                .values()
                .map(|p| Arc::clone(&p.conn))
                .collect(),
        )
    };
    let msg = ServerControl::PeerList { peers };
    for conn in conns {
        send_control(&*conn, &msg).await;
    }
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

    // ---- Register, superseding any existing connection for this user
    // (a reconnecting client whose old connection hasn't timed out yet).
    let superseded: Option<Arc<T>> = lock(&shared.registry)
        .peers
        .insert(
            username.clone(),
            PeerEntry {
                conn_id,
                conn: Arc::clone(&conn),
                info: PeerInfo {
                    username: username.clone(),
                    role,
                    presence: Presence::Present,
                    addresses: vec![remote],
                    connected_since: clock(),
                },
            },
        )
        .map(|old| old.conn);
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
    broadcast_peer_list(&shared.registry).await;

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
    let reason = serve_authed(&*conn, conn_id, &shared).await;
    tracing::info!("{username:?} disconnected: {reason}");

    // Remove ourselves — unless a newer connection superseded us.
    let removed = {
        let mut registry = lock(&shared.registry);
        match registry.peers.get(&username) {
            Some(entry) if entry.conn_id == conn_id => {
                registry.peers.remove(&username);
                true
            }
            _ => false,
        }
    };
    if removed {
        broadcast_peer_list(&shared.registry).await;
    }
}

/// The post-auth message loop. Returns the disconnect reason.
async fn serve_authed<T: Transport>(conn: &T, conn_id: u64, shared: &Shared<T>) -> String {
    let clock = &shared.clock;
    loop {
        let event = match conn.recv().await {
            Ok(event) => event,
            Err(e) => return e.to_string(),
        };
        let (payload, via_datagram) = match event {
            TransportEvent::Control(bytes) => (bytes, false),
            TransportEvent::Datagram(bytes) => (bytes, true),
            TransportEvent::IncomingStream(_) => continue, // Phase 4/9
            TransportEvent::Closed { reason } => return reason,
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
                    return "send failed".into();
                }
            }
            ServerControl::StateOp(op) => {
                let applied = {
                    let mut state = lock(&shared.state);
                    if via_datagram {
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
                // Broadcast to everyone else: reliable, plus an eager
                // datagram copy when it fits.
                let msg = ServerControl::StateOp(op);
                let Ok(frame) = wire::encode(&WireMessage::Control(msg)) else {
                    continue;
                };
                for peer in shared.other_conns(conn_id) {
                    let _ = peer.send_control(&frame).await;
                    if peer
                        .max_datagram_size()
                        .is_some_and(|max| frame.len() <= max)
                    {
                        let _ = peer.send_datagram(&frame).await;
                    }
                }
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
                let server_epoch = Epoch(shared.epoch.load(Ordering::SeqCst));
                if client_state.epoch != server_epoch {
                    tracing::warn!(
                        "ignoring client merge with stale epoch {:?}",
                        client_state.epoch
                    );
                    continue;
                }
                lock(&shared.state).merge(client_state.state);
                shared.dirty.store(true, Ordering::SeqCst);
                // Always rebroadcast: cheap (reconnects are rare), and
                // any change-detection that ignores playback positions
                // (as view_hash must) would fail to propagate a
                // recovered position.
                let merge = ServerControl::StateMerge(shared.snapshot());
                for peer in shared.other_conns(u64::MAX) {
                    send_control(&*peer, &merge).await;
                }
            }
            // Phase 5: EofReached.
            other => {
                tracing::debug!("ignoring (phase 4): {other:?}");
            }
        }
    }
}
