//! 電腦操作工具的「定義與分派」，與傳輸方式無關：
//! MCP server（外部客戶端）與內建 Ask AI（虛擬 MCP 伺服器，不經 HTTP）共用。
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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerSnapshotArgs {
    /// App name or bundle id, as returned by `computer_list_apps`.
    pub app: String,
    /// How many levels of the accessibility tree to read (default 6, max 12).
    #[serde(default)]
    pub max_depth: Option<usize>,
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
    /// Element id from the latest `computer_snapshot` (e.g. "0.3.1"). Preferred over x/y.
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
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComputerKeyArgs {
    /// App name or bundle id.
    pub app: String,
    /// Key or combo, e.g. "return", "cmd+s", "cmd+shift+left".
    pub key: String,
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
}

impl ComputerScrollArgs {
    pub fn at(&self) -> Option<(f64, f64)> {
        match (self.x, self.y) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        }
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
            "Read an app's UI as an accessibility tree: one line per element with an id (e.g. 0.3.1), role, title and frame. Use the ids with computer_click / computer_type. Re-run it after the screen changes. The user is asked to approve the first use of each app.",
            schema::<ComputerSnapshotArgs>(),
        ),
        mk(
            "computer_screenshot",
            "Take a screenshot of an app window. In this chat the image itself cannot be shown to you — only its size is returned — so prefer computer_snapshot to find things on screen.",
            schema::<ComputerScreenshotArgs>(),
        ),
        mk(
            "computer_click",
            "Click an element (by id from the latest computer_snapshot) or a screen coordinate. Clicking something labelled like Delete/Send/Pay always asks the user for confirmation first.",
            schema::<ComputerClickArgs>(),
        ),
        mk(
            "computer_type",
            "Type text into an app. Pass an element id (from computer_snapshot) to focus a field first; otherwise the text goes to whatever has focus.",
            schema::<ComputerTypeArgs>(),
        ),
        mk(
            "computer_key",
            "Press a key or combination in an app, e.g. \"return\", \"escape\", \"cmd+s\". Cmd+Delete always asks the user for confirmation.",
            schema::<ComputerKeyArgs>(),
        ),
        mk(
            "computer_scroll",
            "Scroll in an app by lines (dy positive = up, dx positive = right).",
            schema::<ComputerScrollArgs>(),
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
                "{text}\n（影像 {} KB 已截取，但這個對話看不到影像；請改用 computer_snapshot 讀畫面元件。）",
                png.len() / 1024
            ),
            is_error: false,
        },
        Err(content) => McpToolResult { content, is_error: true },
    }
}

/// 執行內建工具。錯誤一律包成 `is_error` 的結果，讓模型讀得到原因（授權被拒、權限缺少…）。
pub async fn call(service: &ComputerService, tool: &str, args: Value) -> McpToolResult {
    let r: Result<ToolOutput, String> = async {
        match tool {
            "computer_list_apps" => service.list_apps().await,
            "computer_snapshot" => {
                let a: ComputerSnapshotArgs = parse(args)?;
                service.snapshot(&a.app, a.max_depth).await
            }
            "computer_screenshot" => {
                let a: ComputerScreenshotArgs = parse(args)?;
                service.screenshot(a.app.as_deref()).await
            }
            "computer_click" => {
                let (app, target, button, double) = parse::<ComputerClickArgs>(args)?.into_parts()?;
                service.click(&app, target, button, double).await
            }
            "computer_type" => {
                let a: ComputerTypeArgs = parse(args)?;
                service.type_text(&a.app, &a.text, a.element.as_deref()).await
            }
            "computer_key" => {
                let a: ComputerKeyArgs = parse(args)?;
                service.key(&a.app, &a.key).await
            }
            "computer_scroll" => {
                let a: ComputerScrollArgs = parse(args)?;
                let at = a.at();
                service.scroll(&a.app, a.dx.unwrap_or(0), a.dy.unwrap_or(0), at).await
            }
            other => Err(format!("未知的電腦操作工具「{other}」")),
        }
    }
    .await;
    to_result(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_the_seven_tools_with_object_schemas_and_encoded_names() {
        let tools = tool_infos();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "aiterm_computer__computer_list_apps",
                "aiterm_computer__computer_snapshot",
                "aiterm_computer__computer_screenshot",
                "aiterm_computer__computer_click",
                "aiterm_computer__computer_type",
                "aiterm_computer__computer_key",
                "aiterm_computer__computer_scroll",
            ]
        );
        for t in &tools {
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
            assert_eq!(t.server_id, BUILTIN_SERVER_ID);
        }
        // 必填欄位有出現在 schema 裡，模型才知道要帶 app。
        let click = tools.iter().find(|t| t.name.ends_with("computer_click")).unwrap();
        assert!(click.input_schema["required"].as_array().unwrap().iter().any(|v| v == "app"));
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
}
