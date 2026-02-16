use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use quinn::crypto::rustls::QuicClientConfig;
use rustls::pki_types::CertificateDer;
use serde::{Deserialize, Serialize};

use super::ConnectionError;

// --- Protocol types (shared between client and server) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Register { peer_id: String, password: String },
    Keepalive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Registered {
        peers: Vec<PeerEntry>,
        your_addr: SocketAddr,
    },
    PeerList {
        peers: Vec<PeerEntry>,
    },
    AuthFailed {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    pub peer_id: String,
    pub addrs: Vec<SocketAddr>,
}

// --- Framing helpers (length-prefixed postcard, same as peer handshake) ---

pub async fn write_message<T: Serialize>(
    send: &mut quinn::SendStream,
    msg: &T,
) -> Result<(), ConnectionError> {
    let data = postcard::to_allocvec(msg).map_err(|e| ConnectionError::Other(Box::new(e)))?;
    let len = (data.len() as u32).to_be_bytes();
    send.write_all(&len)
        .await
        .map_err(|e| ConnectionError::Other(Box::new(e)))?;
    send.write_all(&data)
        .await
        .map_err(|e| ConnectionError::Other(Box::new(e)))?;
    Ok(())
}

pub async fn read_message<T: for<'de> Deserialize<'de>>(
    recv: &mut quinn::RecvStream,
) -> Result<T, ConnectionError> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| ConnectionError::Other(Box::new(e)))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 {
        return Err(ConnectionError::Other("message too large".into()));
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| ConnectionError::Other(Box::new(e)))?;
    postcard::from_bytes(&buf).map_err(|e| ConnectionError::Other(Box::new(e)))
}

// --- Certificate fingerprinting ---

/// Compute SHA-256 fingerprint of a DER-encoded certificate.
pub fn cert_fingerprint(cert_der: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, cert_der);
    format!("SHA256:{}", BASE64.encode(digest.as_ref()))
}

/// Display-friendly fingerprint (same format as SSH).
pub struct Fingerprint(pub String);

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// --- TOFU verifier ---

/// Trust-On-First-Use certificate verifier (SSH-style).
///
/// On first connection to a server address, accepts the certificate and stores
/// its fingerprint. On subsequent connections, verifies the fingerprint matches.
#[derive(Debug)]
pub struct TofuVerifier {
    known_servers_path: PathBuf,
    /// The key used to look up/store the fingerprint (e.g. "host:port").
    server_key: String,
    crypto_provider: Arc<rustls::crypto::CryptoProvider>,
}

impl TofuVerifier {
    pub fn new(known_servers_path: PathBuf, server_key: String) -> Arc<Self> {
        Arc::new(Self {
            known_servers_path,
            server_key,
            crypto_provider: Arc::new(rustls::crypto::ring::default_provider()),
        })
    }

    fn load_known_fingerprints(&self) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        let contents = match std::fs::read_to_string(&self.known_servers_path) {
            Ok(c) => c,
            Err(_) => return map,
        };
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((addr, fp)) = line.split_once(' ') {
                map.insert(addr.to_string(), fp.to_string());
            }
        }
        map
    }

    fn save_fingerprint(&self, server_name: &str, fingerprint: &str) -> std::io::Result<()> {
        if let Some(parent) = self.known_servers_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.known_servers_path)?;
        writeln!(file, "{server_name} {fingerprint}")?;
        Ok(())
    }
}

impl rustls::client::danger::ServerCertVerifier for TofuVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fingerprint = cert_fingerprint(end_entity.as_ref());
        let name = &self.server_key;
        let known = self.load_known_fingerprints();

        if let Some(stored_fp) = known.get(name) {
            if *stored_fp == fingerprint {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            } else {
                Err(rustls::Error::General(format!(
                    "server certificate has changed for {name}: expected {stored_fp}, got {fingerprint}"
                )))
            }
        } else {
            // First connection — trust and store
            tracing::info!(
                server = %name,
                fingerprint = %fingerprint,
                "TOFU: accepting new server certificate"
            );
            if let Err(e) = self.save_fingerprint(name, &fingerprint) {
                tracing::warn!("failed to save server fingerprint: {e}");
            }
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }
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
        self.crypto_provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

// --- Relay header encoding ---

/// Encode a relay header: `[1 byte: peer_id length][peer_id UTF-8 bytes][payload]`
pub fn encode_relay_header(peer_id: &str, payload: &[u8]) -> Vec<u8> {
    let id_bytes = peer_id.as_bytes();
    let mut buf = Vec::with_capacity(1 + id_bytes.len() + payload.len());
    buf.push(id_bytes.len() as u8);
    buf.extend_from_slice(id_bytes);
    buf.extend_from_slice(payload);
    buf
}

/// Decode a relay header. Returns `(peer_id, payload)`.
pub fn decode_relay_header(data: &[u8]) -> Option<(String, &[u8])> {
    if data.is_empty() {
        return None;
    }
    let id_len = data[0] as usize;
    if data.len() < 1 + id_len {
        return None;
    }
    let id = std::str::from_utf8(&data[1..1 + id_len]).ok()?;
    Some((id.to_string(), &data[1 + id_len..]))
}

// --- Rendezvous client ---

/// Client for the DessPlay rendezvous server.
///
/// Uses the caller's quinn::Endpoint so the server sees the same source address
/// used for peer-to-peer connections (STUN).
pub struct RendezvousClient {
    connection: quinn::Connection,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

impl RendezvousClient {
    /// Connect to the rendezvous server with TOFU certificate verification.
    ///
    /// Returns the client, the initial peer list, and the client's observed address.
    pub async fn connect(
        endpoint: &quinn::Endpoint,
        server_addr: SocketAddr,
        peer_id: &str,
        password: &str,
        known_servers_path: &Path,
        server_key: &str,
    ) -> Result<(Self, Vec<PeerEntry>, SocketAddr), ConnectionError> {
        // Build a client config with TOFU verification
        let tofu = TofuVerifier::new(known_servers_path.to_path_buf(), server_key.to_string());
        let crypto_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(tofu)
            .with_no_client_auth();

        let mut transport = quinn::TransportConfig::default();
        transport.keep_alive_interval(Some(std::time::Duration::from_secs(5)));
        transport.max_idle_timeout(Some(
            quinn::IdleTimeout::try_from(std::time::Duration::from_secs(30)).unwrap(),
        ));

        let mut client_config = quinn::ClientConfig::new(Arc::new(
            QuicClientConfig::try_from(crypto_config)
                .map_err(|e| ConnectionError::Other(Box::new(e)))?,
        ));
        client_config.transport_config(Arc::new(transport));

        let connecting = endpoint
            .connect_with(client_config, server_addr, "dessplay-rendezvous")
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;

        let connection = connecting
            .await
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;

        // Open control stream
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| ConnectionError::Other(Box::new(e)))?;

        // Send Register
        write_message(
            &mut send,
            &ClientMessage::Register {
                peer_id: peer_id.to_string(),
                password: password.to_string(),
            },
        )
        .await?;

        // Read response
        let response: ServerMessage = read_message(&mut recv).await?;

        match response {
            ServerMessage::Registered { peers, your_addr } => Ok((
                Self {
                    connection,
                    send,
                    recv,
                },
                peers,
                your_addr,
            )),
            ServerMessage::AuthFailed { reason } => {
                Err(ConnectionError::Other(format!("auth failed: {reason}").into()))
            }
            _ => Err(ConnectionError::Other(
                "unexpected server response".into(),
            )),
        }
    }

    /// Poll the server for an updated peer list.
    pub async fn keepalive(&mut self) -> Result<Vec<PeerEntry>, ConnectionError> {
        write_message(&mut self.send, &ClientMessage::Keepalive).await?;

        let response: ServerMessage = read_message(&mut self.recv).await?;

        match response {
            ServerMessage::PeerList { peers } => Ok(peers),
            _ => Err(ConnectionError::Other(
                "unexpected server response to keepalive".into(),
            )),
        }
    }

    /// The underlying QUIC connection (for relay forwarding).
    pub fn connection(&self) -> &quinn::Connection {
        &self.connection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_header_round_trip() {
        let payload = b"hello world";
        let encoded = encode_relay_header("alice", payload);
        let (peer_id, decoded_payload) = decode_relay_header(&encoded).unwrap();
        assert_eq!(peer_id, "alice");
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn relay_header_empty_payload() {
        let encoded = encode_relay_header("bob", &[]);
        let (peer_id, payload) = decode_relay_header(&encoded).unwrap();
        assert_eq!(peer_id, "bob");
        assert!(payload.is_empty());
    }

    #[test]
    fn relay_header_decode_empty() {
        assert!(decode_relay_header(&[]).is_none());
    }

    #[test]
    fn relay_header_decode_truncated() {
        // peer_id length says 5 but only 2 bytes follow
        assert!(decode_relay_header(&[5, b'a', b'b']).is_none());
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let cert_data = b"some certificate data";
        let fp1 = cert_fingerprint(cert_data);
        let fp2 = cert_fingerprint(cert_data);
        assert_eq!(fp1, fp2);
        assert!(fp1.starts_with("SHA256:"));
    }
}
