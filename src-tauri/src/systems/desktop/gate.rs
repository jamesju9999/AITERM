//! 桌面 App 的寫入閘門（動作層）。網頁系統靠網路層攔截；桌面沒有那一層，
//! 所以由「AI 宣告的意圖」與「動作對象的標題／按鍵」判定這個動作算不算寫入，再依模式處理。

use crate::computer::backend::KeyCombo;
use crate::computer::policy;
use crate::systems::gate::glob_match;
use crate::systems::types::{Decision, DenyReason, Mode, RuleAction, SystemConnection};

/// 閘門看的動作。`label` 是動作對象的描述（元件標題、要點的文字、按鍵字串）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action<'a> {
    /// `click`／`type`／`key`／`scroll`／`login`／`read`（純觀察）。
    pub kind: &'a str,
    pub label: &'a str,
}

/// 判定結果：處置，以及「AI 宣告讀、實際被升級成寫入」（核准卡要標警示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub decision: Decision,
    pub is_write: bool,
    pub declared_read: bool,
}

/// intent 以「寫」開頭（全形或半形冒號都可）才算宣告寫入。
pub fn declares_write(intent: &str) -> bool {
    intent.trim_start().starts_with('寫') || intent.trim_start().to_ascii_lowercase().starts_with("write")
}

/// 這個動作本身是不是寫入：宣告寫入、危險字樣的元件、危險按鍵、送出類按鍵、登入。
pub fn is_write(a: &Action, declared_write: bool) -> bool {
    if declared_write {
        return true;
    }
    match a.kind {
        "login" => true,
        "read" | "scroll" => false,
        "key" => {
            let combo: Option<KeyCombo> = policy::parse_key(a.label).ok();
            match combo {
                Some(c) => policy::is_dangerous_key(&c) || matches!(c.key.as_str(), "return" | "enter"),
                None => true, // 解析不了的按鍵寧可當成寫入
            }
        }
        "type" => policy::is_dangerous_label(a.label),
        _ => policy::is_dangerous_label(a.label),
    }
}

pub fn decide(conn: &SystemConnection, a: &Action, declared_write: bool) -> Verdict {
    if !conn.enabled {
        return Verdict { decision: Decision::Deny(DenyReason::Disabled), is_write: false, declared_read: false };
    }
    let write = is_write(a, declared_write);
    let declared_read = write && !declared_write;
    if !write {
        return Verdict { decision: Decision::Allow, is_write: false, declared_read: false };
    }
    let decision = match conn.mode {
        Mode::ReadOnly => Decision::Deny(DenyReason::ReadOnly),
        Mode::ApproveEach => Decision::Ask,
        Mode::Autonomous => Decision::Allow,
        Mode::Rules => conn
            .rules
            .iter()
            .find(|r| (r.method == "*" || r.method.eq_ignore_ascii_case(a.kind)) && glob_match(&r.url_pattern, a.label))
            .map(|r| match r.action {
                RuleAction::Allow => Decision::Allow,
                RuleAction::Ask => Decision::Ask,
            })
            .unwrap_or(Decision::Ask),
    };
    Verdict { decision, is_write: true, declared_read }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems::types::{Rule, SystemKind};

    fn conn(mode: Mode) -> SystemConnection {
        SystemConnection { id: "d".into(), name: "D".into(), kind: SystemKind::Desktop, app: "Notes".into(), mode, ..Default::default() }
    }
    fn act<'a>(kind: &'a str, label: &'a str) -> Action<'a> {
        Action { kind, label }
    }

    #[test]
    fn plain_reads_pass_in_every_mode() {
        for mode in [Mode::ReadOnly, Mode::ApproveEach, Mode::Rules, Mode::Autonomous] {
            let v = decide(&conn(mode), &act("click", "查詢"), false);
            assert_eq!(v.decision, Decision::Allow, "{mode:?}");
            assert!(!v.is_write);
        }
    }

    #[test]
    fn declared_write_follows_the_mode() {
        let a = act("type", "備註欄");
        assert_eq!(decide(&conn(Mode::ReadOnly), &a, true).decision, Decision::Deny(DenyReason::ReadOnly));
        assert_eq!(decide(&conn(Mode::ApproveEach), &a, true).decision, Decision::Ask);
        assert_eq!(decide(&conn(Mode::Autonomous), &a, true).decision, Decision::Allow);
        assert_eq!(decide(&conn(Mode::Rules), &a, true).decision, Decision::Ask, "沒命中規則的一律問人");
    }

    #[test]
    fn dangerous_label_upgrades_a_declared_read() {
        let v = decide(&conn(Mode::ReadOnly), &act("click", "刪除"), false);
        assert_eq!(v.decision, Decision::Deny(DenyReason::ReadOnly));
        assert!(v.is_write && v.declared_read, "宣告讀卻被升級成寫入，核准卡要標警示");
        let v = decide(&conn(Mode::ApproveEach), &act("click", "Submit"), false);
        assert_eq!((v.decision, v.declared_read), (Decision::Ask, true));
    }

    #[test]
    fn declared_write_is_not_flagged_as_declared_read() {
        let v = decide(&conn(Mode::ApproveEach), &act("click", "刪除"), true);
        assert!(v.is_write && !v.declared_read);
    }

    #[test]
    fn enter_and_dangerous_keys_count_as_writes_but_navigation_keys_do_not() {
        assert!(decide(&conn(Mode::ReadOnly), &act("key", "return"), false).is_write);
        assert!(decide(&conn(Mode::ReadOnly), &act("key", "enter"), false).is_write);
        assert!(decide(&conn(Mode::ReadOnly), &act("key", "cmd+delete"), false).is_write);
        assert!(decide(&conn(Mode::ReadOnly), &act("key", "???"), false).is_write, "解析不了的按鍵寧可當寫入");
        for k in ["tab", "down", "pagedown", "cmd+f"] {
            assert!(!decide(&conn(Mode::ReadOnly), &act("key", k), false).is_write, "{k}");
        }
    }

    #[test]
    fn login_is_always_a_write() {
        let v = decide(&conn(Mode::ReadOnly), &act("login", "登入"), false);
        assert_eq!(v.decision, Decision::Deny(DenyReason::ReadOnly));
        assert_eq!(decide(&conn(Mode::Autonomous), &act("login", "登入"), false).decision, Decision::Allow);
    }

    #[test]
    fn rules_match_action_and_label_first_hit_wins() {
        let mut c = conn(Mode::Rules);
        c.rules = vec![
            Rule { method: "type".into(), url_pattern: "備註*".into(), action: RuleAction::Allow },
            Rule { method: "*".into(), url_pattern: "*".into(), action: RuleAction::Ask },
        ];
        assert_eq!(decide(&c, &act("type", "備註欄"), true).decision, Decision::Allow);
        assert_eq!(decide(&c, &act("type", "金額"), true).decision, Decision::Ask);
        assert_eq!(decide(&c, &act("click", "備註欄"), true).decision, Decision::Ask, "動作不同不命中第一條");
    }

    #[test]
    fn a_disabled_connection_denies_even_reads() {
        let mut c = conn(Mode::Autonomous);
        c.enabled = false;
        assert_eq!(decide(&c, &act("read", ""), false).decision, Decision::Deny(DenyReason::Disabled));
    }

    #[test]
    fn intent_prefix_detection() {
        assert!(declares_write("寫：送出表單"));
        assert!(declares_write("  寫:改單價"));
        assert!(declares_write("write: update"));
        assert!(!declares_write("讀：看清單"));
        assert!(!declares_write(""));
    }
}
