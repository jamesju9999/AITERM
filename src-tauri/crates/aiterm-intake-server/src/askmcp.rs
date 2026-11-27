//! 「中途問人」工具：Agent 需要人決定時呼叫 `ask_human`，工具呼叫會掛起，直到有人回答（或取消／卡結束）。
//! 掛起的做法同資料庫的每次核准：輪詢資料庫、Claude 的工具逾時已調到 24 小時。
use std::sync::Arc;
use std::time::{Duration, Instant};

use aiterm_intake_core::board_questions::{self as bq, NewQuestion};
use aiterm_intake_core::host::Notifier;
use serde_json::{json, Value};

use crate::dbmcp::McpCtx;
use crate::state::AppState;

const POLL: Duration = Duration::from_millis(500);
const WAIT_LIMIT: Duration = Duration::from_secs(24 * 3600);

pub fn tools() -> Value {
    json!([{
        "name": "ask_human",
        "description": "遇到需要人決定的事（規格不清楚、有多種做法要選、缺少你無法自己取得的資訊）時問人。這個工具會等到有人回答才回來，回答就是工具的結果。能自己判斷的事不要問；一次只問一個問題；有明確選項時請列出選項。",
        "inputSchema": {"type": "object", "properties": {
            "question": {"type": "string", "description": "要問的問題，說清楚你卡在哪裡。"},
            "options": {"type": "array", "items": {"type": "string"}, "description": "選項（最多 6 個），有的話人只要點選。"},
            "recommended": {"type": "string", "description": "你建議的選項或答案。"},
            "context": {"type": "string", "description": "補充背景（你已經試過什麼、為什麼這樣問）。"}
        }, "required": ["question"]}
    }])
}

fn text(s: impl Into<String>, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": s.into()}], "isError": is_error})
}

pub async fn call(st: &Arc<AppState>, token: &str, ctx: &McpCtx, name: &str, args: &Value) -> Value {
    if name != "ask_human" {
        return text(format!("沒有這個工具：{name}"), true);
    }
    let options: Vec<String> = args["options"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let new = NewQuestion {
        project_id: &ctx.project_id,
        card_id: &ctx.card_id,
        agent_id: ctx.agent_id.as_deref(),
        question: args["question"].as_str().unwrap_or(""),
        options: &options,
        recommended: args["recommended"].as_str().filter(|s| !s.trim().is_empty()),
        context: args["context"].as_str().filter(|s| !s.trim().is_empty()),
    };
    let id = match bq::ask(&st.pools.intake, &new).await {
        Ok(Ok(id)) => id,
        Ok(Err(why)) => return text(why, true),
        Err(e) => return text(format!("無法記錄問題：{e}"), true),
    };
    st.events.changed();
    if let Ok(Some(q)) = bq::get(&st.pools.intake, id).await {
        crate::asknotify::opened(st, &q).await;
    }
    let started = Instant::now();
    loop {
        tokio::time::sleep(POLL).await;
        match bq::get(&st.pools.intake, id).await {
            Ok(Some(q)) if q.status == bq::ANSWERED => return text(format!("人的回答：{}", q.answer.unwrap_or_default()), false),
            Ok(Some(q)) if q.status == bq::CANCELLED => return text("這個問題被取消了，沒有人回答。請依你的判斷繼續；不確定的地方在結果裡說明。", true),
            Ok(None) | Err(_) => return text("找不到這個問題的記錄。", true),
            _ => {}
        }
        if !st.mcp.alive(token) {
            if bq::cancel(&st.pools.intake, id, "system").await.unwrap_or(false) {
                st.events.changed();
            }
            return text("這張卡已經結束，問題沒有人回答。", true);
        }
        if started.elapsed() > WAIT_LIMIT {
            if bq::cancel(&st.pools.intake, id, "system").await.unwrap_or(false) {
                st.events.changed();
                if let Ok(Some(q)) = bq::get(&st.pools.intake, id).await {
                    crate::asknotify::closed_one(st, &q).await;
                }
            }
            return text("等了太久沒有人回答，請依你的判斷繼續；不確定的地方在結果裡說明。", true);
        }
    }
}
