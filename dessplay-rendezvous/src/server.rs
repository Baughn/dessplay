//! Rendezvous server: accepts client connections, authenticates, distributes
//! peer lists, handles time sync, and relays traffic for peers that cannot
//! connect directly.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use quinn::Endpoint;
use tokio::sync::RwLock;

use dessplay_core::framing::{read_framed, write_framed, TAG_RV_CONTROL};
use dessplay_core::protocol::{PeerInfo, RvControl};
use dessplay_core::types::PeerId;

/// A connected and authenticated client.
struct ConnectedClient {
    peer_id: PeerId,
    username: String,
    observed_addr: SocketAddr,
    connected_since: u64,
    /// Send half of the control stream (for pushing messages to the client).
    control_tx: tokio::sync::mpsc::UnboundedSender<RvControl>,
}

/// The rendezvous server.
pub struct RendezvousServer {
    endpoint: Endpoint,
    password: String,
    clients: Arc<RwLock<HashMap<PeerId, ConnectedClient>>>,
    next_peer_id: AtomicU64,
}

impl RendezvousServer {
    /// Create a new server with the given endpoint and authentication password.
    pub fn new(endpoint: Endpoint, password: String) -> Arc<Self> {
        Arc::new(Self {
            endpoint,
            password,
            clients: Arc::new(RwLock::new(HashMap::new())),
            next_peer_id: AtomicU64::new(1),
        })
    }

    /// Run the accept loop. Returns only on fatal error.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        tracing::info!("Rendezvous server accepting connections");
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
                    control_tx: msg_tx,
                },
            );
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
                other => {
                    tracing::debug!(%peer_id, "Unhandled message: {other:?}");
                }
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
