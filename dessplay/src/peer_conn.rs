//! Peer-to-peer connection management.
//!
//! Connects to discovered peers from the rendezvous server's PeerList,
//! performs the Hello handshake, and produces NetworkEvents.
//! Accepts incoming peer connections via `spawn_accept_loop`.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, RwLock};

use dessplay_core::framing::{
    decode_datagram, encode_datagram, read_framed, write_framed, TAG_PEER_CONTROL,
    TAG_PEER_DATAGRAM,
};
use dessplay_core::network::{MessageStream, NetworkEvent};
use dessplay_core::protocol::{PeerControl, PeerDatagram, PeerInfo};
use dessplay_core::types::PeerId;

/// A connected peer with its QUIC connection.
struct PeerState {
    username: String,
    connection: quinn::Connection,
    /// Send half of the control stream.
    control_tx: tokio::sync::mpsc::UnboundedSender<PeerControl>,
}

/// Manages peer-to-peer connections discovered via the rendezvous server.
pub struct PeerManager {
    endpoint: quinn::Endpoint,
    peer_client_config: quinn::ClientConfig,
    our_peer_id: PeerId,
    our_username: String,
    /// Currently connected peers.
    peers: Arc<RwLock<HashMap<PeerId, PeerState>>>,
    /// Known peers from the latest PeerList (for verifying incoming connections).
    known_peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    event_tx: tokio::sync::mpsc::UnboundedSender<NetworkEvent>,
    event_rx: Mutex<tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>>,
}

impl PeerManager {
    pub fn new(
        endpoint: quinn::Endpoint,
        peer_client_config: quinn::ClientConfig,
        our_peer_id: PeerId,
        our_username: String,
    ) -> Self {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            endpoint,
            peer_client_config,
            our_peer_id,
            our_username,
            peers: Arc::new(RwLock::new(HashMap::new())),
            known_peers: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            event_rx: Mutex::new(event_rx),
        }
    }

    /// Spawn the accept loop for incoming peer connections.
    ///
    /// Must be called after constructing the PeerManager. Runs until the
    /// endpoint is closed.
    pub fn spawn_accept_loop(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let incoming = match this.endpoint.accept().await {
                    Some(incoming) => incoming,
                    None => {
                        tracing::debug!("Accept loop: endpoint closed");
                        break;
                    }
                };
                let this2 = Arc::clone(&this);
                tokio::spawn(async move {
                    if let Err(e) = this2.handle_incoming(incoming).await {
                        tracing::debug!("Incoming peer connection failed: {e:#}");
                    }
                });
            }
        });
    }

    /// Handle a single incoming QUIC connection.
    async fn handle_incoming(&self, incoming: quinn::Incoming) -> Result<()> {
        let conn = incoming.await.context("incoming QUIC handshake failed")?;

        // Accept the first bidirectional stream (control stream)
        let (send, recv) = conn
            .accept_bi()
            .await
            .context("failed to accept peer control stream")?;
        let mut send = send;
        let mut recv = recv;

        // Read their Hello
        let hello: PeerControl = read_framed(&mut recv, TAG_PEER_CONTROL)
            .await?
            .ok_or_else(|| anyhow::anyhow!("peer closed before Hello"))?;

        let (remote_peer_id, remote_username) = match hello {
            PeerControl::Hello { peer_id, username } => (peer_id, username),
            other => {
                return Err(anyhow::anyhow!("expected Hello, got {other:?}"));
            }
        };

        tracing::info!(%remote_peer_id, %remote_username, "Incoming peer Hello received");

        // Verify this peer is in our known_peers list
        {
            let known = self.known_peers.read().await;
            if !known.contains_key(&remote_peer_id) {
                return Err(anyhow::anyhow!(
                    "incoming peer {remote_peer_id} not in known peer list"
                ));
            }
        }

        // Check not already connected
        {
            let peers = self.peers.read().await;
            if peers.contains_key(&remote_peer_id) {
                tracing::debug!(%remote_peer_id, "Already connected, dropping incoming");
                return Ok(());
            }
        }

        // Send our Hello back
        write_framed(
            &mut send,
            TAG_PEER_CONTROL,
            &PeerControl::Hello {
                peer_id: self.our_peer_id,
                username: self.our_username.clone(),
            },
        )
        .await?;

        tracing::info!(%remote_peer_id, %remote_username, "Incoming peer Hello exchange complete");

        register_peer(
            remote_peer_id,
            remote_username,
            conn,
            send,
            recv,
            &self.peers,
            &self.event_tx,
        )
        .await;

        Ok(())
    }

    /// Update peer connections based on a new PeerList from the server.
    ///
    /// Uses the "higher PeerId initiates" convention: only initiates outbound
    /// connections to peers with a lower PeerId than ours.
    pub async fn update_peer_list(&self, peers: Vec<PeerInfo>) {
        // Update known_peers first
        {
            let mut known = self.known_peers.write().await;
            known.clear();
            for p in &peers {
                if p.peer_id != self.our_peer_id {
                    known.insert(p.peer_id, p.clone());
                }
            }
        }

        let current_ids: Vec<PeerId> = {
            let peers_map = self.peers.read().await;
            peers_map.keys().copied().collect()
        };

        // Filter out ourselves
        let remote_peers: Vec<&PeerInfo> = peers
            .iter()
            .filter(|p| p.peer_id != self.our_peer_id)
            .collect();

        let new_peer_ids: Vec<PeerId> = remote_peers.iter().map(|p| p.peer_id).collect();

        // Disconnect peers no longer in the list
        for &old_id in &current_ids {
            if !new_peer_ids.contains(&old_id) {
                self.disconnect_peer(old_id).await;
            }
        }

        // Connect to new peers — only if our peer_id > theirs ("higher initiates")
        for peer_info in remote_peers {
            if !current_ids.contains(&peer_info.peer_id)
                && self.our_peer_id > peer_info.peer_id
            {
                let peer_info = peer_info.clone();
                let peer_client_config = self.peer_client_config.clone();
                let endpoint = self.endpoint.clone();
                let our_peer_id = self.our_peer_id;
                let our_username = self.our_username.clone();
                let peers = Arc::clone(&self.peers);
                let event_tx = self.event_tx.clone();

                tokio::spawn(async move {
                    if let Err(e) = connect_to_peer(
                        &endpoint,
                        &peer_client_config,
                        our_peer_id,
                        &peer_info,
                        &our_username,
                        &peers,
                        &event_tx,
                    )
                    .await
                    {
                        tracing::warn!(
                            peer_id = %peer_info.peer_id,
                            peer_user = %peer_info.username,
                            "Failed to connect to peer: {e:#}"
                        );
                    }
                });
            }
        }
    }

    /// Send a control message to a specific peer.
    pub async fn send_control(&self, peer: PeerId, msg: &PeerControl) -> Result<()> {
        let peers = self.peers.read().await;
        let peer_state = peers
            .get(&peer)
            .ok_or_else(|| anyhow::anyhow!("peer {peer:?} not connected"))?;
        peer_state
            .control_tx
            .send(msg.clone())
            .map_err(|_| anyhow::anyhow!("peer {peer:?} channel closed"))?;
        Ok(())
    }

    /// Send a datagram to a specific peer.
    pub async fn send_datagram(&self, peer: PeerId, msg: &PeerDatagram) -> Result<()> {
        let peers = self.peers.read().await;
        let peer_state = peers
            .get(&peer)
            .ok_or_else(|| anyhow::anyhow!("peer {peer:?} not connected"))?;
        let data = encode_datagram(TAG_PEER_DATAGRAM, msg)?;
        peer_state
            .connection
            .send_datagram(data.into())
            .context("failed to send datagram")?;
        Ok(())
    }

    /// Open a bidirectional stream to a specific peer.
    pub async fn open_stream(&self, peer: PeerId) -> Result<MessageStream> {
        let peers = self.peers.read().await;
        let peer_state = peers
            .get(&peer)
            .ok_or_else(|| anyhow::anyhow!("peer {peer:?} not connected"))?;
        let (send, recv) = peer_state
            .connection
            .open_bi()
            .await
            .context("failed to open stream")?;
        Ok(MessageStream {
            send: Box::new(send),
            recv: Box::new(recv),
        })
    }

    /// Receive the next network event.
    pub async fn recv(&self) -> Result<NetworkEvent> {
        self.event_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("peer manager shut down"))
    }

    /// Get list of currently connected peer IDs.
    pub async fn connected_peers(&self) -> Vec<PeerId> {
        self.peers.read().await.keys().copied().collect()
    }

    async fn disconnect_peer(&self, peer_id: PeerId) {
        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.remove(&peer_id) {
            peer.connection.close(0u32.into(), b"peer removed");
            let _ = self
                .event_tx
                .send(NetworkEvent::PeerDisconnected { peer_id });
            tracing::info!(%peer_id, username = %peer.username, "Peer disconnected");
        }
    }
}

/// Register a fully-handshaken peer: insert into the peers map, emit
/// `PeerConnected`, and spawn control writer/reader + datagram reader tasks.
async fn register_peer(
    peer_id: PeerId,
    username: String,
    conn: quinn::Connection,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    peers: &Arc<RwLock<HashMap<PeerId, PeerState>>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<NetworkEvent>,
) {
    let (control_tx, mut control_rx) = tokio::sync::mpsc::unbounded_channel();

    // Register the peer
    {
        let mut peers_map = peers.write().await;
        peers_map.insert(
            peer_id,
            PeerState {
                username: username.clone(),
                connection: conn.clone(),
                control_tx,
            },
        );
    }

    // Emit PeerConnected event
    let _ = event_tx.send(NetworkEvent::PeerConnected {
        peer_id,
        username: username.clone(),
    });

    // Spawn control stream writer
    let mut send = send;
    tokio::spawn(async move {
        while let Some(msg) = control_rx.recv().await {
            if let Err(e) = write_framed(&mut send, TAG_PEER_CONTROL, &msg).await {
                tracing::debug!(%peer_id, "Control writer error: {e}");
                break;
            }
        }
    });

    // Spawn control stream reader
    let mut recv = recv;
    let event_tx_reader = event_tx.clone();
    tokio::spawn(async move {
        loop {
            match read_framed::<_, PeerControl>(&mut recv, TAG_PEER_CONTROL).await {
                Ok(Some(msg)) => {
                    let _ = event_tx_reader.send(NetworkEvent::PeerControl {
                        from: peer_id,
                        message: msg,
                    });
                }
                Ok(None) => {
                    tracing::debug!(%peer_id, "Peer control stream closed");
                    let _ = event_tx_reader.send(NetworkEvent::PeerDisconnected { peer_id });
                    break;
                }
                Err(e) => {
                    tracing::debug!(%peer_id, "Peer control read error: {e}");
                    let _ = event_tx_reader.send(NetworkEvent::PeerDisconnected { peer_id });
                    break;
                }
            }
        }
    });

    // Spawn datagram reader
    let event_tx_dgram = event_tx.clone();
    tokio::spawn(async move {
        loop {
            match conn.read_datagram().await {
                Ok(data) => {
                    match decode_datagram::<PeerDatagram>(&data, TAG_PEER_DATAGRAM) {
                        Ok(msg) => {
                            let _ = event_tx_dgram.send(NetworkEvent::PeerDatagram {
                                from: peer_id,
                                message: msg,
                            });
                        }
                        Err(e) => {
                            tracing::debug!(%peer_id, "Datagram decode error: {e}");
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(%peer_id, "Datagram read error: {e}");
                    break;
                }
            }
        }
    });
}

/// Connect to a peer, perform Hello handshake, and register.
async fn connect_to_peer(
    endpoint: &quinn::Endpoint,
    peer_client_config: &quinn::ClientConfig,
    our_peer_id: PeerId,
    peer_info: &PeerInfo,
    our_username: &str,
    peers: &Arc<RwLock<HashMap<PeerId, PeerState>>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<NetworkEvent>,
) -> Result<()> {
    let peer_id = peer_info.peer_id;
    let Some(addr) = peer_info.addresses.first() else {
        return Err(anyhow::anyhow!("no addresses for peer"));
    };

    tracing::info!(%peer_id, %addr, username = %peer_info.username, "Connecting to peer");

    // Connect via QUIC using the peer client config (accept any cert)
    let conn = endpoint
        .connect_with(peer_client_config.clone(), *addr, "dessplay")
        .context("failed to initiate peer connection")?
        .await
        .context("peer QUIC connection failed")?;

    // Open control stream and perform Hello exchange
    let (send, recv) = conn
        .open_bi()
        .await
        .context("failed to open peer control stream")?;
    let mut send = send;
    let mut recv = recv;

    // Send our Hello
    write_framed(
        &mut send,
        TAG_PEER_CONTROL,
        &PeerControl::Hello {
            peer_id: our_peer_id,
            username: our_username.to_string(),
        },
    )
    .await?;

    // Read their Hello
    let response: PeerControl = read_framed(&mut recv, TAG_PEER_CONTROL)
        .await?
        .ok_or_else(|| anyhow::anyhow!("peer closed before Hello"))?;

    let (remote_peer_id, remote_username) = match response {
        PeerControl::Hello { peer_id: rid, username } => (rid, username),
        other => {
            return Err(anyhow::anyhow!("expected Hello, got {other:?}"));
        }
    };

    // Verify the peer_id matches what we expected
    if remote_peer_id != peer_id {
        return Err(anyhow::anyhow!(
            "peer_id mismatch: expected {peer_id}, got {remote_peer_id}"
        ));
    }

    tracing::info!(%peer_id, %remote_username, "Peer Hello exchange complete");

    // Dedup guard: check if already connected (e.g. accept loop won the race)
    {
        let peers_map = peers.read().await;
        if peers_map.contains_key(&peer_id) {
            tracing::debug!(%peer_id, "Already connected via accept, dropping outbound");
            return Ok(());
        }
    }

    register_peer(peer_id, remote_username, conn, send, recv, peers, event_tx).await;

    Ok(())
}
