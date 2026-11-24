//! 憑證驗證：配對前接受任何憑證（身分由互證保證），配對後只信釘住的指紋；公開網路模式用一般根憑證。
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerifier, ServerCertVerified};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DistinguishedName, Error, SignatureScheme};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

pub fn fingerprint_of(cert_der: &[u8]) -> String {
    aiterm_core::share::tls::hex_of(&Sha256::digest(cert_der))
}

/// 記下 server 出示的憑證指紋；`expect` 有值時指紋不符就拒絕。簽章仍照 TLS 規則驗證。
#[derive(Debug)]
pub struct PinVerifier {
    expect: Option<String>,
    pub seen: Mutex<Option<String>>,
    algs: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl PinVerifier {
    pub fn new(expect: Option<String>) -> Arc<Self> {
        aiterm_core::share::ensure_crypto_provider();
        Arc::new(Self { expect, seen: Mutex::new(None), algs: rustls::crypto::ring::default_provider().signature_verification_algorithms })
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _i: &[CertificateDer<'_>], _n: &ServerName<'_>, _o: &[u8], _now: UnixTime) -> Result<ServerCertVerified, Error> {
        let fp = fingerprint_of(end_entity.as_ref());
        *self.seen.lock().unwrap() = Some(fp.clone());
        match &self.expect {
            Some(want) if *want != fp => Err(Error::General("fingerprint mismatch".into())),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        None
    }
}

pub fn client_config(verifier: Arc<PinVerifier>) -> rustls::ClientConfig {
    aiterm_core::share::ensure_crypto_provider();
    rustls::ClientConfig::builder().dangerous().with_custom_certificate_verifier(verifier).with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_sha256_hex() {
        assert_eq!(fingerprint_of(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
