# AI 面板：模式表達設計規格

**日期**：2026-08-11
**狀態**：已實作（commit `5a0e75d`）
**相關 commit**：`5ef0b9d`（Agent 串流）、`fe05109`（放大鈕）

> 本規格為**回溯補寫**。實作先於文件完成，內容依實際落地的程式碼撰寫，非事前設計。

## 問題

AI 面板底部有兩顆開關：⚡（Agent 模式）與 🔧（MCP）。使用者問：「這兩顆是互斥的嗎？」

是——但畫面完全沒表達出來。

### 實際行為

`AiPanel/index.tsx` 的送出分支：

```
if (agentMode) {
  void submitAgent(text);          // ← useMcp 根本沒被讀
} else {
  void chat.send(text, mcpActive, …);
}
```

Agent 迴圈內的 AI 呼叫是寫死的：

```
await invokeAiChat(agentMessages, sessionId, undefined, false, locale);
                                                        ↑ use_mcp
```

所以 Agent 開啟時，那 N 個 MCP 工具完全不會被使用。

### 表達上的三個缺口

1. **互斥沒有被表達**：MCP 按鈕的 `disabled` 原本只看 `mcpToolCount === 0 || isDisabled`，沒把 `agentMode` 算進去。Agent 亮著時 MCP 仍是亮綠的「MCP (20)」，看起來像兩個都生效。
2. **⚡ 沒有任何文字**，只有一個圖示；「MCP」講的是協定名稱，不是它能做什麼。
3. **兩顆都不亮時是什麼模式，無從得知**——使用者無法判斷 AI 會不會自己執行指令。

### 三種模式的真正差別：誰按下執行鍵

| 模式 | 現場資訊 | 指令 | 迭代 |
|---|---|---|---|
| 都不亮（建議） | ✅ | 建議 `<cmd>`，**使用者點** | 單次 |
| ⚡ Agent | ✅ | AI **自己執行**，看輸出再決定下一步 | 至 `max_agent_steps` |
| 🔧 MCP | ✅ | 呼叫 MCP 工具（不經終端機） | 最多 10 輪工具呼叫 |

三種模式都拿得到終端機現場（`commands/ai.rs` 的 `context::snapshot`：cwd、shell、最近約 50 行輸出、目錄列表）。「建議」模式的 `<cmd>` 由 `build_chat_prompt` 的系統提示要求產生，前端渲染成可點的 `CmdTag`。

## 考慮過的方案

以視覺化 brainstorm（`.superpowers/brainstorm/`，四個方案並排、各含兩個狀態的實際按鈕）呈現後由使用者選定。

| 方案 | 內容 | 結果 |
|---|---|---|
| A | 三段式切換（建議／自動執行／工具），三選一 | 未採用 |
| B | 維持兩顆，補上文字標籤 | 未採用 |
| **C** | **兩顆＋一行模式說明** | **採用** |

**A 未採用的理由**：它是最貼合現況的形狀（形狀即語法，「兩個都亮」的疑問從根本消失），但把「三選一」寫死。若日後要讓 Agent 也能使用 MCP 工具，形狀得再拆回去。

**C 被選中的理由**：它是唯一會**明講「MCP 此時不使用」**的方案。按鈕變灰只說了「現在不能點」，沒說「它被忽略了」。代價是永久佔掉一行高度。

## 設計

### 1. 模式說明列（`ModeHint.tsx`，新元件）

純展示元件：`mode / maxAgentSteps / mcpToolCount` 進，一行字出。獨立於 `index.tsx` 之外，可單獨測試。

位置在 `MessageList` 與 `.aiterm-ai-panel-input-area` 之間，貼著輸入區。文案（`i18n.ts`，zh-TW 與 en 皆備，用專案既有的函式型字串慣例帶參數）：

| 模式 | 文案 |
|---|---|
| `suggest` | 💬 AI 只會建議指令，點 ▶ 才會執行 |
| `agent` | ⚡ AI 會自己執行指令並看輸出迭代，最多 {N} 步（此時不使用 MCP 工具） |
| `mcp` | 🔧 AI 可以呼叫 {N} 個 MCP 工具，不經過終端機 |

- 步數取自設定的 `max_agent_steps`；`0`（無限）在上層存成 `9999`，顯示 `∞`——與 Agent 狀態列同一規則
- 工具數取自 `mcpToolCount`
- 左側 2px 色條隨模式換色（灰／紫 `--accent`／綠 `--success`）

**與 Agent 狀態列的分工**：Agent 真的跑起來後（`agentRunning`）這條讓位給 `.aiterm-agent-status`，後者有步驟數與中止鈕。兩條同時堆在輸入框上只是噪音。

### 2. 模式推導的單一來源

```
const mcpActive = useMcp && mcpEnabled && mcpToolCount > 0;
const mode = agentMode ? "agent" : mcpActive ? "mcp" : "suggest";
```

`mcpActive` 同時供送出路徑與說明列使用。分成兩份寫遲早有一邊漏改，畫面就會說謊。

### 3. MCP 按鈕在 Agent 模式下停用

`disabled` 加上 `agentMode ||`；`title` 換成「Agent 模式下不使用 MCP 工具（AI 只透過終端機指令操作）」；同時移除 `--on` 樣式類別——只停用不夠，`:disabled` 僅是 `opacity: 0.4`，留著會變成暗綠而不是灰，看起來仍像正在生效。

與說明列分工：**灰**說「現在不能點」，**說明列**說「而且它被忽略了」。

### 4. 設定改為每次開啟面板時重讀

該 effect 的相依由 `[]` 改為 `[isOpen]`。面板常駐不卸載（`key={sessionId}`），只在掛載時讀一次的話，使用者在設定裡改了 `max_agent_steps`（或裝了新的 MCP server），要重開 app 才會反映。

## 一併修正的既有缺陷

### MCP 工具卡片被壓成一條細線

`.aiterm-message-list` 是 `column` flex + `overflow-y: auto`。flex item 的自動最小尺寸（`min-height: auto`）**只在 `overflow: visible` 時生效**；`.aiterm-tool-card` 為了切齊圓角設了 `overflow: hidden`，最小高度因此變成 0，內容一溢出容器就被 flex 壓扁，只剩上下邊框。訊息氣泡不受影響，因為它們 `overflow` 是 `visible`——所以整串對話裡只有工具卡片會被壓扁。

修法：`.aiterm-tool-card` 加 `flex-shrink: 0`。

這是既有問題，但說明列吃掉約 30px 的垂直空間，提高了它出現的機率。

## 測試

| 層級 | 檔案 | 涵蓋 |
|---|---|---|
| 元件 | `ModeHint.test.tsx` | 三種文案、步數帶入實際值、`9999 → ∞`、Agent 明講不使用 MCP |
| 整合 | `AiPanel.test.tsx` | 切換模式時說明列跟著換、Agent 執行中讓位給狀態列、每次開啟重讀設定、Agent 開啟時 MCP 按鈕停用且失去 `--on` |

測試的 `get_mcp_tools` mock 由寫死的 `[]` 改為可注入的清單——沒有工具時按鈕本來就是停用的，測不出東西。

**測不到的部分**：flex 壓扁問題無法寫成單元測試，jsdom 不做版面計算、量不出高度。該項只能實機驗證（已由使用者確認修復）。

## 未決

**是否讓 Agent 迴圈也能使用 MCP 工具**（即前述 B 案／大改）。若要做：

- Agent 迴圈需同時處理 `<cmd>` 與 `tool_calls` 兩種回覆型態，並把工具結果餵回下一輪
- 會走進 `generate_with_tools` 這條 Rust 路徑——OpenAI/compatible 是 `"stream": false`、Anthropic/Ollama 是整包 `resp.json()` 後送單一 chunk，**等於把剛修好的串流關掉**
- 屆時本規格採用的「兩顆開關」形狀仍適用；反倒是被否決的 A 案（三選一）會變成錯的形狀
