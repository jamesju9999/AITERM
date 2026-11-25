//! Server 的資料庫：問題收集本身用 core 的 `intake.db`；成員、邀請、稽核、專案放 `server.db`。
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;

#[derive(Clone)]
pub struct Pools {
    pub intake: SqlitePool,
    pub server: SqlitePool,
}

async fn open_file(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::new().filename(path).create_if_missing(true);
    SqlitePoolOptions::new().max_connections(5).connect_with(opts).await
}

pub async fn open(dir: &Path) -> Result<Pools, sqlx::Error> {
    std::fs::create_dir_all(dir).map_err(sqlx::Error::Io)?;
    let intake = open_file(&dir.join("intake.db")).await?;
    aiterm_intake_core::store::init_schema(&intake).await?;
    aiterm_intake_core::board_store::init_schema(&intake).await?;
    let server = open_file(&dir.join("server.db")).await?;
    init_server_schema(&server).await?;
    Ok(Pools { intake, server })
}

/// 測試用：記憶體資料庫（單一連線，兩個庫各一個）。
pub async fn open_memory() -> Result<Pools, sqlx::Error> {
    let mk = || async { SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await };
    let intake = mk().await?;
    aiterm_intake_core::store::init_schema(&intake).await?;
    aiterm_intake_core::board_store::init_schema(&intake).await?;
    let server = mk().await?;
    init_server_schema(&server).await?;
    Ok(Pools { intake, server })
}

pub async fn init_server_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS members (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, role TEXT NOT NULL,
            token_hash TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL, revoked_at INTEGER)",
        "CREATE TABLE IF NOT EXISTS invites (
            id TEXT PRIMARY KEY, key_hex TEXT NOT NULL, name TEXT NOT NULL, role TEXT NOT NULL,
            expires_at INTEGER NOT NULL, used_at INTEGER, attempts INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE IF NOT EXISTS audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, member TEXT NOT NULL,
            action TEXT NOT NULL, target TEXT NOT NULL DEFAULT '')",
        "CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, work_dir TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL)",
    ] {
        sqlx::query(sql).execute(pool).await?;
    }
    Ok(())
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
