//! 建團隊前的「方法討論」：開一個真的 Claude Code session（工作目錄是程式庫、參考資料夾用 `--add-dir`
//! 掛進去），使用者在那裡跟 AI 討論；AI 把做法決定寫進 `<專案>/tasks/discussion-<id>/decision.json`，
//! 畫面讀到後顯示決定卡，使用者確認才建團隊。這裡只放路徑與提示詞的純函式，開分頁走 `dispatch::spawn_and_run`。

use std::path::{Path, PathBuf};

use crate::tasks::references::Reference;

/// 前端提示詞裡的占位（見 `methodDecision.ts` 的 `DECISION_PATH_PLACEHOLDER`），啟動時換成真實路徑。
pub const PATH_PLACEHOLDER: &str = "{{DECISION_PATH}}";

/// 討論用的資料夾：跟卡片的 `tasks/<id>/` 同一層，專案搬家時一起帶走。
pub fn discussion_dir(project_path: &Path, id: &str) -> PathBuf {
    crate::tasks::task_dir(project_path, &format!("discussion-{id}"))
}

pub fn decision_path(project_path: &Path, id: &str) -> PathBuf {
    discussion_dir(project_path, id).join("decision.json")
}

/// id 只能是字母數字與連字號：它會被拿來組路徑，不能讓呼叫端用 `../` 讀到專案外。
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn fill_prompt(template: &str, decision_path: &Path) -> String {
    template.replace(PATH_PLACEHOLDER, &decision_path.to_string_lossy())
}

/// 決定檔大小上限：決定只有幾行 JSON，超過就當作不是（資料夾裡可能被放進符號連結或大檔）。
const MAX_DECISION_BYTES: usize = 256 * 1024;

/// 決定檔的文字；還沒有、太大、或 id 不合法就是 `None`。解析與驗證交給前端（檔案可能寫到一半）。
pub fn read_decision(project_path: &Path, id: &str) -> Option<String> {
    if !valid_id(id) {
        return None;
    }
    let path = decision_path(project_path, id);
    if std::fs::metadata(&path).ok()?.len() > MAX_DECISION_BYTES as u64 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// 討論 session 的設定檔：禁止用內建工具編輯參考資料夾（跟派工同一條規則）。
pub fn settings_json(refs: &[Reference]) -> String {
    crate::tasks::references::settings_with_deny("{}", &crate::tasks::references::deny_edit_rules(refs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_decision_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let id = "abc-123";
        std::fs::create_dir_all(discussion_dir(dir.path(), id)).unwrap();
        std::fs::write(decision_path(dir.path(), id), "x".repeat(MAX_DECISION_BYTES + 1)).unwrap();
        assert_eq!(read_decision(dir.path(), id), None, "太大的檔案不讀（決定只有幾行 JSON）");
        std::fs::write(decision_path(dir.path(), id), "{}").unwrap();
        assert_eq!(read_decision(dir.path(), id).as_deref(), Some("{}"));
    }

    #[test]
    fn decision_path_lives_under_the_project_tasks_dir() {
        let p = decision_path(Path::new("/proj"), "abc-123");
        assert_eq!(p, Path::new("/proj").join("tasks").join("discussion-abc-123").join("decision.json"));
    }

    #[test]
    fn valid_id_accepts_uuid_and_rejects_path_tricks() {
        assert!(valid_id("0b9f6d2e-1c3a-4f55-9a7e-2d1e8c3b4a5f"));
        for bad in ["", "../x", "a/b", "a\\b", "a b", "x.json", &"a".repeat(65)] {
            assert!(!valid_id(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn fill_prompt_replaces_every_placeholder() {
        let out = fill_prompt("寫到 {{DECISION_PATH}} 再說 {{DECISION_PATH}}", Path::new("/p/d.json"));
        assert_eq!(out, "寫到 /p/d.json 再說 /p/d.json");
    }

    #[test]
    fn read_decision_is_none_until_the_file_exists_and_rejects_bad_ids() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_decision(dir.path(), "abc"), None);
        let path = decision_path(dir.path(), "abc");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{\"approach\":\"single\"}").unwrap();
        assert_eq!(read_decision(dir.path(), "abc").as_deref(), Some("{\"approach\":\"single\"}"));
        assert_eq!(read_decision(dir.path(), "../abc"), None);
    }

    #[test]
    fn settings_json_denies_edits_in_reference_dirs() {
        let refs = vec![crate::tasks::references::Reference { path: "/old/src".into(), note: String::new() }];
        let s = settings_json(&refs);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["permissions"]["deny"][0], "Edit(//old/src/**)");
    }
}
