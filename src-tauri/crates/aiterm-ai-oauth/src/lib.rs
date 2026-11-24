//! 訂閱登入（OAuth）共用程式：token 交換、續期與一次性呼叫。
//!
//! 這裡的常數與線上格式**以 AITerm 的實作為準**（`commands/provider.rs`、`ai/anthropic.rs`，
//! 已用真實帳號驗證過）；AITerm 端有測試斷言兩邊一致，日後改了一邊會立刻紅燈。
//! 不依賴 tauri、pty，Server 與 AITerm 的「為 server 登入」共用。
pub mod anthropic;
