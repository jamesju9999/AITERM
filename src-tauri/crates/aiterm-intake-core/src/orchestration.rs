//! 共用看板的編排：Coder 做模組 → Reviewer 審查 → 不通過退回修正（最多 2 輪）→ 全部通過後待管理員核准合併。
//!
//! 這裡只有純函式：從卡片的資料算出「模組現在在哪一步」，以及各步驟給 Claude 的提示詞。
//! 不碰資料庫、不碰子行程，所以 server 的編排迴圈與測試都能直接用；每個 tick 重新算一次，
//! server 重啟也不會漏步驟（狀態都在卡片上）。
//!
//! 模組鏈是一條直線：模組（work）→ 審查（review_of = 模組）→ 修正（fix，review_of = 那張審查）
//! → 審查（review_of = 修正）→ …。
use crate::board_store::{BoardAgent, BoardCard, DONE};

/// 審查不通過最多退回修正幾輪；超過就停在「已用完修正次數」，等人處理。
pub const MAX_FIX_ROUNDS: usize = 2;

/// 模組目前的狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleState {
    /// 鏈尾的卡片還沒做完（規劃中、待派送或處理中）。
    Waiting,
    /// 鏈尾（模組或修正）做完了，還沒有審查卡：要建一張。`target` 是被審的那張卡。
    NeedsReview { target: String },
    /// 最後一張審查不通過、還有修正次數：要建一張修正卡。`round` 從 1 算起。
    NeedsFix { review: String, round: usize },
    /// 審查通過（或沒有 Reviewer）；`sha` 是通過的那版。
    Passed { sha: Option<String> },
    /// 審查不通過，而且修正次數用完了。
    Exhausted,
    /// 鏈裡有卡失敗、或審查的回答讀不懂：要人看。
    Stuck(String),
}

fn next_of<'a>(of: &BoardCard, chain: &'a [BoardCard]) -> Option<&'a BoardCard> {
    chain.iter().find(|c| c.review_of.as_deref() == Some(of.id.as_str()))
}

/// `chain` 是屬於這個模組的所有 review／fix 卡（`module_id` 指向它，順序不拘）。
pub fn module_state(module: &BoardCard, chain: &[BoardCard], has_reviewer: bool) -> ModuleState {
    if module.module_override {
        return ModuleState::Passed { sha: module.end_sha.clone() };
    }
    let mut tail = module;
    let mut fixes = 0usize;
    // 鏈的長度有上限，萬一資料壞了（出現環）也不會無限迴圈。
    for _ in 0..chain.len() + 1 {
        match next_of(tail, chain) {
            Some(n) => {
                if n.card_kind == "fix" {
                    fixes += 1;
                }
                tail = n;
            }
            None => break,
        }
    }
    if tail.status != DONE {
        return ModuleState::Waiting;
    }
    if tail.outcome.as_deref() != Some("success") {
        return ModuleState::Stuck(format!("「{}」失敗：{}", tail.title, tail.error_message.clone().unwrap_or_default()));
    }
    match tail.card_kind.as_str() {
        "review" => match tail.verdict.as_deref() {
            Some("pass") => {
                let reviewed = chain.iter().chain(std::iter::once(module)).find(|c| Some(c.id.as_str()) == tail.review_of.as_deref());
                ModuleState::Passed { sha: reviewed.and_then(|c| c.end_sha.clone()) }
            }
            Some("fail") if fixes >= MAX_FIX_ROUNDS => ModuleState::Exhausted,
            Some("fail") => ModuleState::NeedsFix { review: tail.id.clone(), round: fixes + 1 },
            _ => ModuleState::Stuck(format!("審查「{}」沒有結論", tail.title)),
        },
        _ if !has_reviewer => ModuleState::Passed { sha: tail.end_sha.clone() },
        _ => ModuleState::NeedsReview { target: tail.id.clone() },
    }
}

/// 專案裡要走編排的模組：指派給 Coder、而且是 work 卡（不含已封存）。
pub fn modules<'a>(cards: &'a [BoardCard], agents: &[BoardAgent]) -> Vec<&'a BoardCard> {
    cards
        .iter()
        .filter(|c| c.card_kind == "work" && c.archived_at.is_none() && c.agent_id.as_deref().is_some_and(|a| agents.iter().any(|g| g.id == a && g.kind == "coder")))
        .collect()
}

// ---- 審查結論 ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub pass: bool,
    pub reasons: Vec<String>,
}

/// 從 Reviewer 的回答取出 `{"verdict":"pass"|"fail","reasons":[...]}`。容忍前後文字與 ``` 圍欄。
/// 不通過卻沒給理由時補一句，退回修正的人才知道要改什麼。
pub fn parse_review(text: &str) -> Result<Verdict, String> {
    let (a, b) = (text.find('{'), text.rfind('}'));
    let (Some(a), Some(b)) = (a, b) else { return Err("審查的回答裡找不到 JSON".into()) };
    if b < a {
        return Err("審查的回答裡找不到 JSON".into());
    }
    let v: serde_json::Value = serde_json::from_str(&text[a..=b]).map_err(|e| format!("審查的 JSON 讀不懂：{e}"))?;
    let verdict = v["verdict"].as_str().map(|s| s.trim().to_lowercase());
    let pass = match verdict.as_deref() {
        Some("pass") => true,
        Some("fail") => false,
        _ => return Err("審查的 JSON 沒有 verdict（要是 pass 或 fail）".into()),
    };
    let mut reasons: Vec<String> = match &v["reasons"] {
        serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        serde_json::Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => vec![],
    };
    if !pass && reasons.is_empty() {
        reasons.push("審查不通過，但沒有說明理由".into());
    }
    Ok(Verdict { pass, reasons })
}

// ---- 提示詞 ----

/// 模組卡（或修正卡）給 Coder 的提示詞。
pub fn work_prompt(agent: &BoardAgent, card: &BoardCard) -> String {
    let mut s = String::new();
    if !agent.prompt_prefix.trim().is_empty() {
        s.push_str(agent.prompt_prefix.trim());
        s.push_str("\n\n");
    }
    s.push_str(&format!("# {}\n\n{}\n", card.title.trim(), card.body.trim()));
    s.push_str("\n完成後請簡短說明改了哪些檔案與重點。不要執行會對外連線的指令。\n");
    s
}

/// 審查卡給 Reviewer 的提示詞；`diff` 是被審那版相對於派工前的差異。
pub fn review_prompt(agent: &BoardAgent, module: &BoardCard, reviewed: &BoardCard, diff: &str, previous_reasons: &[String]) -> String {
    let mut s = String::new();
    if !agent.prompt_prefix.trim().is_empty() {
        s.push_str(agent.prompt_prefix.trim());
        s.push_str("\n\n");
    }
    s.push_str(&format!("你要審查模組「{}」的實作。\n\n## 需求\n{}\n", module.title.trim(), module.body.trim()));
    if reviewed.id != module.id {
        s.push_str(&format!("\n（這是第二次以上的審查，被審的是修正卡「{}」。）\n", reviewed.title.trim()));
    }
    if !previous_reasons.is_empty() {
        s.push_str("\n## 上一次審查提出的問題（確認是否已改好）\n");
        for r in previous_reasons {
            s.push_str(&format!("- {r}\n"));
        }
    }
    s.push_str("\n## 這次的改動（git diff）\n```diff\n");
    s.push_str(if diff.trim().is_empty() { "（沒有任何改動）" } else { diff });
    s.push_str("\n```\n\n你可以讀專案裡的檔案確認細節，但不要修改任何東西。\n");
    s.push_str("審查完最後只回一段 JSON，不要加其他說明：\n{\"verdict\":\"pass\" 或 \"fail\",\"reasons\":[\"不通過的具體理由，通過就留空陣列\"]}\n");
    s
}

/// 修正卡的內容：把審查的理由原樣交給 Coder。
pub fn fix_body(module: &BoardCard, reasons: &[String], round: usize) -> String {
    let mut s = format!("模組「{}」的審查不通過（第 {round} 次修正），請依下列問題修改：\n\n", module.title.trim());
    for r in reasons {
        s.push_str(&format!("- {r}\n"));
    }
    s.push_str(&format!("\n## 原本的需求\n{}\n", module.body.trim()));
    s
}

/// 審查卡的內容（給人看；實際的提示詞在執行時才帶 diff）。
pub fn review_card_body(module: &BoardCard, reviewed: &BoardCard) -> String {
    format!("審查模組「{}」{}。結論會寫在這張卡上。", module.title.trim(), if reviewed.id == module.id { "的實作".to_string() } else { format!("的修正（{}）", reviewed.title.trim()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, kind: &str, review_of: Option<&str>) -> BoardCard {
        BoardCard {
            id: id.into(), project_id: "p".into(), title: id.into(), body: String::new(), status: DONE.into(), sort_order: 0.0, label: None,
            parallel_ok: true, interactive: false, outcome: Some("success".into()), ai_summary: None, error_message: None, created_by: String::new(),
            created_at: 0, updated_at: 0, dispatched_at: None, finished_at: None, archived_at: None, claimed_by: None, claimed_at: None,
            claim_expires_at: None, version: 1, agent_id: Some("c1".into()), card_kind: kind.into(), module_id: Some("m".into()),
            review_of: review_of.map(str::to_string), verdict: None, review_reasons: None, depends_on: None, start_sha: None, end_sha: Some(format!("sha-{id}")),
            module_override: false, is_spec: false, from_spec: None,
        }
    }
    fn module() -> BoardCard {
        let mut m = card("m", "work", None);
        m.module_id = None;
        m
    }
    fn review(id: &str, of: &str, verdict: &str) -> BoardCard {
        let mut r = card(id, "review", Some(of));
        r.verdict = Some(verdict.into());
        r
    }

    #[test]
    fn a_module_goes_through_review_fix_review_and_stops_when_rounds_run_out() {
        let m = module();
        assert_eq!(module_state(&m, &[], true), ModuleState::NeedsReview { target: "m".into() });
        assert_eq!(module_state(&m, &[], false), ModuleState::Passed { sha: Some("sha-m".into()) }, "沒有 Reviewer 就直接算通過");

        let mut running = m.clone();
        running.status = "running".into();
        assert_eq!(module_state(&running, &[], true), ModuleState::Waiting);

        let r1 = review("r1", "m", "fail");
        assert_eq!(module_state(&m, &[r1.clone()], true), ModuleState::NeedsFix { review: "r1".into(), round: 1 });

        let f1 = card("f1", "fix", Some("r1"));
        assert_eq!(module_state(&m, &[r1.clone(), f1.clone()], true), ModuleState::NeedsReview { target: "f1".into() }, "修正做完要再審，審的是修正那版");
        let r2 = review("r2", "f1", "fail");
        assert_eq!(module_state(&m, &[r1.clone(), f1.clone(), r2.clone()], true), ModuleState::NeedsFix { review: "r2".into(), round: 2 });

        let f2 = card("f2", "fix", Some("r2"));
        let r3 = review("r3", "f2", "fail");
        assert_eq!(module_state(&m, &[r1, f1, r2, f2, r3], true), ModuleState::Exhausted, "第二輪修正之後還不過，不再退回");
    }

    #[test]
    fn a_pass_reports_the_sha_that_was_reviewed_and_order_of_the_chain_does_not_matter() {
        let m = module();
        let r1 = review("r1", "m", "fail");
        let f1 = card("f1", "fix", Some("r1"));
        let r2 = review("r2", "f1", "pass");
        let shuffled = [r2.clone(), f1.clone(), r1.clone()];
        assert_eq!(module_state(&m, &shuffled, true), ModuleState::Passed { sha: Some("sha-f1".into()) });
        let only = [review("r", "m", "pass")];
        assert_eq!(module_state(&m, &only, true), ModuleState::Passed { sha: Some("sha-m".into()) });
    }

    #[test]
    fn failures_and_unreadable_reviews_stop_the_chain_instead_of_looping() {
        let m = module();
        let mut failed = m.clone();
        failed.outcome = Some("failed".into());
        failed.error_message = Some("逾時".into());
        assert!(matches!(module_state(&failed, &[], true), ModuleState::Stuck(s) if s.contains("逾時")));
        let mut bad = review("r", "m", "pass");
        bad.verdict = None;
        assert!(matches!(module_state(&m, &[bad], true), ModuleState::Stuck(_)));
        let mut over = m.clone();
        over.module_override = true;
        assert_eq!(module_state(&over, &[review("r", "m", "fail")], true), ModuleState::Passed { sha: Some("sha-m".into()) }, "人工放行");
        // 資料壞成環也不會卡死。
        let a = card("a", "fix", Some("b"));
        let b = card("b", "fix", Some("a"));
        let _ = module_state(&m, &[a, b], true);
    }

    #[test]
    fn review_json_is_found_inside_chatter_and_fences_and_a_fail_always_carries_a_reason() {
        let v = parse_review("好的，審查完了：\n```json\n{\"verdict\":\"Fail\",\"reasons\":[\"沒有處理空輸入\",\" \"]}\n```\n").unwrap();
        assert_eq!(v, Verdict { pass: false, reasons: vec!["沒有處理空輸入".into()] });
        assert!(parse_review("{\"verdict\":\"pass\"}").unwrap().pass);
        assert_eq!(parse_review("{\"verdict\":\"fail\"}").unwrap().reasons, vec!["審查不通過，但沒有說明理由".to_string()]);
        assert_eq!(parse_review("{\"verdict\":\"fail\",\"reasons\":\"太慢\"}").unwrap().reasons, vec!["太慢".to_string()]);
        for bad in ["沒有 json", "{\"verdict\":\"maybe\"}", "{\"reasons\":[]}", "} {", "{壞掉"] {
            assert!(parse_review(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn prompts_carry_what_each_role_needs() {
        let agent = BoardAgent {
            id: "c1".into(), project_id: "p".into(), kind: "reviewer".into(), name: "審".into(), role: String::new(), prompt_prefix: "嚴格審查".into(),
            canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude".into(), paused_card_id: None, created_by: String::new(),
            created_at: 0, updated_at: 0, version: 1,
        };
        let mut m = module();
        m.title = "登入".into();
        m.body = "做登入頁".into();
        let p = review_prompt(&agent, &m, &m, "+fn login() {}", &["缺測試".into()]);
        assert!(p.starts_with("嚴格審查") && p.contains("做登入頁") && p.contains("+fn login() {}") && p.contains("缺測試") && p.contains("\"verdict\""));
        assert!(review_prompt(&agent, &m, &m, "  ", &[]).contains("沒有任何改動"));
        let fix = fix_body(&m, &["缺測試".into(), "命名不一致".into()], 2);
        assert!(fix.contains("第 2 次修正") && fix.contains("- 缺測試") && fix.contains("- 命名不一致") && fix.contains("做登入頁"));
        let w = work_prompt(&agent, &m);
        assert!(w.contains("# 登入") && w.contains("嚴格審查"));
    }

    #[test]
    fn only_non_archived_work_cards_of_coders_are_modules() {
        let mut coder = BoardAgent {
            id: "c1".into(), project_id: "p".into(), kind: "coder".into(), name: "c".into(), role: String::new(), prompt_prefix: String::new(),
            canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude".into(), paused_card_id: None, created_by: String::new(),
            created_at: 0, updated_at: 0, version: 1,
        };
        let m = module();
        let mut other = module();
        other.id = "o".into();
        other.agent_id = Some("rv".into());
        let mut archived = module();
        archived.id = "a".into();
        archived.archived_at = Some(1);
        let r = review("r", "m", "pass");
        let cards = [m, other, archived, r];
        let agents = [coder.clone()];
        assert_eq!(modules(&cards, &agents).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["m"]);
        coder.kind = "reviewer".into();
        assert!(modules(&cards, &[coder]).is_empty());
    }
}
