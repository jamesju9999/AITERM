# Database Tab Feature — Design Spec
Date: 2026-04-17

## Overview

Add a database connection tab type to AITerm. When the user clicks `+` in the TabBar, a dropdown lets them choose between Terminal and Database. Database tabs provide a SQL Management-style interface with AI-assisted querying.

## Decisions Summary

| Question | Decision |
|----------|----------|
| New tab picker UI | Dropdown near `+` button (Option A) |
| DB Tab layout | Sub-tabs: 瀏覽 / AI Chat / SQL Editor (Option B) |
| Connection management | Both settings page + in-tab selector (Option C) |
| Supported databases | PostgreSQL, MySQL, SQLite, MSSQL, DB2 |
| AI permission level | Full — any SQL, user controls via connection account |
| DB2 driver | ODBC (`odbc-api` crate) + in-app installation instructions |

---

## Section 1: Components & Data Flow

### Frontend

#### `Tab` type extension (`src/components/TabBar/index.tsx`)

```ts
interface Tab {
  id: string;
  title: string;
  type: "terminal" | "database";  // new
  dbConnectionId?: string;         // set after connection is selected
}
```

#### New components

**`NewTabPicker`** (`src/components/NewTabPicker/index.tsx`)
- Floating dropdown rendered below the `+` button in TabBar
- Two items: 終端機 / 資料庫
- Dismissed on outside click or Escape

**`DatabaseView`** (`src/components/DatabaseView/index.tsx`)
- Equivalent of `TerminalView` for database tabs
- **No connection state**: shows connection selector — list of saved connections + "新增連線" shortcut that opens the settings DB page
- **Connected state**: renders three sub-tabs
  - **瀏覽**: left object tree (Schema → Tables → Views) + right data grid + pagination
  - **AI Chat**: conversation UI, AI generates SQL → preview block → execute → result table inline
  - **SQL Editor**: textarea + Run button + result grid below

**`DatabaseConnectionsPage`** (`src/components/Settings/DatabaseConnectionsPage.tsx`)
- New entry in the settings sidebar between "AI Providers" and "一般"
- List of saved connections with edit/delete/test actions
- Add/edit form fields: Name, Type (dropdown), Host, Port, Database, Username, Password, (DB2: DSN)

#### `TerminalApp.tsx` changes
- `handleAddTab` replaced by `handleAddTabWithPicker` — shows `NewTabPicker`
- Tab rendering: `tab.type === "database"` renders `DatabaseView`, otherwise `TerminalView`

---

### Backend (Rust)

#### New module `src-tauri/src/db/`

```
db/
  mod.rs
  adapter.rs     — DbAdapter trait (unified interface)
  postgres.rs    — sqlx PgPool
  mysql.rs       — sqlx MySqlPool
  sqlite.rs      — sqlx SqlitePool
  mssql.rs       — tiberius crate
  db2.rs         — odbc-api crate (requires IBM DB2 ODBC Driver)
  manager.rs     — DbManager: HashMap<Uuid, Box<dyn DbAdapter>>
```

#### `DbAdapter` trait (`adapter.rs`)

```rust
#[async_trait]
pub trait DbAdapter: Send + Sync {
    async fn test(&self) -> Result<()>;
    async fn list_schemas(&self) -> Result<Vec<String>>;
    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>>;
    async fn get_table_schema(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>>;
    async fn execute(&self, sql: &str) -> Result<QueryResult>;
}
```

#### New Tauri commands (`src-tauri/src/commands/db.rs`)

| Command | Description |
|---------|-------------|
| `db_list_connections` | Returns `Vec<DbConnectionInfo>` from config |
| `db_add_connection(config)` | Saves config + password to Keychain, returns UUID |
| `db_update_connection(id, config)` | Updates config + password |
| `db_remove_connection(id)` | Removes config + Keychain entry + disconnects if active |
| `db_test_connection(config)` | Opens a temporary connection, returns Ok or error string |
| `db_connect(id)` | Creates adapter, stores in `DbManager` |
| `db_disconnect(id)` | Removes from `DbManager` |
| `db_list_tables(connection_id, schema)` | Returns `Vec<TableInfo>` |
| `db_get_table_schema(connection_id, schema, table)` | Returns `Vec<ColumnInfo>` |
| `db_execute_query(connection_id, sql)` | Returns `QueryResult` |

#### Password storage
Reuses existing `SecretStore`. Key format: `db:{connection_id}`

---

## Section 2: AI Integration & Data Models

### AI Chat flow

1. On entering the AI Chat sub-tab, frontend calls `db_list_tables` to build a schema summary
2. Each message is sent via the existing `ai_chat` command with DB context injected into the system prompt:
   ```
   你是一個資料庫助手，正在連接 {db_type} 資料庫「{database}」。
   可用的 Tables：{table_list_with_columns}
   請生成有效的 SQL 語句回答使用者的問題。
   回覆格式：先用一句話說明，然後用 ```sql 包住 SQL。
   ```
3. Frontend parses SQL code blocks from the AI response and renders them as executable preview blocks
4. User clicks 執行 → `db_execute_query` → results rendered as table in conversation

**Auto-retry on SQL error:** If the query returns an error, it is fed back to the AI (max 2 retries) before surfacing the error to the user.

### Data models

#### `DbConnection` (stored in `config.json`, password in Keychain)

```ts
interface DbConnection {
  id: string;           // UUID
  name: string;         // display name, e.g. "my_postgres"
  db_type: "postgresql" | "mysql" | "sqlite" | "mssql" | "db2";
  host: string;         // SQLite: file path
  port: number;
  database: string;
  username: string;
  // password → SecretStore key: `db:{id}`
}
```

#### `QueryResult`

```ts
interface QueryResult {
  columns: string[];
  rows: unknown[][];
  affected_rows?: number;    // for INSERT / UPDATE / DELETE
  execution_time_ms: number;
  error?: string;
}
```

### DB2 ODBC notice

DB2 connection form shows a persistent notice:
> ⚠️ DB2 需要 IBM DB2 ODBC Driver。
> Windows / macOS: IBM Data Server Driver Package
> [查看安裝說明 →]

Error code `odbc_driver_not_found` triggers a dedicated installation-guide modal instead of a generic error.

---

## Section 3: Error Handling & Testing

### Error handling

**Connection errors:**
- Connection failure → `DatabaseView` shows error message + 重新連線 button
- `db_test_connection` validates before saving in settings; shows success/failure badge inline

**Query errors:**
- SQL execution errors displayed in the result area (red), non-blocking
- AI Chat errors fed back to AI for auto-correction (max 2 retries)

**DB2 ODBC driver missing:**
- `db_connect` returns error code `odbc_driver_not_found`
- Frontend shows installation-guide modal (not generic error message)

### Testing strategy

**Frontend (Vitest):**
- `NewTabPicker`: show/hide, correct tab type created on selection
- `DatabaseView`: connection selector shown when no connection; sub-tabs switch correctly after connecting
- `DatabaseConnectionsPage`: form validation, add/delete connections

**Backend (Rust unit tests):**
- `DbAdapter` shared logic
- `DbManager`: add, remove, duplicate-connection guard
- Per-adapter integration tests marked `#[ignore]` (require live DB), consistent with existing `SecretStore` test pattern

---

## Cross-platform compatibility

| Database | Windows | macOS | Notes |
|----------|---------|-------|-------|
| PostgreSQL | ✅ | ✅ | `sqlx`, pure Rust TCP |
| MySQL | ✅ | ✅ | `sqlx`, pure Rust |
| SQLite | ✅ | ✅ | `sqlx`, bundled |
| MSSQL | ✅ | ✅ | `tiberius`, pure Rust TDS |
| DB2 | ⚠️ | ⚠️ | Requires IBM DB2 ODBC Driver installed |
| Password storage | ✅ Windows Credential Manager | ✅ Keychain | Existing `keyring` crate |
