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
    /// 排序用：越大越嚴重。
    pub fn rank(self) -> u8 {
        match self {
            QuotaSeverity::Normal => 0,
            QuotaSeverity::Warning => 1,
            QuotaSeverity::Critical => 2,
        }
    }

    /// 上游沒給 severity 時，依已用百分比推導。
    ///
    /// 門檻刻意不對稱：**90.0 屬於 Warning，90.1 才是 Critical**。
    /// 對齊成 `>= 90` 會讓 severity_thresholds 測試紅 —— 那是測試在擋，
    /// 不是測試寫錯。
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
            _ => {
                log::debug!("未知的上游 severity 字串 {s:?}，退回依百分比推導");
                Self::from_percent(used_percent)
            }
        }
    }
}

/// 單一配額窗的正規化快照。
#[derive(Debug, Clone, Serialize)]
pub struct QuotaWindow {
    /// 顯示用標籤，例如 "5h" / "7d" / "premium"。
    ///
    /// 由 adapter 決定且**直接顯示給使用者，不經 i18n**。
    pub label: String,
    /// 已使用百分比，**一律 0.0–100.0**。剩餘導向的來源由 adapter 換算。
    ///
    /// 這是 adapter 的義務，本型別不強制。adapter 必須確保：
    /// 上游欄位缺席時不要讓 serde 預設值算出一個假的 100%（那會顯示成
    /// 滿格紅燈），寧可跳過該窗。
    pub used_percent: f64,
    /// 重置時間（Unix 秒）。來源沒給就是 None。
    pub resets_at: Option<i64>,
    /// 嚴重度。
    ///
    /// **severity 不是 used_percent 的函數。** 上游若有明確的「已被擋住」
    /// 訊號（Codex 的 `limit_reached` / `allowed`、Anthropic 的
    /// `spend.severity`），adapter 必須把該窗提成 Critical —— 花費上限
    /// 觸發時 used_percent 可能還是 0，照百分比推會顯示成綠燈。
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
    /// 快照取得時間（Unix 秒）。
    ///
    /// **必須是本機取得快照的時間**，不可填上游回應裡的時間戳 ——
    /// 快取的 TTL 是 `now - fetched_at`，若上游時鐘比本機快，這個差值
    /// 會是負數而恆小於 TTL，快取就再也不會更新。
    pub fetched_at: i64,
}

impl ProviderQuota {
    /// 收合狀態要顯示的那個窗。
    ///
    /// **取最嚴重的窗**，而不是上游標記的代表窗 —— 5h 窗剛重置 0% 但 7d 窗
    /// 已 96% 時，顯示綠色 0% 會讓使用者誤以為還很寬裕。同嚴重度時才用
    /// `is_primary`（上游的代表窗）決定，再同則取第一個。
    ///
    /// **`is_primary` 的契約：一個 `ProviderQuota` 內至多一個窗為 true。**
    /// adapter 不得倚賴多個 true 時的 tie-break 順序。
    ///
    /// 注意：`src/ipc/usage.ts` 的 `primaryWindow()` 是同一條規則的 TS 實作，
    /// 生產路徑實際走的是 TS 那份。**改這裡就必須同步改那裡。**
    pub fn primary_window(&self) -> Option<&QuotaWindow> {
        self.windows.iter().fold(None, |best, w| match best {
            None => Some(w),
            Some(b) => {
                let bk = (b.severity.rank(), b.is_primary);
                let wk = (w.severity.rank(), w.is_primary);
                if wk > bk {
                    Some(w)
                } else {
                    Some(b)
                }
            }
        })
    }
}

/// 把窗長秒數轉成短標籤。**不要把「5 小時」寫死在任何地方** ——
/// Codex 的 free 方案是 30 天窗，Pro 是 5 小時窗。
///
/// `seconds <= 0` 代表上游沒給窗長（欄位缺席時 serde 預設為 0），回空字串
/// 讓 UI 只顯示百分比，而不是顯示「0d」這種假資訊。
pub fn window_label(seconds: i64) -> String {
    if seconds <= 0 {
        return String::new();
    }
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

use crate::config::types::ProviderType;
use crate::config::ConfigStore;
use crate::secret::SecretStore;

/// 依 provider 設定建立對應的 `QuotaSource`。
///
/// 回傳 `Ok(None)` 代表**這個 provider 沒有配額概念**（Ollama、所有 API key
/// 型），與查詢失敗是不同的狀態，呼叫端必須能分辨。
pub async fn source_for(
    provider_id: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
) -> Result<Option<Box<dyn QuotaSource>>, AiError> {
    let Some(cfg) = config.get_provider(provider_id) else {
        return Ok(None);
    };
    let is_oauth = cfg.auth_method.as_deref() == Some("oauth");

    match cfg.provider_type {
        ProviderType::Anthropic if is_oauth => {
            let token = crate::ai::router::get_valid_oauth_token(provider_id, secrets).await?;
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            Ok(Some(Box::new(anthropic::AnthropicQuota::new(
                provider_id.into(),
                token,
                base,
            ))))
        }
        ProviderType::Codex => {
            let (token, account_id) =
                crate::ai::router::get_valid_codex_oauth_token(provider_id, secrets).await?;
            Ok(Some(Box::new(codex::CodexQuota::new(
                provider_id.into(),
                token,
                account_id,
            ))))
        }
        ProviderType::GithubCopilot => {
            let token = secrets
                .get(provider_id)
                .map_err(|_| AiError::NotConfigured)?
                .ok_or(AiError::NotConfigured)?;
            Ok(Some(Box::new(copilot::CopilotQuota::new(provider_id.into(), token))))
        }
        // Ollama、OpenaiCompatible、所有 API key 型：沒有訂閱配額概念。
        _ => Ok(None),
    }
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
        // 上游沒給窗長（serde 預設 0）時不要編出「0d」這種假標籤。
        assert_eq!(window_label(0), "");
        assert_eq!(window_label(-1800), "");
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
    fn primary_window_prefers_the_most_severe_one() {
        // 5h 窗剛重置 0%（且被上游標成代表窗），7d 窗已經 96%。
        // 收合徽章必須顯示 7d，否則使用者看到綠色 0% 會以為還很寬裕。
        let q = ProviderQuota {
            provider_id: "p".into(),
            plan: None,
            windows: vec![
                QuotaWindow { label: "5h".into(), used_percent: 0.0, resets_at: None,
                              severity: QuotaSeverity::Normal, detail: None, is_primary: true },
                QuotaWindow { label: "7d".into(), used_percent: 96.0, resets_at: None,
                              severity: QuotaSeverity::Critical, detail: None, is_primary: false },
            ],
            fetched_at: 0,
        };
        assert_eq!(q.primary_window().expect("有窗").label, "7d");
    }

    #[test]
    fn equal_severity_falls_back_to_the_flagged_window() {
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
