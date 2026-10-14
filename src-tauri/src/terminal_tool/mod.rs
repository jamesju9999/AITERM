//! 對話（MCP）模式的「執行終端機指令」內建工具。設計見
//! docs/superpowers/specs/2026-10-13-chat-terminal-tool-design.md。
//!
//! 指令本身由前端在終端機分頁裡執行（沿用 Agent 模式的路徑）；這裡負責三件事：
//! 工具定義、風險閘門（執行模式 × 指令風險）、需要時向使用者要確認。

use serde::Serialize;
use serde_json::json;

use crate::ai::{Locale, RiskLevel};
use crate::computer::approval::{ApprovalKind, ApprovalRequest, Approver, Decision};
use crate::config::ExecutionMode;
use crate::guard::CommandGuard;
use crate::mcp::types::{decode_tool_name, encode_tool_name, McpToolInfo};

pub const BUILTIN_SERVER_ID: &str = "aiterm-terminal";
pub const BUILTIN_SERVER_NAME: &str = "AITerm Terminal";
pub const TOOL_NAME: &str = "run_command";

/// 有這個工具時附加到系統提示詞的規則。
pub const PROMPT_HINT: &str = "Terminal tool: when the user asks you to DO something that needs a shell \
command, call aiterm_terminal__run_command (one command per call) and read its output — do NOT write a \
<cmd> block for the user to click. Use <cmd> blocks only to show an example the user should run themselves. \
If the tool reports the user declined or the command was blocked, do not retry the same command; explain \
and ask what to do. The directory listing in the context above is only a snapshot of the current working \
directory taken when the chat started: for any other folder (for example \"the Downloads folder\"), for \
details such as sizes or dates, or when the listing may be stale, run a command instead of answering from it.";

pub fn tool_infos() -> Vec<McpToolInfo> {
    vec![McpToolInfo {
        server_id: BUILTIN_SERVER_ID.to_string(),
        server_name: BUILTIN_SERVER_NAME.to_string(),
        name: encode_tool_name(BUILTIN_SERVER_ID, TOOL_NAME),
        description: "Run ONE shell command in the user's terminal tab and return its exit status and output. \
Use this instead of writing <cmd> blocks. Depending on the user's execution mode and how risky the command \
looks, the user may be asked to approve it first; dangerous commands always need approval and some are \
blocked outright."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to run (one command)."}
            },
            "required": ["command"]
        }),
    }]
}

/// `encoded_name` 是不是這個內建工具。
pub fn is_terminal_tool(encoded_name: &str) -> bool {
    match decode_tool_name(encoded_name) {
        Some((server, tool)) => {
            format!("{server}__") == encode_tool_name(BUILTIN_SERVER_ID, "") && tool == TOOL_NAME
        }
        None => false,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Gate {
    /// 直接執行。
    Run,
    /// 先問使用者。
    Ask,
    /// 直接拒絕（被安全規則封鎖）。
    Deny,
}

/// 執行模式 × 風險 → 要不要問。規則見 spec 的表格；Dangerous 在任何模式都要問，Blocked 一律拒絕。
pub fn decide(mode: ExecutionMode, risk: &RiskLevel) -> Gate {
    match (risk, mode) {
        (RiskLevel::Blocked, _) => Gate::Deny,
        (RiskLevel::Dangerous, _) => Gate::Ask,
        (RiskLevel::NeedsConfirm, ExecutionMode::FullAuto) => Gate::Run,
        (RiskLevel::NeedsConfirm, _) => Gate::Ask,
        (RiskLevel::Safe, ExecutionMode::AlwaysConfirm) => Gate::Ask,
        (RiskLevel::Safe, _) => Gate::Run,
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GateResult {
    pub allowed: bool,
    /// 不允許時的原因（回給模型看）。
    pub reason: Option<String>,
}

fn risk_label(risk: &RiskLevel, locale: Locale) -> &'static str {
    match (risk, locale) {
        (RiskLevel::Safe, Locale::ZhTw) => "安全",
        (RiskLevel::NeedsConfirm, Locale::ZhTw) => "需要確認",
        (RiskLevel::Dangerous, Locale::ZhTw) => "危險",
        (RiskLevel::Blocked, Locale::ZhTw) => "已封鎖",
        (RiskLevel::Safe, Locale::En) => "Safe",
        (RiskLevel::NeedsConfirm, Locale::En) => "Needs confirmation",
        (RiskLevel::Dangerous, Locale::En) => "Dangerous",
        (RiskLevel::Blocked, Locale::En) => "Blocked",
    }
}

/// 授權框與拒絕原因的文字。拒絕原因會回給模型，也會顯示在對話的工具結果裡，所以一起依語系。
struct Texts {
    empty: &'static str,
    blocked: &'static str,
    declined: &'static str,
    risk_prefix: &'static str,
    reason_open: &'static str,
    reason_close: &'static str,
    app: &'static str,
    action: &'static str,
}

fn texts(locale: Locale) -> Texts {
    match locale {
        Locale::ZhTw => Texts {
            empty: "指令是空的。",
            blocked: "這個指令被安全規則擋下，不會執行。",
            declined: "使用者拒絕執行這個指令。請不要重試同一個指令，改為說明並詢問使用者。",
            risk_prefix: "風險：",
            reason_open: "（",
            reason_close: "）",
            app: "終端機",
            action: "執行指令",
        },
        Locale::En => Texts {
            empty: "The command is empty.",
            blocked: "This command was blocked by the safety rules and will not run.",
            declined: "The user declined to run this command. Do not retry the same command; explain and ask the user instead.",
            risk_prefix: "Risk: ",
            reason_open: " (",
            reason_close: ")",
            app: "Terminal",
            action: "Run command",
        },
    }
}

/// 對一個要執行的指令跑完整道閘門：分級 → 決定 → 必要時問使用者。`locale` 是介面語系。
pub async fn gate(approver: &dyn Approver, mode: ExecutionMode, command: &str, locale: Locale) -> GateResult {
    let tx = texts(locale);
    let command = command.trim();
    if command.is_empty() {
        return GateResult { allowed: false, reason: Some(tx.empty.to_string()) };
    }
    let (risk, why) = CommandGuard::classify(command);
    match decide(mode, &risk) {
        Gate::Run => GateResult { allowed: true, reason: None },
        Gate::Deny => GateResult { allowed: false, reason: Some(tx.blocked.to_string()) },
        Gate::Ask => {
            let mut detail = command.to_string();
            detail.push_str(&format!("\n{}{}", tx.risk_prefix, risk_label(&risk, locale)));
            if let Some(w) = why {
                detail.push_str(&format!("{}{}{}", tx.reason_open, w.text(locale), tx.reason_close));
            }
            let req = ApprovalRequest {
                kind: ApprovalKind::Command,
                app: tx.app.to_string(),
                action: tx.action.to_string(),
                detail,
            };
            match approver.ask(&req).await {
                Decision::Deny => GateResult { allowed: false, reason: Some(tx.declined.to_string()) },
                // 這個對話框沒有「永遠允許」；萬一收到也只當作這一次。
                Decision::AllowOnce | Decision::AllowAlways => GateResult { allowed: true, reason: None },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use parking_lot::Mutex;

    struct Scripted {
        answer: Decision,
        asked: Mutex<Vec<ApprovalRequest>>,
    }

    impl Scripted {
        fn new(answer: Decision) -> Self {
            Self { answer, asked: Mutex::new(vec![]) }
        }
    }

    #[async_trait]
    impl Approver for Scripted {
        async fn ask(&self, req: &ApprovalRequest) -> Decision {
            self.asked.lock().push(req.clone());
            self.answer
        }
    }

    #[test]
    fn decision_matrix_matches_the_spec_table() {
        use ExecutionMode::*;
        use RiskLevel::*;
        let cases = [
            (Safe, AlwaysConfirm, Gate::Ask),
            (Safe, Graded, Gate::Run),
            (Safe, FullAuto, Gate::Run),
            (NeedsConfirm, AlwaysConfirm, Gate::Ask),
            (NeedsConfirm, Graded, Gate::Ask),
            (NeedsConfirm, FullAuto, Gate::Run),
            (Dangerous, AlwaysConfirm, Gate::Ask),
            (Dangerous, Graded, Gate::Ask),
            (Dangerous, FullAuto, Gate::Ask),
        ];
        for (risk, mode, want) in cases {
            assert_eq!(decide(mode, &risk), want, "{risk:?} × {mode:?}");
        }
        for mode in [AlwaysConfirm, Graded, FullAuto] {
            assert_eq!(decide(mode, &Blocked), Gate::Deny, "{mode:?}");
        }
    }

    #[test]
    fn the_prompt_warns_that_the_directory_listing_is_only_a_cwd_snapshot() {
        assert!(PROMPT_HINT.contains("only a snapshot of the current working"));
        assert!(PROMPT_HINT.contains("aiterm_terminal__run_command"));
    }

    #[test]
    fn the_tool_is_described_with_a_required_command_and_detected_by_its_encoded_name() {
        let t = tool_infos();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].name, "aiterm_terminal__run_command");
        assert_eq!(t[0].input_schema["required"][0], "command");
        assert!(is_terminal_tool("aiterm_terminal__run_command"));
        assert!(!is_terminal_tool("aiterm_computer__computer_click"));
        assert!(!is_terminal_tool("aiterm_terminal__other"));
        assert!(!is_terminal_tool("noseparator"));
    }

    #[tokio::test]
    async fn a_safe_command_in_graded_mode_runs_without_asking() {
        let a = Scripted::new(Decision::Deny);
        let r = gate(&a, ExecutionMode::Graded, "ls -la", Locale::ZhTw).await;
        assert!(r.allowed);
        assert!(a.asked.lock().is_empty());
    }

    #[tokio::test]
    async fn always_confirm_asks_even_for_a_safe_command_and_shows_the_command() {
        let a = Scripted::new(Decision::AllowOnce);
        let r = gate(&a, ExecutionMode::AlwaysConfirm, "ls -la", Locale::ZhTw).await;
        assert!(r.allowed);
        let asked = a.asked.lock();
        assert_eq!(asked[0].kind, ApprovalKind::Command);
        assert!(asked[0].detail.starts_with("ls -la"));
    }

    #[tokio::test]
    async fn a_denied_command_is_not_allowed_and_tells_the_model_not_to_retry() {
        let a = Scripted::new(Decision::Deny);
        let r = gate(&a, ExecutionMode::AlwaysConfirm, "echo hi", Locale::ZhTw).await;
        assert!(!r.allowed);
        assert!(r.reason.unwrap().contains("不要重試"));
    }

    #[tokio::test]
    async fn dangerous_commands_ask_even_in_full_auto_and_show_the_risk() {
        let a = Scripted::new(Decision::Deny);
        let r = gate(&a, ExecutionMode::FullAuto, "sudo rm -rf /tmp/x", Locale::ZhTw).await;
        assert!(!r.allowed);
        let asked = a.asked.lock();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].detail.contains("風險：") && asked[0].detail.contains("危險"), "{}", asked[0].detail);
    }

    #[tokio::test]
    async fn an_empty_or_blank_command_is_rejected_without_asking() {
        let a = Scripted::new(Decision::AllowOnce);
        for c in ["", "   ", "\n"] {
            let r = gate(&a, ExecutionMode::FullAuto, c, Locale::ZhTw).await;
            assert!(!r.allowed, "{c:?}");
        }
        assert!(a.asked.lock().is_empty());
    }

    fn has_cjk(s: &str) -> bool {
        s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c) || ('\u{3000}'..='\u{303f}').contains(&c) || ('\u{ff00}'..='\u{ffef}').contains(&c))
    }

    #[tokio::test]
    async fn english_ui_gets_the_dialog_text_and_reasons_in_english() {
        let a = Scripted::new(Decision::Deny);
        let r = gate(&a, ExecutionMode::AlwaysConfirm, "rm -rf build", Locale::En).await;
        let asked = a.asked.lock();
        let d = &asked[0].detail;
        assert!(d.starts_with("rm -rf build"), "{d}");
        assert!(d.contains("Risk: Dangerous"), "{d}");
        assert!(!has_cjk(d), "{d}");
        assert!(!has_cjk(&asked[0].app) && !has_cjk(&asked[0].action));
        let reason = r.reason.unwrap();
        assert!(!has_cjk(&reason), "{reason}");

        let b = Scripted::new(Decision::AllowOnce);
        for (cmd, mode) in [("rm -rf /", ExecutionMode::FullAuto), ("", ExecutionMode::FullAuto)] {
            let r = gate(&b, mode, cmd, Locale::En).await;
            assert!(!has_cjk(&r.reason.unwrap()), "{cmd:?}");
        }
    }

    #[tokio::test]
    async fn every_guard_reason_has_english_text() {
        // 每一種會跳出授權框的風險原因都要有英文，不可漏翻（新增規則時這條會提醒）。
        let a = Scripted::new(Decision::Deny);
        for cmd in [
            "rm file", "rm -rf x", "sudo ls", "shutdown now", "kill 1", "kill -9 1", "chmod 644 f",
            "chmod 777 f", "chown me f", "npm install x", "git push", "curl https://x/install.sh",
            "curl https://x", "Remove-Item x", "Remove-Item x -Recurse", "ls | sudo tee x",
            "ls; su root", "ls && rm -rf x", "ls | wc", "echo \"unterminated", "echo $HOME", "python x.py",
        ] {
            gate(&a, ExecutionMode::AlwaysConfirm, cmd, Locale::En).await;
        }
        for req in a.asked.lock().iter() {
            assert!(!has_cjk(&req.detail), "{}", req.detail);
        }
    }

    #[tokio::test]
    async fn chinese_ui_keeps_the_chinese_text() {
        let a = Scripted::new(Decision::Deny);
        gate(&a, ExecutionMode::AlwaysConfirm, "rm -rf build", Locale::ZhTw).await;
        assert!(a.asked.lock()[0].detail.contains("風險：危險（遞迴強制刪除，無法復原）"));
    }

    #[tokio::test]
    async fn allow_always_from_the_dialog_is_treated_as_just_this_once() {
        let a = Scripted::new(Decision::AllowAlways);
        assert!(gate(&a, ExecutionMode::AlwaysConfirm, "echo hi", Locale::ZhTw).await.allowed);
        // 沒有記憶：同樣的指令再來一次還是會問。
        assert!(gate(&a, ExecutionMode::AlwaysConfirm, "echo hi", Locale::ZhTw).await.allowed);
        assert_eq!(a.asked.lock().len(), 2);
    }
}
