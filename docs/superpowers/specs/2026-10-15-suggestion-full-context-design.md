# 下一步建議：拿到整輪完整內容，而不只是看得到的那一屏

## 問題
側欄產生建議時只讀 `serialize({ scrollback: 0 })`——終端機目前看得到的那一屏。AI 一輪回覆比一屏長時，
使用者往上捲才看得到的內容，建議完全不知道；捲動後再產生，建議就變了。

兩種情形成因不同：
- **非全螢幕工具**（Gemini、aider、Copilot CLI，部分設定下的 Codex）：完整輸出在 xterm 的捲動緩衝區裡，只是沒讀。
- **全螢幕工具**（Claude Code `"tui": "fullscreen"`，alternate buffer）：xterm 只有可見列，上面的內容只在工具自己的記憶體裡。
  唯一完整來源是工具寫在磁碟上的對話記錄。

## 設計

### 第一層：讀捲動緩衝區（所有工具）
`serializeTerminalForAi(id)`：`serialize({ scrollback: SUGGEST_SCROLLBACK_ROWS })`（300 列）。
inline TUI 重畫會把中間畫面（spinner、重複提示列）推進捲動緩衝區，所以先 `squeezeRepeatedLines`：
同一行只留最後一次、連續空行壓成一行。
alternate buffer 本來就沒有捲動緩衝區，結果與現在相同。`buildSuggestionRequest` 既有的「只留尾端 8000 字」繼續把關長度。
只有建議請求改用；其他呼叫 `serializeTerminal` 的地方不動。

### 第二層：讀工具自己的對話記錄（Claude Code、Codex）
Rust 模組 `agent_transcript`，指令 `agent_transcript_context(screen) -> Option<TranscriptContext { tool, text }>`：

1. **候選檔**：`$CLAUDE_CONFIG_DIR` 或 `~/.claude/projects/*/*.jsonl`；`$CODEX_HOME` 或 `~/.codex/sessions/**/*.jsonl`。
   只取 15 分鐘內修改過的。
2. **只讀尾端**，先 256 KB；湊不到兩則使用者提問就依序讀 1 MB／4 MB／16 MB（實測：一輪 agent 工作之後，
   上一則提問在 520 KB 之前）。丟掉被切斷的第一行。
3. **轉成統一的條目** `Entry { role: User | Assistant | Tool, text }`：
   - Claude：`type=user` 的純文字（略過 `isMeta`、`isSidechain`、tool_result、`<command-…>`／`<local-command…>`／`<system-reminder>` 開頭的文字）；
     `type=assistant` 的 text 與 tool_use（工具名＋參數，每個參數截到 200 字）。thinking 略過。
   - Codex：`event_msg` 的 `user_message`／`agent_message`；`response_item` 的 `function_call`／`custom_tool_call`（名稱＋參數截斷）。
   - **不收任何工具回傳結果**（讀檔內容、指令輸出），以免把整份檔案送給 AI 供應商。
4. **認出是哪個 session**：取最後兩輪的 user／assistant 文字，按行去掉空白與 markdown 符號（`*_#>|-` 與反引號），
   正規化後 ≥ 8 字的行若出現在同樣正規化過的畫面裡就算一分。分數最高者勝，同分取較新的；**0 分就回 None，不猜**。
   不比對路徑／cwd——同資料夾多個 session 也不會認錯，也沒有 Windows 路徑編碼問題。
5. **渲染**：從倒數第二則使用者提問起到最後，`使用者：…`／`AI：…`／`〔工具〕…`（工具整行 ≤ 300 字）。
   超過 8000 字時，最新那則提問一定放最前（≤ 1500 字），接「…（中間省略）…」，剩下額度給最後面——
   只留尾端的話，長的一輪會把提問本身切掉。

### 前端
`PromptSuggestions.generate`：`aiCliRunning` 時先呼叫 `agent_transcript_context(遮罩前的畫面)`；有結果時：
- 請求加一節「AI 工具的對話記錄（最近兩輪，完整）」，對話記錄先遮罩再放進去；
- 不再附「較早的畫面」（對話記錄已涵蓋）；目前畫面只留尾端 3000 字。
- 狀態列下方顯示「已讀取 Claude Code／Codex 對話記錄」。
任何失敗（指令錯誤、None）都安靜退回只讀畫面。

## 範圍外
里程碑「檢查進度」與提示詞助手不改。Copilot CLI／opencode（SQLite、無公開格式）不做轉接器，靠第一層。

## 測試
- Rust 純函式：兩種格式的解析（含要略過的記錄）、匹配（命中、0 分不猜、同分取新、正規化）、渲染截斷、尾端讀取丟掉殘行、
  mtime 過濾（tempfile）。
- 前端：有對話記錄時請求含該節、不含較早畫面、會被遮罩、顯示來源；沒有時與現在相同；指令丟錯時退回。
  建議請求帶 scrollback。
- 實機：Claude Code fullscreen 下長回覆，確認建議提到只在上方的內容。

## 驗證紀錄
自動化：Rust 15 條＋前端新增測試；Rust 7＋3 個、前端 4 個突變全被抓。真實資料探測（本機這個 session 的 1.6 MB 記錄）：
找到 5 則提問、渲染以最新提問開頭、以最後一則 AI 回覆的末 6 行當畫面能比對回 Claude。實機 UI 未驗。

## 已知限制
格式都沒有官方文件；工具改版、解析不了時退回第一層。
