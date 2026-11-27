//! 配對後的 API 呼叫：HTTPS（釘住憑證指紋）帶 Bearer token，以及 WebSocket 變動推播。
use crate::pin::{client_config, PinVerifier};
use crate::ClientError;
use aiterm_intake_core::api::{Event, Hello, PROTOCOL};
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct Api {
    base: String,
    host: String,
    token: String,
    fingerprint: Option<String>,
    http: reqwest::Client,
}

fn map_send_error(e: reqwest::Error) -> ClientError {
    let chain = format!("{e:?}");
    if chain.contains("fingerprint mismatch") {
        ClientError::FingerprintChanged
    } else {
        ClientError::Offline(e.to_string())
    }
}

impl Api {
    /// `addr` 是 `host:port` 或 `https://host:port`。`fingerprint` 有值就只信那個憑證（內網自簽）；
    /// 沒有就用一般的根憑證驗證（公開網路的正式憑證）。
    pub fn new(addr: &str, token: &str, fingerprint: Option<String>) -> Result<Self, ClientError> {
        let (host, port) = crate::pair::parse_addr(addr)?;
        let mut b = reqwest::Client::builder().timeout(Duration::from_secs(30));
        if fingerprint.is_some() {
            b = b.use_preconfigured_tls(client_config(PinVerifier::new(fingerprint.clone())));
        }
        let http = b.build().map_err(|e| ClientError::Offline(e.to_string()))?;
        Ok(Self { base: format!("https://{host}:{port}"), host: format!("{host}:{port}"), token: token.to_string(), fingerprint, http })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// 通用呼叫。成功回 JSON（沒有內容就是 `Null`）。
    pub async fn request(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, ClientError> {
        let mut req = self.http.request(method, format!("{}{}", self.base, path)).bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.map_err(map_send_error)?;
        let status = res.status();
        match status.as_u16() {
            401 => return Err(ClientError::Revoked),
            403 => return Err(ClientError::Forbidden),
            _ => {}
        }
        let text = res.text().await.map_err(|e| ClientError::Offline(e.to_string()))?;
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or_else(|| format!("HTTP {status}"));
            return Err(ClientError::Server(msg));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| ClientError::Server(format!("回應不是預期的格式：{e}")))
    }

    /// 下載原始位元組（附件）。
    pub async fn download(&self, path: &str) -> Result<Vec<u8>, ClientError> {
        let res = self.http.get(format!("{}{}", self.base, path)).bearer_auth(&self.token).send().await.map_err(map_send_error)?;
        match res.status().as_u16() {
            401 => return Err(ClientError::Revoked),
            403 => return Err(ClientError::Forbidden),
            s if !(200..300).contains(&s) => return Err(ClientError::Server(format!("HTTP {s}"))),
            _ => {}
        }
        res.bytes().await.map(|b| b.to_vec()).map_err(|e| ClientError::Offline(e.to_string()))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        self.typed(reqwest::Method::GET, path, None).await
    }

    pub async fn typed<T: DeserializeOwned>(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<T, ClientError> {
        let v = self.request(method, path, body).await?;
        serde_json::from_value(v).map_err(|e| ClientError::Server(format!("回應不是預期的格式：{e}")))
    }

    /// 確認 server 的協定版本與此版本相容。
    pub async fn check_version(&self) -> Result<Hello, ClientError> {
        let hello: Hello = self.get("/v1/hello").await?;
        if hello.protocol != PROTOCOL {
            return Err(ClientError::Incompatible { server: hello.protocol, client: PROTOCOL });
        }
        Ok(hello)
    }

    /// 開 WebSocket 並回傳事件串流。連上後 server 會先送一則目前的 seq。
    pub async fn events(&self) -> Result<EventStream, ClientError> {
        Ok(EventStream { ws: self.ws("/v1/events").await? })
    }

    /// 開一條 WebSocket（釘住憑證指紋、帶 Bearer token）到 `path`。即時預覽與遠端操作用它。
    pub async fn ws(&self, path: &str) -> Result<WsConn, ClientError> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::http::HeaderValue;
        let verifier = PinVerifier::new(self.fingerprint.clone());
        let cfg = client_config(verifier);
        let mut req = format!("wss://{}{path}", self.host).into_client_request().map_err(|e| ClientError::Server(e.to_string()))?;
        req.headers_mut().insert("authorization", HeaderValue::from_str(&format!("Bearer {}", self.token)).map_err(|e| ClientError::Server(e.to_string()))?);
        let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
        let (host, port) = crate::pair::parse_addr(&self.host)?;
        let tcp = tokio::net::TcpStream::connect((host.as_str(), port)).await.map_err(|e| ClientError::Offline(e.to_string()))?;
        let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|_| ClientError::BadAddress)?;
        let tls = connector.connect(name, tcp).await.map_err(|e| {
            let s = format!("{e:?}");
            if s.contains("fingerprint mismatch") {
                ClientError::FingerprintChanged
            } else {
                ClientError::Offline(e.to_string())
            }
        })?;
        let (ws, _) = tokio_tungstenite::client_async(req, tls).await.map_err(|e| {
            let s = format!("{e:?}");
            if s.contains("401") {
                ClientError::Revoked
            } else if s.contains("404") {
                ClientError::Server("找不到這個畫面（瀏覽器可能已經結束了）".into())
            } else if s.contains("403") {
                ClientError::Server("沒有權限".into())
            } else {
                ClientError::Offline(e.to_string())
            }
        })?;
        Ok(WsConn { ws })
    }
}

/// 一條文字訊息的 WebSocket 連線。
pub struct WsConn {
    ws: tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>,
}

impl WsConn {
    /// 下一則文字訊息；連線結束回 `None`。
    pub async fn next_text(&mut self) -> Option<String> {
        use tokio_tungstenite::tungstenite::Message;
        loop {
            match self.ws.next().await? {
                Ok(Message::Text(t)) => return Some(t.to_string()),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    pub async fn send_text(&mut self, text: &str) -> Result<(), ClientError> {
        use tokio_tungstenite::tungstenite::Message;
        self.ws.send(Message::text(text)).await.map_err(|e| ClientError::Offline(e.to_string()))
    }

    pub async fn close(&mut self) {
        let _ = self.ws.close(None).await;
    }
}

pub struct EventStream {
    ws: WsConn,
}

impl EventStream {
    /// 下一則事件；連線結束回 `None`。
    pub async fn next(&mut self) -> Option<Event> {
        loop {
            let t = self.ws.next_text().await?;
            if let Ok(ev) = serde_json::from_str::<Event>(&t) {
                return Some(ev);
            }
        }
    }
}

/// 配對成功後決定要不要釘住憑證指紋：先用一般根憑證試連 `/v1/hello`，
/// 通過代表 server 用的是正式憑證（公開網路），之後憑證換新也不會誤報，不釘；
/// 通不過（自簽憑證）就釘住配對時看到的指紋。
pub async fn decide_pin(addr: &str, fingerprint: &str) -> Option<String> {
    let Ok((host, port)) = crate::pair::parse_addr(addr) else { return Some(fingerprint.to_string()) };
    let plain = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().ok()?;
    match plain.get(format!("https://{host}:{port}/v1/hello")).send().await {
        Ok(r) if r.status().is_success() => None,
        _ => Some(fingerprint.to_string()),
    }
}
