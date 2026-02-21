//! QUIC endpoint setup with persistent self-signed certificate.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use quinn::Endpoint;

/// ALPN protocol identifier for DessPlay.
pub const ALPN: &[u8] = b"dessplay";

/// Create a server QUIC endpoint with a persistent self-signed certificate.
///
/// If `cert_path` and `key_path` exist, loads them. Otherwise generates a new
/// certificate and saves it.
pub fn create_server_endpoint(
    bind_addr: std::net::SocketAddr,
    cert_path: &Path,
    key_path: &Path,
) -> Result<Endpoint> {
    let (cert_der, key_der) = load_or_generate_cert(cert_path, key_path)?;

    let cert = rustls::pki_types::CertificateDer::from(cert_der);
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(key_der);

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key.into())
        .context("failed to build rustls server config")?;
    server_crypto.alpn_protocols = vec![ALPN.to_vec()];

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
            .context("failed to create QUIC server config")?,
    ));

    let endpoint =
        Endpoint::server(server_config, bind_addr).context("failed to bind QUIC endpoint")?;

    tracing::info!(%bind_addr, "QUIC server listening");
    Ok(endpoint)
}

/// Load existing cert/key from disk, or generate and save new ones.
fn load_or_generate_cert(cert_path: &Path, key_path: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    if cert_path.exists() && key_path.exists() {
        tracing::info!("Loading existing certificate from {}", cert_path.display());
        let cert_der = fs::read(cert_path)
            .with_context(|| format!("failed to read cert from {}", cert_path.display()))?;
        let key_der = fs::read(key_path)
            .with_context(|| format!("failed to read key from {}", key_path.display()))?;
        return Ok((cert_der, key_der));
    }

    tracing::info!("Generating new self-signed certificate");
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["dessplay-rendezvous".to_string()])
            .context("failed to generate self-signed certificate")?;

    let cert_der = cert.der().to_vec();
    let key_der = key_pair.serialize_der();

    // Ensure parent directory exists
    if let Some(parent) = cert_path.parent() {
        fs::create_dir_all(parent).context("failed to create cert directory")?;
    }

    fs::write(cert_path, &cert_der)
        .with_context(|| format!("failed to write cert to {}", cert_path.display()))?;
    fs::write(key_path, &key_der)
        .with_context(|| format!("failed to write key to {}", key_path.display()))?;

    tracing::info!(
        "Certificate saved to {} and {}",
        cert_path.display(),
        key_path.display()
    );

    Ok((cert_der, key_der))
}
