//! 從 subagent 的 transcript 算 token 數。
//!
//! 互動模式的 Agent 工具是非同步的：`PostToolUse` 只回 `async_launched`，沒有
//! `totalTokens`。`SubagentStop` 帶 `agent_transcript_path`，這裡讀它。
//!
//! 取**最後一次模型呼叫**的 input + cache_read + cache_creation + output（也就是結束時的
//! 脈絡大小，跟 Claude 自己回報的 `totalTokens` 同一個量級：實測 20298 對 20484）。
//! 不是各輪累加——快取讀取每輪都會重算，累加會大好幾倍。

use std::path::Path;

use serde_json::Value;

/// 太大的 transcript 不讀（本機隨便一個路徑也不該讓接收器吃掉大量記憶體）。
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// 只接受 Claude Code 放在 `.../subagents/agent-*.jsonl` 的檔案：路徑是 hook 酬載給的，
/// 雖然接收器有密鑰保護，仍不讀任意路徑。
pub fn is_subagent_transcript(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "jsonl")
        && path.parent().and_then(Path::file_name).is_some_and(|d| d == "subagents")
}

/// 最後一個 assistant 訊息的用量加總；讀不到或沒有 assistant 訊息回 `None`。
pub fn transcript_tokens(path: &Path) -> Option<u64> {
    if !is_subagent_transcript(path) || std::fs::metadata(path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let usage = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v.get("type").and_then(Value::as_str) == Some("assistant"))
        .filter_map(|v| v.pointer("/message/usage").cloned())
        .last()?;
    let sum = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens"]
        .iter()
        .filter_map(|k| usage.get(*k).and_then(Value::as_u64))
        .sum();
    Some(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aiterm-cact-tr-{}", uuid::Uuid::new_v4())).join("subagents");
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sums_the_last_assistant_message_only() {
        let f = dir().join("agent-a1.jsonl");
        std::fs::write(
            &f,
            [
                r#"{"type":"user","message":{"content":"hi"}}"#,
                r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":16967,"output_tokens":8}}}"#,
                r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":16967,"output_tokens":8}}}"#,
                r#"{"type":"attachment"}"#,
                r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":2,"cache_read_input_tokens":16967,"cache_creation_input_tokens":3326,"output_tokens":3}}}"#,
                "not json at all",
            ]
            .join("\n"),
        )
        .unwrap();
        assert_eq!(transcript_tokens(&f), Some(2 + 16967 + 3326 + 3));
    }

    #[test]
    fn missing_fields_count_as_zero_and_no_assistant_is_none() {
        let d = dir();
        let a = d.join("agent-a.jsonl");
        std::fs::write(&a, r#"{"type":"assistant","message":{"usage":{"output_tokens":7}}}"#).unwrap();
        assert_eq!(transcript_tokens(&a), Some(7));
        let b = d.join("agent-b.jsonl");
        std::fs::write(&b, r#"{"type":"user"}"#).unwrap();
        assert_eq!(transcript_tokens(&b), None);
        assert_eq!(transcript_tokens(&d.join("agent-gone.jsonl")), None);
    }

    #[test]
    fn only_subagent_transcripts_are_read() {
        let d = dir();
        let elsewhere = d.parent().unwrap().join("agent-x.jsonl");
        std::fs::write(&elsewhere, r#"{"type":"assistant","message":{"usage":{"output_tokens":1}}}"#).unwrap();
        assert_eq!(transcript_tokens(&elsewhere), None, "不在 subagents/ 底下");
        let wrong_ext = d.join("agent-x.txt");
        std::fs::write(&wrong_ext, "x").unwrap();
        assert_eq!(transcript_tokens(&wrong_ext), None, "不是 .jsonl");
        assert!(is_subagent_transcript(&d.join("agent-ok.jsonl")));
    }
}
