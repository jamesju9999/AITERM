# LoopStudio 強化 — 階段 A：可靠性與安全（設計文件）

日期：2026-07-03
狀態：已核可
範圍：LoopStudio 九項改進中的階段 A（共三階段）。
階段 B（sharedContext 壓縮、token 追蹤）與階段 C（trace UI、streaming、i18n）將各自另立設計文件。

## 背景

LoopStudio（openspec change `loop-studio`）已實作 Orchestrator → Sub-agents → Verifier 的
multi-agent loop。現況有四個影響可靠性與安全的問題：

1. Sub-agent 對話歷史用假文字格式記錄 tool calls，降低多輪 tool calling 可靠性。
2. `execute_command` 借用使用者可見的 PTY 並輪詢 sentinel，跨平台脆弱且干擾終端。
3. Sub-agent 可無確認執行任意指令、寫任意路徑檔案；`projectDir` 限制只存在於 prompt。
4. Verifier 只能閱讀 Orchestrator 的自我報告，無法獨立驗證實際檔案狀態。

## A1. Sub-agent 對話歷史格式修正

**檔案**：`src/hooks/useSubAgentLoop.ts`

改用與 `useOrchestratorLoop.ts` 相同的正規 OpenAI tool-calling 格式：

- 每輪 `agentChat` 回覆後，推入一則 assistant 訊息帶 `tool_calls`：
  優先使用 `reply.raw_tool_calls`（保留 provider 原始格式，含 Gemini `thought_signature`），
  缺少時以 `{ id, type: "function", function: { name, arguments } }` 重建。
- 每個 tool call 各推一則 `{ role: "tool", content: <結果>, tool_call_id }`。
- 移除現有的 `<tool_call>...</tool_call>` 假 assistant 文字訊息與 `role: "user"` 假結果訊息。
- 「工具未啟用」錯誤同樣以 `tool` role 回傳。

**迴圈重構**：先執行完該回覆的全部 tool calls 並收集結果，再一次推入
（一則 assistant + N 則 tool），取代目前逐 call 推入造成的多則不完整 assistant 訊息。

## A2. 後端 `agent_exec` command

**新增**：Tauri command `agent_exec`（`src-tauri/src/commands/` 下，隨現有結構放置）

```
agent_exec(command: String, cwd: Option<String>, timeout_ms: Option<u64>)
  -> { stdout: String, stderr: String, exit_code: Option<i32>, timed_out: bool }
```

- Rust 以 `tokio::process::Command` spawn：Unix `sh -c <command>`、Windows `cmd /C <command>`。
- 預設 timeout 60 秒；逾時強制 kill，`timed_out: true`，並回傳已收到的輸出。
- 後端將 stdout/stderr 各截斷在 10,000 字元。
- 在 `lib.rs` invoke_handler 註冊。

**前端**：

- 新增 `src/ipc/exec.ts`：`agentExec(command, cwd?, timeoutMs?)` wrapper。
- `useSubAgentLoop` 的 `execute_command` handler 改呼叫 `agentExec`，
  `cwd = projectDir ?? getSessionCwd(sessionId)`。
- 刪除 `executeCommandInPty` 與 sentinel 輪詢機制；指令不再進入使用者終端。

**取捨**：直接 spawn 不載入使用者互動 shell 環境（alias、nvm 等）。
接受此限制以換取跨平台乾淨與不干擾終端；未來若需要可另議 login-shell 選項。

## A3. 安全閘門

### 路徑限制（程式端強制）

- `write_file`：目標路徑 normalize（解析 `..`、符號等）後必須位於 `projectDir` 內，
  違反時回傳錯誤 tool result（不執行寫入）。
- `read_file` / `list_directory`：不限制。唯讀操作風險低，且讀取專案外參考檔屬合理需求。
- 未設定 `projectDir` 時，以 session CWD 作為限制根目錄。

### 指令分級與暫停確認

- 新增 `src/lib/commandRisk.ts`：規則式分類器，輸出 `"dangerous" | "normal"`。
  危險模式（跨平台）至少涵蓋：`rm -rf`、`sudo`、`curl|wget … | sh`、`git push --force`、
  `dd`、`mkfs`、`chmod -R 777`、`shutdown`/`reboot`、Windows `del /s`、`format`、
  `Remove-Item -Recurse`、`rd /s`。
- `execute_command` 執行前先分類：
  - `normal` → 直接執行。
  - `dangerous` → loop 暫停：`runSubAgent` 新增 `onConfirmNeeded(command) => Promise<boolean>`
    參數，由 `useOrchestratorLoop` 提供實作並暴露
    `pendingConfirmation: { agentName, command, resolve }` state；
    ExecutionTrace 渲染指令內容與「允許 / 拒絕」按鈕。
    拒絕時回傳 tool result：「使用者拒絕執行此指令，請改用其他方式」，loop 繼續。
- Loop 控制區新增 **full-auto 開關**（預設關閉，隨 loop config 儲存）；
  開啟時跳過確認直接執行。
- 確認等待不計入 timeout；abort（Stop 按鈕）需能中斷等待中的確認。

## A4. Verifier 唯讀工具

- 從 `runSubAgent` 抽出共用 tool 執行 helper（tool 定義查表 + 執行 + action 回報）。
- Verifier 呼叫由單次 `agentChat` 改為小型 tool loop（上限 8 輪），
  工具為 `read_file` + `list_directory`。
- Verifier 的工具動作以現有 `sub_agent_action` trace kind 即時顯示
  （`agentName` 為 Verifier 名稱），不新增 entry 類型。
- Verifier system prompt 增補：「可先使用唯讀工具檢查實際檔案狀態再下結論；
  最終回覆仍必須只有 JSON 物件」。JSON 解析邏輯（`parseVerifierResult`）不變，
  取 tool loop 結束後的最終文字回覆解析。

## 錯誤處理

- `agent_exec` 逾時：tool result 註明 `[timeout]` 與已收到輸出，agent 可自行調整。
- 路徑違規、指令被拒：以錯誤 tool result 回饋 agent，loop 不中斷。
- Preflight、重複偵測、abort 機制維持現狀。

## 測試

- `src/lib/commandRisk.test.ts`：危險/正常指令分類規則（含 Windows 模式）。
- `src-tauri/tests/agent_exec_command.rs`：正常執行、逾時 kill、不存在的指令。
- A1 歷史格式與 A4 helper 抽取以 `npx tsc --noEmit` + 手動 loop 驗證
  （補齊 hook 單元測試屬於已排除的範圍 #8，不在本階段）。

## 不做的事（本階段）

- sharedContext 壓縮、token 追蹤（階段 B）。
- Trace UI 展開/折疊、streaming、i18n（階段 C）。
- read_file / list_directory 的路徑限制（明確決定不做）。
- 補齊 openspec tasks.md 既有未完成的測試項目。
