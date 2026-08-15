//! GitHub Copilot 配額。
//!
//! 端點：`GET https://api.github.com/copilot_internal/user`
//!
//! 三家裡唯一**剩餘導向**的來源：`percent_remaining` 要換算成已用百分比。
//! `unlimited: true` 的 snapshot（chat / completions）不產生窗。

use super::{now_secs, ProviderQuota, QuotaSeverity, QuotaSource, QuotaWindow};
use crate::ai::AiError;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::BTreeMap;

const USER_URL: &str = "https://api.github.com/copilot_internal/user";

#[derive(Deserialize)]
struct UserResponse {
    #[serde(default)]
    access_type_sku: Option<String>,
    #[serde(default)]
    quota_reset_date_utc: Option<String>,
    #[serde(default)]
    quota_snapshots: BTreeMap<String, Snapshot>,
}

#[derive(Deserialize)]
struct Snapshot {
    #[serde(default)]
    unlimited: bool,
    /// **刻意用 Option**：欄位缺席時若 serde 補成 0.0，`100.0 - 0.0` 會算出
    /// 「已用 100%」的滿格紅燈，把解析失敗偽裝成配額耗盡。缺席就跳過該窗。
    #[serde(default)]
    percent_remaining: Option<f64>,
    #[serde(default)]
    entitlement: i64,
    #[serde(default)]
    remaining: i64,
}

/// snapshot 的 key 轉成短標籤。
fn label_for(key: &str) -> String {
    match key {
        "premium_interactions" => "premium".into(),
        other => other.into(),
    }
}

pub fn parse_user(
    provider_id: &str,
    body: &str,
    fetched_at: i64,
) -> Result<ProviderQuota, AiError> {
    let r: UserResponse = serde_json::from_str(body)
        .map_err(|e| AiError::ModelError {
            reason: format!("配額回應解析失敗: {e}"),
            raw: body.chars().take(500).collect(),
        })?;

    let resets_at = r
        .quota_reset_date_utc
        .as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp());

    let windows: Vec<QuotaWindow> = r
        .quota_snapshots
        .into_iter()
        // 無限的項目沒有配額可言；entitlement 為 0 代表這個 SKU 沒有這項
        // 額度，硬算會得到「0 / 0」加 100% 紅燈。兩者都跳過。
        .filter(|(_, s)| !s.unlimited && s.entitlement > 0)
        .filter_map(|(key, s)| {
            // 缺 percent_remaining 就跳過，不要用預設值編出假的 100%。
            let remaining_pct = s.percent_remaining?;
            let used = (100.0 - remaining_pct).clamp(0.0, 100.0);
            Some(QuotaWindow {
                label: label_for(&key),
                used_percent: used,
                resets_at,
                severity: QuotaSeverity::from_percent(used),
                detail: Some(format!("{} / {}", s.remaining, s.entitlement)),
                // 契約是「至多一個 is_primary」。Copilot 可能同時有多個有限
                // 項目（chat / completions / premium 都有額度的方案），只有
                // premium 值得放在收合徽章上 —— 其餘設 false，否則
                // primary_window() 會照 BTreeMap 字典序選到 "chat"。
                is_primary: key == "premium_interactions",
            })
        })
        .collect();

    Ok(ProviderQuota {
        provider_id: provider_id.into(),
        plan: r.access_type_sku,
        windows,
        fetched_at,
    })
}

pub struct CopilotQuota {
    provider_id: String,
    github_token: String,
    url: String,
    client: reqwest::Client,
}

impl CopilotQuota {
    pub fn new(provider_id: String, github_token: String) -> Self {
        Self { provider_id, github_token, url: USER_URL.into(), client: reqwest::Client::new() }
    }

    /// 測試用：指向 wiremock 伺服器。
    pub fn with_url(mut self, url: String) -> Self {
        self.url = url;
        self
    }
}

#[async_trait]
impl QuotaSource for CopilotQuota {
    async fn fetch(&self) -> Result<ProviderQuota, AiError> {
        let resp = self
            .client
            .get(&self.url)
            // 注意是 `token`，不是 `Bearer` —— 這是 GitHub 而非 Copilot 端點。
            .header("Authorization", format!("token {}", self.github_token))
            .header("User-Agent", "AITerm/1.0")
            .header("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AiError::AuthFailed);
        }
        if !status.is_success() {
            return Err(AiError::ModelError {
                reason: format!("配額端點回 {status}"),
                raw: body.chars().take(500).collect(),
            });
        }
        parse_user(&self.provider_id, &body, now_secs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = r#"{
      "access_type_sku": "yearly_subscriber_quota",
      "quota_reset_date_utc": "2026-09-01T00:00:00.000Z",
      "quota_snapshots": {
        "chat": { "unlimited": true, "percent_remaining": 100.0, "entitlement": 0, "remaining": 0 },
        "completions": { "unlimited": true, "percent_remaining": 100.0, "entitlement": 0, "remaining": 0 },
        "premium_interactions": { "unlimited": false, "percent_remaining": 47.5,
                                  "entitlement": 300, "remaining": 142, "credits_used": 157 }
      }
    }"#;

    #[test]
    fn remaining_percent_is_converted_to_used_percent() {
        // 47.5% 剩餘 = 52.5% 已用。直接把 percent_remaining 當已用會完全顛倒。
        let q = parse_user("Github-Sonet4.5", REAL, 0).expect("parse");
        let w = q.primary_window().expect("有窗");
        assert!((w.used_percent - 52.5).abs() < 1e-9, "得到 {}", w.used_percent);
    }

    #[test]
    fn unlimited_snapshots_are_skipped() {
        // chat / completions 是無限，不該產生窗。
        let q = parse_user("Github-Sonet4.5", REAL, 0).expect("parse");
        assert_eq!(q.windows.len(), 1);
        assert_eq!(q.windows[0].label, "premium");
    }

    #[test]
    fn detail_preserves_the_original_count_semantics() {
        let q = parse_user("Github-Sonet4.5", REAL, 0).expect("parse");
        assert_eq!(q.windows[0].detail.as_deref(), Some("142 / 300"));
    }

    #[test]
    fn reset_date_parsed_into_unix_seconds() {
        let q = parse_user("Github-Sonet4.5", REAL, 0).expect("parse");
        // 2026-09-01T00:00:00Z
        assert_eq!(q.windows[0].resets_at, Some(1_788_220_800));
    }

    #[test]
    fn sku_is_carried_as_plan() {
        let q = parse_user("Github-Sonet4.5", REAL, 0).expect("parse");
        assert_eq!(q.plan.as_deref(), Some("yearly_subscriber_quota"));
    }

    #[test]
    fn all_unlimited_yields_no_windows_not_an_error() {
        let raw = r#"{"access_type_sku":"x","quota_snapshots":{
            "chat":{"unlimited":true,"percent_remaining":100.0}}}"#;
        let q = parse_user("p", raw, 0).expect("parse");
        assert!(q.windows.is_empty(), "無限方案 = 查得到但沒有配額限制");
    }

    #[test]
    fn severity_derived_from_used_percent() {
        let raw = r#"{"quota_snapshots":{"premium_interactions":
            {"unlimited":false,"percent_remaining":5.0,"entitlement":300,"remaining":15}}}"#;
        let q = parse_user("p", raw, 0).expect("parse");
        assert_eq!(q.windows[0].severity, QuotaSeverity::Critical);
    }

    #[test]
    fn missing_percent_remaining_is_skipped_not_treated_as_full() {
        // 欄位缺席若補 0.0，100.0 - 0.0 = 已用 100%，會把解析失敗顯示成
        // 額度耗盡的紅燈。必須跳過。
        let raw = r#"{"quota_snapshots":{"premium_interactions":
            {"unlimited":false,"entitlement":300,"remaining":142}}}"#;
        let q = parse_user("p", raw, 0).expect("parse");
        assert!(q.windows.is_empty());
    }

    #[test]
    fn zero_entitlement_snapshots_are_skipped() {
        // 這個 SKU 根本沒有這項額度，硬算會得到「0 / 0」加 100% 紅燈。
        let raw = r#"{"quota_snapshots":{"chat":
            {"unlimited":false,"percent_remaining":100.0,"entitlement":0,"remaining":0}}}"#;
        let q = parse_user("p", raw, 0).expect("parse");
        assert!(q.windows.is_empty());
    }

    #[test]
    fn only_premium_is_marked_primary_when_several_are_limited() {
        // 有些方案 chat / completions 也有額度。全部設 is_primary 會讓
        // primary_window() 照字典序選到 "chat"，徽章顯示錯的那一項。
        let raw = r#"{"quota_snapshots":{
            "chat":{"unlimited":false,"percent_remaining":88.0,"entitlement":50,"remaining":44},
            "premium_interactions":{"unlimited":false,"percent_remaining":47.5,
                                    "entitlement":300,"remaining":142}}}"#;
        let q = parse_user("p", raw, 0).expect("parse");
        assert_eq!(q.windows.len(), 2);
        let primaries: Vec<&str> = q.windows.iter()
            .filter(|w| w.is_primary).map(|w| w.label.as_str()).collect();
        assert_eq!(primaries, vec!["premium"], "至多一個 is_primary，且必須是 premium");
    }
}
