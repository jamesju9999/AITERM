# /ai + /agent 感知 ego-browser CLI Design Spec

## 背景與問題

使用者本機裝了 `ego-browser`（ego lite）——一個給 AI agent 用的瀏覽器自動化 CLI，
用法是 `ego-browser nodejs <<'EOF' ...script... EOF`（把 Node.js heredoc 腳本丟給它，
內建 `snapshotText`/`click`/`fillInput`/`captureScreenshot` 等 helper 操作真實瀏覽器）。
這套工具目前只有 Claude Code 這類外部 coding agent 看得到它的用法文件
（`~/.claude/skills/ego-browser/SKILL.md`，symlink 到 ego lite 自己安裝的
`~/.local/share/ego/ego-skills/SKILL.md`）。

AITerm 自己的 `/ai`、`/agent` 由使用者設定的 provider（OpenAI/Anthropic/Ollama…）驅動，
這些 model 完全沒看過那份 SKILL.md，不會自己想到要組 `ego-browser nodejs` heredoc。
但 `/ai`、`/agent` 本來就能執行任意 shell 指令——只要 model 知道這個工具存在、知道
第一步該做什麼，就能自己用起來。

## 現況調查（決定設計形狀的關鍵事實）

1. **`/ai` 和 `/agent` 目前是同一套機制**：`src/components/TerminalView.tsx` 的四個觸發
   點（862、1006、1599、2159 行）裡，`parseAgentPrefix`/`parseAiPrefix` 解析完之後合併成
   同一個 `finalQuery` 字串，兩者都呼叫 `startMission(finalQuery, 5)` + `runAgentLoop(...)`
   ——完全相同的呼叫方式、相同的 `maxSteps=5`，之後完全沒有欄位可以分辨這次是 `/ai` 還是
   `/agent` 觸發的。也就是說 `/ai` 現在不是「單發建議」，而是跟 `/agent` 一樣最多 5 步的
   自主迴圈（`CLAUDE.md` 對 `/ai` 流程的描述已經跟目前程式碼不一致）。
2. **兩者共用同一個 prompt builder**：`src-tauri/src/commands/ai.rs:115`
   `build_single_command_prompt(snapshot, locale)`，透過 `ai_query` command
   （`ai.rs:333-344`）被 `run_single_command` 呼叫，是 `/ai`、`/agent` 每一步生成 shell
   指令唯一共用的系統提示來源。改這一處，`/ai`、`/agent` 同時生效，不需要新增參數區分
   兩者。
3. **`ego-browser --help` 本身就是自我文件化的入口**：實測輸出結尾會印
   ```
   To AI Agent Read
   Please first read and follow the Codex Skill document at:
   ~/.agents/skills/ego-browser/SKILL.md
   ...
   ```
   `~/.agents/skills/ego-browser` 這個 symlink 是 ego lite 安裝時就建立好的通用路徑
   （非 Claude 專屬），任何 agent 執行 `ego-browser --help` 都能拿到這個指引。這代表
   AITerm 不需要在 code 裡硬編一份用法說明——那份 SKILL.md 有自己的版號（目前
   `1.2.3`）會持續更新，硬編會有過時風險。
4. **PATH 偵測已有可仿照的既有實作**：`src-tauri/src/vcs/svn.rs:22-45` 的 `svn_program()`
   /`on_current_path()` 用 `OnceLock` 快取「掃 `PATH` 找執行檔」的結果，Windows 額外查
   `.exe` 後綴。本次沿用同一種寫法，但不需要 `svn.rs` 那種額外的安裝路徑 fallback
   （`ego-browser` 是使用者自己裝的個人工具，不像 svn 常見裝了卻沒進 PATH）。

## 設計

### 1. 偵測：`ego-browser` 是否在 PATH 上

新增一個小函式（建議放在 `src-tauri/src/commands/ai.rs` 或旁邊一個新的小模組，例如
`src-tauri/src/tools_detect.rs`——視現有檔案大小決定，若 `ai.rs` 已經很長就獨立成新檔），
仿照 `svn.rs` 的 `on_current_path` 邏輯：

```rust
fn ego_browser_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| on_current_path("ego-browser"))
}
```

`on_current_path` 掃 `PATH` 環境變數逐一比對檔名是否存在（Windows 找 `ego-browser.exe`），
純檔案系統查詢，不 spawn 子行程。整個 process 生命週期只查一次並快取，`/ai`、`/agent`
每次呼叫 `build_single_command_prompt` 都直接讀快取結果，不影響現有效能。

### 2. Prompt 注入：`build_single_command_prompt`

在 `ai.rs:115-155` 現有的 `format!(...)` 系統提示尾端，偵測到 `ego-browser` 時額外附加
一小段固定文字（未偵測到則完全不附加，維持現狀）：

```
A CLI tool called `ego-browser` is available on this machine for browser
automation (opening pages, clicking, filling forms, screenshots, extracting
page content). If the current task needs it, first run `ego-browser --help`
and follow what it tells you before composing further commands.
```

不做關鍵字比對（例如查詢文字裡有沒有「瀏覽器」「網站」「URL」）來決定要不要加這段——
純粹「有裝就一定加、沒裝就一定不加」，理由：這段文字只有一行，token 成本可忽略，關鍵字
判斷法容易漏判（例如查詢沒提到「瀏覽器」但其實需要），維持邏輯最簡單。

### 3. 執行流程（不需要改 `runAgentLoop`/action 機制）

`/agent`（或表現得跟它一樣的 `/ai`）第一步收到帶有上述提示的 prompt 後，預期行為：

1. Model 判斷任務需要瀏覽器操作，第一步下 `ego-browser --help`。
2. `runAgentLoop` 照現有機制把這一步的 stdout（含「請讀 SKILL.md」的指引）疊進歷史，
   餵給下一步。
3. Model 接著可能下 `cat ~/.agents/skills/ego-browser/SKILL.md`（或直接照 `--help`
   輸出裡的其他線索）取得完整用法，之後才組出真正的
   `ego-browser nodejs <<'EOF' ... EOF` 執行操作。
4. 因為每一步都還是「回傳一句 shell 指令」，完全沿用現有的 `handleAiQuery`/
   `shouldAutoExecute`/`submitCommand` 流程，PTY 逐步執行、畫面上看得到即時輸出
   （含真的開啟的瀏覽器視窗），跟現有任何 shell 指令沒有差別。
5. `maxSteps` 沿用現況（目前固定 `5`，見上面「現況調查」第 1 點）——本次不變更這個
   上限，即使多步 exploration 可能偏緊；若日後發現 5 步不夠讓 model 完成
   「探索用法 + 實際操作」，屬於既有 `maxSteps` 限制的既有問題，非本次功能引入的
   新缺陷。

## 明確排除（Non-goals）

- **不做 Settings 開關**：純自動偵測，沒裝就完全不受影響，不需要額外 UI。
- **不動 AiPanel 對話式聊天**（`useMcpChat.ts` 的 MCP tool-calling 迴圈）。AiPanel 的
  工具清單目前完全來自 `McpManager::list_tool_infos()`（`ai.rs:447-454`），要讓
  ego-browser 在那邊變成可呼叫的工具，需要新增合成 `McpToolDefinition`、新的 Rust
  command 把 script 丟給 `ego-browser nodejs` 子行程執行、並在 `useMcpChat.ts` 攔截
  這個工具名稱——是完全不同、工程量大很多的機制，留給下一個獨立 spec。
- **不硬編完整用法文件**：只給一句指向 `--help` 的提示，完整內容永遠讓 model 自己現查
  ego-browser 當下版本的 SKILL.md，避免 AITerm 程式碼裡的副本跟真正的文件脫節。
- **不改 `ai_query` 的 IPC 簽名**：不需要新增任何欄位區分「這是 /agent 呼叫還是 /ai
  呼叫」，因為兩者本來就走同一個 prompt builder、行為完全一致（見現況調查第 1 點）。

## 對既有程式碼的影響

- `src-tauri/src/commands/ai.rs`：`build_single_command_prompt` 新增偵測後的條件式
  附加文字；新增（或從別處引入）`ego_browser_available()` 快取函式。
- 新增（若獨立成檔）`src-tauri/src/tools_detect.rs` 或等效模組，含 PATH 掃描邏輯，
  可仿照 `src-tauri/src/vcs/svn.rs:35-45` 的 `on_current_path` 直接複用/抽出共用。
- 前端不需要任何改動（`runAgentLoop`、`useAgentMission`、`useMcpChat` 全部維持現狀）。

## 測試策略

### 單元測試（Rust，`src-tauri`）

1. **PATH 偵測函式**：用暫時修改 `PATH` 環境變數（或注入可測試的目錄清單，視實作是否
   把「掃描哪些目錄」抽成參數而定）驗證「目錄裡有 `ego-browser`（或 Windows 的
   `ego-browser.exe`）」回傳 `true`，「目錄裡沒有」回傳 `false`。
2. **`build_single_command_prompt` 條件式附加**：偵測結果為 `true`/`false` 兩種情況下，
   斷言回傳字串是否包含（或不包含）ego-browser 那段提示文字。若偵測函式用了
   `OnceLock` 全域快取不易在單元測試裡假造多種結果，可將「是否偵測到」抽成
   `build_single_command_prompt` 的參數（或注入的 closure），讓純文字組裝邏輯本身可
   獨立測試，`OnceLock` 只留在呼叫端（`ai_query`）做一次性快取。

### 不需要新增前端測試

前端呼叫路徑（`runAgentLoop`、`handleAiQuery`）完全沒有變動，現有測試
（`agentLoop.test.ts` 等）預期不受影響、不需要修改。

### 需要真機驗證（無法只靠單元測試涵蓋）

- 實際跑一次 `/agent`，觀察 model 是否真的會在第一步下 `ego-browser --help`、後續步驟
  是否真的能組出可執行的 `ego-browser nodejs` heredoc 指令並成功操作瀏覽器（這取決於
  使用者設定的 provider/model 本身的能力，AITerm 這邊只負責把「工具存在」的提示放進
  prompt，無法保證每個 model 都會正確使用）。
