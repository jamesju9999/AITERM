//! 收訊管道（LINE webhook、網頁表單）的啟停。設定或祕密變動後呼叫 `reload`。
use crate::state::AppState;

pub async fn reload(_st: &AppState) {}

pub fn line_listening(_st: &AppState) -> bool {
    false
}
