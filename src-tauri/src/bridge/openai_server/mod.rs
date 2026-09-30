//! 對外的 OpenAI 相容 server：把 `/v1/chat/completions` 請求翻譯到 AITerm
//! 已設定的任一 AI 供應商。與 Claude Code 橋接（`bridge::server`）平行、
//! 互不影響，但共用同一套上游 adapter 與中立事件（`upstream::UpstreamEvent`）。
//!
//! 設計見 `docs/superpowers/specs/2026-09-30-openai-compatible-server-design.md`。

pub mod alias_map;
pub mod anthropic_events;
pub mod request;
pub mod response;
pub mod server;
