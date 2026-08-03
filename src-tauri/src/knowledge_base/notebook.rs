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
/// 回傳的向量長度會被記成 `embed_dim`。這是目前唯一實際生效的寫入點
/// （`db::knowledge_base::update_embed_settings` 也會寫這個欄位，但還沒有呼叫者，
/// 它是留給日後「換模型」功能的地基）。
///
/// # 呼叫端必須自己保證的前提
///
/// `embedder` 必須是用這裡傳入的同一組 `embed_provider_id` / `embed_model` 建出來的。
/// `Embedder` trait 不帶身分資訊，本函式無從檢查；若兩者兜不起來，寫進去的就會是
/// 一次「驗證了別的模型」的假紀錄。（不為此在 trait 上加 `fn model()`：只有一個
/// 呼叫端，不值得。）
pub async fn create_notebook_verified(
    pool: &SqlitePool,
    name: &str,
    folder_path: &str,
    embed_provider_id: &str,
    embed_model: &str,
    embedder: &dyn Embedder,
) -> Result<NotebookRow, String> {
    // 先擋掉空的 provider/model 再探測：這種筆記本就算探測成功也永遠同步不了
    // （`resolve_embedder_config` 會找不到 provider），沒必要為它多花一趟 round trip。
    if embed_provider_id.trim().is_empty() {
        return Err("缺少 embedding provider".into());
    }
    if embed_model.trim().is_empty() {
        return Err("缺少 embedding model".into());
    }

    let vectors = embedder
        .embed(&[PROBE_TEXT.to_string()])
        .await
        .map_err(|e| format!("此模型無法用於 embedding: {e}"))?;

    let dim = vectors.first().map(|v| v.len()).unwrap_or(0);
    if dim == 0 {
        return Err("此模型沒有回傳可用的向量，無法用於 embedding".into());
    }

    create_notebook(
        pool, name, folder_path,
        Some(embed_provider_id), Some(embed_model), dim as i64,
    )
    .await
    .map_err(|e| e.to_string())
}
