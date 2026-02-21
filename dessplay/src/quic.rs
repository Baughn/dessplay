//! Client QUIC endpoint with TOFU certificate verification and peer acceptance.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use quinn::Endpoint;

use crate::tls::{AcceptAnyCert, TofuVerifier};

/// ALPN protocol identifier for DessPlay.
pub const ALPN: &[u8] = b"dessplay";

/// A QUIC endpoint that can both connect (as client) and accept (as server).
pub struct DualEndpoint {
    /// The QUIC endpoint (server-capable). Default client config uses TOFU
    /// verification for rendezvous server connections.
    pub endpoint: Endpoint,
    /// Client config that accepts any certificate, for peer-to-peer connections.
    pub peer_client_config: quinn::ClientConfig,
}

/// Create a dual-mode QUIC endpoint.
///
/// - **Server config**: ephemeral self-signed cert, no client auth. Allows `endpoint.accept()`.
/// - **Default client config**: TOFU verifier for rendezvous server connections.
/// - **Peer client config**: `AcceptAnyCert` for peer-to-peer connections.
pub fn create_dual_endpoint(
    bind_addr: SocketAddr,
    tofu_verifier: Arc<TofuVerifier>,
) -> Result<DualEndpoint> {
    // Generate ephemeral self-signed cert for server side
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["dessplay".to_string()])
            .context("failed to generate self-signed cert")?;

    let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
    let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der());

    // Server config: accept incoming peer connections
    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der.into())
        .context("failed to create server TLS config")?;
    server_crypto.alpn_protocols = vec![ALPN.to_vec()];

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
            .context("failed to create QUIC server config")?,
    ));

    // Default client config: TOFU verifier for rendezvous server
    let mut tofu_crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(tofu_verifier)
        .with_no_client_auth();
    tofu_crypto.alpn_protocols = vec![ALPN.to_vec()];

    let tofu_client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tofu_crypto)
            .context("failed to create TOFU QUIC client config")?,
    ));

    // Peer client config: accept any cert (identity verified via Hello handshake)
    let mut peer_crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
        .with_no_client_auth();
    peer_crypto.alpn_protocols = vec![ALPN.to_vec()];

    let peer_client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(peer_crypto)
            .context("failed to create peer QUIC client config")?,
    ));

    // Create server endpoint, then set default client config
    let mut endpoint = Endpoint::server(server_config, bind_addr)
        .context("failed to bind dual QUIC endpoint")?;
    endpoint.set_default_client_config(tofu_client_config);

    tracing::info!(%bind_addr, "Dual QUIC endpoint created");
    Ok(DualEndpoint {
        endpoint,
        peer_client_config,
    })
}
