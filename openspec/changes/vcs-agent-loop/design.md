## Context

現有 VCS Panel 的 `vcs_query` command 是單發模式：用戶輸入 → AI 解析成一個 `VcsIntent` → 執行 → 回傳 `VcsResult`。這無法處理需要多步推理的查詢（例如先搜 log、再看 diff、再確認結果）。

現有可複用的基礎：
- `VcsIntent` / `VcsResult` / `GitClient` / `SvnClient` 執行層完整，不需改動
- `useAgentMission` hook 展示了 terminal agent loop 的狀態管理模式（goal, history, stepCount, maxSteps）
- `DatabaseAiChat` 的 `selectedProviderId` pattern 是模型選擇的參考實作
- `max_agent_steps` 設定已存在，可直接沿用

## Goals / Non-Goals

**Goals:**
- AI 可自主規劃多步 VCS 操作直到達成目標
- 每步透明顯示：實際執行的操作 + AI 語意摘要
- 用戶可中途打字重導方向，AI 調整後繼續
- VCS Chat 可選擇 AI provider/model
- 步驟上限沿用 `max_agent_steps`

**Non-Goals:**
- 不改動 `VcsIntent`、`VcsResult`、`GitClient`、`SvnClient`（執行層完全不動）
- 不改動 write mode gating 邏輯
- 不新增 settings 項目
- 不實作平行多步執行（步驟永遠是序列的）

## Decisions

### 1. `vcs_agent_step`：單步決策 command，不是完整 loop

**選擇**：backend 只提供「給我下一步」的 command (`vcs_agent_step`)，loop 控制邏輯在 frontend (`useVcsAgentLoop`)。

**理由**：與 terminal agent loop 的架構一致（backend 執行單步，frontend 控制 loop）；frontend 可以在每步渲染 UI、接受用戶中途輸入、處理停止邏輯，而不需要長時間佔用 backend。

**替代方案**：backend 執行完整 loop 並 stream 每步結果 → 捨棄，因為中途打字重導需要 frontend 控制權。

---

### 2. History 格式：mixed role array，序列化為 JSON 傳給 backend

```rust
pub enum VcsAgentHistoryEntry {
    User { text: String },
    Step { step_num: u32, operation: String, result_json: String, summary: String },
}
```

**理由**：user message 和 step result 混在同一個 history array，AI 可以感知「用戶在第 2 步後說了什麼」並調整計劃。這是中途重導功能的核心。

---

### 3. `VcsAgentDecision`：AI 回傳的決策結構

```rust
pub struct VcsAgentDecision {
    pub done: bool,
    pub intent: Option<VcsIntent>,     // 下一步要執行的操作（done=false 時必填）
    pub summary: String,               // 給用戶的一句話（每步都有）
    pub final_answer: Option<String>,  // 最終摘要（done=true 時必填）
}
```

**理由**：結構簡單，與 `parse_vcs_intent` 使用相同的 JSON parsing 模式，可複用 fence-stripping 邏輯。

---

### 4. 模型選擇：`provider_id: Option<String>` 加進 `vcs_agent_step`

**選擇**：在 `vcs_agent_step` 加入 optional `provider_id`，frontend 傳入 `selectedProviderId`，backend 透過 `AiRouter` 選擇對應 provider。

**理由**：與 `ai_chat` command 的 `providerId` 參數一致；不需新增設定，零學習成本。

---

### 5. Step 卡片 UI：兩層並列顯示

```
┌─ Step 2/5 ──────────────────────────────────┐
│  ⚙️  git show a3f2c1 --stat                  │  ← commandDisplay
│  💬  查看這個 commit 的詳細改動               │  ← aiSummary
│  [結果卡片：diff / commit / PR...]            │  ← 現有 VcsResult 渲染
└─────────────────────────────────────────────┘
```

複用現有 `VcsMessageBubble` 的 result 渲染，只新增 step header。

## Risks / Trade-offs

- **AI 無限循環** → `max_agent_steps` 強制中止，超限時顯示「已達步驟上限」並回傳目前最佳答案
- **中途打字造成 race condition** → `isRunning` flag 讓 `send()` 在 step 執行期間排隊；新訊息在當前步驟完成後才注入 history 並繼續
- **`vcs_agent_step` AI parse 失敗** → fallback 到 `VcsResult::Error`，loop 終止並顯示錯誤
- **Write intent 在 loop 中出現** → 保留現有 Guarded/ReadOnly gating：Guarded 模式跳出 loop 並顯示確認 UI，用戶確認後可選擇繼續或取消 loop

## Migration Plan

- `useVcsChat` 被 `useVcsAgentLoop` 取代，`VcsView` 改用新 hook
- `vcs_query` command 保留不動（向後相容），`vcs_agent_step` 是新增的
- 舊的單發模式不再從 UI 入口進入，但 command 本身仍可用（例如程式化呼叫）

## Open Questions

- 當 loop 因 write intent + Guarded mode 暫停確認時，確認後要繼續 loop 還是把 write 結果當最後一步結束？→ 初版：把 write 當最後一步結束，不自動繼續
