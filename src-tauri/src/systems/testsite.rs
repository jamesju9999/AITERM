//! 測試用的假網站（只在測試編譯）：GET 回一張表單，POST 讓計數器加一。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::types::{Mode, SystemConnection};

pub async fn fake_site() -> (u16, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let posts = Arc::new(AtomicUsize::new(0));
    let p = posts.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { break };
            let p = p.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let body = if head.starts_with("POST") {
                    p.fetch_add(1, Ordering::SeqCst);
                    "<h1>saved</h1>"
                } else {
                    "<form method=\"POST\" action=\"/submit\"><input name=\"a\" value=\"1\"><button>Save</button></form>"
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    (port, posts)
}

pub fn conn(port: u16, mode: Mode) -> SystemConnection {
    SystemConnection {
        id: "t".into(),
        name: "T".into(),
        start_url: format!("http://127.0.0.1:{port}/"),
        allowed_hosts: vec!["127.0.0.1".into()],
        mode,
        rules: vec![],
        enabled: true,
    }
}

