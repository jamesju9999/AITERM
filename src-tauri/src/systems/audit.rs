//! 系統連線的審計：核心在共用的 aiterm-systems；桌面版的 `systems.db` 另外有配方與定時任務的表。

use std::path::PathBuf;

use sqlx::SqlitePool;

pub use aiterm_systems::audit::*;

/// 應用層的 `systems.db`（跟 knowledge_base.db 同一個資料夾）：審計＋配方＋定時任務。
pub async fn open() -> Result<SqlitePool, sqlx::Error> {
    let dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM");
    let pool = aiterm_systems::audit::open_at(&dir.join("systems.db")).await?;
    super::recipes::init(&pool).await?;
    super::jobs::init(&pool).await?;
    Ok(pool)
}

/// 測試用：全新的記憶體資料庫，建好 systems.db 的所有表。
#[cfg(test)]
pub async fn open_in_memory() -> SqlitePool {
    let pool = aiterm_systems::audit::open_in_memory().await;
    super::recipes::init(&pool).await.unwrap();
    super::jobs::init(&pool).await.unwrap();
    pool
}
