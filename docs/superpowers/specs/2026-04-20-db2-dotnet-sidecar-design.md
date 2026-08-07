# DB2 .NET Sidecar Design

**Date:** 2026-04-20  
**Status:** Approved (amended 2026-04-20 — dual-path platform strategy)

## Problem

The current `Db2Adapter` uses the `odbc-api` Rust crate, which requires the IBM DB2 ODBC Driver installed on the host machine. This driver is difficult to obtain. The `IBM.Data.Db2.Core` NuGet package provides first-class DB2 connectivity in .NET, but its bundled clidriver is **Windows-only** — no macOS/Linux native libraries are included.

## Solution

**Dual-path strategy by platform:**

- **Windows**: delegate all DB2 operations to a long-running .NET sidecar process (`db2-sidecar`) using `IBM.Data.Db2.Core`. The NuGet package bundles the Windows clidriver automatically — no separate driver installation required.
- **macOS / Linux**: keep the existing ODBC-based `Db2Adapter`. Users must install the IBM DB2 ODBC Driver, same as before.

The `Db2Adapter` name is preserved; platform selection is done at **compile time** via `#[cfg(target_os = "windows")]` / `#[cfg(not(target_os = "windows"))]` in Rust.

---

## Architecture

```
Tauri App
├── Rust backend
│   ├── DbManager (unchanged)
│   ├── Db2Adapter — platform-selected at compile time:
│   │   ├── [Windows]     → delegates via Db2SidecarClient (new)
│   │   └── [macOS/Linux] → existing ODBC adapter (unchanged)
│   └── Db2SidecarClient (new, Windows-only)
│       ├── holds stdin/stdout handles to child process
│       └── serializes JSON requests / deserializes JSON responses
│
└── db2-sidecar (.NET 8 console app, Windows self-contained binary only)
    ├── reads stdin JSON lines, writes stdout JSON lines
    ├── maintains conn_id → DB2Connection map internally
    └── uses IBM.Data.Db2.Core NuGet package (bundles Windows clidriver)
```

**Flow:**
1. First DB2 connection request → Rust lazy-spawns `db2-sidecar`
2. Each `DbAdapter` method on `Db2Adapter` sends a JSON request via `Db2SidecarClient`
3. Sidecar executes the operation and returns a JSON response
4. `Db2Adapter` maps the response to existing `QueryResult` / `ColumnInfo` types
5. `DbManager` is unaware of the underlying change — interface is unchanged

---

## JSON Line Protocol

One JSON object per line (`\n` terminated). The `id` field correlates requests to responses.

### Requests (Rust → sidecar)

```json
{"id":"<uuid>","cmd":"connect","conn_id":"<conn-uuid>","conn_string":"DATABASE=mydb;HOSTNAME=...","username":"u","password":"p"}
{"id":"<uuid>","cmd":"ping","conn_id":"<conn-uuid>"}
{"id":"<uuid>","cmd":"execute","conn_id":"<conn-uuid>","sql":"SELECT 1 FROM SYSIBM.SYSDUMMY1"}
{"id":"<uuid>","cmd":"list_schemas","conn_id":"<conn-uuid>"}
{"id":"<uuid>","cmd":"list_tables","conn_id":"<conn-uuid>","schema":"MYSCHEMA"}
{"id":"<uuid>","cmd":"get_table_schema","conn_id":"<conn-uuid>","schema":"MYSCHEMA","table":"MYTABLE"}
{"id":"<uuid>","cmd":"disconnect","conn_id":"<conn-uuid>"}
```

### Responses (sidecar → Rust)

```json
{"id":"<uuid>","ok":true}
{"id":"<uuid>","ok":true,"columns":["A","B"],"rows":[["v1","v2"]],"affected_rows":null,"execution_time_ms":12}
{"id":"<uuid>","ok":false,"error":"CONNECTION_FAILED: <detail>"}
```

- All errors are returned as `{"ok":false,"error":"..."}` — sidecar never crashes on DB errors
- Sidecar stderr is reserved for debug logging; Rust ignores it by default

---

## .NET Sidecar

### Project structure

```
db2-sidecar/
├── db2-sidecar.csproj   (.NET 8, OutputType=Exe, PublishSingleFile=true)
├── Program.cs           (stdin/stdout main loop)
├── ConnectionManager.cs (Dictionary<string, DB2Connection>)
└── Commands.cs          (per-command handlers)
```

### NuGet dependency

`IBM.Data.Db2.Core` — includes DB2 client libraries; no separate IBM driver installation required.

### Main loop (Program.cs)

```csharp
while ((line = Console.ReadLine()) != null) {
    var req = JsonSerializer.Deserialize<Request>(line);
    var response = await HandleCommand(req);
    Console.WriteLine(JsonSerializer.Serialize(response));
}
```

### Build (Windows only — clidriver is Windows-exclusive in this NuGet package)

```bash
dotnet publish -c Release -r win-x64 --self-contained -p:PublishSingleFile=false -o dist/win-x64/
```

`PublishSingleFile=false` is required so the clidriver DLLs that IBM.Data.Db2.Core extracts into the output directory are included alongside the executable. The entire `dist/win-x64/` directory (including `clidriver/`) is bundled into the Tauri Windows build.

---

## Rust Side

### New files

| File | Purpose |
|------|---------|
| `src-tauri/src/db/db2_sidecar.rs` | `Db2SidecarClient` + `Db2SidecarState` — Windows-only, `#[cfg(target_os="windows")]` |
| `src-tauri/src/db/db2.rs` | `Db2Adapter` — sidecar on Windows, ODBC on macOS/Linux |

### Platform dispatch in `db2.rs`

```rust
#[cfg(target_os = "windows")]
mod platform { /* sidecar-based Db2Adapter */ }

#[cfg(not(target_os = "windows"))]
mod platform { /* ODBC-based Db2Adapter (existing code) */ }

pub use platform::Db2Adapter;
```

### `Db2SidecarClient` (Windows only)

- `send(&self, req: Value) -> Result<Value>` — one Mutex wraps stdin+stdout for serial access
- Stored as `Arc<Db2SidecarClient>` in `tauri::State` behind `Db2SidecarState`
- `Db2SidecarState` is only `.manage()`d on Windows via `#[cfg]`

### Cargo.toml changes

- **Keep** `odbc-api = "8"` (macOS/Linux DB2 path unchanged)
- Add `process` and `io-util` features to `tokio`
- `odbc-api` is conditionally used; no platform feature gate needed at crate level

---

## Sidecar Lifecycle

| Event | Behavior |
|-------|---------|
| First DB2 connection request | Rust lazy-spawns sidecar, stores `Arc<Db2SidecarClient>` in app state |
| App shutdown | `Child::kill()` in Tauri `on_window_event` / `RunEvent::Exit` |
| Sidecar crash (pipe broken) | Next `send()` detects EOF, auto-respawns, retries once; returns error if retry fails |
| Sidecar binary missing | Returns `Err` with `"db2_sidecar_not_found"` prefix for frontend to surface install guidance |

---

## Tauri Bundling

`tauri.conf.json`:
```json
"bundle": {
  "externalBin": ["binaries/db2-sidecar"]
}
```

Tauri selects the correct binary by platform triple:
- `binaries/db2-sidecar-aarch64-apple-darwin`
- `binaries/db2-sidecar-x86_64-pc-windows-msvc.exe`
- `binaries/db2-sidecar-x86_64-unknown-linux-gnu`

---

## Error Handling

| Scenario | Error value | Frontend handling |
|----------|------------|-------------------|
| DB2 connection refused | `"CONNECTION_FAILED:..."` | Existing error UI |
| Auth failure | `"AUTH_FAILED:..."` | Existing error UI |
| Sidecar binary not found | `"db2_sidecar_not_found:..."` | Show install guidance |
| Sidecar crash | Auto-retry once, then `"SIDECAR_UNAVAILABLE"` | Generic error UI |

---

## Testing

- **Sidecar (.NET):** xUnit unit tests with mocked stdin/stdout; no real DB2 required
- **Rust `Db2SidecarClient`:** integration tests with real sidecar binary, marked `#[ignore]` (require DB2 environment)
- **`DbManager` tests:** unchanged (use SQLite adapter)
