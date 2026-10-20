//! subagent 樹的純狀態機：只吃 hook 事件，不碰網路與 Tauri，所以能單元測試。
//!
//! 事件形狀來自真 claude（2.1.300）的探勘，見計畫書開頭。

use std::collections::VecDeque;

use serde::Serialize;
use serde_json::Value;

/// 一個 session 最多留幾個節點；超過就丟最舊的「已結束」節點。
pub const MAX_NODES: usize = 200;
const SUMMARY_MAX_CHARS: usize = 400;
const HINT_MAX_CHARS: usize = 80;
pub const MAIN_RECENT_MAX: usize = 10;

static NULL: Value = Value::Null;
/// 建立節點時還不知道型別的暫用名稱，之後 SubagentStart 帶著真型別來會被換掉。
const FALLBACK_TYPE: &str = "agent";

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
    /// 這個 agent 最近一次有任何事件的時間；前端用它判斷「卡住」。
    pub last_event_ms: u64,
    /// 建立這個節點時的提問輪次（見 `Tree::turn`），前端用它分組。
    pub turn: u32,
}

/// 送給前端的完整狀態；事件與快照指令共用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Snapshot {
    pub nodes: Vec<Node>,
    /// 主 Claude 現在在用的工具（沒有 subagent 時側欄不顯示，所以只是附帶資訊）。
    pub main_tool: Option<String>,
    /// 本輪主 Claude 的工具呼叫次數（含呼叫 Agent 本身）。
    pub main_tool_count: u64,
    /// 本輪主 Claude 最近的工具提示，舊的在前，最多 `MAIN_RECENT_MAX` 筆。
    pub main_recent: Vec<String>,
    /// 目前的提問輪次；0＝還沒有過提問。
    pub turn: u32,
}

/// 事件種類；字串就是 settings 檔裡 hook 網址的最後一段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pre,
    Post,
    SubStart,
    SubStop,
    SessionEnd,
    /// `UserPromptSubmit`：使用者送出新的提問。
    Prompt,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "pre" => Some(Kind::Pre),
            "post" => Some(Kind::Post),
            "sub-start" => Some(Kind::SubStart),
            "sub-stop" => Some(Kind::SubStop),
            "session-end" => Some(Kind::SessionEnd),
            "prompt" => Some(Kind::Prompt),
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
    main_tool: Option<String>,
    main_tool_count: u64,
    main_recent: VecDeque<String>,
    turn: u32,
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or(&NULL)
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_owned)
}

/// 事件裡的 agent_type；空字串當成沒有。
fn agent_type_of(body: &Value) -> Option<String> {
    str_of(body, "agent_type").filter(|t| !t.is_empty())
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

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            nodes: self.nodes.clone(),
            main_tool: self.main_tool.clone(),
            main_tool_count: self.main_tool_count,
            main_recent: self.main_recent.iter().cloned().collect(),
            turn: self.turn,
        }
    }

    /// 判斷「有沒有變」用的快照：不含 `last_event_ms`，重複事件只刷新時間戳不該再通知前端。
    fn visible_snapshot(&self) -> Snapshot {
        let mut s = self.snapshot();
        for n in &mut s.nodes {
            n.last_event_ms = 0;
        }
        s
    }

    /// 套用一個 hook 事件；回傳節點有沒有變化（沒變就不必通知前端）。
    pub fn apply(&mut self, kind: Kind, body: &Value, now_ms: u64) -> bool {
        let before = self.visible_snapshot();
        let tool = str_of(body, "tool_name");
        let caller = str_of(body, "agent_id");
        let is_agent_tool = matches!(tool.as_deref(), Some("Agent") | Some("Task"));
        match kind {
            Kind::Pre if is_agent_tool => {
                if caller.is_none() {
                    let desc = str_of(field(body, "tool_input"), "description").unwrap_or_default();
                    let hint = truncate(&format!("Agent: {desc}"), HINT_MAX_CHARS);
                    self.main_tool = Some(hint.clone());
                    self.note_main_tool(hint);
                }
                self.queue_spawn(body, caller)
            }
            Kind::Pre => match (tool.as_deref(), caller.as_deref()) {
                (Some(tool), Some(agent)) => self.on_tool_pre(tool, agent, body, now_ms),
                (Some(tool), None) => {
                    let hint = tool_hint(tool, field(body, "tool_input"));
                    self.main_tool = Some(hint.clone());
                    self.note_main_tool(hint);
                }
                _ => {}
            },
            Kind::Post if is_agent_tool => {
                if caller.is_none() {
                    self.main_tool = None;
                }
                self.on_agent_post(body, caller, now_ms)
            }
            Kind::Post => match caller.as_deref() {
                Some(a) => {
                    if let Some(i) = self.index_of(a) {
                        self.nodes[i].current_tool = None;
                        self.nodes[i].last_event_ms = now_ms;
                    }
                }
                None => self.main_tool = None,
            },
            Kind::SubStart => {
                if let Some(id) = str_of(body, "agent_id") {
                    self.on_start(&id, agent_type_of(body), now_ms);
                }
            }
            Kind::SubStop => {
                if let Some(id) = str_of(body, "agent_id") {
                    // 互動模式下 Claude 會為內部輔助工作送出沒有型別、也沒有對應 Start 的
                    // SubagentStop（實測：agent_type 是空字串）。不是使用者開的 subagent，不顯示。
                    if self.index_of(&id).is_none() && agent_type_of(body).is_none() {
                        return false;
                    }
                    let i = self.ensure(&id, agent_type_of(body), now_ms);
                    self.nodes[i].last_event_ms = now_ms;
                    Self::finish(&mut self.nodes[i], Status::Done, now_ms);
                }
            }
            // 離開 claude（/exit、Ctrl+C、/clear）：圖屬於那一次對話，一併清空，
            // 側欄跟著消失；沒有轉圈到一半被留下來的節點。
            Kind::SessionEnd => {
                self.nodes.clear();
                self.pending.clear();
                self.main_tool = None;
                self.reset_main_activity();
                self.turn = 0;
            }
            Kind::Prompt => {
                self.turn += 1;
                self.main_tool = None;
                self.reset_main_activity();
            }
        }
        self.trim();
        self.visible_snapshot() != before
    }

    /// 結束後從 subagent 的 transcript 算出的 token 數（互動模式的 Agent 是非同步的，
    /// `PostToolUse` 沒有總數）。回傳有沒有變化。
    pub fn set_tokens(&mut self, id: &str, tokens: u64) -> bool {
        match self.index_of(id) {
            Some(i) if self.nodes[i].tokens != Some(tokens) => {
                self.nodes[i].tokens = Some(tokens);
                true
            }
            _ => false,
        }
    }

    fn note_main_tool(&mut self, hint: String) {
        self.main_tool_count += 1;
        self.main_recent.push_back(hint);
        while self.main_recent.len() > MAIN_RECENT_MAX {
            self.main_recent.pop_front();
        }
    }

    fn reset_main_activity(&mut self) {
        self.main_tool_count = 0;
        self.main_recent.clear();
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
            agent_type: agent_type.unwrap_or_else(|| FALLBACK_TYPE.to_owned()),
            description: None,
            status: Status::Running,
            started_ms: now_ms,
            ended_ms: None,
            current_tool: None,
            summary: None,
            tokens: None,
            tool_count: None,
            last_event_ms: now_ms,
            turn: self.turn,
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
        let i = self.ensure(id, agent_type.clone(), now_ms);
        self.nodes[i].last_event_ms = now_ms;
        if let Some(t) = agent_type {
            if self.nodes[i].agent_type == FALLBACK_TYPE {
                self.nodes[i].agent_type = t; // 先由別的事件建起來、當時不知道型別
            }
        }
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
        self.nodes[i].last_event_ms = now_ms;
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
        t.apply(Kind::Pre, &pre_agent("tu1", None, "Run ls", "general-purpose"), 1);
        assert!(t.nodes().is_empty(), "只排隊，還沒有節點");
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
    fn main_agent_tool_calls_never_create_nodes() {
        let mut t = Tree::default();
        let ev = json!({ "tool_name": "Bash", "tool_input": { "command": "ls" } });
        t.apply(Kind::Pre, &ev, 1);
        t.apply(Kind::Post, &ev, 2);
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
    fn session_end_clears_the_whole_graph() {
        let mut t = Tree::default();
        t.apply(Kind::SubStart, &start("done", "general-purpose"), 1);
        t.apply(Kind::SubStop, &stop("done"), 2);
        t.apply(Kind::SubStart, &start("run", "general-purpose"), 3);
        t.apply(Kind::Pre, &pre_agent("tu9", None, "never started", "general-purpose"), 5);
        assert!(t.apply(Kind::SessionEnd, &json!({}), 10));
        assert!(t.nodes().is_empty());
        assert!(!t.apply(Kind::SessionEnd, &json!({}), 11), "已經是空的，沒有變化");
        // 排隊中的 Agent 呼叫也一併清掉：下一次對話的 SubagentStart 不會認領到舊描述。
        t.apply(Kind::SubStart, &start("next", "general-purpose"), 12);
        assert_eq!(node(&t, "next").description, None);
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
    fn untyped_stop_for_an_unknown_agent_is_an_internal_helper_and_is_ignored() {
        // 實測：互動模式會送 agent_type 為空字串、且從沒 Start 過的 SubagentStop。
        let mut t = Tree::default();
        let ev = json!({ "hook_event_name": "SubagentStop", "agent_id": "ghost", "agent_type": "" });
        assert!(!t.apply(Kind::SubStop, &ev, 1));
        assert!(t.nodes().is_empty());
        // 有型別的才算（上面「stop_without_start…」那條）。
    }

    #[test]
    fn untyped_stop_for_a_known_agent_still_finishes_it() {
        let mut t = Tree::default();
        t.apply(Kind::SubStart, &start("a1", "general-purpose"), 1);
        let ev = json!({ "agent_id": "a1", "agent_type": "" });
        assert!(t.apply(Kind::SubStop, &ev, 2));
        assert_eq!(node(&t, "a1").status, Status::Done);
        assert_eq!(node(&t, "a1").agent_type, "general-purpose");
    }

    #[test]
    fn a_later_start_replaces_the_placeholder_type() {
        let mut t = Tree::default();
        // Post(Agent) 先到、回應沒帶型別 → 節點先以暫用型別建起來。
        let mut post = post_agent("tu1", None, "Run pwd", "a1", "async_launched");
        post["tool_response"].as_object_mut().unwrap().remove("agentType");
        t.apply(Kind::Post, &post, 1);
        assert_eq!(node(&t, "a1").agent_type, "agent");
        t.apply(Kind::SubStart, &start("a1", "Explore"), 2);
        assert_eq!(node(&t, "a1").agent_type, "Explore");
    }

    #[test]
    fn set_tokens_updates_a_known_node_and_reports_change_once() {
        let mut t = Tree::default();
        t.apply(Kind::SubStart, &start("a1", "general-purpose"), 1);
        assert!(t.set_tokens("a1", 1234));
        assert!(!t.set_tokens("a1", 1234));
        assert!(!t.set_tokens("nope", 1));
        assert_eq!(node(&t, "a1").tokens, Some(1234));
    }

    #[test]
    fn each_prompt_starts_a_new_turn_and_nodes_remember_theirs() {
        let mut t = Tree::default();
        assert_eq!(t.snapshot().turn, 0);
        assert!(t.apply(Kind::Prompt, &json!({}), 1));
        t.apply(Kind::SubStart, &start("a", "general-purpose"), 2);
        t.apply(Kind::Prompt, &json!({}), 3);
        t.apply(Kind::SubStart, &start("b", "general-purpose"), 4);
        assert_eq!((node(&t, "a").turn, node(&t, "b").turn), (1, 2));
        assert_eq!(t.snapshot().turn, 2);
    }

    #[test]
    fn main_agent_tool_use_is_tracked_separately_from_subagents() {
        let mut t = Tree::default();
        let pre = json!({ "tool_name": "Bash", "tool_input": { "command": "git status" } });
        assert!(t.apply(Kind::Pre, &pre, 1));
        assert_eq!(t.snapshot().main_tool.as_deref(), Some("Bash: git status"));
        assert!(t.nodes().is_empty(), "主 agent 的工具呼叫不產生節點");
        assert!(t.apply(Kind::Post, &json!({ "tool_name": "Bash" }), 2));
        assert_eq!(t.snapshot().main_tool, None);
        assert!(!t.apply(Kind::Post, &json!({ "tool_name": "Bash" }), 3), "沒變化就不通知");
    }

    #[test]
    fn main_agent_shows_the_agent_it_is_waiting_on() {
        let mut t = Tree::default();
        t.apply(Kind::Pre, &pre_agent("tu1", None, "整理檔案", "general-purpose"), 1);
        assert_eq!(t.snapshot().main_tool.as_deref(), Some("Agent: 整理檔案"));
        t.apply(Kind::Post, &post_agent("tu1", None, "整理檔案", "a1", "async_launched"), 2);
        assert_eq!(t.snapshot().main_tool, None);
        // subagent 自己再開 Agent 不算主 agent 在等。
        t.apply(Kind::Pre, &pre_agent("tu2", Some("a1"), "內層", "general-purpose"), 3);
        assert_eq!(t.snapshot().main_tool, None);
    }

    #[test]
    fn session_end_also_resets_the_turn_and_main_tool() {
        let mut t = Tree::default();
        t.apply(Kind::Prompt, &json!({}), 1);
        t.apply(Kind::Pre, &json!({ "tool_name": "Bash", "tool_input": { "command": "ls" } }), 2);
        t.apply(Kind::SessionEnd, &json!({}), 3);
        assert_eq!(t.snapshot(), Snapshot::default());
    }

    #[test]
    fn kind_parse_matches_the_urls_in_the_settings_file() {
        assert_eq!(Kind::parse("pre"), Some(Kind::Pre));
        assert_eq!(Kind::parse("post"), Some(Kind::Post));
        assert_eq!(Kind::parse("sub-start"), Some(Kind::SubStart));
        assert_eq!(Kind::parse("sub-stop"), Some(Kind::SubStop));
        assert_eq!(Kind::parse("session-end"), Some(Kind::SessionEnd));
        assert_eq!(Kind::parse("prompt"), Some(Kind::Prompt));
        assert_eq!(Kind::parse("nope"), None);
    }

    #[test]
    fn node_events_refresh_last_event_ms() {
        let mut t = Tree::default();
        t.apply(Kind::Pre, &pre_agent("tu1", None, "Run ls", "general-purpose"), 1);
        t.apply(Kind::SubStart, &start("a1", "general-purpose"), 10);
        assert_eq!(node(&t, "a1").last_event_ms, 10);
        t.apply(Kind::Pre, &tool_pre("a1", "Bash", json!({"command": "ls"})), 50);
        assert_eq!(node(&t, "a1").last_event_ms, 50);
        t.apply(Kind::Post, &tool_post("a1", "Bash"), 70);
        assert_eq!(node(&t, "a1").last_event_ms, 70);
        t.apply(Kind::SubStop, &stop("a1"), 90);
        assert_eq!(node(&t, "a1").last_event_ms, 90);
    }

    fn main_pre(tool: &str, input: Value) -> Value {
        json!({ "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input })
    }

    #[test]
    fn main_agent_tools_are_counted_and_kept_as_recent() {
        let mut t = Tree::default();
        t.apply(Kind::Pre, &main_pre("Read", json!({"file_path": "/a.rs"})), 1);
        t.apply(Kind::Pre, &main_pre("Bash", json!({"command": "ls"})), 2);
        let s = t.snapshot();
        assert_eq!(s.main_tool_count, 2);
        assert_eq!(s.main_recent, vec!["Read: /a.rs".to_owned(), "Bash: ls".to_owned()]);
    }

    #[test]
    fn main_recent_keeps_only_the_latest_ten() {
        let mut t = Tree::default();
        for i in 0..13 {
            t.apply(Kind::Pre, &main_pre("Grep", json!({"pattern": format!("p{i}")})), i);
        }
        let s = t.snapshot();
        assert_eq!(s.main_tool_count, 13);
        assert_eq!(s.main_recent.len(), 10);
        assert_eq!(s.main_recent.first().map(String::as_str), Some("Grep: p3"));
        assert_eq!(s.main_recent.last().map(String::as_str), Some("Grep: p12"));
    }

    #[test]
    fn main_agent_spawning_an_agent_counts_but_subagent_tools_do_not() {
        let mut t = Tree::default();
        t.apply(Kind::Pre, &pre_agent("tu1", None, "Run ls", "general-purpose"), 1);
        t.apply(Kind::SubStart, &start("a1", "general-purpose"), 2);
        t.apply(Kind::Pre, &tool_pre("a1", "Bash", json!({"command": "ls"})), 3);
        let s = t.snapshot();
        assert_eq!(s.main_tool_count, 1, "只有 Agent 呼叫本身；subagent 的工具不算主 Claude");
        assert_eq!(s.main_recent, vec!["Agent: Run ls".to_owned()]);
    }

    #[test]
    fn prompt_and_session_end_reset_main_activity() {
        let mut t = Tree::default();
        t.apply(Kind::Pre, &main_pre("Bash", json!({"command": "ls"})), 1);
        t.apply(Kind::Prompt, &json!({}), 2);
        let s = t.snapshot();
        assert_eq!((s.main_tool_count, s.main_recent.len()), (0, 0));
        t.apply(Kind::Pre, &main_pre("Bash", json!({"command": "ls"})), 3);
        t.apply(Kind::SessionEnd, &json!({}), 4);
        let s = t.snapshot();
        assert_eq!((s.main_tool_count, s.main_recent.len()), (0, 0));
    }
}
