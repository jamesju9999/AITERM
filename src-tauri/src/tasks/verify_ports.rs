//! 驗收卡用的埠：派工時先挑好空閒的埠交給驗收者，驗收結束後把還在監聽
//! 那些埠的行程收掉——驗收者忘了關的開發伺服器不會一直佔著埠。
//!
//! 只動這幾個埠上的行程，而且永遠跳過 AITerm 自己。

use std::path::Path;

const PORTS_FILE: &str = "ports.json";

/// `n` 個目前空閒、彼此不同的埠。同時綁住再一起放掉，才不會拿到重複的。
pub fn pick_free_ports(n: usize) -> Vec<u16> {
    let listeners: Vec<std::net::TcpListener> =
        (0..n).filter_map(|_| std::net::TcpListener::bind("127.0.0.1:0").ok()).collect();
    listeners.iter().filter_map(|l| l.local_addr().ok().map(|a| a.port())).collect()
}

pub fn save_ports(task_dir: &Path, ports: &[u16]) {
    let res = std::fs::create_dir_all(task_dir)
        .and_then(|_| std::fs::write(task_dir.join(PORTS_FILE), serde_json::to_string(ports).unwrap_or_default()));
    if let Err(e) = res {
        log::warn!("save verify ports {task_dir:?}: {e}");
    }
}

pub fn load_ports(task_dir: &Path) -> Vec<u16> {
    std::fs::read_to_string(task_dir.join(PORTS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// 在 `port` 上監聽的行程。
#[cfg(unix)]
fn pids_listening(port: u16) -> Vec<u32> {
    let Ok(out) = std::process::Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect()
}

#[cfg(windows)]
fn pids_listening(port: u16) -> Vec<u32> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(out) = std::process::Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
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
            (cols.len() >= 5 && cols[0] == "TCP" && cols[1].ends_with(&suffix) && cols[3] == "LISTENING")
                .then(|| cols[4].parse().ok())
                .flatten()
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
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

/// 結束還在監聽這些埠的行程（AITerm 自己除外），回傳結束了哪些 pid。
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

    /// 不在這裡重綁一次來「證明空著」：放掉之後到重綁之間，平行跑的其他測試可能
    /// 正好拿到同一個臨時埠（實際踩過），那個斷言本身就有賽跑。
    #[test]
    fn free_ports_are_distinct() {
        let ports = pick_free_ports(3);
        assert_eq!(ports.len(), 3);
        assert!(ports.iter().all(|p| *p > 0));
        assert_eq!(ports.iter().collect::<std::collections::HashSet<_>>().len(), 3);
    }

    #[test]
    fn ports_round_trip_through_the_card_folder() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("tasks").join("v");
        assert!(load_ports(&d).is_empty());
        save_ports(&d, &[4001, 4002]);
        assert_eq!(load_ports(&d), vec![4001, 4002]);
    }

    #[test]
    fn netstat_listening_rows_for_the_port_only() {
        let text = "\
  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:5173           0.0.0.0:0              LISTENING       4242
  TCP    [::]:5173              [::]:0                 LISTENING       4242
  TCP    127.0.0.1:51730        0.0.0.0:0              LISTENING       7
  TCP    127.0.0.1:5173         127.0.0.1:60000        ESTABLISHED     99
";
        assert_eq!(parse_netstat(text, 5173), vec![4242]);
        assert!(parse_netstat(text, 80).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn leftover_servers_on_our_ports_are_stopped() {
        let port = pick_free_ports(1)[0];
        let mut child = std::process::Command::new("python3")
            .args([
                "-c",
                &format!("import socket,time\ns=socket.socket()\ns.bind(('127.0.0.1',{port}))\ns.listen()\ntime.sleep(60)"),
            ])
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(std::time::Instant::now() < deadline, "測試用的伺服器沒有起來");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert_eq!(stop_listeners(&[port]), vec![child.id()]);
        assert!(!child.wait().unwrap().success(), "應該是被結束的");
    }

    #[test]
    fn never_stops_ourselves() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(stop_listeners(&[port]).is_empty());
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    }
}
