//! ChatGPT 網頁版供應商。
//!
//! 傳輸層是一個載入 chatgpt.com 的隱藏 WebviewWindow：所有上游請求都由注入
//! 該頁面的 JS 發出，因此天然帶有真實瀏覽器的 TLS 指紋與 cookie。設計與實測
//! 依據見 docs/superpowers/specs/2026-08-10-chatgpt-web-provider-design.md。

pub mod protocol;
pub mod tools;
