//! 每張卡用了多少 token：從 Claude Code 自己寫的對話記錄算出來。
//!
//! 一則回覆在 JSONL 裡會拆成好幾行（每個內容區塊一行），每行都帶著同一份
//! `message.usage`（實測：908 則回覆、670 則多行、用量全部一致），所以依
//! `message.id` 只算一次。卡片自己派的子代理記在
//! `<session-id>/subagents/*.jsonl`，要一起加。
//!
//! 結案時算一次存成 `tasks/<id>/usage.json`：Claude Code 預設 30 天後會清掉
//! `~/.claude/projects` 裡的記錄，卡片資料夾裡的那份 `session.jsonl` 又只複製
//! 了主記錄、沒有子代理。執行中的卡片則即時從 `~/.claude/projects` 讀。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    /// API 呼叫次數（不重複的回覆數）。
    pub calls: u64,
}

impl TokenUsage {
    pub fn add(&mut self, o: &TokenUsage) {
        self.input += o.input;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
        self.output += o.output;
        self.calls += o.calls;
    }
}

/// 一份 JSONL 的用量。壞掉的行跳過；同一個 `message.id` 只算一次。
pub fn parse_jsonl(text: &str) -> TokenUsage {
    let mut seen = HashSet::new();
    let mut u = TokenUsage::default();
    for line in text.lines() {
        let Ok(rec) = serde_json::from_str::<serde_json::Value>(line.trim()) else { continue };
        if rec.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        let Some(usage) = rec.pointer("/message/usage") else { continue };
        if let Some(id) = rec.pointer("/message/id").and_then(|v| v.as_str()) {
            if !seen.insert(id.to_string()) {
                continue;
            }
        }
        let n = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        u.input += n("input_tokens");
        u.cache_write += n("cache_creation_input_tokens");
        u.cache_read += n("cache_read_input_tokens");
        u.output += n("output_tokens");
        u.calls += 1;
    }
    u
}

/// 主記錄＋同名資料夾底下 `subagents/*.jsonl` 的用量；主記錄不存在就是 `None`。
pub fn session_usage(main: &Path) -> Option<TokenUsage> {
    let mut total = file_usage(main)?;
    let sub = main.with_extension("").join("subagents");
    if let Ok(entries) = std::fs::read_dir(&sub) {
        for p in entries.filter_map(Result::ok).map(|e| e.path()) {
            if p.extension().is_some_and(|x| x == "jsonl") {
                if let Some(u) = file_usage(&p) {
                    total.add(&u);
                }
            }
        }
    }
    Some(total)
}

/// 讀一個檔案的用量；檔案沒變（修改時間與大小都一樣）就用上次的結果——
/// 畫面會一直重抓執行中卡片的用量，記錄檔動輒數 MB。
fn file_usage(path: &Path) -> Option<TokenUsage> {
    type Cache = Mutex<HashMap<PathBuf, (SystemTime, u64, TokenUsage)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let meta = std::fs::metadata(path).ok()?;
    let key = (meta.modified().ok()?, meta.len());
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some((m, l, u)) = cache.lock().ok()?.get(path) {
        if (*m, *l) == key {
            return Some(*u);
        }
    }
    let u = parse_jsonl(&std::fs::read_to_string(path).ok()?);
    if let Ok(mut c) = cache.lock() {
        c.insert(path.to_path_buf(), (key.0, key.1, u));
    }
    Some(u)
}

const USAGE_FILE: &str = "usage.json";

/// 結案時把用量存進卡片資料夾。失敗只記 log。
pub fn save(project_path: &Path, task_id: &str, u: &TokenUsage) {
    let dir = crate::tasks::task_dir(project_path, task_id);
    let res = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(dir.join(USAGE_FILE), serde_json::to_string(u).unwrap_or_default()));
    if let Err(e) = res {
        log::warn!("save usage {task_id}: {e}");
    }
}

pub fn load(project_path: &Path, task_id: &str) -> Option<TokenUsage> {
    let text = std::fs::read_to_string(crate::tasks::task_dir(project_path, task_id).join(USAGE_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// 一張卡目前的用量：存好的 `usage.json` → 執行中就讀 `~/.claude/projects`
/// 裡的即時記錄 → 舊卡片退回卡片資料夾裡的 `session.jsonl`（沒有子代理）。
pub fn task_usage(
    project_path: &Path,
    projects_root: Option<&Path>,
    task_id: &str,
    session_id: Option<&str>,
    session_path: Option<&str>,
) -> Option<TokenUsage> {
    if let Some(u) = load(project_path, task_id) {
        return Some(u);
    }
    if let (Some(root), Some(sid)) = (projects_root, session_id) {
        if let Some(main) = crate::tasks::session_log::find_session_log(root, sid) {
            return session_usage(&main);
        }
    }
    let stored = session_path?;
    session_usage(&crate::tasks::resolve_stored_path(project_path, stored))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, input: u64, cw: u64, cr: u64, out: u64) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}","usage":{{"input_tokens":{input},"cache_creation_input_tokens":{cw},"cache_read_input_tokens":{cr},"output_tokens":{out}}}}}}}"#
        )
    }

    #[test]
    fn a_reply_split_over_several_lines_is_counted_once() {
        let text = [
            line("m1", 3, 100, 1000, 7),
            line("m1", 3, 100, 1000, 7),
            r#"{"type":"user","message":{"content":"hi"}}"#.to_string(),
            "not json".to_string(),
            line("m2", 1, 0, 2000, 5),
        ]
        .join("\n");
        assert_eq!(
            parse_jsonl(&text),
            TokenUsage { input: 4, cache_write: 100, cache_read: 3000, output: 12, calls: 2 }
        );
    }

    #[test]
    fn subagent_transcripts_are_added_to_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("sid.jsonl");
        std::fs::write(&main, line("m1", 1, 0, 0, 1)).unwrap();
        let sub = dir.path().join("sid").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("agent-a.jsonl"), line("s1", 10, 0, 0, 10)).unwrap();
        std::fs::write(sub.join("agent-a.meta.json"), line("x", 999, 0, 0, 999)).unwrap();
        let u = session_usage(&main).unwrap();
        assert_eq!((u.input, u.output, u.calls), (11, 11, 2));
        assert_eq!(session_usage(&dir.path().join("missing.jsonl")), None);
    }

    #[test]
    fn a_saved_usage_wins_then_the_live_log_then_the_copied_log() {
        let project = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let encoded = root.path().join("-some-dir");
        std::fs::create_dir_all(&encoded).unwrap();
        std::fs::write(encoded.join("sid.jsonl"), line("live", 5, 0, 0, 5)).unwrap();
        let copied = crate::tasks::task_dir(project.path(), "t").join("session.jsonl");
        std::fs::create_dir_all(copied.parent().unwrap()).unwrap();
        std::fs::write(&copied, line("copy", 2, 0, 0, 2)).unwrap();
        let copied_s = copied.to_string_lossy().into_owned();

        let get = || task_usage(project.path(), Some(root.path()), "t", Some("sid"), Some(&copied_s)).map(|u| u.input);
        assert_eq!(get(), Some(5), "還沒存 usage.json：讀即時記錄");
        std::fs::remove_file(encoded.join("sid.jsonl")).unwrap();
        assert_eq!(get(), Some(2), "即時記錄被清掉：退回卡片資料夾裡的副本");
        save(project.path(), "t", &TokenUsage { input: 9, ..Default::default() });
        assert_eq!(get(), Some(9), "存好的用量優先");
        assert_eq!(task_usage(project.path(), Some(root.path()), "other", None, None), None);
    }
}
