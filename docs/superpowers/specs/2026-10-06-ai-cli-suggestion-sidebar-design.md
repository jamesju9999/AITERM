# AI 命令列工具的「下一步建議」側欄 設計

日期：2026-10-06
前置：`2026-10-06-prompt-suggestions-design.md`（建議列，已有）。本文取代它的「位置」一節。

## 為什麼要改

1. 建議列放在 Ask AI 面板裡，兩個功能混在一起，使用者分不清「問 AI」和「給終端機裡的 AI 工具的提示詞」。
2. Ask AI 是疊在終端機上的浮動面板，開著時 `TerminalView` 的 `term.onData` 會直接丟掉所有輸入
   （`panelOpenRef.current` 時 return），所以建議填進終端機後**無法編輯**。
3. 建議只對「終端機裡正跑著 AI 命令列工具」有意義，沒在跑時顯示只會困惑。

## 決策

| 項目 | 決定 |
|------|------|
| 位置 | 獨立側欄，與終端機**並排**（左右兩欄），不疊在上面 |
| Ask AI | 不動，仍是疊在上面的浮動面板；建議列從它裡面**移除** |
| 何時可用 | 該分頁有「AI 命令列工具」正在執行時，開關按鈕**亮起**（不會自動開啟，見下）；在工具執行中打開的側欄，工具結束時自動收起 |
| 哪些算 AI 工具 | 內建清單＋使用者自訂清單；清單外可「手動開啟」 |
| 內建清單 | `claude` `codex` `gemini` `aider` `opencode` `qwen` `amp` `copilot` |
| 點卡片 | 單擊＝填入（不按 Enter，可編輯）；雙擊＝送出（沿用現有行為） |
| 不做 | 「點另一張前清掉上一張」——已在實機證明不可行並撤銷（見備忘），本版不碰 |

## 偵測：AI 工具是否在執行

純推導，不加新狀態：`blocks` 的最後一個 block `status === "running"` 且
`isAiCliCommand(block.command)`。

`isAiCliCommand(cmd, customNames)`（新檔 `src/lib/aiCliCommand.ts`，取代只認 claude 的
`isClaudeCommand` 的用途範圍；`isClaudeCommand` 保留，既有 Claude 通知流程不動）：

- 取第一個 token，切掉路徑（`/` 與 `\`），去掉 `.exe` `.cmd` `.bat` `.ps1` 副檔名，轉小寫，
  與清單比對。
- 刻意不支援環境變數前綴（`FOO=1 claude`）與 `npx xxx`、`bunx xxx`：理由同 `isClaudeCommand`，
  支援它們要把字串比對變成 shell 語法解析，漏報的代價只是這次不自動啟用——使用者可手動開啟。

**已知限制**（實作時要實測並寫進回報，不可假設）：
- 只認透過 AITerm 輸入框送出、並由 `useTerminalBlocks` 建成 block 的指令。在 alternate buffer／
  raw keyboard mode 下使用者直接在 xterm 打的指令不一定有 block。
- 「結束」靠 block 結案（OSC 133 或 `finalizeBlock` 強制結案）。沒有 shell integration 的環境
  可能不結案，側欄會多留著；此時手動關閉即可。
- 工作看板派工開的分頁是否有 block 要實測。

## 手動開啟與自動收起

側欄開關按鈕（狀態列「建議」）永遠存在。偵測到 AI 工具時按鈕亮起；沒偵測到時 tooltip 說明
「未偵測到 AI 工具，點一下仍可手動開啟」，點了就開，側欄標題下方的狀態文字會寫「未偵測到 AI 工具（手動開啟）」。

**實作時的修正（相對於最初草稿）：偵測到工具時不自動開啟側欄。** 側欄一開，終端機欄就變窄、
觸發 PTY resize，而「工具剛啟動」正是 TUI 最怕被縮放打斷的時刻；亮起按鈕已足夠提示。

- 使用者**在工具執行中**打開的側欄 → 工具結束（最後一個 block 結案）時自動收起。
- 使用者**在沒有工具時**打開的側欄 → 由使用者自己關，工具啟動／結束都不影響。

## 自訂清單

`localStorage` 鍵 `aiterm-ai-cli-names`（JSON 字串陣列），依本 repo 慣例（元件內 `useState`＋
`localStorage` 存 UI 偏好），**不新增 Rust 欄位、不改 `AppConfig`**。設定介面放在側欄標題列的
「⋯」小選單：顯示內建清單（唯讀）與自訂清單（可新增／刪除，輸入時即時 trim、小寫、去重、
拒絕含空白或路徑分隔符的值）。所有讀寫包 try/catch。

## 版面：並排

`TerminalView` 最外層目前是 `column` flex、`position: relative` 的單一 div，Ask AI 是它裡面的
`position: absolute` 浮層。改為：

```
<div class="aiterm-term-split" style="display:flex; height:100%">
  <div class="既有的最外層 div" style="flex:1; minWidth:0; position:relative">…（原內容，含 AiPanel）</div>
  {sidebarOpen && <div class="分隔線" /> <SuggestionSidebar style={{ width }} />}
</div>
```

- 分隔線可拖拉，**必須用 `setPointerCapture`**（同 `ArtifactSplit` 的理由）；寬度有下限／上限
  （終端機欄至少 360px、側欄 240–560px），寬度存 `localStorage`。
- Ask AI 的浮層仍相對於左欄（原最外層 div）定位，所以側欄開著時它蓋的是左欄右側，不會蓋住側欄。
- 窄視窗（總寬不夠兩個下限）：側欄不顯示並收起，避免終端機被擠到不可用。

### 縮放風險（最大風險）

側欄開關、拖拉分隔線都會改變終端機寬度 → `FitAddon` 的 `ResizeObserver` → 真正的 PTY resize。
本 repo 在 Windows ConPTY 上，縮放曾造成「舊內容浮現、輸入卡住」（見備忘
`project_conpty_clear_desync`、`project_windows_prompt_blank_investigation`）。對策：

- **不自己呼叫 fit／resize**，完全交給現有的 `ResizeObserver` 與 `resizeRepaintGate`
  （`flushPendingResize`），不新增第二條縮放路徑。
- 拖拉分隔線時**節流**：拖曳中只更新側欄寬度的 CSS，放開（`pointerup`）才讓左欄寬度生效一次，
  避免拖曳過程中連續觸發數十次 PTY resize。實作：拖曳中左欄用 `width` 固定當下值，放開後改回 `flex:1`。
- 我無法在 Windows 實機驗證；回報時必須明講這點，並列出需要使用者在 Windows 實機確認的項目
  （開／關側欄、拖拉、在 Claude 對話中開關後輸入是否正常、舊內容有無浮現）。

## 元件

- `SuggestionSidebar`（新，`src/components/SuggestionSidebar/`）：標題列（✦ 標題、手動／自動狀態、
  「⋯」自訂清單、關閉）＋ 現有 `PromptSuggestions` 的內容。
- `PromptSuggestions`：從 `AiPanel/` **搬到** `SuggestionSidebar/`（連同 css、測試），改為不依賴
  AiPanel 的 `disabled`（Ask AI 串流中）——側欄與 Ask AI 無關了；仍保留 `disabled` prop 給
  「Agent 執行中」之類由 TerminalView 傳入的條件。
- `AiPanel`：移除 `<PromptSuggestions>` 掛載與相關 import。
- `aiCliCommand.ts`（新）：內建清單、`isAiCliCommand`、自訂清單的 load/save/normalize 純函式。
- `buildSuggestionRequest`：提示文字由「Claude Code 的對話」改成泛稱「AI 命令列工具的對話」。
- i18n：標題、手動開啟的提示、自訂清單 UI 的字串，zh-TW 與 en 同步；移除用不到的舊字串。

## 資料流

```
useTerminalBlocks.blocks ─> latest running block ─> isAiCliCommand ─> aiCliRunning
                                                                          │
  使用者按側欄開關 ─────────────> manualOpen ─┐                            │ 偵測觸發開啟／結束自動收起
                                              ▼                            ▼
                                       sidebarOpen（TerminalView 的 state）
                                              │
              serializeTerminal(sessionId) ──> PromptSuggestions ──> invokeAiChatCtx（獨立串流 id）
              fillTerminalInput / submitTerminalInput <── 點卡片
```

## 錯誤處理

沿用建議列的規則（AI 失敗、格式錯誤、無畫面內容皆單行提示＋重試，不影響終端機）。
自訂清單輸入不合法：欄位下方單行說明，不寫入。localStorage 不可用：退回內建清單、不報錯。

## 測試

- `aiCliCommand`：各內建名稱、路徑／副檔名／大小寫、前綴與 `npx` 不認、自訂清單、normalize 邊界
  （空白、路徑符號、重複、非陣列的壞 JSON）。每個測試都要先證明會紅；測資要能區分對錯
  （例如 `claude-helper` 不可誤判成 `claude`）。
- 偵測推導：block 開始→可用、結案→自動收起；手動開啟不被結案收起；偵測開啟被使用者手動關閉後，
  同一個 block 不再自動重開。
- 側欄：點卡片填入／雙擊送出（沿用現有測試搬移）、收合、錯誤、自動模式。
- 版面：側欄開啟時終端機容器 `minWidth: 0`、有 `flex:1`；拖曳中不改左欄寬度，放開才改；
  窄視窗不顯示。
- 回歸：`AiPanel` 不再含建議列；既有 `TerminalView.*` 與縮放相關測試全過。
- 以突變驗證（拿掉 `setPointerCapture`、拿掉節流、拿掉副檔名處理）測試必須轉紅。

## 實作順序

1. `aiCliCommand.ts`＋測試（純函式，最先、風險最低）。
2. 搬 `PromptSuggestions` 到 `SuggestionSidebar/`，`AiPanel` 移除掛載，測試跟著搬，確認全綠。
3. `SuggestionSidebar` 外殼＋ `TerminalView` 並排版面＋拖拉分隔線。
4. 偵測推導＋手動開啟＋自動收起。
5. 自訂清單 UI。
6. i18n、CHANGELOG。
7. 本機啟動應用程式實機檢查（macOS）；Windows 項目明列給使用者。

## 待實機確認／未決

- 「結束」的偵測在各種啟動方式（輸入框送出、工作看板派工、raw keyboard mode）下是否可靠。
- 其他 AI 工具（codex、gemini 等）對 bracketed paste 與 `\r` 的反應，我只在 Claude Code 上驗證過。
- Windows 縮放行為（見上）。
