//! Server 的 TLS：憑證（自簽或指定正式憑證）與會把「這條連線的 exporter」注入請求的 accept 迴圈。
use aiterm_intake_core::api::PAIR_EXPORTER_LABEL;
use axum::Router;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HyperBuilder;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tower_service::Service;

pub struct Identity {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
}

impl Identity {
    /// 憑證指紋（SHA-256，小寫 hex）；Client 配對時釘住它。
    pub fn fingerprint(&self) -> String {
        fingerprint_of(self.cert_der.as_ref())
    }
}

pub fn fingerprint_of(cert_der: &[u8]) -> String {
    aiterm_core::share::tls::hex_of(&Sha256::digest(cert_der))
}

/// 這條連線導出的配對互證材料，由 accept 迴圈放進每個請求的 extensions。
#[derive(Clone, Copy)]
pub struct ConnExporter(pub [u8; 32]);

#[derive(Clone, Copy)]
pub struct PeerAddr(pub SocketAddr);

fn read_pem(cert: &Path, key: &Path) -> anyhow::Result<Identity> {
    let cert_pem = std::fs::read(cert)?;
    let key_pem = std::fs::read(key)?;
    let cert_der = rustls_pemfile::certs(&mut cert_pem.as_slice()).next().ok_or_else(|| anyhow::anyhow!("{} 裡沒有憑證", cert.display()))??;
    let key_der = rustls_pemfile::private_key(&mut key_pem.as_slice())?.ok_or_else(|| anyhow::anyhow!("{} 裡沒有私鑰", key.display()))?;
    Ok(Identity { cert_der, key_der })
}

/// 用指定的正式憑證（公開網路）。
pub fn from_files(cert: &Path, key: &Path) -> anyhow::Result<Identity> {
    read_pem(cert, key)
}

/// `<dir>/tls/cert.pem`、`key.pem` 已存在就讀，否則產生自簽憑證並存起來（私鑰權限 0600）。
/// `sans` 是憑證涵蓋的主機名稱或 IP（內網模式 Client 靠指紋釘住，所以名稱不影響安全）。
pub fn load_or_generate(dir: &Path, sans: Vec<String>) -> anyhow::Result<Identity> {
    let tls_dir = dir.join("tls");
    let (cert_path, key_path) = (tls_dir.join("cert.pem"), tls_dir.join("key.pem"));
    if cert_path.exists() && key_path.exists() {
        return read_pem(&cert_path, &key_path);
    }
    std::fs::create_dir_all(&tls_dir)?;
    let mut names = sans;
    for n in ["localhost", "aiterm-intake"] {
        if !names.iter().any(|x| x == n) {
            names.push(n.to_string());
        }
    }
    let generated = rcgen::generate_simple_self_signed(names)?;
    std::fs::write(&cert_path, generated.cert.pem())?;
    std::fs::write(&key_path, generated.key_pair.serialize_pem())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
    }
    read_pem(&cert_path, &key_path)
}

/// TLS accept 迴圈：握手後導出配對互證材料，連同對端位址放進每個請求。
/// 用 `serve_connection_with_upgrades`，WebSocket 才升級得起來。
pub async fn serve(listener: tokio::net::TcpListener, app: Router, identity: Identity, mut shutdown: tokio::sync::oneshot::Receiver<()>) {
    aiterm_core::share::ensure_crypto_provider();
    let config = match rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![identity.cert_der.clone()], identity.key_der.clone_key()) {
        Ok(c) => c,
        Err(e) => {
            log::error!("TLS 設定失敗：{e}");
            return;
        }
    };
    let acceptor = TlsAcceptor::from(Arc::new(config));
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(x) => x,
                Err(e) => {
                    log::error!("accept 失敗：{e}");
                    continue;
                }
            },
            _ = &mut shutdown => break,
        };
        let (acceptor, app) = (acceptor.clone(), app.clone());
        tokio::spawn(async move {
            let Ok(tls) = acceptor.accept(stream).await else { return };
            // 導不出材料就放掉這條連線，不用預設值頂替：那等於讓沒有身分保證的連線混進配對流程。
            let exporter = match aiterm_core::share::tls::exporter_material_with_label(tls.get_ref().1, PAIR_EXPORTER_LABEL) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("導出配對材料失敗，放棄這條連線：{e}");
                    return;
                }
            };
            let svc = hyper::service::service_fn(move |mut req: hyper::Request<Incoming>| {
                req.extensions_mut().insert(ConnExporter(exporter));
                req.extensions_mut().insert(PeerAddr(peer));
                let mut app = app.clone();
                async move { app.call(req).await }
            });
            let _ = HyperBuilder::new(TokioExecutor::new()).serve_connection_with_upgrades(TokioIo::new(tls), svc).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_identity_is_reused_and_has_a_stable_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_generate(dir.path(), vec!["10.0.0.5".into()]).unwrap();
        let b = load_or_generate(dir.path(), vec![]).unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint(), "第二次讀既有的，不重新產生");
        assert_eq!(a.fingerprint().len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("tls/key.pem")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn a_missing_pem_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        assert!(from_files(&dir.path().join("a.pem"), &dir.path().join("b.pem")).is_err());
    }
}
