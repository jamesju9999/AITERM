//! 配對：以邀請碼換 token，並驗證 server 也握有邀請碼（互證，綁定這條 TLS 連線）。
use crate::pin::{client_config, PinVerifier};
use crate::ClientError;
use aiterm_core::share::auth;
use aiterm_intake_core::api::{PairRequest, PairResponse, HEADER_CLIENT_PROOF, HEADER_HOST_PROOF, PAIR_EXPORTER_LABEL};
use rustls::pki_types::ServerName;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsConnector;

#[derive(Debug, Clone, PartialEq)]
pub struct Paired {
    pub token: String,
    pub response: PairResponse,
}

/// 把 `host:port` 或 `https://host:port[/]` 正規化成 `(host, port)`。沒給埠就是 443。
pub fn parse_addr(input: &str) -> Result<(String, u16), ClientError> {
    let s = input.trim().trim_start_matches("https://").trim_end_matches('/');
    if s.is_empty() || s.contains('/') || s.contains(' ') {
        return Err(ClientError::BadAddress);
    }
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => (h.to_string(), p.parse::<u16>().map_err(|_| ClientError::BadAddress)?),
        _ => (s.to_string(), 443),
    };
    Ok((host, port))
}

/// 簡單的 HTTP/1.1 回應解析（`Connection: close`，讀到結束）。
fn parse_response(raw: &[u8]) -> Option<(u16, Vec<(String, String)>, Vec<u8>)> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let mut lines = head.lines();
    let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
    let mut body = raw[split + 4..].to_vec();
    // hyper 可能用 chunked；此處的回應很小，若是 chunked 就解開。
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        body = dechunk(&body)?;
    }
    Some((status, headers, body))
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let nl = b.windows(2).position(|w| w == b"\r\n")?;
        let size = usize::from_str_radix(std::str::from_utf8(&b[..nl]).ok()?.trim(), 16).ok()?;
        b = &b[nl + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size + 2..)?;
    }
}

pub async fn pair(addr: &str, code: &str) -> Result<Paired, ClientError> {
    pair_with_timeout(addr, code, Duration::from_secs(15)).await
}

pub async fn pair_with_timeout(addr: &str, code: &str, timeout: Duration) -> Result<Paired, ClientError> {
    let (host, port) = parse_addr(addr)?;
    let (invite_id, secret) = crate::pair::split_code(code)?;
    let key = aiterm_intake_core::api::invite::invite_key(&secret);
    tokio::time::timeout(timeout, async move {
        let verifier = PinVerifier::new(None);
        let connector = TlsConnector::from(Arc::new(client_config(verifier.clone())));
        let tcp = tokio::net::TcpStream::connect((host.as_str(), port)).await.map_err(|e| ClientError::Offline(e.to_string()))?;
        let name = ServerName::try_from(host.clone()).map_err(|_| ClientError::BadAddress)?;
        let mut tls = connector.connect(name, tcp).await.map_err(|e| ClientError::Offline(e.to_string()))?;
        let exporter = aiterm_core::share::tls::exporter_material_with_label(tls.get_ref().1, PAIR_EXPORTER_LABEL).map_err(|e| ClientError::Offline(e.to_string()))?;
        let body = serde_json::to_vec(&PairRequest { invite_id }).map_err(|e| ClientError::Server(e.to_string()))?;
        let req = format!(
            "POST /v1/pair HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n{HEADER_CLIENT_PROOF}: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            auth::viewer_proof(&key, &exporter),
            body.len()
        );
        tls.write_all(req.as_bytes()).await.map_err(|e| ClientError::Offline(e.to_string()))?;
        tls.write_all(&body).await.map_err(|e| ClientError::Offline(e.to_string()))?;
        let mut raw = Vec::new();
        // server 回完會關連線；TLS 關閉時可能報 unexpected eof，已讀到的內容仍可用。
        let _ = tls.read_to_end(&mut raw).await;
        let (status, headers, body) = parse_response(&raw).ok_or_else(|| ClientError::Server("回應格式不正確".into()))?;
        match status {
            200 => {}
            429 => return Err(ClientError::RateLimited),
            401 => return Err(ClientError::PairingRefused),
            s => return Err(ClientError::Server(format!("HTTP {s}"))),
        }
        let proof = headers.iter().find(|(k, _)| k == HEADER_HOST_PROOF).map(|(_, v)| v.as_str()).unwrap_or("");
        if !auth::verify_host_proof(&key, &exporter, proof) {
            return Err(ClientError::ServerNotProven);
        }
        let response: PairResponse = serde_json::from_slice(&body).map_err(|e| ClientError::Server(e.to_string()))?;
        // 互證通過，這條連線上看到的憑證就是真的 server 的；指紋以實際看到的為準。
        let seen = verifier.seen.lock().unwrap().clone().unwrap_or_default();
        if seen != response.fingerprint {
            return Err(ClientError::ServerNotProven);
        }
        Ok(Paired { token: response.token.clone(), response })
    })
    .await
    .map_err(|_| ClientError::Offline("逾時".into()))?
}

/// 與 server 相同的邀請碼規則（共用 `aiterm_intake_core::api::invite`）。
pub fn split_code(input: &str) -> Result<(String, String), ClientError> {
    aiterm_intake_core::api::invite::parse_code(input).ok_or(ClientError::BadCode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_normalised() {
        assert_eq!(parse_addr("https://192.168.1.20:8443/").unwrap(), ("192.168.1.20".into(), 8443));
        assert_eq!(parse_addr("intake.example.com").unwrap(), ("intake.example.com".into(), 443));
        assert_eq!(parse_addr(" host:9000 ").unwrap(), ("host".into(), 9000));
        for bad in ["", "host:abc", "a b", "host/path"] {
            assert_eq!(parse_addr(bad), Err(ClientError::BadAddress), "{bad}");
        }
    }

    #[test]
    fn codes_are_checked_before_any_network_call() {
        assert!(split_code("ABCD-EFGH-JKMN-PQRS-TUVW-XY").is_ok());
        assert_eq!(split_code("short"), Err(ClientError::BadCode));
        assert_eq!(split_code("ABCD-EFGH-JKMN-PQRS-TUVW-X0"), Err(ClientError::BadCode));
    }

    #[test]
    fn chunked_bodies_are_decoded() {
        assert_eq!(dechunk(b"5\r\nhello\r\n0\r\n\r\n"), Some(b"hello".to_vec()));
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nX-A: b\r\n\r\n3\r\nabc\r\n0\r\n\r\n";
        let (s, h, b) = parse_response(raw).unwrap();
        assert_eq!((s, b), (200, b"abc".to_vec()));
        assert!(h.iter().any(|(k, v)| k == "x-a" && v == "b"));
    }
}
