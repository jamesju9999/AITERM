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
use tauri::AppHandle;

use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::pty::manager::PtyManager;
use crate::secret::SecretStore;

use super::coordination_ops::CoordinationRegistry;
use super::{coordination_ops, db_ops, kb_ops};

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
    /// Connection id as returned by `list_connections`.
    pub connection_id: String,
    /// Schema name as returned by `list_schemas`.
    pub schema: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TableSchemaArgs {
    /// Connection id as returned by `list_connections`.
    pub connection_id: String,
    /// Schema name as returned by `list_schemas`.
    pub schema: String,
    /// Table name as returned by `list_tables`.
    pub table: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExecuteQueryArgs {
    /// Connection id as returned by `list_connections`.
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
    /// Notebook id as returned by `list_notebooks`.
    pub notebook_id: String,
    /// Document path exactly as returned by `search_documents`.
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SpawnTabArgs {
    /// Working directory for the new tab. Defaults to the user's home directory if
    /// omitted. Recommended: pass your OWN current working directory (e.g. the
    /// output of `pwd`) rather than omitting this — spawning in a directory the
    /// initial command hasn't seen before can trigger that program's own
    /// first-run trust/permission prompt (e.g. Claude Code CLI's "Do you trust
    /// this folder?"), which blocks the new tab until a human answers it. Reusing
    /// your own directory, which the same tool has very likely already been
    /// trusted in, avoids that.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Initial command to run in the new tab once it's ready (e.g. "claude" or "codex").
    #[serde(default)]
    pub command: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendInputArgs {
    /// Tab id as returned by `spawn_tab`. Must be a tab this server spawned — never one the user opened by hand.
    pub tab_id: String,
    /// Text to send, as if typed into the tab followed by Enter.
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TabIdArgs {
    /// Tab id as returned by `spawn_tab`.
    pub tab_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WaitForIdleArgs {
    /// Tab id as returned by `spawn_tab`.
    pub tab_id: String,
    /// Max seconds to wait (default 300, capped at 1800).
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Clone)]
pub struct AiTermTools {
    db_manager: Arc<DbManager>,
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    sidecar: Arc<Db2SidecarState>,
    kb_pool: SqlitePool,
    pty_manager: Arc<PtyManager>,
    app: Option<AppHandle>,
    coordination_registry: Arc<CoordinationRegistry>,
    tool_router: ToolRouter<AiTermTools>,
}

#[tool_router]
impl AiTermTools {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db_manager: Arc<DbManager>,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        sidecar: Arc<Db2SidecarState>,
        kb_pool: SqlitePool,
        pty_manager: Arc<PtyManager>,
        app: Option<AppHandle>,
        coordination_registry: Arc<CoordinationRegistry>,
    ) -> Self {
        Self {
            db_manager,
            config,
            secrets,
            sidecar,
            kb_pool,
            pty_manager,
            app,
            coordination_registry,
            tool_router: Self::tool_router(),
        }
    }

    /// Every coordination tool calls this first. `Ok(())` means proceed;
    /// `Err` is a ready-to-return tool-level error explaining the toggle is
    /// off. Kept as one shared check (not duplicated per tool) so the wording
    /// can't drift between the 4 call sites.
    fn require_coordination_enabled(&self) -> Result<(), String> {
        if self.config.get().mcp_tool_server.coordination_enabled {
            Ok(())
        } else {
            Err("Agent coordination tools are disabled. Enable them in AITerm's Settings → MCP Tool Server page.".to_string())
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

    #[tool(description = "Spawn a new AITerm terminal tab, visible to the user, optionally running an initial command (e.g. 'claude' or 'codex') to start another coding agent in it. Pass cwd as your own current directory to avoid the spawned command's first-run trust prompt in an unfamiliar folder. Returns the new tab's id. Disabled by default — must be enabled in Settings.")]
    async fn spawn_tab(
        &self,
        Parameters(SpawnTabArgs { cwd, command }): Parameters<SpawnTabArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.require_coordination_enabled() {
            return to_call_result(Err(e));
        }
        let Some(app) = &self.app else {
            return to_call_result(Err(
                "spawn_tab requires a running AITerm app instance (not available in this context)".to_string(),
            ));
        };
        to_call_result(coordination_ops::spawn_tab(app, &self.pty_manager, &self.coordination_registry, cwd, command).await)
    }

    #[tool(description = "Send text (as if typed, followed by Enter) to a tab previously created by spawn_tab. Cannot target a tab the user opened by hand. Disabled by default — must be enabled in Settings.")]
    async fn send_input(
        &self,
        Parameters(SendInputArgs { tab_id, text }): Parameters<SendInputArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.require_coordination_enabled() {
            return to_call_result(Err(e));
        }
        to_call_result(coordination_ops::send_input(&self.pty_manager, &self.coordination_registry, &tab_id, &text))
    }

    #[tool(description = "Check whether a spawn_tab-created tab is idle (a terminal bell was observed since the last send_input, meaning the agent inside replied and is waiting for more input) and get its recent output. Disabled by default — must be enabled in Settings.")]
    async fn get_tab_status(
        &self,
        Parameters(TabIdArgs { tab_id }): Parameters<TabIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.require_coordination_enabled() {
            return to_call_result(Err(e));
        }
        to_call_result(coordination_ops::get_tab_status(&self.pty_manager, &self.coordination_registry, &tab_id))
    }

    #[tool(description = "Block until a spawn_tab-created tab becomes idle (see get_tab_status) or the timeout elapses, then return its status. Disabled by default — must be enabled in Settings.")]
    async fn wait_for_idle(
        &self,
        Parameters(WaitForIdleArgs { tab_id, timeout_seconds }): Parameters<WaitForIdleArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.require_coordination_enabled() {
            return to_call_result(Err(e));
        }
        to_call_result(coordination_ops::wait_for_idle(&self.pty_manager, &self.coordination_registry, &tab_id, timeout_seconds).await)
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
