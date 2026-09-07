//! Claude Code 自己寫的 session 記錄：找到它、複製它、渲染它。
//!
//! Claude Code 每一輪 user / assistant / 工具呼叫都會即時寫進
//! `~/.claude/projects/<編碼後的 cwd>/<session-id>.jsonl`。派工時我們用
//! `--session-id` 指定那個 UUID，所以任務結束後不必猜哪個檔案對應哪張卡片。
//!
//! 這個模組除了最外層的 `copy_session_log` 之外全是純函式（吃字串、吐
//! 字串），與檔案系統和 Tauri state 無關。
//!
//! 見 docs/superpowers/specs/2026-09-07-full-task-transcript-design.md。

use std::path::Path;

/// 把絕對路徑編成 Claude Code 用的資料夾名：每一個路徑分隔符換成 `-`。
///
/// 路徑裡既有的 `-` 原樣保留（`/private/tmp/-Users-x` → `-private-tmp--Users-x`），
/// 所以這個轉換不可逆——不需要可逆，我們只需要拼得出同一個資料夾名。
///
/// Windows 的 `\` 與磁碟機代號後面的 `:` 一樣算分隔符，否則 Windows 上
/// 永遠對不到目錄。
pub fn encode_project_dir(dir: &Path) -> String {
    dir.to_string_lossy()
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == ':' { '-' } else { c })
        .collect()
}

/// 工具參數摘要的長度上限（字元數，不是位元組——參數常常是中文）。
const TOOL_ARG_MAX_CHARS: usize = 100;

/// 把 Claude Code 的 session JSONL 渲染成逐輪對話純文字。
///
/// 只處理 `user` / `assistant` 兩種記錄，其餘一律跳過。思考過程與工具
/// 回傳也跳過：呼叫回答「它做了什麼」，那是工作記錄的核心；回傳是雜訊，
/// 而且佔了整個檔案 73% 的體積（spec 實測）。
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
                Some("tool_use") => {
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let arg = first_arg_summary(block.get("input"));
                    push_line(&mut out, &format!("〔工具〕{name} {arg}"));
                }
                // thinking / tool_result / 其他一律跳過。
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

/// 工具呼叫的第一個參數，壓成一行並截短。
///
/// 「第一個」指 JSONL 裡的書寫順序：`serde_json` 開了 `preserve_order`
/// （見 Cargo.toml），所以物件的鍵序就是原始順序，不是字典序。
fn first_arg_summary(input: Option<&serde_json::Value>) -> String {
    let Some(first) = input.and_then(|v| v.as_object()).and_then(|m| m.values().next()) else {
        return String::new();
    };
    let raw = match first {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= TOOL_ARG_MAX_CHARS {
        return one_line;
    }
    let cut: String = one_line.chars().take(TOOL_ARG_MAX_CHARS).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod encode_tests {
    use super::*;

    #[test]
    fn every_slash_becomes_a_dash() {
        assert_eq!(
            encode_project_dir(Path::new("/Users/jamesju/Documents/GitHub/AITERM")),
            "-Users-jamesju-Documents-GitHub-AITERM"
        );
    }

    /// 路徑本身既有的 `-` 必須原樣保留，不可以被合併或跳脫——這是唯一能
    /// 分辨「換掉斜線」與「換掉斜線後又去正規化連字號」兩種實作的案例。
    #[test]
    fn existing_dashes_in_the_path_are_left_alone() {
        assert_eq!(
            encode_project_dir(Path::new("/private/tmp/-Users-x")),
            "-private-tmp--Users-x"
        );
    }

    /// Windows 路徑也要能編碼——反斜線與磁碟機代號都算路徑分隔，
    /// 否則 Windows 上永遠找不到 session 檔。
    #[test]
    fn windows_separators_and_drive_letters_become_dashes() {
        assert_eq!(encode_project_dir(Path::new(r"C:\Users\j\repo")), "C--Users-j-repo");
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    /// 真實 session 檔抽出來的五種記錄（長字串已截短，結構未動），外加
    /// 三種必須被跳過的雜訊記錄、一行壞掉的 JSON 與一行空行。
    const REAL_FIXTURE: &str = r#"{"type": "user", "message": {"role": "user", "content": "請繼續"}}
{"type": "assistant", "message": {"role": "assistant", "content": [{"type": "thinking", "thinking": "先看 repo 狀態", "signature": "SIG"}]}, "isSidechain": false}
{"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_01RZV1vshph15jPaL7ozx2an", "name": "Bash", "input": {"command": "git status --short | head -30", "description": "Check repo state"}}]}, "isSidechain": false}
{"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_01N7g4oADgBWEqZaM4GUcHtA", "content": "No matching deferred tools found"}]}}
{"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "工作目錄乾淨，沒有未推的 commit。"}]}, "isSidechain": false}
{"type": "mode", "uuid": "x"}
{"type": "attachment", "uuid": "ca05c03c-7993-4ec6-baf4-c4e3b71e64e3"}
{"type": "file-history-snapshot", "uuid": "x"}
{this is not valid json

"#;

    #[test]
    fn renders_each_turn_in_order() {
        let out = render_session_log(REAL_FIXTURE);
        let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(
            lines,
            vec![
                "使用者：請繼續",
                "〔工具〕Bash git status --short | head -30",
                "Claude：工作目錄乾淨，沒有未推的 commit。",
            ]
        );
    }

    /// 思考過程與工具回傳都必須不見。這兩條分開斷言而不是只看上面的
    /// 完整比對，是因為它們各自對應 spec 裡一個明確的設計決定。
    #[test]
    fn thinking_and_tool_results_are_dropped() {
        let out = render_session_log(REAL_FIXTURE);
        assert!(!out.contains("先看 repo 狀態"), "思考過程沒有被丟掉：{out}");
        assert!(!out.contains("No matching deferred tools found"), "工具回傳沒有被丟掉：{out}");
    }

    /// 一行壞掉不該讓整份記錄消失——壞掉的那行前後的內容都必須還在。
    #[test]
    fn a_broken_line_does_not_abort_the_whole_render() {
        let out = render_session_log(REAL_FIXTURE);
        assert!(out.contains("使用者：請繼續"), "壞行之前的內容不見了：{out}");
        assert!(out.contains("Claude：工作目錄乾淨"), "壞行之後的內容不見了：{out}");
    }

    /// 實測：`user` 記錄的 content 陣列有 2 個、3 個 block 的情況（本機六份
    /// 真實 session 檔裡共 60 筆）。只讀第一個 block 的實作會漏掉後面的，
    /// 所以這個 fixture 的第二個 block 才是唯一該被輸出的那個。
    #[test]
    fn walks_every_block_in_a_multi_block_record() {
        let jsonl = r#"{"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ignored"}, {"type": "text", "text": "第二個 block"}]}}"#;
        assert_eq!(render_session_log(jsonl).trim(), "使用者：第二個 block");
    }

    /// 工具摘要取的是「第一個參數」——serde_json 開了 preserve_order，
    /// 所以那是 JSONL 裡的書寫順序。這個 fixture 刻意讓書寫順序（`command`）
    /// 與字典順序（`description`）不同，否則測不出兩者的差別。
    #[test]
    fn tool_summary_uses_the_first_written_argument_not_the_alphabetical_one() {
        let jsonl = r#"{"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "t", "name": "Bash", "input": {"command": "ls -la", "description": "aaa list files"}}]}}"#;
        let out = render_session_log(jsonl);
        assert!(out.contains("ls -la"), "沒有用書寫順序的第一個參數：{out}");
        assert!(!out.contains("aaa list files"), "用到了字典序第一個參數：{out}");
    }

    /// 摘要要短、要單行——工具參數常常是好幾 KB 的檔案內容，整段塞進
    /// 記錄就把「做了什麼」淹掉了。
    #[test]
    fn a_long_multiline_argument_is_truncated_to_one_line() {
        let long = "a".repeat(500);
        let jsonl = format!(
            r#"{{"type": "assistant", "message": {{"role": "assistant", "content": [{{"type": "tool_use", "id": "t", "name": "Write", "input": {{"content": "line1\nline2 {long}"}}}}]}}}}"#
        );
        let out = render_session_log(&jsonl);
        assert_eq!(out.lines().count(), 1, "摘要跨了多行：{out}");
        assert!(out.chars().count() <= 140, "摘要沒有被截短（{} 個字）：{out}", out.chars().count());
        assert!(out.contains('…'), "截短後沒有省略號：{out}");
    }

    /// 空輸入不能 panic，也不該生出空白內容——呼叫端靠「渲染結果是空的」
    /// 決定要不要退回 transcript.txt。
    #[test]
    fn an_empty_or_contentless_log_renders_to_nothing() {
        assert_eq!(render_session_log(""), "");
        assert_eq!(render_session_log("{\"type\": \"mode\"}\n"), "");
    }
}
