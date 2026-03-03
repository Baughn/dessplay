//! Rendezvous server: accepts client connections, authenticates, distributes
//! peer lists, handles time sync, and participates in state sync.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use quinn::Endpoint;
use tokio::sync::RwLock;

use dessplay_core::framing::{read_framed, write_framed, TAG_RV_CONTROL};
use dessplay_core::protocol::{PeerInfo, RvControl};
use dessplay_core::sync_engine::{SyncAction, SyncEngine};
use dessplay_core::types::PeerId;

use crate::storage::ServerStorage;

/// A connected and authenticated client.
pub(crate) struct ConnectedClient {
    pub(crate) peer_id: PeerId,
    pub(crate) username: String,
    pub(crate) observed_addr: SocketAddr,
    pub(crate) connected_since: u64,
    /// Send half of the control stream (for pushing messages to the client).
    pub(crate) control_tx: tokio::sync::mpsc::UnboundedSender<RvControl>,
}

/// The rendezvous server.
pub struct RendezvousServer {
    endpoint: Endpoint,
    password: String,
    clients: Arc<RwLock<HashMap<PeerId, ConnectedClient>>>,
    next_peer_id: AtomicU64,
    sync_engine: Arc<tokio::sync::Mutex<SyncEngine>>,
    storage: Arc<std::sync::Mutex<ServerStorage>>,
    compaction_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    anidb_user: Option<String>,
    anidb_password: Option<String>,
}

impl RendezvousServer {
    /// Create a new server with the given endpoint, password, storage, and
    /// optional AniDB credentials.
    pub fn new(
        endpoint: Endpoint,
        password: String,
        storage: ServerStorage,
        anidb_user: Option<String>,
        anidb_password: Option<String>,
    ) -> Arc<Self> {
        // Load persisted CRDT state from storage
        let sync_engine = match storage.load_latest_snapshot() {
            Ok(Some((epoch, snapshot))) => {
                let mut state = dessplay_core::crdt::CrdtState::new();
                state.load_snapshot(epoch, snapshot);
                if let Ok(ops) = storage.load_ops(epoch) {
                    for op in ops {
                        state.apply_op(&op);
                    }
                }
                tracing::info!(%epoch, "Loaded persisted CRDT state");
                SyncEngine::from_persisted(epoch, state, epoch)
            }
            Ok(None) => {
                tracing::info!("No persisted state, starting fresh");
                SyncEngine::new()
            }
            Err(e) => {
                tracing::warn!("Failed to load persisted state: {e}");
                tracing::warn!("Clearing corrupt snapshots/ops from database");
                if let Err(e2) = storage.clear_all_crdt_state() {
                    tracing::warn!("Failed to clear corrupt state: {e2}");
                }
                SyncEngine::new()
            }
        };

        Arc::new(Self {
            endpoint,
            password,
            clients: Arc::new(RwLock::new(HashMap::new())),
            next_peer_id: AtomicU64::new(1),
            sync_engine: Arc::new(tokio::sync::Mutex::new(sync_engine)),
            storage: Arc::new(std::sync::Mutex::new(storage)),
            compaction_task: Arc::new(tokio::sync::Mutex::new(None)),
            anidb_user,
            anidb_password,
        })
    }

    /// Run the accept loop. Returns only on fatal error.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        tracing::info!("Rendezvous server accepting connections");

        // Spawn AniDB worker if credentials are configured
        if let (Some(user), Some(pass)) = (self.anidb_user.clone(), self.anidb_password.clone()) {
            tracing::info!("AniDB credentials configured, spawning worker");
            let sync_engine = Arc::clone(&self.sync_engine);
            let storage = Arc::clone(&self.storage);
            let clients = Arc::clone(&self.clients);
            tokio::spawn(async move {
                crate::anidb::worker::run(sync_engine, storage, clients, user, pass).await;
            });
        } else {
            tracing::warn!("AniDB credentials not configured; metadata lookups disabled");
        }

        // Spawn periodic state summary broadcast (every 1s)
        let server_for_tick = Arc::clone(&self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let actions = server_for_tick.sync_engine.lock().await.on_periodic_tick();
                server_for_tick.dispatch_sync_actions(actions, None).await;
            }
        });

        while let Some(incoming) = self.endpoint.accept().await {
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                match incoming.await {
                    Ok(conn) => {
                        let remote = conn.remote_address();
                        tracing::info!(%remote, "New QUIC connection");
                        if let Err(e) = server.handle_connection(conn).await {
                            tracing::warn!(%remote, "Connection error: {e:#}");
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Incoming connection failed: {e}");
                    }
                }
            });
        }
        Ok(())
    }

    /// Handle a single client connection lifecycle.
    async fn handle_connection(self: &Arc<Self>, conn: quinn::Connection) -> Result<()> {
        let remote = conn.remote_address();

        // Accept the control stream (client opens first bidi stream)
        let (send, recv) = conn
            .accept_bi()
            .await
            .context("failed to accept control stream")?;
        let mut send = send;
        let mut recv = recv;

        // First message must be Auth
        let auth_msg: RvControl = read_framed(&mut recv, TAG_RV_CONTROL)
            .await
            .context("failed to read auth message")?
            .ok_or_else(|| anyhow::anyhow!("connection closed before auth"))?;

        let (password, username) = match auth_msg {
            RvControl::Auth { password, username } => (password, username),
            other => {
                tracing::warn!(%remote, "Expected Auth, got {other:?}");
                return Ok(());
            }
        };

        if password != self.password {
            tracing::info!(%remote, "Auth failed: wrong password");
            write_framed(&mut send, TAG_RV_CONTROL, &RvControl::AuthFailed).await?;
            // Give the client time to read the AuthFailed message before we
            // tear down the connection. Wait for client disconnect or 5s.
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                recv.read(&mut [0u8; 1]),
            )
            .await;
            return Ok(());
        }

        // Assign peer ID
        let peer_id = PeerId(self.next_peer_id.fetch_add(1, Ordering::Relaxed));

        let now = now_millis();

        // Send AuthOk
        write_framed(
            &mut send,
            TAG_RV_CONTROL,
            &RvControl::AuthOk {
                peer_id,
                observed_addr: remote,
            },
        )
        .await?;

        tracing::info!(%remote, %peer_id, %username, "Client authenticated");

        // Create channel for sending messages to this client
        let (msg_tx, mut msg_rx) = tokio::sync::mpsc::unbounded_channel();

        // Register client
        {
            let mut clients = self.clients.write().await;
            clients.insert(
                peer_id,
                ConnectedClient {
                    peer_id,
                    username: username.clone(),
                    observed_addr: remote,
                    connected_since: now,
                    control_tx: msg_tx.clone(),
                },
            );
        }

        // Cancel any pending compaction since a client just connected
        self.cancel_compaction().await;

        // Send our state to the new client
        {
            let eng = self.sync_engine.lock().await;
            // Always send our state summary
            let _ = msg_tx.send(RvControl::StateSummary {
                versions: eng.version_vectors(),
            });
            // If we have any state, send a full snapshot so the client can catch up
            let snapshot = eng.state().snapshot();
            let epoch = eng.epoch();
            if epoch > 0 || !snapshot.user_states.is_empty() || !snapshot.chat.is_empty() {
                let _ = msg_tx.send(RvControl::StateSnapshot {
                    epoch,
                    crdts: snapshot,
                });
            }
        }

        // Broadcast updated peer list to all clients
        self.broadcast_peer_list().await;

        // Spawn writer task (sends queued messages to this client)
        let writer_handle = tokio::spawn(async move {
            while let Some(msg) = msg_rx.recv().await {
                if let Err(e) = write_framed(&mut send, TAG_RV_CONTROL, &msg).await {
                    tracing::debug!("Writer error for {peer_id:?}: {e}");
                    break;
                }
            }
        });

        // Read loop: handle messages from this client
        let result = self.client_read_loop(peer_id, &mut recv).await;

        // Cleanup: remove client and notify others
        writer_handle.abort();
        {
            let mut clients = self.clients.write().await;
            clients.remove(&peer_id);
        }
        tracing::info!(%peer_id, %username, "Client disconnected");
        self.broadcast_peer_list().await;

        // Schedule compaction if no clients remain
        {
            let clients = self.clients.read().await;
            if clients.is_empty() {
                drop(clients);
                self.schedule_compaction().await;
            }
        }

        result
    }

    /// Read and handle messages from an authenticated client.
    async fn client_read_loop(
        &self,
        peer_id: PeerId,
        recv: &mut quinn::RecvStream,
    ) -> Result<()> {
        loop {
            let msg: Option<RvControl> = read_framed(recv, TAG_RV_CONTROL).await?;
            let Some(msg) = msg else {
                // Clean close
                return Ok(());
            };

            match msg {
                RvControl::TimeSyncRequest { client_send } => {
                    let server_recv = now_millis();
                    let server_send = now_millis();
                    let response = RvControl::TimeSyncResponse {
                        client_send,
                        server_recv,
                        server_send,
                    };
                    self.send_to_client(peer_id, response).await;
                }
                RvControl::StateOp { op } => {
                    let actions = self.sync_engine.lock().await.on_remote_op(peer_id, op);
                    self.dispatch_sync_actions(actions, Some(peer_id)).await;
                }
                RvControl::StateSummary { versions } => {
                    let actions = self
                        .sync_engine
                        .lock()
                        .await
                        .on_state_summary(peer_id, versions.epoch, versions);
                    self.dispatch_sync_actions(actions, Some(peer_id)).await;
                }
                RvControl::StateSnapshot { epoch, crdts } => {
                    let actions = self
                        .sync_engine
                        .lock()
                        .await
                        .on_state_snapshot(epoch, crdts);
                    self.dispatch_sync_actions(actions, Some(peer_id)).await;
                }
                RvControl::AniDbLookup { file_id, file_size } => {
                    tracing::info!(%peer_id, %file_id, file_size, "AniDB lookup request");
                    if let Ok(s) = self.storage.lock()
                        && let Err(e) = s.enqueue_anidb_lookup(&file_id, file_size, now_millis())
                    {
                        tracing::warn!("Failed to enqueue AniDB lookup: {e}");
                    }
                }
                other => {
                    tracing::debug!(%peer_id, "Unhandled message: {other:?}");
                }
            }
        }
    }

    /// Dispatch sync actions to clients and storage.
    ///
    /// `from_client` is the peer that triggered the actions (if any). Used to
    /// exclude the sender when broadcasting.
    async fn dispatch_sync_actions(
        &self,
        actions: Vec<SyncAction>,
        from_client: Option<PeerId>,
    ) {
        for action in actions {
            match action {
                SyncAction::SendControl { peer, msg } => {
                    // Convert PeerControl to RvControl
                    let rv_msg = match msg {
                        dessplay_core::protocol::PeerControl::StateOp { op } => {
                            RvControl::StateOp { op }
                        }
                        dessplay_core::protocol::PeerControl::StateSummary { versions, .. } => {
                            RvControl::StateSummary { versions }
                        }
                        dessplay_core::protocol::PeerControl::StateSnapshot { epoch, crdts } => {
                            RvControl::StateSnapshot { epoch, crdts }
                        }
                        _ => continue,
                    };
                    self.send_to_client(peer, rv_msg).await;
                }
                SyncAction::BroadcastControl { msg } => {
                    let rv_msg = match msg {
                        dessplay_core::protocol::PeerControl::StateOp { op } => {
                            RvControl::StateOp { op }
                        }
                        dessplay_core::protocol::PeerControl::StateSummary { versions, .. } => {
                            RvControl::StateSummary { versions }
                        }
                        dessplay_core::protocol::PeerControl::StateSnapshot { epoch, crdts } => {
                            RvControl::StateSnapshot { epoch, crdts }
                        }
                        _ => continue,
                    };
                    // Broadcast to all clients except the sender
                    let clients = self.clients.read().await;
                    for client in clients.values() {
                        if Some(client.peer_id) != from_client {
                            let _ = client.control_tx.send(rv_msg.clone());
                        }
                    }
                }
                SyncAction::PersistOp { op } => {
                    let epoch = self.sync_engine.lock().await.epoch();
                    if let Ok(s) = self.storage.lock()
                        && let Err(e) = s.append_op(epoch, &op)
                    {
                        tracing::warn!("Failed to persist op: {e}");
                    }
                }
                SyncAction::PersistSnapshot { epoch, snapshot } => {
                    if let Ok(s) = self.storage.lock()
                        && let Err(e) = s.save_snapshot(epoch, &snapshot)
                    {
                        tracing::warn!("Failed to persist snapshot: {e}");
                    }
                }
                // Server doesn't send datagrams or do gap fill via streams
                SyncAction::SendDatagram { .. }
                | SyncAction::BroadcastDatagram { .. }
                | SyncAction::RequestGapFill { .. } => {}
            }
        }
    }

    /// Send a message to a specific client.
    async fn send_to_client(&self, peer_id: PeerId, msg: RvControl) {
        let clients = self.clients.read().await;
        if let Some(client) = clients.get(&peer_id) {
            let _ = client.control_tx.send(msg);
        }
    }

    /// Schedule compaction to run after 5 minutes with no connected clients.
    pub(crate) async fn schedule_compaction(self: &Arc<Self>) {
        let mut task = self.compaction_task.lock().await;
        if task.is_some() {
            return; // already scheduled
        }
        let server = Arc::clone(self);
        *task = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5 * 60)).await;
            server.run_compaction().await;
        }));
        tracing::debug!("Compaction scheduled in 5 minutes");
    }

    /// Cancel a pending compaction (e.g. when a client connects).
    pub(crate) async fn cancel_compaction(&self) {
        let mut task = self.compaction_task.lock().await;
        if let Some(handle) = task.take() {
            handle.abort();
            tracing::debug!("Compaction cancelled: client connected");
        }
    }

    /// Run compaction: snapshot the CRDT state, increment epoch, and persist.
    async fn run_compaction(&self) {
        tracing::info!("Compacting CRDT state (no clients for 5 minutes)");
        let (new_epoch, snapshot) = {
            let mut eng = self.sync_engine.lock().await;
            eng.compact()
        };
        if let Ok(s) = self.storage.lock() {
            if let Err(e) = s.save_snapshot(new_epoch, &snapshot) {
                tracing::warn!("Failed to persist compaction snapshot: {e}");
                return;
            }
            if let Err(e) = s.delete_before_epoch(new_epoch) {
                tracing::warn!("Failed to delete pre-compaction data: {e}");
            }
        }
        tracing::info!(%new_epoch, "Compaction complete");
    }

    /// Build and broadcast the current peer list to all clients.
    async fn broadcast_peer_list(&self) {
        let clients = self.clients.read().await;
        let peers: Vec<PeerInfo> = clients
            .values()
            .map(|c| PeerInfo {
                peer_id: c.peer_id,
                username: c.username.clone(),
                addresses: vec![c.observed_addr],
                connected_since: c.connected_since,
            })
            .collect();
        let msg = RvControl::PeerList { peers };
        for client in clients.values() {
            let _ = client.control_tx.send(msg.clone());
        }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use dessplay_core::protocol::CrdtOp;

    /// Create a test server (real QUIC endpoint on loopback, in-memory storage).
    fn create_test_server() -> Arc<RendezvousServer> {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

        let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
        let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der());

        let mut server_crypto = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der.into())
            .unwrap();
        server_crypto.alpn_protocols = vec![b"dessplay".to_vec()];

        let server_config = quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto).unwrap(),
        ));

        let bind: SocketAddr = "[::1]:0".parse().unwrap();
        let endpoint = Endpoint::server(server_config, bind).unwrap();

        let storage = crate::storage::ServerStorage::open_in_memory().unwrap();
        RendezvousServer::new(endpoint, "test".to_string(), storage, None, None)
    }

    #[tokio::test(start_paused = true)]
    async fn compaction_fires_after_timeout() {
        let server = create_test_server();

        // Initial epoch is 0
        assert_eq!(server.sync_engine.lock().await.epoch(), 0);

        // Apply an op so compaction has something to snapshot
        let op = CrdtOp::LwwWrite {
            timestamp: 100,
            value: dessplay_core::protocol::LwwValue::UserState(
                dessplay_core::types::UserId("alice".into()),
                dessplay_core::types::UserState::Ready,
            ),
        };
        let _ = server.sync_engine.lock().await.on_remote_op(PeerId(99), op);

        // Schedule compaction
        server.schedule_compaction().await;

        // Advance past 5 minutes
        tokio::time::sleep(Duration::from_secs(5 * 60 + 1)).await;
        // Yield to let the spawned task run
        tokio::task::yield_now().await;

        // Epoch should be incremented
        assert_eq!(server.sync_engine.lock().await.epoch(), 1);

        // Compaction task should be consumed (ran to completion)
        // The handle stays in the mutex but the task is done
    }

    #[tokio::test(start_paused = true)]
    async fn compaction_cancelled_on_connect() {
        let server = create_test_server();

        // Apply an op
        let op = CrdtOp::LwwWrite {
            timestamp: 200,
            value: dessplay_core::protocol::LwwValue::UserState(
                dessplay_core::types::UserId("bob".into()),
                dessplay_core::types::UserState::Paused,
            ),
        };
        let _ = server.sync_engine.lock().await.on_remote_op(PeerId(99), op);

        // Schedule compaction
        server.schedule_compaction().await;

        // Advance 2 minutes (less than 5 min)
        tokio::time::sleep(Duration::from_secs(2 * 60)).await;

        // Cancel (simulates a client connecting)
        server.cancel_compaction().await;

        // Advance past the original 5 min mark
        tokio::time::sleep(Duration::from_secs(4 * 60)).await;
        tokio::task::yield_now().await;

        // Epoch should NOT have changed
        assert_eq!(server.sync_engine.lock().await.epoch(), 0);
    }
}
