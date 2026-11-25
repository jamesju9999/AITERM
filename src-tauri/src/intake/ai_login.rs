//! 為 server 登入 ChatGPT／Gemini 訂閱：開瀏覽器走授權，由本機回呼收下授權碼、換成憑證。
//! 憑證**只交給 server**，不寫進這台電腦的鑰匙圈（這台電腦自己的登入是另一組，互不影響）。
use aiterm_ai_oauth::{antigravity, codex, Pkce};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 一次登入拿到、要交給 server 的東西。
#[derive(Debug, Clone, PartialEq)]
pub struct Login {
    pub access: String,
    pub refresh: String,
    pub expires_in: Option<u64>,
    pub account_id: Option<String>,
    pub project_id: Option<String>,
}

const DONE_PAGE: &str = concat!(
    "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Authorization Successful</title></head>",
    "<body style=\"font-family:sans-serif;text-align:center;padding:60px 20px;background:#1a1a1a;color:#fff\">",
    "<h2 style=\"color:#4caf50;margin-bottom:12px\">Authorization Successful!</h2>",
    "<p style=\"color:#aaa\">You can close this window and return to AITerm.</p>",
    "</body></html>"
);

/// 等瀏覽器導回本機網址，回傳授權碼。state 對不上、或授權被拒絕都當錯誤。
pub async fn wait_for_callback(listener: &TcpListener, expected_state: &str, timeout: Duration) -> Result<String, String> {
    let (mut stream, _) = tokio::time::timeout(timeout, listener.accept())
        .await
        .map_err(|_| format!("等了 {} 秒瀏覽器都沒有回來，請重試", timeout.as_secs()))?
        .map_err(|e| format!("本機回呼伺服器錯誤：{e}"))?;
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
    let request = String::from_utf8_lossy(&buf[..n]).to_string();
    let path_query = request.lines().next().and_then(|l| l.split_whitespace().nth(1)).ok_or("瀏覽器回來的請求格式不對")?.to_string();
    let parsed = url::Url::parse(&format!("http://localhost{path_query}")).map_err(|e| format!("解析回呼網址失敗：{e}"))?;
    let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
    let (status, body) = if params.contains_key("code") { ("200 OK", DONE_PAGE.to_string()) } else { ("400 Bad Request", "<h3>Authorization failed</h3>".to_string()) };
    let resp = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(resp.as_bytes()).await;
    drop(stream);
    if let Some(err) = params.get("error") {
        return Err(format!("授權被拒絕或失敗：{err}"));
    }
    let code = params.get("code").map(|v| v.to_string()).ok_or("回呼裡沒有授權碼")?;
    if params.get("state").map(|s| s.as_ref()) != Some(expected_state) {
        return Err("state 對不上：授權碼可能過期或被竄改，請重新登入".into());
    }
    Ok(code)
}

const WAIT: Duration = Duration::from_secs(120);

/// ChatGPT（Codex）：回呼埠固定 1455（Codex 的公開 client 只登記了這個）。
pub async fn login_codex(open: impl FnOnce(&str)) -> Result<Login, String> {
    let listener = TcpListener::bind(("127.0.0.1", codex::REDIRECT_PORT)).await.map_err(|e| format!("無法在 1455 埠啟動本機回呼伺服器（可能被其他程式占用，例如正在登入 Codex）：{e}"))?;
    let pkce = Pkce::new();
    open(&codex::auth_url(&pkce));
    let code = wait_for_callback(&listener, &pkce.state, WAIT).await?;
    let t = codex::exchange(&Default::default(), &code, &pkce.verifier).await?;
    let refresh = t.refresh.ok_or("供應商沒有回傳續期憑證，無法交給 server")?;
    let account_id = t.id_token.as_deref().and_then(codex::account_id_from_id_token);
    Ok(Login { access: t.access, refresh, expires_in: t.expires_in, account_id, project_id: None })
}

/// Gemini（Antigravity）：回呼埠隨機；登入後要開通帳號、取得 Cloud Code 專案 id，失敗就整個登入失敗（不留半套）。
pub async fn login_google(open: impl FnOnce(&str)) -> Result<Login, String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| format!("無法啟動本機回呼伺服器：{e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let pkce = Pkce::new();
    open(&antigravity::auth_url(&pkce, port));
    let code = wait_for_callback(&listener, &pkce.state, WAIT).await?;
    let ep = antigravity::Endpoints::default();
    let t = antigravity::exchange(&ep, &code, &pkce.verifier, port).await?;
    let refresh = t.refresh.ok_or("Google 沒有回傳續期憑證（這個帳號可能先前已授權過，請到 Google 帳號撤銷後重試）")?;
    let project_id = antigravity::onboard(&ep, &t.access, Duration::from_secs(5)).await.map_err(|e| format!("Antigravity 開通失敗：{e}"))?;
    Ok(Login { access: t.access, refresh, expires_in: t.expires_in, account_id: None, project_id: Some(project_id) })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn hit(port: u16, path_query: &str) -> String {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(format!("GET {path_query} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    async fn listener() -> (TcpListener, u16) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        (l, p)
    }

    #[tokio::test]
    async fn a_matching_callback_yields_the_code_and_shows_a_friendly_page() {
        let (l, port) = listener().await;
        let waiter = tokio::spawn(async move { wait_for_callback(&l, "S1", Duration::from_secs(5)).await });
        let page = hit(port, "/auth/callback?code=a%2Bb&state=S1").await;
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("Authorization Successful"));
        assert_eq!(waiter.await.unwrap().unwrap(), "a+b", "百分比編碼要還原");
    }

    #[tokio::test]
    async fn a_wrong_state_is_rejected_so_a_forged_callback_cannot_log_the_server_in() {
        let (l, port) = listener().await;
        let waiter = tokio::spawn(async move { wait_for_callback(&l, "S1", Duration::from_secs(5)).await });
        hit(port, "/cb?code=evil&state=OTHER").await;
        assert!(waiter.await.unwrap().unwrap_err().contains("state"));
    }

    #[tokio::test]
    async fn a_denied_authorization_and_a_missing_code_are_errors_and_silence_times_out() {
        let (l, port) = listener().await;
        let waiter = tokio::spawn(async move { wait_for_callback(&l, "S", Duration::from_secs(5)).await });
        let page = hit(port, "/cb?error=access_denied&state=S").await;
        assert!(page.starts_with("HTTP/1.1 400"));
        assert!(waiter.await.unwrap().unwrap_err().contains("access_denied"));

        let (l, port) = listener().await;
        let waiter = tokio::spawn(async move { wait_for_callback(&l, "S", Duration::from_secs(5)).await });
        hit(port, "/cb?state=S").await;
        assert!(waiter.await.unwrap().is_err());

        let (l, _) = listener().await;
        assert!(wait_for_callback(&l, "S", Duration::from_millis(50)).await.unwrap_err().contains("沒有回來"));
    }

    #[tokio::test]
    async fn the_codex_callback_port_in_use_is_explained() {
        let held = TcpListener::bind(("127.0.0.1", codex::REDIRECT_PORT)).await;
        if held.is_ok() {
            let e = login_codex(|_| panic!("埠被占用時不該開瀏覽器")).await.unwrap_err();
            assert!(e.contains("1455"), "{e}");
        } // 埠本來就被別的程式占用時（例如開著的 AITerm）無法重現，略過。
    }
}
