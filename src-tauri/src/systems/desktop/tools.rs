//! 桌面 App 連線對外的 `sys_app_*` 工具：由電腦操作的 `computer_*` 工具改造而來——
//! 拿掉 `app` 參數（由連線綁定，AI 不能指定別的 App），加上必填的 `system` 與 `intent`。

use serde_json::{json, Value};

/// 對外開放的工具（`sys_app_` 後面的名字）→ 對應的電腦操作工具。
/// 不給：open_app／open_url（會切到別的 App）、batch（逐步判定與暫停都靠單步）、icon 類（無法判斷用途）。
const MAPPED: &[&str] = &["snapshot", "find", "read_screen", "screenshot", "click", "click_text", "type", "key", "scroll"];

pub const TOOL_LOGIN: &str = "sys_app_login";
pub const TOOL_NEEDS_LOGIN: &str = "sys_app_report_login_needed";

pub fn is_desktop_tool(name: &str) -> bool {
    name.starts_with("sys_app_")
}

/// `sys_app_click` → `computer_click`；不認得（或不開放）回 `None`。
pub fn computer_name(sys_name: &str) -> Option<String> {
    let short = sys_name.strip_prefix("sys_app_")?;
    MAPPED.contains(&short).then(|| format!("computer_{short}"))
}

fn system_props() -> Value {
    json!({
        "system": { "type": "string", "description": "系統連線的 id（先用 sys_list 查）" },
        "intent": { "type": "string", "description": "這個動作要做什麼，以「讀：」或「寫：」開頭。會改資料、送出、刪除、付款的動作一定要寫「寫：」" }
    })
}

/// 把電腦操作工具的 schema 改成桌面系統版：去掉 `app`、加 `system`／`intent`。
fn adapt_schema(mut schema: Value) -> Value {
    if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        props.remove("app");
        props.remove("observe");
        for (k, v) in system_props().as_object().cloned().unwrap_or_default() {
            props.insert(k, v);
        }
    } else {
        schema = json!({ "type": "object", "properties": system_props() });
    }
    let mut required: Vec<Value> = schema["required"]
        .as_array()
        .map(|a| a.iter().filter(|v| v.as_str() != Some("app")).cloned().collect())
        .unwrap_or_default();
    for k in ["system", "intent"] {
        required.push(json!(k));
    }
    schema["required"] = Value::Array(required);
    schema
}

/// 桌面連線可用的全部工具定義（MCP `tools/list` 的元素）。
pub fn defs() -> Vec<Value> {
    let mut out: Vec<Value> = crate::computer::tools::tool_infos()
        .into_iter()
        .filter_map(|t| {
            let (_, short) = crate::mcp::types::decode_tool_name(&t.name)?;
            let sys = short.strip_prefix("computer_")?;
            MAPPED.contains(&sys).then(|| {
                json!({
                    "name": format!("sys_app_{sys}"),
                    "description": format!("{}（只作用在這個系統綁定的桌面 App。）", t.description),
                    "inputSchema": adapt_schema(t.input_schema),
                })
            })
        })
        .collect();
    let sys = system_props();
    out.push(json!({
        "name": TOOL_LOGIN,
        "description": "App 停在登入畫面時用：指出帳號欄位與密碼欄位的元件 id（來自 sys_app_snapshot／sys_app_find），\
                        由 AITerm 從鑰匙串取出帳密填進去，你看不到密碼。password_element 必須是密碼型欄位，否則不會輸入任何內容。\
                        submit=\"return\" 會在填完後按 Enter 送出。沒有存帳密或需要驗證碼時，改用 sys_app_report_login_needed。",
        "inputSchema": { "type": "object", "properties": {
            "system": sys["system"], "intent": sys["intent"],
            "username_element": { "type": "string", "description": "帳號欄位的元件 id" },
            "password_element": { "type": "string", "description": "密碼欄位的元件 id" },
            "submit": { "type": "string", "enum": ["return"], "description": "填完後是否按 Enter 送出；省略＝不送" }
        }, "required": ["system", "intent", "username_element", "password_element"] }
    }));
    out.push(json!({
        "name": TOOL_NEEDS_LOGIN,
        "description": "App 需要使用者自己登入（沒存帳密、要驗證碼、二階段驗證、帳密被拒）時呼叫。\
                        呼叫後就停下來，用一兩句話回報；不要嘗試自己輸入帳密。",
        "inputSchema": { "type": "object", "properties": {
            "system": sys["system"], "intent": sys["intent"],
            "reason": { "type": "string", "description": "為什麼需要使用者登入" }
        }, "required": ["system", "intent"] }
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_name(name: &str) -> Value {
        defs().into_iter().find(|d| d["name"] == name).unwrap_or_else(|| panic!("缺 {name}"))
    }

    #[test]
    fn exposes_the_mapped_tools_plus_login_and_needs_login() {
        let names: Vec<String> = defs().iter().map(|d| d["name"].as_str().unwrap().to_string()).collect();
        for n in ["snapshot", "find", "read_screen", "screenshot", "click", "click_text", "type", "key", "scroll"] {
            assert!(names.contains(&format!("sys_app_{n}")), "{n}");
        }
        assert!(names.contains(&TOOL_LOGIN.to_string()) && names.contains(&TOOL_NEEDS_LOGIN.to_string()));
    }

    #[test]
    fn never_exposes_tools_that_leave_the_bound_app() {
        let names: Vec<String> = defs().iter().map(|d| d["name"].as_str().unwrap().to_string()).collect();
        for banned in ["open_app", "open_url", "batch", "list_apps", "click_icon", "find_icons"] {
            assert!(!names.iter().any(|n| n.ends_with(banned)), "{banned}");
        }
    }

    #[test]
    fn schemas_drop_app_and_require_system_and_intent() {
        for d in defs() {
            let props = d["inputSchema"]["properties"].as_object().unwrap();
            assert!(!props.contains_key("app"), "{}", d["name"]);
            assert!(props.contains_key("system") && props.contains_key("intent"), "{}", d["name"]);
            let req: Vec<&str> = d["inputSchema"]["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            assert!(req.contains(&"system") && req.contains(&"intent") && !req.contains(&"app"), "{}", d["name"]);
        }
        let click = by_name("sys_app_click");
        assert!(click["inputSchema"]["properties"].get("element").is_some(), "原本的參數要保留");
    }

    #[test]
    fn name_mapping_is_exact() {
        assert_eq!(computer_name("sys_app_click").as_deref(), Some("computer_click"));
        assert_eq!(computer_name("sys_app_read_screen").as_deref(), Some("computer_read_screen"));
        assert_eq!(computer_name("sys_app_open_app"), None);
        assert_eq!(computer_name("sys_click"), None);
        assert!(is_desktop_tool("sys_app_login") && !is_desktop_tool("sys_click"));
    }
}
