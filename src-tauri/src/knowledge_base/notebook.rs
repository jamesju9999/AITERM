//! 建立筆記本前先驗證 embedding 模型真的能用。
//!
//! 分成獨立函式而非寫在 tauri 指令裡，是為了讓它接受 `&dyn Embedder`：
//! 指令層需要 tauri 的 State 才能組出 embedder，那在測試裡很難建構，
//! 而「探測失敗就不寫入」是這個功能最需要被測到的不變式。

use sqlx::SqlitePool;

use super::embedding::Embedder;
use crate::db::knowledge_base::{create_notebook, NotebookRow};

/// 探測用的文字。固定 ASCII 短字串，成本一個 token。
const PROBE_TEXT: &str = "test";

/// 用一次真實的 embed 呼叫驗證模型，成功才寫入筆記本。
///
/// 回傳的向量長度會被記成 `embed_dim`。這是這個欄位唯一的寫入點。
pub async fn create_notebook_verified(
    pool: &SqlitePool,
    name: &str,
    folder_path: &str,
    embed_provider_id: &str,
    embed_model: &str,
    embedder: &dyn Embedder,
) -> Result<NotebookRow, String> {
    let vectors = embedder
        .embed(&[PROBE_TEXT.to_string()])
        .await
        .map_err(|e| format!("此模型無法用於 embedding: {e}"))?;

    let dim = vectors.first().map(|v| v.len()).unwrap_or(0);
    if dim == 0 {
        return Err("此模型未回傳任何向量，無法用於 embedding".into());
    }

    create_notebook(
        pool, name, folder_path,
        Some(embed_provider_id), Some(embed_model), dim as i64,
    )
    .await
    .map_err(|e| e.to_string())
}
