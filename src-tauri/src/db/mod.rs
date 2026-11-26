// 這四個適配器搬到共用的 aiterm-db（server 也用）；這裡 re-export，原本的路徑不變。
pub use aiterm_db::{adapter, mysql, postgres, sqlite};
pub mod manager;
pub mod design;
pub mod loop_sessions;
pub mod mssql;
pub mod db2;
pub mod db2_sidecar;
pub mod knowledge_base;
pub mod kb_chat_sessions;
pub mod mail;

pub use adapter::{DbAdapter, TableInfo, ColumnInfo, QueryResult};
pub use manager::DbManager;
pub use db2_sidecar::{resolve_db2_sidecar_path, Db2SidecarState};
