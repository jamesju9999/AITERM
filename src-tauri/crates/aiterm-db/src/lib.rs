//! 資料庫適配器：統一的 `DbAdapter` 介面與 PostgreSQL／MySQL／SQLite 的實作。
//! 桌面版（`app`）與 server 共用；SQL Server、ODBC、DB2 的適配器只在桌面版。
pub mod adapter;
pub mod connect;
pub mod mysql;
pub mod postgres;
pub mod skill;
pub mod sql_guard;
pub mod sqlite;

pub use adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
