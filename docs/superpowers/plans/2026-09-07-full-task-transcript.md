# 工作看板：完整對話記錄 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓工作看板保存的對話記錄從「終端機最後一屏」變成 Claude Code 自己寫的完整逐輪記錄，工作報告的摘要因此看得到整段工作。

**Architecture:** 派工時給 `claude` 一個指定的 `--session-id <uuid>`，任務結束後把 Claude Code 自己寫在 `~/.claude/projects/<編碼路徑>/<uuid>.jsonl` 的記錄原封不動複製進卡片資料夾；`tasks_read_transcript` 改成優先把那份 JSONL 渲染成逐輪對話，讀不到才退回現行的 `transcript.txt`。所有變更收斂在後端，前端兩個消費端（`TranscriptDialog`、`useWorkReport`）介面不動。

**Tech Stack:** Rust / Tauri 2 / sqlx (SQLite) / serde_json（已開 `preserve_order`）/ uuid v4 / portable-pty；前端 TypeScript + Vitest。

**Spec:** `docs/superpowers/specs/2026-09-07-full-task-transcript-design.md`

---

## 寫計畫時新測到的三件事（spec 沒有，會改變實作）

這三點都是動手前實測出來的，不是推論。每一點都在下面有對應的任務與測試。

### 1. 信任畫面的 PTY 輸出裡**一個空白位元組都沒有**

實跑 `claude` 進一個未信任的資料夾、抓下原始 PTY bytes，統計結果是 `' ' in raw == False`（0 個 0x20）。Claude Code 的 TUI 用游標移動排版，不送空白字元。

所以 spec 第 6 節設想的「比對特徵字串」若寫成 `contains("Yes, I trust this folder")` **永遠不會命中**。`PtyManager::get_recent_output` 會先過 `strip_ansi`，出來的畫面長這樣（每個字之間真的沒有空白）：

```
────────────────────────────────────────────────────────────────────────────────
Accessingworkspace:
/private/tmp/.../trustprobe-23565
Quicksafetycheck:Isthisaprojectyoucreatedoroneyoutrust?(Likeyour
owncode,awell-knownopensourceproject,orworkfromyourteam).Ifnot,
takeamomenttoreviewwhat'sinthisfolderfirst.
ClaudeCode'llbeabletoread,edit,andexecutefileshere.
Securityguide
❯No,exit
Yes,Itrustthisfolder
Entertoconfirm·Esctocancel
```

比對前必須把所有空白字元濾掉。

### 2. 預設選中的是 `No, exit`，而且**選項順序可能對調**

上面的畫面顯示游標 `❯` 停在 `No, exit` 上，所以只送 `\r` 會直接退出。但也**不可以**寫死「送一次 Down 再 Enter」——Claude Code 隨時可能把兩個選項對調或改預設選擇，那樣寫死就會反過來按到 `No, exit`，把使用者的派工直接殺掉。

做法改成：在畫面上同時定位游標行 `❯` 與 `Yes,Itrustthisfolder` 那一行，算出相對位移，再送對應次數的 `\x1b[B`（往下）或 `\x1b[A`（往上），最後 `\r`。**任何一個定位不到就什麼都不送**，退回現行行為（卡住偵測收掉）。這樣即使 UI 改版，最壞情況也只是「不生效」，永遠不會誤按 `No, exit`。

### 3. `claude_command` 可能不是 `claude`

`src-tauri/src/config/types.rs:203` 的 `claude_command` 是使用者可改的設定，而 `dispatch.rs` 的 `NO_TUI_QUIET_MS` 路徑是**刻意**為「設定成不是全螢幕 TUI 的指令」留的。無條件接上 `--session-id` 會讓那些指令直接啟動失敗——不是拿不到完整記錄而已，是整個派工壞掉。

所以旗標只在指令的第一個 token 的 basename 是 `claude` / `claude.exe` 時附加；其他指令維持現行行為，記錄自動退回 `transcript.txt`。

### 附帶確認（不需額外處理）

- **子代理記錄**：JSONL 記錄有 `isSidechain` 欄位。掃過本機六份最新的真實 session 檔，`isSidechain` 全部是 `false`——子代理的對話不寫進主 session 檔。所以不需要任何過濾程式碼。
- **content 陣列長度**：`assistant` 記錄永遠只有 1 個 block（Claude Code 每個 block 寫一筆，見頂層的 `apiBlockIndex`）；但 `user` 記錄實測有 2 個（57 筆）和 3 個（3 筆）的情況。**渲染必須走完整個陣列，不能只看第一個 block。**
- **`serde_json` 已開 `preserve_order`**（`src-tauri/Cargo.toml:21`），所以「第一個參數」就是 JSONL 裡的書寫順序（`Bash` 工具是 `command`）。

---

## File Structure

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/src/tasks/session_log.rs` | 建立 | 純函式：路徑編碼、JSONL 渲染、找出 session 檔、複製進卡片資料夾 |
| `src-tauri/src/tasks/mod.rs` | 修改 | 掛上 `pub mod session_log;`；schema 加兩個 ALTER TABLE |
| `src-tauri/src/tasks/store.rs` | 修改 | `TaskRow` 加兩個欄位；`set_session_id` / `set_session_path` |
| `src-tauri/src/tasks/dispatch.rs` | 修改 | `looks_like_claude` / `launch_command` / `trust_prompt_keys` 三個純函式；`spawn_and_run` 多一個 `session_id` 參數；`wait_until_settled` 處理信任畫面 |
| `src-tauri/src/tasks/scheduler.rs` | 修改 | `RealDispatcher` 產生 uuid、寫回 `session_id`，完成時複製 JSONL 並寫回 `session_path` |
| `src-tauri/src/commands/tasks.rs` | 修改 | `tasks_read_transcript` 優先渲染 session.jsonl |
| `src/ipc/tasks.ts` | 修改 | `TaskRow` 型別加兩個欄位 |
| `src/components/TaskBoard/*.test.*` | 修改 | 三處 fixture 補新欄位 |

`session_log.rs` 刻意只放不碰 Tauri state 的東西，所有解析與路徑計算都是吃 `&str`／`&Path`、吐 `String`／`PathBuf` 的純函式，只有最外層一個 `copy_session_log` 碰檔案系統。

---

## Task 1: `session_log.rs` — 專案路徑編碼

Claude Code 把 session 檔放在 `~/.claude/projects/<編碼後的絕對路徑>/<session-id>.jsonl`。編碼規則是把絕對路徑的每一個 `/` 換成 `-`；路徑本身既有的 `-` 保留不動，所以 `/private/tmp/-Users-x` 會變成 `-private-tmp--Users-x`。spec 說這是整份設計最容易寫錯的一行。

**Files:**
- Create: `src-tauri/src/tasks/session_log.rs`
- Modify: `src-tauri/src/tasks/mod.rs`

- [ ] **Step 1: 建立模組並掛上**

建立 `src-tauri/src/tasks/session_log.rs`，內容先只有檔頭：

```rust
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
```

在 `src-tauri/src/tasks/mod.rs` 的模組宣告區加一行（放在 `pub mod store;` 之後）：

```rust
pub mod store;
pub mod session_log;
pub mod dispatch;
pub mod monitor;
pub mod scheduler;
```

同時把檔頭的模組清單註解補上一行，跟現有格式一致：

```rust
//! - `store`     — `tasks.db` schema + CRUD (sqlx free functions over a pool)
//! - `session_log` — Claude Code 自己寫的 session JSONL：定位、複製、渲染
//! - `dispatch`  — compose the prompt, spawn a visible PTY tab, type it in
```

- [ ] **Step 2: 寫會紅的測試**

在 `session_log.rs` 檔尾加：

```rust
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
```

- [ ] **Step 3: 跑測試，確認它紅**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::session_log 2>&1 | tail -20
```

Expected: 編譯失敗，`cannot find function `encode_project_dir` in this scope`。

- [ ] **Step 4: 實作**

在 `session_log.rs` 的 `use` 之後加：

```rust
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
```

- [ ] **Step 5: 跑測試，確認它綠**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::session_log 2>&1 | tail -20
```

Expected: `test result: ok. 3 passed`。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/tasks/session_log.rs src-tauri/src/tasks/mod.rs
git commit -m "feat(tasks): encode a project dir into Claude Code's session-log folder name

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

---

## Task 2: `session_log.rs` — 把 JSONL 渲染成逐輪對話

這是整份計畫的核心純函式。逐行解析，只處理 `type` 為 `user` / `assistant` 的記錄，其餘（`mode`、`attachment`、`file-history-snapshot`、`cost-state`、`system`…）跳過。

| block 形態 | 處理 |
|---|---|
| `message.content` 是字串（user） | `使用者：<內容>` |
| `{type:"text"}` | 依記錄的 `type`：user → `使用者：`，assistant → `Claude：` |
| `{type:"thinking"}` | 跳過（思考過程不是工作記錄） |
| `{type:"tool_use"}` | `〔工具〕<name> <第一個參數的摘要>` |
| `{type:"tool_result"}` | 跳過（雜訊，且佔了整個檔案 73% 的體積） |

**Files:**
- Modify: `src-tauri/src/tasks/session_log.rs`

- [ ] **Step 1: 放進真實 fixture**

fixture 必須取自真實 session 檔，不可手寫——手寫的乾淨樣本會替錯誤的假設背書。下面這段是從本機一份真實的 `~/.claude/projects/.../<uuid>.jsonl` 抽出來、只把長字串截短的結果，欄位名稱與巢狀結構完全沒動。

在 `session_log.rs` 檔尾加：

```rust
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
}
```

- [ ] **Step 2: 寫會紅的測試**

接在同一個 `mod render_tests` 裡（`REAL_FIXTURE` 之後）：

```rust
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
```

- [ ] **Step 3: 跑測試，確認它紅**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::session_log 2>&1 | tail -20
```

Expected: 編譯失敗，`cannot find function `render_session_log` in this scope`。

- [ ] **Step 4: 實作**

在 `session_log.rs` 的 `encode_project_dir` 之後加：

```rust
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
```

- [ ] **Step 5: 跑測試，確認它綠**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::session_log 2>&1 | tail -20
```

Expected: `test result: ok. 10 passed`（Task 1 的 3 個 + 這裡的 7 個）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/tasks/session_log.rs
git commit -m "feat(tasks): render a Claude Code session log into a turn-by-turn transcript

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

---

## Task 3: `session_log.rs` — 找到並複製 session 檔

複製是**原封不動**，不精簡：一次派工只有幾十 KB 到數 MB，磁碟成本遠低於提早丟資料的風險。要精簡永遠來得及，丟掉的救不回來。

路徑有兩個候選：`task.project_dir` 原樣編碼，以及 `canonicalize` 之後再編碼。macOS 上 `/tmp` 是 `/private/tmp` 的符號連結，兩者編出來的資料夾名不同，而我們無法確定 Claude Code 用的是哪一個。兩個都試，找到誰算誰。

**Files:**
- Modify: `src-tauri/src/tasks/session_log.rs`

- [ ] **Step 1: 寫會紅的測試**

在 `session_log.rs` 檔尾加：

```rust
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
}
```

- [ ] **Step 2: 跑測試，確認它紅**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::session_log 2>&1 | tail -20
```

Expected: 編譯失敗，`cannot find function `copy_session_log` in this scope`。

- [ ] **Step 3: 實作**

在 `session_log.rs` 的 `first_arg_summary` 之後加：

```rust
/// `~/.claude/projects` — Claude Code 放 session 記錄的地方。
/// 家目錄取不到時回 `None`，呼叫端安靜退回 transcript.txt。
pub fn claude_projects_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

/// 這個工作目錄可能對應到的 session 資料夾名（原樣路徑、canonicalize 後的
/// 路徑）。macOS 上 `/tmp` 是 `/private/tmp` 的符號連結，兩者編出來的名字
/// 不同，而 Claude Code 用哪一個我們無法確定，所以兩個都試。
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
```

- [ ] **Step 4: 確認 `tempfile` 是 dev-dependency**

```bash
grep -n "tempfile" src-tauri/Cargo.toml
```

Expected: 在 `[dev-dependencies]` 底下看到 `tempfile`。若沒有，加上 `tempfile = "3"`。

- [ ] **Step 5: 跑測試，確認它綠**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::session_log 2>&1 | tail -20
```

Expected: `test result: ok. 13 passed`。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/tasks/session_log.rs src-tauri/Cargo.toml
git commit -m "feat(tasks): copy a finished card's session log into its task dir

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

---

## Task 4: 資料庫兩個新欄位

`session_id` 在派工時寫入，`session_path` 在完成時寫入。分成兩個 setter 而不是塞進 `set_tab_id` / `finish_task`：兩者發生的時機差了整場任務，硬綁在一起會讓既有測試全部要改。

**Files:**
- Modify: `src-tauri/src/tasks/mod.rs`
- Modify: `src-tauri/src/tasks/store.rs`

- [ ] **Step 1: 寫會紅的測試**

在 `src-tauri/src/tasks/store.rs` 的 `mod archive_tests` 之後，加一個新的測試模組：

```rust
#[cfg(test)]
mod session_column_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    async fn a_task(pool: &SqlitePool) -> String {
        create_task(pool, "t", "b", "/work/repo", true, false).await.unwrap()
    }

    #[tokio::test]
    async fn a_new_card_has_neither_session_id_nor_session_path() {
        let pool = mem_pool().await;
        let id = a_task(&pool).await;
        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.session_id, None);
        assert_eq!(row.session_path, None);
    }

    #[tokio::test]
    async fn set_session_id_round_trips() {
        let pool = mem_pool().await;
        let id = a_task(&pool).await;
        set_session_id(&pool, &id, "3f2a-uuid").await.unwrap();
        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.session_id.as_deref(), Some("3f2a-uuid"));
        // 只動這一欄，別的欄位不受影響。
        assert_eq!(row.session_path, None);
    }

    #[tokio::test]
    async fn set_session_path_round_trips() {
        let pool = mem_pool().await;
        let id = a_task(&pool).await;
        set_session_path(&pool, &id, "/proj/tasks/x/session.jsonl").await.unwrap();
        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.session_path.as_deref(), Some("/proj/tasks/x/session.jsonl"));
    }

    /// 重新派工會產生新的 UUID 與新的 session 檔，session.jsonl 直接覆蓋——
    /// 舊值不可以殘留下來讓 tasks_read_transcript 讀到上一次執行的記錄。
    #[tokio::test]
    async fn claiming_for_dispatch_clears_the_previous_run_s_session_path() {
        let pool = mem_pool().await;
        let id = a_task(&pool).await;
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        set_session_path(&pool, &id, "/old/session.jsonl").await.unwrap();

        assert!(claim_for_dispatch(&pool, &id).await.unwrap());

        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.session_path, None, "上一次執行的 session_path 殘留了");
    }
}
```

- [ ] **Step 2: 跑測試，確認它紅**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::store::session_column 2>&1 | tail -20
```

Expected: 編譯失敗，`no field `session_id` on type `TaskRow``。

- [ ] **Step 3: schema 加兩欄**

在 `src-tauri/src/tasks/mod.rs` 的 `init_schema` 裡，`CREATE TABLE` 的欄位清單加兩行（放在 `archived_at` 之後）：

```rust
            ai_summary      TEXT,
            archived_at     INTEGER,
            session_id      TEXT,
            session_path    TEXT
```

並在 `archived_at` 那段 migration 之後補上兩段，沿用同一個寫法：

```rust
    // Migration: existing databases created before the session-log columns
    // existed. 跟上面幾個同一個寫法——欄位已存在時 ALTER TABLE 會失敗，
    // 那是正常的，所以刻意丟掉錯誤。
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN session_id TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN session_path TEXT")
        .execute(pool)
        .await;
```

- [ ] **Step 4: `TaskRow` 加兩個欄位**

在 `src-tauri/src/tasks/store.rs` 的 `TaskRow` 裡，`archived_at` 之後加：

```rust
    /// 這次派工給 `claude --session-id` 的 UUID。只有指令看起來是 claude
    /// 時才會有（見 `dispatch::looks_like_claude`）。
    pub session_id: Option<String>,
    /// 複製進卡片資料夾的 `session.jsonl` 路徑。有值代表這張卡片有完整的
    /// 逐輪記錄；沒有就退回 `transcript_path`。
    pub session_path: Option<String>,
```

- [ ] **Step 5: 兩個 setter，以及重新派工時清掉舊路徑**

在 `store.rs` 的 `set_tab_id` 之後加：

```rust
/// 記下這次派工用的 `--session-id`。在 `spawn_and_run` 之後、與
/// `set_tab_id` 同一個時機呼叫。
pub async fn set_session_id(pool: &SqlitePool, id: &str, session_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET session_id = ? WHERE id = ?")
        .bind(session_id)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 記下複製進卡片資料夾的 session.jsonl 路徑。在 `finish_task` 之前呼叫，
/// 這樣 `tasks-updated` 送出時該列已經是完整的。
pub async fn set_session_path(pool: &SqlitePool, id: &str, path: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET session_path = ? WHERE id = ?")
        .bind(path)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

接著在 `claim_for_dispatch`（`store.rs:379` 附近）清掉上一次執行留下的 `session_path`。把它的 SQL 從

```rust
        "UPDATE tasks SET status = 'running', dispatched_at = ?, ai_summary = NULL
         WHERE id = ? AND status = 'queued'",
```

改成

```rust
        "UPDATE tasks SET status = 'running', dispatched_at = ?, ai_summary = NULL,
             session_path = NULL
         WHERE id = ? AND status = 'queued'",
```

`session_id` 不必清——下一步的 `set_session_id` 就會覆寫掉。`session_path` 一定要清：不清的話，這一次複製失敗時 `tasks_read_transcript` 會讀到**上一次執行**的記錄，而那比退回 `transcript.txt` 更糟（看起來有內容，內容卻是錯的）。

順手把這個函式的文件註解第一行從「派工時間、清掉上一次執行留下的摘要」改成「派工時間、清掉上一次執行留下的摘要與 session 記錄路徑」。

- [ ] **Step 6: 跑測試，確認它綠**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks:: 2>&1 | tail -20
```

Expected: 全綠。`SELECT *` 搭配 `FromRow` 會自動帶上新欄位，其他查詢不必改。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/tasks/mod.rs src-tauri/src/tasks/store.rs
git commit -m "feat(tasks): add session_id and session_path columns

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

---

## Task 5: `dispatch.rs` — 指令組裝

兩個純函式。`looks_like_claude` 決定要不要給這個指令一個 session id；`launch_command` 組出真正要送進終端機的字串。

**為什麼要 `looks_like_claude`**：`claude_command` 是使用者可改的設定，而 `dispatch.rs` 的 `NO_TUI_QUIET_MS` 路徑是刻意為「設定成不是全螢幕 TUI 的指令」留的。無條件接上 `--session-id` 會讓那些指令直接啟動失敗。

**Files:**
- Modify: `src-tauri/src/tasks/dispatch.rs`

- [ ] **Step 1: 寫會紅的測試**

在 `dispatch.rs` 的 `mod tests` 裡，`blank_body_still_produces_the_attachment_note` 之後加：

```rust
    #[test]
    fn a_plain_claude_command_gets_a_session_id() {
        assert!(looks_like_claude("claude"));
    }

    /// 帶旗標的指令也算——使用者常設成 `claude --dangerously-skip-permissions`。
    #[test]
    fn a_claude_command_with_flags_still_counts() {
        assert!(looks_like_claude("claude --dangerously-skip-permissions"));
    }

    /// 絕對路徑與 Windows 的 .exe 都算。
    #[test]
    fn an_absolute_path_to_claude_counts() {
        assert!(looks_like_claude("/opt/homebrew/bin/claude"));
        assert!(looks_like_claude(r"C:\Program Files\claude.exe --verbose"));
    }

    /// 這是這個函式存在的理由：非 claude 的指令不能被接上旗標，否則直接
    /// 啟動失敗。`claude-code` 這種名字相近但不同的指令也必須是 false——
    /// 只用 `contains("claude")` 的實作會在這裡壞掉。
    #[test]
    fn a_non_claude_command_does_not_get_one() {
        assert!(!looks_like_claude("codex"));
        assert!(!looks_like_claude("bash -lc 'echo hi'"));
        assert!(!looks_like_claude("claude-code"));
        assert!(!looks_like_claude(""));
    }

    #[test]
    fn launch_command_appends_the_flag_when_a_session_id_is_given() {
        assert_eq!(
            launch_command("claude --verbose", Some("3f2a")),
            "claude --verbose --session-id 3f2a"
        );
    }

    #[test]
    fn launch_command_is_the_command_verbatim_without_a_session_id() {
        assert_eq!(launch_command("codex", None), "codex");
    }
```

- [ ] **Step 2: 跑測試，確認它紅**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::dispatch 2>&1 | tail -20
```

Expected: 編譯失敗，`cannot find function `looks_like_claude``。

- [ ] **Step 3: 實作**

在 `dispatch.rs` 的 `build_prompt` 之後加：

```rust
/// 這個設定值看起來是不是 Claude Code 本身。
///
/// `claude_command` 是使用者可改的設定，而這個檔案的 `NO_TUI_QUIET_MS`
/// 路徑是刻意為「設定成不是全螢幕 TUI 的指令」留的。`--session-id` 是
/// claude 專屬旗標，接到別的指令上會讓它直接啟動失敗——所以只在第一個
/// token 的檔名確實是 `claude` / `claude.exe` 時才給 session id。
///
/// 認錯的代價是不對稱的：漏認只是這張卡片退回舊的 transcript.txt，誤認
/// 是整個派工壞掉。所以比對用相等而不是包含（`claude-code` 必須是 false）。
pub fn looks_like_claude(command: &str) -> bool {
    let Some(first) = command.split_whitespace().next() else {
        return false;
    };
    let name = std::path::Path::new(first)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    name == "claude" || name == "claude.exe"
}

/// 真正要送進終端機的那一行。有 session id 就接上 `--session-id <uuid>`，
/// 沒有就是指令原樣。
///
/// 呼叫端負責先用 `looks_like_claude` 決定要不要給 session id——這裡不再
/// 判斷一次，免得兩處規則漂移。
pub fn launch_command(command: &str, session_id: Option<&str>) -> String {
    match session_id {
        Some(sid) => format!("{command} --session-id {sid}"),
        None => command.to_string(),
    }
}
```

- [ ] **Step 4: 跑測試，確認它綠**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::dispatch 2>&1 | tail -20
```

Expected: 上面六個新測試全綠。

- [ ] **Step 5: `spawn_and_run` 多收一個 session_id**

分頁標題是 `Agent: <command>`（見 `src/components/TerminalApp.tsx:275`）。如果把接好旗標的指令當成 `claude_command` 傳進來，標題就會變成 `Agent: claude --session-id 3f2a-...`。所以旗標要在 `spawn_and_run` 裡面接，事件仍然送原本的指令。

把 `spawn_and_run` 的簽章與前兩行改成：

```rust
pub async fn spawn_and_run(
    app: &AppHandle,
    pty: &PtyManager,
    project_dir: &str,
    claude_command: &str,
    session_id: Option<&str>,
    prompt: &str,
    request_done_marker: bool,
) -> Result<(String, DispatchResult), String> {
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let tab_id = pty
        .create_with_app(app.clone(), size, Some(std::path::PathBuf::from(project_dir)), None)
        .map_err(|e| e.to_string())?;

    // 送進終端機的是接好旗標的版本；下面的事件送的是原本的指令——分頁
    // 標題是 `Agent: <command>`（TerminalApp.tsx），把 UUID 塞進去會讓
    // 每個派工分頁的標題都拖著一串亂碼。
    let launch = launch_command(claude_command, session_id);
    if let Err(e) = pty.write(&tab_id, format!("{launch}\r").as_bytes()) {
        let _ = pty.close(&tab_id);
        return Err(e.to_string());
    }
```

底下 `app.emit("mcp-coordination-tab-spawned", ...)` 那段**不動**（它已經用的是 `claude_command`）。

- [ ] **Step 6: 編譯，確認唯一的呼叫端壞掉**

```bash
cd src-tauri && cargo check 2>&1 | tail -20
```

Expected: `scheduler.rs` 報 `this function takes 7 arguments but 6 arguments were supplied`。那是 Task 6 要修的地方。

- [ ] **Step 7: Commit（連同 Task 6 一起，因為現在編不過）**

先不 commit，直接進 Task 6。

---

## Task 6: `scheduler.rs` — 接線

**Files:**
- Modify: `src-tauri/src/tasks/scheduler.rs`

- [ ] **Step 1: 派工時產生 UUID 並寫回**

在 `RealDispatcher::dispatch` 裡，把取設定與 spawn 的那段改成：

```rust
        let claude_cmd = self.config.get().task_board.claude_command;
        // 只有指令確實是 claude 時才給 session id——別的指令接上這個旗標
        // 會直接啟動失敗（見 dispatch::looks_like_claude）。
        let session_id = dispatch::looks_like_claude(&claude_cmd)
            .then(|| uuid::Uuid::new_v4().to_string());

        let (tab_id, disp) = dispatch::spawn_and_run(
            &self.app,
            &self.pty,
            &task.project_dir,
            &claude_cmd,
            session_id.as_deref(),
            &prompt,
            !task.interactive,
        )
        .await?;
```

接著在 `store::set_tab_id(...)` 之後加：

```rust
        if let Some(sid) = session_id.as_deref() {
            if let Err(e) = store::set_session_id(&project.pool, &task.id, sid).await {
                // 記不起來只代表這張卡片拿不到完整記錄，不值得讓派工失敗。
                eprintln!("set_session_id {}: {e}", task.id);
            }
        }
```

- [ ] **Step 2: 把需要的東西捕獲進 watch 的 async block**

在既有的 `let task_id = task.id.clone();` 附近加兩行：

```rust
        let work_dir = std::path::PathBuf::from(&task.project_dir);
        let session_id_for_watch = session_id.clone();
```

- [ ] **Step 3: 完成時複製 session 記錄**

在 async block 裡，`let transcript = write_transcript(...);` 那一行**之後**、`store::finish_task(...)` 之前插入：

```rust
            // 完整的逐輪記錄。找不到就什麼都不做——上面剛寫好的
            // transcript.txt 還在，沒有東西壞掉。
            if let (Some(sid), Some(root)) =
                (session_id_for_watch.as_deref(), crate::tasks::session_log::claude_projects_root())
            {
                if let Some(path) = crate::tasks::session_log::copy_session_log(
                    &root, &project_path, &task_id, &work_dir, sid,
                ) {
                    let _ = store::set_session_path(&pool, &task_id, &path).await;
                }
            }
```

順序要在 `finish_task` 之前：`finish_task` 之後緊接著 `app.emit("tasks-updated", ())`，前端收到時該列就應該已經是完整的。

- [ ] **Step 4: 編譯並跑全部後端測試**

```bash
cd src-tauri && cargo test 2>&1 | tail -30
```

Expected: 全綠。注意這裡跑的是 `cargo test` 而不是 `cargo test --lib`——`--lib` 不會編譯 `tests/` 底下的整合測試（`task_board.rs` 就在那裡），漏掉會在 CI 上才炸。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/dispatch.rs src-tauri/src/tasks/scheduler.rs
git commit -m "feat(tasks): give each dispatch its own claude session id and keep its log

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

---

## Task 7: `tasks_read_transcript` 優先讀 session.jsonl

簽章不變（仍回傳 `String`），所以 `TranscriptDialog` 與 `useWorkReport` 兩個消費端都不必改。

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs`
- Create: `src-tauri/tests/task_transcript_source.rs`

- [ ] **Step 1: 寫會紅的整合測試**

`tasks_read_transcript` 是 `#[tauri::command]`，需要 `State<ProjectRegistry>`，在單元測試裡不好造。改成把「選哪一份、怎麼渲染」抽成一個純函式，由 command 呼叫，測純函式。

建立 `src-tauri/tests/task_transcript_source.rs`：

```rust
//! `tasks_read_transcript` 的來源優先序：有完整的 session 記錄就用它，
//! 沒有才退回終端機擷取。
//!
//! 見 docs/superpowers/specs/2026-09-07-full-task-transcript-design.md。

use aiterm_lib::commands::tasks::resolve_transcript;

fn write(dir: &std::path::Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.to_string_lossy().into_owned()
}

const ONE_TURN: &str =
    r#"{"type":"user","message":{"role":"user","content":"做這件事"}}"#;

#[test]
fn prefers_the_rendered_session_log() {
    let d = tempfile::tempdir().unwrap();
    let session = write(d.path(), "session.jsonl", ONE_TURN);
    let transcript = write(d.path(), "transcript.txt", "只有最後一屏");

    let out = resolve_transcript(Some(&session), Some(&transcript));
    assert!(out.contains("使用者：做這件事"), "沒有用 session 記錄：{out}");
    assert!(!out.contains("只有最後一屏"), "不該同時吐出兩份：{out}");
}

#[test]
fn falls_back_to_the_terminal_capture_when_there_is_no_session_log() {
    let d = tempfile::tempdir().unwrap();
    let transcript = write(d.path(), "transcript.txt", "只有最後一屏");
    assert_eq!(resolve_transcript(None, Some(&transcript)), "只有最後一屏");
}

/// session_path 有值但檔案被刪掉／讀不到——必須退回，不能回空字串。
#[test]
fn falls_back_when_the_session_file_is_gone() {
    let d = tempfile::tempdir().unwrap();
    let transcript = write(d.path(), "transcript.txt", "只有最後一屏");
    let missing = d.path().join("nope.jsonl").to_string_lossy().into_owned();
    assert_eq!(resolve_transcript(Some(&missing), Some(&transcript)), "只有最後一屏");
}

/// 檔案在、但裡面一句對話都渲染不出來（整份都是 mode / attachment 之類的
/// 雜訊記錄）也要退回。「讀得到檔案」不等於「有內容」。
#[test]
fn falls_back_when_the_session_log_renders_to_nothing() {
    let d = tempfile::tempdir().unwrap();
    let session = write(d.path(), "session.jsonl", "{\"type\":\"mode\"}\n");
    let transcript = write(d.path(), "transcript.txt", "只有最後一屏");
    assert_eq!(resolve_transcript(Some(&session), Some(&transcript)), "只有最後一屏");
}

#[test]
fn returns_empty_when_there_is_nothing_at_all() {
    assert_eq!(resolve_transcript(None, None), "");
}
```

- [ ] **Step 2: 跑測試，確認它紅**

```bash
cd src-tauri && cargo test --test task_transcript_source 2>&1 | tail -20
```

Expected: 編譯失敗，`unresolved import `aiterm_lib::commands::tasks::resolve_transcript``。

- [ ] **Step 3: 實作純函式**

在 `src-tauri/src/commands/tasks.rs` 的 `tasks_read_transcript` **之前**加：

```rust
/// 決定要把哪一份記錄交給前端。
///
/// 有 `session_path` 且讀得出對話 → 渲染完整的逐輪記錄；否則退回
/// `transcript_path`（現行行為）。兩者都沒有就是空字串。
///
/// 退回是安靜的、不報錯：claude 根本沒啟動、卡在信任提示、或使用者把
/// `claude_command` 設成別的東西時，JSONL 不存在，而終端機畫面是唯一的
/// 診斷線索。原本的東西還在，沒有東西壞掉，不值得打斷使用者。
pub fn resolve_transcript(session_path: Option<&str>, transcript_path: Option<&str>) -> String {
    if let Some(p) = session_path {
        if let Ok(raw) = fs::read_to_string(p) {
            let rendered = crate::tasks::session_log::render_session_log(&raw);
            if !rendered.trim().is_empty() {
                return rendered;
            }
        }
    }
    transcript_path
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default()
}
```

- [ ] **Step 4: 讓 command 用它**

把 `tasks_read_transcript` 的 `match` 換成：

```rust
    Ok(resolve_transcript(row.session_path.as_deref(), row.transcript_path.as_deref()))
```

注意這改變了一個既有行為：原本 `transcript_path` 存在但讀取失敗時會回 `Err`，現在回空字串。這是刻意的——讀不到記錄不是操作失敗，對話記錄視窗顯示空白比彈錯誤訊息合理，而且和 `session_path` 那半邊的處理一致。

- [ ] **Step 5: 確認 `commands::tasks` 在 lib 外部可見**

```bash
grep -n "pub mod tasks" src-tauri/src/commands/mod.rs
grep -n "pub mod commands" src-tauri/src/lib.rs
```

Expected: 兩個都是 `pub mod`。若不是，改成 `pub mod`。

- [ ] **Step 6: 跑測試，確認它綠**

```bash
cd src-tauri && cargo test --test task_transcript_source 2>&1 | tail -20
```

Expected: `test result: ok. 5 passed`。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands/tasks.rs src-tauri/tests/task_transcript_source.rs
git commit -m "feat(tasks): read the full session log when a card has one

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

---

## Task 8: 前端型別與 fixture

`TaskRow` 是後端 `SELECT *` 直接序列化過來的，多了兩個欄位就要同步，否則 `tsc -b` 過不了型別檢查（fixture 少欄位）。

**Files:**
- Modify: `src/ipc/tasks.ts`
- Modify: `src/components/TaskBoard/ReportDialog.test.tsx:108`
- Modify: `src/components/TaskBoard/index.test.tsx:61`
- Modify: `src/components/TaskBoard/reportPrompts.test.ts:23`

- [ ] **Step 1: 型別加兩個欄位**

在 `src/ipc/tasks.ts` 的 `TaskRow` 裡，`archived_at` 之後加：

```ts
  /** 這次派工給 `claude --session-id` 的 UUID。指令不是 claude 時是 null。 */
  session_id: string | null;
  /** 複製進卡片資料夾的 session.jsonl 路徑。有值代表有完整的逐輪記錄。 */
  session_path: string | null;
```

- [ ] **Step 2: 跑型別檢查，確認它紅**

```bash
npx tsc -b
```

Expected: 三處 fixture 各報一個 `Property 'session_id' is missing in type ...`。

（注意用 `npx tsc -b`，不是 `tsc --noEmit`：根 `tsconfig.json` 是 solution file（`"files": []`），`--noEmit` 什麼都不檢查、永遠 exit 0。）

- [ ] **Step 3: 補三處 fixture**

`src/components/TaskBoard/ReportDialog.test.tsx:108`：

```ts
    ai_summary: null, archived_at: null, session_id: null, session_path: null,
    attachments: [], ...over,
```

`src/components/TaskBoard/index.test.tsx:61`：

```ts
  finished_at: null, ai_summary: null, archived_at: null,
  session_id: null, session_path: null, attachments: [],
```

`src/components/TaskBoard/reportPrompts.test.ts:23`：

```ts
  archived_at: null,
  session_id: null,
  session_path: null,
```

- [ ] **Step 4: 型別檢查 + 前端測試**

```bash
npx tsc -b && npm run test 2>&1 | tail -20
```

Expected: 型別檢查無輸出（成功），Vitest 全綠。

- [ ] **Step 5: Commit**

```bash
git add src/ipc/tasks.ts src/components/TaskBoard/ReportDialog.test.tsx \
        src/components/TaskBoard/index.test.tsx src/components/TaskBoard/reportPrompts.test.ts
git commit -m "feat(tasks): mirror the session-log columns in the frontend TaskRow

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

---

## Task 9: 信任畫面 — 算出要送哪些按鍵

沒信任過的目錄會讓 `claude` 停在「Quick safety check」畫面，直到卡住偵測介入才收掉——期間卡片看起來像在執行，實際上什麼都沒發生。

**這個任務的兩個關鍵事實都是實測來的，見計畫開頭：**

1. PTY 輸出裡**一個空白字元都沒有**，所以比對前必須濾掉所有空白。
2. 預設選中的是 `No, exit`，而且選項順序**可能對調**——所以不能寫死按鍵次數，要在畫面上定位游標與 `Yes` 選項的相對位置再算。

明確**不採用**改寫 `~/.claude.json` 的做法：那是 Claude Code 與 AITerm 共用的設定檔（本機已有 50 個專案的資料在裡面），兩邊同時寫有損毀整份設定的風險；這個 repo 過去就因為盲目寫入使用者正在編輯的設定檔，造成整份設定解析失敗。

**Files:**
- Modify: `src-tauri/src/tasks/dispatch.rs`

- [ ] **Step 1: 放進真實 fixture 並寫會紅的測試**

在 `dispatch.rs` 的 `mod tests` 裡加：

```rust
    /// 真實抓下來的信任畫面：實跑 `claude` 進一個未信任的資料夾、擷取原始
    /// PTY bytes、過 `strip_ansi` 之後的結果。
    ///
    /// 字與字之間真的沒有空白——Claude Code 的 TUI 用游標移動排版，整份
    /// 輸出裡一個 0x20 都沒有（實測）。所以 `contains("Yes, I trust this
    /// folder")` 這種帶空白的比對永遠不會命中。
    const REAL_TRUST_SCREEN: &str = "\
────────────────────────────────────────────────────────────────────────────────
Accessingworkspace:

/private/tmp/probe/trustprobe-23565

Quicksafetycheck:Isthisaprojectyoucreatedoroneyoutrust?(Likeyour
owncode,awell-knownopensourceproject,orworkfromyourteam).Ifnot,
takeamomenttoreviewwhat'sinthisfolderfirst.

ClaudeCode'llbeabletoread,edit,andexecutefileshere.

Securityguide

❯No,exit
Yes,Itrustthisfolder

Entertoconfirm·Esctocancel
";

    const DOWN: &[u8] = b"\x1b[B";
    const UP: &[u8] = b"\x1b[A";

    /// 真實畫面：游標在 No 上、Yes 在下一行 → 往下一次再 Enter。
    #[test]
    fn moves_down_to_reach_yes_on_the_real_screen() {
        assert_eq!(
            trust_prompt_keys(REAL_TRUST_SCREEN),
            Some([DOWN, b"\r"].concat())
        );
    }

    /// 選項對調（Yes 在上、游標停在下面的 No）→ 必須往**上**。
    /// 寫死「往下一次」的實作會在這裡按到 No, exit，把派工直接殺掉——
    /// 這個測試就是為了擋那件事而存在的。
    #[test]
    fn moves_up_when_the_options_are_swapped() {
        let swapped = REAL_TRUST_SCREEN
            .replace("❯No,exit\nYes,Itrustthisfolder", "Yes,Itrustthisfolder\n❯No,exit");
        assert_eq!(trust_prompt_keys(&swapped), Some([UP, b"\r"].concat()));
    }

    /// 游標已經停在 Yes 上 → 只送 Enter，一次都不要動。
    #[test]
    fn just_confirms_when_the_cursor_is_already_on_yes() {
        let already = REAL_TRUST_SCREEN
            .replace("❯No,exit\nYes,Itrustthisfolder", "No,exit\n❯Yes,Itrustthisfolder");
        assert_eq!(trust_prompt_keys(&already), Some(b"\r".to_vec()));
    }

    /// 不是信任畫面就什麼都不送。
    #[test]
    fn sends_nothing_on_an_ordinary_screen() {
        assert_eq!(trust_prompt_keys("$ ls\nsrc  target\n"), None);
        assert_eq!(trust_prompt_keys(""), None);
    }

    /// 認得出 Yes 那一行、卻找不到游標（UI 改版換掉了 `❯`）→ 什麼都不送。
    /// 猜一個方向亂按有可能按到 No, exit；不送最壞只是退回現行行為
    /// （卡住偵測收掉），永遠不會弄壞東西。
    #[test]
    fn sends_nothing_when_the_cursor_cannot_be_located() {
        let no_cursor = REAL_TRUST_SCREEN.replace('❯', " ");
        assert_eq!(trust_prompt_keys(&no_cursor), None);
    }

    /// 只是有人在聊天裡提到這句話，不該被當成信任畫面——畫面上必須同時
    /// 有游標與那一整行選項。
    #[test]
    fn does_not_fire_on_a_mention_of_the_phrase_in_prose() {
        let prose = "❯somethingelse\nItoldyouYes,Itrustthisfolderisthewording\n";
        assert_eq!(trust_prompt_keys(prose), None);
    }
```

- [ ] **Step 2: 跑測試，確認它紅**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::dispatch::tests::trust 2>&1 | tail -20
```

Expected: 編譯失敗，`cannot find function `trust_prompt_keys``。

- [ ] **Step 3: 實作**

在 `dispatch.rs` 的 `launch_command` 之後加：

```rust
/// 信任畫面上「我信任這個資料夾」那一個選項的文字，**已去掉所有空白**。
/// Claude Code 的 TUI 用游標移動排版，PTY 輸出裡一個空白位元組都沒有
/// （實測），所以比對只能在去空白之後做。
const TRUST_YES_OPTION: &str = "Yes,Itrustthisfolder";
/// 目前選中的那一行的游標符號。
const TRUST_CURSOR: char = '❯';

/// 這個畫面停在資料夾信任提示上時，回傳「把選擇移到『我信任這個資料夾』
/// 並確認」要送的按鍵；不是那個畫面、或定位不到就回 `None`。
///
/// `screen` 是 `PtyManager::get_recent_output` 的輸出（ANSI 已剝除）。
///
/// 為什麼不寫死「往下一次再 Enter」：預設選中的是 `No, exit`，而選項順序
/// 隨時可能被 Claude Code 改掉或對調。寫死方向的實作在那一天會反過來按到
/// `No, exit`，把使用者的派工直接殺掉。改成在畫面上同時定位游標行與 Yes
/// 那一行、算相對位移，任何一個找不到就回 `None`——什麼都不送，退回現行
/// 行為（卡住偵測收掉）。風險因此從「可能誤殺派工」降到「可能不生效」。
pub fn trust_prompt_keys(screen: &str) -> Option<Vec<u8>> {
    // 去掉所有空白之後才比對；空行整行丟掉，這樣「游標行」與「Yes 行」
    // 的距離就是實際的按鍵次數，不會被排版用的空行灌水。
    let lines: Vec<String> = screen
        .lines()
        .map(|l| l.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        .filter(|l| !l.is_empty())
        .collect();

    // 選項那一行就只有選項本身（可能前面帶游標符號），不是夾在句子裡的
    // 一段話——後者代表有人剛好提到這句話，不是信任畫面。
    let yes = lines.iter().position(|l| {
        l.trim_start_matches(TRUST_CURSOR) == TRUST_YES_OPTION
    })?;
    let cursor = lines.iter().position(|l| l.starts_with(TRUST_CURSOR))?;

    let mut keys = Vec::new();
    let (step, times) = if yes >= cursor {
        (&b"\x1b[B"[..], yes - cursor)
    } else {
        (&b"\x1b[A"[..], cursor - yes)
    };
    for _ in 0..times {
        keys.extend_from_slice(step);
    }
    keys.extend_from_slice(b"\r");
    Some(keys)
}
```

- [ ] **Step 4: 跑測試，確認它綠**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::dispatch 2>&1 | tail -20
```

Expected: 六個新測試全綠。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/dispatch.rs
git commit -m "feat(tasks): work out which keys accept Claude Code's folder-trust prompt

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

---

## Task 10: 把信任處理接進 `wait_until_settled`

信任畫面出現時，`\x1b[?1049h` 已經送過了（TUI 起來了）、畫面也已經安靜——所以現行的 `wait_until_settled` 會判定「settled」並把提示詞打進信任對話框裡。信任檢查必須排在 settled 判斷**之前**。

**Files:**
- Modify: `src-tauri/src/tasks/dispatch.rs`

- [ ] **Step 1: 寫會紅的測試**

在 `dispatch.rs` 的 `mod tests` 裡加：

```rust
    /// 停在信任畫面時不可以判定 settled——那樣會把提示詞打進信任對話框裡。
    ///
    /// 手法照抄同檔案的 `run_on_session_sends_a_multiline_prompt_verbatim_
    /// then_a_standalone_cr`：把 pty 切進 raw 模式關掉回音，再 `od` 讀出
    /// 我們實際寫進去的位元組。不看「回音」是因為 canonical 模式的終端機會
    /// 把 ESC 顯示成 `^[`，那樣斷言驗到的是終端機的顯示規則，不是我們寫了
    /// 什麼——見 feedback「沒有失敗訊號不等於正確」。
    ///
    /// 畫面先印替代畫面序列讓 `tui_started` latch 起來（模擬 claude 已經
    /// 啟動），再印真實信任畫面的三行。`printf '%b\n'` 而不是 `'%s\n'`：
    /// `%s` 不處理跳脫序列，`\342\235\257`（❯ 的 UTF-8）會被原樣印出來，
    /// 游標行就永遠定位不到。
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn accepts_the_trust_prompt_before_declaring_the_session_settled() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(settle_size(), |_| {}).unwrap();

        // `'MARK''READY'` 用串接寫，這樣被回音出來的指令本身不含連續的
        // marker（同上，照抄那個測試的手法）。N = 4：`\x1b[B` 加 `\r`。
        #[cfg(not(windows))]
        pty.write(
            &tab,
            b"stty raw -echo; printf '\\033[?1049h'; \
              printf '%b\\n' 'Quicksafetycheck:' '\\342\\235\\257No,exit' 'Yes,Itrustthisfolder'; \
              printf 'MARK''READY'; od -An -tx1 -N 4\n",
        )
        .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let out = pty.get_recent_output(&tab, 16 * 1024).unwrap_or_default();
            if out.contains("MARKREADY") {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "信任畫面沒有印出來：{out}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let _ = tokio::time::timeout(
            Duration::from_millis(NO_TUI_QUIET_MS + 2_000),
            wait_until_settled(&pty, &tab),
        )
        .await;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let hex: Vec<String> = loop {
            let out = pty.get_recent_output(&tab, 16 * 1024).unwrap_or_default();
            let after = out.split("MARKREADY").nth(1).unwrap_or("").to_string();
            let hex: Vec<String> = after
                .split_whitespace()
                .filter(|t| t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_string)
                .collect();
            if hex.len() >= 4 {
                break hex;
            }
            assert!(tokio::time::Instant::now() < deadline, "沒有送出任何按鍵：{out}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        // ESC [ B CR — 往下一格（Yes 在 No 下面）再確認。
        assert_eq!(&hex[..4], ["1b", "5b", "42", "0d"], "送出的按鍵不對：{hex:?}");
    }
```

- [ ] **Step 2: 跑測試，確認它紅**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::dispatch::tests::accepts_the_trust 2>&1 | tail -20
```

Expected: 斷言失敗，`沒有送出移動到 Yes 的按鍵`。

- [ ] **Step 3: 實作**

把 `wait_until_settled` 整個換成：

```rust
async fn wait_until_settled(pty: &PtyManager, tab_id: &str) {
    let mut deadline = tokio::time::Instant::now() + Duration::from_millis(SETTLE_TIMEOUT_MS);
    let mut tui_started = false;
    // 只送一次。信任畫面的位元組會留在輸出環裡，接受之後再比對還是會命中，
    // 不 latch 就會一直重送按鍵。
    let mut trust_handled = false;
    loop {
        // Latch it: the raw ring is bounded, and a chatty TUI can push the
        // sequence out of the window it was found in.
        if !tui_started {
            tui_started = pty
                .get_recent_raw(tab_id, 256 * 1024)
                .is_some_and(|b| {
                    b.windows(ALT_SCREEN_ENTER.len()).any(|w| w == ALT_SCREEN_ENTER)
                });
        }

        // 信任提示的檢查一定要排在 settled 判斷之前：那個畫面出現時 TUI
        // 已經啟動、畫面也已經安靜，兩個條件都成立，先判 settled 就會把
        // 提示詞打進信任對話框裡。
        if !trust_handled {
            if let Some(keys) =
                pty.get_recent_output(tab_id, 16 * 1024).as_deref().and_then(trust_prompt_keys)
            {
                let _ = pty.write(tab_id, &keys);
                trust_handled = true;
                // 接受之後 claude 才真正開始啟動，重新給它完整的等待預算。
                deadline = tokio::time::Instant::now() + Duration::from_millis(SETTLE_TIMEOUT_MS);
                tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
                continue;
            }
        }

        let quiet = pty.ms_since_output(tab_id).unwrap_or(u64::MAX);
        let settled = if tui_started { quiet >= SETTLE_QUIET_MS } else { quiet >= NO_TUI_QUIET_MS };
        if settled || tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}
```

同時把這個函式的文件註解補上一段（接在既有那段之後）：

```rust
/// 另外在同一個迴圈裡順便處理資料夾信任提示。沒信任過的目錄會讓 `claude`
/// 停在「Quick safety check」畫面上，而那時替代畫面序列早就送過、畫面也
/// 安靜了——兩個條件都成立，所以不特別處理的話提示詞會被打進對話框裡，
/// 卡片看起來在執行、實際上什麼都沒發生。見 `trust_prompt_keys`。
```

- [ ] **Step 4: 跑測試，確認它綠，而且既有的三個 settle 測試沒被弄壞**

```bash
cd src-tauri && cargo test --package aiterm --lib tasks::dispatch 2>&1 | tail -25
```

Expected: 全綠，特別確認 `does_not_settle_while_the_tui_has_not_started_yet`、`settles_once_the_tui_is_up_and_quiet`、`a_non_tui_command_still_settles_on_the_longer_quiet_window` 三個都還在。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/dispatch.rs
git commit -m "fix(tasks): accept the folder-trust prompt instead of typing into it

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

---

## Task 11: 全面驗證與收尾

- [ ] **Step 1: 完整測試套件**

```bash
npx tsc -b
npm run lint
npm run test 2>&1 | tail -20
cd src-tauri && cargo test 2>&1 | tail -30
```

Expected: 四個都成功。`cargo test` 不加 `--lib`——`--lib` 不編譯 `tests/` 底下的整合測試。

- [ ] **Step 2: 跑 `cargo test` 之前先確認 sidecar 已備好**

`tauri-build` 的 `build.rs` 會在編譯期驗證每一個 `externalBin` 都在磁碟上，所以沒先跑過對應平台的 `scripts/setup-uv-*` 的話連 `cargo check` 都會失敗。若 Step 1 卡在 missing resource path：

```bash
ls src-tauri/binaries/
```

缺的話跑 `scripts/setup-uv-mac.sh`（或對應平台的那支）。

- [ ] **Step 3: 真機驗證**

```bash
npm run tauri:dev
```

驗這四件事：

1. 派一張新卡片到「待執行」，等它跑完，打開對話記錄視窗——內容應該是逐輪的 `使用者：` / `Claude：` / `〔工具〕`，而不是一屏 TUI 外框。
2. 確認卡片資料夾裡兩份都在：`ls <專案>/tasks/<卡片id>/` 應該同時看到 `transcript.txt` 與 `session.jsonl`。
3. 產一份工作報告，確認摘要講的是整段工作而不只是最後一屏。
4. 分頁標題仍然是 `Agent: claude`，**沒有**拖著 `--session-id <uuid>`。

- [ ] **Step 4: 真機驗證信任提示（要一個沒信任過的資料夾）**

```bash
mkdir -p ~/tmp/trust-check-$(date +%s)
```

把新建的那個資料夾設成一張卡片的工作目錄、派工，確認它不會停在「Quick safety check」畫面，而是自己選了「Yes, I trust this folder」繼續跑。

（這一步無法自動化——`trust_prompt_keys` 的單元測試證明的是按鍵算得對，證不了 Claude Code 的 TUI 收到那些按鍵後真的會照做。）

- [ ] **Step 5: 更新 spec 狀態**

把 `docs/superpowers/specs/2026-09-07-full-task-transcript-design.md` 的 `**狀態**` 從「設計已確認，待寫實作計畫」改成「已實作」，並在「已驗證的事實」那一節後面補一小段，記下寫計畫時新測到的三件事（PTY 沒有空白字元、預設選 No 且順序可能對調、`claude_command` 可能不是 claude）。

- [ ] **Step 6: 把 spec 的狀態改動加進版控**

spec 與這份計畫在寫計畫時就已經進版控了（`.gitignore:47` 的 `docs` 規則會擋掉它們，當時是用 `git add -f` 強制加的）。Step 5 改的狀態行同樣要 `-f`：

```bash
git add -f docs/superpowers/specs/2026-09-07-full-task-transcript-design.md
git status --short
```

`git status --short` 確認只有預期的檔案——**不要用 `git add -A`**。

- [ ] **Step 7: 寫 CHANGELOG**

在 `CHANGELOG.md` 開頭加一段新版本（版號沿用 `package.json` 的下一個 patch/minor，跟過去的慣例一致），內容寫給使用者看：

- 工作看板的對話記錄現在是完整的逐輪內容，不再只有終端機的最後一個畫面
- 工作報告的摘要因此看得到整段工作，而不是只看到結尾
- 派工到沒信任過的資料夾不會再卡在「Quick safety check」提示畫面上

不要寫成 commit 標題堆出來的開發者日誌——沒寫這一段的話，發版 workflow 會退回用 commit 標題自動生成。

- [ ] **Step 8: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: spec、實作計畫與 CHANGELOG

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017K4djFzy16JuJyNZNGMmSo"
```

**不要自己打 tag。** 推 `vX.Y.Z` tag 會觸發三平台的 release build，一定要先問過使用者。

---

## 不在這次範圍（照 spec）

- 舊卡片的回溯補齊（沒有 `session_id`，無從對應）
- 保留多次執行的歷史記錄（重新派工直接覆蓋 `session.jsonl`）
- 對話記錄視窗的排版重做（仍是純文字 `pre` 區塊）
- 大檔案的分頁載入（實測量級用不到，真的遇到再說）
- 子代理記錄的特別處理（實測 `isSidechain` 在主 session 檔裡永遠是 false，不需要）
