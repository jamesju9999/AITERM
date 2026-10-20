# Claude 協作流程圖優化 Implementation Plan

> 接續 `2026-10-20-claude-subagent-activity-sidebar.md`。使用者要求：全做。

**Goal:** 讓流程圖在 subagent 很多、巢狀深、側欄窄時仍好用，並多提供一個看平行度的時間軸。

**新資料（後端）：** 事件酬載由「節點陣列」改成 `Snapshot { nodes, main_tool, turn }`。
`Node.turn` 是節點建立時的「提問輪次」（`UserPromptSubmit` hook → `Kind::Prompt` 使輪次 +1）；
`main_tool` 是主 Claude 現在在用的工具（沒有 `agent_id` 的 `PreToolUse`／`PostToolUse`）。

**前端各項：**
1. 折疊已完成：根的子樹「全部完成」者 ≥3 個時預設折成一張「已完成 N 個」卡（`fold.ts`，純函式），
   工具列可展開／收起。
2. 依輪次分組：預設看最新一輪；多輪時工具列出現 ‹ 第 N/M 輪 ›。
3. 標題兩行＋`title` 全文；`CARD_H` 74→92。
4. 縮放（− ＋）、「符合寬度」、新的執行中節點自動捲入視野。
5. 選中節點：路徑（祖先鏈）高亮、其餘淡化。
6. 根卡顯示 `main_tool`（`ROOT_W` 84→110，340px 側欄仍放得下根＋一層）。
7. 時間軸檢視（`timeline.ts`，純函式）：每個 subagent 一條橫條，依開始時間排序，看平行度。
8. 潤飾：執行中連線上有移動的小點（`animateMotion`，reduced-motion 時隱藏）；兄弟節點失敗者置前。

**順序：** Rust（Snapshot／turn／main_tool／prompt hook）→ 前端型別與 hook → 純函式（fold、timeline、layout 排序）→ 面板 → TerminalView → 驗收。每步 TDD、突變驗證。
