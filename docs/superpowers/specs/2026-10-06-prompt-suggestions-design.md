# Ask AI 提示詞建議（Prompt Suggestions）設計

日期：2026-10-06

## 目標

在 Ask AI 面板輸入框上方，依終端機（多半跑著 Claude Code）最近的對話，產生 3–5 個
「下一步提示詞」卡片。單擊卡片＝把文字**填入**終端機輸入位置，不按 Enter；雙擊＝再送出。

## 已定案的決策

| 項目 | 決定 |
|------|------|
| 位置 | `AiPanel` 的 `extraAboveInput` 槽，不新增側欄 |
| 建議來源 | AI 依終端機畫面內容即時生成 |
| 送出方式 | 單擊＝填入、不按 Enter（可編輯）；雙擊＝送出。雙擊的第一下已填入，第二下（`event.detail>=2`）只補一個 Enter，不重複貼上 |
| 產生時機 | 預設手動按鈕；面板內有「自動」開關，開了就在終端機閒置後自動產生 |

## 架構

```
AiPanel ── extraAboveInput ──> PromptSuggestions（新元件，自管狀態）
                                  │ 讀畫面：serializeTerminal(sessionId, scrollback)
                                  │ 產生：invokeAiChatCtx(…, connId="suggest-<sessionId>")
                                  │ 解析：parseSuggestions(reply)（純函式）
                                  └ 填入：fillTerminalInput(sessionId, text)
```

- **獨立請求**：用 `ai_chat_ctx`，串流事件以 `suggest-<sessionId>` 為 id，
  不會被 `useMcpChat(sessionId)` 收到，也不寫入 `chat.messages`。
- **畫面內容放在使用者訊息裡**，不依賴後端 `recent_output` 的 2000 字截斷。
- **填入**：`fillTerminalInput` 放在 `terminalInstanceRegistry`。終端機有開 bracketed
  paste（Claude Code 會開）就包成 `ESC[200~…ESC[201~`，沒開就把換行壓成空白，
  避免換行被當成 Enter 而直接執行。
- **停用條件**：Ask AI 串流中、Agent 執行中、終端機最近 `IDLE_MS` 內仍有輸出
  （Claude 還在回）、沒有可讀的畫面內容、沒有 sessionId。
- **自動模式**：`getIdleMs()` 由忙碌轉為閒置時觸發一次；同一份畫面內容不重複產生
 （以內容雜湊比對）。偏好存 `localStorage`（`aiterm-suggest-auto`）。
- **競態**：每次產生遞增 request id，過期的回應丟棄；元件卸載後不 setState；
  使用者切走或重新產生時呼叫 `ai_abort` 取消上一個。

## 提示詞要求（給 AI）

只回 JSON 陣列，3–5 筆，每筆 `{ "title": 短標題(≤12字), "prompt": 完整提示詞 }`；
語言跟隨介面語系；提示詞要能直接丟給 Claude Code 當下一個指令；不要建議破壞性操作。
`parseSuggestions` 容忍 code fence、前後說明文字；最多取 5 筆、去重、丟棄空字串，
`prompt` 超過 500 字截斷。

## 錯誤處理

AI 失敗、逾時、格式錯誤、未設定供應商：卡片區顯示單行錯誤與「重試」，
不影響 Ask AI 其他功能。無畫面內容：按鈕停用並顯示原因。

## 測試

- `parseSuggestions`：標準 JSON、code fence、前後雜訊、非法輸入、去重、截斷。
- `fillTerminalInput`：有／無 bracketed paste、換行處理、未註冊的 id。
- `PromptSuggestions` 元件：手動產生、點選填入且**不送 Enter**、停用條件、
  過期回應丟棄、錯誤顯示、自動模式只在閒置轉換時觸發一次。
- `AiPanel` 回歸：附件、貼上、拖放仍正常；建議不出現在 `chat.messages`。
- i18n：zh-TW 與 en 同步補齊。

## 未決／待實機驗證

填入 Claude Code 輸入框要避開它的 TUI 選單與模式：Claude 在權限選單、`/` 選單
等狀態下貼上文字可能被當成選項輸入。本版以「終端機閒置」為唯一保護；
實機驗證若發現選單狀態會誤吞文字，再加偵測（畫面含選單特徵時停用卡片）。
