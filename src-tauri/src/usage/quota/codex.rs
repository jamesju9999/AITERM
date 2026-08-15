//! Codex（ChatGPT 訂閱）配額。
//!
//! 端點：`GET https://chatgpt.com/backend-api/codex/usage`（無文件）
//!
//! 窗長依方案而異（free 是 30 天、付費是 5 小時 + 7 天），**一律由
//! `limit_window_seconds` 推導標籤，不要寫死**。

use super::{now_secs, window_label, ProviderQuota, QuotaSeverity, QuotaSource, QuotaWindow};
use crate::ai::AiError;
use async_trait::async_trait;
use serde::Deserialize;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/codex/usage";

#[derive(Deserialize)]
struct UsageResponse {
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    rate_limit: Option<RateLimit>,
}

#[derive(Deserialize)]
struct RateLimit {
    /// 上游明確說「已經擋住你了」。可能在 used_percent 還是 0 時就為 true
    /// （花費上限、帳號層限流），所以**不能只看百分比**。
    #[serde(default)]
    limit_reached: bool,
    /// 預設 true：欄位缺席時不要誤判成「被擋住」。
    #[serde(default = "default_true")]
    allowed: bool,
    #[serde(default)]
    primary_window: Option<Window>,
    #[serde(default)]
    secondary_window: Option<Window>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct Window {
    #[serde(default)]
    used_percent: f64,
    #[serde(default)]
    limit_window_seconds: i64,
    #[serde(default)]
    reset_at: Option<i64>,
}

impl Window {
    /// `blocked` 為 true 時一律提成 Critical —— 上游說被擋住了，
    /// 百分比是多少都不重要（實測 limit_reached 可能與 used_percent: 0 併存）。
    fn into_quota_window(self, is_primary: bool, blocked: bool) -> QuotaWindow {
        let severity = if blocked {
            QuotaSeverity::Critical
        } else {
            QuotaSeverity::from_percent(self.used_percent)
        };
        QuotaWindow {
            label: window_label(self.limit_window_seconds),
            used_percent: self.used_percent,
            resets_at: self.reset_at,
            severity,
            detail: None,
            is_primary,
        }
    }
}

pub fn parse_usage(
    provider_id: &str,
    body: &str,
    fetched_at: i64,
) -> Result<ProviderQuota, AiError> {
    let r: UsageResponse = serde_json::from_str(body)
        .map_err(|e| AiError::ModelError {
            reason: format!("配額回應解析失敗: {e}"),
            raw: body.chars().take(500).collect(),
        })?;

    let mut windows = Vec::new();
    if let Some(rl) = r.rate_limit {
        let blocked = rl.limit_reached || !rl.allowed;
        if let Some(w) = rl.primary_window {
            windows.push(w.into_quota_window(true, blocked));
        }
        if let Some(w) = rl.secondary_window {
            windows.push(w.into_quota_window(false, blocked));
        }
    }

    Ok(ProviderQuota { provider_id: provider_id.into(), plan: r.plan_type, windows, fetched_at })
}

pub struct CodexQuota {
    provider_id: String,
    access_token: String,
    account_id: Option<String>,
    url: String,
    timeout: std::time::Duration,
    client: reqwest::Client,
}

impl CodexQuota {
    pub fn new(provider_id: String, access_token: String, account_id: Option<String>) -> Self {
        Self {
            provider_id,
            access_token,
            account_id,
            url: USAGE_URL.into(),
            timeout: std::time::Duration::from_secs(5),
            client: reqwest::Client::new(),
        }
    }

    /// 測試用：指向 wiremock 伺服器。
    pub fn with_url(mut self, url: String) -> Self {
        self.url = url;
        self
    }

    /// 測試用：縮短逾時，讓逾時測試不必真的等 5 秒。
    pub fn with_timeout(mut self, d: std::time::Duration) -> Self {
        self.timeout = d;
        self
    }
}

#[async_trait]
impl QuotaSource for CodexQuota {
    async fn fetch(&self) -> Result<ProviderQuota, AiError> {
        let mut req = self
            .client
            .get(&self.url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("originator", "codex_cli_rs")
            .timeout(self.timeout);
        if let Some(id) = &self.account_id {
            req = req.header("chatgpt-account-id", id.as_str());
        }
        let resp = req.send().await.map_err(|e| AiError::Network { message: e.to_string() })?;

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
        parse_usage(&self.provider_id, &body, now_secs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = r#"{
      "user_id": "user-x", "account_id": "acc-x", "email": "a@b.c",
      "plan_type": "free",
      "rate_limit": {
        "allowed": true, "limit_reached": false,
        "primary_window": { "used_percent": 0, "limit_window_seconds": 2592000,
                            "reset_after_seconds": 2268666, "reset_at": 1789029443 },
        "secondary_window": null
      },
      "credits": { "has_credits": false, "unlimited": false, "balance": null },
      "spend_control": { "reached": false, "individual_limit": null }
    }"#;

    #[test]
    fn null_secondary_window_yields_single_window() {
        // 實測回應就是 secondary_window: null。硬要兩個窗會直接爆掉。
        let q = parse_usage("GPT5.6", REAL, 0).expect("parse");
        assert_eq!(q.windows.len(), 1);
    }

    #[test]
    fn window_label_comes_from_seconds_not_hardcoded() {
        // free 方案是 30 天窗。把「5h」寫死會顯示成錯的。
        let q = parse_usage("GPT5.6", REAL, 0).expect("parse");
        assert_eq!(q.windows[0].label, "30d");
    }

    #[test]
    fn five_hour_window_labels_correctly() {
        let raw = r#"{"plan_type":"plus","rate_limit":{
            "primary_window":{"used_percent":42,"limit_window_seconds":18000,"reset_at":1789029443},
            "secondary_window":{"used_percent":8,"limit_window_seconds":604800,"reset_at":1789600000}}}"#;
        let q = parse_usage("GPT5.6", raw, 0).expect("parse");
        assert_eq!(q.windows.len(), 2);
        assert_eq!(q.windows[0].label, "5h");
        assert_eq!(q.windows[1].label, "7d");
        assert!((q.windows[0].used_percent - 42.0).abs() < 1e-9);
    }

    #[test]
    fn primary_window_is_marked_primary() {
        let q = parse_usage("GPT5.6", REAL, 0).expect("parse");
        assert!(q.windows[0].is_primary);
        assert_eq!(q.primary_window().expect("有代表窗").label, "30d");
    }

    #[test]
    fn plan_type_is_carried_through() {
        let q = parse_usage("GPT5.6", REAL, 0).expect("parse");
        assert_eq!(q.plan.as_deref(), Some("free"));
    }

    #[test]
    fn reset_at_is_used_directly_as_unix_seconds() {
        // Codex 給的是 Unix 秒，不是 RFC3339。不要再轉一次。
        let q = parse_usage("GPT5.6", REAL, 0).expect("parse");
        assert_eq!(q.windows[0].resets_at, Some(1_789_029_443));
    }

    #[test]
    fn severity_is_derived_since_upstream_gives_none() {
        let raw = r#"{"rate_limit":{"primary_window":
            {"used_percent":95,"limit_window_seconds":18000,"reset_at":1}}}"#;
        let q = parse_usage("p", raw, 0).expect("parse");
        assert_eq!(q.windows[0].severity, QuotaSeverity::Critical);
    }

    #[test]
    fn limit_reached_forces_critical_even_at_zero_percent() {
        // 花費上限觸發時 used_percent 還是 0，照百分比推會顯示成綠燈，
        // 使用者以為隨便用、實際每次請求都被拒。
        let raw = r#"{"rate_limit":{"allowed":false,"limit_reached":true,
            "primary_window":{"used_percent":0,"limit_window_seconds":18000,"reset_at":1}}}"#;
        let q = parse_usage("p", raw, 0).expect("parse");
        assert_eq!(q.windows[0].severity, QuotaSeverity::Critical);
    }

    #[test]
    fn allowed_defaults_to_true_when_absent() {
        // 欄位缺席不可被當成「被擋住」，否則所有舊版回應都會變紅燈。
        let raw = r#"{"rate_limit":{
            "primary_window":{"used_percent":1,"limit_window_seconds":18000,"reset_at":1}}}"#;
        let q = parse_usage("p", raw, 0).expect("parse");
        assert_eq!(q.windows[0].severity, QuotaSeverity::Normal);
    }

    #[test]
    fn absent_window_seconds_yields_empty_label_not_zero_d() {
        // limit_window_seconds 缺席時 serde 預設 0，不可編出「0d」這種假標籤。
        let raw = r#"{"rate_limit":{"primary_window":{"used_percent":5,"reset_at":1}}}"#;
        let q = parse_usage("p", raw, 0).expect("parse");
        assert_eq!(q.windows[0].label, "");
    }

    #[test]
    fn missing_rate_limit_yields_no_windows_not_an_error() {
        // 「查得到但沒有配額」與「查詢失敗」是不同狀態。
        let q = parse_usage("p", r#"{"plan_type":"pro"}"#, 0).expect("parse");
        assert!(q.windows.is_empty());
        assert!(q.primary_window().is_none());
    }
}
