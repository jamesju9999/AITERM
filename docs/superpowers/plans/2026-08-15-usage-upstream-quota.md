# 上游配額顯示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `ModelPickerButton` 上常駐顯示目前選中 provider 的訂閱配額剩餘，並在 Settings 用量頁列出所有 provider 的配額狀態。

**Architecture:** 三個 `QuotaSource` adapter（Anthropic OAuth / Codex / GitHub Copilot）把各自語意不同的上游回應**正規化成同一個 `ProviderQuota` 形狀**，UI 只認這個形狀。外層加 60 秒 TTL 的記憶體快取，避免多視圖同時掛載時重複打網路。

**Tech Stack:** Rust / reqwest / async_trait / wiremock / Tauri 2 IPC / React 19 / Vitest

**規格來源：** `docs/superpowers/specs/2026-08-15-model-usage-display-design.md`

**與另一份計畫的關係：** 與 `2026-08-15-usage-local-accounting.md` **互不依賴**，可並行。唯一的交會點是 Task 8（`ModelPickerButton` 對沒有配額概念的 provider 顯示今日 token 數）與 Task 9（用量頁的配額區塊）—— 若本地累計計畫尚未完成，那兩處先顯示「—」，不要卡住。

**探勘實證：** 本計畫所有 fixture 都取自 `src-tauri/tests/usage_probe.rs` 的真實回應 dump。**不要自己編 fixture** —— 自編會把錯誤的假設固化成「通過的測試」。若 dump 檔已不在 scratchpad，重跑：

```bash
cd src-tauri && cargo test --test usage_probe -- --ignored --nocapture
```

---

## File Structure

| 檔案 | 責任 |
|---|---|
| `src-tauri/src/usage/quota/mod.rs`（新增） | `ProviderQuota` / `QuotaWindow` 型別、`QuotaSource` trait、adapter 工廠 |
| `src-tauri/src/usage/quota/anthropic.rs`（新增） | Anthropic OAuth 的 `/api/oauth/usage` 解析 |
| `src-tauri/src/usage/quota/codex.rs`（新增） | Codex 的 `/backend-api/codex/usage` 解析 |
| `src-tauri/src/usage/quota/copilot.rs`（新增） | Copilot 的 `/copilot_internal/user` 解析 |
| `src-tauri/src/usage/quota/cache.rs`（新增） | 60 秒 TTL 的記憶體快取 |
| `src-tauri/src/commands/usage.rs`（修改/新增） | `usage_quota`、`usage_quota_all` IPC |
| `src-tauri/tests/quota_wiremock.rs`（新增） | adapter 對假伺服器的整合測試 |
| `src/ipc/usage.ts`（修改/新增） | 前端型別與 invoke 包裝 |
| `src/components/ModelPickerButton.tsx`（修改） | 常駐徽章 + 下拉列徽章 + 抓取時機 |
| `src/components/QuotaBadge.tsx`（新增） | 徽章元件，收合與展開共用 |
| `src/components/QuotaBadge.css`（新增） | 樣式 |
| `src/components/Settings/UsagePage.tsx`（修改） | 配額區塊 |
| `src/lib/i18n.ts`（修改） | en / zh-TW 字串 |

> `src-tauri/src/usage/mod.rs` 若尚不存在（本地累計計畫尚未執行），本計畫的 Task 1 會建立它。兩份計畫都動到這個檔案，並行時注意合併。

---

## Task 1: 正規化型別與 `QuotaSource` trait

三家的配額語意不同：Anthropic 是**兩個時間窗的使用百分比**，Codex 是**一個窗的 used_percent**（窗長依方案 5 小時或 30 天），Copilot 是**剩餘次數**（142/300）。不能塞進同一個扁平結構，必須正規化。

**Files:**
- Create: `src-tauri/src/usage/quota/mod.rs`
- Modify: `src-tauri/src/usage/mod.rs`（若不存在則建立）

- [ ] **Step 1: 建立模組**

若 `src-tauri/src/usage/mod.rs` 不存在，建立：

```rust
//! 用量記帳與配額查詢。

pub mod quota;
```

若已存在（本地累計計畫已執行），只加一行 `pub mod quota;`。

確認 `src-tauri/src/lib.rs` 有 **`pub mod usage;`**（與 `pub mod ai;`、`pub mod db;` 同區塊，`lib.rs:1-9`），沒有就加。

> 必須是 `pub`：`src-tauri/tests/` 下的整合測試是獨立 crate，Task 6 的 wiremock 測試要以 `aiterm_lib::usage::quota::…` 存取。

- [ ] **Step 2: 寫失敗的測試**

`src-tauri/src/usage/quota/mod.rs`：

```rust
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
```

- [ ] **Step 3: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib usage::quota 2>&1 | tail -10`
Expected: 編譯失敗，`cannot find type ProviderQuota`

- [ ] **Step 4: 實作**

在測試模組之前加入：

```rust
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
```

- [ ] **Step 5: 建立三個 adapter 的空檔案讓它編得過**

`anthropic.rs`、`codex.rs`、`copilot.rs`、`cache.rs` 各建立一個只有模組註解的空檔（內容在後續 Task 填）。

- [ ] **Step 6: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib usage::quota 2>&1 | tail -8`
Expected: `test result: ok. 6 passed`

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/usage/mod.rs src-tauri/src/usage/quota/
git commit -m "feat(quota): 配額的正規化型別與 QuotaSource trait

三家語意不同（百分比/剩餘次數、一窗/兩窗、窗長不一），統一成
QuotaWindow { label, used_percent 0-100, resets_at, severity, detail }。
窗長標籤一律由秒數推導 —— Codex free 方案是 30 天窗不是 5 小時。"
```

---

## Task 2: Anthropic adapter

**Files:**
- Modify: `src-tauri/src/usage/quota/anthropic.rs`

真實回應（探勘 dump `probe_anthropic_usage.txt`）：

```json
{
  "five_hour": { "utilization": 7.0, "resets_at": "2026-08-15T07:00:00.318695+00:00" },
  "seven_day": { "utilization": 4.0, "resets_at": "2026-08-19T16:00:00.318714+00:00" },
  "limits": [
    { "kind": "session", "percent": 7, "severity": "normal",
      "resets_at": "2026-08-15T07:00:00.318695+00:00", "is_active": true },
    { "kind": "weekly_all", "percent": 4, "severity": "normal",
      "resets_at": "2026-08-19T16:00:00.318714+00:00", "is_active": false }
  ]
}
```

> ⚠️ **`utilization` 是百分比（`7.0` = 7%），不是小數。** 回應 header 上的同名欄位才是小數（`0.06`）。這是本計畫最容易寫錯的地方，Step 1 的第一個測試就是為了釘死它。

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/usage/quota/anthropic.rs`：

```rust
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
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib usage::quota::anthropic 2>&1 | tail -10`
Expected: 編譯失敗，`cannot find function parse_usage`

- [ ] **Step 3: 確認 RFC3339 解析用的相依已存在**

Run: `cd src-tauri && grep -n "^chrono\|^time " Cargo.toml`
Expected: 出現 `chrono`。若沒有，改用 `time` crate；若兩者皆無，加 `chrono = "0.4"` 到 `[dependencies]`。下方程式碼以 `chrono` 撰寫。

- [ ] **Step 4: 實作**

在測試模組之前加入：

```rust
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
```

- [ ] **Step 5: 抽出共用的 header 常數**

`src-tauri/src/ai/anthropic.rs:53` 目前把 `"claude-code-20250219,oauth-2025-04-20"` 寫成字面值。抽成 `pub const OAUTH_BETA_HEADER`，並確認 `ANTHROPIC_VERSION` 已是 `pub`（目前是私有 const，改成 `pub const`）。原本使用該字面值的地方改用常數。

Run: `cd src-tauri && grep -n "claude-code-20250219\|ANTHROPIC_VERSION" src/ai/anthropic.rs`

確認改完後**沒有任何地方還在用字面值** —— 兩份定義各改一半是最典型的 bug 溫床。

- [ ] **Step 6: 確認 `AiError` 變體形狀無誤**

Run: `cd src-tauri && grep -n "pub enum AiError" -A 22 src/ai/mod.rs`

本計畫的程式碼已對照 `ai/mod.rs:32-53` 寫成：`Network { message }`、`ModelError { reason, raw }`、**`AuthFailed` 是無欄位的單元變體**。若上游定義已變動，以實際定義為準。

- [ ] **Step 7: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib usage::quota::anthropic 2>&1 | tail -8`
Expected: `test result: ok. 7 passed`

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/usage/quota/anthropic.rs src-tauri/src/ai/anthropic.rs
git commit -m "feat(quota): Anthropic OAuth 配額 adapter

/api/oauth/usage 的 utilization 是百分比（7.0=7%），header 上的同名
欄位才是小數（0.06）。測試釘死這一點。
OAuth beta header 抽成共用常數，避免兩份定義各改一半。"
```

---

## Task 3: Codex adapter

**Files:**
- Modify: `src-tauri/src/usage/quota/codex.rs`

真實回應（探勘 dump `probe_codex_usage.txt`）：

```json
{
  "plan_type": "free",
  "rate_limit": {
    "allowed": true, "limit_reached": false,
    "primary_window": { "used_percent": 0, "limit_window_seconds": 2592000,
                        "reset_after_seconds": 2268666, "reset_at": 1789029443 },
    "secondary_window": null
  }
}
```

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/usage/quota/codex.rs`：

```rust
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
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib usage::quota::codex 2>&1 | tail -10`
Expected: 編譯失敗，`cannot find function parse_usage`

- [ ] **Step 3: 實作**

```rust
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
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib usage::quota::codex 2>&1 | tail -8`
Expected: `test result: ok. 11 passed`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/usage/quota/codex.rs
git commit -m "feat(quota): Codex 配額 adapter

secondary_window 可能為 null（實測即是）；窗長標籤由
limit_window_seconds 推導，free 方案是 30 天窗不是 5 小時。"
```

---

## Task 4: Copilot adapter

**Files:**
- Modify: `src-tauri/src/usage/quota/copilot.rs`

真實回應（探勘 dump `probe_copilot_quota.txt`）：

```json
{
  "access_type_sku": "yearly_subscriber_quota",
  "quota_reset_date_utc": "2026-09-01T00:00:00.000Z",
  "quota_snapshots": {
    "chat":        { "unlimited": true,  "percent_remaining": 100.0 },
    "completions": { "unlimited": true,  "percent_remaining": 100.0 },
    "premium_interactions": { "unlimited": false, "entitlement": 300,
                              "remaining": 142, "percent_remaining": 47.5 }
  }
}
```

> Copilot 是三家裡唯一**剩餘導向**的：`percent_remaining: 47.5` 要換算成 `used_percent: 52.5`。

- [ ] **Step 1: 寫失敗的測試**

```rust
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
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib usage::quota::copilot 2>&1 | tail -10`
Expected: 編譯失敗，`cannot find function parse_user`

- [ ] **Step 3: 實作**

```rust
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
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib usage::quota::copilot 2>&1 | tail -8`
Expected: `test result: ok. 10 passed`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/usage/quota/copilot.rs
git commit -m "feat(quota): GitHub Copilot 配額 adapter

三家裡唯一剩餘導向的：percent_remaining 47.5 換算成 used_percent 52.5，
直接當已用會完全顛倒。unlimited 的 snapshot 不產生窗。
Authorization 用 token 而非 Bearer（這是 GitHub 端點）。"
```

---

## Task 5: 快取與 adapter 工廠

**Files:**
- Modify: `src-tauri/src/usage/quota/cache.rs`
- Modify: `src-tauri/src/usage/quota/mod.rs`（加工廠函式）

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/usage/quota/cache.rs`：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSource {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl QuotaSource for CountingSource {
        async fn fetch(&self) -> Result<ProviderQuota, AiError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ProviderQuota {
                provider_id: "p".into(), plan: None, windows: vec![], fetched_at: 0,
            })
        }
    }

    #[tokio::test]
    async fn second_call_within_ttl_does_not_refetch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = QuotaCache::new();
        let src = CountingSource { calls: calls.clone() };

        cache.get_or_fetch("p", &src, false, 1_000).await.expect("first");
        cache.get_or_fetch("p", &src, false, 1_030).await.expect("second");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "30 秒內不該重打");
    }

    #[tokio::test]
    async fn call_after_ttl_refetches() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = QuotaCache::new();
        let src = CountingSource { calls: calls.clone() };

        cache.get_or_fetch("p", &src, false, 1_000).await.expect("first");
        cache.get_or_fetch("p", &src, false, 1_061).await.expect("second");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "超過 60 秒要重打");
    }

    #[tokio::test]
    async fn force_bypasses_cache() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = QuotaCache::new();
        let src = CountingSource { calls: calls.clone() };

        cache.get_or_fetch("p", &src, false, 1_000).await.expect("first");
        cache.get_or_fetch("p", &src, true, 1_001).await.expect("forced");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn different_providers_have_separate_entries() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = QuotaCache::new();
        let src = CountingSource { calls: calls.clone() };

        cache.get_or_fetch("a", &src, false, 1_000).await.expect("a");
        cache.get_or_fetch("b", &src, false, 1_001).await.expect("b");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "不同 provider 不共用快取");
    }
}
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib usage::quota::cache 2>&1 | tail -10`
Expected: 編譯失敗，`cannot find type QuotaCache`

- [ ] **Step 3: 實作**

```rust
//! 配額快照的記憶體快取。
//!
//! 配額變動以分鐘計，但 UI 每次開下拉就要顯示、又有 5 分鐘的背景輪詢，
//! 多個視圖同時掛載時若不快取會重複打上游。60 秒 TTL 讓「多視圖同時掛載」
//! 只產生一次實際請求。

use super::{ProviderQuota, QuotaSource};
use crate::ai::AiError;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const TTL_SECS: i64 = 60;

#[derive(Default)]
pub struct QuotaCache {
    entries: Mutex<HashMap<String, ProviderQuota>>,
}

impl QuotaCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 快取命中且未過期就直接回；否則呼叫 `source.fetch()`。
    /// `now` 由呼叫端注入以便測試。
    pub async fn get_or_fetch(
        &self,
        provider_id: &str,
        source: &dyn QuotaSource,
        force: bool,
        now: i64,
    ) -> Result<ProviderQuota, AiError> {
        if !force {
            let guard = self.entries.lock().unwrap();
            if let Some(hit) = guard.get(provider_id) {
                if now - hit.fetched_at < TTL_SECS {
                    return Ok(hit.clone());
                }
            }
        }
        let mut fresh = source.fetch().await?;
        // fetched_at 以呼叫端的時鐘為準，避免 adapter 與快取用到不同時間源。
        fresh.fetched_at = now;
        self.entries
            .lock()
            .unwrap()
            .insert(provider_id.to_string(), fresh.clone());
        Ok(fresh)
    }
}
```

> `Mutex` 的 guard 在 `.await` 之前就 drop 了（`if !force { ... }` 區塊結束），所以不會跨 await 持鎖。**不要把 guard 的生存期延長到 `source.fetch().await`** —— 那會在多視圖並發時死鎖。

- [ ] **Step 4: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib usage::quota::cache 2>&1 | tail -8`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: 加上 adapter 工廠**

在 `src-tauri/src/usage/quota/mod.rs` 加：

```rust
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
```

- [ ] **Step 6: 對照實際 API 修正**

Run: `cd src-tauri && grep -n "pub fn get_provider" -A 4 src/config/mod.rs`
Run: `cd src-tauri && grep -n "auth_method" src/config/types.rs | head -3`

確認 `get_provider` 的回傳型別與 `auth_method` 的欄位型別，照實際的調整。上方 `.map_err(|e| AiError::NotConfigured)` 會產生 unused variable 警告，改成 `.map_err(|_| AiError::NotConfigured)`。

- [ ] **Step 7: 編譯**

Run: `cd src-tauri && cargo check 2>&1 | grep -E "^(error|warning: unused)" -A 5`
Expected: 無輸出

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/usage/quota/cache.rs src-tauri/src/usage/quota/mod.rs
git commit -m "feat(quota): 60 秒 TTL 快取與 adapter 工廠

工廠回 Ok(None) 表示「沒有配額概念」，與查詢失敗是不同狀態。
快取的 Mutex guard 不跨 await 持有，避免多視圖並發時死鎖。"
```

---

## Task 6: wiremock 整合測試

**Files:**
- Create: `src-tauri/tests/quota_wiremock.rs`

- [ ] **Step 1: 寫測試**

```rust
//! 三個配額 adapter 對假伺服器的整合測試。
//!
//! 單元測試已覆蓋解析邏輯；這裡驗證的是 HTTP 層：狀態碼分流、header、
//! 逾時。回應主體一律取自探勘 dump 的真實形狀。

use aiterm_lib::ai::AiError;
use aiterm_lib::usage::quota::{codex::CodexQuota, copilot::CopilotQuota, QuotaSource};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CODEX_BODY: &str = r#"{"plan_type":"free","rate_limit":{
    "primary_window":{"used_percent":12,"limit_window_seconds":18000,"reset_at":1789029443},
    "secondary_window":null}}"#;

#[tokio::test]
async fn codex_quota_sends_auth_and_account_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(header("authorization", "Bearer tok"))
        .and(header("chatgpt-account-id", "acc-1"))
        .and(header("originator", "codex_cli_rs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(CODEX_BODY))
        .mount(&server)
        .await;

    let q = CodexQuota::new("GPT5.6".into(), "tok".into(), Some("acc-1".into()))
        .with_url(server.uri());
    let quota = q.fetch().await.expect("fetch");
    assert_eq!(quota.windows.len(), 1);
    assert_eq!(quota.windows[0].label, "5h");
}

#[tokio::test]
async fn unauthorized_maps_to_auth_failed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{}"))
        .mount(&server)
        .await;

    let q = CodexQuota::new("p".into(), "tok".into(), None).with_url(server.uri());
    let err = q.fetch().await.expect_err("應該失敗");
    assert!(matches!(err, AiError::AuthFailed), "得到 {err:?}");
}

#[tokio::test]
async fn not_found_is_an_error_not_a_panic() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let q = CodexQuota::new("p".into(), "tok".into(), None).with_url(server.uri());
    assert!(q.fetch().await.is_err());
}

#[tokio::test]
async fn html_body_on_200_is_a_parse_error_not_a_panic() {
    // Cloudflare 擋下來時會回 200 + HTML。必須降級成錯誤，不能 panic。
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>challenge</html>"))
        .mount(&server)
        .await;

    let q = CodexQuota::new("p".into(), "tok".into(), None).with_url(server.uri());
    assert!(q.fetch().await.is_err());
}

#[tokio::test]
async fn timeout_maps_to_network_error() {
    // 逾時用可注入的 100ms，不要讓 CI 每次真的等 5 秒。
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(CODEX_BODY)
                .set_delay(std::time::Duration::from_secs(2)),
        )
        .mount(&server)
        .await;

    let q = CodexQuota::new("p".into(), "tok".into(), None)
        .with_url(server.uri())
        .with_timeout(std::time::Duration::from_millis(100));
    let err = q.fetch().await.expect_err("應該逾時");
    assert!(matches!(err, AiError::Network { .. }), "得到 {err:?}");
}

#[tokio::test]
async fn copilot_uses_token_scheme_not_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", "token ghp_x"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_type_sku":"x","quota_snapshots":{"premium_interactions":
               {"unlimited":false,"percent_remaining":47.5,"entitlement":300,"remaining":142}}}"#,
        ))
        .mount(&server)
        .await;

    let q = CopilotQuota::new("gh".into(), "ghp_x".into()).with_url(server.uri());
    let quota = q.fetch().await.expect("fetch");
    assert!((quota.windows[0].used_percent - 52.5).abs() < 1e-9);
}
```

- [ ] **Step 2: 確認 `AnthropicQuota` 也能指向假伺服器**

`AnthropicQuota::new` 已收 `base_url`，可直接傳 `server.uri()`。加一個測試：

```rust
use aiterm_lib::usage::quota::anthropic::AnthropicQuota;

#[tokio::test]
async fn anthropic_quota_hits_oauth_usage_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/oauth/usage"))
        .and(header("x-app", "cli"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"five_hour":{"utilization":7.0,"resets_at":null},
                "seven_day":null,"limits":[]}"#,
        ))
        .mount(&server)
        .await;

    let q = AnthropicQuota::new("anthropic-pro".into(), "tok".into(), server.uri());
    let quota = q.fetch().await.expect("fetch");
    assert!((quota.windows[0].used_percent - 7.0).abs() < 1e-9);
}
```

- [ ] **Step 3: 跑測試**

Run: `cd src-tauri && cargo test --test quota_wiremock 2>&1 | tail -10`
Expected: `test result: ok. 7 passed`

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/quota_wiremock.rs
git commit -m "test(quota): adapter 的 HTTP 層整合測試

涵蓋 200/401/404、header 正確性，以及 200 + HTML（Cloudflare 擋下時的
實際行為）必須降級成錯誤而非 panic。"
```

---

## Task 7: IPC 指令

**Files:**
- Modify/Create: `src-tauri/src/commands/usage.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 實作**

加到 `src-tauri/src/commands/usage.rs`（檔案若不存在則建立，並在 `commands/mod.rs` 加 `pub mod usage;`）：

```rust
use crate::config::ConfigStore;
use crate::secret::SecretStore;
use crate::usage::quota::{cache::QuotaCache, source_for, ProviderQuota};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

/// 單一 provider 的查詢結果。
///
/// 三種狀態必須能分辨：查到了、沒有配額概念、查詢失敗。
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QuotaResult {
    Ok { quota: ProviderQuota },
    /// 這個 provider 沒有訂閱配額概念（Ollama、API key 型）。
    NotApplicable { provider_id: String },
    Failed { provider_id: String, message: String },
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn fetch_one(
    provider_id: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    cache: &QuotaCache,
    force: bool,
) -> QuotaResult {
    let source = match source_for(provider_id, config, secrets).await {
        Ok(Some(s)) => s,
        Ok(None) => return QuotaResult::NotApplicable { provider_id: provider_id.into() },
        Err(e) => {
            return QuotaResult::Failed {
                provider_id: provider_id.into(),
                message: format!("{e:?}"),
            }
        }
    };
    match cache.get_or_fetch(provider_id, source.as_ref(), force, now_secs()).await {
        Ok(quota) => QuotaResult::Ok { quota },
        Err(e) => QuotaResult::Failed {
            provider_id: provider_id.into(),
            message: format!("{e:?}"),
        },
    }
}

#[tauri::command]
pub async fn usage_quota(
    provider_id: String,
    force: bool,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    cache: State<'_, Arc<QuotaCache>>,
) -> Result<QuotaResult, String> {
    Ok(fetch_one(&provider_id, &config, &secrets, &cache, force).await)
}

#[tauri::command]
pub async fn usage_quota_all(
    force: bool,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    cache: State<'_, Arc<QuotaCache>>,
) -> Result<Vec<QuotaResult>, String> {
    let ids: Vec<String> = config.get().providers.iter().map(|p| p.id.clone()).collect();
    let mut out = Vec::with_capacity(ids.len());
    // 逐一查而非 join_all：一次打三個上游端點沒有實質好處，而且逐一查
    // 讓某一家卡住時不會拖累其他家的 5 秒逾時疊加。
    for id in ids {
        out.push(fetch_one(&id, &config, &secrets, &cache, force).await);
    }
    Ok(out)
}
```

> `usage_quota_all` 回 `Vec<QuotaResult>` 而非 `Result<Vec<_>>`：一個 provider 的 token 過期不該讓整批查詢失敗。

- [ ] **Step 2: 註冊 state 與 handler**

`src-tauri/src/lib.rs`：

在 `.manage()` 區塊加：

```rust
        .manage(Arc::new(crate::usage::quota::cache::QuotaCache::new()))
```

在 `tauri::generate_handler![...]` 加：

```rust
        commands::usage::usage_quota,
        commands::usage::usage_quota_all,
```

- [ ] **Step 3: 確認 `ConfigStore` / `SecretStore` 的 state 型別對得上**

Run: `cd src-tauri && grep -n "\.manage(config)\|\.manage(secrets)" -B 5 src/lib.rs`

若它們是以 `Arc<ConfigStore>` 註冊就照上面寫；若是裸值，把 `State<'_, Arc<ConfigStore>>` 改成 `State<'_, ConfigStore>`。**以實際註冊方式為準。**

- [ ] **Step 4: 編譯與測試**

Run: `cd src-tauri && cargo test 2>&1 | tail -10`
Expected: 全過

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/usage.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(quota): usage_quota / usage_quota_all IPC

QuotaResult 是三態（ok / not_applicable / failed）：沒有配額概念與
查詢失敗必須能分辨。批次查詢回 Vec 而非 Result，一家過期不拖垮全部。"
```

---

## Task 8: `QuotaBadge` 與 `ModelPickerButton` 常駐顯示

**Files:**
- Create: `src/components/QuotaBadge.tsx`, `QuotaBadge.css`, `QuotaBadge.test.tsx`
- Modify: `src/components/ModelPickerButton.tsx`
- Modify/Create: `src/ipc/usage.ts`
- Create: `src/components/ModelPickerButton.quota.test.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 前端型別**

`src/ipc/usage.ts` **可能已存在**（本地累計計畫的 Task 9 會建立它並放入 `usageSummary`）。
若已存在就**附加**下列內容，保留既有的 export，不要整檔覆寫；若不存在則建立：

```ts
import { invoke } from "@tauri-apps/api/core";

export type QuotaSeverity = "normal" | "warning" | "critical";

export interface QuotaWindow {
  label: string;
  /** 已使用百分比，0–100。 */
  used_percent: number;
  resets_at: number | null;
  severity: QuotaSeverity;
  /** Copilot 的 "142 / 300" 之類的原始語意。 */
  detail: string | null;
  is_primary: boolean;
}

export interface ProviderQuota {
  provider_id: string;
  plan: string | null;
  /** 空陣列 = 查得到但沒有配額限制，與查詢失敗不同。 */
  windows: QuotaWindow[];
  fetched_at: number;
}

export type QuotaResult =
  | { status: "ok"; quota: ProviderQuota }
  | { status: "not_applicable"; provider_id: string }
  | { status: "failed"; provider_id: string; message: string };

export function usageQuota(providerId: string, force = false): Promise<QuotaResult> {
  return invoke<QuotaResult>("usage_quota", { providerId, force });
}

export function usageQuotaAll(force = false): Promise<QuotaResult[]> {
  return invoke<QuotaResult[]>("usage_quota_all", { force });
}

const SEVERITY_RANK: Record<QuotaSeverity, number> = {
  normal: 0,
  warning: 1,
  critical: 2,
};

/**
 * 收合狀態要顯示的那個窗。
 *
 * **取最嚴重的窗**，而不是上游標記的代表窗 —— 5h 窗剛重置 0% 但 7d 窗已
 * 96% 時，顯示綠色 0% 會讓使用者誤以為還很寬裕。同嚴重度時才用 is_primary
 * 決定，再同則取第一個。
 *
 * 這條規則在 `src-tauri/src/usage/quota/mod.rs` 的 `ProviderQuota::primary_window`
 * 有一份對應的 Rust 實作。**改這裡就必須同步改那裡。**
 */
export function primaryWindow(q: ProviderQuota): QuotaWindow | null {
  let best: QuotaWindow | null = null;
  for (const w of q.windows) {
    if (best === null) { best = w; continue; }
    const d = SEVERITY_RANK[w.severity] - SEVERITY_RANK[best.severity];
    if (d > 0 || (d === 0 && w.is_primary && !best.is_primary)) best = w;
  }
  return best;
}
```

- [ ] **Step 2: 寫 `QuotaBadge` 的失敗測試**

`src/components/QuotaBadge.test.tsx`：

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { QuotaBadge } from "./QuotaBadge";
import type { QuotaWindow } from "../ipc/usage";

const win = (over: Partial<QuotaWindow> = {}): QuotaWindow => ({
  label: "5h", used_percent: 7, resets_at: null,
  severity: "normal", detail: null, is_primary: true, ...over,
});

describe("QuotaBadge", () => {
  it("normal 時仍然顯示（A 案的核心，不是只有超標才出現）", () => {
    render(<QuotaBadge window={win({ severity: "normal", used_percent: 7 })} />);
    expect(screen.getByTestId("quota-badge")).toHaveTextContent("5h 7%");
  });

  it("依 severity 套用對應的 class", () => {
    const { rerender } = render(<QuotaBadge window={win({ severity: "normal" })} />);
    expect(screen.getByTestId("quota-badge").className).toContain("normal");
    rerender(<QuotaBadge window={win({ severity: "warning" })} />);
    expect(screen.getByTestId("quota-badge").className).toContain("warning");
    rerender(<QuotaBadge window={win({ severity: "critical" })} />);
    expect(screen.getByTestId("quota-badge").className).toContain("critical");
  });

  it("有 detail 時優先顯示原始語意", () => {
    render(<QuotaBadge window={win({ label: "premium", detail: "142 / 300" })} />);
    expect(screen.getByTestId("quota-badge")).toHaveTextContent("premium 142 / 300");
  });

  it("百分比四捨五入到整數", () => {
    render(<QuotaBadge window={win({ used_percent: 52.5 })} />);
    expect(screen.getByTestId("quota-badge")).toHaveTextContent("53%");
  });
});
```

- [ ] **Step 3: 跑測試確認它失敗**

Run: `npm run test -- QuotaBadge 2>&1 | tail -15`
Expected: FAIL，`Failed to resolve import "./QuotaBadge"`

- [ ] **Step 4: 實作 `QuotaBadge`**

`src/components/QuotaBadge.tsx`：

```tsx
import type { QuotaWindow } from "../ipc/usage";
import "./QuotaBadge.css";

interface Props {
  window: QuotaWindow;
}

export function QuotaBadge({ window: w }: Props) {
  // detail 保留了上游的原始語意（Copilot 的次數），比百分比精確。
  const value = w.detail ?? `${Math.round(w.used_percent)}%`;
  return (
    <span className={`quota-badge quota-badge--${w.severity}`} data-testid="quota-badge">
      {w.label} {value}
    </span>
  );
}
```

`src/components/QuotaBadge.css`：

```css
.quota-badge {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 4px;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
/* normal 刻意低調：常駐顯示但不搶注意力，靠顏色而非有無來分級。 */
.quota-badge--normal { color: var(--text-secondary, #8a8a8a); }
.quota-badge--warning { color: #f59e0b; background: rgba(245, 158, 11, 0.12); }
.quota-badge--critical { color: #ef4444; background: rgba(239, 68, 68, 0.14); }
```

- [ ] **Step 5: 跑測試確認通過**

Run: `npm run test -- QuotaBadge 2>&1 | tail -8`
Expected: `4 passed`

- [ ] **Step 6: 寫 `ModelPickerButton` 的失敗測試**

`src/components/ModelPickerButton.quota.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelPickerButton } from "./ModelPickerButton";
import type { ProviderInfo } from "../ipc/provider";

const mockQuota = vi.fn();
const mockQuotaAll = vi.fn();
vi.mock("../ipc/usage", async () => {
  const actual = await vi.importActual<typeof import("../ipc/usage")>("../ipc/usage");
  return {
    ...actual,
    usageQuota: (id: string, force?: boolean) => mockQuota(id, force),
    usageQuotaAll: (force?: boolean) => mockQuotaAll(force),
  };
});

const providers: ProviderInfo[] = [
  { id: "anthropic-pro", display_name: "Anthropic", provider_type: "anthropic",
    base_url: null, oauth_client_id: null, model: "claude-sonnet-4-5",
    supports_json_mode: true, has_api_key: false, is_default: true, auth_method: "oauth" },
  { id: "ollama-local", display_name: "Ollama", provider_type: "ollama",
    base_url: null, oauth_client_id: null, model: "llama3",
    supports_json_mode: true, has_api_key: false, is_default: false, auth_method: null },
];

const okQuota = (pct: number, sev: "normal" | "warning" | "critical" = "normal") => ({
  status: "ok" as const,
  quota: {
    provider_id: "anthropic-pro", plan: "Claude Pro", fetched_at: 0,
    windows: [{ label: "5h", used_percent: pct, resets_at: null,
                severity: sev, detail: null, is_primary: true }],
  },
});

describe("ModelPickerButton 配額徽章", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockQuota.mockReset();
    mockQuotaAll.mockReset();
    mockQuota.mockResolvedValue(okQuota(7));
    mockQuotaAll.mockResolvedValue([okQuota(7)]);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("掛載時只查選中的那一個，不查全部", async () => {
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("anthropic-pro", false));
    expect(mockQuotaAll).not.toHaveBeenCalled();
  });

  it("severity 為 normal 時仍然顯示徽章", async () => {
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    expect(await screen.findByTestId("quota-badge")).toHaveTextContent("5h 7%");
  });

  it("下拉展開時才查全部", async () => {
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button"));
    await waitFor(() => expect(mockQuotaAll).toHaveBeenCalledWith(false));
  });

  it("切換 provider 會立即重查新選中者", async () => {
    const { rerender } = render(
      <ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />
    );
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("anthropic-pro", false));
    rerender(<ModelPickerButton providers={providers} selectedId="ollama-local" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("ollama-local", false));
  });

  it("多窗時徽章顯示最嚴重的那個，不是上游標記的代表窗", async () => {
    // 5h 剛重置 0%（上游標成代表窗），7d 已 96%。顯示綠色 0% 會誤導。
    mockQuota.mockResolvedValue({
      status: "ok",
      quota: {
        provider_id: "anthropic-pro", plan: "Claude Pro", fetched_at: 0,
        windows: [
          { label: "5h", used_percent: 0, resets_at: null,
            severity: "normal", detail: null, is_primary: true },
          { label: "7d", used_percent: 96, resets_at: null,
            severity: "critical", detail: null, is_primary: false },
        ],
      },
    });
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    const badge = await screen.findByTestId("quota-badge");
    expect(badge).toHaveTextContent("7d 96%");
    expect(badge.className).toContain("critical");
  });

  it("查詢失敗時按鈕仍可點開", async () => {
    mockQuota.mockResolvedValue({ status: "failed", provider_id: "anthropic-pro", message: "boom" });
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalled());
    expect(screen.queryByTestId("quota-badge")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button"));
    expect(screen.getByText("Ollama")).toBeInTheDocument();
  });

  it("沒有配額概念的 provider 不顯示徽章", async () => {
    mockQuota.mockResolvedValue({ status: "not_applicable", provider_id: "ollama-local" });
    render(<ModelPickerButton providers={providers} selectedId="ollama-local" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalled());
    expect(screen.queryByTestId("quota-badge")).not.toBeInTheDocument();
  });

  it("每 5 分鐘輪詢一次", async () => {
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
    await waitFor(() => expect(mockQuota).toHaveBeenCalledTimes(2));
  });
});
```

> `severity 為 normal 時仍然顯示徽章` 是這個 Task 最重要的測試 —— 它是 A 案與被否決的 B 案唯一的行為差異，最容易在日後改動中被悄悄退回。

- [ ] **Step 7: 跑測試確認它失敗**

Run: `npm run test -- ModelPickerButton.quota 2>&1 | tail -20`
Expected: FAIL，找不到 `quota-badge`

- [ ] **Step 8: 修改 `ModelPickerButton`**

在 `src/components/ModelPickerButton.tsx` 加入：

```tsx
import { QuotaBadge } from "./QuotaBadge";
import { usageQuota, usageQuotaAll, primaryWindow,
         type QuotaResult, type QuotaWindow } from "../ipc/usage";
```

在元件內部（`const selected = ...` 之後）加：

```tsx
  /** 選中 provider 的代表窗；null 代表沒有配額概念、查詢失敗或尚未載入。 */
  const [selectedWindow, setSelectedWindow] = useState<QuotaWindow | null>(null);
  /** 展開時查到的全部配額，key 是 provider id。 */
  const [allWindows, setAllWindows] = useState<Record<string, QuotaWindow>>({});

  // 常駐顯示：掛載與每 5 分鐘只查「選中的那一個」，不是全部。
  useEffect(() => {
    let cancelled = false;
    const load = () => {
      usageQuota(selectedId, false)
        .then((r: QuotaResult) => {
          if (cancelled) return;
          setSelectedWindow(r.status === "ok" ? primaryWindow(r.quota) : null);
        })
        .catch(() => { if (!cancelled) setSelectedWindow(null); });
    };
    load();
    const timer = setInterval(() => {
      // 背景放著不動的視窗不該一直打上游。
      if (!document.hidden) load();
    }, 5 * 60 * 1000);
    return () => { cancelled = true; clearInterval(timer); };
  }, [selectedId]);

  // 展開時才查全部，這是唯一會一次打多個上游端點的時機。
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    usageQuotaAll(false)
      .then((results: QuotaResult[]) => {
        if (cancelled) return;
        const map: Record<string, QuotaWindow> = {};
        for (const r of results) {
          if (r.status !== "ok") continue;
          const w = primaryWindow(r.quota);
          if (w) map[r.quota.provider_id] = w;
        }
        setAllWindows(map);
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [open]);
```

在按鈕的既有內容之後（收合狀態）加：

```tsx
        {selectedWindow && <QuotaBadge window={selectedWindow} />}
```

在下拉選單每個 provider 列的既有內容之後加：

```tsx
            {allWindows[p.id] && <QuotaBadge window={allWindows[p.id]} />}
```

- [ ] **Step 9: 跑測試確認通過**

Run: `npm run test -- ModelPickerButton 2>&1 | tail -10`
Expected: 新測試全過，且 **既有的 ModelPickerButton 測試也全過**

- [ ] **Step 10: i18n 與型別檢查**

`src/lib/i18n.ts` 加（en / zh-TW）：

```ts
  quotaResetsIn: "resets in {0}",   // zh-TW: "{0} 後重置"
  quotaUnavailable: "Quota unavailable",  // zh-TW: "無法取得配額"
  quotaPlan: "Plan",                // zh-TW: "方案"
```

Run: `npx tsc -b && npm run test 2>&1 | tail -8`
Expected: 皆通過

- [ ] **Step 11: Commit**

```bash
git add src/components/QuotaBadge.tsx src/components/QuotaBadge.css \
        src/components/QuotaBadge.test.tsx src/components/ModelPickerButton.tsx \
        src/components/ModelPickerButton.quota.test.tsx src/ipc/usage.ts src/lib/i18n.ts
git commit -m "feat(quota): ModelPickerButton 常駐配額徽章

收合狀態一律顯示選中 provider 的代表窗，靠顏色而非有無來分級 ——
測試釘死 severity=normal 時仍顯示，這是與舊設計唯一的行為差異。

抓取時機：掛載與每 5 分鐘只查選中的一個，展開才查全部，失焦停止輪詢。"
```

---

## Task 9: Settings 用量頁的配額區塊

**Files:**
- Modify: `src/components/Settings/UsagePage.tsx`（若本地累計計畫尚未執行，則建立）
- Modify: `src/components/Settings/UsagePage.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

加到 `UsagePage.test.tsx`：

```tsx
  it("每個訂閱型 provider 一張卡，顯示方案與各窗", async () => {
    mockQuotaAll.mockResolvedValue([
      { status: "ok", quota: { provider_id: "anthropic-pro", plan: "Claude Pro", fetched_at: 0,
        windows: [
          { label: "5h", used_percent: 7, resets_at: null, severity: "normal", detail: null, is_primary: true },
          { label: "7d", used_percent: 4, resets_at: null, severity: "normal", detail: null, is_primary: false },
        ] } },
    ]);
    render(<UsagePage />);
    expect(await screen.findByText("anthropic-pro")).toBeInTheDocument();
    expect(screen.getByText("Claude Pro")).toBeInTheDocument();
    expect(screen.getAllByTestId("quota-badge")).toHaveLength(2);
  });

  it("沒有配額概念的 provider 不列在配額區", async () => {
    mockQuotaAll.mockResolvedValue([{ status: "not_applicable", provider_id: "ollama-local" }]);
    render(<UsagePage />);
    await waitFor(() => expect(mockQuotaAll).toHaveBeenCalled());
    expect(screen.queryByText("ollama-local")).not.toBeInTheDocument();
  });

  it("查詢失敗的 provider 顯示錯誤訊息但不影響其他張卡", async () => {
    mockQuotaAll.mockResolvedValue([
      { status: "failed", provider_id: "GPT5.6", message: "AuthFailed" },
      { status: "ok", quota: { provider_id: "anthropic-pro", plan: null, fetched_at: 0,
        windows: [{ label: "5h", used_percent: 7, resets_at: null,
                    severity: "normal", detail: null, is_primary: true }] } },
    ]);
    render(<UsagePage />);
    expect(await screen.findByText("anthropic-pro")).toBeInTheDocument();
    expect(screen.getByTestId("quota-failed-GPT5.6")).toBeInTheDocument();
  });

  it("重新整理鈕會強制略過快取", async () => {
    mockQuotaAll.mockResolvedValue([]);
    render(<UsagePage />);
    await waitFor(() => expect(mockQuotaAll).toHaveBeenCalledWith(false));
    await userEvent.click(screen.getByTestId("quota-refresh"));
    await waitFor(() => expect(mockQuotaAll).toHaveBeenCalledWith(true));
  });
```

同時在檔案頂端的 mock 補上 `usageQuotaAll`：

```tsx
const mockQuotaAll = vi.fn();
// 併入既有的 vi.mock("../../ipc/usage", ...) 回傳物件：
//   usageQuotaAll: (force?: boolean) => mockQuotaAll(force),
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `npm run test -- UsagePage 2>&1 | tail -20`
Expected: FAIL，找不到 `quota-refresh`

- [ ] **Step 3: 實作配額區塊**

在 `UsagePage.tsx` 的區間切換之上加：

```tsx
  const [quotas, setQuotas] = useState<QuotaResult[]>([]);

  const loadQuotas = useCallback((force: boolean) => {
    usageQuotaAll(force).then(setQuotas).catch(() => setQuotas([]));
  }, []);

  useEffect(() => { loadQuotas(false); }, [loadQuotas]);
```

在 JSX 最前面加：

```tsx
      <section className="usage-quota-section">
        <div className="usage-quota-header">
          <h3>{t("usageQuotaTitle")}</h3>
          <button data-testid="quota-refresh" onClick={() => loadQuotas(true)}>
            {t("usageRefresh")}
          </button>
        </div>
        {quotas.map((r) => {
          if (r.status === "not_applicable") return null;
          if (r.status === "failed") {
            return (
              <div key={r.provider_id} className="usage-quota-card usage-quota-card--failed"
                   data-testid={`quota-failed-${r.provider_id}`}>
                <span className="usage-quota-provider">{r.provider_id}</span>
                <span className="usage-quota-error">{t("quotaUnavailable")}: {r.message}</span>
              </div>
            );
          }
          return (
            <div key={r.quota.provider_id} className="usage-quota-card">
              <div className="usage-quota-card-head">
                <span className="usage-quota-provider">{r.quota.provider_id}</span>
                {r.quota.plan && <span className="usage-quota-plan">{r.quota.plan}</span>}
              </div>
              <div className="usage-quota-windows">
                {r.quota.windows.map((w) => (
                  <div key={w.label} className="usage-quota-window">
                    <QuotaBadge window={w} />
                    <div className="usage-quota-bar">
                      <div className="usage-quota-bar-fill"
                           style={{ width: `${Math.min(100, w.used_percent)}%` }} />
                    </div>
                  </div>
                ))}
              </div>
            </div>
          );
        })}
      </section>
```

補上 import：`useCallback`、`usageQuotaAll`、`type QuotaResult`、`QuotaBadge`。

- [ ] **Step 4: 樣式**

加到 `UsagePage.css`：

```css
.usage-quota-section { margin-bottom: 24px; }
.usage-quota-header { display: flex; align-items: center; justify-content: space-between; }
.usage-quota-card {
  padding: 10px 12px; border: 1px solid var(--border, #2a2a2a);
  border-radius: 8px; margin-bottom: 8px;
}
.usage-quota-card-head { display: flex; gap: 8px; align-items: baseline; margin-bottom: 8px; }
.usage-quota-provider { font-weight: 600; }
.usage-quota-plan { font-size: 12px; opacity: 0.6; }
.usage-quota-window { display: flex; align-items: center; gap: 10px; margin-bottom: 4px; }
.usage-quota-bar {
  flex: 1; height: 6px; border-radius: 3px;
  background: var(--border, #2a2a2a); overflow: hidden;
}
.usage-quota-bar-fill { height: 100%; background: var(--accent, #3b82f6); }
.usage-quota-card--failed .usage-quota-error { font-size: 12px; opacity: 0.7; }
```

- [ ] **Step 5: i18n**

```ts
  usageQuotaTitle: "Subscription quota",  // zh-TW: "訂閱額度"
  usageRefresh: "Refresh",                // zh-TW: "重新整理"
```

- [ ] **Step 6: 跑測試確認通過**

Run: `npm run test -- UsagePage 2>&1 | tail -10`
Expected: 全過

- [ ] **Step 7: 完整驗證**

Run: `npx tsc -b && npm run test 2>&1 | tail -5 && cd src-tauri && cargo test 2>&1 | tail -5`
Expected: 三者皆通過

- [ ] **Step 8: Commit**

```bash
git add src/components/Settings/UsagePage.tsx src/components/Settings/UsagePage.css \
        src/components/Settings/UsagePage.test.tsx src/lib/i18n.ts
git commit -m "feat(quota): Settings 用量頁的配額區塊

每個訂閱型 provider 一張卡（方案 + 各窗進度條），沒有配額概念的略過，
查詢失敗的單獨顯示但不影響其他卡。重新整理鈕強制略過 60 秒快取。"
```

---

## 完成驗收

- [ ] `cd src-tauri && cargo test` 全過
- [ ] `npx tsc -b` 無輸出
- [ ] `npm run test` 全過
- [ ] `npm run lint` 無新增警告
- [ ] **`npm run tauri:dev` 用真實憑證實測**，逐項確認：
  - [ ] 選中 `anthropic-pro` 時按鈕上出現 `5h N%` 徽章，數字與 `/api/oauth/usage` 的 `five_hour.utilization` **一致**（不是它的 100 倍或 1/100）
  - [ ] 選中 `Github-Sonet4.5` 時徽章顯示 `premium 142 / 300` 這種剩餘次數
  - [ ] 選中 `Qwen3.6-27B`（本地）時**不顯示**徽章
  - [ ] 下拉展開後每一列都有各自的徽章
  - [ ] Settings → 用量頁的配額卡與徽章數字一致

> 最後一項不能只看「測試綠」。測試用的是 fixture 與 wiremock；真正的上游可能已經改了欄位。刻度錯誤（7% 顯示成 700%）在單元測試裡看起來完全正常，只有真跑一次才會發現。

---

## 後續維護提醒

三個端點都是**無文件的逆向端點**，上游隨時可能改。若日後徽章開始顯示「—」：

1. 重跑探勘：`cd src-tauri && cargo test --test usage_probe -- --ignored --nocapture`
2. 比對 dump 與 adapter 裡的 `#[derive(Deserialize)]` 結構
3. 改的只會是三個 adapter 之一，`QuotaWindow` 這層形狀不需要動

這正是把正規化層與 adapter 分開的目的。
