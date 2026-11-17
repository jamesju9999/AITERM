//! 代理 Playwright MCP：對 AI 暴露 `sys_*` 工具（加上 `system`、`intent` 參數），
//! 把閘門的結果翻成 AI 看得懂的話。探勘結果（逾時、錯誤頁）見規格文件。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use super::approvals::{Approvals, APPROVAL_TIMEOUT};

/// 會動作的工具：一定要帶 `intent`。
const ACTING: &[&str] =
    &["click", "type", "fill_form", "press_key", "select_option", "drag", "navigate", "handle_dialog"];
/// 只讀取畫面或調整檢視的工具。
const READONLY: &[&str] = &[
    "snapshot", "take_screenshot", "find", "wait_for", "tabs", "navigate_back", "hover", "console_messages",
    "network_requests", "network_request", "resize",
];

fn base_name(upstream: &str) -> Option<&str> {
    upstream.strip_prefix("browser_")
}

pub fn is_acting(base: &str) -> bool {
    ACTING.contains(&base)
}

/// 把 Playwright MCP 的工具清單改成對外的清單：只留白名單、改名 `sys_*`、加參數。
pub fn expose_tools(upstream: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for t in upstream {
        let Some(base) = t["name"].as_str().and_then(base_name) else { continue };
        let acting = is_acting(base);
        if !acting && !READONLY.contains(&base) {
            continue;
        }
        let mut schema = t["inputSchema"].clone();
        if !schema.is_object() {
            schema = json!({ "type": "object", "properties": {} });
        }
        let props = schema["properties"].as_object().cloned().unwrap_or_default();
        let mut props = props;
        props.insert("system".into(), json!({ "type": "string", "description": "系統連線的 id" }));
        let mut required: Vec<Value> = schema["required"].as_array().cloned().unwrap_or_default();
        required.push(json!("system"));
        if acting {
            props.insert(
                "intent".into(),
                json!({ "type": "string", "description": "一句話說明這個動作要做什麼，並以「讀：」或「寫：」開頭表明是讀取還是寫入" }),
            );
            required.push(json!("intent"));
        }
        schema["properties"] = Value::Object(props);
        schema["required"] = Value::Array(required);
        out.push(json!({
            "name": format!("sys_{base}"),
            "description": t["description"].clone(),
            "inputSchema": schema,
        }));
    }
    out
}

/// 動作類工具沒有 `intent`（或是空白）就拒絕。
pub fn check_intent(tool: &str, args: &Value) -> Result<(), String> {
    let base = tool.strip_prefix("sys_").unwrap_or(tool);
    if !is_acting(base) {
        return Ok(());
    }
    match args["intent"].as_str() {
        Some(i) if !i.trim().is_empty() => Ok(()),
        _ => Err(format!("{tool} 必須帶 intent：一句話說明要做什麼，並以「讀：」或「寫：」開頭。")),
    }
}

pub fn is_playwright_timeout(result: &Value) -> bool {
    result["isError"] == true && result.to_string().contains("TimeoutError")
}

pub fn is_blocked_page(result: &Value) -> bool {
    result.to_string().contains("chrome-error://")
}

pub fn blocked_message(what: &str) -> String {
    format!("這個動作被閘門拒絕（{what}），請求沒有送出。不可重試同一個動作；請改用其他做法，或回報使用者需要開放寫入權限。")
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn result_text(result: &Value) -> String {
    result["content"]
        .as_array()
        .map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

/// 與 Playwright MCP 子行程的 stdio JSON-RPC 連線（一行一則訊息）。
pub struct Upstream {
    tx: mpsc::UnboundedSender<String>,
    next_id: AtomicU64,
    waiters: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    child: tokio::sync::Mutex<tokio::process::Child>,
}

impl Upstream {
    /// `cdp_http` 為 `None` 時不連瀏覽器（只用來取得工具清單）。
    pub async fn spawn(cdp_http: Option<&str>, output_dir: &std::path::Path) -> Result<Self, String> {
        let mut args = vec!["-y".to_string(), crate::tasks::ask::PLAYWRIGHT_MCP.to_string()];
        if let Some(c) = cdp_http {
            args.extend(["--cdp-endpoint".to_string(), c.to_string()]);
        }
        args.extend(["--output-dir".to_string(), output_dir.to_string_lossy().into_owned()]);
        let mut cmd = crate::mcp::transport::build_command("npx", &args, &HashMap::new());
        cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| format!("無法啟動 Playwright MCP：{e}"))?;
        let mut stdin = child.stdin.take().ok_or("沒有 stdin")?;
        let stdout = child.stdout.take().ok_or("沒有 stdout")?;
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let waiters: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> = Arc::new(Mutex::new(HashMap::new()));
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if stdin.write_all(format!("{line}\n").as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });
        let w = waiters.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(id) = v["id"].as_u64() {
                    if let Some(tx) = w.lock().remove(&id) {
                        let _ = tx.send(v);
                    }
                }
            }
        });
        let up = Self { tx, next_id: AtomicU64::new(1), waiters, child: tokio::sync::Mutex::new(child) };
        up.request("initialize", crate::mcp::protocol::initialize_params(), Duration::from_secs(60)).await?;
        let _ = up.tx.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string());
        Ok(up)
    }

    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().insert(id, tx);
        self.tx
            .send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string())
            .map_err(|_| "Playwright MCP 已結束".to_string())?;
        let v = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| format!("Playwright MCP {method} 逾時"))?
            .map_err(|_| "Playwright MCP 已結束".to_string())?;
        if let Some(e) = v.get("error") {
            return Err(format!("Playwright MCP {method} 失敗：{e}"));
        }
        Ok(v["result"].clone())
    }

    pub async fn kill(&self) {
        let _ = self.child.lock().await.kill().await;
    }
}

/// 取得對外的工具清單：啟動一個不連瀏覽器的 Playwright MCP 讀 `tools/list` 後關掉。
pub async fn fetch_catalog(output_dir: &std::path::Path) -> Result<Vec<Value>, String> {
    let up = Upstream::spawn(None, output_dir).await?;
    let list = up.request("tools/list", json!({}), Duration::from_secs(30)).await;
    up.kill().await;
    let list = list?;
    Ok(expose_tools(list["tools"].as_array().map(Vec::as_slice).unwrap_or(&[])))
}

/// 一個系統的代理：持有上游連線與閘門共享的狀態。
pub struct Proxy {
    pub system_id: String,
    upstream: Upstream,
    tools: Vec<Value>,
    approvals: Arc<Approvals>,
    intent: Arc<Mutex<String>>,
    manual: Arc<std::sync::atomic::AtomicBool>,
    last_denied: Arc<Mutex<Option<String>>>,
}

impl Proxy {
    pub async fn start(
        system_id: &str,
        upstream: Upstream,
        approvals: Arc<Approvals>,
        intent: Arc<Mutex<String>>,
        manual: Arc<std::sync::atomic::AtomicBool>,
        last_denied: Arc<Mutex<Option<String>>>,
    ) -> Result<Self, String> {
        let list = upstream.request("tools/list", json!({}), Duration::from_secs(30)).await?;
        let tools = expose_tools(list["tools"].as_array().map(Vec::as_slice).unwrap_or(&[]));
        Ok(Self { system_id: system_id.into(), upstream, tools, approvals, intent, manual, last_denied })
    }

    pub fn tools(&self) -> &[Value] {
        &self.tools
    }

    pub async fn close(&self) {
        self.upstream.kill().await;
    }

    async fn upstream_call(&self, name: &str, args: Value) -> Result<Value, String> {
        self.upstream.request("tools/call", json!({ "name": name, "arguments": args }), Duration::from_secs(90)).await
    }

    /// 轉發一次工具呼叫，並把閘門造成的結果改寫成 AI 看得懂的話。
    pub async fn call(&self, tool: &str, mut args: Value) -> Result<Value, String> {
        let base = tool.strip_prefix("sys_").ok_or_else(|| format!("未知的工具：{tool}"))?;
        if !self.tools.iter().any(|t| t["name"] == tool) {
            return Err(format!("未知的工具：{tool}"));
        }
        check_intent(tool, &args)?;
        if self.manual.load(Ordering::SeqCst) {
            return Ok(text_result("使用者目前接手操作中，請稍候；使用者交還後再繼續。".into(), true));
        }
        if let Some(i) = args["intent"].as_str() {
            *self.intent.lock() = i.to_string();
        }
        if let Some(o) = args.as_object_mut() {
            o.remove("system");
            o.remove("intent");
        }
        let mut result = self.upstream_call(&format!("browser_{base}"), args).await?;

        if is_playwright_timeout(&result) && self.approvals.has_pending(&self.system_id) {
            // 這個動作的請求正等使用者核准。Playwright 自己的逾時不能讓 AI 看到，否則它會以為失敗而重送。
            let deadline = tokio::time::Instant::now() + APPROVAL_TIMEOUT + Duration::from_secs(5);
            while self.approvals.has_pending(&self.system_id) && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            tokio::time::sleep(Duration::from_millis(800)).await;
            let snap = self.upstream_call("browser_snapshot", json!({})).await?;
            if is_blocked_page(&snap) {
                result = snap;
            } else {
                return Ok(text_result(
                    format!("這個動作需要人工核准，使用者已處理。是否核准以最新畫面為準：\n{}", result_text(&snap)),
                    false,
                ));
            }
        }
        if is_blocked_page(&result) {
            let what = self.last_denied.lock().clone().unwrap_or_else(|| "寫入請求".into());
            let _ = self.upstream_call("browser_navigate_back", json!({})).await;
            return Ok(text_result(blocked_message(&what), true));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whitelisted_tools_are_exposed_and_get_system_and_intent_params() {
        let upstream = json!([
            {"name":"browser_click","description":"Click","inputSchema":{"type":"object","properties":{"target":{"type":"string"}},"required":["target"]}},
            {"name":"browser_evaluate","description":"Eval","inputSchema":{"type":"object","properties":{}}},
            {"name":"browser_snapshot","description":"Snap","inputSchema":{"type":"object","properties":{}}}
        ]);
        let tools = expose_tools(upstream.as_array().unwrap());
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["sys_click", "sys_snapshot"], "evaluate 不開放");
        assert_eq!(tools[0]["inputSchema"]["required"], json!(["target", "system", "intent"]));
        assert_eq!(tools[1]["inputSchema"]["required"], json!(["system"]), "唯讀的 snapshot 不必 intent");
        assert!(tools[0]["inputSchema"]["properties"]["target"].is_object(), "原本的參數要保留");
    }

    #[test]
    fn dangerous_tools_are_never_exposed() {
        let names = ["evaluate", "run_code_unsafe", "file_upload", "drop", "close", "emulate_media"];
        let upstream: Vec<Value> =
            names.iter().map(|n| json!({"name": format!("browser_{n}"), "inputSchema": {"type":"object"}})).collect();
        assert!(expose_tools(&upstream).is_empty());
    }

    #[test]
    fn acting_tools_without_intent_are_rejected() {
        let err = check_intent("sys_click", &json!({"system":"s","target":"e4"})).unwrap_err();
        assert!(err.contains("intent"));
        assert!(check_intent("sys_click", &json!({"system":"s","intent":"  "})).is_err(), "空白不算");
        assert!(check_intent("sys_click", &json!({"system":"s","target":"e4","intent":"讀：點查詢"})).is_ok());
        assert!(check_intent("sys_snapshot", &json!({"system":"s"})).is_ok());
    }

    #[test]
    fn a_timeout_is_recognised_only_when_it_is_an_error_timeout() {
        let timeout = json!({"content":[{"type":"text","text":"### Error\nTimeoutError: browserBackend.callTool: Timeout 5000ms exceeded."}],"isError":true});
        assert!(is_playwright_timeout(&timeout));
        assert!(!is_playwright_timeout(&json!({"content":[{"type":"text","text":"ok"}]})));
        assert!(!is_playwright_timeout(&json!({"content":[{"type":"text","text":"page says TimeoutError"}]})), "沒有 isError 不算");
    }

    #[test]
    fn a_gate_blocked_page_is_rewritten_for_the_ai() {
        let blocked = json!({"content":[{"type":"text","text":"### Page\n- Page URL: chrome-error://chromewebdata/\n"}]});
        assert!(is_blocked_page(&blocked));
        assert!(!is_blocked_page(&json!({"content":[{"type":"text","text":"- Page URL: https://x/"}]})));
        let msg = blocked_message("POST /orders");
        assert!(msg.contains("被閘門拒絕") && msg.contains("不可重試") && msg.contains("POST /orders"));
    }

    /// 需要真的 Chrome 與網路（npx 取 Playwright MCP）：
    /// `cargo test -p app --lib -- --ignored --test-threads=1 systems::proxy`
    #[tokio::test]
    #[ignore]
    async fn read_only_through_the_proxy_gives_the_ai_a_clear_refusal_and_the_server_sees_nothing() {
        use crate::systems::browser;
        use crate::systems::cdp::{run_gate, GateCtx, GateEvent};
        use crate::systems::testsite::{conn, fake_site};
        use crate::systems::types::Mode;

        let (port, posts) = fake_site().await;
        let dir = tempfile::tempdir().unwrap();
        let mut b = browser::launch(dir.path(), "t", "about:blank").await.unwrap();
        let approvals = Arc::new(Approvals::new());
        let last_denied: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let ld = last_denied.clone();
        let emit: Arc<dyn Fn(GateEvent) + Send + Sync> = Arc::new(move |e| {
            if let GateEvent::Denied { method, url, .. } = e {
                *ld.lock() = Some(format!("{method} {url}"));
            }
        });
        let ctx = GateCtx::new(conn(port, Mode::ReadOnly), approvals.clone(), emit);
        let (intent, manual) = (ctx.intent.clone(), ctx.manual.clone());
        let ws = b.ws_url.clone();
        let gate = tokio::spawn(async move { run_gate(&ws, ctx).await });
        tokio::time::sleep(Duration::from_millis(800)).await;

        let up = Upstream::spawn(Some(&b.cdp_http()), dir.path()).await.unwrap();
        let proxy = Proxy::start("t", up, approvals, intent, manual, last_denied).await.unwrap();
        assert!(proxy.tools().iter().any(|t| t["name"] == "sys_click"));

        let url = format!("http://127.0.0.1:{port}/");
        proxy.call("sys_navigate", json!({"system":"t","url":url,"intent":"讀：開首頁"})).await.unwrap();
        let snap = proxy.call("sys_snapshot", json!({"system":"t"})).await.unwrap();
        let text = result_text(&snap);
        let r = text.split("button \"Save\" [ref=").nth(1).and_then(|s| s.split(']').next()).expect(&text).to_string();
        let click = proxy.call("sys_click", json!({"system":"t","target":r,"intent":"寫：按儲存"})).await.unwrap();
        let said = result_text(&click);
        assert!(said.contains("被閘門拒絕") && said.contains("POST"), "{said}");
        assert_eq!(posts.load(Ordering::SeqCst), 0, "伺服器不該收到 POST");

        proxy.close().await;
        gate.abort();
        b.close().await;
    }

    /// 請求被扣住等核准時 Playwright 約 5 秒後會逾時；代理不能把逾時回給 AI，要等使用者決定。
    #[tokio::test]
    #[ignore]
    async fn a_click_waiting_for_approval_does_not_surface_playwrights_timeout() {
        use crate::systems::browser;
        use crate::systems::cdp::{run_gate, GateCtx, GateEvent};
        use crate::systems::testsite::{conn, fake_site};
        use crate::systems::types::Mode;

        let (port, posts) = fake_site().await;
        let dir = tempfile::tempdir().unwrap();
        let mut b = browser::launch(dir.path(), "t", "about:blank").await.unwrap();
        let approvals = Arc::new(Approvals::new());
        let ap = approvals.clone();
        let emit: Arc<dyn Fn(GateEvent) + Send + Sync> = Arc::new(move |e| {
            if let GateEvent::NeedApproval { id, .. } = e {
                let ap = ap.clone();
                // 比 Playwright 的 5 秒逾時更晚才核准。
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(8)).await;
                    ap.decide(&id, true);
                });
            }
        });
        let ctx = GateCtx::new(conn(port, Mode::ApproveEach), approvals.clone(), emit);
        let (intent, manual) = (ctx.intent.clone(), ctx.manual.clone());
        let ws = b.ws_url.clone();
        let gate = tokio::spawn(async move { run_gate(&ws, ctx).await });
        tokio::time::sleep(Duration::from_millis(800)).await;
        let up = Upstream::spawn(Some(&b.cdp_http()), dir.path()).await.unwrap();
        let proxy = Proxy::start("t", up, approvals, intent, manual, Arc::new(Mutex::new(None))).await.unwrap();

        let url = format!("http://127.0.0.1:{port}/");
        proxy.call("sys_navigate", json!({"system":"t","url":url,"intent":"讀：開首頁"})).await.unwrap();
        let snap = proxy.call("sys_snapshot", json!({"system":"t"})).await.unwrap();
        let text = result_text(&snap);
        let r = text.split("button \"Save\" [ref=").nth(1).and_then(|s| s.split(']').next()).expect(&text).to_string();
        let click = proxy.call("sys_click", json!({"system":"t","target":r,"intent":"寫：按儲存"})).await.unwrap();
        let said = result_text(&click);
        assert!(!said.contains("TimeoutError"), "AI 不該看到 Playwright 的逾時：{said}");
        assert!(said.contains("人工核准"), "{said}");
        assert_eq!(posts.load(Ordering::SeqCst), 1, "核准後伺服器收到一次");

        proxy.close().await;
        gate.abort();
        b.close().await;
    }
}
