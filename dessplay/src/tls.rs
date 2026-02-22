//! TOFU (Trust On First Use) certificate verifier for rustls.
//!
//! On first connection to a server, accepts any certificate and stores its
//! SHA-256 fingerprint. On subsequent connections, verifies the certificate
//! matches the stored fingerprint.

use std::sync::{Arc, Mutex};

use ring::digest;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};

use crate::storage::ClientStorage;

/// Certificate verifier that accepts any certificate.
///
/// Used for peer-to-peer connections where identity is verified via the
/// `PeerControl::Hello` handshake (peer_id), not TLS certificates.
#[derive(Debug)]
pub struct AcceptAnyCert;

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

/// Details of a TOFU fingerprint mismatch, for display to the user.
pub struct TofuMismatch {
    pub server: String,
    pub stored_fingerprint: Vec<u8>,
    pub received_fingerprint: Vec<u8>,
}

/// TOFU certificate verifier that stores fingerprints in SQLite.
pub struct TofuVerifier {
    storage: Arc<Mutex<ClientStorage>>,
    server_address: String,
    mismatch: Mutex<Option<TofuMismatch>>,
}

impl TofuVerifier {
    pub fn new(storage: Arc<Mutex<ClientStorage>>, server_address: String) -> Self {
        Self {
            storage,
            server_address,
            mismatch: Mutex::new(None),
        }
    }

    /// Take the stored mismatch details, if any. Returns `None` if the last
    /// verification failure was not a fingerprint mismatch.
    pub fn take_mismatch(&self) -> Option<TofuMismatch> {
        self.mismatch.lock().ok()?.take()
    }
}

impl std::fmt::Debug for TofuVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TofuVerifier")
            .field("server_address", &self.server_address)
            .finish()
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
    ) -> Result<ServerCertVerified, Error> {
        let fingerprint = digest::digest(&digest::SHA256, end_entity.as_ref());
        let fp_bytes = fingerprint.as_ref();

        let storage = self
            .storage
            .lock()
            .map_err(|_| Error::General("storage lock poisoned".into()))?;

        match storage.get_cert(&self.server_address) {
            Ok(Some(stored_fp)) => {
                if stored_fp == fp_bytes {
                    Ok(ServerCertVerified::assertion())
                } else {
                    tracing::error!(
                        server = %self.server_address,
                        "Certificate fingerprint mismatch! Stored fingerprint does not match."
                    );
                    if let Ok(mut m) = self.mismatch.lock() {
                        *m = Some(TofuMismatch {
                            server: self.server_address.clone(),
                            stored_fingerprint: stored_fp.clone(),
                            received_fingerprint: fp_bytes.to_vec(),
                        });
                    }
                    Err(Error::General(format!(
                        "TOFU: certificate fingerprint changed for {}",
                        self.server_address
                    )))
                }
            }
            Ok(None) => {
                // First connection — trust and store
                tracing::info!(
                    server = %self.server_address,
                    "First connection — trusting certificate"
                );
                storage
                    .store_cert(&self.server_address, fp_bytes)
                    .map_err(|e| Error::General(format!("failed to store cert: {e}")))?;
                Ok(ServerCertVerified::assertion())
            }
            Err(e) => Err(Error::General(format!("failed to query cert store: {e}"))),
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        // We trust the TOFU-verified cert, so accept its signatures
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}
