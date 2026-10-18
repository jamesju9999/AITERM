//! Agent 卡改用 Codex 做事時的專屬部分：組互動式 `codex`（TUI）指令、認信任畫面、解析
//! Codex 的對話記錄（rollout JSONL）。見 docs/superpowers/specs/2026-10-17-agent-codex-design.md；
//! 每一條行為都是實測過的（0.148.0 exec、0.163.0 TUI；記憶 project_codex_dispatch_probe）。
//!
//! 指令是打進使用者 shell 的一行字。`-c` 的值用單引號包住、裡面的 TOML 字串用雙引號：
//! zsh／bash／fish／PowerShell 7 都照字面傳。值裡出現單引號就放棄（回 `None`）。
//! PowerShell 5.1 傳給外部程式時會吃掉內層雙引號——Windows 未實機驗證。

use std::path::Path;

use crate::tasks::usage::TokenUsage;

/// Playwright MCP 的版本跟 Claude 卡片用的一樣（見 `tasks::ask`）。
pub struct CodexLaunch<'a> {
    pub model: Option<&'a str>,
    /// 卡片內容寫在這個檔案裡；指令只叫 Codex 去讀它（避免多行內容在各 shell 的引號問題）。
    pub prompt_file: &'a Path,
    /// 有值 = `codex resume <id>` 接著做。
    pub resume_session: Option<&'a str>,
    /// SessionStart hook 要執行的指令（`hook_command` 的結果）。
    pub hook_command: Option<&'a str>,
    /// ask_human 的 MCP 端點。
    pub ask_url: Option<&'a str>,
    /// 驗收卡的 Playwright MCP 參數（`npx` 之後那串）。
    pub playwright_args: Option<&'a [String]>,
    /// 精簡啟動：要關掉的使用者 MCP 伺服器名稱。
    pub disable_mcp: &'a [String],
}

/// 「Codex 環境變數」存在 keychain 的 key（內容是多行 `KEY=VALUE`）。
pub const CODEX_ENV_KEY: &str = "task_board.codex_env";

/// Codex 的設定資料夾：`$CODEX_HOME` 或 `~/.codex`。
pub fn codex_home() -> Option<std::path::PathBuf> {
    match std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        Some(v) => Some(std::path::PathBuf::from(v)),
        None => dirs::home_dir().map(|h| h.join(".codex")),
    }
}

/// 等人回答 ask_human 可能很久：工具逾時拉到一天。
pub const ASK_TOOL_TIMEOUT_SECS: u64 = 86_400;

/// TOML 的基本字串：`"..."`，跳脫 `\` 與 `"`。
pub fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// MCP 伺服器名稱只收英數、`_`、`-`：它是 `-c` 路徑的一段，其他字元要加引號才合法。
fn plain_key(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 模型名稱的白名單同 Claude 卡片的 `--model`（`scheduler::model_flag`）。
fn safe_model(m: &str) -> bool {
    !m.is_empty() && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '[' | ']' | ':' | '/'))
}

/// `-c '<key>=<value>'`；值裡有單引號就 `None`。
fn config_arg(key: &str, value: &str) -> Option<String> {
    let kv = format!("{key}={value}");
    (!kv.contains('\'')).then(|| format!("-c '{kv}'"))
}

/// 整行互動式 `codex ...`（TUI，跟 Claude 卡片一樣看得到過程、可以插話）。
/// 第一句提示從參數帶入就開工；做完由 `CodexCloser` 送 `/exit` 收掉。
/// 任何一段無法安全放進 shell 就回 `None`（呼叫端把派工標成失敗並說明）。
pub fn launch_command(o: &CodexLaunch) -> Option<String> {
    let mut parts: Vec<String> = vec!["codex".into()];
    if let Some(sid) = o.resume_session {
        if !plain_key(sid) {
            return None;
        }
        parts.push("resume".into());
        parts.push(sid.into());
    }
    // 互動版的 `codex` 與 `codex resume` 都收 -s／-a（0.163.0 實測）。
    // `-a never`：沒有人在旁邊按核准；做不到的事模型自己會回報。
    parts.push("-s workspace-write -a never".into());
    // 一律走 OpenAI 官方（用使用者 `codex login` 的帳號：ChatGPT 或 API key），
    // 不吃 config.toml 的預設供應商——那可能是本機模型伺服器。
    parts.push(config_arg("model_provider", &toml_str("openai"))?);
    parts.push(config_arg("sandbox_workspace_write.network_access", "true")?);
    if let Some(m) = o.model.map(str::trim).filter(|m| safe_model(m)) {
        parts.push(format!("-m {m}"));
    }
    if let Some(hook) = o.hook_command {
        parts.push("--dangerously-bypass-hook-trust".into());
        let v = format!("[{{hooks=[{{type=\"command\",command={}}}]}}]", toml_str(hook));
        parts.push(config_arg("hooks.SessionStart", &v)?);
    }
    if let Some(url) = o.ask_url {
        parts.push(config_arg("mcp_servers.aiterm.url", &toml_str(url))?);
        parts.push(config_arg("mcp_servers.aiterm.default_tools_approval_mode", &toml_str("approve"))?);
        parts.push(config_arg("mcp_servers.aiterm.tool_timeout_sec", &ASK_TOOL_TIMEOUT_SECS.to_string())?);
    }
    if let Some(args) = o.playwright_args {
        let arr = format!("[{}]", args.iter().map(|a| toml_str(a)).collect::<Vec<_>>().join(","));
        parts.push(config_arg("mcp_servers.playwright.command", &toml_str("npx"))?);
        parts.push(config_arg("mcp_servers.playwright.args", &arr)?);
        parts.push(config_arg("mcp_servers.playwright.default_tools_approval_mode", &toml_str("approve"))?);
    }
    for name in o.disable_mcp.iter().filter(|n| plain_key(n)) {
        parts.push(config_arg(&format!("mcp_servers.{name}.enabled"), "false")?);
    }
    let file = o.prompt_file.to_str()?;
    let prompt = format!("請讀 {file} 並照裡面的指示完成工作。");
    if prompt.contains('\'') {
        return None;
    }
    parts.push(format!("'{prompt}'"));
    Some(parts.join(" "))
}

/// SessionStart hook：把 Codex 給的 JSON（含 session_id、transcript_path）原封不動 POST 回 AITerm。
/// 刻意不含任何引號（它會被放進 TOML 字串、再放進 shell 單引號裡）。
pub fn hook_command(base: &str) -> String {
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    format!("{curl} -s -m 5 -X POST -H Content-Type:application/json --data-binary @- {base}/codex-session")
}

/// rollout 裡最後一筆有內容的 `token_count`：`total_token_usage` 是累計值。
/// `input_tokens` 已含 `cached_input_tokens`（實測），拆成「輸入／快取讀取」兩項。
pub fn parse_usage(text: &str) -> Option<TokenUsage> {
    let mut last = None;
    let mut calls = 0;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else { continue };
        if v.pointer("/payload/type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }
        let Some(total) = v.pointer("/payload/info/total_token_usage") else { continue };
        calls += 1;
        last = Some(total.clone());
    }
    let t = last?;
    let n = |k: &str| t.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let cached = n("cached_input_tokens");
    Some(TokenUsage {
        input: n("input_tokens").saturating_sub(cached),
        cache_write: n("cache_write_input_tokens"),
        cache_read: cached,
        output: n("output_tokens"),
        calls,
    })
}

/// rollout 裡 `task_complete` 的筆數：每一輪結束（成功或出錯）各一筆。
/// TUI 做完不退出、出錯也不觸發 Stop hook，所以「這一輪結束了」只看這個（0.163.0 實測）。
pub fn completed_turns(text: &str) -> usize {
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
        .filter(|v| v.pointer("/payload/type").and_then(|t| t.as_str()) == Some("task_complete"))
        .count()
}

/// 畫面停在 Codex 的資料夾信任提示、而且游標在「Trust and continue」上時回傳 `true`
/// （呼叫端送一個 Enter）。`screen` 是去掉 ANSI 的輸出；游標定位碼被剝掉後各行會黏在一起，
/// 所以拿掉所有空白再比對，並且只看最後一次出現提示之後的部分。
///
/// 信任紀錄寫在**主程式庫**（不是各個 worktree），所以同一個專案只會問一次（實測）。
/// 游標不在「Trust」上（例如選項被改過順序）就什麼都不送——寧可卡住，也不要按到 Quit。
pub fn trust_prompt_pending(screen: &str) -> bool {
    let flat: String = screen.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(at) = flat.rfind("Trustthisfolder?") else { return false };
    let tail = &flat[at..];
    tail.contains("\u{203a}1.Trustandcontinue")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexError {
    /// 額度用完（`codex_error_info == "usage_limit_exceeded"`）；字串是 Codex 的訊息（含恢復時間）。
    UsageLimit(String),
    Other(String),
}

/// rollout 最後一筆 `task_complete` 帶的錯誤。出錯與額度用完都**不會**觸發 Stop hook，只能從這裡判斷
/// （TUI 下結束碼也是 0——`/exit` 是我們送的）。
pub fn last_error(text: &str) -> Option<CodexError> {
    let mut out = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else { continue };
        if v.pointer("/payload/type").and_then(|t| t.as_str()) != Some("task_complete") {
            continue;
        }
        out = v.pointer("/payload/error").filter(|e| !e.is_null()).map(|e| {
            let msg = e.get("message").and_then(|m| m.as_str()).unwrap_or("Codex 回報錯誤").to_string();
            match e.get("codex_error_info").and_then(|i| i.as_str()) {
                Some("usage_limit_exceeded") => CodexError::UsageLimit(msg),
                _ => CodexError::Other(msg),
            }
        });
    }
    out
}

/// 使用者 `~/.codex/config.toml` 裡的 MCP 伺服器名稱（精簡啟動時逐一關掉）。
pub fn user_mcp_servers(config_toml: &str) -> Vec<String> {
    let Ok(v) = config_toml.parse::<toml::Table>() else { return Vec::new() };
    v.get("mcp_servers")
        .and_then(|s| s.as_table())
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

/// Codex 模型清單裡的一個模型（設定對話框的下拉選單用）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CodexModel {
    pub slug: String,
    pub name: String,
}

/// `codex debug models` 的 JSON：只留 `visibility == "list"` 的（Codex 自己的 /model 選單也只列這些）。
pub fn parse_models(json: &str) -> Vec<CodexModel> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let list = v.get("models").unwrap_or(&v);
    list.as_array()
        .into_iter()
        .flatten()
        .filter(|m| m.get("visibility").and_then(|x| x.as_str()) == Some("list"))
        .filter_map(|m| {
            let slug = m.get("slug").and_then(|x| x.as_str())?.to_string();
            let name = m.get("display_name").and_then(|x| x.as_str()).unwrap_or(&slug).to_string();
            Some(CodexModel { slug, name })
        })
        .collect()
}

/// 卡片記的對話記錄路徑是不是 Codex 的 rollout（不是 Claude 的 JSONL）。
pub fn is_rollout_path(p: &str) -> bool {
    Path::new(p)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base<'a>(file: &'a Path) -> CodexLaunch<'a> {
        CodexLaunch {
            model: None,
            prompt_file: file,
            resume_session: None,
            hook_command: None,
            ask_url: None,
            playwright_args: None,
            disable_mcp: &[],
        }
    }

    #[test]
    fn a_plain_card_opens_the_tui_on_openai_with_write_access_and_network() {
        let f = Path::new("/p/tasks/t1/prompt.md");
        let line = launch_command(&CodexLaunch { model: Some("gpt-6-luna"), ..base(f) }).unwrap();
        assert_eq!(
            line,
            "codex -s workspace-write -a never -c 'model_provider=\"openai\"' -c 'sandbox_workspace_write.network_access=true' \
             -m gpt-6-luna '請讀 /p/tasks/t1/prompt.md 並照裡面的指示完成工作。'"
        );
    }

    #[test]
    fn resuming_puts_the_session_right_after_resume() {
        let f = Path::new("/p/prompt.md");
        let line = launch_command(&CodexLaunch { resume_session: Some("01a14a6c-f298-70c0"), ..base(f) }).unwrap();
        assert!(line.starts_with("codex resume 01a14a6c-f298-70c0 -s workspace-write -a never "), "{line}");
        assert!(launch_command(&CodexLaunch { resume_session: Some("x; rm -rf /"), ..base(f) }).is_none());
    }

    #[test]
    fn ask_human_is_approved_and_may_wait_a_day() {
        let f = Path::new("/p/prompt.md");
        let line = launch_command(&CodexLaunch { ask_url: Some("http://127.0.0.1:9/mcp/s/t"), ..base(f) }).unwrap();
        assert!(line.contains("-c 'mcp_servers.aiterm.url=\"http://127.0.0.1:9/mcp/s/t\"'"), "{line}");
        assert!(line.contains("-c 'mcp_servers.aiterm.default_tools_approval_mode=\"approve\"'"), "{line}");
        assert!(line.contains("-c 'mcp_servers.aiterm.tool_timeout_sec=86400'"), "{line}");
    }

    #[test]
    fn the_session_hook_trusts_itself_and_posts_back() {
        let f = Path::new("/p/prompt.md");
        let hook = hook_command("http://127.0.0.1:9/quota/s/t");
        assert!(!hook.contains('"') && !hook.contains('\''), "hook 指令不能有引號：{hook}");
        let line = launch_command(&CodexLaunch { hook_command: Some(&hook), ..base(f) }).unwrap();
        assert!(line.contains("--dangerously-bypass-hook-trust"), "{line}");
        assert!(
            line.contains(&format!("-c 'hooks.SessionStart=[{{hooks=[{{type=\"command\",command=\"{hook}\"}}]}}]'")),
            "{line}"
        );
        assert!(hook.ends_with("http://127.0.0.1:9/quota/s/t/codex-session"));
    }

    #[test]
    fn the_verifier_gets_playwright_and_lean_start_turns_off_user_servers() {
        let f = Path::new("/p/prompt.md");
        let args = vec!["-y".to_string(), "@playwright/mcp@0.0.83".to_string(), "--headless".to_string()];
        let off = vec!["pencil".to_string(), "bad name".to_string()];
        let line = launch_command(&CodexLaunch { playwright_args: Some(&args), disable_mcp: &off, ..base(f) }).unwrap();
        assert!(line.contains("-c 'mcp_servers.playwright.args=[\"-y\",\"@playwright/mcp@0.0.83\",\"--headless\"]'"), "{line}");
        assert!(line.contains("-c 'mcp_servers.playwright.default_tools_approval_mode=\"approve\"'"), "{line}");
        assert!(line.contains("-c 'mcp_servers.pencil.enabled=false'"), "{line}");
        assert!(!line.contains("bad name"), "名稱有空白的不能放進設定路徑：{line}");
    }

    #[test]
    fn unsafe_pieces_are_left_out_or_refused() {
        let f = Path::new("/p/prompt.md");
        let line = launch_command(&CodexLaunch { model: Some("gpt;rm -rf"), ..base(f) }).unwrap();
        assert!(!line.contains(" -m "), "不合法的模型名不加：{line}");
        assert!(launch_command(&base(Path::new("/p/it's/prompt.md"))).is_none(), "路徑有單引號就不派");
        assert_eq!(toml_str(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    // 下面兩份是探勘時真實的 rollout 行（codex-cli 0.148.0），只截掉無關欄位。
    const OK_ROLLOUT: &str = r#"{"type":"session_meta","payload":{"id":"x"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":21018,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":28,"reasoning_output_tokens":0,"total_tokens":21046}},"rate_limits":{"limit_id":"codex"}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":42121,"cached_input_tokens":21011,"cache_write_input_tokens":0,"output_tokens":29,"reasoning_output_tokens":1,"total_tokens":42150}},"rate_limits":{"limit_id":"codex"}}}
{"type":"event_msg","payload":{"type":"task_complete","turn_id":"t","last_agent_message":"DONE"}}"#;

    const LIMIT_ROLLOUT: &str = r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"limit_id":"premium","rate_limit_reached_type":null}}}
{"type":"event_msg","payload":{"type":"task_complete","turn_id":"t","last_agent_message":null,"error":{"message":"You've hit your usage limit. Upgrade to Plus to continue using Codex (https://chatgpt.com/explore/plus), or try again at Nov 10th, 2026 7:04 PM.","codex_error_info":"usage_limit_exceeded"}}}"#;

    #[test]
    fn usage_is_the_last_cumulative_count_with_cached_input_split_out() {
        let u = parse_usage(OK_ROLLOUT).unwrap();
        assert_eq!(u, TokenUsage { input: 42121 - 21011, cache_write: 0, cache_read: 21011, output: 29, calls: 2 });
        assert_eq!(parse_usage(LIMIT_ROLLOUT), None, "沒有任何用量就是 None");
    }

    #[test]
    fn a_usage_limit_is_told_apart_from_other_errors() {
        assert_eq!(last_error(OK_ROLLOUT), None);
        match last_error(LIMIT_ROLLOUT) {
            Some(CodexError::UsageLimit(m)) => assert!(m.contains("Nov 10th"), "{m}"),
            other => panic!("{other:?}"),
        }
        let other = r#"{"type":"event_msg","payload":{"type":"task_complete","error":{"message":"Model 'x' not found"}}}"#;
        assert_eq!(last_error(other), Some(CodexError::Other("Model 'x' not found".into())));
    }

    #[test]
    fn every_turn_end_counts_whether_it_succeeded_or_failed() {
        assert_eq!(completed_turns(OK_ROLLOUT), 1);
        assert_eq!(completed_turns(&format!("{OK_ROLLOUT}\n{LIMIT_ROLLOUT}")), 2);
        assert_eq!(completed_turns("{\"type\":\"session_meta\",\"payload\":{}}\nnot json"), 0);
    }

    // 0.163.0 的信任畫面經 strip_ansi 之後的樣子（游標定位碼剝掉、各行黏在一起）。
    const TRUST_SCREEN: &str = "\u{203a} Ask Codex to do anything ? for shortcuts /p/r1 Folder access /p/r1 \
        Trust this folder? Codex can read, edit, and run files here, subject to your permission settings. \
        Your trust decision will be saved.\u{203a} 1. Trust and continue                    2. Quitenter continue \u{b7} esc quit";

    #[test]
    fn the_trust_screen_is_confirmed_only_when_the_cursor_is_on_trust() {
        assert!(trust_prompt_pending(TRUST_SCREEN));
        let on_quit = TRUST_SCREEN.replace("\u{203a} 1. Trust", "1. Trust").replace("2. Quit", "\u{203a} 2. Quit");
        assert!(!trust_prompt_pending(&on_quit), "游標在 Quit 上不能按 Enter");
        assert!(!trust_prompt_pending("\u{203a} Ask Codex to do anything"));
        // 提到這句話但沒有選項（例如模型的回覆）不算。
        assert!(!trust_prompt_pending("I saw a \"Trust this folder?\" prompt earlier."));
    }

    #[test]
    fn user_servers_come_from_the_mcp_servers_table() {
        let cfg = "model = \"m\"\n[mcp_servers.pencil]\ncommand = \"/x\"\n[mcp_servers.node_repl]\nargs = []\n[mcp_servers.node_repl.env]\nA = \"1\"\n";
        let mut names = user_mcp_servers(cfg);
        names.sort();
        assert_eq!(names, vec!["node_repl", "pencil"]);
        assert!(user_mcp_servers("not = [toml").is_empty());
    }

    #[test]
    fn the_model_list_keeps_only_listed_models() {
        // 0.163.0 `codex debug models -c model_provider="openai"` 的形狀（截掉無關欄位）。
        let json = r#"{"models":[
            {"slug":"gpt-6-luna","display_name":"GPT-6-Luna","visibility":"list"},
            {"slug":"gpt-reserve","display_name":"GPT-Reserve","visibility":"hide"},
            {"slug":"gpt-5.6-terra","visibility":"list"}]}"#;
        assert_eq!(
            parse_models(json),
            vec![
                CodexModel { slug: "gpt-6-luna".into(), name: "GPT-6-Luna".into() },
                CodexModel { slug: "gpt-5.6-terra".into(), name: "gpt-5.6-terra".into() },
            ]
        );
        assert!(parse_models("not json").is_empty());
    }

    #[test]
    fn rollout_paths_are_recognised() {
        assert!(is_rollout_path("/Users/a/.codex/sessions/2026/10/17/rollout-2026-10-17T23-13-36-01a1.jsonl"));
        assert!(!is_rollout_path("/p/tasks/t/session.jsonl"));
    }
}
