//! 測試用的假網站（只在測試編譯）：GET 回一張表單，POST 讓計數器加一。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::types::{Mode, SystemConnection};

pub async fn fake_site() -> (u16, Arc<AtomicUsize>) {
    let (port, posts, _) = fake_site_with_bodies().await;
    (port, posts)
}

/// 同上，另外回傳每次 POST 的內容（表單編碼的字串）。
pub async fn fake_site_with_bodies() -> (u16, Arc<AtomicUsize>, Arc<parking_lot::Mutex<Vec<String>>>) {
    let bodies: Arc<parking_lot::Mutex<Vec<String>>> = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let b = bodies.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let posts = Arc::new(AtomicUsize::new(0));
    let p = posts.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { break };
            let p = p.clone();
            let b = b.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let echo;
                let body = if head.starts_with("GET /whoami") {
                    // 回報瀏覽器送來的 Cookie 標頭（驗證 session 有沒有載進去）。
                    let cookie = head.lines().find(|l| l.to_ascii_lowercase().starts_with("cookie:")).map(|l| l[7..].trim().to_string()).unwrap_or_default();
                    echo = format!("<h1>cookie={cookie}</h1>");
                    echo.as_str()
                } else if head.starts_with("POST") {
                    p.fetch_add(1, Ordering::SeqCst);
                    b.lock().push(head.split("\r\n\r\n").nth(1).unwrap_or("").to_string());
                    "<h1>saved</h1>"
                } else if head.starts_with("GET /drop") {
                    // 純拖曳區塊：沒有 input[type=file]，只聽 drop 事件，把檔案用 fetch 送出。
                    "<div id=\"dz\" role=\"button\" aria-label=\"Drop here\" tabindex=\"0\" style=\"width:300px;height:120px;border:2px dashed #888\">drop zone</div><script>const z=document.getElementById('dz');z.addEventListener('dragover',e=>e.preventDefault());z.addEventListener('drop',e=>{e.preventDefault();const f=new FormData();for(const x of e.dataTransfer.files)f.append('f',x);fetch('/submit',{method:'POST',body:f}).then(()=>{z.textContent='sent'})});</script>"
                } else if head.starts_with("GET /upload") {
                    "<form method=\"POST\" action=\"/submit\" enctype=\"multipart/form-data\"><input type=\"file\" name=\"f\"><button type=\"submit\">Send</button></form>"
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
    (port, posts, bodies)
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
        model: None, upload_dirs: vec![], ..Default::default()
    }
}

