# Claude Code CLI 提供者（純文字）

日期：2026-11-19
狀態：設計，尚未實作

## 背景

Anthropic 訂閱的 OAuth 憑證，只要請求帶 `tools` 就會被記到額外用量而不是訂閱額度。
現有解法是偵測到 400 後改走「工具描述注入系統提示」（不帶 `tools`），見 `ai/anthropic.rs`。
這條退路合規、已上線，**維持不動**。

另一條合規的路是直接呼叫使用者本機真的 `claude` CLI：那是 Anthropic 自己的客戶端，用訂閱本來就沒有限制。
AITerm 的派工、Agent 協作、系統連線任務已經在 spawn `claude`（`systems/jobs.rs` 的 `claude_args`、
`tasks/tools_check.rs` 找執行檔），所以環境與慣例都現成。

本期**只做文字生成**，當作 Ask AI 可選的另一個「訂閱提供者」，與 Anthropic OAuth 並存、互不影響。

## 不做

- **不偽裝**：不模仿 Claude Code 的請求簽章、`metadata.user_id`、Stainless 標頭。參考過 OmniRoute 的做法，
  其核心是偽造 Anthropic 用來辨識真 Claude Code 的內容簽章，不屬於 AITerm 該做的事。
- 不改現有 Anthropic 提供者、不新增切換邏輯；使用者在提供者清單自己選。
- 不讓 CLI 自己跑工具迴圈（Bash/Read/Edit 等）。本期 `--tools ""`，工具一律走 AITerm 現有路徑。
- 不把 AITerm 的工具包成 MCP 給 CLI 用（第二期，視第一期實機結果再評估）。
- 不做 API key 模式；CLI 自己管登入。

## 設計

### 提供者

`ProviderType::ClaudeCli`，新檔 `src-tauri/src/ai/claude_cli.rs`，實作 `AiProvider`：

| 方法 | 行為 |
|---|---|
| `generate` | spawn `claude`，提示走標準輸入，解析 stream-json，轉成 `GenerateChunk` |
| `health_check` | `claude --version` 能執行，並以一次極小的生成確認已登入 |
| `generate_with_tools` | 不覆寫，沿用預設 `Unsupported`，呼叫端會自動走既有的提示注入路徑 |
| `generate_json` | 不覆寫（預設忽略 schema） |

`router.rs::resolve_by_id` 新增一個分支，不需要金鑰、不碰 `SecretStore`。設定項：`model`（別名 `haiku`／`sonnet`／`opus` 或完整名稱）。

### 呼叫命令

```
claude -p
  --model <model>
  --tools ""
  --output-format stream-json --verbose --include-partial-messages
  --system-prompt <system_prompt>
  --setting-sources ""
  --strict-mcp-config
  --disable-slash-commands
```

提示（把 `messages` 攤平成文字）走 **stdin**，避免多行中文在各平台命令列引號出問題（與 `claude_args` 同一原則）。

**瘦身參數是必要的，已實測（claude 2.1.328，Mac）：**

| 設定 | 首次 `cache_creation_input_tokens` |
|---|---|
| 只有 `--tools ""` | 12,497（使用者的 hooks／CLAUDE.md／skills 全被帶進來） |
| 加 `--setting-sources "" --strict-mcp-config --disable-slash-commands` | 636 |
| `--bare` | **不可用**：回「Not logged in」，因為 `--bare` 不吃訂閱 OAuth |

不用瘦身參數的話，每次 Ask AI 都會背著使用者的 SessionStart hook 輸出，既貴又會汙染答案。

### 串流解析

stdout 每行一個 JSON（實測事件）：

- `stream_event` → `event.type == "content_block_delta"` 且 `delta.type == "text_delta"` → 推 `GenerateChunk { delta: delta.text, done: false }`。
- `thinking_delta`、`signature_delta`、`system.*`（hook／init／status／thinking_tokens）、`rate_limit_event`、`assistant`（完整訊息，與 delta 重複）一律忽略。
- `result`：`subtype == "success"` 且 `is_error == false` → 推 `done: true`，`usage` 取 `input_tokens`／`output_tokens`／`cache_read_input_tokens`／`cache_creation_input_tokens` 對應 `TokenUsage`。
- `result.is_error == true` → 依 `result` 文字對應 `AiError`（見下）。

未知事件類型一律略過，不報錯（CLI 版本升級會新增事件）。

### 錯誤對應

| 情況 | `AiError` |
|---|---|
| 找不到 `claude` | `NotConfigured`（訊息指向「請安裝 Claude Code」，沿用 `tools_check` 的 `~/.local/bin` 偵測） |
| `result` 含 `Not logged in`／`/login` | `AuthFailed` |
| 含 rate limit／usage limit 字樣 | `RateLimit`（`retry_after` 無從得知，留空） |
| 行程非零結束、無 `result` | `ModelError { reason: stderr 摘要, raw }` |
| spawn 失敗 | `Network { message }` |

### 取消與逾時

- 呼叫端丟棄 future／關閉 `tx` 時要終止子行程（`kill_on_drop(true)` + 明確 `kill`），不留孤兒 `claude`。
  這在本專案踩過：見 worktree 清理與派工分頁行程殘留的紀錄。
- 逾時 180 秒（可調），超時殺行程回 `Network`。
- Windows：spawn 要 `CREATE_NO_WINDOW`，避免閃出黑色視窗。

### 執行檔位置

GUI App 的 PATH 不含 Homebrew／npm／`~/.local/bin`。找法沿用 `tasks/tools_check.rs`
（用派工分頁同一個環境去找，找不到再看預設位置），不另寫一套。

### 多輪對話

第一期**每次重送整段歷史**（攤平成 `User:`／`Assistant:` 文字），不用 `--resume`。
理由：`--resume` 會把這次對話留在使用者的 Claude Code 工作階段清單裡，汙染他們自己的 `/resume`；
也讓 AITerm 的歷史與 CLI 狀態分家。代價是長對話重複計 token，由 CLI 自己的 prompt cache 吸收。
加 `--no-session-persistence` 避免寫入磁碟工作階段（若該旗標存在；實作時先以 `claude --help` 確認）。

### 前端

- `ProviderForm.tsx`：新增「Claude Code（本機 CLI）」選項；只有模型欄位，沒有金鑰／Base URL／OAuth。
- `ipc/provider.ts`、`i18n.ts`（en／zh-TW 兩套）補型別與字串。
- 提供者卡片註明：「需先安裝並登入 Claude Code；每次呼叫會啟動一個本機行程。」
- Ask AI 的工具迴圈在這個提供者上**自動走提示注入**（`generate_with_tools` 回 `Unsupported`），UI 不需要額外處理。

### 計費與額度

使用 CLI 自己登入的訂閱，與使用者在終端機手動用 `claude` 同一個桶。`MeteredProvider` 照樣記帳（`result.usage`）。
`result.total_cost_usd` 是 CLI 估算值，**不寫進帳**（訂閱使用者不是按 USD 付費，會誤導）。

## 風險

1. **延遲**：每次起行程加載 CLI，首字元會比直連慢。實測要量；若不可接受，本提供者只定位成「訂閱額度的備用路徑」。
2. **CLI 版本飄移**：stream-json 事件格式不是穩定 API。解析要寬鬆（未知略過），並在 `health_check` 印出 CLI 版本。
3. **使用者設定洩漏**：瘦身參數若在某版本改名，會退回背整包 hooks。以測試鎖住（見下），並在實機驗收時看 `cache_creation_input_tokens`。
4. **孤兒行程**：見取消與逾時。

## 測試

- **單元（無需 claude）**：用錄下來的真實 stream-json 當 fixture（含 thinking、hook、rate_limit 雜訊），驗證 `text_delta` 串接、`result` 結案、`is_error` 對應、未知事件略過。fixture 必須是真輸出，不可手抄。
- **命令列組裝**：斷言引數含 `--tools ""`、`--setting-sources ""`、`--strict-mcp-config`、`--disable-slash-commands`，且**提示不在 argv 而在 stdin**。
- **假 `claude`**（沿用既有「假 claude」演練做法）：腳本輸出 fixture，驗證端到端串流、取消後行程確實結束、逾時被殺。
- **真 `claude` 的實機測試**（`#[ignore]`，手動跑）：回「你好」、`cache_creation_input_tokens` 低於 2,000。
- 前端：`ProviderForm` 選到此類型時只顯示模型欄位；i18n 兩語系鍵值一致。

## 驗收（實機，Mac 先、Windows 後）

1. 設定 → 新增提供者 → Claude Code → 測試連線成功。
2. Ask AI 一句話，文字逐字串流出來。
3. 中途取消，`ps` 看不到殘留 `claude`。
4. 未登入狀態下回報「請先登入 Claude Code」，不是不明錯誤。
5. 同時保留一個 Anthropic OAuth 提供者，兩者可來回切換，後者行為不變。
6. Windows：`claude.exe` 找得到、不閃黑視窗、中文提示經 stdin 不亂碼。

## 第二期（不在本期範圍）

把 AITerm 現有的內建 MCP 端點（`canvas_tool`、系統連線）交給 CLI 用原生工具呼叫，迴圈主導權改由 CLI 持有。
要先決定授權確認、風險分級（`execution_mode`）如何對接 CLI 自己的權限機制，所以等第一期實機驗證後再另開 spec。
