//! TOFU (Trust On First Use) certificate handling.
//!
//! The server presents a persistent self-signed certificate
//! ([`load_or_generate_cert`]). Clients pin its SHA-256 fingerprint on
//! first connection and verify it thereafter ([`TofuVerifier`]).
//!
//! The verifier itself never touches storage: the caller loads the pin
//! *before* connecting and reads the observed fingerprint *after* a
//! successful first connection (then persists it). This keeps SQLite out
//! of the TLS handshake.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// SHA-256 fingerprint of a DER-encoded certificate.
pub fn fingerprint(cert_der: &[u8]) -> [u8; 32] {
    Sha256::digest(cert_der).into()
}

/// Shared slot holding the fingerprint observed during the most recent
/// successful handshake.
pub type ObservedFingerprint = Arc<Mutex<Option<Vec<u8>>>>;

/// Client-side certificate verifier implementing TOFU.
///
/// - With a pinned fingerprint: the presented certificate must match
///   exactly; anything else aborts the handshake.
/// - Without one (first use): any certificate is accepted and its
///   fingerprint is recorded in `observed` for the caller to persist.
///
/// Signature verification still runs normally — TOFU replaces *identity*
/// verification (the CA chain), not the proof of key possession.
#[derive(Debug)]
pub struct TofuVerifier {
    expected: Option<Vec<u8>>,
    observed: Arc<Mutex<Option<Vec<u8>>>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl TofuVerifier {
    /// Build a verifier. `expected` is the pinned fingerprint, if any.
    /// The returned handle yields the fingerprint observed during the
    /// most recent successful handshake.
    pub fn new(expected: Option<Vec<u8>>) -> (Arc<Self>, ObservedFingerprint) {
        let observed = Arc::new(Mutex::new(None));
        let verifier = Arc::new(Self {
            expected,
            observed: Arc::clone(&observed),
            algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        });
        (verifier, observed)
    }
}

impl ServerCertVerifier for TofuVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity.as_ref());
        if let Some(expected) = &self.expected
            && expected.as_slice() != fp.as_slice()
        {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        if let Ok(mut observed) = self.observed.lock() {
            *observed = Some(fp.to_vec());
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Load the server's persistent certificate from `dir`, generating and
/// saving a fresh self-signed one on first run. Returns (cert, key).
pub fn load_or_generate_cert(
    dir: &Path,
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), String> {
    let cert_path = dir.join("cert.der");
    let key_path = dir.join("key.der");

    if cert_path.exists() && key_path.exists() {
        let cert = std::fs::read(&cert_path).map_err(|e| format!("reading cert: {e}"))?;
        let key = std::fs::read(&key_path).map_err(|e| format!("reading key: {e}"))?;
        return Ok((
            CertificateDer::from(cert),
            PrivateKeyDer::try_from(key).map_err(|e| format!("bad stored key: {e}"))?,
        ));
    }

    std::fs::create_dir_all(dir).map_err(|e| format!("creating {dir:?}: {e}"))?;
    let generated = rcgen::generate_simple_self_signed(vec!["dessplay".into()])
        .map_err(|e| format!("generating certificate: {e}"))?;
    let cert_der = generated.cert.der().to_vec();
    let key_der = generated.key_pair.serialize_der();

    std::fs::write(&cert_path, &cert_der).map_err(|e| format!("writing cert: {e}"))?;
    std::fs::write(&key_path, &key_der).map_err(|e| format!("writing key: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
    }

    Ok((
        CertificateDer::from(cert_der),
        PrivateKeyDer::try_from(key_der).map_err(|e| format!("bad generated key: {e}"))?,
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn cert_is_generated_once_and_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let (cert_a, _) = load_or_generate_cert(dir.path()).unwrap();
        let (cert_b, _) = load_or_generate_cert(dir.path()).unwrap();
        assert_eq!(cert_a, cert_b, "regenerated instead of reloading");
        assert_ne!(fingerprint(cert_a.as_ref()), [0u8; 32]);
    }
}
