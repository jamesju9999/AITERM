# Claude subagent 協作側欄 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 AITerm 一般終端機分頁手動跑 `claude` 時，分頁右側出現側欄，即時顯示 Claude 開出的 subagent 樹狀、狀態、目前工具、任務描述／結果摘要與 token 用量。

**Architecture:** AITerm 為每個 PTY 寫一份 `--settings` hooks 檔（Claude Code 的 **HTTP hook**，直接 POST 到 AITerm 本機接收器，網址內含密鑰與 session id，不需任何 helper 程式），並把檔案路徑放進環境變數 `AITERM_CLAUDE_SETTINGS`。AITerm 既有的 shell 整合腳本（在使用者 rc 檔**之後**載入）定義一個 `claude` 函式，自動補上 `--settings`。後端 `claude_activity` 模組把 hook 事件餵進純狀態機（`tree.rs`），狀態變更時 emit 完整快照到 `claude-activity://{sessionId}`；前端側欄先訂閱再取快照。

**Tech Stack:** Rust（axum 0.8、parking_lot、serde_json、tauri 2）、aiterm-core 的 shell 整合腳本（zsh／bash／PowerShell）、React 19 + Vitest。

**與 spec 的差異（探勘後修正，已在 Task 1 回寫 spec）：**
1. 事件管道改用 Claude Code 的 **HTTP hook**（`"type":"http"`，派工的額度 hook 已在用，三平台一樣、不經 shell），所以 spec 的「curl／helper」不需要。
2. 「PATH shim」改為「shell 整合腳本裡的 `claude` 函式」。原因：使用者的 `.zshrc`／`.bashrc` 常把 `~/.local/bin`（原生安裝的 claude 所在）前置到 PATH，shim 目錄會被蓋過；函式永遠贏過 PATH，而 AITerm 的整合腳本本來就是在使用者 rc 之後載入。意圖不變（只攔 AITerm 內以 `claude` 啟動者、不改使用者設定檔）。
3. 「費用」只做 token：Agent 工具的 `PostToolUse` 回應帶 `totalTokens`／`totalToolUseCount`／`totalDurationMs`，沒有金額。不讀 transcript。

**實測得到的 hook 酬載（claude 2.1.300，2026-10-20 用真 claude 探勘）：**
- 子 agent 內的 `PreToolUse`／`PostToolUse` 帶 `agent_id`、`agent_type`；主 agent 的沒有。
- 主 agent 的 `PreToolUse` `tool_name:"Agent"`：`tool_input.description`、`tool_input.subagent_type`、`tool_use_id`。
- `SubagentStart`：只有 `agent_id`、`agent_type`（沒有描述）→ 要靠前一個 `PreToolUse(Agent)` 的 FIFO 佇列認領描述；`PostToolUse(Agent)` 的 `tool_response.agentId` 才是權威對應。
- `PostToolUse(Agent)`：`tool_response` 有 `status`("completed")、`agentId`、`agentType`、`totalTokens`、`totalToolUseCount`、`totalDurationMs`、`handbackReport.text`。
- 子 agent 回報前會呼叫工具 `SubagentHandback`（`tool_input.message` 是結果摘要）。
- 巢狀：子 agent 自己再開 Agent 時，那個 `PreToolUse(Agent)` 的 `agent_id` 是它自己 → 即父節點。
- `SessionEnd` 的 HTTP hook 有送達（Claude 結束時）。
- 尚未實測：失敗 / 背景 agent 的 `tool_response.status` 值（計畫以 `failed`／`error` 視為失敗，其餘非 `completed` 不改狀態；Task 9 實機驗收要補驗）。

---

## File Structure

| 檔案 | 責任 |
|---|---|
| `src-tauri/src/claude_activity/tree.rs`（新） | 純狀態機：hook 事件 → subagent 節點樹。不碰網路／Tauri。 |
| `src-tauri/src/claude_activity/mod.rs`（新） | `ClaudeActivity`：session 登記、settings 檔產生、axum 接收器、emit、快照、清理。 |
| `src-tauri/src/claude_activity/commands.rs`（新） | `claude_activity_snapshot` Tauri 指令。 |
| `src-tauri/src/lib.rs`（改） | 宣告模組、setup 建立並 `manage`、註冊指令。 |
| `src-tauri/src/pty/manager.rs`（改） | `create_with_app_env` 先產生 id，向 `ClaudeActivity` 要環境變數。 |
| `src-tauri/src/pty/commands.rs`（改） | `pty_close` 後 `forget`。 |
| `src-tauri/crates/aiterm-core/src/pty/shell.rs`（改） | zsh／bash／PowerShell 整合腳本加 `claude` 函式 + 真行程測試。 |
| `src/ipc/claudeActivity.ts`（新） | 型別、事件名、`getClaudeActivity`、`listenClaudeActivity`。 |
| `src/components/ClaudeActivity/flattenTree.ts`（新） | 節點 → 帶縮排深度的列。 |
| `src/components/ClaudeActivity/format.ts`（新） | 時間／token 格式化。 |
| `src/components/ClaudeActivity/useClaudeActivity.ts`（新） | 先訂閱再取快照的 hook。 |
| `src/components/ClaudeActivity/ClaudeActivityPanel.tsx` + `.css`（新） | 側欄 UI。 |
| `src/lib/i18n.ts`（改） | zh-TW／en 字串。 |
| `src/components/TerminalView.tsx`（改） | 把面板放進 `SuggestionSplit` 的 sidebar 槽。 |

---

### Task 1: 開分支並把探勘結論回寫 spec

**Files:**
- Modify: `docs/superpowers/specs/2026-10-20-claude-subagent-activity-sidebar-design.md`

- [ ] **Step 1: 開功能分支**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git checkout -b feat/claude-subagent-activity
```
Expected: `Switched to a new branch 'feat/claude-subagent-activity'`（spec 那個 commit 一併帶過來）。

- [ ] **Step 2: 修改 spec 的「架構與資料流」與「決策」**

把 spec 中「決策」表的「注入方式」列改為：

```
| 注入方式 | AITerm 的 shell 整合腳本（在使用者 rc 之後載入）定義 `claude` 函式，自動補 `--settings`；不用 PATH shim（使用者 rc 常把原生安裝的 claude 目錄前置，shim 會被蓋過） |
```

把「架構與資料流」第 1、2 點整段替換為：

```
1. **注入**：`pty_create` 時為該 session 寫一份 `<data_dir>/AITERM/claude-activity/<sessionId>.json`
   （hooks 設定），路徑放進子行程環境變數 `AITERM_CLAUDE_SETTINGS`。zsh／bash／PowerShell 的整合腳本
   各定義 `claude` 函式：參數已含 `--settings` 就原樣轉交（派工路徑），否則補
   `--settings "$AITERM_CLAUDE_SETTINGS"`；使用者自己已定義 `claude` 別名／函式時不覆蓋（該情況不會有
   側欄）。cmd.exe 不支援（Windows 預設 shell 是 PowerShell）。
2. **hooks**：Claude Code 的 **HTTP hook**（`"type":"http"`，不經 shell），訂閱 `PreToolUse`、
   `PostToolUse`、`SubagentStart`、`SubagentStop`、`SessionEnd`，POST 到
   `http://127.0.0.1:<port>/activity/<secret>/<sessionId>/<kind>`。不需要 curl／helper。
   Token 用量不讀 transcript：Agent 工具的 `PostToolUse` 回應已帶 `totalTokens`／
   `totalToolUseCount`；只顯示 token，不顯示金額。
```

同時把「顯示內容」決策的「Token／費用」改為「token 用量」，「錯誤處理」中「helper 逾時 1 秒…」改為「接收器收到事件立即回 `{}`；AITerm 未開時 HTTP hook 連線失敗不影響 Claude」。

- [ ] **Step 3: Commit**

```bash
git add -f docs/superpowers/specs/2026-10-20-claude-subagent-activity-sidebar-design.md docs/superpowers/plans/2026-10-20-claude-subagent-activity-sidebar.md
git commit -m "docs: claude subagent activity — plan, spec amended with probe findings

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: 狀態機 `tree.rs`

**Files:**
- Create: `src-tauri/src/claude_activity/tree.rs`
- Create: `src-tauri/src/claude_activity/mod.rs`（此 task 先只放 `pub mod tree;`）
- Modify: `src-tauri/src/lib.rs`（宣告模組）

- [ ] **Step 1: 宣告模組**

`src-tauri/src/lib.rs` 在 `pub mod bridge;` 附近（字母序）加：

```rust
pub mod claude_activity;
```

建立 `src-tauri/src/claude_activity/mod.rs`：

```rust
pub mod tree;
```

- [ ] **Step 2: 先寫失敗的測試**

建立 `src-tauri/src/claude_activity/tree.rs`，只含型別骨架與測試（實作留空會編譯失敗，這就是「紅」）。先放完整測試：

```rust
//! subagent 樹的純狀態機：只吃 hook 事件，不碰網路與 Tauri，所以能單元測試。
//!
//! 事件形狀來自真 claude（2.1.300）的探勘，見計畫書開頭。

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pre_agent(tool_use_id: &str, caller: Option<&str>, desc: &str, kind: &str) -> Value {
        let mut v = json!({
            "hook_event_name": "PreToolUse", "tool_name": "Agent", "tool_use_id": tool_use_id,
            "tool_input": { "description": desc, "prompt": "p", "subagent_type": kind }
        });
        if let Some(c) = caller { v["agent_id"] = c.into(); v["agent_type"] = "general-purpose".into(); }
        v
    }
    fn start(id: &str, kind: &str) -> Value {
        json!({ "hook_event_name": "SubagentStart", "agent_id": id, "agent_type": kind })
    }
    fn stop(id: &str) -> Value {
        json!({ "hook_event_name": "SubagentStop", "agent_id": id, "agent_type": "general-purpose" })
    }
    fn post_agent(tool_use_id: &str, caller: Option<&str>, desc: &str, agent: &str, status: &str) -> Value {
        let mut v = json!({
            "hook_event_name": "PostToolUse", "tool_name": "Agent", "tool_use_id": tool_use_id,
            "tool_input": { "description": desc },
            "tool_response": { "status": status, "agentId": agent, "agentType": "general-purpose",
                "totalTokens": 20484, "totalToolUseCount": 2, "totalDurationMs": 4958,
                "handbackReport": { "text": "first file is events.jsonl" } }
        });
        if let Some(c) = caller { v["agent_id"] = c.into(); }
        v
    }
    fn tool_pre(agent: &str, tool: &str, input: Value) -> Value {
        json!({ "hook_event_name": "PreToolUse", "agent_id": agent, "agent_type": "general-purpose",
                "tool_name": tool, "tool_input": input })
    }
    fn tool_post(agent: &str, tool: &str) -> Value {
        json!({ "hook_event_name": "PostToolUse", "agent_id": agent, "agent_type": "general-purpose", "tool_name": tool })
    }
    fn node<'a>(t: &'a Tree, id: &str) -> &'a Node {
        t.nodes().iter().find(|n| n.id == id).unwrap_or_else(|| panic!("no node {id}"))
    }

    #[test]
    fn happy_path_links_description_tokens_and_summary() {
        let mut t = Tree::default();
        assert!(!t.apply(Kind::Pre, &pre_agent("tu1", None, "Run ls", "general-purpose"), 1), "只排隊，還沒有節點");
        assert!(t.apply(Kind::SubStart, &start("a1", "general-purpose"), 2));
        assert_eq!(node(&t, "a1").description.as_deref(), Some("Run ls"));
        assert_eq!(node(&t, "a1").status, Status::Running);
        assert!(t.apply(Kind::SubStop, &stop("a1"), 5));
        assert_eq!(node(&t, "a1").status, Status::Done);
        assert!(t.apply(Kind::Post, &post_agent("tu1", None, "Run ls", "a1", "completed"), 6));
        let n = node(&t, "a1");
        assert_eq!(n.tokens, Some(20484));
        assert_eq!(n.tool_count, Some(2));
        assert_eq!(n.summary.as_deref(), Some("first file is events.jsonl"));
        assert_eq!(n.parent, None);
        assert_eq!(n.status, Status::Done);
        assert_eq!(n.ended_ms, Some(5));
    }

    #[test]
    fn parallel_spawns_end_up_with_the_right_descriptions() {
        // 兩個同型別 subagent 平行開：SubagentStart 沒有描述，FIFO 可能配錯，
        // 但 PostToolUse 帶 agentId 與 tool_input.description，是權威對應。
        let mut t = Tree::default();
        t.apply(Kind::Pre, &pre_agent("tu1", None, "task one", "general-purpose"), 1);
        t.apply(Kind::Pre, &pre_agent("tu2", None, "task two", "general-purpose"), 1);
        t.apply(Kind::SubStart, &start("b", "general-purpose"), 2); // 先到的是第二個任務
        t.apply(Kind::SubStart, &start("a", "general-purpose"), 2);
        t.apply(Kind::Post, &post_agent("tu2", None, "task two", "a", "completed"), 3);
        t.apply(Kind::Post, &post_agent("tu1", None, "task one", "b", "completed"), 3);
        assert_eq!(node(&t, "a").description.as_deref(), Some("task two"));
        assert_eq!(node(&t, "b").description.as_deref(), Some("task one"));
    }

    #[test]
    fn nested_subagent_gets_its_spawner_as_parent() {
        let mut t = Tree::default();
        t.apply(Kind::Pre, &pre_agent("tu1", None, "outer", "general-purpose"), 1);
        t.apply(Kind::SubStart, &start("outer", "general-purpose"), 2);
        t.apply(Kind::Pre, &pre_agent("tu2", Some("outer"), "inner", "general-purpose"), 3);
        t.apply(Kind::SubStart, &start("inner", "general-purpose"), 4);
        assert_eq!(node(&t, "inner").parent.as_deref(), Some("outer"));
        assert_eq!(node(&t, "outer").parent, None);
    }

    #[test]
    fn agent_type_filters_which_pending_spawn_is_claimed() {
        let mut t = Tree::default();
        t.apply(Kind::Pre, &pre_agent("tu1", None, "explore it", "Explore"), 1);
        t.apply(Kind::SubStart, &start("g", "general-purpose"), 2);
        assert_eq!(node(&t, "g").description, None, "型別不同不認領");
        t.apply(Kind::SubStart, &start("e", "Explore"), 3);
        assert_eq!(node(&t, "e").description.as_deref(), Some("explore it"));
    }

    #[test]
    fn subagent_tool_calls_show_and_clear_current_tool() {
        let mut t = Tree::default();
        t.apply(Kind::SubStart, &start("a1", "general-purpose"), 1);
        assert!(t.apply(Kind::Pre, &tool_pre("a1", "Bash", json!({ "command": "ls -la\nsecond line" })), 2));
        assert_eq!(node(&t, "a1").current_tool.as_deref(), Some("Bash: ls -la"));
        assert!(t.apply(Kind::Pre, &tool_pre("a1", "Read", json!({ "file_path": "/x/y.rs" })), 3));
        assert_eq!(node(&t, "a1").current_tool.as_deref(), Some("Read: /x/y.rs"));
        assert!(t.apply(Kind::Post, &tool_post("a1", "Read"), 4));
        assert_eq!(node(&t, "a1").current_tool, None);
    }

    #[test]
    fn main_agent_tool_calls_are_ignored() {
        let mut t = Tree::default();
        let ev = json!({ "tool_name": "Bash", "tool_input": { "command": "ls" } });
        assert!(!t.apply(Kind::Pre, &ev, 1));
        assert!(!t.apply(Kind::Post, &ev, 2));
        assert!(t.nodes().is_empty());
    }

    #[test]
    fn handback_message_becomes_the_summary_before_the_post_arrives() {
        let mut t = Tree::default();
        t.apply(Kind::SubStart, &start("a1", "general-purpose"), 1);
        t.apply(Kind::Pre, &tool_pre("a1", "SubagentHandback", json!({ "message": "  all good  " })), 2);
        assert_eq!(node(&t, "a1").summary.as_deref(), Some("all good"));
        assert_eq!(node(&t, "a1").current_tool, None, "SubagentHandback 不當成工具顯示");
    }

    #[test]
    fn stop_without_start_still_creates_a_finished_node() {
        let mut t = Tree::default();
        assert!(t.apply(Kind::SubStop, &stop("late"), 9));
        assert_eq!(node(&t, "late").status, Status::Done);
        assert_eq!(node(&t, "late").ended_ms, Some(9));
    }

    #[test]
    fn duplicate_start_is_idempotent() {
        let mut t = Tree::default();
        assert!(t.apply(Kind::SubStart, &start("a1", "general-purpose"), 1));
        assert!(!t.apply(Kind::SubStart, &start("a1", "general-purpose"), 2));
        assert_eq!(t.nodes().len(), 1);
        assert_eq!(node(&t, "a1").started_ms, 1);
    }

    #[test]
    fn failed_response_overrides_an_earlier_done() {
        let mut t = Tree::default();
        t.apply(Kind::SubStart, &start("a1", "general-purpose"), 1);
        t.apply(Kind::SubStop, &stop("a1"), 2);
        t.apply(Kind::Post, &post_agent("tu1", None, "x", "a1", "failed"), 3);
        assert_eq!(node(&t, "a1").status, Status::Failed);
    }

    #[test]
    fn unknown_status_does_not_end_a_running_agent() {
        let mut t = Tree::default();
        t.apply(Kind::SubStart, &start("a1", "general-purpose"), 1);
        t.apply(Kind::Post, &post_agent("tu1", None, "x", "a1", "async_launched"), 2);
        assert_eq!(node(&t, "a1").status, Status::Running);
    }

    #[test]
    fn session_end_interrupts_only_running_nodes() {
        let mut t = Tree::default();
        t.apply(Kind::SubStart, &start("done", "general-purpose"), 1);
        t.apply(Kind::SubStop, &stop("done"), 2);
        t.apply(Kind::SubStart, &start("run", "general-purpose"), 3);
        t.apply(Kind::Pre, &tool_pre("run", "Bash", json!({ "command": "sleep 99" })), 4);
        t.apply(Kind::Pre, &pre_agent("tu9", None, "never started", "general-purpose"), 5);
        assert!(t.apply(Kind::SessionEnd, &json!({}), 10));
        assert_eq!(node(&t, "done").status, Status::Done);
        assert_eq!(node(&t, "run").status, Status::Interrupted);
        assert_eq!(node(&t, "run").ended_ms, Some(10));
        assert_eq!(node(&t, "run").current_tool, None);
        assert!(!t.apply(Kind::SessionEnd, &json!({}), 11), "第二次沒有變化");
    }

    #[test]
    fn oldest_finished_nodes_are_dropped_past_the_cap() {
        let mut t = Tree::default();
        for i in 0..(MAX_NODES + 5) {
            let id = format!("n{i}");
            t.apply(Kind::SubStart, &start(&id, "general-purpose"), i as u64);
            t.apply(Kind::SubStop, &stop(&id), i as u64);
        }
        assert_eq!(t.nodes().len(), MAX_NODES);
        assert_eq!(t.nodes()[0].id, "n5");
    }

    #[test]
    fn summary_is_truncated() {
        let mut t = Tree::default();
        t.apply(Kind::SubStart, &start("a1", "general-purpose"), 1);
        let long = "x".repeat(1000);
        t.apply(Kind::Pre, &tool_pre("a1", "SubagentHandback", json!({ "message": long })), 2);
        let s = node(&t, "a1").summary.clone().unwrap();
        assert_eq!(s.chars().count(), 401, "400 個字 + …");
        assert!(s.ends_with('…'));
    }

    #[test]
    fn kind_parse_matches_the_urls_in_the_settings_file() {
        assert_eq!(Kind::parse("pre"), Some(Kind::Pre));
        assert_eq!(Kind::parse("post"), Some(Kind::Post));
        assert_eq!(Kind::parse("sub-start"), Some(Kind::SubStart));
        assert_eq!(Kind::parse("sub-stop"), Some(Kind::SubStop));
        assert_eq!(Kind::parse("session-end"), Some(Kind::SessionEnd));
        assert_eq!(Kind::parse("nope"), None);
    }
}
```

- [ ] **Step 3: 確認測試是紅的**

Run: `cd src-tauri && cargo test --lib claude_activity::tree 2>&1 | tail -15`
Expected: 編譯失敗（`cannot find type Tree/Node/Kind/Status…`）。（本機需先跑過 `scripts/setup-uv-mac.sh`，見 CLAUDE.md。）

- [ ] **Step 4: 寫實作**

在 `tree.rs` 的 `#[cfg(test)]` 之前（檔案開頭註解之後）加入：

```rust
use std::collections::VecDeque;

use serde::Serialize;
use serde_json::Value;

/// 一個 session 最多留幾個節點；超過就丟最舊的「已結束」節點。
pub const MAX_NODES: usize = 200;
const SUMMARY_MAX_CHARS: usize = 400;
const HINT_MAX_CHARS: usize = 80;

static NULL: Value = Value::Null;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Done,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub id: String,
    pub parent: Option<String>,
    pub agent_type: String,
    pub description: Option<String>,
    pub status: Status,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub current_tool: Option<String>,
    pub summary: Option<String>,
    pub tokens: Option<u64>,
    pub tool_count: Option<u64>,
}

/// 事件種類；字串就是 settings 檔裡 hook 網址的最後一段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pre,
    Post,
    SubStart,
    SubStop,
    SessionEnd,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "pre" => Some(Kind::Pre),
            "post" => Some(Kind::Post),
            "sub-start" => Some(Kind::SubStart),
            "sub-stop" => Some(Kind::SubStop),
            "session-end" => Some(Kind::SessionEnd),
            _ => None,
        }
    }
}

/// 主 agent（或某個 subagent）剛呼叫 Agent 工具、但對應的 `SubagentStart` 還沒到。
struct Pending {
    tool_use_id: String,
    parent: Option<String>,
    agent_type: Option<String>,
    description: Option<String>,
}

#[derive(Default)]
pub struct Tree {
    nodes: Vec<Node>,
    pending: VecDeque<Pending>,
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or(&NULL)
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_owned)
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// 「Bash: ls -la」這種一行提示；沒有可顯示的細節就只回工具名。
fn tool_hint(tool: &str, input: &Value) -> String {
    let detail = match tool {
        "Bash" => str_of(input, "command"),
        "Read" | "Edit" | "Write" | "NotebookEdit" => str_of(input, "file_path"),
        "Grep" | "Glob" => str_of(input, "pattern"),
        "WebFetch" => str_of(input, "url"),
        "WebSearch" => str_of(input, "query"),
        _ => None,
    };
    match detail {
        Some(d) => truncate(&format!("{tool}: {}", d.lines().next().unwrap_or("")), HINT_MAX_CHARS),
        None => tool.to_owned(),
    }
}

impl Tree {
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// 套用一個 hook 事件；回傳節點有沒有變化（沒變就不必通知前端）。
    pub fn apply(&mut self, kind: Kind, body: &Value, now_ms: u64) -> bool {
        let before = self.nodes.clone();
        let tool = str_of(body, "tool_name");
        let caller = str_of(body, "agent_id");
        let is_agent_tool = matches!(tool.as_deref(), Some("Agent") | Some("Task"));
        match kind {
            Kind::Pre if is_agent_tool => self.queue_spawn(body, caller),
            Kind::Pre => {
                if let (Some(tool), Some(agent)) = (tool.as_deref(), caller.as_deref()) {
                    self.on_tool_pre(tool, agent, body, now_ms);
                }
            }
            Kind::Post if is_agent_tool => self.on_agent_post(body, caller, now_ms),
            Kind::Post => {
                if let Some(i) = caller.as_deref().and_then(|a| self.index_of(a)) {
                    self.nodes[i].current_tool = None;
                }
            }
            Kind::SubStart => {
                if let Some(id) = str_of(body, "agent_id") {
                    self.on_start(&id, str_of(body, "agent_type"), now_ms);
                }
            }
            Kind::SubStop => {
                if let Some(id) = str_of(body, "agent_id") {
                    let i = self.ensure(&id, str_of(body, "agent_type"), now_ms);
                    Self::finish(&mut self.nodes[i], Status::Done, now_ms);
                }
            }
            Kind::SessionEnd => {
                for n in &mut self.nodes {
                    Self::finish(n, Status::Interrupted, now_ms);
                }
                self.pending.clear();
            }
        }
        self.trim();
        self.nodes != before
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    fn ensure(&mut self, id: &str, agent_type: Option<String>, now_ms: u64) -> usize {
        if let Some(i) = self.index_of(id) {
            return i;
        }
        self.nodes.push(Node {
            id: id.to_owned(),
            parent: None,
            agent_type: agent_type.unwrap_or_else(|| "agent".to_owned()),
            description: None,
            status: Status::Running,
            started_ms: now_ms,
            ended_ms: None,
            current_tool: None,
            summary: None,
            tokens: None,
            tool_count: None,
        });
        self.nodes.len() - 1
    }

    /// 只有還在跑的節點會被改狀態；已經結束的保持原樣。
    fn finish(node: &mut Node, status: Status, now_ms: u64) {
        if node.status == Status::Running {
            node.status = status;
            node.ended_ms = Some(now_ms);
        }
        node.current_tool = None;
    }

    fn queue_spawn(&mut self, body: &Value, caller: Option<String>) {
        let Some(tool_use_id) = str_of(body, "tool_use_id") else { return };
        let input = field(body, "tool_input");
        self.pending.push_back(Pending {
            tool_use_id,
            parent: caller,
            agent_type: str_of(input, "subagent_type"),
            description: str_of(input, "description"),
        });
    }

    fn on_start(&mut self, id: &str, agent_type: Option<String>, now_ms: u64) {
        let existed = self.index_of(id).is_some();
        let i = self.ensure(id, agent_type, now_ms);
        if existed && self.nodes[i].description.is_some() {
            return; // 重複事件
        }
        let want = self.nodes[i].agent_type.clone();
        let pos = self
            .pending
            .iter()
            .position(|p| p.agent_type.as_deref().map_or(true, |t| t == want));
        if let Some(p) = pos.and_then(|pos| self.pending.remove(pos)) {
            let n = &mut self.nodes[i];
            n.parent = p.parent;
            n.description = p.description;
        }
    }

    fn on_tool_pre(&mut self, tool: &str, agent: &str, body: &Value, now_ms: u64) {
        let input = field(body, "tool_input");
        let i = self.ensure(agent, str_of(body, "agent_type"), now_ms);
        if tool == "SubagentHandback" {
            if let Some(m) = str_of(input, "message") {
                self.nodes[i].summary = Some(truncate(&m, SUMMARY_MAX_CHARS));
            }
            return;
        }
        self.nodes[i].current_tool = Some(tool_hint(tool, input));
    }

    fn on_agent_post(&mut self, body: &Value, caller: Option<String>, now_ms: u64) {
        let resp = field(body, "tool_response");
        let claimed = str_of(body, "tool_use_id")
            .and_then(|id| self.pending.iter().position(|p| p.tool_use_id == id))
            .and_then(|pos| self.pending.remove(pos));
        let Some(agent_id) = str_of(resp, "agentId") else { return };
        let i = self.ensure(&agent_id, str_of(resp, "agentType"), now_ms);
        let n = &mut self.nodes[i];
        // tool_input.description 是權威來源（平行開多個時 FIFO 認領可能配錯，這裡校正）。
        if let Some(d) = str_of(field(body, "tool_input"), "description") {
            n.description = Some(d);
        } else if n.description.is_none() {
            n.description = claimed.and_then(|p| p.description);
        }
        if n.parent.is_none() {
            n.parent = caller;
        }
        if let Some(t) = resp.get("totalTokens").and_then(Value::as_u64) {
            n.tokens = Some(t);
        }
        if let Some(c) = resp.get("totalToolUseCount").and_then(Value::as_u64) {
            n.tool_count = Some(c);
        }
        let text = resp
            .pointer("/handbackReport/text")
            .and_then(Value::as_str)
            .or_else(|| resp.pointer("/content/0/text").and_then(Value::as_str));
        if let Some(t) = text {
            n.summary = Some(truncate(t, SUMMARY_MAX_CHARS));
        }
        match resp.get("status").and_then(Value::as_str) {
            Some("completed") => Self::finish(n, Status::Done, now_ms),
            Some("failed") | Some("error") => {
                n.status = Status::Failed;
                n.ended_ms.get_or_insert(now_ms);
                n.current_tool = None;
            }
            _ => {}
        }
    }

    fn trim(&mut self) {
        while self.nodes.len() > MAX_NODES {
            match self.nodes.iter().position(|n| n.status != Status::Running) {
                Some(i) => {
                    self.nodes.remove(i);
                }
                None => break,
            }
        }
    }
}
```

- [ ] **Step 5: 跑到綠**

Run: `cd src-tauri && cargo test --lib claude_activity::tree 2>&1 | grep -E "^test |test result"`
Expected: 14 條 `ok`，`test result: ok. 14 passed`。

- [ ] **Step 6: 證明測試有咬到（突變）**

暫時把 `on_agent_post` 裡 `Some("failed") | Some("error") => {…}` 整段改成 `Some("failed") | Some("error") => Self::finish(n, Status::Failed, now_ms),`，跑 `cargo test --lib claude_activity::tree::tests::failed_response`，Expected: FAIL（Done 沒被覆蓋）。還原。再暫時把 `on_agent_post` 開頭 `description` 那段的 `if let Some(d) = …tool_input…` 整個註解掉，跑 `parallel_spawns`，Expected: FAIL。還原後全綠再繼續。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/claude_activity src-tauri/src/lib.rs
git commit -m "feat(claude-activity): subagent tree state machine

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: 接收器與 session 登記 `mod.rs`

**Files:**
- Modify: `src-tauri/src/claude_activity/mod.rs`

- [ ] **Step 1: 先寫失敗的測試**

把 `mod.rs` 改成下列內容的測試部分（實作在 Step 3 補上；此時缺型別，編譯失敗即為紅）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aiterm-claude-activity-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    type Seen = Arc<StdMutex<Vec<(String, Vec<Node>)>>>;
    fn activity(tag: &str) -> (Arc<ClaudeActivity>, Seen) {
        let seen: Seen = Arc::default();
        let sink = seen.clone();
        let emit: Emitter = Arc::new(move |sid, nodes| sink.lock().unwrap().push((sid.to_owned(), nodes.to_vec())));
        (ClaudeActivity::new(temp_dir(tag), emit), seen)
    }

    #[test]
    fn settings_json_points_every_hook_at_its_own_path() {
        let v: serde_json::Value = serde_json::from_str(&settings_json("http://h/activity/s/sid")).unwrap();
        for (event, path) in [
            ("PreToolUse", "pre"),
            ("PostToolUse", "post"),
            ("SubagentStart", "sub-start"),
            ("SubagentStop", "sub-stop"),
            ("SessionEnd", "session-end"),
        ] {
            let h = &v["hooks"][event][0]["hooks"][0];
            assert_eq!(h["type"], "http", "{event}");
            assert_eq!(h["url"], format!("http://h/activity/s/sid/{path}"), "{event}");
        }
    }

    #[test]
    fn prepare_writes_a_settings_file_and_returns_its_path_in_env() {
        let (a, _) = activity("prepare");
        let env = a.prepare("sess-1");
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].0, SETTINGS_ENV);
        let text = std::fs::read_to_string(&env[0].1).unwrap();
        assert!(text.contains("/activity/"), "{text}");
        assert!(text.contains("/sess-1/pre"), "{text}");
    }

    #[test]
    fn ingest_updates_the_snapshot_and_emits_once_per_change() {
        let (a, seen) = activity("ingest");
        a.prepare("s1");
        let start = json!({ "agent_id": "a1", "agent_type": "general-purpose" });
        a.ingest("s1", Kind::SubStart, &start, 1);
        a.ingest("s1", Kind::SubStart, &start, 2); // 重複：不再通知
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "s1");
        assert_eq!(a.snapshot("s1").len(), 1);
    }

    #[test]
    fn events_for_unregistered_sessions_are_dropped() {
        let (a, seen) = activity("unknown");
        a.ingest("ghost", Kind::SubStart, &json!({ "agent_id": "a1" }), 1);
        assert!(seen.lock().unwrap().is_empty());
        assert!(a.snapshot("ghost").is_empty());
    }

    #[test]
    fn forget_drops_state_and_deletes_the_settings_file() {
        let (a, _) = activity("forget");
        let env = a.prepare("s1");
        a.forget("s1");
        assert!(!std::path::Path::new(&env[0].1).exists());
        a.ingest("s1", Kind::SubStart, &json!({ "agent_id": "a1" }), 1);
        assert!(a.snapshot("s1").is_empty(), "忘記之後的事件不再被接受");
    }

    #[test]
    fn forget_ignores_ids_that_were_never_registered() {
        let (a, _) = activity("forget-unknown");
        let victim = a.dir.join("../victim.json");
        std::fs::write(&victim, "x").ok();
        a.forget("../victim");
        assert!(victim.exists(), "沒登記過的 id 不能拿來刪檔");
        std::fs::remove_file(&victim).ok();
    }

    #[test]
    fn stale_settings_files_are_purged_on_startup_but_fresh_ones_stay() {
        let dir = temp_dir("purge");
        let old = dir.join("old.json");
        let fresh = dir.join("fresh.json");
        std::fs::write(&old, "{}").unwrap();
        std::fs::write(&fresh, "{}").unwrap();
        let f = std::fs::File::options().write(true).open(&old).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 3600)).unwrap();
        purge_stale(&dir);
        assert!(!old.exists());
        assert!(fresh.exists());
    }

    /// 真的走一次 HTTP：驗證密鑰、session id、事件種類都從網址解出來，
    /// 而且錯誤的密鑰不會被接受。
    #[tokio::test]
    async fn the_http_receiver_accepts_the_real_urls_and_rejects_a_wrong_secret() {
        let (a, seen) = activity("http");
        let env = a.prepare("s1");
        let settings: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&env[0].1).unwrap()).unwrap();
        let url = settings["hooks"]["SubagentStart"][0]["hooks"][0]["url"].as_str().unwrap().to_owned();
        let client = reqwest::Client::new();

        let bad = url.replace("/activity/", "/activity/x");
        client.post(&bad).json(&json!({ "agent_id": "a1" })).send().await.unwrap();
        assert!(seen.lock().unwrap().is_empty(), "密鑰錯誤不能被接受");

        let resp = client
            .post(&url)
            .json(&json!({ "agent_id": "a1", "agent_type": "general-purpose" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.text().await.unwrap(), "{}", "Claude 會把回應當 hook 輸出解析，空物件＝沒意見");
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(a.snapshot("s1")[0].id, "a1");
    }
}
```

- [ ] **Step 2: 確認是紅的**

Run: `cd src-tauri && cargo test --lib claude_activity 2>&1 | tail -8`
Expected: 編譯失敗（找不到 `ClaudeActivity` 等）。

- [ ] **Step 3: 寫實作**

`mod.rs` 在測試前加入：

```rust
//! 一般終端機分頁裡手動跑的 Claude：subagent 協作側欄的後端。
//!
//! 流程：`pty_create` → `prepare` 為這個 session 寫一份 hooks 設定檔、把路徑放進
//! `AITERM_CLAUDE_SETTINGS`（shell 整合腳本的 `claude` 函式會補上 `--settings`）→
//! Claude 的 HTTP hook 打到本機接收器 → `Tree` 狀態機 → emit 完整快照給前端。

pub mod commands;
pub mod tree;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use axum::extract::{Path as AxPath, State};
use axum::routing::post;
use parking_lot::Mutex;
use serde_json::Value;

use tree::{Kind, Node, Tree};

/// 子行程環境變數：該 session 的 hooks 設定檔路徑。
pub const SETTINGS_ENV: &str = "AITERM_CLAUDE_SETTINGS";
/// 殘留（app 當掉沒清到）的設定檔多久後清掉。
const STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);

/// 狀態變更時的通知：(session id, 完整快照)。正式版 emit Tauri 事件，測試收進 Vec。
pub type Emitter = Arc<dyn Fn(&str, &[Node]) + Send + Sync>;

/// 交給 `claude --settings` 的設定檔內容；`base` 是 `http://127.0.0.1:<port>/activity/<secret>/<sid>`。
pub fn settings_json(base: &str) -> String {
    let hook = |path: &str| serde_json::json!([{ "hooks": [{ "type": "http", "url": format!("{base}/{path}") }] }]);
    serde_json::json!({
        "hooks": {
            "PreToolUse": hook("pre"),
            "PostToolUse": hook("post"),
            "SubagentStart": hook("sub-start"),
            "SubagentStop": hook("sub-stop"),
            "SessionEnd": hook("session-end"),
        }
    })
    .to_string()
}

fn purge_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > STALE_AFTER);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct HookServer {
    port: u16,
    secret: String,
}

struct HandlerState {
    activity: Arc<ClaudeActivity>,
    secret: String,
}

pub struct ClaudeActivity {
    dir: PathBuf,
    emit: Emitter,
    sessions: Mutex<HashMap<String, Tree>>,
    /// 第一次 `prepare` 時才啟動接收器；啟動失敗＝`None`（分頁照開，只是沒有側欄）。
    server: OnceLock<Option<HookServer>>,
}

impl ClaudeActivity {
    pub fn new(dir: PathBuf, emit: Emitter) -> Arc<Self> {
        purge_stale(&dir);
        Arc::new(Self {
            dir,
            emit,
            sessions: Mutex::new(HashMap::new()),
            server: OnceLock::new(),
        })
    }

    /// 為新 PTY 準備：登記 session、寫設定檔，回傳要加進子行程的環境變數。
    /// 任何一步失敗都回空（不注入比注入一個壞掉的設定檔更好除錯）。
    pub fn prepare(self: &Arc<Self>, session_id: &str) -> Vec<(String, String)> {
        let Some(server) = self.server.get_or_init(|| self.start_server()).as_ref() else {
            return Vec::new();
        };
        let base = format!("http://127.0.0.1:{}/activity/{}/{}", server.port, server.secret, session_id);
        let path = self.dir.join(format!("{session_id}.json"));
        let written = std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(&path, settings_json(&base)));
        if let Err(e) = written {
            eprintln!("claude activity: write settings failed: {e}");
            return Vec::new();
        }
        self.sessions.lock().insert(session_id.to_owned(), Tree::default());
        vec![(SETTINGS_ENV.to_owned(), path.to_string_lossy().into_owned())]
    }

    /// 套用一個 hook 事件；沒登記過的 session 一律丟棄（本機其他程式亂打也不會污染）。
    pub fn ingest(&self, session_id: &str, kind: Kind, body: &Value, now_ms: u64) {
        let snapshot = {
            let mut map = self.sessions.lock();
            let Some(tree) = map.get_mut(session_id) else { return };
            if !tree.apply(kind, body, now_ms) {
                return;
            }
            tree.nodes().to_vec()
        };
        (self.emit)(session_id, &snapshot);
    }

    pub fn snapshot(&self, session_id: &str) -> Vec<Node> {
        self.sessions.lock().get(session_id).map(|t| t.nodes().to_vec()).unwrap_or_default()
    }

    /// PTY 關閉：丟掉狀態並刪設定檔。只處理登記過的 id，所以不能拿路徑字元亂刪檔。
    pub fn forget(&self, session_id: &str) {
        if self.sessions.lock().remove(session_id).is_some() {
            let _ = std::fs::remove_file(self.dir.join(format!("{session_id}.json")));
        }
    }

    fn start_server(self: &Arc<Self>) -> Option<HookServer> {
        // 同步綁埠（`prepare` 從同步的 Tauri 指令呼叫，此時不在 async runtime 裡），
        // 之後交給 Tauri 的 runtime 去 serve。
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
        std_listener.set_nonblocking(true).ok()?;
        let port = std_listener.local_addr().ok()?.port();
        let secret = uuid::Uuid::new_v4().simple().to_string();
        let state = Arc::new(HandlerState { activity: self.clone(), secret: secret.clone() });
        let app = axum::Router::new()
            .route("/activity/{secret}/{sid}/{kind}", post(handle))
            .with_state(state);
        tauri::async_runtime::spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(std_listener) else { return };
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("claude activity server stopped: {e}");
            }
        });
        Some(HookServer { port, secret })
    }
}

async fn handle(
    State(st): State<Arc<HandlerState>>,
    AxPath((secret, sid, kind)): AxPath<(String, String, String)>,
    body: axum::body::Bytes,
) -> &'static str {
    // 只接受自己發出去的網址；本機其他程式猜不到密鑰。
    if secret == st.secret {
        if let Some(kind) = Kind::parse(&kind) {
            let json: Value = serde_json::from_slice(&body).unwrap_or_default();
            st.activity.ingest(&sid, kind, &json, now_ms());
        }
    }
    // Claude Code 會把回應當成 hook 輸出解析；空物件 = 沒有意見。
    "{}"
}
```

建立 `src-tauri/src/claude_activity/commands.rs`：

```rust
use std::sync::Arc;

use tauri::State;

use super::tree::Node;
use super::ClaudeActivity;

/// 側欄掛載時先取一次快照（事件可能在它訂閱之前就送出了）。
#[tauri::command]
pub fn claude_activity_snapshot(activity: State<'_, Arc<ClaudeActivity>>, session_id: String) -> Vec<Node> {
    activity.snapshot(&session_id)
}
```

- [ ] **Step 4: 跑到綠**

Run: `cd src-tauri && cargo test --lib claude_activity 2>&1 | grep -E "^test |test result|error"`
Expected: tree 14 條 + mod 8 條全 `ok`。

- [ ] **Step 5: 證明密鑰檢查有咬到**

暫時把 `handle` 裡 `if secret == st.secret {` 改成 `if true {`，跑 `cargo test --lib the_http_receiver`，Expected: FAIL（"密鑰錯誤不能被接受"）。還原。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/claude_activity
git commit -m "feat(claude-activity): hook receiver, per-session settings, snapshot

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 接上 Tauri（setup、PTY 建立／關閉、指令）

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/pty/manager.rs`
- Modify: `src-tauri/src/pty/commands.rs`

- [ ] **Step 1: `lib.rs` 建立並 manage**

在 `.setup(|app| {` 之後、`resident::setup(app.handle());` 之前加入（先 `Read` 該處確認縮排）：

```rust
            {
                use tauri::Emitter as _;
                let handle = app.handle().clone();
                let dir = dirs::data_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("AITERM")
                    .join("claude-activity");
                let emit: claude_activity::Emitter = Arc::new(move |sid, nodes| {
                    let _ = handle.emit(&format!("claude-activity://{sid}"), nodes);
                });
                app.manage(claude_activity::ClaudeActivity::new(dir, emit));
            }
```

在 `invoke_handler` 的 `// PTY` 區塊 `pty_close,` 之後加一行：

```rust
            claude_activity::commands::claude_activity_snapshot,
```

- [ ] **Step 2: `create_with_app_env` 注入環境變數**

`src-tauri/src/pty/manager.rs`：`use tauri::{AppHandle, Emitter};` 改成 `use tauri::{AppHandle, Emitter, Manager};`。把函式中

```rust
    envs.extend(extra_env);

    let id = uuid::Uuid::new_v4().to_string();
    let event_name = data_event_name(&id);
```

改成

```rust
    envs.extend(extra_env);

    let id = uuid::Uuid::new_v4().to_string();
    // 手動在這個分頁跑的 claude 會透過 shell 整合腳本的 `claude` 函式讀到這個設定檔，
    // 它的 subagent 事件才會打回 AITerm 的側欄。id 必須在 spawn 前就決定，設定檔才寫得出網址。
    if let Some(activity) = app.try_state::<std::sync::Arc<crate::claude_activity::ClaudeActivity>>() {
        envs.extend(activity.prepare(&id));
    }
    let event_name = data_event_name(&id);
```

- [ ] **Step 3: `pty_close` 清理**

`src-tauri/src/pty/commands.rs`：

```rust
#[tauri::command]
pub fn pty_close(
    manager: State<'_, std::sync::Arc<PtyManager>>,
    activity: State<'_, std::sync::Arc<crate::claude_activity::ClaudeActivity>>,
    id: String,
) -> Result<(), PtyError> {
    let result = manager.close(&id);
    activity.forget(&id);
    result
}
```

- [ ] **Step 4: 編譯並跑全部 Rust 測試**

Run: `cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result|error\[|FAILED|failed"`
Expected: 每個 `test result:` 都是 `ok`（已知 aiterm-core 的 pty 測試可能因環境 openpty 競爭失敗，若失敗先單獨重跑那條，確認與本次改動無關再記錄）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/pty
git commit -m "feat(claude-activity): wire into PTY create/close and Tauri

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: shell 整合腳本的 `claude` 函式（zsh／bash／PowerShell）

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/pty/shell.rs`

設計重點（寫進程式註解）：函式贏過 PATH；使用者自己已有 `claude` 別名／函式就不覆蓋；參數已含 `--settings`（派工路徑）原樣轉交；`AITERM_CLAUDE_SETTINGS` 沒設（aiterm-host 等）時原樣轉交，行為與沒有這個函式一樣。

- [ ] **Step 1: 先寫失敗的真行程測試**

在 `shell.rs` 的 `#[cfg(test)] mod tests`（檔案後段已有）內加入。先 `Read` 檔尾確認 `mod tests` 位置與已有的 `use`。測試用假 `claude` 腳本把收到的參數寫進檔案：

```rust
    /// 假的 claude：把收到的每個參數一行寫進 $FAKE_CLAUDE_OUT。
    #[cfg(not(windows))]
    fn fake_claude_bin() -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("aiterm-fake-claude-{}", uuid::Uuid::new_v4()));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = bin.join("claude");
        std::fs::write(&script, "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"$FAKE_CLAUDE_OUT\"\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (root, bin)
    }

    /// 用 AITerm 實際產生的整合設定啟動一個互動式 shell 跑 `line`，回傳假 claude 收到的參數。
    #[cfg(not(windows))]
    fn run_claude_line(shell: &str, line: &str, settings_env: Option<&str>) -> Option<Vec<String>> {
        use std::process::Command;
        if !std::path::Path::new(shell).exists() {
            return None;
        }
        let (root, bin) = fake_claude_bin();
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let out = root.join("args.txt");
        let spec = inject_shell_integration(PathBuf::from(shell));
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args).args(["-i", "-c", line]);
        cmd.env_clear()
            .env("HOME", &home)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("FAKE_CLAUDE_OUT", &out);
        for (k, v) in &spec.envs {
            cmd.env(k, v);
        }
        if let Some(s) = settings_env {
            cmd.env("AITERM_CLAUDE_SETTINGS", s);
        }
        let status = cmd.status().unwrap();
        assert!(status.success(), "{shell} exited with {status}");
        let text = std::fs::read_to_string(&out).ok()?;
        Some(text.lines().map(str::to_owned).collect())
    }

    #[cfg(not(windows))]
    #[test]
    fn claude_function_adds_settings_in_bash_and_zsh() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            let Some(args) = run_claude_line(shell, "claude --model haiku 'a b'", Some("/tmp/s p/x.json")) else {
                eprintln!("skip {shell}: not installed");
                continue;
            };
            assert_eq!(args, ["--settings", "/tmp/s p/x.json", "--model", "haiku", "a b"], "{shell}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn claude_function_leaves_explicit_settings_alone() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            for line in ["claude --settings /d/s.json -p hi", "claude --settings=/d/s.json -p hi"] {
                let Some(args) = run_claude_line(shell, line, Some("/tmp/x.json")) else { continue };
                assert!(!args.contains(&"/tmp/x.json".to_owned()), "{shell} {line}: {args:?}");
                assert_eq!(args.last().map(String::as_str), Some("hi"), "{shell} {line}");
            }
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn claude_function_passes_through_when_no_settings_env() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            let Some(args) = run_claude_line(shell, "claude -p hi", None) else { continue };
            assert_eq!(args, ["-p", "hi"], "{shell}");
        }
    }

    #[test]
    fn powershell_script_defines_the_claude_function_without_clobbering_user_ones() {
        let s = POWERSHELL_INTEGRATION_SCRIPT;
        assert!(s.contains("function global:claude"));
        assert!(s.contains("$env:AITERM_CLAUDE_SETTINGS"));
        assert!(
            s.contains("-not (Get-Command claude -CommandType Function,Alias"),
            "使用者自己的 claude 別名／函式優先"
        );
        assert!(s.contains("--settings"));
    }
```

另外加一條「使用者自己的別名不被蓋掉」的真行程測試（bash 與 zsh 都用 `~/.bashrc`／`~/.zshrc` 定義 alias）：

```rust
    #[cfg(not(windows))]
    #[test]
    fn claude_function_does_not_override_a_user_alias() {
        for (shell, rc) in [("/bin/bash", ".bashrc"), ("/bin/zsh", ".zshrc")] {
            if !std::path::Path::new(shell).exists() {
                continue;
            }
            let (root, bin) = fake_claude_bin();
            let home = root.join("home");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(home.join(rc), "alias claude='claude --user-alias'\nshopt -s expand_aliases 2>/dev/null\n").unwrap();
            let out = root.join("args.txt");
            let spec = inject_shell_integration(PathBuf::from(shell));
            let mut cmd = std::process::Command::new(&spec.program);
            cmd.args(&spec.args).args(["-i", "-c", "claude hi"]);
            cmd.env_clear()
                .env("HOME", &home)
                .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
                .env("FAKE_CLAUDE_OUT", &out)
                .env("AITERM_CLAUDE_SETTINGS", "/tmp/x.json");
            for (k, v) in &spec.envs {
                cmd.env(k, v);
            }
            assert!(cmd.status().unwrap().success());
            let args: Vec<String> = std::fs::read_to_string(&out).unwrap().lines().map(str::to_owned).collect();
            assert_eq!(args, ["--user-alias", "hi"], "{shell}：別名要原樣生效，不被 AITerm 的函式蓋掉");
        }
    }
```

> 注意：`bash -i -c` 預設不展開別名；上面 rc 內 `shopt -s expand_aliases` 在 bash 讓 `-c` 字串可展開別名。若 zsh 的 `-c` 行為不同導致此測試對 zsh 不穩，改用 `echo 'claude hi' | zsh -i` 的 stdin 形式；以實際跑出的行為為準，並在測試註解寫明原因。

- [ ] **Step 2: 確認是紅的**

Run: `cd src-tauri && cargo test -p aiterm-core claude_function 2>&1 | grep -E "^test |test result|panicked|assert"`
Expected: `claude_function_adds_settings…` FAIL（args 缺 `--settings`，因為函式還不存在）；`passes_through` 此時可能已綠（本來就原樣轉交）— 這條是對照組，保留。PowerShell 那條 FAIL。

- [ ] **Step 3: 實作**

`shell.rs` 的 zsh 腳本：在 `add-zsh-hook precmd __aiterm_precmd` 之後、`"#;` 之前加入：

```zsh

# ── AITerm: Claude subagent 協作側欄 ──
# 在使用者 rc 之後才定義，所以函式一定贏過 PATH 上任何位置的 claude。
# 使用者自己已有 claude 別名／函式就不覆蓋（該分頁不會有側欄，但行為完全是他原本的）。
# 沒有 AITERM_CLAUDE_SETTINGS（例如 aiterm-host）時原樣轉交。
if (( ! ${+aliases[claude]} )) && (( ! ${+functions[claude]} )); then
  claude() {
    local a
    if [[ -n "$AITERM_CLAUDE_SETTINGS" ]]; then
      for a in "$@"; do
        case "$a" in --settings|--settings=*) command claude "$@"; return;; esac
      done
      command claude --settings "$AITERM_CLAUDE_SETTINGS" "$@"
    else
      command claude "$@"
    fi
  }
fi
```

bash 腳本：在 `PROMPT_COMMAND="__aiterm_precmd…"` 那行之後、`"#;` 之前加入：

```bash

# ── AITerm: Claude subagent 協作側欄 ──
# 在使用者 rc 之後才定義，所以函式一定贏過 PATH 上任何位置的 claude。
# 使用者自己已有 claude 別名／函式就不覆蓋；沒有 AITERM_CLAUDE_SETTINGS 時原樣轉交。
if ! alias claude >/dev/null 2>&1 && ! declare -F claude >/dev/null 2>&1; then
  claude() {
    local a
    if [[ -n "$AITERM_CLAUDE_SETTINGS" ]]; then
      for a in "$@"; do
        case "$a" in --settings|--settings=*) command claude "$@"; return;; esac
      done
      command claude --settings "$AITERM_CLAUDE_SETTINGS" "$@"
    else
      command claude "$@"
    fi
  }
fi
```

`POWERSHELL_INTEGRATION_SCRIPT`：在最後一個 `}`（`Set-PSReadLineKeyHandler … -ScriptBlock {…}` 結尾）之後、`"#;` 之前加入：

```powershell

# ── AITerm: Claude subagent 協作側欄 ──
# 使用者自己已有 claude 函式／別名就不覆蓋；沒有 AITERM_CLAUDE_SETTINGS 時原樣轉交。
if ($env:AITERM_CLAUDE_SETTINGS -and -not (Get-Command claude -CommandType Function,Alias -ErrorAction SilentlyContinue)) {
    function global:claude {
        $real = Get-Command claude -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
        if (-not $real) { Write-Error "claude: command not found"; return }
        $hasSettings = $false
        foreach ($a in $args) {
            if ($a -is [string] -and ($a -eq '--settings' -or $a -like '--settings=*')) { $hasSettings = $true }
        }
        if ($hasSettings -or -not $env:AITERM_CLAUDE_SETTINGS) {
            & $real.Source @args
        } else {
            & $real.Source --settings $env:AITERM_CLAUDE_SETTINGS @args
        }
    }
}
```

- [ ] **Step 4: 跑到綠**

Run: `cd src-tauri && cargo test -p aiterm-core claude_function powershell_script 2>&1 | grep -E "^test |test result|panicked"`
Expected: 全部 `ok`。再跑 `cargo test -p aiterm-core shell 2>&1 | grep "test result"` 確認原有 shell 測試（OSC 133 腳本內容、UTF-16 編碼等）沒壞。

- [ ] **Step 5: 證明測試有咬到**

暫時把 bash 函式裡 `command claude --settings "$AITERM_CLAUDE_SETTINGS" "$@"` 改成 `command claude "$@"`，Expected: `claude_function_adds_settings…` FAIL。還原。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/pty/shell.rs
git commit -m "feat(shell-integration): claude function injects the activity settings

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: 前端 IPC 與純函式

**Files:**
- Create: `src/ipc/claudeActivity.ts`
- Create: `src/components/ClaudeActivity/flattenTree.ts` + `flattenTree.test.ts`
- Create: `src/components/ClaudeActivity/format.ts` + `format.test.ts`

- [ ] **Step 1: IPC 模組**

`src/ipc/claudeActivity.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ActivityStatus = "running" | "done" | "failed" | "interrupted";

/** 對應 Rust `claude_activity::tree::Node`。 */
export interface ActivityNode {
  id: string;
  parent: string | null;
  agent_type: string;
  description: string | null;
  status: ActivityStatus;
  started_ms: number;
  ended_ms: number | null;
  current_tool: string | null;
  summary: string | null;
  tokens: number | null;
  tool_count: number | null;
}

export const claudeActivityEvent = (sessionId: string): string =>
  `claude-activity://${sessionId}`;

export function getClaudeActivity(sessionId: string): Promise<ActivityNode[]> {
  return invoke<ActivityNode[]>("claude_activity_snapshot", { sessionId });
}

/** 事件酬載永遠是完整快照，所以收到就整個取代。 */
export function listenClaudeActivity(
  sessionId: string,
  onSnapshot: (nodes: ActivityNode[]) => void,
): Promise<UnlistenFn> {
  return listen<ActivityNode[]>(claudeActivityEvent(sessionId), (e) => onSnapshot(e.payload));
}
```

- [ ] **Step 2: `flattenTree` 的測試（紅）**

`src/components/ClaudeActivity/flattenTree.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import type { ActivityNode } from "../../ipc/claudeActivity";
import { flattenTree } from "./flattenTree";

const n = (id: string, parent: string | null, started = 0): ActivityNode => ({
  id, parent, agent_type: "general-purpose", description: null, status: "running",
  started_ms: started, ended_ms: null, current_tool: null, summary: null, tokens: null, tool_count: null,
});

describe("flattenTree", () => {
  it("把子節點排在父節點底下並帶縮排深度", () => {
    const rows = flattenTree([n("a", null, 1), n("a1", "a", 2), n("b", null, 3), n("a1x", "a1", 4)]);
    expect(rows.map((r) => [r.node.id, r.depth])).toEqual([["a", 0], ["a1", 1], ["a1x", 2], ["b", 0]]);
  });

  it("父節點不存在（被裁掉或事件遺失）就當根節點，不會消失", () => {
    const rows = flattenTree([n("orphan", "gone", 1)]);
    expect(rows.map((r) => [r.node.id, r.depth])).toEqual([["orphan", 0]]);
  });

  it("循環參照不會無限迴圈，每個節點只出現一次", () => {
    const rows = flattenTree([n("x", "y", 1), n("y", "x", 2)]);
    expect(rows.map((r) => r.node.id).sort()).toEqual(["x", "y"]);
  });

  it("空陣列回空陣列", () => {
    expect(flattenTree([])).toEqual([]);
  });
});
```

Run: `npx vitest run src/components/ClaudeActivity/flattenTree.test.ts`
Expected: FAIL（找不到 `./flattenTree`）。

- [ ] **Step 3: 實作 `flattenTree.ts`**

```ts
import type { ActivityNode } from "../../ipc/claudeActivity";

export interface ActivityRow {
  node: ActivityNode;
  depth: number;
}

/** 依開始順序做深度優先展開；孤兒（父節點不在清單內）與循環中的節點當根，保證每個節點恰好出現一次。 */
export function flattenTree(nodes: ActivityNode[]): ActivityRow[] {
  const ids = new Set(nodes.map((n) => n.id));
  const children = new Map<string, ActivityNode[]>();
  const roots: ActivityNode[] = [];
  for (const n of nodes) {
    if (n.parent && ids.has(n.parent) && n.parent !== n.id) {
      const list = children.get(n.parent) ?? [];
      list.push(n);
      children.set(n.parent, list);
    } else {
      roots.push(n);
    }
  }
  const rows: ActivityRow[] = [];
  const seen = new Set<string>();
  const visit = (n: ActivityNode, depth: number) => {
    if (seen.has(n.id)) return;
    seen.add(n.id);
    rows.push({ node: n, depth });
    for (const c of children.get(n.id) ?? []) visit(c, depth + 1);
  };
  for (const r of roots) visit(r, 0);
  // 循環中的節點沒有任何根可達：補上，避免消失。
  for (const n of nodes) visit(n, 0);
  return rows;
}
```

Run 同一條測試，Expected: 4 passed。

- [ ] **Step 4: `format` 測試（紅）→ 實作**

`src/components/ClaudeActivity/format.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { formatDuration, formatTokens } from "./format";

describe("formatDuration", () => {
  it.each([
    [0, "0s"],
    [4958, "4.9s"],
    [59_000, "59s"],
    [65_000, "1m 5s"],
    [3_725_000, "1h 2m"],
  ])("%i ms → %s", (ms, text) => expect(formatDuration(ms)).toBe(text));

  it("負數（時鐘偏差）當 0", () => expect(formatDuration(-5)).toBe("0s"));
});

describe("formatTokens", () => {
  it.each([
    [0, "0"],
    [999, "999"],
    [1000, "1.0k"],
    [20484, "20.5k"],
    [1_250_000, "1.3M"],
  ])("%i → %s", (n, text) => expect(formatTokens(n)).toBe(text));
});
```

Run: FAIL。實作 `format.ts`：

```ts
/** 4.9s／59s／1m 5s／1h 2m。 */
export function formatDuration(ms: number): string {
  const total = Math.max(0, ms);
  if (total < 10_000) {
    const tenths = Math.floor(total / 100);
    return tenths % 10 === 0 ? `${tenths / 10}s` : `${(tenths / 10).toFixed(1)}s`;
  }
  const s = Math.floor(total / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** 999／1.0k／20.5k／1.3M。 */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}
```

Run 測試，Expected: 全綠。

- [ ] **Step 5: Commit**

```bash
git add src/ipc/claudeActivity.ts src/components/ClaudeActivity
git commit -m "feat(claude-activity): frontend ipc, tree flattening and formatters

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: `useClaudeActivity` hook

**Files:**
- Create: `src/components/ClaudeActivity/useClaudeActivity.ts` + `useClaudeActivity.test.ts`

- [ ] **Step 1: 測試（紅）**

```ts
import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ActivityNode } from "../../ipc/claudeActivity";

const getClaudeActivity = vi.fn();
const handlers = new Map<string, (nodes: ActivityNode[]) => void>();
const unlisten = vi.fn();
vi.mock("../../ipc/claudeActivity", () => ({
  getClaudeActivity: (...a: unknown[]) => getClaudeActivity(...a),
  listenClaudeActivity: (sid: string, cb: (n: ActivityNode[]) => void) => {
    handlers.set(sid, cb);
    return Promise.resolve(unlisten);
  },
}));

import { useClaudeActivity } from "./useClaudeActivity";

const node = (id: string): ActivityNode => ({
  id, parent: null, agent_type: "general-purpose", description: id, status: "running",
  started_ms: 1, ended_ms: null, current_tool: null, summary: null, tokens: null, tool_count: null,
});

beforeEach(() => {
  getClaudeActivity.mockReset();
  handlers.clear();
  unlisten.mockClear();
});

describe("useClaudeActivity", () => {
  it("掛載時取快照；之後事件整份取代", async () => {
    getClaudeActivity.mockResolvedValue([node("a")]);
    const { result } = renderHook(() => useClaudeActivity("s1"));
    await waitFor(() => expect(result.current.map((n) => n.id)).toEqual(["a"]));
    act(() => handlers.get("s1")!([node("a"), node("b")]));
    expect(result.current.map((n) => n.id)).toEqual(["a", "b"]);
  });

  it("事件比快照先到：較舊的快照不能蓋掉較新的事件", async () => {
    let resolveSnap!: (n: ActivityNode[]) => void;
    getClaudeActivity.mockReturnValue(new Promise((r) => { resolveSnap = r; }));
    const { result } = renderHook(() => useClaudeActivity("s1"));
    await waitFor(() => expect(handlers.has("s1")).toBe(true));
    act(() => handlers.get("s1")!([node("new")]));
    await act(async () => resolveSnap([node("old")]));
    expect(result.current.map((n) => n.id)).toEqual(["new"]);
  });

  it("沒有 sessionId 時什麼都不訂閱", () => {
    renderHook(() => useClaudeActivity(""));
    expect(getClaudeActivity).not.toHaveBeenCalled();
    expect(handlers.size).toBe(0);
  });

  it("卸載後退訂，且晚到的事件不會 setState", async () => {
    getClaudeActivity.mockResolvedValue([]);
    const { unmount } = renderHook(() => useClaudeActivity("s1"));
    await waitFor(() => expect(handlers.has("s1")).toBe(true));
    unmount();
    await waitFor(() => expect(unlisten).toHaveBeenCalled());
    expect(() => handlers.get("s1")!([node("z")])).not.toThrow();
  });

  it("快照失敗不炸（沒有側欄而已）", async () => {
    getClaudeActivity.mockRejectedValue(new Error("boom"));
    const { result } = renderHook(() => useClaudeActivity("s1"));
    await waitFor(() => expect(getClaudeActivity).toHaveBeenCalled());
    expect(result.current).toEqual([]);
  });
});
```

Run: `npx vitest run src/components/ClaudeActivity/useClaudeActivity.test.ts` → FAIL。

- [ ] **Step 2: 實作**

```ts
import { useEffect, useState } from "react";
import {
  getClaudeActivity,
  listenClaudeActivity,
  type ActivityNode,
} from "../../ipc/claudeActivity";

/**
 * 這個分頁 PTY 裡 Claude 的 subagent 清單。
 *
 * 先訂閱、再取快照：事件在兩者之間送出也不會漏。若事件已經先到，較舊的快照就不採用
 * （事件永遠是完整快照，比一次性的 invoke 結果新）。`alive` 防卸載後 setState
 * （Tauri listen 是非同步的，跟 useAiChat 的 mountedRef 同一個坑）。
 */
export function useClaudeActivity(sessionId: string): ActivityNode[] {
  const [nodes, setNodes] = useState<ActivityNode[]>([]);

  useEffect(() => {
    if (!sessionId) return;
    let alive = true;
    let gotEvent = false;
    let unlisten: (() => void) | undefined;
    setNodes([]);

    listenClaudeActivity(sessionId, (next) => {
      if (!alive) return;
      gotEvent = true;
      setNodes(next);
    })
      .then((u) => {
        if (alive) unlisten = u;
        else u();
      })
      .catch(() => {});

    getClaudeActivity(sessionId)
      .then((snap) => {
        if (alive && !gotEvent) setNodes(snap);
      })
      .catch(() => {});

    return () => {
      alive = false;
      unlisten?.();
    };
  }, [sessionId]);

  return nodes;
}
```

注意：`unlisten` 在 listen promise 比卸載晚 resolve 時由 `.then` 內 `else u()` 處理；測試「卸載後退訂」靠的是 effect cleanup 或 `.then` 其中之一呼叫到。

- [ ] **Step 3: 跑到綠並做突變**

Run 測試，Expected: 5 passed。突變：把 `if (alive && !gotEvent) setNodes(snap);` 的 `&& !gotEvent` 拿掉，Expected: 「事件比快照先到」FAIL。還原。

- [ ] **Step 4: Commit**

```bash
git add src/components/ClaudeActivity
git commit -m "feat(claude-activity): useClaudeActivity subscribe-then-snapshot hook

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 8: i18n 與側欄面板

**Files:**
- Modify: `src/lib/i18n.ts`
- Create: `src/components/ClaudeActivity/ClaudeActivityPanel.tsx`, `.css`, `ClaudeActivityPanel.test.tsx`

- [ ] **Step 1: i18n 字串**

在 `src/lib/i18n.ts` 的 `zhTW`（第 9 行起的物件）內、`term_close_cancel` 那一組附近加入：

```ts
    claude_activity_title: "Claude 協作",
    claude_activity_running: (n: number) => `執行中 ${n}`,
    claude_activity_total: (n: number) => `共 ${n} 個`,
    claude_activity_collapse: "收合",
    claude_activity_expand: "展開",
    claude_activity_untitled: "（未命名任務）",
    claude_activity_status_running: "執行中",
    claude_activity_status_done: "完成",
    claude_activity_status_failed: "失敗",
    claude_activity_status_interrupted: "已中斷",
    claude_activity_tokens: (s: string) => `${s} tokens`,
    claude_activity_tools: (n: number) => `${n} 次工具呼叫`,
    claude_activity_show_result: "查看結果",
    claude_activity_hide_result: "收起結果",
```

在 `enRaw`（約第 2381 行起）對應位置加入：

```ts
    claude_activity_title: "Claude activity",
    claude_activity_running: (n: number) => `${n} running`,
    claude_activity_total: (n: number) => `${n} total`,
    claude_activity_collapse: "Collapse",
    claude_activity_expand: "Expand",
    claude_activity_untitled: "(untitled task)",
    claude_activity_status_running: "Running",
    claude_activity_status_done: "Done",
    claude_activity_status_failed: "Failed",
    claude_activity_status_interrupted: "Interrupted",
    claude_activity_tokens: (s: string) => `${s} tokens`,
    claude_activity_tools: (n: number) => `${n} tool calls`,
    claude_activity_show_result: "Show result",
    claude_activity_hide_result: "Hide result",
```

Run: `npx vitest run src/lib/i18n.test.ts` Expected: pass（key 對齊測試，比對的是 `localeSources`）。

- [ ] **Step 2: 面板測試（紅）**

`src/components/ClaudeActivity/ClaudeActivityPanel.test.tsx`：

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import type { ActivityNode } from "../../ipc/claudeActivity";

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});

import { ClaudeActivityPanel } from "./ClaudeActivityPanel";

const node = (over: Partial<ActivityNode> & { id: string }): ActivityNode => ({
  parent: null, agent_type: "general-purpose", description: null, status: "running",
  started_ms: 1_000, ended_ms: null, current_tool: null, summary: null, tokens: null, tool_count: null,
  ...over,
});

beforeEach(() => localStorage.clear());

describe("ClaudeActivityPanel", () => {
  it("標題顯示執行中數量與總數", () => {
    render(<ClaudeActivityPanel nodes={[node({ id: "a" }), node({ id: "b", status: "done", ended_ms: 2_000 })]} fill />);
    expect(screen.getByText("Claude 協作")).toBeInTheDocument();
    expect(screen.getByText("執行中 1")).toBeInTheDocument();
    expect(screen.getByText("共 2 個")).toBeInTheDocument();
  });

  it("每列顯示任務描述、型別、狀態；沒有描述用佔位文字", () => {
    render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" }), node({ id: "b" })]} fill />);
    expect(screen.getByText("跑 ls")).toBeInTheDocument();
    expect(screen.getByText("（未命名任務）")).toBeInTheDocument();
    expect(screen.getAllByText("general-purpose")).toHaveLength(2);
  });

  it("執行中的節點顯示目前工具；結束後顯示耗時、token 與工具次數", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[
          node({ id: "r", description: "跑中", current_tool: "Bash: ls -la" }),
          node({ id: "d", description: "做完", status: "done", started_ms: 1_000, ended_ms: 5_900, tokens: 20484, tool_count: 2 }),
        ]}
      />,
    );
    expect(screen.getByText("Bash: ls -la")).toBeInTheDocument();
    expect(screen.getByText("4.9s")).toBeInTheDocument();
    expect(screen.getByText("20.5k tokens")).toBeInTheDocument();
    expect(screen.getByText("2 次工具呼叫")).toBeInTheDocument();
  });

  it("子節點縮排；結果摘要預設收起，按鈕展開", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[
          node({ id: "p", description: "外層" }),
          node({ id: "c", parent: "p", description: "內層", status: "done", ended_ms: 2_000, summary: "檔名是 events.jsonl" }),
        ]}
      />,
    );
    const inner = screen.getByText("內層").closest("[data-depth]") as HTMLElement;
    expect(inner.getAttribute("data-depth")).toBe("1");
    expect(screen.queryByText("檔名是 events.jsonl")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "查看結果" }));
    expect(screen.getByText("檔名是 events.jsonl")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "收起結果" }));
    expect(screen.queryByText("檔名是 events.jsonl")).toBeNull();
  });

  it("收合只留標題，並記在 localStorage", () => {
    const { unmount } = render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" })]} fill />);
    fireEvent.click(screen.getByRole("button", { name: "收合" }));
    expect(screen.queryByText("跑 ls")).toBeNull();
    expect(screen.getByRole("button", { name: "展開" })).toBeInTheDocument();
    unmount();
    render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" })]} fill />);
    expect(screen.queryByText("跑 ls")).toBeNull();
  });

  it("狀態用文字標示（不只靠顏色）", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[
          node({ id: "f", description: "x", status: "failed", ended_ms: 2_000 }),
          node({ id: "i", description: "y", status: "interrupted", ended_ms: 2_000 }),
        ]}
      />,
    );
    expect(screen.getByText("失敗")).toBeInTheDocument();
    expect(screen.getByText("已中斷")).toBeInTheDocument();
  });
});
```

Run: FAIL（找不到元件）。

- [ ] **Step 3: 實作面板**

`ClaudeActivityPanel.tsx`：

```tsx
import { useEffect, useMemo, useState } from "react";
import type { ActivityNode, ActivityStatus } from "../../ipc/claudeActivity";
import { useLocale } from "../../contexts/LocaleContext";
import { flattenTree } from "./flattenTree";
import { formatDuration, formatTokens } from "./format";
import "./ClaudeActivityPanel.css";

const COLLAPSED_KEY = "aiterm-claude-activity-collapsed";

function loadCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

/** 有執行中的節點時每秒更新一次，讓耗時跳動；全部結束就停。 */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active]);
  return now;
}

interface Props {
  nodes: ActivityNode[];
  /** true＝獨佔側欄整欄；false＝與建議側欄上下並排時只佔上半部（有上限、可捲動）。 */
  fill: boolean;
}

export function ClaudeActivityPanel({ nodes, fill }: Props) {
  const { t } = useLocale();
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const [openResults, setOpenResults] = useState<Set<string>>(() => new Set());
  const rows = useMemo(() => flattenTree(nodes), [nodes]);
  const running = nodes.filter((n) => n.status === "running").length;
  const now = useNow(running > 0);

  const toggleCollapsed = () => {
    const next = !collapsed;
    setCollapsed(next);
    try { localStorage.setItem(COLLAPSED_KEY, next ? "1" : "0"); } catch { /* ignore */ }
  };
  const toggleResult = (id: string) =>
    setOpenResults((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const statusLabel: Record<ActivityStatus, string> = {
    running: t.claude_activity_status_running,
    done: t.claude_activity_status_done,
    failed: t.claude_activity_status_failed,
    interrupted: t.claude_activity_status_interrupted,
  };

  return (
    <section
      className={`aiterm-cact${fill ? " aiterm-cact--fill" : ""}${collapsed ? " aiterm-cact--collapsed" : ""}`}
      aria-label={t.claude_activity_title}
    >
      <header className="aiterm-cact__header">
        <span className="aiterm-cact__title">{t.claude_activity_title}</span>
        <span className="aiterm-cact__count">{t.claude_activity_running(running)}</span>
        <span className="aiterm-cact__count">{t.claude_activity_total(nodes.length)}</span>
        <button
          type="button"
          className="aiterm-cact__toggle"
          aria-expanded={!collapsed}
          onClick={toggleCollapsed}
        >
          {collapsed ? t.claude_activity_expand : t.claude_activity_collapse}
        </button>
      </header>
      {!collapsed && (
        <ul className="aiterm-cact__list">
          {rows.map(({ node, depth }) => {
            const elapsed = (node.ended_ms ?? now) - node.started_ms;
            const open = openResults.has(node.id);
            return (
              <li
                key={node.id}
                className={`aiterm-cact__row aiterm-cact__row--${node.status}`}
                data-depth={depth}
                style={{ paddingLeft: `${8 + depth * 14}px` }}
              >
                <div className="aiterm-cact__line">
                  <span className={`aiterm-cact__dot aiterm-cact__dot--${node.status}`} aria-hidden="true" />
                  <span className="aiterm-cact__desc">{node.description ?? t.claude_activity_untitled}</span>
                  <span className="aiterm-cact__badge">{node.agent_type}</span>
                  <span className="aiterm-cact__status">{statusLabel[node.status]}</span>
                </div>
                {node.status === "running" && node.current_tool && (
                  <div className="aiterm-cact__tool">{node.current_tool}</div>
                )}
                <div className="aiterm-cact__meta">
                  <span>{formatDuration(elapsed)}</span>
                  {node.tokens != null && <span>{t.claude_activity_tokens(formatTokens(node.tokens))}</span>}
                  {node.tool_count != null && <span>{t.claude_activity_tools(node.tool_count)}</span>}
                </div>
                {node.summary && (
                  <>
                    <button type="button" className="aiterm-cact__result-btn" onClick={() => toggleResult(node.id)}>
                      {open ? t.claude_activity_hide_result : t.claude_activity_show_result}
                    </button>
                    {open && <p className="aiterm-cact__summary">{node.summary}</p>}
                  </>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
```

`ClaudeActivityPanel.css`（沿用 repo 變數；顏色同時有文字標示，不只靠顏色）：

```css
.aiterm-cact {
  display: flex;
  flex-direction: column;
  min-height: 0;
  font-size: 12px;
  color: var(--text-primary, #e6e6e6);
  border-bottom: 1px solid var(--border-color, #2a2a2a);
  flex: 0 1 auto;
  max-height: 45%;
}
.aiterm-cact--fill { flex: 1 1 0%; max-height: none; border-bottom: none; }
.aiterm-cact--collapsed { max-height: none; flex: 0 0 auto; }
.aiterm-cact__header {
  display: flex; align-items: center; gap: 8px; padding: 8px;
  border-bottom: 1px solid var(--border-color, #2a2a2a);
}
.aiterm-cact__title { font-weight: 600; }
.aiterm-cact__count { color: var(--text-secondary, #9a9a9a); }
.aiterm-cact__toggle {
  margin-left: auto; background: none; border: 1px solid var(--border-color, #2a2a2a);
  color: inherit; border-radius: 4px; padding: 2px 8px; cursor: pointer; font-size: 11px;
}
.aiterm-cact__list { list-style: none; margin: 0; padding: 0; overflow-y: auto; min-height: 0; }
.aiterm-cact__row { padding: 6px 8px; border-bottom: 1px solid var(--border-color, #2a2a2a); }
.aiterm-cact__line { display: flex; align-items: center; gap: 6px; min-width: 0; }
.aiterm-cact__dot { width: 8px; height: 8px; border-radius: 50%; flex: 0 0 auto; background: var(--text-secondary, #9a9a9a); }
.aiterm-cact__dot--running { background: #60a5fa; animation: aiterm-cact-pulse 1.2s ease-in-out infinite; }
.aiterm-cact__dot--done { background: #4ade80; }
.aiterm-cact__dot--failed { background: #f87171; }
.aiterm-cact__dot--interrupted { background: #fbbf24; }
.aiterm-cact__desc { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.aiterm-cact__badge {
  flex: 0 0 auto; padding: 0 6px; border-radius: 8px; font-size: 10px;
  background: var(--bg-glass, rgba(255,255,255,0.08)); color: var(--text-secondary, #9a9a9a);
}
.aiterm-cact__status { flex: 0 0 auto; font-size: 11px; color: var(--text-secondary, #9a9a9a); }
.aiterm-cact__tool {
  margin: 2px 0 0 14px; font-family: var(--font-mono, monospace); font-size: 11px;
  color: #93c5fd; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.aiterm-cact__meta { display: flex; gap: 10px; margin: 2px 0 0 14px; color: var(--text-secondary, #9a9a9a); font-size: 11px; }
.aiterm-cact__result-btn {
  margin: 2px 0 0 14px; background: none; border: none; padding: 0;
  color: var(--accent, #c084fc); cursor: pointer; font-size: 11px; text-decoration: underline;
}
.aiterm-cact__summary { margin: 4px 0 0 14px; white-space: pre-wrap; word-break: break-word; color: var(--text-secondary, #bdbdbd); }
@keyframes aiterm-cact-pulse { 50% { opacity: 0.35; } }
@media (prefers-reduced-motion: reduce) { .aiterm-cact__dot--running { animation: none; } }
```

- [ ] **Step 4: 跑到綠並做突變**

Run: `npx vitest run src/components/ClaudeActivity src/lib/i18n.test.ts`
Expected: 全綠。突變：把面板裡 `if (!collapsed && …)` 的收合判斷拿掉（`!collapsed &&` 刪掉），Expected: 「收合只留標題」FAIL。還原。

- [ ] **Step 5: Commit**

```bash
git add src/components/ClaudeActivity src/lib/i18n.ts
git commit -m "feat(claude-activity): sidebar panel and strings

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 9: 掛進 TerminalView

**Files:**
- Modify: `src/components/TerminalView.tsx`
- Create: `src/components/TerminalView.claudeActivity.test.tsx`

- [ ] **Step 1: 整合測試（紅）**

沿用 `TerminalView.suggestionSidebar.test.tsx` 的 mock 骨架（那份檔案開頭到 `mount` 的 1–70 行原樣複製，`invoke` mock 另外讓 `claude_activity_snapshot` 回 `[]`）：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "pty_create") return Promise.resolve("test-session");
    if (cmd === "claude_activity_snapshot") return Promise.resolve([]);
    return new Promise(() => {});
  }),
}));

const listenHandlers = new Map<string, ((event: { payload: unknown }) => void)[]>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (event: { payload: unknown }) => void) => {
    const arr = listenHandlers.get(event) ?? [];
    arr.push(handler);
    listenHandlers.set(event, arr);
    return Promise.resolve(() => {
      const idx = arr.indexOf(handler);
      if (idx >= 0) arr.splice(idx, 1);
    });
  }),
}));

vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn(() => Promise.resolve("/home/test")) }));

Element.prototype.scrollTo = Element.prototype.scrollTo || (() => {});
if (!window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false, media: query, onchange: null,
    addListener: () => {}, removeListener: () => {},
    addEventListener: () => {}, removeEventListener: () => {}, dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}
class FakeResizeObserver { observe() {} unobserve() {} disconnect() {} }
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;

vi.mock("../hooks/useAgentMission", () => ({
  useAgentMission: () => ({ agentMission: null, startMission: vi.fn(), stopMission: vi.fn(), addTokens: vi.fn() }),
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";

beforeEach(() => {
  listenHandlers.clear();
  localStorage.clear();
});

const mount = () =>
  render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
      </MemoryRouter>
    </LocaleProvider>,
  );

const emit = (payload: unknown) =>
  act(async () => {
    for (const h of listenHandlers.get("claude-activity://test-session") ?? []) h({ payload });
  });

const node = (id: string, description: string) => ({
  id, parent: null, agent_type: "general-purpose", description, status: "running",
  started_ms: 1, ended_ms: null, current_tool: "Bash: ls", summary: null, tokens: null, tool_count: null,
});

describe("終端機分頁的 Claude 協作側欄", () => {
  it("沒有任何 subagent 事件時不出現側欄（版面與以前完全一樣）", async () => {
    const { container } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    expect(container.querySelector(".aiterm-cact")).toBeNull();
    expect(container.querySelector(".aiterm-sugg-split__side")).toBeNull();
  });

  it("收到事件後，側欄在終端機旁（兄弟欄）顯示 subagent", async () => {
    const { container, getByText } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    await emit([node("a1", "整理檔案")]);
    await waitFor(() => expect(getByText("整理檔案")).toBeInTheDocument());
    const split = container.querySelector(".aiterm-sugg-split") as HTMLElement;
    const side = split.querySelector(".aiterm-sugg-split__side") as HTMLElement;
    const main = split.querySelector(".aiterm-sugg-split__main") as HTMLElement;
    expect(side.querySelector(".aiterm-cact")).not.toBeNull();
    expect(main.contains(side)).toBe(false);
    expect(side.querySelector(".aiterm-cact--fill")).not.toBeNull();
  });

  it("與建議側欄同時開時上下並排，面板不再獨佔整欄", async () => {
    const { container } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    await emit([node("a1", "整理檔案")]);
    const toggle = () => container.querySelector(".aiterm-sugg-toggle") as HTMLButtonElement;
    await waitFor(() => expect(toggle()).not.toBeNull());
    await act(async () => { toggle().click(); });
    await waitFor(() => expect(container.querySelector(".aiterm-sugg-sidebar")).not.toBeNull());
    const side = container.querySelector(".aiterm-sugg-split__side") as HTMLElement;
    expect(side.querySelector(".aiterm-cact")).not.toBeNull();
    expect(side.querySelector(".aiterm-cact--fill")).toBeNull();
    expect(side.querySelector(".aiterm-sugg-sidebar")).not.toBeNull();
  });
});
```

Run: `npx vitest run src/components/TerminalView.claudeActivity.test.tsx` → FAIL。

- [ ] **Step 2: 實作**

`TerminalView.tsx` 頂端 import 區（`import { SuggestionSplit } …` 之後）加：

```tsx
import { ClaudeActivityPanel } from "./ClaudeActivity/ClaudeActivityPanel";
import { useClaudeActivity } from "./ClaudeActivity/useClaudeActivity";
```

在 `const suggestionSidebar = useSuggestionSidebar(blocks, customAiCliNames);`（約第 501 行）之後加：

```tsx
  const claudeActivity = useClaudeActivity(sessionId);
```

把 `return ( <SuggestionSplit sidebar={suggestionSidebar.open && sessionId ? ( <SuggestionSidebar … /> ) : null} >` 這段改成（先 `Read` 該處取得精確縮排與全部 props；只改 `sidebar=` 的值，`SuggestionSidebar` 的 props 一字不動）：

```tsx
  const suggestionsShown = suggestionSidebar.open && !!sessionId;
  const sidebarContent =
    claudeActivity.length > 0 || suggestionsShown ? (
      <>
        {claudeActivity.length > 0 && (
          <ClaudeActivityPanel nodes={claudeActivity} fill={!suggestionsShown} />
        )}
        {suggestionsShown && (
          <div style={{ flex: "1 1 0%", minHeight: 0, display: "flex", flexDirection: "column" }}>
            <SuggestionSidebar
              {/* …原本的 props 原樣保留… */}
            />
          </div>
        )}
      </>
    ) : null;

  return (
    <SuggestionSplit sidebar={sidebarContent}>
```

（`suggestionsShown`／`sidebarContent` 兩個常數放在 `handleCancel` 之後、`return (` 之前。）

- [ ] **Step 3: 跑到綠**

Run: `npx vitest run src/components/TerminalView.claudeActivity.test.tsx src/components/TerminalView.suggestionSidebar.test.tsx`
Expected: 兩份全綠（舊的建議側欄測試必須不受影響——尤其「side 是 split 的直接子元素」「手動開啟」）。

- [ ] **Step 4: 突變**

把 `fill={!suggestionsShown}` 改成 `fill`，Expected: 「上下並排」測試 FAIL。還原。

- [ ] **Step 5: 全套驗證**

Run:
```bash
npx tsc -b && npm run lint && npm run test 2>&1 | tail -15
```
Expected: tsc 無輸出；lint 無錯誤；vitest 全綠（`MailView` flaky 測試只在記憶體吃緊時紅，見記憶；失敗時先單獨重跑確認與本次無關再記錄）。

- [ ] **Step 6: Commit**

```bash
git add src/components/TerminalView.tsx src/components/TerminalView.claudeActivity.test.tsx
git commit -m "feat(claude-activity): show the sidebar next to the terminal

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 10: 實機驗收（假事件 → 真 claude）

**Files:** 無（驗收；發現的 bug 另開 commit 修）。

- [ ] **Step 1: 全套 Rust**

```bash
cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result|FAILED|failed"
```
Expected: 每個 `test result:` 皆 `ok`（逐行看，不是只看最後一行）。

- [ ] **Step 2: 啟動 dev 版並確認跑的是新二進位**

```bash
npm run tauri:dev
```
（依記憶 `feedback_verify_running_binary`：乾淨重起，確認沒有舊 watcher 殘留；若 `APPLE_SIGNING_IDENTITY` 干擾用 `env -u APPLE_SIGNING_IDENTITY`。）

- [ ] **Step 3: 假事件驗收**

在 AITerm 開一個終端機分頁，從該分頁的 shell 取 `echo $AITERM_CLAUDE_SETTINGS`（Expected: 一個存在的 `.json` 路徑）、`type claude`（Expected: 顯示是 shell function）。讀設定檔取得 `sub-start` 的 URL，用 `curl` 依序送：pre(Agent) → sub-start → pre(Bash) → sub-stop → post(Agent)（酬載照 Task 2 測試的 JSON）。Expected: 側欄出現、顯示描述／目前工具／完成後的耗時與 token；再送錯誤密鑰的 URL，Expected: 側欄無變化。

- [ ] **Step 4: 真 claude 驗收**

在同一分頁輸入 `claude`，請它 `用 Agent 工具開兩個平行 subagent，各自用 Bash 跑 ls，再各自回報`。Expected: 側欄即時出現兩列、各自顯示目前工具、結束後顯示 token 與結果摘要；Claude 結束（`/exit`）後沒有轉圈的節點；補驗 spec 中標記「尚未實測」的失敗／背景 agent 狀態值，若實際字串與計畫的 `failed`／`error` 不同，回頭修 `tree.rs` 並補測試。

- [ ] **Step 5: 回歸**

在同一分頁確認：派工分頁（工作看板）的 claude 仍正常（函式遇到 `--settings` 原樣轉交）；`Ctrl+L`、輸入、貼上無異常；關閉分頁後 `ls "$HOME/Library/Application Support/AITERM/claude-activity"` 該 session 的 json 已刪。

- [ ] **Step 6: 收尾提交**

```bash
git status --short   # 只 add 明確路徑，不要 git add -A
```
驗收發現的修正各自 commit。**不打 tag、不 push**（需使用者同意；打 tag 前要先寫 CHANGELOG，且只寫實際看過的畫面行為）。
