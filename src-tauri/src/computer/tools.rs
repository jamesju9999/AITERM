//! 電腦操作工具的「定義與分派」，與傳輸方式無關：
//! MCP server（外部客戶端）與內建 Ask AI（虛擬 MCP 伺服器，不經 HTTP）共用同一個 [`execute`]。
//!
//! 內建 Ask AI 的 MCP client 只保留文字，所以影像在這邊會被換成一段說明。

use rmcp::schemars;
use serde_json::Value;

use super::backend::MouseButton;
use super::{ClickTarget, ComputerService, ToolOutput};
use crate::mcp::types::{decode_tool_name, encode_tool_name, McpToolInfo, McpToolResult};

/// 內建虛擬伺服器的 id／顯示名稱（工具名編碼成 `aiterm_computer__computer_click`）。
pub const BUILTIN_SERVER_ID: &str = "aiterm-computer";
pub const BUILTIN_SERVER_NAME: &str = "AITerm Computer Use";

/// 一批動作最多幾步：夠把「點搜尋欄 → 輸入 → 送出」做完，又不會變成失控的腳本。
pub const MAX_BATCH_STEPS: usize = 10;

/// 有電腦操作工具時附加到 Ask AI 系統提示詞的操作心法（弱一點的模型靠這個少走很多彎路）。
pub const PROMPT_HINT: &str = "Computer-use tips: (1) Call computer_list_apps once to get exact app names. \
(2) To find a button or field, use computer_find with a word from its label — it is much cheaper than \
computer_snapshot. (3) Actions (click/type/key/scroll) already return the screen afterwards, so do not call \
snapshot again just to look. (4) When you know several steps in advance (click the search box, type, press \
return), send them together with computer_batch. (5) To replace a field's text use computer_type with \
replace=true; to clear it send cmd+a then delete. (6) Always read the result text to confirm the action \
really worked before telling the user it is done. (7) Element ids come from the latest snapshot/find/action \
result; if told an id is stale, use the ids from the newest result.";

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerSnapshotArgs {
    /// App name or bundle id, as returned by `computer_list_apps`.
    pub app: String,
    /// How many levels of the accessibility tree to read (default 6, max 12).
    #[serde(default)]
    pub max_depth: Option<usize>,
    /// Default true: omit empty layout containers and invisible elements (ids are unchanged).
    /// Set false to see every element.
    #[serde(default)]
    pub compact: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerFindArgs {
    /// App name or bundle id.
    pub app: String,
    /// Text to look for in element titles/roles (case-insensitive), e.g. "search" or "Send".
    #[serde(default)]
    pub query: Option<String>,
    /// Only elements with this accessibility role, e.g. "AXButton", "AXTextField".
    #[serde(default)]
    pub role: Option<String>,
    /// Maximum results (default 20, max 100).
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerScreenshotArgs {
    /// App name or bundle id. Omit to capture the whole main screen.
    #[serde(default)]
    pub app: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerClickArgs {
    /// App name or bundle id, as returned by `computer_list_apps`.
    pub app: String,
    /// Element id from the latest `computer_snapshot`/`computer_find` (e.g. "0.3.1"). Preferred over x/y.
    #[serde(default)]
    pub element: Option<String>,
    /// Screen x coordinate (use with y when no element id is available).
    #[serde(default)]
    pub x: Option<f64>,
    /// Screen y coordinate.
    #[serde(default)]
    pub y: Option<f64>,
    /// "left" (default) or "right".
    #[serde(default)]
    pub button: Option<String>,
    /// Double-click.
    #[serde(default)]
    pub double: Option<bool>,
    /// Default true: append the resulting screen to the reply. Set false to skip it.
    #[serde(default)]
    pub observe: Option<bool>,
}

impl ComputerClickArgs {
    pub fn into_parts(self) -> Result<(String, ClickTarget, MouseButton, bool), String> {
        let target = match (self.element, self.x, self.y) {
            (Some(id), _, _) => ClickTarget::Element(id),
            (None, Some(x), Some(y)) => ClickTarget::Point(x, y),
            _ => return Err("請提供 element，或同時提供 x 與 y".to_string()),
        };
        let button = match self.button.as_deref() {
            None | Some("left") => MouseButton::Left,
            Some("right") => MouseButton::Right,
            Some(other) => return Err(format!("button 只能是 left 或 right，收到「{other}」")),
        };
        Ok((self.app, target, button, self.double.unwrap_or(false)))
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerTypeArgs {
    /// App name or bundle id.
    pub app: String,
    /// Text to type into the focused field.
    pub text: String,
    /// Optional element id (from `computer_snapshot`) to focus before typing.
    #[serde(default)]
    pub element: Option<String>,
    /// If true, select all existing text first so the typed text REPLACES it
    /// (use this to overwrite or clear-and-fill a field).
    #[serde(default)]
    pub replace: Option<bool>,
    /// Default true: append the resulting screen to the reply. Set false to skip it.
    #[serde(default)]
    pub observe: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerKeyArgs {
    /// App name or bundle id.
    pub app: String,
    /// Key or combo, e.g. "return", "cmd+s", "cmd+shift+left".
    pub key: String,
    /// Default true: append the resulting screen to the reply. Set false to skip it.
    #[serde(default)]
    pub observe: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerScrollArgs {
    /// App name or bundle id.
    pub app: String,
    /// Horizontal scroll in lines (positive = right).
    #[serde(default)]
    pub dx: Option<i32>,
    /// Vertical scroll in lines (positive = up).
    #[serde(default)]
    pub dy: Option<i32>,
    /// Optional screen x to scroll at.
    #[serde(default)]
    pub x: Option<f64>,
    /// Optional screen y to scroll at.
    #[serde(default)]
    pub y: Option<f64>,
    /// Default true: append the resulting screen to the reply. Set false to skip it.
    #[serde(default)]
    pub observe: Option<bool>,
}

impl ComputerScrollArgs {
    pub fn at(&self) -> Option<(f64, f64)> {
        match (self.x, self.y) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        }
    }
}

/// `computer_batch` 的一步。`action` 決定要填哪些欄位：
/// click(element 或 x+y，可選 button/double)、type(text，可選 element/replace)、
/// key(key)、scroll(dx/dy，可選 x/y)。
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchStep {
    /// "click", "type", "key" or "scroll".
    pub action: String,
    #[serde(default)]
    pub element: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub button: Option<String>,
    #[serde(default)]
    pub double: Option<bool>,
    /// For "type".
    #[serde(default)]
    pub text: Option<String>,
    /// For "type": select all first so the text replaces the existing content.
    #[serde(default)]
    pub replace: Option<bool>,
    /// For "key", e.g. "return" or "cmd+a".
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub dx: Option<i32>,
    #[serde(default)]
    pub dy: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerBatchArgs {
    /// App name or bundle id.
    pub app: String,
    /// Up to 10 steps, run in order; stops at the first failure. Every step still goes through the same
    /// permission and confirmation checks as a single call. Element ids must come from the latest result;
    /// if a step changes the UI so that later ids go stale, use computer_find between batches instead.
    pub steps: Vec<BatchStep>,
    /// Default true: append the final screen to the reply.
    #[serde(default)]
    pub observe: Option<bool>,
}

/// 一次工具呼叫。MCP server 與 Ask AI 都先轉成它，再交給 [`execute`]。
pub enum Call {
    ListApps,
    Snapshot(ComputerSnapshotArgs),
    Find(ComputerFindArgs),
    Screenshot(ComputerScreenshotArgs),
    Click(ComputerClickArgs),
    Type(ComputerTypeArgs),
    Key(ComputerKeyArgs),
    Scroll(ComputerScrollArgs),
    Batch(ComputerBatchArgs),
}

/// 動作成功後，依 `observe`（預設 true）附上畫面現況；失敗的動作不附。
async fn with_observation(
    service: &ComputerService,
    app: &str,
    observe: Option<bool>,
    r: Result<ToolOutput, String>,
) -> Result<ToolOutput, String> {
    match r {
        Ok(ToolOutput::Text(text)) if observe.unwrap_or(true) => match service.observe(app).await {
            Some(screen) => Ok(ToolOutput::Text(format!("{text}\n\n畫面現況：\n{screen}"))),
            None => Ok(ToolOutput::Text(text)),
        },
        other => other,
    }
}

fn first_line(out: &ToolOutput) -> String {
    match out {
        ToolOutput::Text(t) => t.lines().next().unwrap_or("").to_string(),
        ToolOutput::Image { text, .. } => text.clone(),
    }
}

fn need<T>(v: Option<T>, step: usize, field: &str, action: &str) -> Result<T, String> {
    v.ok_or_else(|| format!("步驟 {step}：「{action}」需要欄位 {field}"))
}

/// 批次執行前先驗證每一步的形狀：任何一步有問題，整批都不動手。
fn validate_batch(steps: &[BatchStep]) -> Result<(), String> {
    if steps.is_empty() {
        return Err("steps 至少要有一步".to_string());
    }
    if steps.len() > MAX_BATCH_STEPS {
        return Err(format!("一批最多 {MAX_BATCH_STEPS} 步，收到 {} 步；請拆成多批", steps.len()));
    }
    for (i, s) in steps.iter().enumerate() {
        let n = i + 1;
        match s.action.as_str() {
            "click" => {
                if s.element.is_none() && !(s.x.is_some() && s.y.is_some()) {
                    return Err(format!("步驟 {n}：「click」需要 element，或同時需要 x 與 y"));
                }
            }
            "type" => {
                need(s.text.as_ref(), n, "text", "type")?;
            }
            "key" => {
                need(s.key.as_ref(), n, "key", "key")?;
            }
            "scroll" => {}
            other => return Err(format!("步驟 {n}：未知的動作「{other}」（可用：click、type、key、scroll）")),
        }
    }
    Ok(())
}

async fn run_batch(service: &ComputerService, a: ComputerBatchArgs) -> Result<ToolOutput, String> {
    validate_batch(&a.steps)?;
    let total = a.steps.len();
    let mut lines: Vec<String> = Vec::new();
    for (i, s) in a.steps.iter().enumerate() {
        let n = i + 1;
        if i > 0 {
            let d = service.settle();
            if !d.is_zero() {
                tokio::time::sleep(d).await; // 讓上一步的 UI 反應完再做下一步
            }
        }
        let result = match s.action.as_str() {
            "click" => {
                let args = ComputerClickArgs {
                    app: a.app.clone(),
                    element: s.element.clone(),
                    x: s.x,
                    y: s.y,
                    button: s.button.clone(),
                    double: s.double,
                    observe: Some(false),
                };
                match args.into_parts() {
                    Ok((app, target, button, double)) => service.click(&app, target, button, double).await,
                    Err(e) => Err(e),
                }
            }
            "type" => {
                service
                    .type_text(&a.app, s.text.as_deref().unwrap_or(""), s.element.as_deref(), s.replace.unwrap_or(false))
                    .await
            }
            "key" => service.key(&a.app, s.key.as_deref().unwrap_or("")).await,
            _ => {
                let at = match (s.x, s.y) {
                    (Some(x), Some(y)) => Some((x, y)),
                    _ => None,
                };
                service.scroll(&a.app, s.dx.unwrap_or(0), s.dy.unwrap_or(0), at).await
            }
        };
        match result {
            Ok(out) => lines.push(format!("步驟 {n}/{total} {}：{}", s.action, first_line(&out))),
            Err(e) => {
                lines.push(format!("步驟 {n} 失敗（{}）：{e}", s.action));
                return Err(lines.join("\n"));
            }
        }
    }
    with_observation(service, &a.app, a.observe, Ok(ToolOutput::Text(lines.join("\n")))).await
}

/// 所有入口的唯一執行點：授權、危險確認、稽核都在 `ComputerService` 裡。
pub async fn execute(service: &ComputerService, call: Call) -> Result<ToolOutput, String> {
    match call {
        Call::ListApps => service.list_apps().await,
        Call::Snapshot(a) => service.snapshot_opts(&a.app, a.max_depth, a.compact.unwrap_or(true)).await,
        Call::Find(a) => {
            service
                .find(&a.app, a.query.as_deref().unwrap_or(""), a.role.as_deref(), a.max_results.unwrap_or(20))
                .await
        }
        Call::Screenshot(a) => service.screenshot(a.app.as_deref()).await,
        Call::Click(a) => {
            let observe = a.observe;
            let (app, target, button, double) = a.into_parts()?;
            let r = service.click(&app, target, button, double).await;
            with_observation(service, &app, observe, r).await
        }
        Call::Type(a) => {
            let r = service.type_text(&a.app, &a.text, a.element.as_deref(), a.replace.unwrap_or(false)).await;
            with_observation(service, &a.app, a.observe, r).await
        }
        Call::Key(a) => {
            let r = service.key(&a.app, &a.key).await;
            with_observation(service, &a.app, a.observe, r).await
        }
        Call::Scroll(a) => {
            let at = a.at();
            let r = service.scroll(&a.app, a.dx.unwrap_or(0), a.dy.unwrap_or(0), at).await;
            with_observation(service, &a.app, a.observe, r).await
        }
        Call::Batch(a) => run_batch(service, a).await,
    }
}

const NO_ARGS: &str = r#"{"type":"object","properties":{}}"#;

fn schema<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_else(|_| serde_json::from_str(NO_ARGS).unwrap())
}

/// 內建虛擬伺服器提供的工具（已套用 `aiterm_computer__` 前綴）。
pub fn tool_infos() -> Vec<McpToolInfo> {
    let mk = |name: &str, description: &str, input_schema: Value| McpToolInfo {
        server_id: BUILTIN_SERVER_ID.to_string(),
        server_name: BUILTIN_SERVER_NAME.to_string(),
        name: encode_tool_name(BUILTIN_SERVER_ID, name),
        description: description.to_string(),
        input_schema,
    };
    vec![
        mk(
            "computer_list_apps",
            "List the macOS apps with visible windows that can be controlled (name, bundle id, pid, frontmost). Call this first to get exact app names.",
            serde_json::from_str(NO_ARGS).unwrap(),
        ),
        mk(
            "computer_snapshot",
            "Read an app's UI as an accessibility tree: one line per element with an id (e.g. 0.3.1), role, title and frame. Prefer computer_find when you know what you are looking for. The user is asked to approve the first use of each app.",
            schema::<ComputerSnapshotArgs>(),
        ),
        mk(
            "computer_find",
            "Find elements in an app by text (title/role) or role and get their ids directly — much cheaper than a full snapshot. Example: query=\"search\", or role=\"AXTextField\". The returned ids can be used immediately with computer_click / computer_type.",
            schema::<ComputerFindArgs>(),
        ),
        mk(
            "computer_screenshot",
            "Take a screenshot of an app window. In this chat the image itself cannot be shown to you — only its size is returned — so prefer computer_find / computer_snapshot to see what is on screen.",
            schema::<ComputerScreenshotArgs>(),
        ),
        mk(
            "computer_click",
            "Click an element (by id from the latest snapshot/find/action result) or a screen coordinate. The reply includes the resulting screen, so you usually do not need another snapshot. Clicking something labelled like Delete/Send/Pay always asks the user for confirmation first.",
            schema::<ComputerClickArgs>(),
        ),
        mk(
            "computer_type",
            "Type text into an app. Pass an element id to focus a field first; otherwise the text goes to whatever has focus. Set replace=true to overwrite the field's existing text. The reply reports the field's current content and the resulting screen — check it.",
            schema::<ComputerTypeArgs>(),
        ),
        mk(
            "computer_key",
            "Press ONE key or combination in an app, e.g. \"return\", \"escape\", \"cmd+s\", \"cmd+a\". To clear a field send \"cmd+a\" then \"delete\" (a bare \"delete\" removes one character). The reply reports the field's current content and the resulting screen. Cmd+Delete always asks the user for confirmation.",
            schema::<ComputerKeyArgs>(),
        ),
        mk(
            "computer_scroll",
            "Scroll in an app by lines (dy positive = up, dx positive = right). The reply includes the resulting screen.",
            schema::<ComputerScrollArgs>(),
        ),
        mk(
            "computer_batch",
            "Run up to 10 click/type/key/scroll steps in order in ONE call and get the final screen back — use it when you already know the sequence (e.g. click the search box, type a name, press return). Stops at the first failure and reports progress. Every step still goes through the usual permission and confirmation checks.",
            schema::<ComputerBatchArgs>(),
        ),
    ]
}

/// 若 `encoded_name` 屬於內建虛擬伺服器，回傳原始工具名（`computer_click`…）。
pub fn builtin_tool_name(encoded_name: &str) -> Option<&str> {
    let (server, tool) = decode_tool_name(encoded_name)?;
    let wanted = encode_tool_name(BUILTIN_SERVER_ID, "");
    (format!("{server}__") == wanted).then_some(tool)
}

fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("參數錯誤：{e}"))
}

fn to_result(r: Result<ToolOutput, String>) -> McpToolResult {
    match r {
        Ok(ToolOutput::Text(content)) => McpToolResult { content, is_error: false },
        Ok(ToolOutput::Image { png, text }) => McpToolResult {
            content: format!(
                "{text}\n（影像 {} KB 已截取，但這個對話看不到影像；請改用 computer_find 或 computer_snapshot 讀畫面元件。）",
                png.len() / 1024
            ),
            is_error: false,
        },
        Err(content) => McpToolResult { content, is_error: true },
    }
}

/// Ask AI 的入口：把工具名與 JSON 參數轉成 [`Call`] 並執行。錯誤一律包成 `is_error` 的結果，
/// 讓模型讀得到原因（授權被拒、權限缺少、參數錯誤…）。
pub async fn call(service: &ComputerService, tool: &str, args: Value) -> McpToolResult {
    let r: Result<ToolOutput, String> = async {
        let call = match tool {
            "computer_list_apps" => Call::ListApps,
            "computer_snapshot" => Call::Snapshot(parse(args)?),
            "computer_find" => Call::Find(parse(args)?),
            "computer_screenshot" => Call::Screenshot(parse(args)?),
            "computer_click" => Call::Click(parse(args)?),
            "computer_type" => Call::Type(parse(args)?),
            "computer_key" => Call::Key(parse(args)?),
            "computer_scroll" => Call::Scroll(parse(args)?),
            "computer_batch" => Call::Batch(parse(args)?),
            other => return Err(format!("未知的電腦操作工具「{other}」")),
        };
        execute(service, call).await
    }
    .await;
    to_result(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_the_nine_tools_with_object_schemas_and_encoded_names() {
        let tools = tool_infos();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "aiterm_computer__computer_list_apps",
                "aiterm_computer__computer_snapshot",
                "aiterm_computer__computer_find",
                "aiterm_computer__computer_screenshot",
                "aiterm_computer__computer_click",
                "aiterm_computer__computer_type",
                "aiterm_computer__computer_key",
                "aiterm_computer__computer_scroll",
                "aiterm_computer__computer_batch",
            ]
        );
        for t in &tools {
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
            assert_eq!(t.server_id, BUILTIN_SERVER_ID);
        }
        let click = tools.iter().find(|t| t.name.ends_with("computer_click")).unwrap();
        assert!(click.input_schema["required"].as_array().unwrap().iter().any(|v| v == "app"));
        let batch = tools.iter().find(|t| t.name.ends_with("computer_batch")).unwrap();
        assert!(batch.input_schema["required"].as_array().unwrap().iter().any(|v| v == "steps"));
    }

    #[test]
    fn builtin_name_detection_matches_only_our_prefix() {
        assert_eq!(builtin_tool_name("aiterm_computer__computer_click"), Some("computer_click"));
        assert_eq!(builtin_tool_name("filesystem__read_file"), None);
        assert_eq!(builtin_tool_name("aiterm_computer_x__computer_click"), None);
        assert_eq!(builtin_tool_name("noseparator"), None);
    }

    #[test]
    fn click_parts_validate_target_and_button() {
        let ok = |v: Value| parse::<ComputerClickArgs>(v).unwrap().into_parts();
        let (app, t, b, d) = ok(serde_json::json!({"app":"Notes","element":"0.1","double":true})).unwrap();
        assert_eq!((app.as_str(), b, d), ("Notes", MouseButton::Left, true));
        assert!(matches!(t, ClickTarget::Element(ref id) if id == "0.1"));
        assert!(matches!(ok(serde_json::json!({"app":"Notes","x":1,"y":2})).unwrap().1, ClickTarget::Point(..)));
        assert!(ok(serde_json::json!({"app":"Notes","x":1})).is_err());
        assert!(ok(serde_json::json!({"app":"Notes","element":"0","button":"middle"})).is_err());
    }

    #[test]
    fn image_output_becomes_text_the_chat_can_carry() {
        let r = to_result(Ok(ToolOutput::Image { png: vec![0; 4096], text: "截圖 10x10".into() }));
        assert!(!r.is_error);
        assert!(r.content.contains("截圖 10x10") && r.content.contains("computer_snapshot"));
        let e = to_result(Err("使用者拒絕".into()));
        assert!(e.is_error && e.content == "使用者拒絕");
    }

    #[test]
    fn scroll_position_needs_both_coordinates() {
        let a: ComputerScrollArgs = parse(serde_json::json!({"app":"x","x":5})).unwrap();
        assert_eq!(a.at(), None);
        let b: ComputerScrollArgs = parse(serde_json::json!({"app":"x","x":5,"y":6})).unwrap();
        assert_eq!(b.at(), Some((5.0, 6.0)));
    }

    #[test]
    fn the_prompt_hint_mentions_every_efficiency_tool() {
        for w in ["computer_find", "computer_batch", "replace=true", "cmd+a"] {
            assert!(PROMPT_HINT.contains(w), "{w}");
        }
    }
}
