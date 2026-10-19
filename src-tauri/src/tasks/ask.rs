//! 每張派出去的卡專用的極小 MCP 端點（streamable HTTP，只回 JSON），只有一個工具
//! `ask_human`。路由與狀態在 `tasks::quota` 的本機接收器上。見
//! docs/superpowers/specs/2026-10-16-agent-ask-human-design.md（含實測過的握手流程）。

use serde_json::{json, Value};

use crate::tasks::questions::NewQuestion;

pub const TOOL_NAME: &str = "ask_human";
/// 派工時要放行的完整工具名稱（`mcp__<伺服器>__<工具>`）。
pub const ALLOWED_TOOL: &str = "mcp__aiterm__ask_human";

/// 一則 JSON-RPC 訊息要怎麼處理。
#[derive(Debug, PartialEq)]
pub enum Rpc {
    /// 通知（沒有 id）：回 202，不用內容。
    Notification,
    /// 直接回這個 JSON-RPC 回應。
    Respond(Value),
    /// 呼叫 `ask_human`：由接收器建立問題後用 `tool_result` 回覆。
    Ask { id: Value, question: NewQuestion },
    /// 資料庫技能的工具（只有裝了技能的卡才會出現）：由接收器檢查權限、執行、記錄。
    Db { id: Value, call: DbCall },
}

/// 資料庫技能的一次工具呼叫，見 docs/superpowers/specs/2026-10-19-agent-db-skill-design.md。
#[derive(Debug, Clone, PartialEq)]
pub enum DbCall {
    ListConnections,
    ListSchemas { connection_id: String },
    ListTables { connection_id: String, schema: String },
    TableSchema { connection_id: String, schema: String, table: String },
    Query { connection_id: String, sql: String },
}

/// 派工時要放行的資料庫工具（`--allowedTools`）。
pub const DB_ALLOWED_TOOLS: &str = "mcp__aiterm__db_list_connections mcp__aiterm__db_list_schemas \
mcp__aiterm__db_list_tables mcp__aiterm__db_table_schema mcp__aiterm__db_query";

fn db_tool(name: &str, description: &str, props: &[(&str, &str)]) -> Value {
    let properties: serde_json::Map<String, Value> =
        props.iter().map(|(k, d)| (k.to_string(), json!({ "type": "string", "description": d }))).collect();
    let required: Vec<&str> = props.iter().map(|(k, _)| *k).collect();
    json!({ "name": name, "description": description, "inputSchema": { "type": "object", "properties": properties, "required": required } })
}

pub fn db_tool_definitions() -> Vec<Value> {
    let conn = ("connection_id", "連線 id（db_list_connections 列出的 id）");
    vec![
        db_tool("db_list_connections", "列出開放給你的資料庫連線（id、名稱、類型、資料庫、權限）。先呼叫這個。", &[]),
        db_tool("db_list_schemas", "列出連線上的 schema。", &[conn]),
        db_tool("db_list_tables", "列出 schema 裡的資料表與檢視。", &[conn, ("schema", "schema 名稱")]),
        db_tool("db_table_schema", "列出資料表的欄位（名稱、型別、可否為空、預設值）。", &[conn, ("schema", "schema 名稱"), ("table", "資料表名稱")]),
        db_tool(
            "db_query",
            "在連線上執行一句 SQL，回傳 JSON 結果。權限依連線設定：唯讀只能跑 SELECT／WITH／SHOW／DESCRIBE／EXPLAIN；\
需核准的寫入會等使用者按核准；被拒絕時不要換個寫法硬做，改在待確認事項說明。一次只送一句。",
            &[conn, ("sql", "一句 SQL")],
        ),
    ]
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn parse_db_call(name: &str, args: &Value) -> Option<Result<DbCall, String>> {
    let need = |key: &str| str_arg(args, key).ok_or_else(|| format!("{name} 需要 {key}"));
    let call = match name {
        "db_list_connections" => Ok(DbCall::ListConnections),
        "db_list_schemas" => need("connection_id").map(|connection_id| DbCall::ListSchemas { connection_id }),
        "db_list_tables" => (|| Ok(DbCall::ListTables { connection_id: need("connection_id")?, schema: need("schema")? }))(),
        "db_table_schema" => (|| {
            Ok(DbCall::TableSchema { connection_id: need("connection_id")?, schema: need("schema")?, table: need("table")? })
        })(),
        "db_query" => (|| Ok(DbCall::Query { connection_id: need("connection_id")?, sql: need("sql")? }))(),
        _ => return None,
    };
    Some(call)
}

pub fn tool_definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "向負責的人提出需要人決定的問題（需求不清、取捨會影響成果時）。送出後立刻回傳；\
請接著結束這一輪回覆等待答案，答案會以新訊息送來。不要自己先做這個決定。小細節不要問，自己採合理做法並記錄。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "要人決定的問題，一句話講清楚" },
                "options": { "type": "array", "items": { "type": "string" }, "description": "可選的答案（最多 6 個）" },
                "recommended": { "type": "string", "description": "你建議的選項（要跟 options 其中一個完全相同）" },
                "context": { "type": "string", "description": "為什麼要問、各選項的影響" }
            },
            "required": ["question"]
        }
    })
}

fn ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// 工具呼叫的結果。
pub fn tool_result(id: &Value, text: &str, is_error: bool) -> Value {
    ok(id, json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
}

pub fn classify(msg: &Value) -> Rpc {
    classify_with(msg, false)
}

/// `db`：這張卡的 Agent 有沒有資料庫技能（有才列出、才接受 db 工具）。
pub fn classify_with(msg: &Value, db: bool) -> Rpc {
    let Some(id) = msg.get("id").cloned() else { return Rpc::Notification };
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let version = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18");
            Rpc::Respond(ok(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "aiterm", "version": env!("CARGO_PKG_VERSION") }
                }),
            ))
        }
        // 新版協定的探測：回空結果，Claude Code 就改走 initialize（實測）。
        "server/discover" | "ping" => Rpc::Respond(ok(&id, json!({}))),
        "tools/list" => {
            let mut tools = vec![tool_definition()];
            if db {
                tools.extend(db_tool_definitions());
            }
            Rpc::Respond(ok(&id, json!({ "tools": tools })))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if db {
                if let Some(call) = parse_db_call(name, params.get("arguments").unwrap_or(&Value::Null)) {
                    return match call {
                        Ok(call) => Rpc::Db { id, call },
                        Err(msg) => Rpc::Respond(tool_result(&id, &msg, true)),
                    };
                }
            }
            if name != TOOL_NAME {
                return Rpc::Respond(tool_result(&id, &format!("沒有這個工具：{name}"), true));
            }
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match serde_json::from_value::<NewQuestion>(args) {
                Ok(q) if !q.question.trim().is_empty() => Rpc::Ask { id, question: q },
                _ => Rpc::Respond(tool_result(&id, "ask_human 需要 question（要人決定的問題）", true)),
            }
        }
        _ => Rpc::Respond(err(&id, -32601, &format!("不支援的方法：{method}"))),
    }
}

/// 派工時給 claude 的 `--mcp-config` 檔案內容。
pub fn mcp_config_json(url: &str) -> String {
    mcp_config_json_with(Some(url), None).unwrap_or_default()
}

/// 驗收者用的瀏覽器工具（Playwright MCP）。版本釘死：工具參數在版本之間會改
/// （實測 `browser_click` 的參數已改名成 `target`），不釘的話某天會突然壞掉。
pub const PLAYWRIGHT_MCP: &str = "@playwright/mcp@0.0.83";
/// `--allowedTools` 裡代表整個 playwright 伺服器的工具。
pub const BROWSER_TOOLS: &str = "mcp__playwright";

/// 驗收者的瀏覽器：有裝 Chrome 就用它（不必下載）；沒有就用 Playwright 的
/// Chrome for Testing，第一次用要先下載（約 100MB，裝在使用者自己的快取，不需要
/// 管理員權限；實測同一個 session 裡裝好就能直接用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    SystemChrome,
    Bundled,
}

/// 驗收者沒有瀏覽器時要執行的安裝指令（寫進提示）。
pub const BROWSER_INSTALL: &str = "npx -y @playwright/mcp@0.0.83 install-browser chrome-for-testing";

/// 這台電腦有沒有裝 Google Chrome（Playwright 的 `chrome` 通道找的位置）。
pub fn detect_browser() -> Browser {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        candidates.push("/Applications/Google Chrome.app".into());
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Applications/Google Chrome.app"));
        }
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(var) {
                candidates.push(std::path::Path::new(&base).join("Google/Chrome/Application/chrome.exe"));
            }
        }
    } else {
        candidates.push("/opt/google/chrome/chrome".into());
    }
    if candidates.iter().any(|p| p.exists()) {
        Browser::SystemChrome
    } else {
        Browser::Bundled
    }
}

/// `--mcp-config` 的內容：`ask_url` 有值就放 ask_human；`browser_out` 有值就放
/// 無頭、獨立設定檔的瀏覽器，截圖等輸出寫在那個資料夾（它只准寫那裡與工作目錄）。
/// 兩個都沒有就是 `None`。
pub fn mcp_config_json_with(ask_url: Option<&str>, browser_out: Option<&std::path::Path>) -> Option<String> {
    mcp_config_json_for(ask_url, browser_out, Browser::SystemChrome)
}

/// Playwright MCP 的啟動參數（`npx` 之後那串）；Claude 與 Codex 的驗收卡共用。
pub fn playwright_args(out: &std::path::Path, browser: Browser) -> Vec<String> {
    let mut args = vec![
        "-y".to_string(),
        PLAYWRIGHT_MCP.into(),
        "--headless".into(),
        "--isolated".into(),
        "--output-dir".into(),
        out.to_string_lossy().into_owned(),
    ];
    if browser == Browser::Bundled {
        args.extend(["--browser".to_string(), "chromium".to_string()]);
    }
    args
}

pub fn mcp_config_json_for(ask_url: Option<&str>, browser_out: Option<&std::path::Path>, browser: Browser) -> Option<String> {
    let mut servers = serde_json::Map::new();
    if let Some(url) = ask_url {
        servers.insert("aiterm".into(), json!({ "type": "http", "url": url }));
    }
    if let Some(out) = browser_out {
        servers.insert("playwright".into(), json!({ "command": "npx", "args": playwright_args(out, browser) }));
    }
    (!servers.is_empty()).then(|| json!({ "mcpServers": servers }).to_string())
}

/// `--allowedTools` 的值；兩個都沒有就是 `None`。
pub fn allowed_tools(ask: bool, browser: bool) -> Option<String> {
    let tools: Vec<&str> = [(ask, ALLOWED_TOOL), (browser, BROWSER_TOOLS)].iter().filter(|(on, _)| *on).map(|(_, t)| *t).collect();
    (!tools.is_empty()).then(|| tools.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_tools_are_listed_only_for_cards_with_the_skill() {
        let list = |db: bool| {
            let Rpc::Respond(v) = classify_with(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}), db) else { panic!() };
            v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        };
        assert_eq!(list(false), vec!["ask_human"]);
        assert_eq!(list(true), vec!["ask_human", "db_list_connections", "db_list_schemas", "db_list_tables", "db_table_schema", "db_query"]);
    }

    #[test]
    fn db_calls_are_parsed_and_refused_without_the_skill() {
        let call = |name: &str, args: Value| json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":name,"arguments":args}});
        assert_eq!(
            classify_with(&call("db_query", json!({"connection_id":"dev","sql":"SELECT 1"})), true),
            Rpc::Db { id: json!(7), call: DbCall::Query { connection_id: "dev".into(), sql: "SELECT 1".into() } }
        );
        assert_eq!(classify_with(&call("db_list_connections", json!({})), true), Rpc::Db { id: json!(7), call: DbCall::ListConnections });
        assert_eq!(
            classify_with(&call("db_table_schema", json!({"connection_id":"dev","schema":"main","table":"t"})), true),
            Rpc::Db { id: json!(7), call: DbCall::TableSchema { connection_id: "dev".into(), schema: "main".into(), table: "t".into() } }
        );
        let Rpc::Respond(missing) = classify_with(&call("db_query", json!({"connection_id":"dev"})), true) else { panic!() };
        assert_eq!(missing["result"]["isError"], true, "缺 sql");
        let Rpc::Respond(off) = classify_with(&call("db_query", json!({"connection_id":"dev","sql":"SELECT 1"})), false) else { panic!() };
        assert_eq!(off["result"]["isError"], true, "沒裝技能就沒有這個工具");
    }

    #[test]
    fn the_verifier_gets_a_pinned_headless_isolated_browser() {
        let out = std::path::Path::new("/p/tasks/v/evidence");
        let v: Value = serde_json::from_str(&mcp_config_json_with(Some("http://x/mcp"), Some(out)).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["aiterm"]["url"], "http://x/mcp");
        assert_eq!(v["mcpServers"]["playwright"]["command"], "npx");
        assert_eq!(
            v["mcpServers"]["playwright"]["args"],
            json!(["-y", "@playwright/mcp@0.0.83", "--headless", "--isolated", "--output-dir", "/p/tasks/v/evidence"])
        );
        let browser_only: Value = serde_json::from_str(&mcp_config_json_with(None, Some(out)).unwrap()).unwrap();
        assert!(browser_only["mcpServers"].get("aiterm").is_none());
        assert_eq!(mcp_config_json_with(None, None), None);
        // 其他卡片的設定跟原本一模一樣。
        assert_eq!(mcp_config_json("http://x/mcp"), r#"{"mcpServers":{"aiterm":{"type":"http","url":"http://x/mcp"}}}"#);
    }

    #[test]
    fn without_chrome_the_verifier_uses_the_bundled_browser() {
        let out = std::path::Path::new("/e");
        let v: Value = serde_json::from_str(&mcp_config_json_for(None, Some(out), Browser::Bundled).unwrap()).unwrap();
        let args = v["mcpServers"]["playwright"]["args"].as_array().unwrap();
        assert_eq!(&args[args.len() - 2..], &[json!("--browser"), json!("chromium")]);
        assert!(BROWSER_INSTALL.contains(PLAYWRIGHT_MCP), "安裝指令與 MCP 同一個版本");
    }

    #[test]
    fn allowed_tools_for_each_combination() {
        assert_eq!(allowed_tools(true, false).as_deref(), Some("mcp__aiterm__ask_human"));
        assert_eq!(allowed_tools(true, true).as_deref(), Some("mcp__aiterm__ask_human mcp__playwright"));
        assert_eq!(allowed_tools(false, true).as_deref(), Some("mcp__playwright"));
        assert_eq!(allowed_tools(false, false), None);
    }

    #[test]
    fn handshake_and_tool_listing_follow_what_claude_code_sends() {
        // 實測：claude 2.1.300 依序送 server/discover、initialize、notifications/initialized、tools/list。
        assert_eq!(classify(&json!({"jsonrpc":"2.0","id":"p","method":"server/discover"})), Rpc::Respond(json!({"jsonrpc":"2.0","id":"p","result":{}})));
        let Rpc::Respond(init) = classify(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25"}})) else { panic!() };
        assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        assert_eq!(classify(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})), Rpc::Notification);
        let Rpc::Respond(list) = classify(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})) else { panic!() };
        assert_eq!(list["result"]["tools"][0]["name"], TOOL_NAME);
        assert_eq!(list["result"]["tools"][0]["inputSchema"]["required"][0], "question");
        let Rpc::Respond(e) = classify(&json!({"jsonrpc":"2.0","id":2,"method":"resources/list"})) else { panic!() };
        assert_eq!(e["error"]["code"], -32601);
    }

    #[test]
    fn ask_human_calls_carry_the_question() {
        let call = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ask_human","arguments":{
            "question":"名稱用中文？","options":["中文","英文"],"recommended":"英文","context":"README 標題"}}});
        match classify(&call) {
            Rpc::Ask { id, question } => {
                assert_eq!(id, json!(3));
                assert_eq!(question.question, "名稱用中文？");
                assert_eq!(question.options, vec!["中文", "英文"]);
                assert_eq!(question.recommended.as_deref(), Some("英文"));
            }
            other => panic!("{other:?}"),
        }
        let Rpc::Respond(bad) = classify(&json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ask_human","arguments":{"question":"  "}}})) else { panic!() };
        assert_eq!(bad["result"]["isError"], true);
        let Rpc::Respond(other) = classify(&json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"rm","arguments":{}}})) else { panic!() };
        assert_eq!(other["result"]["isError"], true);
    }

    #[test]
    fn mcp_config_points_claude_at_the_http_endpoint() {
        let v: Value = serde_json::from_str(&mcp_config_json("http://127.0.0.1:9/mcp/s/t")).unwrap();
        assert_eq!(v["mcpServers"]["aiterm"]["type"], "http");
        assert_eq!(v["mcpServers"]["aiterm"]["url"], "http://127.0.0.1:9/mcp/s/t");
        assert!(ALLOWED_TOOL.ends_with(TOOL_NAME) && ALLOWED_TOOL.contains("aiterm"));
    }
}
