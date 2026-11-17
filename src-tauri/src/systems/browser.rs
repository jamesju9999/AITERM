//! AITerm 管理的獨立 Chrome：每個系統一個設定檔、有頭視窗（使用者看得到、能接手）、
//! 偵錯埠只綁 127.0.0.1。Playwright MCP 與 AITerm 的閘門都用 CDP 連上它。

use std::path::{Path, PathBuf};
use std::time::Duration;

/// 每個系統自己的瀏覽器設定檔（cookie 與登入狀態在這裡）。
pub fn profile_dir(base: &Path, system_id: &str) -> PathBuf {
    base.join("system-profiles").join(system_id)
}

pub fn chrome_args(profile: &Path, port: u16, start_url: &str) -> Vec<String> {
    vec![
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--remote-debugging-address=127.0.0.1".into(),
        format!("--remote-debugging-port={port}"),
        format!("--user-data-dir={}", profile.display()),
        start_url.into(),
    ]
}

/// 綁 127.0.0.1:0 取得一個空閒埠，再放掉交給 Chrome。
pub fn pick_free_port() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

/// 系統裝的 Google Chrome 執行檔；找不到回 `None`。
pub fn chrome_executable() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        candidates.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"));
        }
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(var) {
                candidates.push(Path::new(&base).join("Google/Chrome/Application/chrome.exe"));
            }
        }
    } else {
        candidates.push("/opt/google/chrome/chrome".into());
        for name in ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"] {
            candidates.push(PathBuf::from("/usr/bin").join(name));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

pub struct RunningBrowser {
    child: tokio::process::Child,
    pub port: u16,
    pub ws_url: String,
    pub profile: PathBuf,
}

impl RunningBrowser {
    /// Playwright MCP 要用的 `--cdp-endpoint`。
    pub fn cdp_http(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// 行程還在不在（使用者可能自己把視窗關了）。
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn close(&mut self) {
        let _ = self.child.kill().await;
    }
}

pub async fn launch(base: &Path, system_id: &str, start_url: &str) -> Result<RunningBrowser, String> {
    let exe = chrome_executable().ok_or("找不到 Google Chrome，請先安裝")?;
    let profile = profile_dir(base, system_id);
    std::fs::create_dir_all(&profile).map_err(|e| format!("無法建立瀏覽器設定檔資料夾：{e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700));
    }
    let port = pick_free_port().map_err(|e| e.to_string())?;
    let child = tokio::process::Command::new(&exe)
        .args(chrome_args(&profile, port, start_url))
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("無法啟動 Chrome：{e}"))?;
    let mut browser = RunningBrowser { child, port, ws_url: String::new(), profile };
    let url = format!("http://127.0.0.1:{port}/json/version");
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                if let Some(ws) = v["webSocketDebuggerUrl"].as_str() {
                    browser.ws_url = ws.to_string();
                    return Ok(browser);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    browser.close().await;
    Err("Chrome 啟動後 10 秒內沒有開啟偵錯埠".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_dir_is_per_system_and_under_the_app_folder() {
        let a = profile_dir(Path::new("/data"), "sys-a");
        let b = profile_dir(Path::new("/data"), "sys-b");
        assert_ne!(a, b);
        assert!(a.starts_with("/data/system-profiles"));
    }

    #[test]
    fn chrome_args_open_a_loopback_debug_port_with_an_isolated_profile() {
        let args = chrome_args(Path::new("/p"), 19222, "https://x/");
        assert!(args.contains(&"--remote-debugging-port=19222".to_string()));
        assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_string()));
        assert!(args.iter().any(|a| a == "--user-data-dir=/p"));
        assert_eq!(args.last().unwrap(), "https://x/");
        assert!(!args.iter().any(|a| a.contains("headless")), "要有頭：使用者看得到、能接手");
    }

    #[test]
    fn free_port_is_bindable_on_loopback() {
        let p = pick_free_port().unwrap();
        assert!(std::net::TcpListener::bind(("127.0.0.1", p)).is_ok());
    }

    /// 需要真的 Chrome：`cargo test -p app --lib -- --ignored systems::browser`。
    #[tokio::test]
    #[ignore]
    async fn launch_opens_a_loopback_debug_port_and_close_stops_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = launch(dir.path(), "t", "about:blank").await.unwrap();
        assert!(b.ws_url.starts_with("ws://127.0.0.1:"), "{}", b.ws_url);
        let url = format!("{}/json/version", b.cdp_http());
        assert!(reqwest::get(&url).await.is_ok());
        b.close().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(reqwest::get(&url).await.is_err(), "關閉後埠不再回應");
    }
}
