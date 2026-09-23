//! Claude Code 自己寫的 session 記錄：找到它、複製它、渲染它。
//!
//! Claude Code 每一輪 user / assistant / 工具呼叫都會即時寫進
//! `~/.claude/projects/<某個資料夾>/<session-id>.jsonl`。派工時我們用
//! `--session-id` 指定那個 UUID v4，所以任務結束後只要在 `~/.claude/projects`
//! 底下掃一輪、找檔名對得上的那個檔案就好——不需要知道、也不需要重建
//! Claude Code 把它排進哪個資料夾的編碼規則。掃描法對 canonicalize、符號
//! 連結（macOS `/private/tmp`）、Windows `canonicalize()` 帶 `\\?\` verbatim
//! 前綴這些會讓「猜資料夾名」失準的坑全部免疫，因為根本不需要猜。
//!
//! 這個模組除了最外層的 `copy_session_log` 之外全是純函式（吃字串、吐
//! 字串），與檔案系統和 Tauri state 無關。
//!
//! 見 docs/superpowers/specs/2026-09-07-full-task-transcript-design.md。

use std::path::{Path, PathBuf};

/// 單一區塊（一個工具參數、一筆工具回傳）的長度上限，單位是字元不是位元組
/// ——內容常常是中文。超過的部分截掉並標出截了多少，見 `truncate_chars`。
///
/// 上限只是為了不讓一個 500 行的 `Write` 或一份整檔 `cat` 撐爆記錄（工作報告
/// 也是拿這份文字去餵模型）；原始資料完整躺在 `session.jsonl`，要調整隨時
/// 可以，丟掉的救不回來，所以寧可放寬。
const BLOCK_MAX_CHARS: usize = 2000;

/// 把 Claude Code 的 session JSONL 渲染成逐輪對話純文字。
///
/// 只處理 `user` / `assistant` 兩種記錄，其餘一律跳過；`isMeta: true` 的
/// 記錄也跳過——那是 Claude Code 自己塞進對話的系統訊息（斜線指令的 caveat、
/// session 命名提示），不是使用者說的話。思考過程跳過（不是「做了什麼」，
/// 實測常常是空的）。
///
/// 工具呼叫與工具回傳都留：呼叫回答「它做了什麼」，回傳回答「結果如何」。
/// 兩者都受 `BLOCK_MAX_CHARS` 限制，截斷會標明。
///
/// 單行 JSON 解析失敗就跳過那一行，不中止整份渲染——一行壞掉不該讓整份
/// 記錄消失。
pub fn render_session_log(jsonl: &str) -> String {
    let mut out = String::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if rec.get("isMeta").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        let speaker = match rec.get("type").and_then(|v| v.as_str()) {
            Some("user") => "使用者",
            Some("assistant") => "Claude",
            _ => continue,
        };
        let Some(content) = rec.pointer("/message/content") else {
            continue;
        };
        if let Some(text) = content.as_str() {
            push_line(&mut out, &format!("{speaker}：{text}"));
            continue;
        }
        // `user` 記錄實測會有 2–3 個 block，所以走完整個陣列。
        for block in content.as_array().into_iter().flatten() {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        push_line(&mut out, &format!("{speaker}：{text}"));
                    }
                }
                Some("tool_use") => render_tool_use(&mut out, block),
                Some("tool_result") => render_tool_result(&mut out, block),
                // thinking / 其他一律跳過。
                _ => {}
            }
        }
    }
    out
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

/// 多行內容的第二行起縮排兩格，這樣內容裡剛好以 `使用者：` 或 `〔工具〕`
/// 開頭的行不會被誤認成獨立的一筆記錄。
fn indent_continuation(text: &str) -> String {
    text.replace('\n', "\n  ")
}

/// 超過 `max` 個字元就截斷，並在尾端標出截掉幾個字元。
///
/// 按字元算，不能用 `&s[..max]`——中文每字 3 個位元組，切在字元中間會 panic。
fn truncate_chars(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max).collect();
    format!("{kept}…（已截斷 {} 字）", total - max)
}

/// `〔工具〕<name>`，後面每個參數一行 `  key: value`。
///
/// 「每個」指 JSONL 裡的書寫順序：`serde_json` 開了 `preserve_order`（見
/// Cargo.toml），所以物件的鍵序就是原始順序，不是字典序。過去只取第一個
/// 參數，`Edit` 的第一個參數是 `replace_all`，記錄裡就只剩 `Edit false`。
fn render_tool_use(out: &mut String, block: &serde_json::Value) {
    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("?");
    push_line(out, &format!("〔工具〕{name}"));
    let Some(input) = block.get("input").and_then(|v| v.as_object()) else {
        return;
    };
    for (key, value) in input {
        let raw = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let text = indent_continuation(&truncate_chars(&raw, BLOCK_MAX_CHARS));
        push_line(out, &format!("  {key}: {text}"));
    }
}

/// `〔結果〕<內容>`；失敗的呼叫標成 `〔結果·失敗〕`。
///
/// `content` 有兩種形態：字串，或 `[{type:"text"}, {type:"image"}, …]`。
/// 圖片放不進純文字，但要標出「這裡有一張」，不能無聲消失。內容是空的
/// 也留一行——那證明呼叫確實跑完了。
fn render_tool_result(out: &mut String, block: &serde_json::Value) {
    let label = if block.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
        "〔結果·失敗〕"
    } else {
        "〔結果〕"
    };
    let text = match block.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| match p.get("type").and_then(|v| v.as_str()) {
                Some("text") => p.get("text").and_then(|v| v.as_str()).map(str::to_string),
                Some("image") => Some("［圖片］".to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    if text.trim().is_empty() {
        push_line(out, &format!("{label}（無輸出）"));
        return;
    }
    let text = indent_continuation(&truncate_chars(&text, BLOCK_MAX_CHARS));
    push_line(out, &format!("{label}{text}"));
}

/// `~/.claude/projects` — Claude Code 放 session 記錄的地方。
/// 家目錄取不到時回 `None`，呼叫端安靜退回 transcript.txt。
pub fn claude_projects_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

/// 在 `projects_root` 底下掃過每個子資料夾，找檔名是 `<session_id>.jsonl`
/// 的那個檔案。`session_id` 是派工時自己生的 UUID v4，檔名對得上就是它，
/// 不會撞名——不需要知道、也不需要重建 Claude Code 把它排進哪個資料夾的
/// 編碼規則。
fn find_session_log(projects_root: &Path, session_id: &str) -> Option<PathBuf> {
    let filename = format!("{session_id}.jsonl");
    std::fs::read_dir(projects_root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(&filename))
        .find(|p| p.is_file())
}

/// 把 `<projects_root>` 底下找到的 `<session_id>.jsonl` **原封不動**
/// 複製成 `<project_path>/tasks/<task_id>/session.jsonl`，回傳目的地路徑。
///
/// 不精簡是刻意的：一次派工只有幾十 KB 到數 MB，磁碟成本遠低於提早丟資料
/// 的風險。要精簡永遠來得及，丟掉的救不回來。
///
/// 找不到來源、或複製失敗（權限、磁碟），都安靜回 `None` 並寫進 log——
/// 原本的 `transcript.txt` 還在，沒有東西壞掉，不值得打斷使用者。
pub fn copy_session_log(
    projects_root: &Path,
    session_id: &str,
    project_path: &Path,
    task_id: &str,
) -> Option<String> {
    let Some(src) = find_session_log(projects_root, session_id) else {
        // 最常發生的失敗就是這一條（claude_command 不是 claude、旗標沒生效，
        // 所以 --session-id 從沒生效，根本沒有這個檔案）——原本靜悄悄地回
        // None。
        log::warn!("session log {session_id}.jsonl not found under {projects_root:?}");
        return None;
    };

    let dir = crate::tasks::task_dir(project_path, task_id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!("session log dir {dir:?}: {e}");
        return None;
    }
    let dest = dir.join("session.jsonl");
    match std::fs::copy(&src, &dest) {
        Ok(_) => Some(dest.to_string_lossy().into_owned()),
        Err(e) => {
            log::error!("copy session log {src:?} → {dest:?}: {e}");
            None
        }
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    /// 真實 session 檔抽出來的五種記錄（長字串已截短，結構未動），外加
    /// 三種必須被跳過的雜訊記錄、一行壞掉的 JSON 與一行空行。
    ///
    /// 壞掉那行刻意放在 tool_use 與 tool_result 之間，而不是檔尾：後面
    /// 還有一筆會被渲染的 `text` 記錄，這樣「整份中止」與「只跳這行」
    /// 才會產出不同結果，見 `a_broken_line_does_not_abort_the_whole_render`。
    const REAL_FIXTURE: &str = r#"{"type": "user", "message": {"role": "user", "content": "請繼續"}}
{"type": "assistant", "message": {"role": "assistant", "content": [{"type": "thinking", "thinking": "先看 repo 狀態", "signature": "SIG"}]}, "isSidechain": false}
{"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_01RZV1vshph15jPaL7ozx2an", "name": "Bash", "input": {"command": "git status --short | head -30", "description": "Check repo state"}}]}, "isSidechain": false}
{this is not valid json
{"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_01N7g4oADgBWEqZaM4GUcHtA", "content": "No matching deferred tools found"}]}}
{"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "工作目錄乾淨，沒有未推的 commit。"}]}, "isSidechain": false}
{"type": "mode", "uuid": "x"}
{"type": "attachment", "uuid": "ca05c03c-7993-4ec6-baf4-c4e3b71e64e3"}
{"type": "file-history-snapshot", "uuid": "x"}

"#;

    /// 單一 `assistant` 工具呼叫記錄。`input_json` 是 `input` 物件的原始 JSON。
    fn tool_use_record(name: &str, input_json: &str) -> String {
        format!(
            r#"{{"type": "assistant", "message": {{"role": "assistant", "content": [{{"type": "tool_use", "id": "t", "name": "{name}", "input": {input_json}}}]}}}}"#
        )
    }

    /// 單一 `user` 工具回傳記錄。`content_json` 是 `content` 欄位的原始 JSON。
    fn tool_result_record(content_json: &str, is_error: bool) -> String {
        format!(
            r#"{{"type": "user", "message": {{"role": "user", "content": [{{"type": "tool_result", "tool_use_id": "t", "content": {content_json}, "is_error": {is_error}}}]}}}}"#
        )
    }

    #[test]
    fn renders_each_turn_in_order() {
        let out = render_session_log(REAL_FIXTURE);
        let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(
            lines,
            vec![
                "使用者：請繼續",
                "〔工具〕Bash",
                "  command: git status --short | head -30",
                "  description: Check repo state",
                "〔結果〕No matching deferred tools found",
                "Claude：工作目錄乾淨，沒有未推的 commit。",
            ]
        );
    }

    /// 思考過程仍然不收：它不是「做了什麼」，而且實測常常是空的（被遮蔽）。
    /// 工具回傳則改成要收——見 `tool_results_are_kept`。
    #[test]
    fn thinking_is_still_dropped() {
        let out = render_session_log(REAL_FIXTURE);
        assert!(!out.contains("先看 repo 狀態"), "思考過程沒有被丟掉：{out}");
    }

    /// 一行壞掉不該讓整份記錄消失。
    ///
    /// 壞掉那行的**位置**是這個測試的全部價值所在：它必須夾在可渲染的
    /// 記錄中間，後面還有東西。放在檔尾的話，`break`（整份中止）與
    /// `continue`（只跳這行）產出完全一樣，測試就抓不到前者了。
    #[test]
    fn a_broken_line_does_not_abort_the_whole_render() {
        let out = render_session_log(REAL_FIXTURE);
        assert!(out.contains("使用者：請繼續"), "壞行之前的內容不見了：{out}");
        assert!(out.contains("Claude：工作目錄乾淨"), "壞行之後的內容不見了：{out}");
    }

    /// 實測：`user` 記錄的 content 陣列有 2 個、3 個 block 的情況。三個
    /// block 都是會被渲染的種類，而且順序是 text → tool_result → text：
    /// 「只讀第一個」「只讀最後一個」「只讀 text」三種錯誤實作各會漏掉
    /// 一句，測不出「走完整個陣列、照原順序」的話這條就是空的。
    #[test]
    fn walks_every_block_in_a_multi_block_record() {
        let jsonl = r#"{"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "第一段"}, {"type": "tool_result", "tool_use_id": "t1", "content": "中間的回傳"}, {"type": "text", "text": "最後一段"}]}}"#;
        let out = render_session_log(jsonl);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, vec!["使用者：第一段", "〔結果〕中間的回傳", "使用者：最後一段"]);
    }

    /// 工具回傳要收：沒有它，記錄只剩「呼叫了什麼」，看不出結果如何。
    /// 兩種 content 形態都要能處理——字串，與 `[{type:"text"}]` 陣列。
    #[test]
    fn tool_results_are_kept() {
        let as_string = render_session_log(&tool_result_record(r#""exit 0""#, false));
        assert_eq!(as_string.trim(), "〔結果〕exit 0");

        let as_blocks = render_session_log(&tool_result_record(
            r#"[{"type": "text", "text": "第一塊"}, {"type": "text", "text": "第二塊"}]"#,
            false,
        ));
        assert_eq!(as_blocks.trim(), "〔結果〕第一塊\n  第二塊");
    }

    /// 失敗的工具呼叫要看得出來是失敗，否則「Claude 接著換了個做法」
    /// 在記錄裡毫無來由。
    #[test]
    fn a_failed_tool_result_is_marked() {
        let out = render_session_log(&tool_result_record(r#""command not found""#, true));
        assert_eq!(out.trim(), "〔結果·失敗〕command not found");
    }

    /// 沒有輸出的工具回傳仍然要留一行：它證明那次呼叫確實跑完了。
    #[test]
    fn an_empty_tool_result_still_leaves_a_line() {
        let out = render_session_log(&tool_result_record(r#""""#, false));
        assert_eq!(out.trim(), "〔結果〕（無輸出）");
    }

    /// 回傳裡的圖片沒辦法放進純文字，但要標出「這裡有一張圖」。
    #[test]
    fn an_image_in_a_tool_result_is_marked_not_dropped_silently() {
        let out = render_session_log(&tool_result_record(
            r#"[{"type": "image", "source": {"type": "base64", "data": "AAAA"}}]"#,
            false,
        ));
        assert_eq!(out.trim(), "〔結果〕［圖片］");
        assert!(!out.contains("AAAA"), "把 base64 內容印出來了：{out}");
    }

    /// 過去只取「第一個參數」：`Edit` 的第一個參數是 `replace_all`，於是
    /// 記錄裡只有一行 `〔工具〕Edit false`，完全看不出改了哪個檔案、改了什麼
    /// （Windows 實機記錄裡真的出現過）。現在每個參數都要出現，且照
    /// JSONL 裡的書寫順序。
    #[test]
    fn a_tool_call_shows_every_argument_in_written_order() {
        let out = render_session_log(&tool_use_record(
            "Edit",
            r#"{"replace_all": false, "file_path": "/repo/a.rs", "old_string": "foo", "new_string": "bar"}"#,
        ));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines,
            vec![
                "〔工具〕Edit",
                "  replace_all: false",
                "  file_path: /repo/a.rs",
                "  old_string: foo",
                "  new_string: bar",
            ]
        );
    }

    /// 多行參數（`Edit` 的 old_string / new_string、`Write` 的 content 幾乎
    /// 都是多行）每一行都要留著，而且縮排，才不會被誤認成獨立的一行記錄——
    /// 尤其是內容剛好以 `使用者：` 開頭的時候。
    #[test]
    fn a_multiline_argument_keeps_every_line_indented() {
        let out = render_session_log(&tool_use_record(
            "Write",
            r#"{"file_path": "/x.txt", "content": "line1\n使用者：假的\nline3"}"#,
        ));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines,
            vec![
                "〔工具〕Write",
                "  file_path: /x.txt",
                "  content: line1",
                "  使用者：假的",
                "  line3",
            ]
        );
    }

    /// 過長的內容要截斷並**說明截掉多少**，而不是靜悄悄地少一截——
    /// 使用者要的是完整記錄，截斷至少得看得見。
    #[test]
    fn an_overlong_result_is_truncated_with_the_dropped_count() {
        let long = "a".repeat(BLOCK_MAX_CHARS + 123);
        let out = render_session_log(&tool_result_record(&format!("\"{long}\""), false));
        let body = out.trim().strip_prefix("〔結果〕").expect("格式不對");
        let kept = body.split('…').next().unwrap();
        assert_eq!(kept.chars().count(), BLOCK_MAX_CHARS, "保留的字數不對");
        assert!(body.ends_with("…（已截斷 123 字）"), "沒有標出截掉的字數：{}", &body[body.len() - 40..]);
    }

    /// 剛好等於上限不能截——差一個字元的錯誤（`<` 寫成 `<=`）只有這個
    /// 邊界值抓得到。
    #[test]
    fn a_result_exactly_at_the_limit_is_not_truncated() {
        let exact = "b".repeat(BLOCK_MAX_CHARS);
        let out = render_session_log(&tool_result_record(&format!("\"{exact}\""), false));
        assert!(!out.contains('…'), "剛好在上限就被截了");
    }

    /// 截短要按**字元**算，不是位元組。
    ///
    /// 內容常常整段是中文，而每個中文字是 3 個 UTF-8 位元組——用
    /// `&s[..N]` 這種位元組切法會切在字元中間直接 panic。這個 repo 已經
    /// 被同一類錯誤咬過一次（見 `pty/ansi.rs` 的 `is_char_boundary` 護欄，
    /// 註解記著實機上讓整個 app abort 的那次）。
    #[test]
    fn truncation_counts_characters_not_bytes() {
        let long = "中".repeat(BLOCK_MAX_CHARS + 50);
        let out = render_session_log(&tool_result_record(&format!("\"{long}\""), false));
        let body = out.trim().strip_prefix("〔結果〕").expect("格式不對");
        let kept = body.split('…').next().unwrap();
        assert_eq!(kept.chars().count(), BLOCK_MAX_CHARS, "沒有按字元截短");
        assert!(kept.len() > BLOCK_MAX_CHARS, "看起來是按位元組截的");
        assert!(body.ends_with("（已截斷 50 字）"), "截掉的字數不對");
    }

    /// 工具參數同樣受上限管：`Write` 一個 500 行的檔案不能把記錄撐爆。
    #[test]
    fn an_overlong_argument_is_truncated_too() {
        let long = "c".repeat(BLOCK_MAX_CHARS + 7);
        let out = render_session_log(&tool_use_record("Write", &format!(r#"{{"content": "{long}"}}"#)));
        assert!(out.contains("（已截斷 7 字）"), "參數沒有被截短：{}", out.chars().take(80).collect::<String>());
    }

    /// `isMeta: true` 是 Claude Code 自己塞進對話的系統訊息（斜線指令的
    /// caveat、session 命名提示……），不是使用者打的字。
    ///
    /// 第二筆沒有 `isMeta` 的斜線指令記錄刻意留著：那是使用者真的下了指令，
    /// 只認 `<command-name>` 標籤就濾掉的話會連它一起弄丟。
    #[test]
    fn meta_records_are_dropped_but_real_slash_commands_stay() {
        let jsonl = r#"{"type": "user", "isMeta": true, "message": {"role": "user", "content": "<local-command-caveat>Caveat: ...</local-command-caveat>"}}
{"type": "user", "message": {"role": "user", "content": "<command-name>/list-agents</command-name>"}}
{"type": "user", "isMeta": true, "message": {"role": "user", "content": [{"type": "text", "text": "<system-reminder>named arm</system-reminder>"}]}}
"#;
        let out = render_session_log(jsonl);
        assert_eq!(out.trim(), "使用者：<command-name>/list-agents</command-name>");
    }

    /// 空輸入不能 panic，也不該生出空白內容——呼叫端靠「渲染結果是空的」
    /// 決定要不要退回 transcript.txt。
    #[test]
    fn an_empty_or_contentless_log_renders_to_nothing() {
        assert_eq!(render_session_log(""), "");
        assert_eq!(render_session_log("{\"type\": \"mode\"}\n"), "");
    }
}

#[cfg(test)]
mod copy_tests {
    use super::*;

    /// 建一棵假的 `~/.claude/projects` 樹，回傳 (projects_root, 卡片專案資料夾)。
    /// 子資料夾名字刻意跟 session_id、work_dir 都無關——找法不看資料夾叫
    /// 什麼名字，這正是這個模組要證明的事。
    fn fake_tree(subdir_name: &str, session_id: &str, body: &str) -> (tempfile::TempDir, tempfile::TempDir) {
        let projects = tempfile::tempdir().unwrap();
        let dir = projects.path().join(subdir_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{session_id}.jsonl")), body).unwrap();
        (projects, tempfile::tempdir().unwrap())
    }

    #[test]
    fn copies_the_session_file_verbatim_into_the_task_dir() {
        let (projects, project) = fake_tree("some-project", "SID", "{\"type\":\"user\"}\n");

        let dest = copy_session_log(projects.path(), "SID", project.path(), "card1").unwrap();

        assert!(dest.ends_with("session.jsonl"), "存錯檔名：{dest}");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "{\"type\":\"user\"}\n");
        assert_eq!(
            std::path::Path::new(&dest).parent().unwrap(),
            crate::tasks::task_dir(project.path(), "card1"),
            "沒有存進卡片資料夾"
        );
    }

    /// claude 根本沒啟動、或使用者把 claude_command 設成別的東西時，
    /// 來源檔不存在。必須安靜回 None，讓呼叫端退回 transcript.txt。
    #[test]
    fn returns_none_when_the_session_file_does_not_exist() {
        let projects = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        assert!(copy_session_log(projects.path(), "SID", project.path(), "card1").is_none());
    }

    /// 核心行為：找法不看資料夾名，只看檔名對不對得上 session_id。子資料夾
    /// 這裡故意取一個跟任何 cwd 編碼規則都對不上的名字——不管是 Claude Code
    /// 自己怎麼編碼 cwd、還是 canonicalize／符號連結／Windows `\\?\` verbatim
    /// 前綴這些會讓「猜資料夾名」失準的情況，掃描法都找得到，因為
    /// `session_id` 本身是派工時生的 UUID v4，不會撞名，根本不需要猜資料夾名。
    #[test]
    fn finds_the_session_log_regardless_of_the_folder_name() {
        let (projects, project) = fake_tree("this-name-matches-no-encoding-scheme", "SID", "hello\n");

        let dest = copy_session_log(projects.path(), "SID", project.path(), "card1").unwrap();

        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hello\n");
    }

    /// 重新派工會產生新的 UUID 與新的 session 檔，`session.jsonl` **直接覆蓋**
    /// （見設計規格第 8 節）。
    ///
    /// 這條靠的是 `std::fs::copy` 覆蓋目的檔的保證，但我們的程式碼沒有把這個
    /// 依賴寫下來——日後有人好心加上「目的檔已存在就不覆蓋」的護欄，重新派工
    /// 的卡片就會顯示**上一次執行**的記錄。那比退回 transcript.txt 更糟：
    /// 看起來有內容，內容卻是錯的。所以要有測試把這個要求釘住。
    #[test]
    fn a_second_dispatch_overwrites_the_previous_session_log() {
        let (projects, project) = fake_tree("some-project", "OLD", "第一次執行\n");
        let first = copy_session_log(projects.path(), "OLD", project.path(), "card1").unwrap();
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "第一次執行\n");

        // 第二次派工：新的 session id、新的內容，同一張卡片。
        std::fs::write(projects.path().join("some-project").join("NEW.jsonl"), "第二次執行\n").unwrap();
        let second = copy_session_log(projects.path(), "NEW", project.path(), "card1").unwrap();

        assert_eq!(second, first, "第二次應該寫到同一個 session.jsonl");
        assert_eq!(
            std::fs::read_to_string(&second).unwrap(),
            "第二次執行\n",
            "舊的內容沒有被覆蓋掉"
        );
    }
}
