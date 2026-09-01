# /ai + /agent 感知 ego-browser CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓驅動 `/ai`、`/agent` 的 AI 在偵測到本機裝有 `ego-browser` CLI 時，收到一句
輕量提示，讓它自己執行 `ego-browser --help` 學會怎麼用，藉此完成瀏覽器自動化任務。

**Architecture:** `/ai` 和 `/agent` 目前共用同一個系統提示產生函式
`build_single_command_prompt`（`src-tauri/src/commands/ai.rs:115`）。新增一個
`PATH` 掃描 + `OnceLock` 快取的偵測函式，偵測到時讓
`build_single_command_prompt` 多附加一段固定文字；偵測不到則完全不影響現有輸出。
不動前端任何程式碼、不改 IPC 簽名。

**Tech Stack:** Rust（`src-tauri`），既有的 `cargo test` 單元測試（`tempfile` 已是
dev-dependency）。

---

## 這份計畫涵蓋的 spec 章節

對應 `docs/superpowers/specs/2026-09-01-ego-browser-agent-awareness-design.md`：

- 設計「1. 偵測」→ Task 1
- 設計「2. Prompt 注入」→ Task 2
- 設計「3. 執行流程」不需要改動任何程式碼（現有 `runAgentLoop`/`handleAiQuery` 原樣沿用），
  無對應 Task；Task 2 完成後即為終態。
- 「明確排除」章節（Settings 開關、AiPanel、硬編完整文件、改 `ai_query` IPC 簽名）——
  本計畫刻意不涵蓋，符合 spec 的排除範圍。

---

### Task 1: `ego-browser` 的 PATH 偵測

**Files:**
- Modify: `src-tauri/src/commands/ai.rs:111`（在 `extract_json_from_response` 結尾
  之後、`build_single_command_prompt` 的 doc comment 之前插入新函式）
- Modify: `src-tauri/src/commands/ai.rs:1081`（在 `mod tests` 收尾 `}` 之前插入新測試）
- Test: `src-tauri/src/commands/ai.rs`（inline `#[cfg(test)] mod tests`，沿用既有慣例）

- [ ] **Step 1: 寫兩個會失敗（無法編譯，因為函式還不存在）的測試**

在 `src-tauri/src/commands/ai.rs` 第 1080 行（`prompt_truncates_long_recent_output`
測試結尾的 `}`）之後、第 1081 行（`mod tests` 收尾的 `}`）之前，插入：

```rust

    #[test]
    fn find_on_path_true_when_executable_present() {
        let dir = tempfile::tempdir().unwrap();
        let exe_name = if cfg!(windows) { "ego-browser.exe" } else { "ego-browser" };
        std::fs::write(dir.path().join(exe_name), b"").unwrap();
        let path_var = std::env::join_paths([dir.path()]).unwrap();
        assert!(find_on_path("ego-browser", &path_var));
    }

    #[test]
    fn find_on_path_false_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path_var = std::env::join_paths([dir.path()]).unwrap();
        assert!(!find_on_path("ego-browser", &path_var));
    }
```

- [ ] **Step 2: 執行測試，確認因為 `find_on_path` 未定義而編譯失敗**

Run: `cd src-tauri && cargo test --lib commands::ai::tests::find_on_path -- --nocapture`
Expected: 編譯錯誤，訊息包含 `cannot find function `find_on_path` in this scope`

- [ ] **Step 3: 實作 `find_on_path` 與 `ego_browser_available`**

在 `src-tauri/src/commands/ai.rs` 第 111 行（`extract_json_from_response` 的收尾
`}`）之後、第 112 行原本的空行與 `build_single_command_prompt` doc comment 之前，插入：

```rust

/// True when `program` exists as an executable file somewhere in `path_var`
/// (a raw `PATH`-style list). Pure and dependency-free — takes the path list
/// as a parameter instead of reading the real `PATH` env var directly — so
/// it can be unit-tested with a fake directory without mutating global
/// process state. `ego_browser_available` below is the cached wrapper that
/// reads the real `PATH`.
fn find_on_path(program: &str, path_var: &std::ffi::OsStr) -> bool {
    let exe_name = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_string()
    };
    std::env::split_paths(path_var).any(|dir| dir.join(&exe_name).is_file())
}

/// Whether the `ego-browser` CLI (ego lite's AI-agent browser automation
/// tool) is installed and on `PATH`. Checked once per process and cached in
/// a `OnceLock` — `build_single_command_prompt` calls this on every
/// `/ai`/`/agent` query, and re-scanning `PATH` every time would be wasteful.
fn ego_browser_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::env::var_os("PATH")
            .map(|path| find_on_path("ego-browser", &path))
            .unwrap_or(false)
    })
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib commands::ai::tests::find_on_path`
Expected: `test commands::ai::tests::find_on_path_true_when_executable_present ... ok`
和 `test commands::ai::tests::find_on_path_false_when_absent ... ok`，共 2 個測試通過。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "feat(ai): detect ego-browser CLI on PATH

Adds a cached PATH scan for the ego-browser (ego lite) automation CLI,
used by the next commit to let /ai and /agent prompts mention it when
installed."
```

---

### Task 2: 把偵測結果注入 `build_single_command_prompt`

**Files:**
- Modify: `src-tauri/src/commands/ai.rs:115`（函式簽名與 body）
- Modify: `src-tauri/src/commands/ai.rs:244`（唯一的正式呼叫點，在 `run_single_command`
  裡）
- Modify: `src-tauri/src/commands/ai.rs:956,973,987,995,1074`（5 個既有測試呼叫點，
  補上第三個參數）
- Test: `src-tauri/src/commands/ai.rs`（新增 2 個測試）

- [ ] **Step 1: 修改函式簽名與 body，新增條件式附加段落**

把第 115-155 行整個函式（目前簽名是
`pub fn build_single_command_prompt(snapshot: &crate::ai::EnvSnapshot, locale: Locale) -> String`）
改成：

```rust
pub fn build_single_command_prompt(
    snapshot: &crate::ai::EnvSnapshot,
    locale: Locale,
    ego_browser_available: bool,
) -> String {
    let recent_section = snapshot.recent_output.as_deref().map(|o| {
        let trimmed = if o.len() > 2000 { &o[o.len() - 2000..] } else { o };
        format!("\nRecent terminal output (last ~50 lines):\n```\n{trimmed}\n```")
    }).unwrap_or_default();

    let dir_section = snapshot.dir_listing.as_deref().map(|d| {
        format!("\nDirectory listing ({}):\n```\n{d}\n```", snapshot.cwd.display())
    }).unwrap_or_default();

    let ego_browser_section = if ego_browser_available {
        "\n\nA CLI tool called `ego-browser` is available on this machine for browser \
automation (opening pages, clicking, filling forms, screenshots, extracting page \
content). If the current task needs it, first run `ego-browser --help` and follow \
what it tells you before composing further commands."
    } else {
        ""
    };

    format!(
r#"You are an AI command generator for a cross-platform terminal application.
Your only job is to translate the user's natural-language request (or execution goal) into ONE
executable shell command for their current environment.

Environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}            (may be slightly stale; prefer relative paths or
                         shell variables over hardcoded absolute paths){recent_section}{dir_section}

Rules:
1. Output ONLY a JSON object, no prose, no markdown fences, no extra keys.
2. Schema:
   {{
     "explanation": "one-sentence description of what this command does, or a summary of the completed result (use {language})",
     "command":     "a single shell command, no prompt prefix, no line breaks. SET TO 'DONE' IF GOAL IS FULLY MET.",
     "risk_level":  one of "safe", "needs_confirm", "dangerous"
   }}
3. The command must be syntactically valid for {shell}. Do not mix shells.
4. If the request cannot be satisfied with one command, pick the most useful
   single command to progress further.
5. If the user provides an execution history and it shows their ultimate goal is achieved, you MUST set "command" to "DONE".
6. Never produce destructive operations against system roots. If the user
   explicitly asks for one, set risk_level="dangerous".{ego_browser_section}"#,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display(),
        language = crate::ai::language_name(locale),
    )
}
```

（唯一的實質變動：新增 `ego_browser_available: bool` 參數、新增
`ego_browser_section` 區塊、在 format! 樣板結尾的 `"dangerous".` 之後接上
`{ego_browser_section}`。其餘文字逐字不變。）

- [ ] **Step 2: 更新唯一的正式呼叫點**

在 `run_single_command`（第 244 行）把：

```rust
    let prompt = build_single_command_prompt(&snapshot, locale);
```

改成：

```rust
    let prompt = build_single_command_prompt(&snapshot, locale, ego_browser_available());
```

- [ ] **Step 3: 更新 5 個既有測試呼叫點，補上 `false`**

在 `mod tests` 裡，把這 5 處（`prompt_contains_environment_fields`、
`prompt_includes_recent_output_when_present`、`prompt_includes_dir_listing_when_present`、
`prompt_omits_context_sections_when_none`、`prompt_truncates_long_recent_output`）裡的

```rust
        let prompt = build_single_command_prompt(&snap, Locale::ZhTw);
```

全部改成：

```rust
        let prompt = build_single_command_prompt(&snap, Locale::ZhTw, false);
```

（這 5 個測試本來就沒有涉及 ego-browser，補 `false` 維持它們原本測的行為不變。）

- [ ] **Step 4: 新增兩個測試，驗證條件式附加**

緊接在 Task 1 新增的 `find_on_path_false_when_absent` 測試之後（仍在 `mod tests`
收尾 `}` 之前）插入：

```rust

    #[test]
    fn prompt_appends_ego_browser_hint_when_available() {
        let snap = make_snap("macos", "zsh", "/home/u");
        let prompt = build_single_command_prompt(&snap, Locale::ZhTw, true);
        assert!(prompt.contains("ego-browser"));
        assert!(prompt.contains("--help"));
    }

    #[test]
    fn prompt_omits_ego_browser_hint_when_unavailable() {
        let snap = make_snap("macos", "zsh", "/home/u");
        let prompt = build_single_command_prompt(&snap, Locale::ZhTw, false);
        assert!(!prompt.contains("ego-browser"));
    }
```

- [ ] **Step 5: 執行完整測試，確認全部通過**

Run: `cd src-tauri && cargo test --lib commands::ai::`
Expected: 全部通過，無失敗。特別確認舊有 5 個測試（Step 3 改過呼叫點的）依然是
`ok`，代表補上 `false` 沒有意外改變它們原本驗證的內容。

- [ ] **Step 6: 型別檢查整個 crate**

Run: `cd src-tauri && cargo check`
Expected: 無錯誤（`build_single_command_prompt` 在整個 crate 裡只有這 6 個呼叫點
——1 個正式呼叫點 + 5 個既有測試呼叫點——全部已在前面步驟更新過，不應該再有其他
呼叫點觸發型別錯誤）。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "feat(ai): mention ego-browser CLI in /ai and /agent prompts when installed

build_single_command_prompt now appends a one-line hint pointing the
model at \`ego-browser --help\` when the CLI is detected on PATH. /ai and
/agent share this prompt builder, so both pick it up without any other
change; nothing changes when ego-browser isn't installed."
```

---

## 完成後的驗證（無法自動化，需要真機手動確認）

這兩個 Task 完成、`cargo test`/`cargo check` 都綠燈之後，程式碼層面已符合 spec。
以下是 spec 裡明確標註「需要真機驗證」的部分，Task 執行者完成上面兩個 Task 後
應該提醒使用者親自做：

1. 在裝有 `ego-browser` 的機器上開一個 AITerm 分頁，打 `/agent 幫我去 example.com 截圖`
   之類的瀏覽器任務，觀察 model 是否真的會在某一步下 `ego-browser --help`、後續是否
   組得出可執行的 `ego-browser nodejs` heredoc 指令並成功操作瀏覽器。這取決於使用者
   設定的 provider/model 本身的能力，程式碼這邊只負責把提示放進 prompt。
2. 在沒有裝 `ego-browser` 的環境（或暫時把它從 PATH 移掉）跑同樣的 `/ai`/`/agent`
   查詢，確認完全沒有提到 ego-browser、行為與這次改動之前一致。
