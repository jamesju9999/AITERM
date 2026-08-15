//! Anthropic OAuth 訂閱配額。
//!
//! 端點：`GET {base}/api/oauth/usage`（無文件的逆向端點）
//!
//! 必要 header 與既有的 `ai::anthropic` OAuth 請求相同。**共用同一份常數，
//! 不要重複定義** —— 重複常數是 bug 溫床。

use super::{now_secs, ProviderQuota, QuotaSeverity, QuotaSource, QuotaWindow};
use crate::ai::AiError;
use async_trait::async_trait;
use serde::Deserialize;

const USAGE_PATH: &str = "/api/oauth/usage";

#[derive(Deserialize)]
struct UsageResponse {
    #[serde(default)]
    five_hour: Option<Window>,
    #[serde(default)]
    seven_day: Option<Window>,
    #[serde(default)]
    limits: Vec<LimitEntry>,
}

#[derive(Deserialize)]
struct Window {
    /// **百分比 0-100**，不是小數。
    #[serde(default)]
    utilization: f64,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Deserialize)]
struct LimitEntry {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    is_active: bool,
}

/// RFC3339 → Unix 秒。解析不了就回 None，不要讓整份回應失敗。
fn parse_ts(s: &Option<String>) -> Option<i64> {
    let s = s.as_ref()?;
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp())
}

/// 把 `/api/oauth/usage` 的回應正規化。`fetched_at` 由呼叫端注入以便測試。
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

    // limits[] 裡 is_active 的那個 kind 決定代表窗。
    let active_kind = r.limits.iter().find(|l| l.is_active).map(|l| l.kind.as_str());
    let severity_of = |kind: &str, pct: f64| {
        r.limits
            .iter()
            .find(|l| l.kind == kind)
            .map(|l| QuotaSeverity::from_upstream(&l.severity, pct))
            .unwrap_or_else(|| QuotaSeverity::from_percent(pct))
    };

    let mut windows = Vec::new();
    if let Some(w) = r.five_hour {
        windows.push(QuotaWindow {
            label: "5h".into(),
            used_percent: w.utilization,
            resets_at: parse_ts(&w.resets_at),
            severity: severity_of("session", w.utilization),
            detail: None,
            is_primary: active_kind == Some("session"),
        });
    }
    if let Some(w) = r.seven_day {
        windows.push(QuotaWindow {
            label: "7d".into(),
            used_percent: w.utilization,
            resets_at: parse_ts(&w.resets_at),
            severity: severity_of("weekly_all", w.utilization),
            detail: None,
            is_primary: active_kind == Some("weekly_all"),
        });
    }

    Ok(ProviderQuota { provider_id: provider_id.into(), plan: None, windows, fetched_at })
}

pub struct AnthropicQuota {
    provider_id: String,
    access_token: String,
    base_url: String,
    client: reqwest::Client,
}

impl AnthropicQuota {
    pub fn new(provider_id: String, access_token: String, base_url: String) -> Self {
        Self { provider_id, access_token, base_url, client: reqwest::Client::new() }
    }
}

#[async_trait]
impl QuotaSource for AnthropicQuota {
    async fn fetch(&self) -> Result<ProviderQuota, AiError> {
        let url = format!("{}{USAGE_PATH}", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("anthropic-beta", crate::ai::anthropic::OAUTH_BETA_HEADER)
            .header("anthropic-version", crate::ai::anthropic::ANTHROPIC_VERSION)
            .header("x-app", "cli")
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
        parse_usage(&self.provider_id, &body, now_secs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真實回應的精簡版，欄位與數值原封不動取自探勘 dump。
    const REAL: &str = r#"{
      "five_hour": { "utilization": 7.0, "resets_at": "2026-08-15T07:00:00.318695+00:00",
                     "limit_dollars": null, "used_dollars": null, "remaining_dollars": null },
      "seven_day": { "utilization": 4.0, "resets_at": "2026-08-19T16:00:00.318714+00:00",
                     "limit_dollars": null, "used_dollars": null, "remaining_dollars": null },
      "seven_day_opus": null,
      "limits": [
        { "kind": "session", "group": "session", "percent": 7, "severity": "normal",
          "resets_at": "2026-08-15T07:00:00.318695+00:00", "scope": null, "is_active": true },
        { "kind": "weekly_all", "group": "weekly", "percent": 4, "severity": "normal",
          "resets_at": "2026-08-19T16:00:00.318714+00:00", "scope": null, "is_active": false }
      ],
      "extra_usage": { "is_enabled": false, "monthly_limit": 20000, "used_credits": 0.0,
                       "utilization": 0.0, "currency": "USD" }
    }"#;

    #[test]
    fn utilization_is_percent_not_fraction() {
        // 端點的 utilization 是 0-100 的百分比。header 上的同名欄位才是 0-1 的
        // 小數。搞混會讓 7% 顯示成 700% 或 0.07%。
        let q = parse_usage("anthropic-pro", REAL, 1_786_760_000).expect("parse");
        let five = q.windows.iter().find(|w| w.label == "5h").expect("5h 窗");
        assert!((five.used_percent - 7.0).abs() < 1e-9, "得到 {}", five.used_percent);
    }

    #[test]
    fn maps_both_windows() {
        let q = parse_usage("anthropic-pro", REAL, 1_786_760_000).expect("parse");
        assert_eq!(q.windows.len(), 2);
        let seven = q.windows.iter().find(|w| w.label == "7d").expect("7d 窗");
        assert!((seven.used_percent - 4.0).abs() < 1e-9);
    }

    #[test]
    fn parses_rfc3339_resets_at_into_unix_seconds() {
        let q = parse_usage("anthropic-pro", REAL, 1_786_760_000).expect("parse");
        let five = q.windows.iter().find(|w| w.label == "5h").expect("5h 窗");
        // 2026-08-15T07:00:00Z。這個值有獨立佐證：同一份探勘回應的 header
        // `anthropic-ratelimit-unified-5h-reset` 就是 1786777200。
        assert_eq!(five.resets_at, Some(1_786_777_200));
    }

    #[test]
    fn active_limit_marks_primary_window() {
        // limits[] 裡 is_active 的那個就是代表窗（session → 5h）。
        let q = parse_usage("anthropic-pro", REAL, 1_786_760_000).expect("parse");
        assert_eq!(q.primary_window().expect("有代表窗").label, "5h");
    }

    #[test]
    fn adopts_upstream_severity() {
        let q = parse_usage("anthropic-pro", REAL, 1_786_760_000).expect("parse");
        assert_eq!(q.windows[0].severity, QuotaSeverity::Normal);
    }

    #[test]
    fn missing_seven_day_yields_single_window() {
        // 上游可能只回一個窗，不得因此整個解析失敗。
        let raw = r#"{"five_hour":{"utilization":50.0,"resets_at":null},
                      "seven_day":null,"limits":[]}"#;
        let q = parse_usage("p", raw, 0).expect("parse");
        assert_eq!(q.windows.len(), 1);
        assert_eq!(q.windows[0].label, "5h");
        // 沒有 limits[] 可參考時，靠 used_percent 自己推 severity。
        assert_eq!(q.windows[0].severity, QuotaSeverity::Normal);
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(parse_usage("p", "not json at all", 0).is_err());
    }
}
