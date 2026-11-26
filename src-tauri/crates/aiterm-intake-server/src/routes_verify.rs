//! 驗收者的端點：專案開關、驗收報告與證據檔、放行與重新驗收、禁止清單。
//! 成員可讀報告；開關、放行、重新驗收與禁止清單的修改只有管理員。
use crate::deny::{self, Rule};
use crate::routes::Admin;
use crate::routes_data::{bad, conflict, internal, not_found};
use crate::state::AppState;
use aiterm_intake_core::board_store as bs;
use aiterm_intake_core::host::Notifier;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

type St = State<Arc<AppState>>;

async fn project_exists(st: &AppState, pid: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projects WHERE id = ?").bind(pid).fetch_one(&st.pools.server).await.unwrap_or(0) > 0
}

// ---- 專案開關 ----

#[derive(Serialize, Deserialize)]
struct VerifySettings {
    enabled: bool,
    /// 畫布上有沒有驗收者（沒有的話就算開了也不會驗）。
    #[serde(default)]
    has_verifier: bool,
}

async fn get_settings(State(st): St, Path(pid): Path<String>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    let has_verifier = bs::list_agents(&st.pools.intake, &pid).await.map(|a| a.iter().any(|x| x.kind == "verifier")).unwrap_or(false);
    Json(VerifySettings { enabled: crate::orchestrator::verify_enabled(&st.pools.intake, &pid).await, has_verifier }).into_response()
}

async fn put_settings(State(st): St, Admin(me): Admin, Path(pid): Path<String>, Json(b): Json<VerifySettings>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    if b.enabled {
        let wd: String = sqlx::query_scalar("SELECT work_dir FROM projects WHERE id = ?").bind(&pid).fetch_optional(&st.pools.server).await.ok().flatten().unwrap_or_default();
        if wd.trim().is_empty() {
            return bad("這個專案還沒設定工作資料夾（work_dir），無法由 server 驗收");
        }
        if !bs::list_agents(&st.pools.intake, &pid).await.map(|a| a.iter().any(|x| x.kind == "verifier")).unwrap_or(false) {
            return bad("這個專案的畫布上還沒有驗收者，請先加一位驗收者");
        }
    }
    match crate::orchestrator::set_verify_enabled(&st.pools.intake, &pid, b.enabled).await {
        Ok(()) => {
            crate::audit::record(&st.pools.server, &me.name, if b.enabled { "board.verify_on" } else { "board.verify_off" }, &pid).await;
            st.events.changed();
            Json(VerifySettings { enabled: b.enabled, has_verifier: true }).into_response()
        }
        Err(e) => internal(e),
    }
}

// ---- 報告與證據 ----

#[derive(Serialize)]
struct ReportView {
    card_id: String,
    status: String,
    outcome: Option<String>,
    verdict: Option<String>,
    error_message: Option<String>,
    signature: String,
    report: Option<serde_json::Value>,
    denied: Vec<String>,
    manual_count: i64,
    overridden_by: Option<String>,
    ports: Vec<u16>,
    /// 證據資料夾第一層的檔名（截圖、輸出）。
    evidence: Vec<String>,
}

async fn verify_card(st: &AppState, id: &str) -> Option<bs::BoardCard> {
    bs::get_card(&st.pools.intake, id).await.ok().flatten().filter(|c| c.card_kind == "verify")
}

async fn report(State(st): St, Path(id): Path<String>) -> Response {
    let Some(card) = verify_card(&st, &id).await else { return not_found() };
    let meta = match bs::get_verify(&st.pools.intake, &id).await {
        Ok(m) => m,
        Err(e) => return internal(e),
    };
    let evidence_dir = crate::verifier::card_dir(&st, &id).join("evidence");
    let mut evidence: Vec<String> = std::fs::read_dir(&evidence_dir).map(|d| d.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
    evidence.sort();
    let view = match meta {
        Some(m) => ReportView {
            card_id: id,
            status: card.status,
            outcome: card.outcome,
            verdict: card.verdict,
            error_message: card.error_message,
            signature: m.signature,
            report: m.report.and_then(|r| serde_json::from_str(&r).ok()),
            denied: m.denied,
            manual_count: m.manual_count,
            overridden_by: m.overridden_by,
            ports: m.ports,
            evidence,
        },
        None => ReportView { card_id: id, status: card.status, outcome: card.outcome, verdict: card.verdict, error_message: card.error_message, signature: String::new(), report: None, denied: vec![], manual_count: 0, overridden_by: None, ports: vec![], evidence },
    };
    Json(view).into_response()
}

/// 檔名只能是證據資料夾底下的單一檔案：不含路徑分隔、不含 `..`、不是隱藏檔。
pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 200 && !name.contains(['/', '\\', '\0']) && !name.contains("..") && !name.starts_with('.')
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("txt") | Some("log") | Some("md") => "text/plain; charset=utf-8",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

async fn evidence_file(State(st): St, Path((id, name)): Path<(String, String)>) -> Response {
    if verify_card(&st, &id).await.is_none() {
        return not_found();
    }
    if !is_safe_name(&name) {
        return bad("檔名不合法");
    }
    let dir = crate::verifier::card_dir(&st, &id).join("evidence");
    let (Ok(root), Ok(path)) = (dir.canonicalize(), dir.join(&name).canonicalize()) else { return not_found() };
    if !path.starts_with(&root) || !path.is_file() {
        return not_found();
    }
    match std::fs::read(&path) {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, content_type(&name)), (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff")], bytes).into_response(),
        Err(_) => not_found(),
    }
}

// ---- 放行與重新驗收 ----

async fn override_pass(State(st): St, Admin(me): Admin, Path(id): Path<String>) -> Response {
    let Some(card) = verify_card(&st, &id).await else { return not_found() };
    let stuck = card.status == bs::DONE && (card.outcome.as_deref() != Some("success") || matches!(card.verdict.as_deref(), Some("fail") | Some("attention")));
    if !stuck {
        return conflict("這張驗收卡沒有卡住，不需要放行");
    }
    let pool = &st.pools.intake;
    match bs::force_verify_pass(pool, &id).await {
        Ok(true) => {
            bs::set_verify_override(pool, &id, &me.name).await.ok();
            crate::audit::record(&st.pools.server, &me.name, "board.verify_override", &id).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => conflict("這張驗收卡現在不能放行"),
        Err(e) => internal(e),
    }
}

async fn retry(State(st): St, Admin(me): Admin, Path(id): Path<String>) -> Response {
    let Some(card) = verify_card(&st, &id).await else { return not_found() };
    if card.status != bs::DONE || (card.outcome.as_deref() == Some("success") && card.verdict.as_deref() == Some("pass")) {
        return conflict("只有結案而且沒通過的驗收卡能重新驗收");
    }
    let pool = &st.pools.intake;
    let Ok(agents) = bs::list_agents(pool, &card.project_id).await else { return bad("讀不到專案的 Agent") };
    let Some(verifier) = agents.iter().find(|a| a.kind == "verifier") else { return bad("這個專案的畫布上沒有驗收者") };
    // 已經有一張還沒結案的就不再建（避免連按兩次）。
    if bs::verify_cards(pool, &card.project_id).await.map(|v| v.iter().any(|c| c.status != bs::DONE)).unwrap_or(false) {
        return conflict("已經有一張驗收卡在進行中");
    }
    let sig = bs::get_verify(pool, &id).await.ok().flatten().map(|v| v.signature).unwrap_or_default();
    let base = sig.split('#').next().unwrap_or("").to_string();
    let views = {
        let mut out = Vec::new();
        for c in bs::verify_cards(pool, &card.project_id).await.unwrap_or_default() {
            let s = bs::get_verify(pool, &c.id).await.ok().flatten().map(|v| v.signature).unwrap_or_default();
            out.push(aiterm_intake_core::verifier::VerifyCardView { id: c.id, signature: s, status: c.status, outcome: c.outcome, verdict: c.verdict });
        }
        out
    };
    let next = crate::verifier::next_signature(&views, &base);
    match crate::verifier::create_verify_card(pool, &card.project_id, verifier, &next).await {
        Ok(new) => {
            crate::audit::record(&st.pools.server, &me.name, "board.verify_retry", &id).await;
            st.events.changed();
            Json(new).into_response()
        }
        Err(e) => internal(e),
    }
}

// ---- 禁止清單 ----

async fn get_global(State(st): St, Admin(_me): Admin) -> Response {
    Json(deny::load_global(&st.pools.intake).await).into_response()
}

#[derive(Deserialize)]
struct GlobalBody {
    rules: Vec<Rule>,
}

async fn put_global(State(st): St, Admin(me): Admin, Json(b): Json<GlobalBody>) -> Response {
    match deny::save_global(&st.pools.intake, &b.rules).await {
        Ok(()) => {
            crate::audit::record(&st.pools.server, &me.name, "board.verify_deny", &format!("global:{}條", b.rules.len())).await;
            Json(deny::load_global(&st.pools.intake).await).into_response()
        }
        Err(e) => bad(&e),
    }
}

async fn reset_global(State(st): St, Admin(me): Admin) -> Response {
    match deny::reset_global(&st.pools.intake).await {
        Ok(()) => {
            crate::audit::record(&st.pools.server, &me.name, "board.verify_deny", "global:reset").await;
            Json(deny::load_global(&st.pools.intake).await).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Serialize)]
struct ProjectDeny {
    global: Vec<Rule>,
    overrides: Vec<bs::DenyOverride>,
    effective: Vec<String>,
}

async fn project_deny(st: &AppState, pid: &str) -> ProjectDeny {
    ProjectDeny { global: deny::load_global(&st.pools.intake).await, overrides: bs::list_deny_overrides(&st.pools.intake, pid).await.unwrap_or_default(), effective: deny::effective_for(&st.pools.intake, pid).await }
}

async fn get_project_deny(State(st): St, Admin(_me): Admin, Path(pid): Path<String>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    Json(project_deny(&st, &pid).await).into_response()
}

#[derive(Deserialize)]
struct OverrideIn {
    rule: String,
    kind: String,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct ProjectBody {
    overrides: Vec<OverrideIn>,
}

async fn put_project_deny(State(st): St, Admin(me): Admin, Path(pid): Path<String>, Json(b): Json<ProjectBody>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    for o in &b.overrides {
        if let Err(e) = deny::validate_rule(&o.rule) {
            return bad(&format!("{}：{e}", o.rule));
        }
        if !matches!(o.kind.as_str(), "add" | "remove") {
            return bad(&format!("{}：kind 要是 add 或 remove", o.rule));
        }
    }
    let pool = &st.pools.intake;
    let existing = bs::list_deny_overrides(pool, &pid).await.unwrap_or_default();
    for e in existing.iter().filter(|e| !b.overrides.iter().any(|o| o.rule.trim() == e.rule)) {
        if let Err(err) = bs::delete_deny_override(pool, &pid, &e.rule).await {
            return internal(err);
        }
    }
    for o in &b.overrides {
        if let Err(err) = bs::put_deny_override(pool, &pid, o.rule.trim(), &o.kind, o.enabled).await {
            return internal(err);
        }
    }
    crate::audit::record(&st.pools.server, &me.name, "board.verify_deny", &format!("{pid}:{}條", b.overrides.len())).await;
    Json(project_deny(&st, &pid).await).into_response()
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/board/projects/{pid}/verify", get(get_settings).put(put_settings))
        .route("/v1/board/cards/{id}/verify", get(report))
        .route("/v1/board/cards/{id}/verify/files/{name}", get(evidence_file))
        .route("/v1/board/cards/{id}/verify/override", post(override_pass))
        .route("/v1/board/cards/{id}/verify/retry", post(retry))
        .route("/v1/verify/deny", get(get_global).put(put_global))
        .route("/v1/verify/deny/reset", post(reset_global))
        .route("/v1/board/projects/{pid}/verify/deny", get(get_project_deny).put(put_project_deny))
}
