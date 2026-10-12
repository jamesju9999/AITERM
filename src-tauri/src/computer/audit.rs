//! 稽核紀錄：每次工具呼叫（含被拒絕的）一行 JSON，追加寫入。

use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    /// Unix 秒。
    pub ts: u64,
    pub tool: String,
    pub app: String,
    pub detail: String,
    /// `ok`、`denied`，或 `error: …`。
    pub outcome: String,
}

pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> PathBuf {
        dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM").join("computer_audit.jsonl")
    }

    pub fn append(&self, tool: &str, app: &str, detail: &str, outcome: &str) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = AuditEntry { ts, tool: tool.into(), app: app.into(), detail: detail.into(), outcome: outcome.into() };
        let Ok(line) = serde_json::to_string(&entry) else { return };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            Ok(mut f) => {
                if let Err(e) = writeln!(f, "{line}") {
                    log::warn!("寫入 computer 稽核紀錄失敗：{e}");
                }
            }
            Err(e) => log::warn!("開啟 computer 稽核紀錄失敗：{e}"),
        }
    }

    /// 最後 `n` 筆，新的在前。壞掉的行略過。
    pub fn tail(&self, n: usize) -> Vec<AuditEntry> {
        let Ok(text) = std::fs::read_to_string(&self.path) else { return vec![] };
        let mut v: Vec<AuditEntry> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        v.reverse();
        v.truncate(n);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_then_tail_returns_newest_first_and_skips_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("a.jsonl");
        let log = AuditLog::new(path.clone());
        assert!(log.tail(10).is_empty()); // 檔案不存在不是錯誤
        log.append("computer_click", "Notes", "Save", "ok");
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"not json\n").unwrap();
        log.append("computer_key", "Notes", "cmd+s", "denied");

        let got = log.tail(10);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].tool, "computer_key");
        assert_eq!(got[0].outcome, "denied");
        assert_eq!(got[1].detail, "Save");
        assert_eq!(log.tail(1).len(), 1);
    }
}
