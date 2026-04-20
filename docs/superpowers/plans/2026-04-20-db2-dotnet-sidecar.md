# DB2 .NET Sidecar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On Windows, replace IBM ODBC Driver requirement with a .NET sidecar (IBM.Data.Db2.Core). On macOS/Linux, keep the existing ODBC adapter unchanged. Platform selection is at compile time.

**Architecture (amended — dual-path):** Windows uses a .NET sidecar process for DB2; macOS/Linux keeps the existing ODBC Db2Adapter. `Db2Adapter` is the same public name; platform selected via `#[cfg(target_os = "windows")]`. The sidecar is built for `win-x64` only (IBM.Data.Db2.Core bundles Windows-only clidriver DLLs).

**Tech Stack:** .NET 8, IBM.Data.Db2.Core 3.x (NuGet, Windows), System.Text.Json, Rust tokio::process, serde_json, Tauri 2 externalBin (Windows only)

---

## File Map

**Create:**
- `db2-sidecar/db2-sidecar.csproj` ✅
- `db2-sidecar/Models.cs` ✅
- `db2-sidecar/ConnectionManager.cs` ✅
- `db2-sidecar/CommandHandler.cs` ✅
- `db2-sidecar/Program.cs` ✅ (committed, smoke tests skipped — Windows-only binary)
- `src-tauri/src/db/db2_sidecar.rs` — Windows-only (`#[cfg(target_os="windows")]`)

**Modify (amended):**
- `src-tauri/src/db/db2.rs` — add `#[cfg]` platform dispatch; keep ODBC code for non-Windows
- `src-tauri/src/db/mod.rs` — add `pub mod db2_sidecar` (Windows-only)
- `src-tauri/src/commands/db.rs` — platform-conditional `build_adapter` for DB2
- `src-tauri/src/lib.rs` — manage `Db2SidecarState` on Windows only
- `src-tauri/Cargo.toml` — keep `odbc-api`, add `tokio/process` + `io-util` features
- `src-tauri/tauri.conf.json` — add `externalBin` for Windows sidecar

---

## Task 1: .NET sidecar project scaffolding

**Files:**
- Create: `db2-sidecar/db2-sidecar.csproj`
- Create: `db2-sidecar/Models.cs`

- [ ] **Step 1: Create the project file**

```xml
<!-- db2-sidecar/db2-sidecar.csproj -->
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <Nullable>enable</Nullable>
    <ImplicitUsings>enable</ImplicitUsings>
    <AssemblyName>db2-sidecar</AssemblyName>
    <RootNamespace>Db2Sidecar</RootNamespace>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="IBM.Data.Db2.Core" Version="7.*" />
  </ItemGroup>
</Project>
```

- [ ] **Step 2: Create Models.cs with request/response types**

```csharp
// db2-sidecar/Models.cs
namespace Db2Sidecar;

public class Request
{
    public string Id { get; set; } = "";
    public string Cmd { get; set; } = "";
    public string? ConnId { get; set; }
    public string? ConnString { get; set; }
    public string? Username { get; set; }
    public string? Password { get; set; }
    public string? Sql { get; set; }
    public string? Schema { get; set; }
    public string? Table { get; set; }
}

public class Response
{
    public string Id { get; set; } = "";
    public bool Ok { get; set; }
    public string? Error { get; set; }
    public List<string>? Columns { get; set; }
    public List<List<string?>>? Rows { get; set; }
    public long? AffectedRows { get; set; }
    public long ExecutionTimeMs { get; set; }
}
```

Note: `System.Text.Json` with `JsonNamingPolicy.SnakeCaseLower` will serialize `ConnId` → `conn_id`, `ExecutionTimeMs` → `execution_time_ms`, etc. No `[JsonPropertyName]` attributes needed.

- [ ] **Step 3: Restore packages**

Run: `cd db2-sidecar && dotnet restore`

Expected: packages restored without errors (IBM.Data.Db2.Core downloaded).

- [ ] **Step 4: Commit**

```bash
git add db2-sidecar/
git commit -m "feat(db2-sidecar): add .NET sidecar project with Models"
```

---

## Task 2: ConnectionManager

**Files:**
- Create: `db2-sidecar/ConnectionManager.cs`

- [ ] **Step 1: Write ConnectionManager.cs**

```csharp
// db2-sidecar/ConnectionManager.cs
using IBM.Data.Db2;

namespace Db2Sidecar;

public class ConnectionManager
{
    private readonly Dictionary<string, DB2Connection> _connections = new();

    /// <summary>
    /// Opens a DB2 connection and stores it under connId.
    /// Returns null on success, or an error message on failure.
    /// </summary>
    public async Task<string?> Connect(string connId, string connString, string username, string password)
    {
        try
        {
            // IBM.Data.Db2.Core accepts UID/PWD in the connection string
            var fullConnStr = $"{connString};UID={username};PWD={password};";
            var conn = new DB2Connection(fullConnStr);
            await conn.OpenAsync();
            _connections[connId] = conn;
            return null;
        }
        catch (Exception ex)
        {
            return ex.Message;
        }
    }

    /// <summary>Returns the live connection, or null if not found.</summary>
    public DB2Connection? Get(string connId) =>
        _connections.TryGetValue(connId, out var conn) ? conn : null;

    /// <summary>Closes and removes a connection. No-op if not found.</summary>
    public void Disconnect(string connId)
    {
        if (_connections.Remove(connId, out var conn))
        {
            conn.Close();
            conn.Dispose();
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd db2-sidecar && dotnet build`

Expected: Build succeeded, 0 warnings.

- [ ] **Step 3: Commit**

```bash
git add db2-sidecar/ConnectionManager.cs
git commit -m "feat(db2-sidecar): add ConnectionManager"
```

---

## Task 3: CommandHandler

**Files:**
- Create: `db2-sidecar/CommandHandler.cs`

- [ ] **Step 1: Write CommandHandler.cs**

```csharp
// db2-sidecar/CommandHandler.cs
using System.Diagnostics;
using IBM.Data.Db2;

namespace Db2Sidecar;

public static class CommandHandler
{
    public static async Task<Response> Handle(Request req, ConnectionManager cm)
    {
        return req.Cmd switch
        {
            "connect"          => await Connect(req, cm),
            "disconnect"       => Disconnect(req, cm),
            "ping"             => await Ping(req, cm),
            "execute"          => await Execute(req, cm),
            "list_schemas"     => await ListSchemas(req, cm),
            "list_tables"      => await ListTables(req, cm),
            "get_table_schema" => await GetTableSchema(req, cm),
            _                  => new Response { Id = req.Id, Ok = false, Error = $"unknown_cmd:{req.Cmd}" }
        };
    }

    // ── connect ──────────────────────────────────────────────────────────────

    private static async Task<Response> Connect(Request req, ConnectionManager cm)
    {
        var err = await cm.Connect(req.ConnId!, req.ConnString!, req.Username!, req.Password!);
        return err is null
            ? new Response { Id = req.Id, Ok = true }
            : new Response { Id = req.Id, Ok = false, Error = err };
    }

    // ── disconnect ───────────────────────────────────────────────────────────

    private static Response Disconnect(Request req, ConnectionManager cm)
    {
        cm.Disconnect(req.ConnId!);
        return new Response { Id = req.Id, Ok = true };
    }

    // ── ping ─────────────────────────────────────────────────────────────────

    private static async Task<Response> Ping(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        try
        {
            using var cmd = new DB2Command("SELECT 1 FROM SYSIBM.SYSDUMMY1", conn);
            await cmd.ExecuteScalarAsync();
            return new Response { Id = req.Id, Ok = true };
        }
        catch (Exception ex)
        {
            return new Response { Id = req.Id, Ok = false, Error = ex.Message };
        }
    }

    // ── execute ──────────────────────────────────────────────────────────────

    private static async Task<Response> Execute(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        return await RunSql(req.Id, conn, req.Sql!);
    }

    // ── list_schemas ─────────────────────────────────────────────────────────

    private static async Task<Response> ListSchemas(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        const string sql =
            "SELECT DISTINCT SCHEMANAME FROM SYSCAT.SCHEMATA " +
            "WHERE DEFINERTYPE = 'U' ORDER BY SCHEMANAME";
        return await RunSql(req.Id, conn, sql);
    }

    // ── list_tables ──────────────────────────────────────────────────────────

    private static async Task<Response> ListTables(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        var schema = req.Schema!.Replace("'", "''");
        var sql = $"SELECT TABNAME, TYPE FROM SYSCAT.TABLES WHERE TABSCHEMA = '{schema}' ORDER BY TABNAME";
        return await RunSql(req.Id, conn, sql);
    }

    // ── get_table_schema ─────────────────────────────────────────────────────

    private static async Task<Response> GetTableSchema(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        var schema = req.Schema!.Replace("'", "''");
        var table  = req.Table!.Replace("'", "''");
        var sql =
            $"SELECT COLNAME, TYPENAME, DEFAULT, NULLS " +
            $"FROM SYSCAT.COLUMNS " +
            $"WHERE TABSCHEMA = '{schema}' AND TABNAME = '{table}' " +
            $"ORDER BY COLNO";
        return await RunSql(req.Id, conn, sql);
    }

    // ── shared SQL runner ────────────────────────────────────────────────────

    private static async Task<Response> RunSql(string reqId, DB2Connection conn, string sql)
    {
        var sw = Stopwatch.StartNew();
        try
        {
            using var cmd    = new DB2Command(sql, conn);
            using var reader = await cmd.ExecuteReaderAsync();

            var columns = Enumerable.Range(0, reader.FieldCount)
                .Select(i => reader.GetName(i))
                .ToList();

            var rows = new List<List<string?>>();
            while (await reader.ReadAsync())
            {
                var row = Enumerable.Range(0, reader.FieldCount)
                    .Select(i => reader.IsDBNull(i) ? null : reader.GetValue(i)?.ToString())
                    .ToList();
                rows.Add(row);
            }

            sw.Stop();
            return new Response
            {
                Id = reqId, Ok = true,
                Columns = columns, Rows = rows,
                ExecutionTimeMs = sw.ElapsedMilliseconds
            };
        }
        catch (Exception ex)
        {
            sw.Stop();
            return new Response
            {
                Id = reqId, Ok = false,
                Error = ex.Message,
                ExecutionTimeMs = sw.ElapsedMilliseconds
            };
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd db2-sidecar && dotnet build`

Expected: Build succeeded.

- [ ] **Step 3: Commit**

```bash
git add db2-sidecar/CommandHandler.cs
git commit -m "feat(db2-sidecar): add CommandHandler with all DB2 commands"
```

---

## Task 4: Program.cs — stdin/stdout main loop

**Files:**
- Create: `db2-sidecar/Program.cs`

- [ ] **Step 1: Write Program.cs**

```csharp
// db2-sidecar/Program.cs
using System.Text.Json;
using System.Text.Json.Serialization;
using Db2Sidecar;

var jsonOptions = new JsonSerializerOptions
{
    PropertyNamingPolicy         = JsonNamingPolicy.SnakeCaseLower,
    DefaultIgnoreCondition       = JsonIgnoreCondition.WhenWritingNull,
    PropertyNameCaseInsensitive  = true,
};

var cm = new ConnectionManager();

string? line;
while ((line = Console.ReadLine()) is not null)
{
    if (string.IsNullOrWhiteSpace(line)) continue;
    try
    {
        var req  = JsonSerializer.Deserialize<Request>(line, jsonOptions)!;
        var resp = await CommandHandler.Handle(req, cm);
        Console.WriteLine(JsonSerializer.Serialize(resp, jsonOptions));
    }
    catch (Exception ex)
    {
        // Malformed JSON or unexpected error: emit a safe error response
        Console.WriteLine(JsonSerializer.Serialize(
            new Response { Id = "?", Ok = false, Error = ex.Message },
            jsonOptions));
    }
}
```

- [ ] **Step 2: Build and run a smoke test**

```bash
cd db2-sidecar && dotnet build
```

Then pipe a `connect` command (expect `{"id":"t1","ok":false,"error":"..."}` since no DB2 is running):

```bash
echo '{"id":"t1","cmd":"connect","conn_id":"c1","conn_string":"DATABASE=test;HOSTNAME=localhost;PORT=50000;PROTOCOL=TCPIP;","username":"u","password":"p"}' \
  | dotnet run --project db2-sidecar/db2-sidecar.csproj
```

Expected output (one JSON line): `{"id":"t1","ok":false,"error":"...connection refused..."}` or similar. The key is `ok` is `false` and the program exits cleanly.

- [ ] **Step 3: Test unknown command handling**

```bash
echo '{"id":"t2","cmd":"bogus"}' | dotnet run --project db2-sidecar/db2-sidecar.csproj
```

Expected: `{"id":"t2","ok":false,"error":"unknown_cmd:bogus"}`

- [ ] **Step 4: Test conn_not_found handling**

```bash
echo '{"id":"t3","cmd":"execute","conn_id":"nosuchconn","sql":"SELECT 1 FROM SYSIBM.SYSDUMMY1"}' \
  | dotnet run --project db2-sidecar/db2-sidecar.csproj
```

Expected: `{"id":"t3","ok":false,"error":"conn_not_found"}`

- [ ] **Step 5: Commit**

```bash
git add db2-sidecar/Program.cs
git commit -m "feat(db2-sidecar): add stdin/stdout main loop"
```

---

## Task 5: Build self-contained sidecar binaries

**Files:**
- Create directory: `src-tauri/binaries/`

- [ ] **Step 1: Create binaries directory**

```bash
mkdir -p src-tauri/binaries
```

- [ ] **Step 2: Build for current platform**

On macOS Apple Silicon:
```bash
dotnet publish db2-sidecar/db2-sidecar.csproj \
  -c Release -r osx-arm64 --self-contained \
  -p:PublishSingleFile=true \
  -o src-tauri/binaries/tmp-osx-arm64
cp src-tauri/binaries/tmp-osx-arm64/db2-sidecar \
   src-tauri/binaries/db2-sidecar-aarch64-apple-darwin
rm -rf src-tauri/binaries/tmp-osx-arm64
```

On macOS Intel:
```bash
dotnet publish db2-sidecar/db2-sidecar.csproj \
  -c Release -r osx-x64 --self-contained \
  -p:PublishSingleFile=true \
  -o src-tauri/binaries/tmp-osx-x64
cp src-tauri/binaries/tmp-osx-x64/db2-sidecar \
   src-tauri/binaries/db2-sidecar-x86_64-apple-darwin
rm -rf src-tauri/binaries/tmp-osx-x64
```

On Windows (run in PowerShell):
```powershell
dotnet publish db2-sidecar/db2-sidecar.csproj `
  -c Release -r win-x64 --self-contained `
  -p:PublishSingleFile=true `
  -o src-tauri\binaries\tmp-win-x64
Copy-Item src-tauri\binaries\tmp-win-x64\db2-sidecar.exe `
          src-tauri\binaries\db2-sidecar-x86_64-pc-windows-msvc.exe
Remove-Item -Recurse src-tauri\binaries\tmp-win-x64
```

On Linux x64:
```bash
dotnet publish db2-sidecar/db2-sidecar.csproj \
  -c Release -r linux-x64 --self-contained \
  -p:PublishSingleFile=true \
  -o src-tauri/binaries/tmp-linux-x64
cp src-tauri/binaries/tmp-linux-x64/db2-sidecar \
   src-tauri/binaries/db2-sidecar-x86_64-unknown-linux-gnu
rm -rf src-tauri/binaries/tmp-linux-x64
```

- [ ] **Step 3: Smoke-test the binary directly**

```bash
echo '{"id":"t1","cmd":"connect","conn_id":"c1","conn_string":"DATABASE=test;HOSTNAME=localhost;PORT=50000;PROTOCOL=TCPIP;","username":"u","password":"p"}' \
  | ./src-tauri/binaries/db2-sidecar-aarch64-apple-darwin
```

Expected: one JSON line with `"ok":false` (connection refused — no DB2 running).

- [ ] **Step 4: Add binaries/ to .gitignore (they are large compiled artifacts)**

Add to `.gitignore`:
```
src-tauri/binaries/
```

- [ ] **Step 5: Commit**

```bash
git add .gitignore
git commit -m "chore: ignore built db2-sidecar binaries"
```

---

## Task 6: Rust `Db2SidecarClient`

**Files:**
- Create: `src-tauri/src/db/db2_sidecar.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add `process` feature to tokio in Cargo.toml**

In `src-tauri/Cargo.toml`, change:
```toml
tokio = { version = "1", features = ["sync", "rt-multi-thread", "macros"] }
```
to:
```toml
tokio = { version = "1", features = ["sync", "rt-multi-thread", "macros", "process", "io-util"] }
```

Also remove `odbc-api`:
```toml
# DELETE this line:
odbc-api = "8"
```

- [ ] **Step 2: Write the failing test (add to end of db2_sidecar.rs)**

Create `src-tauri/src/db/db2_sidecar.rs` with just the test first:

```rust
//! Manages the db2-sidecar child process and provides JSON line I/O.

use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

struct SidecarIo {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct Db2SidecarClient {
    io: Mutex<SidecarIo>,
    _child: Mutex<Child>,
}

// ── placeholder so it compiles ───────────────────────────────────────────────
impl Db2SidecarClient {
    pub fn spawn(_path: PathBuf) -> Result<Self> {
        unimplemented!()
    }
    pub async fn send(&self, _req: Value) -> Result<Value> {
        unimplemented!()
    }
}

// ── State wrapper for lazy init ───────────────────────────────────────────────
pub struct Db2SidecarState {
    client: Mutex<Option<std::sync::Arc<Db2SidecarClient>>>,
    sidecar_path: PathBuf,
}

impl Db2SidecarState {
    pub fn new(sidecar_path: PathBuf) -> Self {
        Self { client: Mutex::new(None), sidecar_path }
    }
    pub async fn get_client(&self) -> Result<std::sync::Arc<Db2SidecarClient>> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sidecar_bin() -> PathBuf {
        // Path relative to the workspace root when running `cargo test`
        let mut p = std::env::current_dir().unwrap();
        p.push("binaries");
        // Adjust filename for current platform
        #[cfg(target_os = "macos")]
        p.push(if cfg!(target_arch = "aarch64") {
            "db2-sidecar-aarch64-apple-darwin"
        } else {
            "db2-sidecar-x86_64-apple-darwin"
        });
        #[cfg(target_os = "windows")]
        p.push("db2-sidecar-x86_64-pc-windows-msvc.exe");
        #[cfg(target_os = "linux")]
        p.push("db2-sidecar-x86_64-unknown-linux-gnu");
        p
    }

    #[tokio::test]
    async fn sidecar_returns_error_on_invalid_connect() {
        let client = Db2SidecarClient::spawn(sidecar_bin()).unwrap();
        let resp = client.send(serde_json::json!({
            "id": "test-1",
            "cmd": "connect",
            "conn_id": "c1",
            "conn_string": "DATABASE=test;HOSTNAME=127.0.0.1;PORT=50000;PROTOCOL=TCPIP;",
            "username": "u",
            "password": "p",
        })).await.unwrap();
        assert_eq!(resp["id"], "test-1");
        assert_eq!(resp["ok"], false);
        assert!(resp["error"].as_str().is_some());
    }

    #[tokio::test]
    async fn sidecar_returns_error_for_unknown_command() {
        let client = Db2SidecarClient::spawn(sidecar_bin()).unwrap();
        let resp = client.send(serde_json::json!({
            "id": "test-2",
            "cmd": "bogus_cmd",
        })).await.unwrap();
        assert_eq!(resp["ok"], false);
        let err = resp["error"].as_str().unwrap();
        assert!(err.contains("unknown_cmd"));
    }

    #[tokio::test]
    async fn sidecar_conn_not_found() {
        let client = Db2SidecarClient::spawn(sidecar_bin()).unwrap();
        let resp = client.send(serde_json::json!({
            "id": "test-3",
            "cmd": "execute",
            "conn_id": "no-such-conn",
            "sql": "SELECT 1 FROM SYSIBM.SYSDUMMY1",
        })).await.unwrap();
        assert_eq!(resp["ok"], false);
        assert_eq!(resp["error"], "conn_not_found");
    }
}
```

- [ ] **Step 3: Run tests to confirm they fail**

```bash
cd src-tauri && cargo test db::db2_sidecar 2>&1 | tail -20
```

Expected: tests panic with `unimplemented!()`.

- [ ] **Step 4: Implement `Db2SidecarClient::spawn` and `send`**

Replace the placeholder `impl Db2SidecarClient` block:

```rust
impl Db2SidecarClient {
    /// Spawn the sidecar process. Returns Err with "db2_sidecar_not_found:" prefix
    /// if the binary does not exist, so the frontend can show install guidance.
    pub fn spawn(path: PathBuf) -> Result<Self> {
        let mut child = tokio::process::Command::new(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    anyhow::anyhow!("db2_sidecar_not_found: {}", path.display())
                } else {
                    anyhow::anyhow!("failed to spawn db2-sidecar: {e}")
                }
            })?;

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));

        Ok(Self {
            io: Mutex::new(SidecarIo { stdin, stdout }),
            _child: Mutex::new(child),
        })
    }

    /// Send one JSON request, receive one JSON response.
    /// The Mutex ensures serial (one-at-a-time) request/response pairs.
    pub async fn send(&self, req: Value) -> Result<Value> {
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');

        let mut io = self.io.lock().await;
        io.stdin.write_all(line.as_bytes()).await?;
        io.stdin.flush().await?;

        let mut resp_line = String::new();
        io.stdout.read_line(&mut resp_line).await?;

        Ok(serde_json::from_str(resp_line.trim())?)
    }
}
```

- [ ] **Step 5: Implement `Db2SidecarState::get_client`**

Replace the placeholder `impl Db2SidecarState` block:

```rust
impl Db2SidecarState {
    pub fn new(sidecar_path: PathBuf) -> Self {
        Self { client: Mutex::new(None), sidecar_path }
    }

    /// Returns the shared sidecar client, spawning it on first call.
    pub async fn get_client(&self) -> Result<std::sync::Arc<Db2SidecarClient>> {
        let mut guard = self.client.lock().await;
        if let Some(ref c) = *guard {
            return Ok(c.clone());
        }
        let c = std::sync::Arc::new(Db2SidecarClient::spawn(self.sidecar_path.clone())?);
        *guard = Some(c.clone());
        Ok(c)
    }
}
```

- [ ] **Step 6: Add module to mod.rs**

In `src-tauri/src/db/mod.rs`, add:
```rust
pub mod db2_sidecar;
```

So it becomes:
```rust
pub mod adapter;
pub mod manager;
pub mod postgres;
pub mod mysql;
pub mod sqlite;
pub mod mssql;
pub mod db2;
pub mod db2_sidecar;

pub use adapter::{DbAdapter, TableInfo, ColumnInfo, QueryResult};
pub use manager::DbManager;
pub use db2_sidecar::Db2SidecarState;
```

- [ ] **Step 7: Run tests — they should pass now**

```bash
cd src-tauri && cargo test db::db2_sidecar 2>&1 | tail -20
```

Expected: 3 tests pass.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/db/db2_sidecar.rs src-tauri/src/db/mod.rs src-tauri/Cargo.toml
git commit -m "feat(db2): add Db2SidecarClient and Db2SidecarState"
```

---

## Task 7: New `Db2Adapter` (replaces ODBC version)

**Files:**
- Replace: `src-tauri/src/db/db2.rs`

- [ ] **Step 1: Write the failing test (append to db2.rs after implementing shell)**

Replace the entire content of `src-tauri/src/db/db2.rs`:

```rust
//! DB2 adapter — delegates all DB2 operations to the .NET db2-sidecar process.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use super::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use super::db2_sidecar::Db2SidecarClient;

pub struct Db2Adapter {
    conn_id: String,
    client: Arc<Db2SidecarClient>,
}

impl Db2Adapter {
    /// Establish a DB2 connection via the sidecar.
    ///
    /// `conn_string` should be an IBM.Data.Db2.Core connection string, e.g.:
    /// `"DATABASE=mydb;HOSTNAME=myhost;PORT=50000;PROTOCOL=TCPIP;"`
    pub async fn connect(
        client: Arc<Db2SidecarClient>,
        conn_string: String,
        username: String,
        password: String,
    ) -> Result<Self> {
        let conn_id = uuid::Uuid::new_v4().to_string();
        let resp = client
            .send(serde_json::json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "cmd": "connect",
                "conn_id": conn_id,
                "conn_string": conn_string,
                "username": username,
                "password": password,
            }))
            .await?;

        if !resp["ok"].as_bool().unwrap_or(false) {
            let err = resp["error"].as_str().unwrap_or("unknown").to_string();
            return Err(anyhow::anyhow!("{err}"));
        }

        Ok(Self { conn_id, client })
    }

    async fn cmd(&self, mut req: serde_json::Value) -> Result<serde_json::Value> {
        req["id"] = serde_json::Value::String(uuid::Uuid::new_v4().to_string());
        req["conn_id"] = serde_json::Value::String(self.conn_id.clone());
        self.client.send(req).await
    }
}

/// Send a disconnect on drop (fire-and-forget).
impl Drop for Db2Adapter {
    fn drop(&mut self) {
        let client = self.client.clone();
        let conn_id = self.conn_id.clone();
        tokio::spawn(async move {
            let _ = client
                .send(serde_json::json!({
                    "id": uuid::Uuid::new_v4().to_string(),
                    "cmd": "disconnect",
                    "conn_id": conn_id,
                }))
                .await;
        });
    }
}

#[async_trait]
impl DbAdapter for Db2Adapter {
    async fn test(&self) -> Result<()> {
        let resp = self.cmd(serde_json::json!({"cmd": "ping"})).await?;
        if resp["ok"].as_bool().unwrap_or(false) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("{}", resp["error"].as_str().unwrap_or("ping failed")))
        }
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        let resp = self.cmd(serde_json::json!({"cmd": "list_schemas"})).await?;
        check_ok(&resp)?;
        Ok(resp["rows"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|row| row.as_array()?.first()?.as_str().map(|s| s.to_string()))
            .collect())
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let resp = self
            .cmd(serde_json::json!({"cmd": "list_tables", "schema": schema}))
            .await?;
        check_ok(&resp)?;
        Ok(resp["rows"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|row| {
                let arr = row.as_array()?;
                let name = arr.first()?.as_str()?.to_string();
                let tt = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
                Some(TableInfo {
                    name,
                    table_type: if tt == "V" { "view".into() } else { "table".into() },
                })
            })
            .collect())
    }

    async fn get_table_schema(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let resp = self
            .cmd(serde_json::json!({"cmd": "get_table_schema", "schema": schema, "table": table}))
            .await?;
        check_ok(&resp)?;
        Ok(resp["rows"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|row| {
                let arr = row.as_array()?;
                let s = |i: usize| arr.get(i)?.as_str().map(|s| s.to_string()).unwrap_or_default();
                let default_val = s(2);
                Some(ColumnInfo {
                    name: s(0),
                    data_type: s(1),
                    nullable: s(3) == "Y",
                    default: if default_val.is_empty() { None } else { Some(default_val) },
                })
            })
            .collect())
    }

    async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let resp = self
            .cmd(serde_json::json!({"cmd": "execute", "sql": sql}))
            .await?;

        let elapsed = resp["execution_time_ms"].as_u64().unwrap_or(0);

        if !resp["ok"].as_bool().unwrap_or(false) {
            return Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                affected_rows: None,
                execution_time_ms: elapsed,
                error: Some(resp["error"].as_str().unwrap_or("unknown").to_string()),
            });
        }

        let columns: Vec<String> = resp["columns"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let rows: Vec<Vec<serde_json::Value>> = resp["rows"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|row| {
                row.as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::Null => serde_json::Value::Null,
                        serde_json::Value::String(s) => serde_json::Value::String(s.clone()),
                        other => serde_json::Value::String(other.to_string()),
                    })
                    .collect()
            })
            .collect();

        Ok(QueryResult {
            columns,
            rows,
            affected_rows: resp["affected_rows"].as_u64(),
            execution_time_ms: elapsed,
            error: None,
        })
    }
}

fn check_ok(resp: &serde_json::Value) -> Result<()> {
    if resp["ok"].as_bool().unwrap_or(false) {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{}", resp["error"].as_str().unwrap_or("db2 error")))
    }
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd src-tauri && cargo build 2>&1 | tail -20
```

Expected: no errors. Warnings about unused imports from old db2.rs should be gone.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db/db2.rs
git commit -m "feat(db2): replace ODBC adapter with .NET sidecar delegation"
```

---

## Task 8: Wire up in Tauri

**Files:**
- Modify: `src-tauri/src/commands/db.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Update `commands/db.rs` — add sidecar import and update `build_adapter`**

At the top of `src-tauri/src/commands/db.rs`, add to imports:
```rust
use crate::db::{
    adapter::{ColumnInfo, QueryResult, TableInfo},
    db2::Db2Adapter,
    db2_sidecar::Db2SidecarState,   // ADD THIS
    manager::DbManager,
    mssql::MssqlAdapter,
    mysql::MySqlAdapter,
    postgres::PostgresAdapter,
    sqlite::SqliteAdapter,
};
```

- [ ] **Step 2: Update `build_adapter` signature and DB2 arm**

Change `build_adapter`:
```rust
async fn build_adapter(
    conn: &DbConnection,
    password: &str,
    sidecar: &Db2SidecarState,          // ADD parameter
) -> anyhow::Result<Box<dyn crate::db::adapter::DbAdapter>> {
    match conn.db_type {
        DbType::Postgresql => { /* unchanged */ }
        DbType::Mysql      => { /* unchanged */ }
        DbType::Sqlite     => { /* unchanged */ }
        DbType::Mssql      => { /* unchanged */ }
        DbType::Db2 => {
            // IBM.Data.Db2.Core connection string format (no ODBC DSN needed)
            let cs = format!(
                "DATABASE={};HOSTNAME={};PORT={};PROTOCOL=TCPIP;",
                conn.database, conn.host, conn.port
            );
            let client = sidecar.get_client().await?;
            Ok(Box::new(
                Db2Adapter::connect(client, cs, conn.username.clone(), password.to_string())
                    .await?,
            ))
        }
    }
}
```

- [ ] **Step 3: Update `db_connect` to accept and pass sidecar state**

```rust
#[tauri::command]
pub async fn db_connect(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    manager: State<'_, DbManager>,
    sidecar: State<'_, Db2SidecarState>,    // ADD THIS
) -> Result<(), String> {
    if manager.is_connected(&id).await {
        return Ok(());
    }
    let conn = config
        .get()
        .db_connections
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("connection not found: {id}"))?;
    let password = secrets.get(&secret_key(&id)).ok().flatten().unwrap_or_default();
    let adapter = build_adapter(&conn, &password, &sidecar)
        .await
        .map_err(|e| e.to_string())?;
    manager.insert(id, adapter).await;
    Ok(())
}
```

- [ ] **Step 4: Update `db_test_connection` to accept and pass sidecar state**

```rust
#[tauri::command]
pub async fn db_test_connection(
    input: DbConnectionInput,
    secrets: State<'_, Arc<SecretStore>>,
    sidecar: State<'_, Db2SidecarState>,    // ADD THIS
) -> Result<(), String> {
    let password = if !input.password.is_empty() {
        input.password.clone()
    } else if let Some(id) = &input.id {
        secrets.get(&secret_key(id)).ok().flatten().unwrap_or_default()
    } else {
        String::new()
    };

    let temp_conn = DbConnection {
        id: input.id.clone().unwrap_or_default(),
        name: input.name.clone(),
        db_type: input.db_type,
        host: input.host.clone(),
        port: input.port,
        database: input.database.clone(),
        username: input.username.clone(),
    };

    let adapter = build_adapter(&temp_conn, &password, &sidecar)
        .await
        .map_err(|e| e.to_string())?;
    adapter.test().await.map_err(|e| e.to_string())
}
```

- [ ] **Step 5: Update `lib.rs` — manage `Db2SidecarState`**

In `src-tauri/src/lib.rs`, add the import:
```rust
use db::Db2SidecarState;
```

In the `run()` function, resolve the sidecar path and add it to managed state:
```rust
pub fn run() {
    let config  = Arc::new(ConfigStore::new());
    let secrets = Arc::new(SecretStore::new());
    let router  = AiRouter::new(config.clone(), secrets.clone());

    // Resolve path to db2-sidecar binary (placed next to the main executable)
    let sidecar_path = {
        let mut p = std::env::current_exe()
            .expect("current_exe")
            .parent()
            .expect("parent dir")
            .to_path_buf();
        #[cfg(target_os = "windows")]
        p.push("db2-sidecar-x86_64-pc-windows-msvc.exe");
        #[cfg(not(target_os = "windows"))]
        {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            p.push("db2-sidecar-aarch64-apple-darwin");
            #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
            p.push("db2-sidecar-x86_64-apple-darwin");
            #[cfg(target_os = "linux")]
            p.push("db2-sidecar-x86_64-unknown-linux-gnu");
        }
        p
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())
        .manage(PtyManager::new())
        .manage(config)
        .manage(secrets)
        .manage(router)
        .manage(DbManager::new())
        .manage(Db2SidecarState::new(sidecar_path))   // ADD THIS
        .invoke_handler(tauri::generate_handler![
            /* all existing handlers unchanged */
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 6: Update `tauri.conf.json` — add externalBin**

In `src-tauri/tauri.conf.json`, update the `bundle` section:
```json
"bundle": {
  "active": true,
  "targets": "all",
  "icon": [
    "icons/32x32.png",
    "icons/128x128.png",
    "icons/128x128@2x.png",
    "icons/icon.icns",
    "icons/icon.ico"
  ],
  "externalBin": [
    "binaries/db2-sidecar"
  ]
}
```

- [ ] **Step 7: Build to verify everything compiles**

```bash
cd src-tauri && cargo build 2>&1 | tail -30
```

Expected: Build succeeded, 0 errors.

- [ ] **Step 8: Run existing tests to confirm nothing broke**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: All previously passing tests still pass. DB2 tests remain `#[ignore]` (require real DB2).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/commands/db.rs src-tauri/src/lib.rs src-tauri/tauri.conf.json
git commit -m "feat(db2): wire Db2SidecarState into Tauri commands and bundle config"
```

---

## Task 9: Final verification

- [ ] **Step 1: Full cargo build + test**

```bash
cd src-tauri && cargo test 2>&1 | grep -E "^(test|FAILED|error)" | head -40
```

Expected: all tests pass, no compilation errors.

- [ ] **Step 2: Type-check frontend (no frontend changes, just confirm)**

```bash
npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 3: Tauri dev smoke test (optional, requires macOS with Tauri toolchain)**

```bash
npm run tauri:dev
```

Open a DB2 connection in the UI — expect a proper error (connection refused or invalid host) rather than a crash, confirming the sidecar spawns and responds correctly.

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat(db2): complete DB2 .NET sidecar integration

Replaces IBM ODBC Driver requirement with IBM.Data.Db2.Core via
a long-running .NET sidecar process communicating over stdin/stdout JSON."
```

---

## Self-Review Checklist (for implementor)

- [ ] `odbc-api` removed from `Cargo.toml`
- [ ] `tokio` has `process` and `io-util` features
- [ ] `Db2SidecarState` in managed state in `lib.rs`
- [ ] Both `db_connect` and `db_test_connection` accept `State<'_, Db2SidecarState>`
- [ ] DB2 connection string uses `.NET` format (`HOSTNAME=...;PORT=...`) not ODBC DSN
- [ ] Sidecar binary naming matches Tauri triple convention
- [ ] `src-tauri/binaries/` in `.gitignore`
