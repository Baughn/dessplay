//! Client-side rendezvous server connection: auth, time sync, peer list.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use tokio::sync::RwLock;

use dessplay_core::framing::{read_framed, write_framed, TAG_RV_CONTROL};
use dessplay_core::protocol::{CrdtOp, CrdtSnapshot, PeerInfo, RvControl, VersionVectors};
use dessplay_core::time_sync::TimeSyncState;
use dessplay_core::types::{PeerId, SharedTimestamp};

/// Client connection to the rendezvous server.
pub struct RendezvousClient {
    /// Our server-assigned peer ID.
    pub peer_id: PeerId,
    /// Our observed public address (as seen by the server).
    pub observed_addr: SocketAddr,
    /// Time synchronization state.
    time_sync: Arc<RwLock<TimeSyncState>>,
    /// Sender for outgoing messages to the server.
    msg_tx: tokio::sync::mpsc::UnboundedSender<RvControl>,
    /// Receiver for events (peer lists, etc.).
    event_rx: tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<RendezvousEvent>>,
    /// QUIC connection handle (for stats).
    connection: quinn::Connection,
}

/// Events from the rendezvous server.
#[derive(Debug, Clone)]
pub enum RendezvousEvent {
    /// Updated peer list from the server.
    PeerList { peers: Vec<PeerInfo> },
    /// A CRDT operation from the server.
    StateOp { op: CrdtOp },
    /// A state summary from the server.
    StateSummary { versions: VersionVectors },
    /// A full state snapshot from the server (epoch upgrade).
    StateSnapshot { epoch: u64, crdts: CrdtSnapshot },
}

impl RendezvousClient {
    /// Connect to the rendezvous server, authenticate, and run initial time sync.
    pub async fn connect(
        endpoint: &quinn::Endpoint,
        server_addr: SocketAddr,
        server_name: &str,
        password: &str,
        username: &str,
    ) -> Result<Self> {
        tracing::info!(%server_addr, "Connecting to rendezvous server");

        let conn = endpoint
            .connect(server_addr, server_name)
            .context("failed to initiate QUIC connection")?
            .await
            .context("QUIC connection failed")?;

        tracing::info!("Connected to server, authenticating");

        // Open control stream
        let (send, recv) = conn
            .open_bi()
            .await
            .context("failed to open control stream")?;
        let mut send = send;
        let mut recv = recv;

        // Send Auth
        write_framed(
            &mut send,
            TAG_RV_CONTROL,
            &RvControl::Auth {
                password: password.to_string(),
                username: username.to_string(),
            },
        )
        .await
        .context("failed to send Auth")?;

        // Read AuthOk or AuthFailed
        let response: RvControl = read_framed(&mut recv, TAG_RV_CONTROL)
            .await
            .context("failed to read auth response")?
            .ok_or_else(|| anyhow::anyhow!("server closed connection during auth"))?;

        let (peer_id, observed_addr) = match response {
            RvControl::AuthOk {
                peer_id,
                observed_addr,
            } => {
                tracing::info!(%peer_id, %observed_addr, "Authenticated");
                (peer_id, observed_addr)
            }
            RvControl::AuthFailed => {
                return Err(anyhow::anyhow!("authentication failed: wrong password"));
            }
            other => {
                return Err(anyhow::anyhow!(
                    "unexpected response to Auth: {other:?}"
                ));
            }
        };

        let time_sync = Arc::new(RwLock::new(TimeSyncState::new()));
        let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn writer task
        let msg_tx_clone = msg_tx.clone();
        tokio::spawn(async move {
            let mut msg_rx = msg_rx;
            while let Some(msg) = msg_rx.recv().await {
                if let Err(e) = write_framed(&mut send, TAG_RV_CONTROL, &msg).await {
                    tracing::debug!("Writer error: {e}");
                    break;
                }
            }
        });

        // Spawn reader task
        let time_sync_clone = Arc::clone(&time_sync);
        tokio::spawn(async move {
            loop {
                match read_framed::<_, RvControl>(&mut recv, TAG_RV_CONTROL).await {
                    Ok(Some(msg)) => {
                        Self::handle_server_message(
                            msg,
                            &time_sync_clone,
                            &event_tx,
                        )
                        .await;
                    }
                    Ok(None) => {
                        tracing::info!("Server closed connection");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!("Error reading from server: {e}");
                        break;
                    }
                }
            }
        });

        let client = Self {
            peer_id,
            observed_addr,
            time_sync,
            msg_tx: msg_tx_clone,
            event_rx: tokio::sync::Mutex::new(event_rx),
            connection: conn,
        };

        // Run initial time sync (5 rapid rounds)
        for _ in 0..5 {
            client.send_time_sync_request();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Spawn periodic time sync (every 30s)
        let msg_tx_periodic = client.msg_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;
                let client_send = now_millis();
                let _ = msg_tx_periodic.send(RvControl::TimeSyncRequest { client_send });
            }
        });

        tracing::info!("Rendezvous client ready");
        Ok(client)
    }

    /// Handle a message from the server.
    async fn handle_server_message(
        msg: RvControl,
        time_sync: &Arc<RwLock<TimeSyncState>>,
        event_tx: &tokio::sync::mpsc::UnboundedSender<RendezvousEvent>,
    ) {
        match msg {
            RvControl::TimeSyncResponse {
                client_send,
                server_recv,
                server_send,
            } => {
                let t4 = now_millis();
                let mut ts = time_sync.write().await;
                ts.process_response(client_send, server_recv, server_send, t4);
                tracing::debug!(
                    offset_ms = ts.offset_ms(),
                    samples = ts.sample_count(),
                    "Time sync updated"
                );
            }
            RvControl::PeerList { peers } => {
                tracing::info!(
                    count = peers.len(),
                    "Received peer list"
                );
                let _ = event_tx.send(RendezvousEvent::PeerList { peers });
            }
            RvControl::StateOp { op } => {
                tracing::debug!("State op from server");
                let _ = event_tx.send(RendezvousEvent::StateOp { op });
            }
            RvControl::StateSummary { versions } => {
                tracing::debug!("State summary from server");
                let _ = event_tx.send(RendezvousEvent::StateSummary { versions });
            }
            RvControl::StateSnapshot { epoch, crdts } => {
                tracing::info!(%epoch, "State snapshot from server");
                let _ = event_tx.send(RendezvousEvent::StateSnapshot { epoch, crdts });
            }
            other => {
                tracing::debug!("Unhandled server message: {other:?}");
            }
        }
    }

    /// Send a time sync request to the server.
    fn send_time_sync_request(&self) {
        let client_send = now_millis();
        let _ = self
            .msg_tx
            .send(RvControl::TimeSyncRequest { client_send });
    }

    /// Get the current shared timestamp (adjusted by server clock offset).
    pub async fn shared_now(&self) -> SharedTimestamp {
        self.time_sync.read().await.shared_now()
    }

    /// Receive the next event from the server.
    pub async fn recv(&self) -> Option<RendezvousEvent> {
        self.event_rx.lock().await.recv().await
    }

    /// Send a message to the server.
    pub fn send(&self, msg: RvControl) {
        let _ = self.msg_tx.send(msg);
    }

    /// Cumulative UDP bytes (tx, rx) on the server connection.
    pub fn udp_bytes(&self) -> (u64, u64) {
        let stats = self.connection.stats();
        (stats.udp_tx.bytes, stats.udp_rx.bytes)
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
