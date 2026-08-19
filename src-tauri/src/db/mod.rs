pub mod adapter;
pub mod manager;
pub mod design;
pub mod loop_sessions;
pub mod postgres;
pub mod mysql;
pub mod sqlite;
pub mod mssql;
pub mod db2;
pub mod db2_sidecar;
pub mod knowledge_base;
pub mod kb_chat_sessions;
pub mod mail;

pub use adapter::{DbAdapter, TableInfo, ColumnInfo, QueryResult};
pub use manager::DbManager;
pub use db2_sidecar::{resolve_db2_sidecar_path, Db2SidecarState};
