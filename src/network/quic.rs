use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use quinn::crypto::rustls::QuicClientConfig;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use super::{ConnectionError, ConnectionEvent, ConnectionManager, PeerId};

/// Handshake message exchanged after QUIC connection establishment.
#[derive(Serialize, Deserialize)]
struct Handshake {
    peer_id: String,
}


/// QUIC-based implementation of `ConnectionManager`.
///
/// Each peer generates a self-signed certificate. Certificate verification is
/// skipped (all peers are trusted). Authentication is handled at the rendezvous
/// layer (out of scope here).
pub struct QuicConnectionManager {
    endpoint: quinn::Endpoint,
    local_peer_id: PeerId,
    peers: Arc<RwLock<HashMap<PeerId, PeerState>>>,
    datagram_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<(PeerId, Vec<u8>)>>,
    reliable_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<(PeerId, Vec<u8>)>>,
    datagram_tx: mpsc::UnboundedSender<(PeerId, Vec<u8>)>,
    reliable_tx: mpsc::UnboundedSender<(PeerId, Vec<u8>)>,
    event_tx: broadcast::Sender<ConnectionEvent>,
    accept_task: Mutex<Option<JoinHandle<()>>>,
}

struct PeerState {
    connection: quinn::Connection,
    _tasks: Vec<JoinHandle<()>>,
}

impl QuicConnectionManager {
    /// Create a new QUIC connection manager bound to the given address.
    ///
    /// Generates a self-signed certificate. Starts accepting incoming connections
    /// immediately. Use `connect_to` to initiate outbound connections.
    pub async fn new(
        bind_addr: SocketAddr,
        local_peer_id: PeerId,
    ) -> Result<Self, ConnectionError> {
        let (server_config, client_config) = Self::make_tls_configs()?;

        let mut endpoint = quinn::Endpoint::server(server_config, bind_addr)
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;
        endpoint.set_default_client_config(client_config);

        let (datagram_tx, datagram_rx) = mpsc::unbounded_channel();
        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(256);
        let peers: Arc<RwLock<HashMap<PeerId, PeerState>>> = Arc::new(RwLock::new(HashMap::new()));

        let accept_task = tokio::spawn(accept_loop(
            endpoint.clone(),
            local_peer_id.clone(),
            Arc::clone(&peers),
            datagram_tx.clone(),
            reliable_tx.clone(),
            event_tx.clone(),
        ));

        Ok(Self {
            endpoint,
            local_peer_id,
            peers,
            datagram_rx: tokio::sync::Mutex::new(datagram_rx),
            reliable_rx: tokio::sync::Mutex::new(reliable_rx),
            datagram_tx,
            reliable_tx,
            event_tx,
            accept_task: Mutex::new(Some(accept_task)),
        })
    }

    /// The local address this endpoint is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.endpoint
            .local_addr()
            .expect("endpoint should have a local address")
    }

    /// Connect to a peer at the given address. Returns the peer's ID
    /// (received via handshake).
    pub async fn connect_to(&self, addr: SocketAddr) -> Result<PeerId, ConnectionError> {
        let connecting = self
            .endpoint
            .connect(addr, "dessplay")
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;

        let connection = connecting
            .await
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;

        // Open bidirectional stream for handshake
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;

        // Send our peer ID
        let handshake = Handshake {
            peer_id: self.local_peer_id.0.clone(),
        };
        let data = postcard::to_allocvec(&handshake)
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;
        let len = (data.len() as u32).to_be_bytes();
        send.write_all(&len)
            .await
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;
        send.write_all(&data)
            .await
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;
        send.finish()
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;

        // Read peer's response
        let peer_handshake = read_handshake(&mut recv).await?;
        let peer_id = PeerId(peer_handshake.peer_id);

        // Spawn per-connection tasks and register
        let tasks = spawn_connection_tasks(
            peer_id.clone(),
            connection.clone(),
            Arc::clone(&self.peers),
            self.datagram_tx.clone(),
            self.reliable_tx.clone(),
            self.event_tx.clone(),
        );

        {
            let mut peers = self.peers.write().unwrap();
            peers.insert(
                peer_id.clone(),
                PeerState {
                    connection,
                    _tasks: tasks,
                },
            );
        }

        let _ = self
            .event_tx
            .send(ConnectionEvent::PeerConnected(peer_id.clone()));

        tracing::info!(local = %self.local_peer_id, remote = %peer_id, "connected to peer");
        Ok(peer_id)
    }

    /// Close the endpoint and all connections.
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"shutdown");
        if let Some(task) = self.accept_task.lock().unwrap().take() {
            task.abort();
        }
    }

    fn make_tls_configs() -> Result<(quinn::ServerConfig, quinn::ClientConfig), ConnectionError> {
        let certified_key = rcgen::generate_simple_self_signed(vec!["dessplay".into()])
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;

        let cert_der = CertificateDer::from(certified_key.cert);
        let priv_key = PrivatePkcs8KeyDer::from(certified_key.key_pair.serialize_der());

        // Shared transport config
        let mut transport_config = quinn::TransportConfig::default();
        transport_config.keep_alive_interval(Some(std::time::Duration::from_secs(5)));
        transport_config.max_idle_timeout(Some(
            quinn::IdleTimeout::try_from(std::time::Duration::from_secs(30)).unwrap(),
        ));
        let transport_config = Arc::new(transport_config);

        // Server config with self-signed cert
        let mut server_config =
            quinn::ServerConfig::with_single_cert(vec![cert_der], priv_key.into())
                .map_err(|e| ConnectionError::Other(Box::new(e)))?;
        server_config.transport = Arc::clone(&transport_config);

        // Client config with skip-verification (all peers trusted)
        let crypto_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_no_client_auth();

        let mut client_config = quinn::ClientConfig::new(Arc::new(
            QuicClientConfig::try_from(crypto_config)
                .map_err(|e| ConnectionError::Other(Box::new(e)))?,
        ));
        client_config.transport_config(Arc::clone(&transport_config));

        Ok((server_config, client_config))
    }
}

impl Drop for QuicConnectionManager {
    fn drop(&mut self) {
        self.close();
    }
}

#[async_trait]
impl ConnectionManager for QuicConnectionManager {
    async fn send_datagram(&self, peer: &PeerId, data: &[u8]) -> Result<(), ConnectionError> {
        let connection = {
            let peers = self.peers.read().unwrap();
            peers
                .get(peer)
                .ok_or_else(|| ConnectionError::PeerNotConnected(peer.clone()))?
                .connection
                .clone()
        };
        connection
            .send_datagram(data.to_vec().into())
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;
        Ok(())
    }

    async fn recv_datagram(&self) -> Result<(PeerId, Vec<u8>), ConnectionError> {
        let mut rx = self.datagram_rx.lock().await;
        rx.recv().await.ok_or(ConnectionError::Closed)
    }

    async fn send_reliable(&self, peer: &PeerId, data: &[u8]) -> Result<(), ConnectionError> {
        let connection = {
            let peers = self.peers.read().unwrap();
            peers
                .get(peer)
                .ok_or_else(|| ConnectionError::PeerNotConnected(peer.clone()))?
                .connection
                .clone()
        };
        let mut send = connection
            .open_uni()
            .await
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;
        send.write_all(data)
            .await
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;
        send.finish()
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;
        Ok(())
    }

    async fn recv_reliable(&self) -> Result<(PeerId, Vec<u8>), ConnectionError> {
        let mut rx = self.reliable_rx.lock().await;
        rx.recv().await.ok_or(ConnectionError::Closed)
    }

    fn subscribe(&self) -> broadcast::Receiver<ConnectionEvent> {
        self.event_tx.subscribe()
    }

    fn connected_peers(&self) -> Vec<PeerId> {
        let peers = self.peers.read().unwrap();
        peers.keys().cloned().collect()
    }
}

// --- Background tasks ---

async fn accept_loop(
    endpoint: quinn::Endpoint,
    local_peer_id: PeerId,
    peers: Arc<RwLock<HashMap<PeerId, PeerState>>>,
    datagram_tx: mpsc::UnboundedSender<(PeerId, Vec<u8>)>,
    reliable_tx: mpsc::UnboundedSender<(PeerId, Vec<u8>)>,
    event_tx: broadcast::Sender<ConnectionEvent>,
) {
    while let Some(incoming) = endpoint.accept().await {
        let local_peer_id = local_peer_id.clone();
        let peers = Arc::clone(&peers);
        let datagram_tx = datagram_tx.clone();
        let reliable_tx = reliable_tx.clone();
        let event_tx = event_tx.clone();

        tokio::spawn(async move {
            let connection = match incoming.await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!("failed to accept connection: {e}");
                    return;
                }
            };

            // Accept handshake bidirectional stream from initiator
            let (mut send, mut recv) = match connection.accept_bi().await {
                Ok(streams) => streams,
                Err(e) => {
                    tracing::warn!("failed to accept handshake stream: {e}");
                    return;
                }
            };

            // Read initiator's handshake
            let peer_handshake = match read_handshake(&mut recv).await {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!("failed to read handshake: {e}");
                    return;
                }
            };
            let peer_id = PeerId(peer_handshake.peer_id);

            // Send our handshake response
            let response = Handshake {
                peer_id: local_peer_id.0.clone(),
            };
            let data = match postcard::to_allocvec(&response) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("failed to serialize handshake: {e}");
                    return;
                }
            };
            let len = (data.len() as u32).to_be_bytes();
            if let Err(e) = send.write_all(&len).await {
                tracing::warn!("failed to write handshake response: {e}");
                return;
            }
            if let Err(e) = send.write_all(&data).await {
                tracing::warn!("failed to write handshake response: {e}");
                return;
            }
            if let Err(e) = send.finish() {
                tracing::warn!("failed to finish handshake stream: {e}");
                return;
            }

            let tasks = spawn_connection_tasks(
                peer_id.clone(),
                connection.clone(),
                Arc::clone(&peers),
                datagram_tx,
                reliable_tx,
                event_tx.clone(),
            );

            {
                let mut peers_guard = peers.write().unwrap();
                peers_guard.insert(
                    peer_id.clone(),
                    PeerState {
                        connection,
                        _tasks: tasks,
                    },
                );
            }

            let _ = event_tx.send(ConnectionEvent::PeerConnected(peer_id.clone()));
            tracing::info!(local = %local_peer_id, remote = %peer_id, "accepted peer connection");
        });
    }
}

fn spawn_connection_tasks(
    peer_id: PeerId,
    connection: quinn::Connection,
    peers: Arc<RwLock<HashMap<PeerId, PeerState>>>,
    datagram_tx: mpsc::UnboundedSender<(PeerId, Vec<u8>)>,
    reliable_tx: mpsc::UnboundedSender<(PeerId, Vec<u8>)>,
    event_tx: broadcast::Sender<ConnectionEvent>,
) -> Vec<JoinHandle<()>> {
    let mut tasks = Vec::new();

    // Datagram reader task
    tasks.push(tokio::spawn({
        let peer_id = peer_id.clone();
        let connection = connection.clone();
        let peers = Arc::clone(&peers);
        let event_tx = event_tx.clone();
        async move {
            while let Ok(data) = connection.read_datagram().await {
                let _ = datagram_tx.send((peer_id.clone(), data.to_vec()));
            }
            remove_peer(&peers, &peer_id, &event_tx);
        }
    }));

    // Unidirectional stream acceptor task
    tasks.push(tokio::spawn({
        let peer_id = peer_id.clone();
        let connection = connection.clone();
        let peers = Arc::clone(&peers);
        let event_tx = event_tx.clone();
        async move {
            while let Ok(mut recv) = connection.accept_uni().await {
                let peer_id = peer_id.clone();
                let reliable_tx = reliable_tx.clone();
                tokio::spawn(async move {
                    match read_stream_to_end(&mut recv).await {
                        Ok(data) => {
                            let _ = reliable_tx.send((peer_id, data));
                        }
                        Err(e) => {
                            tracing::warn!(
                                peer = %peer_id,
                                "failed to read reliable stream: {e}"
                            );
                        }
                    }
                });
            }
            remove_peer(&peers, &peer_id, &event_tx);
        }
    }));

    tasks
}

fn remove_peer(
    peers: &RwLock<HashMap<PeerId, PeerState>>,
    peer_id: &PeerId,
    event_tx: &broadcast::Sender<ConnectionEvent>,
) {
    let mut peers = peers.write().unwrap();
    if peers.remove(peer_id).is_some() {
        let _ = event_tx.send(ConnectionEvent::PeerDisconnected(peer_id.clone()));
        tracing::info!(peer = %peer_id, "peer disconnected");
    }
}

async fn read_stream_to_end(recv: &mut quinn::RecvStream) -> Result<Vec<u8>, quinn::ReadError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match recv.read(&mut chunk).await? {
            Some(n) => buf.extend_from_slice(&chunk[..n]),
            None => return Ok(buf),
        }
    }
}

async fn read_handshake(recv: &mut quinn::RecvStream) -> Result<Handshake, ConnectionError> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| ConnectionError::Other(Box::new(e)))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 1024 {
        return Err(ConnectionError::Other(
            "handshake too large".to_string().into(),
        ));
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| ConnectionError::Other(Box::new(e)))?;
    postcard::from_bytes(&buf).map_err(|e| ConnectionError::Other(Box::new(e)))
}

// --- TLS skip-verification ---

#[derive(Debug)]
struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(Arc::new(rustls::crypto::ring::default_provider())))
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
