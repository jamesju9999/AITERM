//! 系統連線（外部網頁系統）的設定指令。見
//! docs/superpowers/specs/2026-11-17-system-connections-design.md。

use std::sync::Arc;

use tauri::State;

use crate::config::ConfigStore;
use crate::systems::gate::host_of;
use crate::systems::types::SystemConnection;

/// 儲存前的檢查：網址要是 http(s)；名稱不可空白；允許的網域必須包含起始網址的網域。
pub fn validate(conn: &SystemConnection) -> Result<(), String> {
    if conn.name.trim().is_empty() {
        return Err("名稱不可空白".into());
    }
    if !(conn.start_url.starts_with("http://") || conn.start_url.starts_with("https://")) {
        return Err("起始網址必須是 http 或 https".into());
    }
    let host = host_of(&conn.start_url).ok_or("起始網址不正確")?;
    let covered = conn.allowed_hosts.iter().any(|a| {
        let a = a.trim().trim_start_matches("*.").to_ascii_lowercase();
        !a.is_empty() && (host == a || host.ends_with(&format!(".{a}")))
    });
    if !covered {
        return Err("允許的網域必須包含起始網址的網域".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn systems_list(config: State<'_, Arc<ConfigStore>>) -> Result<Vec<SystemConnection>, String> {
    Ok(config.get().system_connections)
}

/// 新增或取代（同 id）。
#[tauri::command]
pub async fn systems_save(conn: SystemConnection, config: State<'_, Arc<ConfigStore>>) -> Result<(), String> {
    validate(&conn)?;
    config
        .update(|cfg| match cfg.system_connections.iter().position(|c| c.id == conn.id) {
            Some(i) => cfg.system_connections[i] = conn.clone(),
            None => cfg.system_connections.push(conn.clone()),
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn systems_remove(id: String, config: State<'_, Arc<ConfigStore>>) -> Result<(), String> {
    config.update(|cfg| cfg.system_connections.retain(|c| c.id != id)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems::types::Mode;

    fn ok() -> SystemConnection {
        SystemConnection {
            id: "s".into(),
            name: "訂單系統".into(),
            start_url: "https://app.example.com/login".into(),
            allowed_hosts: vec!["example.com".into()],
            mode: Mode::ReadOnly,
            rules: vec![],
            enabled: true,
        }
    }

    #[test]
    fn a_proper_connection_passes() {
        assert!(validate(&ok()).is_ok());
    }

    #[test]
    fn rejects_blank_name_non_http_and_uncovered_host() {
        let mut c = ok();
        c.name = "  ".into();
        assert!(validate(&c).unwrap_err().contains("名稱"));

        let mut c = ok();
        c.start_url = "ftp://app.example.com/".into();
        assert!(validate(&c).unwrap_err().contains("http"));

        let mut c = ok();
        c.allowed_hosts = vec!["other.test".into()];
        assert!(validate(&c).unwrap_err().contains("網域"));

        let mut c = ok();
        c.allowed_hosts = vec![];
        assert!(validate(&c).is_err(), "白名單空的不能存");
    }
}
