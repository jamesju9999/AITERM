//! 用量記帳與配額查詢。
//!
//! `store` 負責把每次 AI 請求的 token 落地到 `usage.db`；`metered` 是包在
//! `AiProvider` 外的裝飾器，是唯一的記帳接點；`pricing` 是純函式的成本估算。

pub mod quota;
pub mod store;

pub use store::{UsageRange, UsageStore, UsageSummaryRow};
