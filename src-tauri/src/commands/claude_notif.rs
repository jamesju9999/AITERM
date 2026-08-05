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
        .map_err(|e| format!("failed to read {}: {e}", settings_path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!("{} does not have a JSON object at its top level", settings_path.display())),
        Err(e) => Err(format!("{} is not valid JSON: {e}", settings_path.display())),
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
/// 檔案不存在就建立——上層目錄不用另外建：`needs_prompt_at` 在卡片出現前
/// 就已經確認過 parent 是目錄，這裡不會遇到目錄不存在的情況。JSON 壞掉時
/// 回 Err 而且**不寫入**：使用者的設定檔壞掉是他自己要處理的事，不是我們
/// 拿來重置的理由。
///
/// 寫入採「先寫暫存檔、成功了才 rename 過去」，避免磁碟空間不足或其他寫入
/// 失敗把使用者原本的內容清空（`fs::write` 是先 truncate 再寫，寫到一半失敗
/// 就會留下空檔案或半份 JSON）。rename 前先解開符號連結、複製原檔權限：
/// dotfile 管理工具（chezmoi/stow/yadm）常把 settings.json 做成連結，且原檔
/// 可能刻意設成 0600，兩者都不該被這次寫入動到。
pub(crate) fn enable_bell_at(settings_path: &Path) -> Result<(), String> {
    let mut map = read_object(settings_path)?;
    map.insert(KEY.to_string(), Value::String(BELL.to_string()));

    let text = serde_json::to_string_pretty(&Value::Object(map))
        .map_err(|e| format!("failed to serialize settings: {e}"))?
        + "\n";

    let target = fs::canonicalize(settings_path).unwrap_or_else(|_| settings_path.to_path_buf());
    let tmp = target.with_extension("json.aiterm-tmp");
    fs::write(&tmp, &text).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("failed to write temp file: {e}")
    })?;
    if let Ok(meta) = fs::metadata(&target) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("failed to write {}: {e}", target.display())
    })
}

#[tauri::command]
pub fn claude_notif_needs_prompt() -> bool {
    claude_settings_path().map(|p| needs_prompt_at(&p)).unwrap_or(false)
}

#[tauri::command]
pub fn claude_notif_enable_bell() -> Result<(), String> {
    let path = claude_settings_path().ok_or_else(|| "could not resolve the user's home directory".to_string())?;
    enable_bell_at(&path)
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
    fn empty_settings_file_asks() {
        let path = claude_dir_with(Some(""));
        assert!(needs_prompt_at(&path));
    }

    #[test]
    fn enable_bell_fills_in_an_empty_file() {
        let path = claude_dir_with(Some("   \n"));
        enable_bell_at(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""preferredNotifChannel": "terminal_bell""#));
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

    #[test]
    fn write_ends_with_a_trailing_newline() {
        let path = claude_dir_with(Some(r#"{"model":"sonnet"}"#));
        enable_bell_at(&path).unwrap();
        assert!(fs::read_to_string(&path).unwrap().ends_with("}\n"));
    }

    #[cfg(unix)]
    #[test]
    fn enable_bell_at_preserves_a_symlinked_settings_file() {
        // dotfile 管理工具（chezmoi/stow/yadm）常把 settings.json 做成連結到別處
        // 的真正檔案。寫入時如果直接對連結路徑操作（沒先 canonicalize），
        // rename 會把連結整個換成一份普通檔案，砍斷它跟原始檔案的關係。
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let real = dir.path().join("real-settings.json");
        fs::write(&real, r#"{"model":"sonnet"}"#).unwrap();
        let link = dir.path().join("settings.json");
        symlink(&real, &link).unwrap();

        enable_bell_at(&link).unwrap();

        // 連結本身還是連結，沒有被 rename 換成普通檔案。
        let link_meta = fs::symlink_metadata(&link).unwrap();
        assert!(link_meta.file_type().is_symlink());

        // 連結指向的真正檔案內容有更新。
        let text = fs::read_to_string(&real).unwrap();
        assert!(text.contains(r#""preferredNotifChannel": "terminal_bell""#));
    }

    #[cfg(unix)]
    #[test]
    fn enable_bell_at_preserves_file_permissions() {
        // 使用者可能刻意把 settings.json 設成 0600（其他 dotfile 管理慣例），
        // 這次寫入不該把它放寬。
        use std::os::unix::fs::PermissionsExt;

        let path = claude_dir_with(Some(r#"{"model":"sonnet"}"#));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        enable_bell_at(&path).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn write_failure_leaves_original_untouched() {
        // 目的是證明 temp+rename 真的有擋住「寫到一半失敗」把原檔清空的問題
        // （這正是 fs::write 直接 truncate 原檔會犯的錯）。用跨平台可控的方式
        // 讓寫入真的失敗很難——這裡用拿掉目錄寫入權限來逼暫存檔寫不進去，
        // 是 Unix-only 的權限模型，Windows 沒有對等的可攜作法，所以不跑。
        use std::os::unix::fs::PermissionsExt;

        let original = r#"{"model":"sonnet"}"#;
        let path = claude_dir_with(Some(original));
        let dir = path.parent().unwrap();
        fs::set_permissions(dir, fs::Permissions::from_mode(0o500)).unwrap();

        let result = enable_bell_at(&path);

        // 先復原權限，才能讀檔驗證、也才能讓 tempdir 之後清得掉。
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
}
