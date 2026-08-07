# AI 輸出語言跟隨系統 Locale — 設計規格

**日期：** 2026-07-15
**狀態：** 已核准，待實作

---

## 目標

目前 AI 的回覆語言與 UI 的 locale 設定（`LocaleContext`，`en` / `zh-TW`）完全無關——後端多處 system prompt 直接寫死「請以繁體中文回覆」，即使使用者把介面切成 English，AI 依然固定回中文。

改為：**AI 的輸出語言跟隨目前的 UI locale**。使用者把介面切成 English 時，所有 AI 對話介面（單指令、多輪聊天、Design 助手、跨庫/單庫 SQL 助手、LoopStudio Orchestrator/Sub-agent）都應該用英文回覆；切回 zh-TW 則恢復繁體中文。

---

## 範疇

**包含：**
- `ai_query` / `build_single_command_prompt`（單指令產生）
- `ai_chat` / `build_chat_prompt`（AiPanel 多輪聊天，同時服務 CrossDbAiChat / DatabaseAiChat）
- `design_chat` / `build_design_prompt`（DesignView SDD 助手）
- LoopStudio Orchestrator（`useOrchestratorLoop.ts`）中**真正送給 AI** 的 system/user/tool-result 訊息
- LoopStudio Sub-agent（`useSubAgentLoop.ts`）的 system prompt

**不包含：**
- `vcs_query` / `vcs_agent_step`（本來就沒有語言指令，語言中立，不用改）
- LoopStudio 的 trace/log 顯示字串（如「✓ 目標達成」「🔍 正在測試...」）——這些只顯示在畫面上、不會送給 AI，且已有另一條 i18n UI 字串專案在處理
- 新增獨立的「AI 回覆語言」設定——UI locale 就是唯一依據，不做覆寫機制
- 後端全域 locale 狀態——每次請求都由前端顯式帶入，不做前後端狀態同步

---

## 核心機制

所有 prompt builder 統一原則：**指令本文一律用英文撰寫（AI 讀得懂指令語言，不影響它輸出什麼語言），只有「請用 X 語言回覆／輸出」這一條規則依 locale 動態產生**。避免維護中/英兩份平行模板。

### Rust 側

新增 `Locale` enum（`src-tauri/src/ai/mod.rs`）：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Locale {
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-TW")]
    ZhTw,
}

pub fn language_name(locale: Locale) -> &'static str {
    match locale {
        Locale::En => "English",
        Locale::ZhTw => "Traditional Chinese (繁體中文)",
    }
}
```

### TypeScript 側

新增 helper（`src/lib/i18n.ts`）：

```ts
export function languageDirective(locale: Locale): string {
  return locale === "zh-TW" ? "Traditional Chinese (繁體中文)" : "English";
}
```

LoopStudio 的 prompt builder（純 TS，不經過 Rust）直接用這個 helper 產生「回覆語言」規則。

### 傳遞方式

每次 invoke 顯式帶 `locale` 參數，呼叫端一律用 `useLocale().locale` 取值，不做後端全域狀態：

- `ai_query(query, sessionId, locale)`
- `ai_chat(messages, sessionId, providerId?, useMcp?, locale)`
- `design_chat(sessionId, messages, providerId?, locale)`
- `LoopConfig` 新增 `locale: Locale` 欄位，由 `LoopStudio/index.tsx` 啟動 Loop 時填入，往下傳給 `buildOrchestratorSystemPrompt`（已有 `config` 可直接讀）、`buildVerifierSystemPrompt(..., locale)`、`runSubAgent(..., { locale })`

---

## 各面向改動細節

### 1. `ai_query` / `build_single_command_prompt`（`src-tauri/src/commands/ai.rs`）

JSON schema 裡 `"explanation"` 欄位的語言提示，從寫死 `"(use Traditional Chinese)"` 改成 `format!("(use {})", language_name(locale))`。

### 2. `ai_chat` / `build_chat_prompt`（`src-tauri/src/commands/ai.rs`）

Rule 1 從 `"Respond in Traditional Chinese (繁體中文)."` 改成 `format!("Respond in {}.", language_name(locale))`。

### 3. `design_chat` / `build_design_prompt`（`src-tauri/src/commands/design.rs`）

改動面最大：整個模板本文（各狀態的角色說明 `role_instruction`、頂層規則列表、`status_label`）目前都是繁中寫死，全部改寫成英文中立文字（意思不變，只是換成英文撰寫），例如：
- `"你現在的角色是「產品經理 (Product Manager)」..."` → `"Your current role is Product Manager..."`
- Rule 1 `"請以「繁體中文 (zh-TW)」進行對話。"` → `format!("Respond in {}.", language_name(locale))`
- 其餘規則（標籤塊格式、Mermaid 語法注意事項等）逐條翻成英文，語意保持一致

### 4. `CrossDbAiChat.tsx` / `DatabaseAiChat.tsx`

這兩個元件自己組的 system prompt（`t.cdb_ai_system_prompt` / `t.db_ai_system_prompt`）已經是透過 `t`（依 locale 切換的 i18n 物件）產生，本身就是 locale-aware，**不需要改內容**。

問題在於：這兩個元件呼叫的 `aiChat()` 背後，Rust `ai_chat` command 目前**無條件**額外組一份 `build_chat_prompt()` 當作 `GenerateRequest.system_prompt`，與前端自組的 system message 是兩份分開送給模型的指令——目前這兩份對「該用什麼語言回覆」有可能互相矛盾（例如 UI locale 是 en，前端訊息說英文，後端這份還說繁中）。本次改動讓 `build_chat_prompt` 也吃 `locale`，兩份訊息會得出一致的語言指示，衝突消除。（兩份系統提示並存本身是既有架構，非本次範疇要處理的問題，不動。）

### 5. LoopStudio Orchestrator（`src/hooks/useOrchestratorLoop.ts`）

以下函式/訊息目前混雜繁中撰寫的指令本文，全部改成英文中立文字 + 動態語言規則：

- `buildOrchestratorSystemPrompt`：`## 先前迭代的累積 Context` 等標題改英文；Rule 3/4（「用繁體中文回覆最終答案」）合併成一條依 `config.locale` 動態產生的規則
- `buildVerifierSystemPrompt`：新增 `locale` 參數；JSON schema 範例文字（`"一句話總結目前整體進度"` 等提示文字）改英文；`"Write all values in Traditional Chinese (繁體中文)"` 改動態
- `buildSharedContextUpdate`：標題（`### 迭代 #${iter} 結果`、`**已完成：**` 等）改英文——這段文字會被塞回下一輪的 system prompt，屬於 AI-bound 內容
- 初始 kickoff 訊息（`請開始執行目標：${config.goal}`）、preflight 測試訊息（system + user）、工具呼叫失敗時的糾正提示（`你剛才描述了計畫但沒有呼叫 call_agent 工具...`）、找不到 sub-agent 時回給 AI 的錯誤訊息、verifier 回應無法解析時塞回歷史的提示——這些都是**真正送進 `agentChat` 對話歷史**的內容，全部改英文
- verifier 回饋组回 sharedContext 的區塊（`## Verifier 反饋`、`**總結：**` 等）——同樣是 AI-bound，改英文

**額外需要處理的相依風險**：第 385 行附近有一段用中文關鍵字（`/應該|需要|建議|請|call_agent|delegate|assign/i`）偵測「Orchestrator 是不是只輸出文字描述計畫、沒有真的呼叫工具」，用來觸發糾正重試。這是針對 AI 回應文字做的啟發式判斷，如果 AI 改成用英文回覆，純中文關鍵字會偵測不到，導致 en locale 下這個重試修正機制失效。本次需要把這個正則同時加入等效的英文關鍵字（如 `should|need|will|let me|plan to`），維持兩種語言下偵測都有效。

以下維持不動（純 UI trace，不送給 AI）：`addTraceBuffered` 呼叫裡的 `text` 欄位內容（如「🔍 正在測試...」「✓ 工具呼叫前置測試通過」「⚠ 偵測到重複迴圈...」等）。

### 6. LoopStudio Sub-agent（`src/hooks/useSubAgentLoop.ts`）

`runSubAgent` 內組的 `systemPrompt`：
- `## 先前迭代的累積 Context` 標題改英文
- `## 指示 / 使用可用的工具完成指派的任務。完成後，用繁體中文回報：...` 整段改英文中立指令，最後一句「回報所用語言」改成依 `options.locale` 動態產生
- 新增 `locale` 到 `RunSubAgentOptions`，由呼叫端（Orchestrator 的 `call_agent` 處理邏輯）從 `config.locale` 傳入

---

## 檔案清單

| 檔案 | 變更類型 |
|---|---|
| `src-tauri/src/ai/mod.rs` | 新增：`Locale` enum、`language_name()` helper |
| `src-tauri/src/commands/ai.rs` | 修改：`ai_query`/`ai_chat` 新增 `locale` 參數；`build_single_command_prompt`/`build_chat_prompt` 依 locale 動態產生語言規則 |
| `src-tauri/src/commands/design.rs` | 修改：`design_chat` 新增 `locale` 參數；`build_design_prompt` 模板改寫為英文中立文字 + 動態語言規則 |
| `src/lib/i18n.ts` | 新增：`languageDirective()` helper |
| `src/ipc/ai.ts` | 修改：`invokeAiQuery`、`aiChat`/`invokeAiChat` 簽名新增 `locale` 參數 |
| `src/ipc/design.ts` | 修改：`designChat` 簽名新增 `locale` 參數 |
| `src/hooks/useAiChat.ts`（及其他呼叫 `aiChat`/`invokeAiQuery`/`designChat` 的元件，如 `AiPanel`、`DesignView.tsx`、`CrossDbAiChat.tsx`、`DatabaseAiChat.tsx`） | 修改：呼叫時帶入 `useLocale().locale` |
| `src/hooks/useOrchestratorLoop.ts` | 修改：`LoopConfig` 新增 `locale` 欄位；prompt builder 與 AI-bound 訊息改英文中立文字 + 動態語言規則；補上英文版的「只講計畫沒呼叫工具」偵測關鍵字 |
| `src/hooks/useSubAgentLoop.ts` | 修改：`RunSubAgentOptions` 新增 `locale`；`systemPrompt` 改英文中立文字 + 動態語言規則 |
| `src/components/LoopStudio/index.tsx`（或啟動 Loop 的呼叫點） | 修改：組 `LoopConfig` 時填入 `useLocale().locale` |
| `src-tauri/tests/*`（涉及上述 command 的既有測試） | 修改：呼叫參數補上 `locale` |
| `src/**/*.test.ts(x)`（涉及上述 invoke 的既有測試） | 修改：呼叫參數補上 `locale`，或針對兩種 locale 各補一組斷言 |

---

## 成功標準

1. Settings 語言切成 English 後，AiPanel 單指令（`/ai`）與多輪聊天的 AI 回覆變成英文
2. English locale 下，DesignView 的 SDD 助手對話變成英文
3. English locale 下，CrossDbAiChat / DatabaseAiChat 的回覆變成英文，且不再出現中英夾雜或語言指示衝突的情況
4. English locale 下，LoopStudio 跑一次 Loop，Orchestrator 的最終總結、Sub-agent 回報內容都是英文；「只講計畫沒呼叫工具」的糾正重試邏輯在英文回覆下依然正常觸發
5. 切回 zh-TW 後，以上五個面向全部恢復原本的繁體中文行為（回歸測試）
6. `cargo test` 與 `npm run test` 全數通過；`npx tsc --noEmit` 無型別錯誤
