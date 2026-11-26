//! 驗收者的純邏輯（格式與桌面版的 `verify.json` 一致）：verify.json 解析與品質檢查、驗收簽章、
//! 失敗項分組到模組、合併卡閘門、提示詞與驗收退回的修正卡內文。
//! 不碰資料庫、不碰子行程；server 的編排與執行在 `aiterm-intake-server`。
//! 設計見 `docs/superpowers/specs/2026-11-26-server-verifier-design.md`。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 驗收不通過最多自動退回幾輪（與審查相同）。
pub use crate::orchestration::MAX_FIX_ROUNDS;

/// 驗收者沒有瀏覽器時要執行的安裝指令（寫進提示）。
pub const BROWSER_INSTALL: &str = "npx -y @playwright/mcp@0.0.83 install-browser chrome-for-testing";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct VerifyItem {
    pub criterion: String,
    pub status: String,
    #[serde(default)]
    pub module_id: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub steps: Option<String>,
    #[serde(default)]
    pub expected: Option<String>,
    #[serde(default)]
    pub actual: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct VerifyReport {
    pub verdict: String,
    #[serde(default)]
    pub project_type: Option<String>,
    #[serde(default)]
    pub items: Vec<VerifyItem>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// 解析 verify.json。有任何 `fail` 項就是 `fail`，不管它自己寫什麼。
pub fn parse_verify_json(text: &str) -> Result<VerifyReport, String> {
    let mut v: VerifyReport = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    if !matches!(v.verdict.as_str(), "pass" | "fail") {
        return Err(format!("verdict 必須是 pass 或 fail，收到 {}", v.verdict));
    }
    if let Some(bad) = v.items.iter().find(|i| !matches!(i.status.as_str(), "pass" | "fail" | "manual")) {
        return Err(format!("status 必須是 pass、fail 或 manual，收到 {}", bad.status));
    }
    if v.items.iter().any(|i| i.status == "fail") {
        v.verdict = "fail".into();
    }
    Ok(v)
}

/// 品質檢查：`manual` 要寫原因（`notes`）；`pass` 要附證據。不合格回清單（驗收卡當成失敗，可重新驗收）。
pub fn check_quality(r: &VerifyReport) -> Result<(), Vec<String>> {
    let mut bad = Vec::new();
    for i in &r.items {
        if i.status == "manual" && i.notes.as_deref().map_or(true, |n| n.trim().is_empty()) {
            bad.push(format!("「{}」標成 manual 卻沒有寫原因", i.criterion));
        }
        if i.status == "pass" && i.evidence.iter().all(|e| e.trim().is_empty()) {
            bad.push(format!("「{}」通過但沒有附證據", i.criterion));
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad)
    }
}

/// 實際驗證通過的條件數（`status == pass`，不含 manual）。
pub fn passed_count(r: &VerifyReport) -> usize {
    r.items.iter().filter(|i| i.status == "pass").count()
}

pub fn manual_count(r: &VerifyReport) -> usize {
    r.items.iter().filter(|i| i.status == "manual").count()
}

/// 驗收簽章：各模組通過的版本（`(模組 id, sha)`，順序不拘）加重試序號。簽章不同＝受驗內容變了，要重新驗。
pub fn signature(passed: &[(String, Option<String>)], retry: u32) -> String {
    let mut parts: Vec<String> = passed.iter().map(|(m, s)| format!("{m}={}", s.as_deref().unwrap_or(""))).collect();
    parts.sort();
    let mut h = Sha256::new();
    h.update(parts.join("\n"));
    h.update(format!("\nretry={retry}"));
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// 失敗項分組的結果。
#[derive(Debug, PartialEq)]
pub struct Attribution<'a> {
    /// 有對應模組的失敗項，依模組分組（模組順序依首次出現）。
    pub by_module: Vec<(String, Vec<&'a VerifyItem>)>,
    /// `module_id` 沒寫、或不是任何已知模組的失敗項。
    pub unattributed: Vec<&'a VerifyItem>,
}

pub fn attribute<'a>(r: &'a VerifyReport, module_ids: &[String]) -> Attribution<'a> {
    let mut by_module: Vec<(String, Vec<&VerifyItem>)> = Vec::new();
    let mut unattributed = Vec::new();
    for i in r.items.iter().filter(|i| i.status == "fail") {
        match i.module_id.as_deref().filter(|m| module_ids.iter().any(|k| k == m)) {
            Some(m) => match by_module.iter_mut().find(|(k, _)| k == m) {
                Some((_, v)) => v.push(i),
                None => by_module.push((m.to_string(), vec![i])),
            },
            None => unattributed.push(i),
        }
    }
    Attribution { by_module, unattributed }
}

/// 閘門判讀用的驗收卡摘要（server 從 `board_cards` ＋ `board_verify` 組出來，按建立時間由舊到新）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyCardView {
    pub id: String,
    pub signature: String,
    pub status: String,
    pub outcome: Option<String>,
    pub verdict: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// 目前這組版本還沒有驗收卡：要建一張。
    NeedsVerify,
    /// 驗收卡還在跑或排隊。
    Waiting,
    /// 同簽章通過：可以建合併卡。
    Passed,
    /// 同簽章不通過、輪數沒滿：呼叫端要依失敗項開 vfix 卡。
    Failed { rounds: usize },
    /// 停住等管理員：attention、驗收卡自己失敗、或輪數用完。
    NeedsHuman,
}

/// 依目前簽章與驗收卡歷史（舊→新）判斷下一步。
/// 輪數 = 從最近一次 `pass` 之後，連續 `fail`／`attention` 的驗收卡張數；`Failed { rounds }` 的 `rounds` 就是
/// 這次要開的修正的輪次（第 1 次失敗＝第 1 輪修正）。最多自動修正 `MAX_FIX_ROUNDS` 輪，再失敗就停下來等人。
pub fn gate(cards: &[VerifyCardView], current: &str) -> Gate {
    let tail_rounds = cards
        .iter()
        .rev()
        .take_while(|c| !(c.status == "done" && c.verdict.as_deref() == Some("pass")))
        .filter(|c| c.status == "done" && matches!(c.verdict.as_deref(), Some("fail") | Some("attention")))
        .count();
    let Some(last) = cards.iter().rev().find(|c| c.signature == current) else { return Gate::NeedsVerify };
    if last.status != "done" {
        return Gate::Waiting;
    }
    if last.outcome.as_deref() != Some("success") {
        return Gate::NeedsHuman;
    }
    match last.verdict.as_deref() {
        Some("pass") => Gate::Passed,
        Some("fail") if tail_rounds > MAX_FIX_ROUNDS => Gate::NeedsHuman,
        Some("fail") => Gate::Failed { rounds: tail_rounds },
        _ => Gate::NeedsHuman,
    }
}

fn take_chars(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push_str("…（略）");
    }
    out
}

/// 驗收卡的提示。`modules` 是 `(id, 標題, 需求)`；`has_browser` 為假（沒有 Node）時要求相關條件標 manual。
pub fn verify_prompt(modules: &[(String, String, String)], spec_text: &str, json_path: &str, evidence: &str, ports: &[u16], has_browser: bool) -> String {
    let mut s = String::from(
        "你是驗收者。目前目錄是各工程師的成果合併之後的版本（拋棄式的工作資料夾）。請確認它真的能用：實際建置、執行、操作，照驗收條件逐條確認。\n\
         - 不要修改、commit 或 push 任何原始碼；安裝相依套件、建置產生的檔案沒關係。\n\
         - 你只做判定，不修 bug。你沒有編輯檔案的工具，用 Bash 把結果寫成檔案。\n\n\
         模組清單（module_id 要把方括號裡的 id 原樣整串貼上，不要縮短、不要自己編；不屬於任何一個模組的整合類條目就省略 module_id）：\n",
    );
    for (id, title, body) in modules {
        s.push_str(&format!("- [{id}] {title}\n  需求：{}\n", take_chars(body, 2000).replace('\n', "\n  ")));
    }
    if spec_text.trim().is_empty() {
        s.push_str("\n沒有規格：請依各模組需求自己列出驗收條件，每條都要驗。\n");
    } else {
        s.push_str(&format!("\n規格全文（照裡面的驗收條件逐條驗）：\n{spec_text}\n"));
    }
    s.push_str(
        "\n先判斷專案類型，照下面的方式驗：\n\
         - 網頁前端：啟動開發伺服器，用瀏覽器工具（mcp__playwright__ 開頭的工具）照驗收條件實際點擊、輸入、換頁；用 browser_console_messages 檢查主控台錯誤；每條條件用 browser_take_screenshot 截圖。\n\
         - 後端 API：啟動服務，實際呼叫端點，檢查狀態碼與回應內容。\n\
         - 全端：前後端都啟動，從瀏覽器操作，確認資料存進後端、重新整理後讀得回來。\n\
         - 命令列工具：實際執行各種參數，檢查輸出與結束碼。\n\
         - 函式庫：跑測試，再寫一小段程式實際呼叫公開介面（寫在目前目錄以外的暫存位置）。\n\
         - Electron：用瀏覽器工具操作。\n\
         - 其他桌面或手機 App（原生、Tauri、iOS、Android）：只做建置、測試、啟動不當掉；需要操作畫面的條件標成 manual。\n\
         \n專案自己有測試就一定要跑。\n\
         \n不管規格有沒有寫，都要先驗證專案的主要入口能用：網頁就打開根網址，確認畫面有內容、沒有跳錯誤頁、主控台沒有錯誤；後端就呼叫一個最基本的端點；命令列工具就執行一次最基本的指令。這一項用 criterion「主要入口可用」寫進 items（省略 module_id），入口打不開就是 fail。\n",
    );
    s.push_str(&format!("\n截圖一律存成這個資料夾底下的絕對路徑：{evidence}（例如 {evidence}/start.png）。存到別的地方會被拒絕。\n"));
    s.push_str(&format!(
        "伺服器只能用這些埠：{}。驗收結束前把你啟動的伺服器與瀏覽器都關掉（browser_close）。\n",
        ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("、")
    ));
    if has_browser {
        s.push_str(&format!(
            "瀏覽器工具回報瀏覽器沒有安裝（not installed／not found）時，先執行 `{BROWSER_INSTALL}` 再重試一次（第一次要下載約 100MB）。\n\
             瀏覽器工具或其他工具還是不能用時，把需要它的條件標成 manual 並寫原因，不能當成通過。\n"
        ));
    } else {
        s.push_str("瀏覽器工具不可用（這台電腦沒有 Node.js）：需要瀏覽器的條件一律標成 manual 並寫原因，不能當成通過。\n");
    }
    s.push_str(&format!(
        "\n你不能問人，需要人確認的條件一律標成 manual 並寫原因。\n\
         \n完成後把結果寫成 JSON 檔（用 Bash 寫）到：{json_path}\n\
         格式：\n\
         {{\"verdict\": \"pass\" 或 \"fail\", \"project_type\": \"web\", \"items\": [\
         {{\"criterion\": \"驗收條件\", \"status\": \"pass\" 或 \"fail\" 或 \"manual\", \
         \"module_id\": \"這條屬於哪個模組的 id（跨模組的全域條件才省略）\", \
         \"evidence\": [\"evidence/start.png\", \"指令輸出摘要\"], \
         \"steps\": \"重現步驟（fail 必填）\", \"expected\": \"預期\", \"actual\": \"實際\", \
         \"notes\": \"manual 的原因或其他說明\"}}], \"notes\": \"其他觀察\"}}\n\
         任何一條 fail，verdict 就是 fail。manual 不算失敗，但一定要寫原因。evidence 裡的檔案路徑寫成相對於 JSON 檔所在資料夾的路徑。\n\
         \n上面清單裡的每個模組都至少要有一條 items（通過的也要寫，用 module_id 標明是哪個模組）；實在驗不了的模組也要寫一條 status 為 manual 的並說明原因。\n\
         \n每一條 status 為 pass 的都必須附證據（evidence）：截圖路徑，或實際跑過的指令與輸出摘要。沒有證據的 pass 視為沒驗；只讀程式碼不算驗過。\n"
    ));
    s
}

/// 驗收退回的修正卡內文。`items` 是屬於這個模組的失敗項。
pub fn vfix_body(module_title: &str, module_body: &str, items: &[&VerifyItem], verify_id: &str, round: usize) -> String {
    let mut s = format!("驗收沒有通過，請修正模組「{module_title}」（驗收退回第 {round} 輪，最多 {MAX_FIX_ROUNDS} 輪；驗收卡 {verify_id}）。\n驗收發現的問題：\n");
    for i in items {
        s.push_str(&format!("- {}\n", i.criterion));
        if let Some(v) = &i.steps {
            s.push_str(&format!("  重現步驟：{v}\n"));
        }
        if let Some(v) = &i.expected {
            s.push_str(&format!("  預期：{v}\n"));
        }
        if let Some(v) = &i.actual {
            s.push_str(&format!("  實際：{v}\n"));
        }
        if !i.evidence.is_empty() {
            s.push_str(&format!("  證據：{}\n", i.evidence.join("、")));
        }
    }
    s.push_str(&format!("\n這個模組原本的需求（供對照）：\n{}\n\n修好後照一般流程交付，會再審查、再驗收。\n", take_chars(module_body, 2000)));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vc(id: &str, sig: &str, status: &str, outcome: Option<&str>, verdict: Option<&str>) -> VerifyCardView {
        VerifyCardView { id: id.into(), signature: sig.into(), status: status.into(), outcome: outcome.map(Into::into), verdict: verdict.map(Into::into) }
    }

    #[test]
    fn any_failed_item_makes_the_verdict_fail_whatever_the_file_says() {
        let r = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"pass","evidence":["x.png"]},{"criterion":"b","status":"fail","steps":"s","expected":"e","actual":"a"}]}"#).unwrap();
        assert_eq!(r.verdict, "fail");
    }

    #[test]
    fn bad_shapes_are_refused_with_a_reason() {
        assert!(parse_verify_json("not json").is_err());
        assert!(parse_verify_json(r#"{"verdict":"maybe","items":[]}"#).unwrap_err().contains("pass 或 fail"));
        assert!(parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"ok"}]}"#).unwrap_err().contains("pass、fail 或 manual"));
        assert!(parse_verify_json("\u{feff}{\"verdict\":\"pass\",\"items\":[]}").is_ok(), "容忍 BOM");
    }

    #[test]
    fn a_manual_item_needs_a_reason_and_a_pass_needs_evidence() {
        let no_reason = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"manual"}]}"#).unwrap();
        assert_eq!(check_quality(&no_reason).unwrap_err(), vec!["「a」標成 manual 卻沒有寫原因".to_string()]);
        let no_evidence = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"b","status":"pass"}]}"#).unwrap();
        assert_eq!(check_quality(&no_evidence).unwrap_err(), vec!["「b」通過但沒有附證據".to_string()]);
        let ok = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"c","status":"pass","evidence":["npm test: 12 passed"]},{"criterion":"d","status":"manual","notes":"原生畫面"}]}"#).unwrap();
        assert!(check_quality(&ok).is_ok());
        assert_eq!(manual_count(&ok), 1);
    }

    #[test]
    fn only_really_verified_items_count_as_passed() {
        let all_manual = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"manual","notes":"伺服器被擋"},{"criterion":"b","status":"manual","notes":"無法開瀏覽器"}]}"#).unwrap();
        assert_eq!((passed_count(&all_manual), manual_count(&all_manual)), (0, 2), "全部 manual＝沒有實際驗證任何條件");
        let some = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"pass","evidence":["x"]},{"criterion":"b","status":"manual","notes":"n"}]}"#).unwrap();
        assert_eq!(passed_count(&some), 1);
    }

    #[test]
    fn the_signature_follows_the_passed_versions_and_ignores_order() {
        assert_eq!(
            signature(&[("b".into(), Some("2".into())), ("a".into(), Some("1".into()))], 0),
            signature(&[("a".into(), Some("1".into())), ("b".into(), Some("2".into()))], 0)
        );
        assert_ne!(signature(&[("a".into(), Some("1".into()))], 0), signature(&[("a".into(), Some("9".into()))], 0), "版本變了簽章就變");
        assert_ne!(signature(&[("a".into(), Some("1".into()))], 0), signature(&[("a".into(), Some("1".into()))], 1), "重新驗收的序號也算");
    }

    #[test]
    fn failed_items_are_grouped_by_module_and_unknown_ids_are_unattributed() {
        let r = parse_verify_json(
            r#"{"verdict":"fail","items":[
            {"criterion":"a","status":"fail","module_id":"m1","steps":"s","expected":"e","actual":"x"},
            {"criterion":"b","status":"fail","module_id":"m1"},
            {"criterion":"c","status":"fail","module_id":"ghost"},
            {"criterion":"d","status":"fail"},
            {"criterion":"e","status":"pass","module_id":"m2","evidence":["o"]}]}"#,
        )
        .unwrap();
        let a = attribute(&r, &["m1".to_string(), "m2".to_string()]);
        assert_eq!(a.by_module.len(), 1);
        assert_eq!(a.by_module[0].0, "m1");
        assert_eq!(a.by_module[0].1.len(), 2);
        assert_eq!(a.unattributed.len(), 2, "ghost 與沒寫 module_id 的都找不到模組");
    }

    #[test]
    fn the_gate_decides_what_to_create_next() {
        assert_eq!(gate(&[], "sig1"), Gate::NeedsVerify);
        assert_eq!(gate(&[vc("v1", "sig1", "queued", None, None)], "sig1"), Gate::Waiting);
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("success"), Some("pass"))], "sig1"), Gate::Passed);
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("success"), Some("pass"))], "sig2"), Gate::NeedsVerify, "簽章變了（修正卡做完），舊的通過不算");
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("success"), Some("fail"))], "sig1"), Gate::Failed { rounds: 1 });
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("success"), Some("attention"))], "sig1"), Gate::NeedsHuman);
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("failed"), None)], "sig1"), Gate::NeedsHuman, "驗收卡自己失敗：停住等管理員");
        assert_eq!(
            gate(&[vc("v1", "s1", "done", Some("success"), Some("fail")), vc("v2", "s2", "done", Some("success"), Some("fail"))], "s2"),
            Gate::Failed { rounds: 2 },
            "第二次失敗還有第二輪修正"
        );
        assert_eq!(
            gate(
                &[
                    vc("v1", "s1", "done", Some("success"), Some("fail")),
                    vc("v2", "s2", "done", Some("success"), Some("fail")),
                    vc("v3", "s3", "done", Some("success"), Some("fail"))
                ],
                "s3"
            ),
            Gate::NeedsHuman,
            "兩輪修正之後再失敗：停下來等人"
        );
        // 中間通過過一次，輪數重新算。
        assert_eq!(
            gate(
                &[
                    vc("v1", "s1", "done", Some("success"), Some("fail")),
                    vc("v2", "s2", "done", Some("success"), Some("pass")),
                    vc("v3", "s3", "done", Some("success"), Some("fail"))
                ],
                "s3"
            ),
            Gate::Failed { rounds: 1 }
        );
    }

    #[test]
    fn the_verify_prompt_lists_modules_ports_paths_and_never_offers_ask_human() {
        let mods = vec![("m1".to_string(), "計時器".to_string(), "按開始倒數 25 分鐘".to_string())];
        let p = verify_prompt(&mods, "規格：倒數 25 分鐘", "/v/verify.json", "/v/evidence", &[4001, 4002, 4003], true);
        for needle in ["[m1] 計時器", "按開始倒數 25 分鐘", "規格：倒數 25 分鐘", "/v/verify.json", "/v/evidence", "4001", "4002", "4003", "mcp__playwright", "manual", "module_id", "install-browser chrome-for-testing"] {
            assert!(p.contains(needle), "缺：{needle}\n{p}");
        }
        assert!(!p.contains("ask_human"), "server 沒有 ask_human");
        assert!(p.contains("用 Bash 寫"), "驗收者沒有編輯工具，要用 Bash 寫 verify.json");
        let no_browser = verify_prompt(&mods, "", "/v/verify.json", "/v/evidence", &[4001, 4002, 4003], false);
        assert!(no_browser.contains("瀏覽器工具不可用"), "沒有 Node 時要求標 manual");
        assert!(no_browser.contains("沒有規格"));
        assert!(!no_browser.contains("install-browser"));
    }

    #[test]
    fn the_verify_fix_body_carries_the_reproduction_and_the_round() {
        let items = vec![VerifyItem {
            criterion: "按開始倒數".into(),
            status: "fail".into(),
            module_id: Some("m1".into()),
            evidence: vec!["evidence/a.png".into()],
            steps: Some("按開始".into()),
            expected: Some("25:00".into()),
            actual: Some("24:58".into()),
            notes: None,
        }];
        let refs: Vec<&VerifyItem> = items.iter().collect();
        let b = vfix_body("計時器", "按開始倒數 25 分鐘", &refs, "v-card-1", 1);
        for needle in ["計時器", "第 1 輪", "按開始倒數", "按開始", "25:00", "24:58", "evidence/a.png", "按開始倒數 25 分鐘"] {
            assert!(b.contains(needle), "缺：{needle}\n{b}");
        }
    }
}
