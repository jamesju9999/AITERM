//! AITerm 連線 aiterm-intake-server 的 Client：配對、釘住憑證指紋、之後帶 token 呼叫 API。
pub mod pair;
pub mod pin;

use thiserror::Error;

/// Client 端各種失敗的原因。畫面要能對應不同的說明，所以不是一個通用字串。
#[derive(Error, Debug, Clone, PartialEq)]
pub enum ClientError {
    #[error("位址格式不正確")]
    BadAddress,
    #[error("邀請碼格式不正確")]
    BadCode,
    #[error("連不上 server：{0}")]
    Offline(String),
    #[error("配對失敗：邀請碼錯誤、已過期或已被使用")]
    PairingRefused,
    #[error("嘗試次數過多，請稍後再試")]
    RateLimited,
    #[error("server 的身分驗證失敗（對方沒有出示正確的證明），可能被中間人攔截")]
    ServerNotProven,
    #[error("server 憑證與上次不同，可能被中間人攔截；若 server 確實換了憑證，請重新配對")]
    FingerprintChanged,
    #[error("你已被移除，請向管理員重新取得邀請")]
    Revoked,
    #[error("沒有權限做這個操作")]
    Forbidden,
    #[error("版本不相容：server 協定 {server}，此版本支援 {client}。請更新 AITerm 或 server")]
    Incompatible { server: u32, client: u32 },
    #[error("server 回傳錯誤：{0}")]
    Server(String),
}
