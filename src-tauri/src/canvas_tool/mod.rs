//! Ask AI 的「Agent 協作」內建工具：讓 Ask AI 把工作派給 Agent 協作的角色、查看板與結果。
//! 設計見 docs/superpowers/specs/2026-11-18-ask-ai-agent-canvas-tools-design.md。
//!
//! 跟電腦操作（`computer`）一樣是內建虛擬伺服器：工具清單由 `ai_chat` 依設定組出，
//! 執行走 `execute_mcp_tool`。建卡要使用者確認；其餘只讀。

pub mod ops;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use crate::config::{AppConfig, ConfigStore};
use crate::mcp::types::{decode_tool_name, encode_tool_name, McpToolInfo, McpToolResult};
use crate::projects::ProjectRegistry;
use crate::tasks::agents;
use crate::tasks::scheduler::SchedulerHandle;

pub const BUILTIN_SERVER_ID: &str = "aiterm-canvas";
const BUILTIN_SERVER_NAME: &str = "AITerm Agent Canvas";
/// 使用者沒回應多久就當作拒絕。
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(300);
pub const CONFIRM_EVENT: &str = "canvas://confirm-dispatch";

/// 附加到系統提示詞的使用規則（只在工具真的給出去時附上）。
pub const PROMPT_HINT: &str = "Agent canvas tools: use them for work that is long-running or should run in the background by a team role. \
First call aiterm_canvas__canvas_list_agents to see who has the skills needed (for example a system connection), then \
aiterm_canvas__canvas_dispatch with a clear title and a complete instruction (what to do and what output you need). \
The user must approve every dispatch; if they decline or it times out, do not retry — tell them. After dispatching, report the card id and \
do not wait for the result; when the user later asks, use aiterm_canvas__canvas_read_card.";

/// 一次只允許一個待確認的派工。
#[derive(Default)]
pub struct CanvasConfirm {
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Approved,
    Rejected,
    TimedOut,
}

impl CanvasConfirm {
    /// 送出確認請求並等使用者決定。已有待確認的請求時直接回 `Err`。
    pub async fn ask(&self, emit: impl FnOnce(&str, Value), mut payload: Value, timeout: Duration) -> Result<Decision, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let rx = {
            let mut p = self.pending.lock();
            if !p.is_empty() {
                return Err("有另一個派工正在等使用者確認，請稍後再派。".into());
            }
            let (tx, rx) = oneshot::channel();
            p.insert(id.clone(), tx);
            rx
        };
        payload["id"] = json!(id);
        emit(CONFIRM_EVENT, payload);
        let out = tokio::time::timeout(timeout, rx).await;
        self.pending.lock().remove(&id);
        Ok(match out {
            Ok(Ok(true)) => Decision::Approved,
            Ok(_) => Decision::Rejected,
            Err(_) => Decision::TimedOut,
        })
    }

    /// 前端回覆。請求已逾時或不存在回 `false`。
    pub fn decide(&self, id: &str, approve: bool) -> bool {
        match self.pending.lock().remove(id) {
            Some(tx) => tx.send(approve).is_ok(),
            None => false,
        }
    }
}

pub fn tool_infos(cfg: &AppConfig) -> Vec<McpToolInfo> {
    if !cfg.mcp_tool_server.agent_canvas_enabled {
        return Vec::new();
    }
    let mk = |name: &str, description: &str, schema: Value| McpToolInfo {
        server_id: BUILTIN_SERVER_ID.to_string(),
        server_name: BUILTIN_SERVER_NAME.to_string(),
        name: encode_tool_name(BUILTIN_SERVER_ID, name),
        description: description.to_string(),
        input_schema: schema,
    };
    let project = json!({ "type": "string", "description": "專案名稱或 id；只有一個專案開著時可省略" });
    vec![
        mk(
            "canvas_list_projects",
            "List the projects currently open in AITerm's Agent canvas (name, id, folder).",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        mk(
            "canvas_list_agents",
            "List the roles (agents) of a project: name, id, kind (coder/verifier/reviewer/…), skills (system connections, database…) and whether it is busy. Only coders and verifiers can be dispatched to.",
            json!({ "type": "object", "properties": { "project": project }, "additionalProperties": false }),
        ),
        mk(
            "canvas_dispatch",
            "Dispatch a job to a coder or verifier role: creates a card that the role works on in the background. The user is asked to approve before anything is created. Returns the card id. Fails if the role is busy.",
            json!({
                "type": "object",
                "properties": {
                    "project": project,
                    "agent": { "type": "string", "description": "角色名稱或 id" },
                    "title": { "type": "string", "description": "卡片標題（80 字內）" },
                    "instruction": { "type": "string", "description": "完整指示：要做什麼、輸出要求（4000 字內）" }
                },
                "required": ["agent", "title", "instruction"],
                "additionalProperties": false
            }),
        ),
        mk(
            "canvas_list_cards",
            "List the cards on a project's board (title, id, status, role, kind), newest first. Optional status filter: planning, queued, running or done.",
            json!({ "type": "object", "properties": { "project": project, "status": { "type": "string" } }, "additionalProperties": false }),
        ),
        mk(
            "canvas_read_card",
            "Read a card's result: status, error, summary and the end of its conversation log. Says so when the card is not finished yet.",
            json!({ "type": "object", "properties": { "project": project, "card_id": { "type": "string" } }, "required": ["card_id"], "additionalProperties": false }),
        ),
    ]
}

/// 若 `encoded_name` 屬於這個內建伺服器，回傳原始工具名。
pub fn builtin_tool_name(encoded_name: &str) -> Option<&str> {
    let (server, tool) = decode_tool_name(encoded_name)?;
    (format!("{server}__") == encode_tool_name(BUILTIN_SERVER_ID, "")).then_some(tool)
}

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or("")
}

/// Ask AI 的入口。錯誤一律包成 `is_error` 的結果，讓模型讀得到原因。
pub async fn call(app: &AppHandle, tool: &str, args: Value) -> McpToolResult {
    let enabled = app.state::<Arc<ConfigStore>>().get().mcp_tool_server.agent_canvas_enabled;
    if !enabled {
        return McpToolResult { content: "Agent 協作工具已停用，請使用者到設定 → MCP 工具伺服器開啟。".into(), is_error: true };
    }
    match run(app, tool, &args).await {
        Ok(content) => McpToolResult { content, is_error: false },
        Err(content) => McpToolResult { content, is_error: true },
    }
}

async fn run(app: &AppHandle, tool: &str, args: &Value) -> Result<String, String> {
    let reg = app.state::<ProjectRegistry>();
    let projects = reg.all();
    if tool == "canvas_list_projects" {
        return Ok(ops::list_projects_text(&projects));
    }
    let project = ops::find_project(&projects, arg(args, "project"))?;
    let pool = &project.pool;
    match tool {
        "canvas_list_agents" => ops::list_agents_text(pool).await,
        "canvas_list_cards" => ops::list_cards_text(pool, args["status"].as_str()).await,
        "canvas_read_card" => ops::read_card_text(pool, arg(args, "card_id")).await,
        "canvas_dispatch" => {
            let all = agents::list_agents(pool).await.map_err(|e| e.to_string())?;
            let agent = ops::find_agent(&all, arg(args, "agent"))?;
            let (title, instruction) = ops::validate_dispatch(agent, arg(args, "title"), arg(args, "instruction"))?;
            if ops::agent_busy(pool, &agent.id).await? {
                return Err(format!("「{}」現在有卡片在排隊或執行，請稍後再派，或改派別的角色。", agent.name));
            }
            let head_sha = if agent.kind == "verifier" { crate::tasks::scheduler::schedule_head_sha(pool).await } else { None };

            let confirm = app.state::<Arc<CanvasConfirm>>();
            let handle = app.clone();
            let decision = confirm
                .ask(move |name, payload| { let _ = handle.emit(name, payload); }, ops::confirm_payload(&project.name, agent, &title, &instruction), CONFIRM_TIMEOUT)
                .await?;
            match decision {
                Decision::Approved => {}
                Decision::Rejected => return Err("使用者拒絕了這次派工。請告訴使用者，不要重試。".into()),
                Decision::TimedOut => return Err("使用者沒有在時限內回應，已取消派工。請告訴使用者，不要重試。".into()),
            }
            let id = ops::dispatch(pool, &project.path, agent, &title, &instruction, head_sha.as_deref()).await?;
            app.state::<SchedulerHandle>().poke();
            let _ = app.emit("tasks-updated", ());
            Ok(format!("已派工給「{}」，卡片 id：{id}。它會在背景執行；之後可用 canvas_read_card 查結果。", agent.name))
        }
        other => Err(format!("未知的工具：{other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(on: bool) -> AppConfig {
        let mut c = AppConfig::default();
        c.mcp_tool_server.agent_canvas_enabled = on;
        c
    }

    #[test]
    fn tools_are_hidden_unless_the_switch_is_on() {
        assert!(tool_infos(&cfg(false)).is_empty());
        let names: Vec<String> = tool_infos(&cfg(true)).into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            ["canvas_list_projects", "canvas_list_agents", "canvas_dispatch", "canvas_list_cards", "canvas_read_card"].map(|n| format!("aiterm_canvas__{n}"))
        );
    }

    #[test]
    fn only_dispatch_requires_the_content_fields() {
        let infos = tool_infos(&cfg(true));
        let d = infos.iter().find(|t| t.name.ends_with("canvas_dispatch")).unwrap();
        assert_eq!(d.input_schema["required"], json!(["agent", "title", "instruction"]));
    }

    #[test]
    fn builtin_names_are_recognised_and_others_are_not() {
        assert_eq!(builtin_tool_name("aiterm_canvas__canvas_dispatch"), Some("canvas_dispatch"));
        assert_eq!(builtin_tool_name("aiterm_computer__computer_click"), None);
        assert_eq!(builtin_tool_name("srv__x"), None);
    }

    #[tokio::test]
    async fn approval_resolves_the_waiting_request() {
        let c = Arc::new(CanvasConfirm::default());
        let seen = Arc::new(Mutex::new(None::<String>));
        let (c2, s2) = (c.clone(), seen.clone());
        let t = tokio::spawn(async move {
            c2.ask(move |_, p| { *s2.lock() = p["id"].as_str().map(String::from); }, json!({"title": "x"}), Duration::from_secs(5)).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let id = seen.lock().clone().expect("事件應帶 id");
        assert!(c.decide(&id, true));
        assert_eq!(t.await.unwrap().unwrap(), Decision::Approved);
        assert!(!c.decide(&id, true), "同一個請求不能決定兩次");
    }

    #[tokio::test]
    async fn rejection_timeout_and_a_second_concurrent_request_are_handled() {
        let c = Arc::new(CanvasConfirm::default());
        let ids = Arc::new(Mutex::new(Vec::<String>::new()));
        let (c2, i2) = (c.clone(), ids.clone());
        let first = tokio::spawn(async move { c2.ask(move |_, p| i2.lock().push(p["id"].as_str().unwrap().into()), json!({}), Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        // 已有一個待確認時，第二個直接被擋。
        let blocked = c.ask(|_, _| {}, json!({}), Duration::from_secs(1)).await;
        assert!(blocked.unwrap_err().contains("另一個派工"));
        let id = ids.lock()[0].clone();
        c.decide(&id, false);
        assert_eq!(first.await.unwrap().unwrap(), Decision::Rejected);
        // 逾時視為沒回應，且清掉待確認，下一個請求可以進來。
        assert_eq!(c.ask(|_, _| {}, json!({}), Duration::from_millis(50)).await.unwrap(), Decision::TimedOut);
        assert!(c.pending.lock().is_empty());
    }
}
