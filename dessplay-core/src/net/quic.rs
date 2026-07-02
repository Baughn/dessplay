//! QUIC implementations of the transport traits, via quinn.
//!
//! Both sides share one `TransportConfig`: 10s keep-alives, 30s idle
//! timeout (the Lost threshold), flow-control windows sized for bulk
//! file transfer rather than request/response, and datagrams enabled.
//! The control stream is the first bidirectional stream, opened by the
//! client immediately after the handshake and prioritized above
//! transfer streams.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::sync::{Mutex, mpsc};

use super::framing::{FrameError, read_frame, write_frame};
use super::tofu::TofuVerifier;
use super::transport::{BiStream, Connector, Listener, Transport, TransportError, TransportEvent};

/// ALPN protocol id.
pub const ALPN: &[u8] = b"dessplay/1";

/// Keep-alive interval (liveness signal for presence tracking).
pub const KEEP_ALIVE: Duration = Duration::from_secs(10);

/// Idle timeout — the presence Lost threshold.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Control stream priority (transfer streams use the default 0).
pub const CONTROL_PRIORITY: i32 = 100;

/// How long a completed handshake may wait for the client to open its
/// control stream before the connection is dropped. The idle timeout
/// cannot bound this — the server's keep-alives keep refreshing it, so a
/// peer that finishes the TLS handshake and never opens the control stream
/// would otherwise linger indefinitely. Kept well above a healthy client's
/// "open immediately after handshake" latency.
pub const CONTROL_STREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// Per-stream flow control window. Sized for pushing video chunks over
/// a ~250Mbit, tens-of-ms path with headroom, not for request/response.
pub const STREAM_RECEIVE_WINDOW: u64 = 16 * 1024 * 1024;

/// Connection-level flow control window (multiple transfer streams).
pub const CONNECTION_RECEIVE_WINDOW: u64 = 64 * 1024 * 1024;

fn shared_transport_config() -> quinn::TransportConfig {
    let mut config = quinn::TransportConfig::default();
    config.keep_alive_interval(Some(KEEP_ALIVE));
    if let Ok(timeout) = quinn::IdleTimeout::try_from(IDLE_TIMEOUT) {
        config.max_idle_timeout(Some(timeout));
    }
    config.stream_receive_window(
        quinn::VarInt::from_u64(STREAM_RECEIVE_WINDOW).unwrap_or(quinn::VarInt::MAX),
    );
    config.receive_window(
        quinn::VarInt::from_u64(CONNECTION_RECEIVE_WINDOW).unwrap_or(quinn::VarInt::MAX),
    );
    config.send_window(CONNECTION_RECEIVE_WINDOW);
    config.datagram_receive_buffer_size(Some(1024 * 1024));
    config
}

fn setup<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> TransportError + '_ {
    move |e| TransportError::Setup(format!("{what}: {e}"))
}

/// An established QUIC connection (either side).
pub struct QuicTransport {
    conn: quinn::Connection,
    control_send: Mutex<quinn::SendStream>,
    /// Frames read off the control stream by a dedicated task —
    /// `read_frame` is not cancel-safe, so it must not race in a
    /// `select!`.
    control_frames: Mutex<mpsc::Receiver<Result<Vec<u8>, FrameError>>>,
}

impl QuicTransport {
    fn new(
        conn: quinn::Connection,
        control_send: quinn::SendStream,
        mut control_recv: quinn::RecvStream,
    ) -> Self {
        // Elevate the control stream above bulk transfer/relay streams on
        // *both* endpoints. The server is the relay hub, so server->client
        // carries both bulk ChunkData and state-sync control traffic to a
        // downloading peer; setting it here (rather than only on the client
        // connect path) keeps a bulk download from starving state sync in
        // that direction too. Best-effort: a stream already closing yields
        // ClosedStream, which is harmless.
        let _ = control_send.set_priority(CONTROL_PRIORITY);
        let (frame_tx, frame_rx) = mpsc::channel(64);
        tokio::spawn(async move {
            loop {
                let frame = read_frame(&mut control_recv).await;
                let terminal = frame.is_err();
                if frame_tx.send(frame).await.is_err() || terminal {
                    break;
                }
            }
        });
        Self {
            conn,
            control_send: Mutex::new(control_send),
            control_frames: Mutex::new(frame_rx),
        }
    }

    fn closed_reason(&self) -> String {
        match self.conn.close_reason() {
            Some(reason) => reason.to_string(),
            None => "connection lost".into(),
        }
    }
}

impl Transport for QuicTransport {
    async fn send_control(&self, frame: &[u8]) -> Result<(), TransportError> {
        let mut send = self.control_send.lock().await;
        write_frame(&mut *send, frame).await.map_err(|e| match e {
            FrameError::Io(_) | FrameError::Closed => {
                TransportError::ConnectionLost(self.closed_reason())
            }
            other => TransportError::Frame(other),
        })
    }

    async fn send_datagram(&self, frame: &[u8]) -> Result<(), TransportError> {
        self.conn
            .send_datagram(frame.to_vec().into())
            .map_err(|e| match e {
                quinn::SendDatagramError::TooLarge => TransportError::DatagramTooLarge {
                    len: frame.len(),
                    max: self.conn.max_datagram_size().unwrap_or(0),
                },
                quinn::SendDatagramError::UnsupportedByPeer
                | quinn::SendDatagramError::Disabled => TransportError::DatagramUnsupported,
                quinn::SendDatagramError::ConnectionLost(_) => {
                    TransportError::ConnectionLost(self.closed_reason())
                }
            })
    }

    fn max_datagram_size(&self) -> Option<usize> {
        self.conn.max_datagram_size()
    }

    async fn open_stream(&self) -> Result<BiStream, TransportError> {
        let (send, recv) = self
            .conn
            .open_bi()
            .await
            .map_err(|_| TransportError::ConnectionLost(self.closed_reason()))?;
        Ok(BiStream {
            send: Box::new(send),
            recv: Box::new(recv),
        })
    }

    async fn recv(&self) -> Result<TransportEvent, TransportError> {
        let mut frames = self.control_frames.lock().await;
        tokio::select! {
            frame = frames.recv() => match frame {
                Some(Ok(payload)) => Ok(TransportEvent::Control(payload)),
                Some(Err(FrameError::Closed)) | None => Ok(TransportEvent::Closed {
                    reason: self.closed_reason(),
                }),
                Some(Err(e)) => Err(TransportError::Frame(e)),
            },
            datagram = self.conn.read_datagram() => match datagram {
                Ok(bytes) => Ok(TransportEvent::Datagram(bytes.to_vec())),
                Err(_) => Ok(TransportEvent::Closed { reason: self.closed_reason() }),
            },
            stream = self.conn.accept_bi() => match stream {
                Ok((send, recv)) => Ok(TransportEvent::IncomingStream(BiStream {
                    send: Box::new(send),
                    recv: Box::new(recv),
                })),
                Err(_) => Ok(TransportEvent::Closed { reason: self.closed_reason() }),
            },
        }
    }

    async fn close(&self, reason: &str) {
        self.conn
            .close(quinn::VarInt::from_u32(0), reason.as_bytes());
    }
}

/// Dials the rendezvous server over QUIC with TOFU verification.
pub struct QuicConnector {
    endpoint: quinn::Endpoint,
    server_addr: SocketAddr,
    server_name: String,
    observed: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
}

impl QuicConnector {
    /// Build a connector. `pinned` is the stored fingerprint, if any;
    /// after the first successful [`Connector::connect`],
    /// [`QuicConnector::observed_fingerprint`] yields what to pin.
    pub fn new(
        server_addr: SocketAddr,
        server_name: impl Into<String>,
        pinned: Option<Vec<u8>>,
    ) -> Result<Self, TransportError> {
        let (verifier, observed) = TofuVerifier::new(pinned);
        let crypto = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(setup("tls versions"))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
        let mut crypto = crypto;
        crypto.alpn_protocols = vec![ALPN.to_vec()];

        let quic_crypto = QuicClientConfig::try_from(crypto).map_err(setup("quic tls"))?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));
        client_config.transport_config(Arc::new(shared_transport_config()));

        let bind: SocketAddr = if server_addr.is_ipv4() {
            (Ipv4Addr::UNSPECIFIED, 0).into()
        } else {
            (Ipv6Addr::UNSPECIFIED, 0).into()
        };
        let mut endpoint = quinn::Endpoint::client(bind).map_err(setup("binding endpoint"))?;
        endpoint.set_default_client_config(client_config);

        Ok(Self {
            endpoint,
            server_addr,
            server_name: server_name.into(),
            observed,
        })
    }

    /// The fingerprint presented during the most recent successful
    /// handshake. Persist this after a first-use connection.
    pub fn observed_fingerprint(&self) -> Option<Vec<u8>> {
        self.observed.lock().ok().and_then(|guard| guard.clone())
    }
}

impl Connector for QuicConnector {
    type Conn = QuicTransport;

    async fn connect(&self) -> Result<QuicTransport, TransportError> {
        let started = std::time::Instant::now();
        tracing::debug!(
            addr = %self.server_addr,
            server_name = %self.server_name,
            "dialing"
        );
        let connecting = self
            .endpoint
            .connect(self.server_addr, &self.server_name)
            .map_err(setup("dial"))?;
        let conn = connecting.await.map_err(setup("handshake"))?;
        tracing::debug!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "QUIC handshake complete"
        );
        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(setup("opening control stream"))?;
        tracing::debug!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "control stream open"
        );
        Ok(QuicTransport::new(conn, send, recv))
    }
}

/// Accepts client connections on the rendezvous server.
pub struct QuicListener {
    endpoint: quinn::Endpoint,
    /// Connections whose handshake *and* control stream have both
    /// completed, produced by the background acceptor task. Draining a
    /// queue of already-ready connections (rather than doing the handshake
    /// and control-stream wait inline in `accept`) is what keeps one slow
    /// or malicious peer from wedging the accept path for everyone else.
    ready: Mutex<mpsc::Receiver<(QuicTransport, SocketAddr)>>,
    /// The acceptor task; aborted on drop so dropping the listener releases
    /// the endpoint (the task holds the other endpoint clone).
    acceptor: tokio::task::JoinHandle<()>,
}

impl QuicListener {
    /// Bind a server endpoint with the given persistent certificate.
    ///
    /// Must be called from within a tokio runtime: it spawns a background
    /// acceptor task (and quinn binds its socket through the active
    /// runtime).
    pub fn bind(
        addr: SocketAddr,
        cert: CertificateDer<'static>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Self, TransportError> {
        let mut crypto = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(setup("tls versions"))?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(setup("server cert"))?;
        crypto.alpn_protocols = vec![ALPN.to_vec()];

        let quic_crypto = QuicServerConfig::try_from(crypto).map_err(setup("quic tls"))?;
        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
        server_config.transport_config(Arc::new(shared_transport_config()));

        let endpoint =
            quinn::Endpoint::server(server_config, addr).map_err(setup("binding endpoint"))?;
        let (acceptor, ready) = spawn_acceptor(endpoint.clone());
        Ok(Self {
            endpoint,
            ready: Mutex::new(ready),
            acceptor,
        })
    }

    /// The bound local address (useful with port 0 in tests).
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        self.endpoint.local_addr().map_err(setup("local addr"))
    }
}

impl Drop for QuicListener {
    fn drop(&mut self) {
        self.acceptor.abort();
    }
}

/// Background acceptor: pull each `Incoming` off the endpoint and hand it to
/// its own task for the handshake and the wait for the client's control
/// stream, so neither step happens on the hot accept path. A peer that
/// stalls at either step delays only its own task; completed connections are
/// delivered over the returned channel. This is the fix for the accept-loop
/// wedge — previously the handshake and `accept_bi` ran inline in `accept`,
/// so a single peer that completed the handshake and never opened a control
/// stream blocked every subsequent connection (and the idle timeout could
/// not save it, since keep-alives refresh it).
fn spawn_acceptor(
    endpoint: quinn::Endpoint,
) -> (
    tokio::task::JoinHandle<()>,
    mpsc::Receiver<(QuicTransport, SocketAddr)>,
) {
    let (tx, rx) = mpsc::channel(32);
    let handle = tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let remote = incoming.remote_address();
                let conn = match incoming.await {
                    Ok(conn) => conn,
                    // Handshake failure (bad ALPN, TOFU abort...): not fatal
                    // for the listener.
                    Err(e) => {
                        tracing::debug!(%remote, error = %e, "incoming handshake failed");
                        return;
                    }
                };
                tracing::debug!(%remote, "incoming handshake complete");
                // The client opens the control stream immediately after the
                // handshake; bound the wait explicitly so a peer that never
                // does is reclaimed rather than held forever.
                match tokio::time::timeout(CONTROL_STREAM_TIMEOUT, conn.accept_bi()).await {
                    Ok(Ok((send, recv))) => {
                        // A send error means the listener was dropped; let
                        // the connection drop with it.
                        let _ = tx
                            .send((QuicTransport::new(conn, send, recv), remote))
                            .await;
                    }
                    Ok(Err(e)) => {
                        tracing::debug!(%remote, error = %e, "peer never opened a control stream");
                    }
                    Err(_) => {
                        tracing::debug!(
                            %remote,
                            "control stream not opened within timeout; dropping"
                        );
                        conn.close(0u32.into(), b"control-stream timeout");
                    }
                }
            });
        }
        tracing::debug!("listener endpoint closed; acceptor stopping");
    });
    (handle, rx)
}

impl Listener for QuicListener {
    type Conn = QuicTransport;

    async fn accept(&self) -> Result<(QuicTransport, SocketAddr), TransportError> {
        // Just drain the next ready connection; all the handshake and
        // control-stream work happens off-path in the background acceptor
        // (see `spawn_acceptor`). A closed channel means the endpoint closed.
        self.ready
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| TransportError::Setup("endpoint closed".into()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::net::tofu::load_or_generate_cert;

    /// Both endpoints must elevate the control stream above bulk
    /// transfer/relay streams. The server is the relay hub, so the
    /// historical bug — `set_priority` wired only into the client connect
    /// path — let a bulk download to a peer add latency to that peer's
    /// server->client state sync. Real localhost QUIC: connect a client to
    /// a listener and read the priority back off *each* side's control send
    /// stream.
    #[tokio::test]
    async fn both_endpoints_prioritize_the_control_stream() {
        let cert_dir = tempfile::tempdir().unwrap();
        let (cert, key) = load_or_generate_cert(cert_dir.path()).unwrap();
        let listener = QuicListener::bind("127.0.0.1:0".parse().unwrap(), cert, key).unwrap();
        let server_addr = listener.local_addr().unwrap();

        let connector = QuicConnector::new(server_addr, "dessplay", None).unwrap();
        // The control stream opens lazily, so the server's `accept_bi`
        // (inside `accept`) only fires once the client writes a frame on
        // it — send one after connecting to unblock the accept.
        let client_fut = async {
            let client = connector.connect().await.unwrap();
            client.send_control(b"hello").await.unwrap();
            client
        };
        let (client, accepted) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(client_fut, listener.accept())
        })
        .await
        .expect("connect/accept budget exhausted");
        let (server, _addr) = accepted.unwrap();

        let client_priority = client.control_send.lock().await.priority().unwrap();
        let server_priority = server.control_send.lock().await.priority().unwrap();
        assert_eq!(
            client_priority, CONTROL_PRIORITY,
            "client control stream priority"
        );
        assert_eq!(
            server_priority, CONTROL_PRIORITY,
            "server control stream priority (the relay-hub direction)"
        );
    }

    /// Regression: a peer that completes the QUIC handshake but never opens
    /// its control stream must not wedge the accept path. Before the
    /// background-acceptor fix, `accept` ran the handshake and `accept_bi`
    /// inline, so this idle peer blocked every subsequent connection
    /// forever (the idle timeout never fires — keep-alives refresh it). A
    /// well-behaved client connecting afterwards must still be accepted.
    #[tokio::test]
    async fn idle_peer_does_not_block_the_accept_loop() {
        let cert_dir = tempfile::tempdir().unwrap();
        let (cert, key) = load_or_generate_cert(cert_dir.path()).unwrap();
        let listener = QuicListener::bind("127.0.0.1:0".parse().unwrap(), cert, key).unwrap();
        let server_addr = listener.local_addr().unwrap();

        let connector = QuicConnector::new(server_addr, "dessplay", None).unwrap();

        // The "attacker": completes the handshake (and locally opens the
        // control stream via `connect`) but never *writes* a frame on it, so
        // the server's `accept_bi` never observes the stream. Held alive for
        // the duration so its connection stays up.
        let _idle = connector.connect().await.unwrap();

        // A normal client connects and sends its first control frame.
        let normal_connector = QuicConnector::new(server_addr, "dessplay", None).unwrap();
        let normal = normal_connector.connect().await.unwrap();
        normal.send_control(b"hello").await.unwrap();

        // The listener must hand back the normal client despite the idle one
        // still occupying a connection. Bounded well under CONTROL_STREAM_TIMEOUT
        // so we are asserting the idle peer was skipped, not merely reclaimed.
        let (accepted, _remote) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("accept blocked behind the idle peer")
            .expect("accept failed");

        // Confirm it is really the normal client: its "hello" frame arrives.
        let event = tokio::time::timeout(Duration::from_secs(5), accepted.recv())
            .await
            .expect("recv timed out")
            .expect("recv failed");
        match event {
            TransportEvent::Control(payload) => assert_eq!(payload, b"hello"),
            other => panic!("expected the normal client's control frame, got {other:?}"),
        }
    }
}
