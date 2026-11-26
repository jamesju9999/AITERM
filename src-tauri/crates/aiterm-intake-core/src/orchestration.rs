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
    /// 規格模組審查通過了，還沒有拆解卡：要建一張給 Orchestrator。`sha` 是通過的那版。
    NeedsPlan { sha: Option<String> },
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

/// 模組鏈目前的尾端：從模組開始沿 `review_of` 一路走到底。驗收退回的修正卡（vfix）要接在這張後面。
pub fn chain_tail<'a>(module: &'a BoardCard, chain: &'a [BoardCard]) -> &'a BoardCard {
    let mut tail = module;
    for _ in 0..chain.len() + 1 {
        match next_of(tail, chain) {
            Some(n) => tail = n,
            None => break,
        }
    }
    tail
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

/// 模組的完整狀態：審查鏈（`module_state`）之外，規格模組通過審查後還要過「拆解」這一關
/// （沒有 Orchestrator 就照一般模組算通過；人工放行的規格不拆）。`chain` 要包含拆解卡。
pub fn module_status(module: &BoardCard, chain: &[BoardCard], has_reviewer: bool, has_orchestrator: bool) -> ModuleState {
    let state = module_state(module, chain, has_reviewer);
    let ModuleState::Passed { sha } = state else { return state };
    if !module.is_spec || module.module_override || !has_orchestrator {
        return ModuleState::Passed { sha };
    }
    match chain.iter().find(|c| c.card_kind == "plan") {
        None => ModuleState::NeedsPlan { sha },
        Some(p) if p.status != DONE => ModuleState::Waiting,
        Some(p) if p.outcome.as_deref() == Some("success") => ModuleState::Passed { sha },
        Some(p) => ModuleState::Stuck(format!("拆解「{}」失敗：{}", module.title, p.error_message.clone().unwrap_or_default())),
    }
}

// ---- 拆解（Orchestrator）----

/// 一次最多拆成幾張實作卡。
pub const MAX_PLAN_TASKS: usize = 20;
/// 介面約定的字數上限；超過就截斷（太長會把每張卡的內文洗掉）。
pub const INTERFACES_LIMIT: usize = 1500;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct PlanTask {
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// 依賴的前面任務的索引（0 起算，只能指向排在它前面的任務）。
    #[serde(default)]
    pub depends_on: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub tasks: Vec<PlanTask>,
    /// 任務之間共用的介面約定；沒有就是 `None`。
    pub interfaces: Option<String>,
}

#[derive(serde::Deserialize)]
struct PlanFile {
    #[serde(default)]
    tasks: Vec<PlanTask>,
    #[serde(default)]
    interfaces: Option<String>,
}

/// 從 Orchestrator 的回答取出拆解結果（格式同桌版的 tasks.json）。容忍前後文字與 ``` 圍欄。
/// 驗證：1～`MAX_PLAN_TASKS` 張、每張標題非空、depends_on 只能指向排在前面的任務。
pub fn parse_plan(text: &str) -> Result<Plan, String> {
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else { return Err("拆解的回答裡找不到 JSON".into()) };
    if b < a {
        return Err("拆解的回答裡找不到 JSON".into());
    }
    let f: PlanFile = serde_json::from_str(&text[a..=b]).map_err(|e| format!("拆解的 JSON 讀不懂：{e}"))?;
    let tasks: Vec<PlanTask> = f
        .tasks
        .into_iter()
        .map(|mut t| {
            t.depends_on.sort_unstable();
            t.depends_on.dedup();
            PlanTask { title: t.title.trim().to_string(), ..t }
        })
        .collect();
    if tasks.is_empty() {
        return Err("tasks 是空的".into());
    }
    if tasks.len() > MAX_PLAN_TASKS {
        return Err(format!("最多 {MAX_PLAN_TASKS} 個任務，收到 {}", tasks.len()));
    }
    if tasks.iter().any(|t| t.title.is_empty()) {
        return Err("有任務沒有標題".into());
    }
    for (i, t) in tasks.iter().enumerate() {
        if let Some(bad) = t.depends_on.iter().find(|&&d| d >= i) {
            return Err(format!("第 {i} 個任務「{}」的 depends_on 有 {bad}：只能指向排在它前面的任務（0 到 {}）", t.title, i as isize - 1));
        }
    }
    let interfaces = f.interfaces.map(|i| i.trim().to_string()).filter(|i| !i.is_empty()).map(|i| {
        if i.chars().count() > INTERFACES_LIMIT {
            format!("{}…（太長，已截斷）", i.chars().take(INTERFACES_LIMIT).collect::<String>())
        } else {
            i
        }
    });
    Ok(Plan { tasks, interfaces })
}

/// 拆解卡給 Orchestrator 的提示詞。`spec_text` 是規格全文（server 從 git 讀出來附上）。
pub fn plan_prompt(agent: &BoardAgent, module: &BoardCard, implementers: &[(String, String)], spec_text: &str) -> String {
    let mut s = String::new();
    if !agent.prompt_prefix.trim().is_empty() {
        s.push_str(agent.prompt_prefix.trim());
        s.push_str("\n\n");
    }
    s.push_str(&format!(
        "請把模組「{}」已通過審查的規格拆解成可以分頭實作的任務。你只能讀檔，不要修改任何東西。\n注意：在這個流程裡你只負責拆解；合併分支、解衝突、跑測試都不是你的工作（合併由管理員核准後處理）。上面的角色說明若提到這些，請忽略，只做拆解。\n\n原始需求：\n{}\n\n可以接手實作的工程師（任務會自動派給最閒的一位，你不用指定）：\n",
        module.title.trim(),
        module.body.trim()
    ));
    if implementers.is_empty() {
        s.push_str("- （目前沒有）\n");
    }
    for (name, role) in implementers {
        s.push_str(&format!("- {name}（{role}）\n"));
    }
    s.push_str(&format!(
        "\n拆解原則：每個任務要能獨立實作與審查；body 要自給自足，寫清楚要做什麼、對應規格的哪個章節、驗收條件。最多 {MAX_PLAN_TASKS} 個任務。\n\
         \n相依關係（depends_on）：每位工程師在自己的分支上做，看不到別人還沒合併的程式碼。\n\
         - 任務需要用到前面某些任務寫出來的程式碼，就在 depends_on 列出那些任務的索引（0 起算，只能指向排在它前面的任務）。\n\
         - 整合類任務（例如把各模組組裝起來）要列出它要組裝的所有模組。\n\
         - 被依賴的任務排在前面；彼此獨立的任務不要寫 depends_on，才能平行做。\n\
         \n介面約定（interfaces）：任務之間會互相呼叫或共用的東西，先在這裡一次定死，每張任務都會附上這段。\n\
         - 寫模組之間的函式簽名、資料格式、檔案或目錄的分工、共用的名稱與常數；只寫跨任務要對得上的部分，{INTERFACES_LIMIT} 字以內。\n\
         - 任務只有一個、或彼此完全獨立時可以省略。\n"
    ));
    s.push_str("\n## 規格全文\n");
    s.push_str(if spec_text.trim().is_empty() { "（讀不到規格檔案；請自己用工具讀專案裡的規格文件）" } else { spec_text });
    s.push_str("\n\n拆完最後只回一段 JSON，不要加其他說明：\n{\"interfaces\": \"…\", \"tasks\": [{\"title\": \"…\", \"body\": \"…\"}, {\"title\": \"…\", \"body\": \"…\", \"depends_on\": [0]}]}\n");
    s
}

/// 實作卡的內文：任務內容＋介面約定＋規格來源與全文（實作的 Coder 看不到規格撰寫師分支上的檔案，所以附全文）。
pub fn impl_body(task: &PlanTask, interfaces: Option<&str>, spec_title: &str, spec_text: &str) -> String {
    let contract = interfaces.map(|i| format!("\n\n## 介面約定（各任務共同遵守，不要自行更改）\n{i}")).unwrap_or_default();
    format!("{}{contract}\n\n（這是從規格「{spec_title}」拆出來的任務；規格文件也一併附在下面。）\n\n{spec_text}", task.body.trim())
}

/// 卡片的前置（`depends_on`，JSON 陣列字串）。沒有或格式壞掉都當作沒有。
pub fn deps_of(card: &BoardCard) -> Vec<String> {
    card.depends_on.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default()
}

// ---- 合併 ----

/// 專案現在要不要有一張「合併待核准」卡，以及能不能核准。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStatus {
    /// 沒有新的成果需要合併（沒有模組，或上次合併之後沒有新完成的工作）。
    NothingToMerge,
    /// 有新成果，但還有模組沒審查通過或正在處理中：現在合併會動到還在用的 worktree。
    NotReady,
    /// 所有模組都審查通過，而且有上次合併之後的新成果。
    Ready,
}

/// 模組（連同它的審查與修正）最近一次有動靜的時間。
fn last_activity(module: &BoardCard, cards: &[BoardCard]) -> i64 {
    cards
        .iter()
        .filter(|c| c.module_id.as_deref() == Some(module.id.as_str()))
        .chain(std::iter::once(module))
        .map(|c| c.finished_at.unwrap_or(0).max(c.dispatched_at.unwrap_or(0)))
        .max()
        .unwrap_or(0)
}

pub fn merge_status(cards: &[BoardCard], agents: &[BoardAgent]) -> MergeStatus {
    let mods = modules(cards, agents);
    if mods.is_empty() {
        return MergeStatus::NothingToMerge;
    }
    let has_reviewer = agents.iter().any(|a| a.kind == "reviewer");
    let has_orchestrator = agents.iter().any(|a| a.kind == "orchestrator");
    let last_merge = cards.iter().filter(|c| c.card_kind == "merge" && c.status == DONE).filter_map(|c| c.finished_at).max().unwrap_or(0);
    if !mods.iter().any(|m| last_activity(m, cards) > last_merge) {
        return MergeStatus::NothingToMerge;
    }
    let all_passed = mods.iter().all(|m| {
        let chain: Vec<BoardCard> = cards.iter().filter(|c| c.module_id.as_deref() == Some(m.id.as_str()) && matches!(c.card_kind.as_str(), "review" | "fix" | "vfix" | "plan")).cloned().collect();
        matches!(module_status(m, &chain, has_reviewer, has_orchestrator), ModuleState::Passed { .. })
    });
    if all_passed {
        MergeStatus::Ready
    } else {
        MergeStatus::NotReady
    }
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

/// 上次處理到一半被打斷（server 重啟、額度用完）後，接續同一段對話時給 Coder 的提示詞。
/// 工作資料夾還留著上次的進度；附上原本的任務是為了對話被清掉時也看得懂（對話還在時是重複，無害）。
pub fn resume_prompt(original: &str) -> String {
    format!(
        "上次處理到一半被中斷了（server 重啟或額度用完）。你的工作資料夾還留著上次做到的進度。\n\
         請先看目前的檔案狀態（例如 git status、git diff、讀已經改過的檔案），確認哪些已經做完，\n\
         然後接著完成原本的任務，不要重做已經做好的部分。\n\n\
         原本的任務如下，供對照：\n\n{original}"
    )
}

/// 附在 Coder 提示詞後面，說明這個環境能不能跑測試：沒設驗證指令時 Coder 沒有執行指令的權限，
/// 不講清楚它會一直嘗試被拒絕；設了就告訴它做完後 server 會跑，它自己也可以先跑。
pub fn verify_note(verify_command: &str) -> String {
    let c = verify_command.trim();
    if c.is_empty() {
        "\n注意：這個環境沒有設定驗證指令，你沒有執行任何指令的權限（跑測試、編譯、安裝套件都會被拒絕），不要嘗試。請仔細讀程式碼、自己推演行為，並在回報裡註明「未實際執行測試」。\n".to_string()
    } else {
        format!("\n做完後 server 會在你的分支上執行驗證指令 `{c}` 並把結果交給審查者；你也可以先自己執行它確認。除此之外你沒有執行其他指令的權限。\n")
    }
}

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
pub fn review_prompt(agent: &BoardAgent, module: &BoardCard, reviewed: &BoardCard, diff: &str, previous_reasons: &[String], verification: Option<(&str, bool)>) -> String {
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
    if let Some((report, passed)) = verification {
        s.push_str("\n## 驗證結果（server 在專案環境實際執行的，不是 Coder 自己說的）\n");
        s.push_str(report.trim());
        s.push('\n');
        if !passed {
            s.push_str("驗證沒有通過：除非你確定是環境因素而不是這次改動造成的，否則請判不通過，並在理由裡寫出失敗的原因。\n");
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

    /// 驗收退回的修正卡（vfix）接在通過的審查後面：模組不能還是 Passed，修好後要重新審查；
    /// 而且 vfix 不計入審查的退回輪數（兩種輪數各自算）。
    #[test]
    fn a_verify_fix_after_a_passed_review_puts_the_module_back_in_progress_and_does_not_use_up_review_rounds() {
        let m = module();
        let r1 = review("r1", "m", "pass");
        let mut v1 = card("v1", "vfix", Some("r1"));
        v1.status = "queued".into();
        v1.outcome = None;
        assert_eq!(module_state(&m, &[r1.clone(), v1.clone()], true), ModuleState::Waiting, "vfix 還沒做完＝修正中，不是 Passed");
        v1.status = DONE.into();
        v1.outcome = Some("success".into());
        assert_eq!(module_state(&m, &[r1.clone(), v1.clone()], true), ModuleState::NeedsReview { target: "v1".into() });
        let r2 = review("r2", "v1", "pass");
        assert_eq!(module_state(&m, &[r1, v1.clone(), r2], true), ModuleState::Passed { sha: Some("sha-v1".into()) });
        assert_eq!(module_state(&m, &[review("r1", "m", "pass"), v1], false), ModuleState::Passed { sha: Some("sha-v1".into()) }, "沒有 Reviewer：vfix 做完就算通過");

        // 一次審查退回的 fix ＋ 一次驗收退回的 vfix，再被審查退回：審查退回輪數只算 fix 的 1 次，還有一次修正機會。
        let f1 = review("f1", "m", "fail");
        let x1 = card("x1", "fix", Some("f1"));
        let p1 = review("p1", "x1", "pass");
        let vf = card("vf", "vfix", Some("p1"));
        let f2 = review("f2", "vf", "fail");
        assert_eq!(module_state(&m, &[f1, x1, p1, vf, f2], true), ModuleState::NeedsFix { review: "f2".into(), round: 2 }, "vfix 不算審查退回輪數");
    }

    #[test]
    fn the_chain_tail_is_the_last_card_in_the_chain_whatever_the_order() {
        let m = module();
        assert_eq!(chain_tail(&m, &[]).id, "m");
        let r1 = review("r1", "m", "fail");
        let f1 = card("f1", "fix", Some("r1"));
        let r2 = review("r2", "f1", "pass");
        assert_eq!(chain_tail(&m, &[r2.clone(), r1.clone(), f1.clone()]).id, "r2");
        // 環也不會卡死。
        let a = card("a", "fix", Some("b"));
        let b = card("b", "fix", Some("a"));
        let _ = chain_tail(&m, &[a, b]);
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
            canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude".into(), paused_card_id: None, custom_skills: None, created_by: String::new(),
            created_at: 0, updated_at: 0, version: 1,
        };
        let mut m = module();
        m.title = "登入".into();
        m.body = "做登入頁".into();
        let p = review_prompt(&agent, &m, &m, "+fn login() {}", &["缺測試".into()], None);
        assert!(p.starts_with("嚴格審查") && p.contains("做登入頁") && p.contains("+fn login() {}") && p.contains("缺測試") && p.contains("\"verdict\""));
        assert!(review_prompt(&agent, &m, &m, "  ", &[], None).contains("沒有任何改動"));
        assert!(!p.contains("驗證結果"), "沒有驗證就不放這一段");
        let v = review_prompt(&agent, &m, &m, "+x", &[], Some(("【驗證】\n測試通過：cargo test", true)));
        assert!(v.contains("驗證結果") && v.contains("測試通過：cargo test") && !v.contains("驗證沒有通過"));
        let bad = review_prompt(&agent, &m, &m, "+x", &[], Some(("【驗證】\n測試未通過：cargo test", false)));
        assert!(bad.contains("測試未通過") && bad.contains("驗證沒有通過") && bad.contains("判不通過"));
        let fix = fix_body(&m, &["缺測試".into(), "命名不一致".into()], 2);
        assert!(fix.contains("第 2 次修正") && fix.contains("- 缺測試") && fix.contains("- 命名不一致") && fix.contains("做登入頁"));
        let w = work_prompt(&agent, &m);
        assert!(w.contains("# 登入") && w.contains("嚴格審查"));
    }

    #[test]
    fn only_non_archived_work_cards_of_coders_are_modules() {
        let mut coder = BoardAgent {
            id: "c1".into(), project_id: "p".into(), kind: "coder".into(), name: "c".into(), role: String::new(), prompt_prefix: String::new(),
            canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude".into(), paused_card_id: None, custom_skills: None, created_by: String::new(),
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

    fn coder_agent(id: &str) -> BoardAgent {
        BoardAgent {
            id: id.into(), project_id: "p".into(), kind: "coder".into(), name: "c".into(), role: String::new(), prompt_prefix: String::new(),
            canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude".into(), paused_card_id: None, custom_skills: None, created_by: String::new(),
            created_at: 0, updated_at: 0, version: 1,
        }
    }
    fn reviewer_agent() -> BoardAgent {
        BoardAgent { id: "rv".into(), kind: "reviewer".into(), ..coder_agent("rv") }
    }

    #[test]
    fn a_new_merge_is_due_only_for_work_finished_after_the_last_merge_and_never_while_modules_are_unfinished() {
        let agents = [coder_agent("c1"), reviewer_agent()];
        let mut m = module();
        m.finished_at = Some(100);
        let mut r = review("r", "m", "pass");
        r.finished_at = Some(110);
        assert_eq!(merge_status(&[m.clone(), r.clone()], &agents), MergeStatus::Ready, "還沒合併過：有成果就該合併");

        let mut merged = card("mg", "merge", None);
        merged.module_id = None;
        merged.agent_id = None;
        merged.finished_at = Some(120);
        assert_eq!(merge_status(&[m.clone(), r.clone(), merged.clone()], &agents), MergeStatus::NothingToMerge, "合併過之後沒有新成果");

        // 合併之後又做了第二個模組並通過：要再合併一次。
        let mut m2 = module();
        m2.id = "m2".into();
        m2.finished_at = Some(200);
        let mut r2 = review("r2", "m2", "pass");
        r2.module_id = Some("m2".into());
        r2.finished_at = Some(210);
        assert_eq!(merge_status(&[m.clone(), r.clone(), merged.clone(), m2.clone(), r2], &agents), MergeStatus::Ready);

        // 第二個模組還沒審查完：不能合併（會動到還在用的 worktree）。
        assert_eq!(merge_status(&[m.clone(), r.clone(), merged.clone(), m2.clone()], &agents), MergeStatus::NotReady);
        let mut running = m2;
        running.status = "running".into();
        running.finished_at = None;
        running.dispatched_at = Some(205);
        assert_eq!(merge_status(&[m, r, merged, running], &agents), MergeStatus::NotReady, "處理中的也算有動靜");
        assert_eq!(merge_status(&[], &agents), MergeStatus::NothingToMerge);
    }

    fn plan_card(outcome: Option<&str>, status: &str) -> BoardCard {
        let mut p = card("plan1", "plan", None);
        p.status = status.into();
        p.outcome = outcome.map(str::to_string);
        p.error_message = if outcome == Some("failed") { Some("JSON 壞了".into()) } else { None };
        p
    }

    #[test]
    fn a_spec_module_must_also_get_through_decomposition_before_it_counts_as_passed() {
        let mut spec = module();
        spec.is_spec = true;
        let pass = review("r", "m", "pass");
        let base = [pass.clone()];
        assert_eq!(module_status(&spec, &base, true, true), ModuleState::NeedsPlan { sha: Some("sha-m".into()) }, "審查通過、還沒有拆解卡");
        assert_eq!(module_status(&spec, &base, true, false), ModuleState::Passed { sha: Some("sha-m".into()) }, "沒有 Orchestrator 就照一般模組");
        let with = |p: BoardCard| [pass.clone(), p];
        assert_eq!(module_status(&spec, &with(plan_card(None, "queued")), true, true), ModuleState::Waiting);
        assert_eq!(module_status(&spec, &with(plan_card(None, "running")), true, true), ModuleState::Waiting);
        assert_eq!(module_status(&spec, &with(plan_card(Some("success"), "done")), true, true), ModuleState::Passed { sha: Some("sha-m".into()) });
        assert!(matches!(module_status(&spec, &with(plan_card(Some("failed"), "done")), true, true), ModuleState::Stuck(s) if s.contains("JSON 壞了")));
        // 審查還沒過：不會提早要求拆解。
        assert_eq!(module_status(&spec, &[review("r", "m", "fail")], true, true), ModuleState::NeedsFix { review: "r".into(), round: 1 });
        // 人工放行的規格不拆；一般模組完全不受影響。
        let mut over = spec.clone();
        over.module_override = true;
        assert_eq!(module_status(&over, &[], true, true), ModuleState::Passed { sha: Some("sha-m".into()) });
        assert_eq!(module_status(&module(), &base, true, true), ModuleState::Passed { sha: Some("sha-m".into()) });
    }

    #[test]
    fn plans_are_read_out_of_chatter_validated_and_dependencies_may_only_point_backwards() {
        let ok = parse_plan("好的：\n```json\n{\"interfaces\":\" 共用 User 結構 \",\"tasks\":[{\"title\":\" 資料層 \",\"body\":\"做 db\"},{\"title\":\"介面\",\"depends_on\":[0,0]}]}\n```").unwrap();
        assert_eq!(ok.interfaces.as_deref(), Some("共用 User 結構"));
        assert_eq!((ok.tasks[0].title.as_str(), ok.tasks[1].depends_on.clone(), ok.tasks[1].body.as_str()), ("資料層", vec![0], ""), "標題去空白、重複的相依去掉、body 可省略");
        assert_eq!(parse_plan("{\"tasks\":[{\"title\":\"a\"}],\"interfaces\":\"  \"}").unwrap().interfaces, None);
        let long = format!("{{\"tasks\":[{{\"title\":\"a\"}}],\"interfaces\":\"{}\"}}", "字".repeat(INTERFACES_LIMIT + 5));
        assert!(parse_plan(&long).unwrap().interfaces.unwrap().ends_with("（太長，已截斷）"));
        for (bad, why) in [
            ("沒有 json", "找不到 JSON"),
            ("{\"tasks\":[]}", "空的"),
            ("{\"tasks\":[{\"title\":\" \"}]}", "沒有標題"),
            ("{\"tasks\":[{\"title\":\"a\",\"depends_on\":[0]}]}", "只能指向排在它前面"),
            ("{\"tasks\":[{\"title\":\"a\"},{\"title\":\"b\",\"depends_on\":[5]}]}", "只能指向排在它前面"),
        ] {
            assert!(parse_plan(bad).unwrap_err().contains(why), "{bad}");
        }
        let many = format!("{{\"tasks\":[{}]}}", (0..=MAX_PLAN_TASKS).map(|i| format!("{{\"title\":\"t{i}\"}}")).collect::<Vec<_>>().join(","));
        assert!(parse_plan(&many).unwrap_err().contains("最多"));
    }

    #[test]
    fn the_resume_prompt_tells_the_coder_to_continue_and_keeps_the_original_task_for_reference() {
        let p = resume_prompt("# 做登入頁\n要有記住我");
        assert!(p.contains("被中斷") && p.contains("不要重做"), "{p}");
        assert!(p.ends_with("# 做登入頁\n要有記住我"), "原本的任務附在最後");
    }

    #[test]
    fn verify_note_tells_the_coder_whether_it_can_run_anything() {
        assert!(verify_note("  ").contains("沒有設定驗證指令") && verify_note("").contains("未實際執行測試"));
        let n = verify_note(" cargo test ");
        assert!(n.contains("`cargo test`") && !n.contains("沒有設定驗證指令"));
    }

    #[test]
    fn the_plan_prompt_and_implementation_card_carry_the_contract_and_the_spec() {
        let agent = BoardAgent { prompt_prefix: "你是 Orchestrator".into(), ..coder_agent("o1") };
        let mut m = module();
        m.title = "記帳 App".into();
        m.body = "做一個記帳 App".into();
        let p = plan_prompt(&agent, &m, &[("工程師2".into(), "前端".into())], "### spec.md\n\n# 規格");
        assert!(p.starts_with("你是 Orchestrator") && p.contains("做一個記帳 App") && p.contains("工程師2（前端）") && p.contains("# 規格") && p.contains("depends_on") && p.contains("\"interfaces\""));
        let merge_agent = BoardAgent { prompt_prefix: "你是協調者。依序合併各工程師的分支，合併後跑測試。".into(), ..coder_agent("o2") };
        let mp = plan_prompt(&merge_agent, &m, &[], "");
        assert!(mp.find("依序合併").unwrap() < mp.find("只負責拆解").unwrap() && mp.contains("請忽略"), "角色說明偏合併時，拆解提示詞要在它後面明確覆蓋");
        assert!(plan_prompt(&agent, &m, &[], "").contains("（目前沒有）") && plan_prompt(&agent, &m, &[], "").contains("讀不到規格"));
        let t = PlanTask { title: "資料層".into(), body: "做 db".into(), depends_on: vec![] };
        let b = impl_body(&t, Some("User{id:int}"), "記帳 App", "## 規格全文");
        assert!(b.starts_with("做 db") && b.contains("User{id:int}") && b.contains("從規格「記帳 App」拆出來") && b.contains("## 規格全文"));
        assert!(!impl_body(&t, None, "x", "").contains("介面約定"));
        let mut c = module();
        assert!(deps_of(&c).is_empty());
        c.depends_on = Some("[\"a\",\"b\"]".into());
        assert_eq!(deps_of(&c), vec!["a", "b"]);
        c.depends_on = Some("壞掉".into());
        assert!(deps_of(&c).is_empty());
    }
}
