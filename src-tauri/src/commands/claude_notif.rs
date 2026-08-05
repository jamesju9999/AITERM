//! 判斷並設定 Claude Code 的 terminal bell 通知channel。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

const KEY: &str = "preferredNotifChannel";
const BELL: &str = "terminal_bell";

/// `~/.claude/settings.json`。取不到 home 目錄時回 None。
fn claude_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

/// 讀成頂層 JSON 物件。檔案不存在或是空的都算「還沒有設定」→ 空物件。
/// 解析失敗或根不是物件都回 Err——呼叫端各自決定要怎麼處理。
fn read_object(settings_path: &Path) -> Result<Map<String, Value>, String> {
    if !settings_path.exists() {
        return Ok(Map::new());
    }
    let text = fs::read_to_string(settings_path)
        .map_err(|e| format!("讀取 {} 失敗：{e}", settings_path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!("{} 的最外層不是 JSON 物件", settings_path.display())),
        Err(e) => Err(format!("{} 不是合法的 JSON：{e}", settings_path.display())),
    }
}

/// 要不要提示使用者設定 terminal bell？
///
/// 條件：`~/.claude/` 存在，且 `preferredNotifChannel` 不存在或等於 "auto"。
/// 任何其他明確的值都代表使用者已經做過決定，不打擾。解析不了的檔案一律
/// 回 false——看不懂的東西就不要碰。
pub(crate) fn needs_prompt_at(settings_path: &Path) -> bool {
    match settings_path.parent() {
        Some(dir) if dir.is_dir() => {}
        _ => return false,
    }
    let Ok(map) = read_object(settings_path) else { return false };
    match map.get(KEY) {
        None => true,
        Some(Value::String(s)) => s == "auto",
        Some(_) => false,
    }
}

/// 把 `preferredNotifChannel` 設成 `terminal_bell`，其餘內容原樣保留。
///
/// 檔案不存在就建立（含上層目錄）。JSON 壞掉時回 Err 而且**不寫入**：
/// 使用者的設定檔壞掉是他自己要處理的事，不是我們拿來重置的理由。
pub(crate) fn enable_bell_at(settings_path: &Path) -> Result<(), String> {
    let mut map = read_object(settings_path)?;
    map.insert(KEY.to_string(), Value::String(BELL.to_string()));

    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("建立 {} 失敗：{e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(map))
        .map_err(|e| format!("序列化設定失敗：{e}"))?;
    fs::write(settings_path, text + "\n")
        .map_err(|e| format!("寫入 {} 失敗：{e}", settings_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// 建一個假的 ~/.claude 目錄，回傳 settings.json 的路徑。
    /// 目錄本身要活到測試結束，所以 leak 掉——測試裡可以接受，
    /// 與 config/mod.rs 既有的 temp_store() 同樣做法。
    fn claude_dir_with(contents: Option<&str>) -> std::path::PathBuf {
        let dir = tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        let path = claude.join("settings.json");
        if let Some(text) = contents {
            fs::write(&path, text).unwrap();
        }
        std::mem::forget(dir);
        path
    }

    #[test]
    fn no_claude_dir_means_no_prompt() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".claude").join("settings.json");
        assert!(!needs_prompt_at(&path));
    }

    #[test]
    fn missing_settings_file_asks() {
        let path = claude_dir_with(None);
        assert!(needs_prompt_at(&path));
    }

    #[test]
    fn absent_key_asks() {
        let path = claude_dir_with(Some(r#"{"model":"sonnet"}"#));
        assert!(needs_prompt_at(&path));
    }

    #[test]
    fn auto_asks() {
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"auto"}"#));
        assert!(needs_prompt_at(&path));
    }

    #[test]
    fn already_terminal_bell_does_not_ask() {
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"terminal_bell"}"#));
        assert!(!needs_prompt_at(&path));
    }

    #[test]
    fn explicit_other_channel_does_not_ask() {
        // 使用者已經為別的終端機做過決定，不要打擾。
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"iterm2"}"#));
        assert!(!needs_prompt_at(&path));
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"notifications_disabled"}"#));
        assert!(!needs_prompt_at(&path));
    }

    #[test]
    fn broken_json_does_not_ask() {
        // 看不懂的檔案就不要碰。
        let path = claude_dir_with(Some("{ this is not json"));
        assert!(!needs_prompt_at(&path));
    }

    #[test]
    fn creates_file_when_missing() {
        let path = claude_dir_with(None);
        enable_bell_at(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""preferredNotifChannel": "terminal_bell""#));
    }

    #[test]
    fn preserves_existing_keys_and_their_order() {
        let original = "{\n  \"zeta\": 1,\n  \"alpha\": 2,\n  \"middle\": {\n    \"nested\": true\n  }\n}\n";
        let path = claude_dir_with(Some(original));
        enable_bell_at(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        // 原本的 key 一個都不能少，值也不能變。
        assert!(text.contains("\"zeta\": 1"));
        assert!(text.contains("\"alpha\": 2"));
        assert!(text.contains("\"nested\": true"));
        // 順序不變：zeta 仍在 alpha 前面（沒有被字母重排）。
        let zeta = text.find("zeta").unwrap();
        let alpha = text.find("alpha").unwrap();
        assert!(zeta < alpha, "key 順序被重排了：{text}");
        // 新的 key 附加在最後。
        assert!(text.find("preferredNotifChannel").unwrap() > alpha);
    }

    #[test]
    fn overwrites_auto() {
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"auto"}"#));
        enable_bell_at(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("terminal_bell"));
        assert!(!text.contains("\"auto\""));
    }

    #[test]
    fn broken_json_errors_and_leaves_file_untouched() {
        // 使用者的設定檔壞掉是他自己要處理的事，不是我們拿來重置的理由。
        let original = "{ this is not json";
        let path = claude_dir_with(Some(original));
        assert!(enable_bell_at(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn non_object_root_errors_and_leaves_file_untouched() {
        let original = "[1, 2, 3]";
        let path = claude_dir_with(Some(original));
        assert!(enable_bell_at(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
}
