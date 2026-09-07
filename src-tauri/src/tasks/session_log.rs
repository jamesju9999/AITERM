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

use std::path::{Path, PathBuf};

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

/// `~/.claude/projects` — Claude Code 放 session 記錄的地方。
/// 家目錄取不到時回 `None`，呼叫端安靜退回 transcript.txt。
pub fn claude_projects_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

/// 這個工作目錄可能對應到的 session 資料夾名（原樣路徑、canonicalize 後的
/// 路徑）。macOS 上 `/tmp` 是 `/private/tmp` 的符號連結，兩者編出來的名字
/// 不同，而 Claude Code 用哪一個我們無法確定，所以兩個都試。
///
/// 已知侷限（刻意不修）：Windows 上 `canonicalize` 回傳帶 `\\?\` 前綴的
/// verbatim 路徑（如 `\\?\C:\Users\j\repo`），編碼後會是
/// `--?-C--Users-j-repo`，幾乎可以確定不是 Claude Code 實際寫的資料夾名。
/// 因為原樣路徑候選排在第一個，Windows 常見情況仍然找得到；要不要剝掉
/// `\\?\` 前綴要等實測 Claude Code 在 Windows 上到底寫哪個名字才能決定，
/// 這裡不先猜。
fn dir_candidates(work_dir: &Path) -> Vec<String> {
    let mut out = vec![encode_project_dir(work_dir)];
    if let Ok(canonical) = std::fs::canonicalize(work_dir) {
        let encoded = encode_project_dir(&canonical);
        if !out.contains(&encoded) {
            out.push(encoded);
        }
    }
    out
}

/// 把 `<projects_root>/<編碼後的 work_dir>/<session_id>.jsonl` **原封不動**
/// 複製成 `<project_path>/tasks/<task_id>/session.jsonl`，回傳目的地路徑。
///
/// 不精簡是刻意的：一次派工只有幾十 KB 到數 MB，磁碟成本遠低於提早丟資料
/// 的風險。要精簡永遠來得及，丟掉的救不回來。
///
/// 找不到來源、或複製失敗（權限、磁碟），都安靜回 `None` 並寫進 stderr——
/// 原本的 `transcript.txt` 還在，沒有東西壞掉，不值得打斷使用者。
pub fn copy_session_log(
    projects_root: &Path,
    project_path: &Path,
    task_id: &str,
    work_dir: &Path,
    session_id: &str,
) -> Option<String> {
    let src = dir_candidates(work_dir)
        .into_iter()
        .map(|d| projects_root.join(d).join(format!("{session_id}.jsonl")))
        .find(|p| p.is_file())?;

    let dir = crate::tasks::task_dir(project_path, task_id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("session log dir {dir:?}: {e}");
        return None;
    }
    let dest = dir.join("session.jsonl");
    match std::fs::copy(&src, &dest) {
        Ok(_) => Some(dest.to_string_lossy().into_owned()),
        Err(e) => {
            eprintln!("copy session log {src:?} → {dest:?}: {e}");
            None
        }
    }
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

    /// 實測：`user` 記錄的 content 陣列有 2 個、3 個 block 的情況（本機六份
    /// 真實 session 檔裡共 60 筆，其中 3 筆是 3 個 block）。可渲染的 block
    /// 刻意放在中間（前後各夾一個該被跳過的 `tool_result`）：如果放在最後，
    /// 「只讀第一個 block」與「只讀最後一個 block」這兩種錯誤實作都會
    /// 意外通過，測不出「走完整個陣列」這個要求。
    #[test]
    fn walks_every_block_in_a_multi_block_record() {
        let jsonl = r#"{"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ignored"}, {"type": "text", "text": "中間的 block"}, {"type": "tool_result", "tool_use_id": "t2", "content": "also ignored"}]}}"#;
        assert_eq!(render_session_log(jsonl).trim(), "使用者：中間的 block");
    }

    /// 工具摘要取的是「第一個參數」——serde_json 開了 preserve_order，
    /// 所以那是 JSONL 裡的書寫順序。
    ///
    /// fixture 用 `Write` 而不是 `Bash`：`Bash` 的 `{command, description}`
    /// 兩種順序**剛好一樣**（c < d），拿它當 fixture 的話，就算 serde_json
    /// 改成照字典序排也照樣會綠——那是一個空測試。`Write` 的
    /// `{file_path, content}` 才會分岔：書寫序是 `file_path`，字典序是
    /// `content`。兩個都是實機 session 檔裡真實出現過的形狀。
    #[test]
    fn tool_summary_uses_the_first_written_argument_not_the_alphabetical_one() {
        let jsonl = r#"{"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "t", "name": "Write", "input": {"file_path": "/repo/src/lib.rs", "content": "aaa file body"}}]}}"#;
        let out = render_session_log(jsonl);
        assert!(out.contains("/repo/src/lib.rs"), "沒有用書寫順序的第一個參數：{out}");
        assert!(!out.contains("aaa file body"), "用到了字典序第一個參數：{out}");
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

    /// 截短要按**字元**算，不是位元組。
    ///
    /// 工具參數常常整段是中文，而每個中文字是 3 個 UTF-8 位元組——用
    /// `&s[..100]` 這種位元組切法會切在字元中間直接 panic。這個 repo 已經
    /// 被同一類錯誤咬過一次（見 `pty/ansi.rs` 的 `is_char_boundary` 護欄，
    /// 註解記著實機上讓整個 app abort 的那次），所以這裡要有測試撐著
    /// `first_arg_summary` 那句「字元數，不是位元組」的宣稱。
    #[test]
    fn truncation_counts_characters_not_bytes() {
        let long = "把這段中文重複很多次".repeat(30); // 300 個字元、900 位元組
        let jsonl = format!(
            r#"{{"type": "assistant", "message": {{"role": "assistant", "content": [{{"type": "tool_use", "id": "t", "name": "Write", "input": {{"content": "{long}"}}}}]}}}}"#
        );
        let out = render_session_log(&jsonl);
        // 沒 panic 就已經是一半的價值了。另一半：截出來的必須是 100 個
        // 字元（而不是 100 個位元組 ≈ 33 個字），所以位元組長度遠大於 100。
        let arg = out.trim().strip_prefix("〔工具〕Write ").expect("格式不對：{out}");
        let arg = arg.strip_suffix('…').expect("沒有截短：{arg}");
        assert_eq!(arg.chars().count(), 100, "沒有按字元截短");
        assert!(arg.len() > 100, "看起來是按位元組截的：{} 位元組", arg.len());
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
    fn fake_tree(work_dir: &Path, session_id: &str, body: &str) -> (tempfile::TempDir, tempfile::TempDir) {
        let projects = tempfile::tempdir().unwrap();
        let dir = projects.path().join(encode_project_dir(work_dir));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{session_id}.jsonl")), body).unwrap();
        (projects, tempfile::tempdir().unwrap())
    }

    #[test]
    fn copies_the_session_file_verbatim_into_the_task_dir() {
        let work = Path::new("/work/repo");
        let (projects, project) = fake_tree(work, "SID", "{\"type\":\"user\"}\n");

        let dest = copy_session_log(projects.path(), project.path(), "card1", work, "SID").unwrap();

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
        assert!(
            copy_session_log(projects.path(), project.path(), "card1", Path::new("/work/repo"), "SID")
                .is_none()
        );
    }

    /// macOS 上 /tmp 是 /private/tmp 的符號連結，兩者編出來的資料夾名不同。
    /// Claude Code 用哪一個我們無法確定，所以兩個候選都要試。這個測試把
    /// 檔案只放在 canonicalize 後的那個資料夾裡——只試原樣路徑的實作會找不到。
    #[test]
    fn falls_back_to_the_canonicalised_form_of_the_work_dir() {
        let real = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(real.path()).unwrap();
        // 這個測試只有在 tempdir 真的會被 canonicalize 改寫時才有鑑別力
        // （macOS 的 /var → /private/var）。否則兩個候選相同，測不出差別。
        if canonical == real.path() {
            eprintln!("跳過：這個平台的 tempdir 路徑已經是 canonical 形式");
            return;
        }
        let projects = tempfile::tempdir().unwrap();
        let dir = projects.path().join(encode_project_dir(&canonical));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SID.jsonl"), "x").unwrap();

        let project = tempfile::tempdir().unwrap();
        let dest = copy_session_log(projects.path(), project.path(), "card1", real.path(), "SID");
        assert!(dest.is_some(), "沒有試 canonicalize 後的路徑候選");
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
        let work = Path::new("/work/repo");
        let (projects, project) = fake_tree(work, "OLD", "第一次執行\n");
        let first = copy_session_log(projects.path(), project.path(), "card1", work, "OLD").unwrap();
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "第一次執行\n");

        // 第二次派工：新的 session id、新的內容，同一張卡片。
        let dir = projects.path().join(encode_project_dir(work));
        std::fs::write(dir.join("NEW.jsonl"), "第二次執行\n").unwrap();
        let second = copy_session_log(projects.path(), project.path(), "card1", work, "NEW").unwrap();

        assert_eq!(second, first, "第二次應該寫到同一個 session.jsonl");
        assert_eq!(
            std::fs::read_to_string(&second).unwrap(),
            "第二次執行\n",
            "舊的內容沒有被覆蓋掉"
        );
    }
}
