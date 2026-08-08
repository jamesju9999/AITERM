//! 上游 adapter。
//!
//! 三條路徑收斂到 [`UpstreamEvent`]，Anthropic SSE 序列化器因此只需要寫一份；
//! 新增一個上游 = 寫一支 adapter，序列化端零改動。
//!
//! Anthropic 家族是特例：它本來就講同一個協定，解析再重組是純粹的損耗，
//! 所以走 [`UpstreamResponse::Passthrough`] 直接 pipe。

use async_trait::async_trait;
use futures_util::stream::BoxStream;

use crate::ai::AiError;
use crate::bridge::anthropic::request::MessagesRequest;

pub mod anthropic;
pub mod antigravity;
pub mod codex;
pub mod openai;

#[derive(Debug, Clone, PartialEq)]
pub enum UpstreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    /// 工具名稱確定後才發 —— Anthropic 的 `content_block_start` 就要帶 name，
    /// 而 OpenAI 是 name 先到、arguments 分片後到。
    ToolUseStart { id: String, name: String },
    /// 工具參數的 partial JSON 片段，不需要是完整的 JSON。
    ToolInputDelta(String),
    ToolUseEnd,
    Done { stop_reason: StopReason, usage: Usage },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            StopReason::EndTurn => "end_turn",
            StopReason::MaxTokens => "max_tokens",
            StopReason::ToolUse => "tool_use",
            StopReason::StopSequence => "stop_sequence",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub enum UpstreamResponse {
    /// SSE 原樣轉發，不解析。
    Passthrough(reqwest::Response),
    Events(BoxStream<'static, Result<UpstreamEvent, AiError>>),
}

// 手動實作而非 `#[derive(Debug)]`：`Events` 裡的 `BoxStream` 是 trait object，
// 不會自動滿足 `Debug`，但整合測試用到的 `Result::unwrap_err` 要求
// `T: Debug`。
impl std::fmt::Debug for UpstreamResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpstreamResponse::Passthrough(_) => write!(f, "UpstreamResponse::Passthrough(..)"),
            UpstreamResponse::Events(_) => write!(f, "UpstreamResponse::Events(..)"),
        }
    }
}

#[async_trait]
pub trait BridgeUpstream: Send + Sync {
    /// `model` 是映射後的上游模型名，不是 Claude Code 送來的哨兵字串。
    async fn send(
        &self,
        req: &MessagesRequest,
        model: &str,
    ) -> Result<UpstreamResponse, AiError>;
}
