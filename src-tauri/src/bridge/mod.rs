//! Claude Code 橋接：一個綁 127.0.0.1 的 Anthropic Messages API 相容 server，
//! 把 Claude Code CLI 的請求翻譯到 AITerm 已設定的任一 AI 供應商。
//!
//! 為什麼不擴充 `ai::AiProvider`：Claude Code 需要串流 tool_use、thinking
//! 區塊與 cache_control，這些保真度 AITerm 自己的 UI 完全用不到。把它們塞
//! 進共用 trait 會讓 7 支 client、Agent loop 與 chat hook 全進入爆炸半徑，
//! 受益者卻只有一個消費者。因此另開模組，但憑證解析與端點常數共用
//! （見 `upstream/` 各 adapter）。
