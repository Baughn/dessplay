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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dessplay_core::net::{
    Listener, PeerInfo, Presence, ServerControl, Transport, TransportEvent, WireMessage,
};
use dessplay_core::types::UserId;
use dessplay_core::wire;

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

impl<T: Transport> Registry<T> {
    fn peer_infos(&self) -> Vec<PeerInfo> {
        let mut infos: Vec<PeerInfo> = self.peers.values().map(|p| p.info.clone()).collect();
        infos.sort_by(|a, b| a.username.cmp(&b.username));
        infos
    }
}

/// Run the accept loop until the listener fails. Each connection gets
/// its own task.
pub async fn run<L: Listener>(listener: L, config: ServerConfig, clock: Clock)
where
    L::Conn: Transport,
{
    let config = Arc::new(config);
    let registry = Arc::new(Mutex::new(Registry::<L::Conn> {
        peers: HashMap::new(),
    }));
    let mut next_conn_id: u64 = 0;

    loop {
        let (conn, remote) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::info!("listener stopped: {e}");
                return;
            }
        };
        let conn_id = next_conn_id;
        next_conn_id += 1;
        let config = Arc::clone(&config);
        let registry = Arc::clone(&registry);
        let clock = Arc::clone(&clock);
        tokio::spawn(async move {
            serve_connection(Arc::new(conn), conn_id, remote, config, registry, clock).await;
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
    registry: Arc<Mutex<Registry<T>>>,
    clock: Clock,
) {
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

    let Ok(Some((username, password, role, _epoch))) = auth else {
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
    let superseded: Option<Arc<T>> = lock(&registry)
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
    broadcast_peer_list(&registry).await;
    tracing::info!("{username:?} connected from {remote} as {role:?}");

    // ---- Serve until the connection ends.
    let reason = serve_authed(&*conn, &clock).await;
    tracing::info!("{username:?} disconnected: {reason}");

    // Remove ourselves — unless a newer connection superseded us.
    let removed = {
        let mut registry = lock(&registry);
        match registry.peers.get(&username) {
            Some(entry) if entry.conn_id == conn_id => {
                registry.peers.remove(&username);
                true
            }
            _ => false,
        }
    };
    if removed {
        broadcast_peer_list(&registry).await;
    }
}

/// The post-auth message loop. Returns the disconnect reason.
async fn serve_authed<T: Transport>(conn: &T, clock: &Clock) -> String {
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
            // Phase 4: StateOp/StateMerge handling. Phase 5: EofReached.
            other => {
                tracing::debug!("ignoring (phase 3): {other:?}");
            }
        }
    }
}
