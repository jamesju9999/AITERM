//! The MCP protocol surface: an `rmcp` tool router exposing AITerm's DB and
//! knowledge-base capabilities. Each `#[tool]` method here is a thin
//! wrapper — the actual logic lives in `db_ops`/`kb_ops` (kept separate so
//! that logic is unit-testable without going through MCP's JSON-RPC layer).
//!
//! Code shape (macros, `Parameters<T>` extractor, `ServerHandler` via
//! `#[tool_handler]`) follows the official `rmcp` example at
//! `https://github.com/modelcontextprotocol/rust-sdk/blob/main/examples/servers/src/common/counter.rs`.

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};
use sqlx::SqlitePool;

use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::secret::SecretStore;

use super::{db_ops, kb_ops};

fn to_call_result(r: Result<String, String>) -> Result<CallToolResult, McpError> {
    match r {
        Ok(s) => Ok(CallToolResult::success(vec![ContentBlock::text(s)])),
        Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e)])),
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectionIdArgs {
    /// Connection id as returned by `list_connections`.
    pub connection_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListTablesArgs {
    pub connection_id: String,
    pub schema: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TableSchemaArgs {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExecuteQueryArgs {
    pub connection_id: String,
    /// Arbitrary SQL. Not restricted to read-only.
    pub sql: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchDocumentsArgs {
    /// Notebook id as returned by `list_notebooks`.
    pub notebook_id: String,
    /// Natural-language description of what you're looking for — not just keywords.
    pub query: String,
    /// Number of results to return (default 8, max 20).
    #[serde(default)]
    pub top_k: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadDocumentArgs {
    pub notebook_id: String,
    /// Document path exactly as returned by `search_documents`.
    pub path: String,
}

#[derive(Clone)]
pub struct AiTermTools {
    db_manager: Arc<DbManager>,
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    sidecar: Arc<Db2SidecarState>,
    kb_pool: SqlitePool,
    tool_router: ToolRouter<AiTermTools>,
}

#[tool_router]
impl AiTermTools {
    pub fn new(
        db_manager: Arc<DbManager>,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        sidecar: Arc<Db2SidecarState>,
        kb_pool: SqlitePool,
    ) -> Self {
        Self {
            db_manager,
            config,
            secrets,
            sidecar,
            kb_pool,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List AITerm's configured database connections (id, name, type, database — no credentials).")]
    async fn list_connections(&self) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::list_connections(&self.config).await)
    }

    #[tool(description = "List schema names visible on a database connection. Connects automatically if not already connected.")]
    async fn list_schemas(
        &self,
        Parameters(ConnectionIdArgs { connection_id }): Parameters<ConnectionIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::list_schemas(&connection_id, &self.config, &self.secrets, &self.db_manager, &self.sidecar).await)
    }

    #[tool(description = "List tables and views in a schema on a database connection. Connects automatically if not already connected.")]
    async fn list_tables(
        &self,
        Parameters(ListTablesArgs { connection_id, schema }): Parameters<ListTablesArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::list_tables(&connection_id, &schema, &self.config, &self.secrets, &self.db_manager, &self.sidecar).await)
    }

    #[tool(description = "Get column metadata (name, type, nullable, default) for a table. Connects automatically if not already connected.")]
    async fn get_table_schema(
        &self,
        Parameters(TableSchemaArgs { connection_id, schema, table }): Parameters<TableSchemaArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::get_table_schema(&connection_id, &schema, &table, &self.config, &self.secrets, &self.db_manager, &self.sidecar).await)
    }

    #[tool(description = "Execute arbitrary SQL on a database connection and return the result as JSON. NOT restricted to read-only — INSERT/UPDATE/DELETE/DDL are all allowed. Connects automatically if not already connected.")]
    async fn execute_query(
        &self,
        Parameters(ExecuteQueryArgs { connection_id, sql }): Parameters<ExecuteQueryArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::execute_query(&connection_id, &sql, &self.config, &self.secrets, &self.db_manager, &self.sidecar).await)
    }

    #[tool(description = "List notebooks in AITerm's knowledge base.")]
    async fn list_notebooks(&self) -> Result<CallToolResult, McpError> {
        to_call_result(kb_ops::list_notebooks(&self.kb_pool).await)
    }

    #[tool(description = "Semantic search over a knowledge base notebook's indexed documents. Returns the most relevant text chunks, each tagged with its source file path, location hint, and similarity score. Call list_notebooks first if you don't know the notebook_id.")]
    async fn search_documents(
        &self,
        Parameters(SearchDocumentsArgs { notebook_id, query, top_k }): Parameters<SearchDocumentsArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(kb_ops::search_documents(&self.kb_pool, &self.config, &self.secrets, &notebook_id, &query, top_k).await)
    }

    #[tool(description = "Read a knowledge base document's full converted content by its exact path (as shown in search_documents results).")]
    async fn read_document(
        &self,
        Parameters(ReadDocumentArgs { notebook_id, path }): Parameters<ReadDocumentArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(kb_ops::read_document(&self.kb_pool, &notebook_id, &path).await)
    }
}

#[tool_handler]
impl ServerHandler for AiTermTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
        .with_instructions(
            "Query AITerm's configured database connections and knowledge base notebooks. \
             Call list_connections or list_notebooks first to discover ids.".to_string(),
        )
    }
}
