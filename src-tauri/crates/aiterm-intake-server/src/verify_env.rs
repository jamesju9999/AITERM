//! 驗收的執行環境：空閒埠、結束佔埠的行程、瀏覽器偵測與 Playwright MCP 設定。
//!
//! 由桌面版 `src-tauri/src/tasks/verify_ports.rs` 與 `tasks/ask.rs` 複製調整而來（不改桌面版，兩份各自維護；
//! 桌面版已在 macOS 與 Windows 實測，Linux 與 server 的 Windows 版這裡沒有實測）。
//! 只動指定埠上的行程，而且永遠跳過 server 自己。

/// Playwright MCP 版本釘死：工具參數在版本之間會改（實測 `browser_click` 的參數已改名成 `target`）。
pub const PLAYWRIGHT_MCP: &str = "@playwright/mcp@0.0.83";

/// 驗收者的瀏覽器：有裝 Chrome 就用它（不必下載）；沒有就用 Playwright 的 Chrome for Testing，
/// 第一次用要先下載（約 100MB，裝在使用者自己的快取，不需要管理員權限）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    SystemChrome,
    Bundled,
}

/// 這台電腦有沒有裝 Google Chrome（Playwright 的 `chrome` 通道找的位置）。
pub fn detect_browser() -> Browser {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        candidates.push("/Applications/Google Chrome.app".into());
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Applications/Google Chrome.app"));
        }
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(var) {
                candidates.push(std::path::Path::new(&base).join("Google/Chrome/Application/chrome.exe"));
            }
        }
    } else {
        candidates.push("/opt/google/chrome/chrome".into());
    }
    if candidates.iter().any(|p| p.exists()) {
        Browser::SystemChrome
    } else {
        Browser::Bundled
    }
}

/// Playwright MCP 的啟動參數（`npx` 之後那串）：無頭、獨立設定檔，輸出（截圖）寫到 `out`。
pub fn playwright_args(out: &std::path::Path, browser: Browser) -> Vec<String> {
    let mut args = vec!["-y".to_string(), PLAYWRIGHT_MCP.into(), "--headless".into(), "--isolated".into(), "--output-dir".into(), out.to_string_lossy().into_owned()];
    if browser == Browser::Bundled {
        args.extend(["--browser".to_string(), "chromium".to_string()]);
    }
    args
}

/// `--mcp-config` 的內容：只放釘死版本的無頭瀏覽器，不帶別的 MCP。
pub fn mcp_config_json(out: &std::path::Path, browser: Browser) -> String {
    serde_json::json!({ "mcpServers": { "playwright": { "command": "npx", "args": playwright_args(out, browser) } } }).to_string()
}

/// 這台電腦有沒有 `npx`（沒有就沒有瀏覽器工具，驗收者要把需要瀏覽器的條件標 manual）。
pub fn node_available() -> bool {
    std::process::Command::new(if cfg!(windows) { "npx.cmd" } else { "npx" })
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `n` 個目前空閒、彼此不同的埠。同時綁住再一起放掉，才不會拿到重複的。
pub fn pick_free_ports(n: usize) -> Vec<u16> {
    let listeners: Vec<std::net::TcpListener> = (0..n).filter_map(|_| std::net::TcpListener::bind("127.0.0.1:0").ok()).collect();
    listeners.iter().filter_map(|l| l.local_addr().ok().map(|a| a.port())).collect()
}

/// 在 `port` 上監聽的行程。
#[cfg(unix)]
fn pids_listening(port: u16) -> Vec<u32> {
    let Ok(out) = std::process::Command::new("lsof").args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect()
}

#[cfg(windows)]
fn pids_listening(port: u16) -> Vec<u32> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(out) = std::process::Command::new("netstat").args(["-ano", "-p", "TCP"]).creation_flags(CREATE_NO_WINDOW).output() else {
        return Vec::new();
    };
    parse_netstat(&String::from_utf8_lossy(&out.stdout), port)
}

/// `netstat -ano` 的輸出裡，本地位址是 `:<port>` 而且狀態是 LISTENING 的 PID。
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_netstat(text: &str, port: u16) -> Vec<u32> {
    let suffix = format!(":{port}");
    let mut pids: Vec<u32> = text
        .lines()
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            (cols.len() >= 5 && cols[0] == "TCP" && cols[1].ends_with(&suffix) && cols[3] == "LISTENING").then(|| cols[4].parse().ok()).flatten()
        })
        .collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

#[cfg(unix)]
fn terminate(pid: u32) {
    let alive = |pid: u32| std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().is_ok_and(|s| s.success());
    let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    for _ in 0..10 {
        if !alive(pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status();
}

#[cfg(windows)]
fn terminate(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(CREATE_NO_WINDOW).status();
}

/// 結束還在監聽這些埠的行程（server 自己除外），回傳結束了哪些 pid。只動這些埠，不碰別的。
pub fn stop_listeners(ports: &[u16]) -> Vec<u32> {
    let me = std::process::id();
    let mut pids: Vec<u32> = ports.iter().flat_map(|p| pids_listening(*p)).filter(|p| *p != me).collect();
    pids.sort_unstable();
    pids.dedup();
    for pid in &pids {
        terminate(*pid);
    }
    pids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_distinct_and_the_mcp_config_only_has_the_pinned_headless_browser() {
        let ports = pick_free_ports(3);
        assert_eq!(ports.len(), 3);
        assert_eq!(ports.iter().collect::<std::collections::HashSet<_>>().len(), 3);
        let json = mcp_config_json(std::path::Path::new("/v/evidence"), Browser::SystemChrome);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["mcpServers"].as_object().unwrap().len(), 1, "只有 playwright，不帶別的 MCP");
        assert_eq!(v["mcpServers"]["playwright"]["args"], serde_json::json!(["-y", "@playwright/mcp@0.0.83", "--headless", "--isolated", "--output-dir", "/v/evidence"]));
        let bundled: serde_json::Value = serde_json::from_str(&mcp_config_json(std::path::Path::new("/v/evidence"), Browser::Bundled)).unwrap();
        assert!(bundled["mcpServers"]["playwright"]["args"].as_array().unwrap().iter().any(|a| a == "chromium"));
    }

    #[test]
    fn netstat_listening_rows_for_the_port_only() {
        let text = "\
  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:5173           0.0.0.0:0              LISTENING       4242
  TCP    [::]:5173              [::]:0                 LISTENING       4242
  TCP    127.0.0.1:51730        0.0.0.0:0              LISTENING       7
  TCP    127.0.0.1:5173         127.0.0.1:60000        ESTABLISHED     9
";
        assert_eq!(parse_netstat(text, 5173), vec![4242], "只要 LISTENING 且埠完全相符；51730 與 ESTABLISHED 都不算，同一個 pid 去重");
    }

    #[cfg(unix)]
    #[test]
    fn only_processes_listening_on_the_given_ports_are_stopped() {
        use std::process::{Command, Stdio};
        let mine = pick_free_ports(1)[0];
        let other = pick_free_ports(1)[0];
        let spawn = |port: u16| {
            Command::new("python3")
                .args(["-c", &format!("import socket,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',{port}));s.listen(1);time.sleep(60)")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        };
        let (mut a, mut b) = (spawn(mine), spawn(other));
        std::thread::sleep(std::time::Duration::from_millis(800));
        let stopped = stop_listeners(&[mine]);
        assert_eq!(stopped.len(), 1, "只結束那個埠上的行程");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(a.try_wait().unwrap().is_some(), "指定埠的行程結束了");
        assert!(b.try_wait().unwrap().is_none(), "別的埠的行程沒動");
        b.kill().ok();
        b.wait().ok();
    }

    #[test]
    fn node_availability_is_detected_without_panicking() {
        let _ = node_available();
    }
}
