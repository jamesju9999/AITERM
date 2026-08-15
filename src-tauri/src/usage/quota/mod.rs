//! 上游訂閱配額的查詢與正規化。
//!
//! 三家的語意各不相同（百分比 vs 剩餘次數、一窗 vs 兩窗、窗長不一），
//! 各 adapter 負責換算成本模組的共通形狀，**UI 只認這個形狀**。

pub mod anthropic;
pub mod cache;
pub mod codex;
pub mod copilot;

use crate::ai::AiError;
use async_trait::async_trait;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSeverity {
    Normal,
    Warning,
    Critical,
}

impl QuotaSeverity {
    /// 上游沒給 severity 時，依已用百分比推導。
    pub fn from_percent(used_percent: f64) -> Self {
        if used_percent > 90.0 {
            QuotaSeverity::Critical
        } else if used_percent >= 75.0 {
            QuotaSeverity::Warning
        } else {
            QuotaSeverity::Normal
        }
    }

    /// 優先採用上游自己的 severity 字串，不認得才退回推導。
    pub fn from_upstream(s: &str, used_percent: f64) -> Self {
        match s {
            "normal" => QuotaSeverity::Normal,
            "warning" => QuotaSeverity::Warning,
            "critical" => QuotaSeverity::Critical,
            _ => Self::from_percent(used_percent),
        }
    }
}

/// 單一配額窗的正規化快照。
#[derive(Debug, Clone, Serialize)]
pub struct QuotaWindow {
    /// 顯示用標籤，例如 "5h" / "7d" / "premium"。
    pub label: String,
    /// 已使用百分比，**一律 0.0–100.0**。剩餘導向的來源由 adapter 換算。
    pub used_percent: f64,
    /// 重置時間（Unix 秒）。來源沒給就是 None。
    pub resets_at: Option<i64>,
    pub severity: QuotaSeverity,
    /// 保留原始語意的補充說明，例如 Copilot 的 "142 / 300"。
    pub detail: Option<String>,
    /// 收合狀態只顯示一個窗，由這個旗標決定顯示哪個。
    pub is_primary: bool,
}

/// 一個 provider 的完整配額快照。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderQuota {
    pub provider_id: String,
    /// 方案別，顯示用（"Claude Pro" / "free" / "yearly_subscriber_quota"）。
    pub plan: Option<String>,
    /// 可能 0、1 或 2 個窗。**空陣列代表查得到但沒有配額限制**
    /// （Copilot 的全無限方案），與「查詢失敗」是不同的狀態。
    pub windows: Vec<QuotaWindow>,
    /// 快照取得時間（Unix 秒），UI 用來顯示「幾分鐘前」。
    pub fetched_at: i64,
}

impl ProviderQuota {
    /// 收合狀態要顯示的那個窗：有 `is_primary` 就用它，否則取第一個。
    pub fn primary_window(&self) -> Option<&QuotaWindow> {
        self.windows
            .iter()
            .find(|w| w.is_primary)
            .or_else(|| self.windows.first())
    }
}

/// 把窗長秒數轉成短標籤。**不要把「5 小時」寫死在任何地方** ——
/// Codex 的 free 方案是 30 天窗，Pro 是 5 小時窗。
pub fn window_label(seconds: i64) -> String {
    if seconds % 86_400 == 0 {
        format!("{}d", seconds / 86_400)
    } else if seconds % 3_600 == 0 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}m", seconds / 60)
    }
}

#[async_trait]
pub trait QuotaSource: Send + Sync {
    async fn fetch(&self) -> Result<ProviderQuota, AiError>;
}

/// 現在時間（Unix 秒）。
pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_thresholds() {
        assert!(matches!(QuotaSeverity::from_percent(0.0), QuotaSeverity::Normal));
        assert!(matches!(QuotaSeverity::from_percent(74.9), QuotaSeverity::Normal));
        assert!(matches!(QuotaSeverity::from_percent(75.0), QuotaSeverity::Warning));
        assert!(matches!(QuotaSeverity::from_percent(90.0), QuotaSeverity::Warning));
        assert!(matches!(QuotaSeverity::from_percent(90.1), QuotaSeverity::Critical));
        assert!(matches!(QuotaSeverity::from_percent(100.0), QuotaSeverity::Critical));
    }

    #[test]
    fn upstream_severity_string_is_preferred_over_derived() {
        // Anthropic 自己會給 severity，直接採用它而不是自己推。
        assert!(matches!(QuotaSeverity::from_upstream("normal", 99.0), QuotaSeverity::Normal));
        assert!(matches!(QuotaSeverity::from_upstream("warning", 1.0), QuotaSeverity::Warning));
        assert!(matches!(QuotaSeverity::from_upstream("critical", 1.0), QuotaSeverity::Critical));
        // 不認得的字串才退回自己推。
        assert!(matches!(QuotaSeverity::from_upstream("weird", 95.0), QuotaSeverity::Critical));
    }

    #[test]
    fn window_label_from_seconds() {
        assert_eq!(window_label(18_000), "5h");
        assert_eq!(window_label(604_800), "7d");
        assert_eq!(window_label(2_592_000), "30d");
        // 沒見過的長度也要有合理輸出，不能 panic 或回空字串。
        assert_eq!(window_label(3_600), "1h");
        assert_eq!(window_label(172_800), "2d");
    }

    #[test]
    fn primary_window_falls_back_to_first() {
        let q = ProviderQuota {
            provider_id: "p".into(),
            plan: None,
            windows: vec![
                QuotaWindow { label: "a".into(), used_percent: 1.0, resets_at: None,
                              severity: QuotaSeverity::Normal, detail: None, is_primary: false },
                QuotaWindow { label: "b".into(), used_percent: 2.0, resets_at: None,
                              severity: QuotaSeverity::Normal, detail: None, is_primary: false },
            ],
            fetched_at: 0,
        };
        assert_eq!(q.primary_window().expect("有窗").label, "a");
    }

    #[test]
    fn primary_window_prefers_flagged_one() {
        let q = ProviderQuota {
            provider_id: "p".into(),
            plan: None,
            windows: vec![
                QuotaWindow { label: "a".into(), used_percent: 1.0, resets_at: None,
                              severity: QuotaSeverity::Normal, detail: None, is_primary: false },
                QuotaWindow { label: "b".into(), used_percent: 2.0, resets_at: None,
                              severity: QuotaSeverity::Normal, detail: None, is_primary: true },
            ],
            fetched_at: 0,
        };
        assert_eq!(q.primary_window().expect("有窗").label, "b");
    }

    #[test]
    fn no_windows_means_no_primary() {
        // Copilot 的無限方案會落在這裡：查得到，但沒有配額限制。
        let q = ProviderQuota {
            provider_id: "p".into(), plan: None, windows: vec![], fetched_at: 0,
        };
        assert!(q.primary_window().is_none());
    }
}
