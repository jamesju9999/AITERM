# DB2 Sidecar Java Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the broken C# db2-sidecar with a Java implementation using IBM JDBC (`db2jcc4.jar`), enabling DB2 connectivity on both macOS (ARM64) and Windows (x64).

**Architecture:** A new `db2-sidecar-java/` Maven project produces a fat jar (`db2sidecar.jar`) bundled with a minimal Temurin 21 JRE. The Rust spawn code in `db2_sidecar.rs` is updated to launch `jre/bin/java -jar db2sidecar.jar` instead of a native binary. The stdin/stdout JSON protocol is preserved identically.

**Tech Stack:** Java 21, IBM JDBC `com.ibm.db2:jcc:12.1.0.0` (Maven Central), Jackson `2.17.2`, Maven Shade Plugin, Eclipse Temurin 21 JRE, Rust (spawn changes only)

---

## File Map

**Create:**
- `db2-sidecar-java/pom.xml` — Maven build: IBM JDBC + Jackson deps + shade fat-jar
- `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/Main.java` — stdin/stdout loop
- `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/Request.java` — POJO deserialized from snake_case JSON
- `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/Response.java` — POJO serialized to snake_case JSON
- `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/ConnectionManager.java` — JDBC connection pool
- `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/CommandHandler.java` — 8 commands
- `scripts/setup-db2-win.ps1` — Windows PowerShell setup (new)

**Modify:**
- `scripts/setup-db2-mac.sh` — Full rewrite: remove clidriver/GCC logic, add mvn + JRE download
- `src-tauri/src/db/db2_sidecar.rs` — Spawn Java instead of native binary
- `src-tauri/src/lib.rs` — Detect sidecar directory (not binary path)
- `src-tauri/src/commands/db.rs:134-138` — Change connection string format from `Server=host:port;Database=db;` to `jdbc:db2://host:port/db`
- `src-tauri/tauri.macos.conf.json` — Resources destination `"."` → `"db2-sidecar"`
- `src-tauri/tauri.windows.conf.json` — Resources destination `"."` → `"db2-sidecar"`

**Bundle structure (both platforms):**
```
src-tauri/binaries/db2-sidecar-{mac-arm64,win-x64}/
├── db2sidecar.jar
└── jre/
    └── bin/
        └── java          (java.exe on Windows)
```

---

### Task 1: Maven project scaffolding

**Files:**
- Create: `db2-sidecar-java/pom.xml`

- [ ] **Step 1: Create the Maven project directory**

```bash
mkdir -p db2-sidecar-java/src/main/java/com/aiterm/db2sidecar
```

- [ ] **Step 2: Create `db2-sidecar-java/pom.xml`**

```xml
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0
                             https://maven.apache.org/xsd/maven-4.0.0.xsd">
  <modelVersion>4.0.0</modelVersion>

  <groupId>com.aiterm</groupId>
  <artifactId>db2sidecar</artifactId>
  <version>1.0.0</version>
  <packaging>jar</packaging>

  <properties>
    <maven.compiler.source>21</maven.compiler.source>
    <maven.compiler.target>21</maven.compiler.target>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
  </properties>

  <dependencies>
    <!-- IBM DB2 JDBC driver (pure Java, no native libs needed) -->
    <dependency>
      <groupId>com.ibm.db2</groupId>
      <artifactId>jcc</artifactId>
      <version>12.1.0.0</version>
    </dependency>
    <!-- JSON serialization -->
    <dependency>
      <groupId>com.fasterxml.jackson.core</groupId>
      <artifactId>jackson-databind</artifactId>
      <version>2.17.2</version>
    </dependency>
  </dependencies>

  <build>
    <plugins>
      <plugin>
        <groupId>org.apache.maven.plugins</groupId>
        <artifactId>maven-shade-plugin</artifactId>
        <version>3.6.0</version>
        <executions>
          <execution>
            <phase>package</phase>
            <goals><goal>shade</goal></goals>
            <configuration>
              <finalName>db2sidecar</finalName>
              <createDependencyReducedPom>false</createDependencyReducedPom>
              <transformers>
                <transformer implementation="org.apache.maven.plugins.shade.resource.ManifestResourceTransformer">
                  <mainClass>com.aiterm.db2sidecar.Main</mainClass>
                </transformer>
                <transformer implementation="org.apache.maven.plugins.shade.resource.ServicesResourceTransformer"/>
              </transformers>
              <filters>
                <filter>
                  <artifact>*:*</artifact>
                  <excludes>
                    <exclude>META-INF/*.SF</exclude>
                    <exclude>META-INF/*.DSA</exclude>
                    <exclude>META-INF/*.RSA</exclude>
                  </excludes>
                </filter>
              </filters>
            </configuration>
          </execution>
        </executions>
      </plugin>
    </plugins>
  </build>
</project>
```

- [ ] **Step 3: Verify Maven can resolve dependencies**

```bash
cd db2-sidecar-java && mvn dependency:resolve -q
```

Expected: BUILD SUCCESS with no errors. IBM jcc 12.1.0.0 should download from Maven Central.

---

### Task 2: Data models (Request.java and Response.java)

**Files:**
- Create: `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/Request.java`
- Create: `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/Response.java`

- [ ] **Step 1: Create `Request.java`**

Jackson's `SNAKE_CASE` naming strategy maps JSON `conn_id` → Java `connId`, etc.

```java
package com.aiterm.db2sidecar;

public class Request {
    public String id = "";
    public String cmd = "";
    public String connId;
    public String connString;
    public String username;
    public String password;
    public String sql;
    public String schema;
    public String table;
}
```

- [ ] **Step 2: Create `Response.java`**

```java
package com.aiterm.db2sidecar;

import com.fasterxml.jackson.annotation.JsonInclude;
import java.util.List;

@JsonInclude(JsonInclude.Include.NON_NULL)
public class Response {
    public String id = "";
    public boolean ok;
    public String error;
    public List<String> columns;
    public List<List<String>> rows;
    public Long affectedRows;
    public long executionTimeMs;
}
```

- [ ] **Step 3: Compile to verify no syntax errors**

```bash
cd db2-sidecar-java && mvn compile -q
```

Expected: BUILD SUCCESS

---

### Task 3: ConnectionManager.java

**Files:**
- Create: `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/ConnectionManager.java`

- [ ] **Step 1: Create `ConnectionManager.java`**

```java
package com.aiterm.db2sidecar;

import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.SQLException;
import java.util.HashMap;
import java.util.Map;

public class ConnectionManager {
    private final Map<String, Connection> connections = new HashMap<>();

    /**
     * Opens a JDBC connection and stores it under connId.
     * Returns null on success, or an error message on failure.
     */
    public String connect(String connId, String jdbcUrl, String username, String password) {
        try {
            // Ensure the IBM JDBC driver is loaded
            Class.forName("com.ibm.db2.jcc.DB2Driver");
            Connection conn = DriverManager.getConnection(jdbcUrl, username, password);
            Connection existing = connections.put(connId, conn);
            if (existing != null) {
                try { existing.close(); } catch (SQLException ignored) {}
            }
            return null;
        } catch (ClassNotFoundException e) {
            return "IBM JDBC driver not found: " + e.getMessage();
        } catch (SQLException e) {
            return formatSqlError(e);
        }
    }

    /** Returns the live connection, or null if not found. */
    public Connection get(String connId) {
        return connections.get(connId);
    }

    /** Closes and removes a connection. No-op if not found. */
    public void disconnect(String connId) {
        Connection conn = connections.remove(connId);
        if (conn != null) {
            try { conn.close(); } catch (SQLException ignored) {}
        }
    }

    private String formatSqlError(SQLException e) {
        StringBuilder sb = new StringBuilder(e.getMessage());
        sb.append(" [SQLSTATE=").append(e.getSQLState())
          .append(" ErrorCode=").append(e.getErrorCode()).append("]");
        return sb.toString();
    }
}
```

- [ ] **Step 2: Compile**

```bash
cd db2-sidecar-java && mvn compile -q
```

Expected: BUILD SUCCESS

---

### Task 4: CommandHandler.java

**Files:**
- Create: `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/CommandHandler.java`

This mirrors `db2-sidecar/CommandHandler.cs` exactly — same SQL queries, same error strings.

- [ ] **Step 1: Create `CommandHandler.java`**

```java
package com.aiterm.db2sidecar;

import java.sql.*;
import java.util.ArrayList;
import java.util.List;

public class CommandHandler {

    public static Response handle(Request req, ConnectionManager cm) throws Exception {
        switch (req.cmd) {
            case "connect":        return connect(req, cm);
            case "disconnect":     return disconnect(req, cm);
            case "ping":           return ping(req, cm);
            case "execute":        return execute(req, cm);
            case "list_schemas":   return listSchemas(req, cm);
            case "list_tables":    return listTables(req, cm);
            case "get_table_schema": return getTableSchema(req, cm);
            default:
                Response r = new Response();
                r.id = req.id;
                r.ok = false;
                r.error = "unknown_cmd:" + req.cmd;
                return r;
        }
    }

    private static Response connect(Request req, ConnectionManager cm) {
        String err = cm.connect(req.connId, req.connString, req.username, req.password);
        Response r = new Response();
        r.id = req.id;
        r.ok = (err == null);
        r.error = err;
        return r;
    }

    private static Response disconnect(Request req, ConnectionManager cm) {
        cm.disconnect(req.connId);
        Response r = new Response();
        r.id = req.id;
        r.ok = true;
        return r;
    }

    private static Response ping(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        try {
            try (Statement stmt = conn.createStatement()) {
                stmt.executeQuery("SELECT 1 FROM SYSIBM.SYSDUMMY1").close();
            }
            Response r = new Response();
            r.id = req.id;
            r.ok = true;
            return r;
        } catch (SQLException e) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = e.getMessage();
            return r;
        }
    }

    private static Response execute(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        return runSql(req.id, conn, req.sql);
    }

    private static Response listSchemas(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        String sql = "SELECT DISTINCT SCHEMANAME FROM SYSCAT.SCHEMATA " +
                     "WHERE DEFINERTYPE = 'U' ORDER BY SCHEMANAME";
        return runSql(req.id, conn, sql);
    }

    private static Response listTables(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        String schema = req.schema.replace("'", "''");
        String sql = "SELECT TABNAME, TYPE FROM SYSCAT.TABLES " +
                     "WHERE TABSCHEMA = '" + schema + "' ORDER BY TABNAME";
        return runSql(req.id, conn, sql);
    }

    private static Response getTableSchema(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        String schema = req.schema.replace("'", "''");
        String table  = req.table.replace("'", "''");
        String sql = "SELECT COLNAME, TYPENAME, DEFAULT, NULLS " +
                     "FROM SYSCAT.COLUMNS " +
                     "WHERE TABSCHEMA = '" + schema + "' AND TABNAME = '" + table + "' " +
                     "ORDER BY COLNO";
        return runSql(req.id, conn, sql);
    }

    private static Response runSql(String reqId, Connection conn, String sql) {
        long start = System.currentTimeMillis();
        try (Statement stmt = conn.createStatement()) {
            boolean hasResultSet = stmt.execute(sql);
            long elapsed = System.currentTimeMillis() - start;
            Response r = new Response();
            r.id = reqId;
            r.ok = true;
            r.executionTimeMs = elapsed;
            if (hasResultSet) {
                try (ResultSet rs = stmt.getResultSet()) {
                    ResultSetMetaData meta = rs.getMetaData();
                    int colCount = meta.getColumnCount();
                    List<String> columns = new ArrayList<>(colCount);
                    for (int i = 1; i <= colCount; i++) {
                        columns.add(meta.getColumnName(i));
                    }
                    List<List<String>> rows = new ArrayList<>();
                    while (rs.next()) {
                        List<String> row = new ArrayList<>(colCount);
                        for (int i = 1; i <= colCount; i++) {
                            String val = rs.getString(i);
                            row.add(rs.wasNull() ? null : val);
                        }
                        rows.add(row);
                    }
                    r.columns = columns;
                    r.rows = rows;
                }
            } else {
                r.affectedRows = (long) stmt.getUpdateCount();
            }
            return r;
        } catch (SQLException e) {
            long elapsed = System.currentTimeMillis() - start;
            Response r = new Response();
            r.id = reqId;
            r.ok = false;
            r.error = e.getMessage();
            r.executionTimeMs = elapsed;
            return r;
        }
    }
}
```

- [ ] **Step 2: Compile**

```bash
cd db2-sidecar-java && mvn compile -q
```

Expected: BUILD SUCCESS

---

### Task 5: Main.java and fat-jar build

**Files:**
- Create: `db2-sidecar-java/src/main/java/com/aiterm/db2sidecar/Main.java`

- [ ] **Step 1: Create `Main.java`**

```java
package com.aiterm.db2sidecar;

import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;
import com.fasterxml.jackson.annotation.JsonInclude;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;

public class Main {
    public static void main(String[] args) throws Exception {
        ObjectMapper mapper = new ObjectMapper()
            .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
            .setSerializationInclusion(JsonInclude.Include.NON_NULL)
            .configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);

        ConnectionManager cm = new ConnectionManager();

        BufferedReader in = new BufferedReader(
            new InputStreamReader(System.in, StandardCharsets.UTF_8));
        PrintStream out = new PrintStream(System.out, true, StandardCharsets.UTF_8);

        String line;
        while ((line = in.readLine()) != null) {
            if (line.isBlank()) continue;
            try {
                Request req = mapper.readValue(line, Request.class);
                Response resp = CommandHandler.handle(req, cm);
                out.println(mapper.writeValueAsString(resp));
            } catch (Exception ex) {
                Response err = new Response();
                err.id = "?";
                err.ok = false;
                err.error = ex.getMessage();
                out.println(mapper.writeValueAsString(err));
            }
        }
    }
}
```

- [ ] **Step 2: Build the fat jar**

```bash
cd db2-sidecar-java && mvn package -q
```

Expected: BUILD SUCCESS. Output: `db2-sidecar-java/target/db2sidecar.jar`

- [ ] **Step 3: Smoke-test the jar locally (requires local JRE)**

```bash
echo '{"id":"t1","cmd":"connect","conn_id":"c1","conn_string":"jdbc:db2://172.19.2.83:25000/LBOTHODC","username":"db2admin","password":"!qaz2wsx"}' \
  | java -jar db2-sidecar-java/target/db2sidecar.jar
```

Expected output (one JSON line):
```json
{"id":"t1","ok":true,"execution_time_ms":0}
```

- [ ] **Step 4: Commit**

```bash
git add db2-sidecar-java/
git commit -m "feat(db2): add Java sidecar using IBM JDBC (db2jcc4)"
```

---

### Task 6: Rust — update spawn, path detection, and connection string

**Files:**
- Modify: `src-tauri/src/db/db2_sidecar.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/db.rs:134-138`

The `sidecar_path` passed from `lib.rs` now points to the **directory** (not a binary). `spawn()` finds `jre/bin/java[.exe]` and `db2sidecar.jar` within it.

- [ ] **Step 1: Rewrite `src-tauri/src/db/db2_sidecar.rs`**

Replace the entire `spawn` function body and remove all native env-var logic:

```rust
pub fn spawn(sidecar_dir: PathBuf) -> Result<Self> {
    #[cfg(target_os = "windows")]
    let java_bin = sidecar_dir.join("jre").join("bin").join("java.exe");
    #[cfg(not(target_os = "windows"))]
    let java_bin = sidecar_dir.join("jre").join("bin").join("java");

    let jar = sidecar_dir.join("db2sidecar.jar");

    if !jar.exists() {
        return Err(anyhow::anyhow!(
            "db2_sidecar_not_found: {}",
            jar.display()
        ));
    }

    let mut cmd = tokio::process::Command::new(&java_bin);
    cmd.arg("-jar")
        .arg(&jar)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(&sidecar_dir);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("db2_sidecar_not_found: java not found at {}", java_bin.display())
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
```

Also update `Db2SidecarState::new` to take `sidecar_dir: PathBuf` (rename the field):

```rust
pub struct Db2SidecarState {
    client: Mutex<Option<Arc<Db2SidecarClient>>>,
    sidecar_dir: PathBuf,
}

impl Db2SidecarState {
    pub fn new(sidecar_dir: PathBuf) -> Self {
        Self {
            client: Mutex::new(None),
            sidecar_dir,
        }
    }

    pub async fn reset(&self) {
        *self.client.lock().await = None;
    }

    pub async fn get_client(&self) -> Result<Arc<Db2SidecarClient>> {
        let mut guard = self.client.lock().await;
        if let Some(ref c) = *guard {
            return Ok(c.clone());
        }
        let c = Arc::new(Db2SidecarClient::spawn(self.sidecar_dir.clone())?);
        *guard = Some(c.clone());
        Ok(c)
    }
}
```

Full file after rewrite:

```rust
//! Manages the db2-sidecar child process and provides JSON line I/O.

use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

struct SidecarIo {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct Db2SidecarClient {
    io: Mutex<SidecarIo>,
    _child: Mutex<Child>,
}

impl Db2SidecarClient {
    /// Spawn the sidecar. `sidecar_dir` must contain `db2sidecar.jar` and `jre/bin/java[.exe]`.
    /// Returns Err with "db2_sidecar_not_found:" prefix if the jar is missing.
    pub fn spawn(sidecar_dir: PathBuf) -> Result<Self> {
        #[cfg(target_os = "windows")]
        let java_bin = sidecar_dir.join("jre").join("bin").join("java.exe");
        #[cfg(not(target_os = "windows"))]
        let java_bin = sidecar_dir.join("jre").join("bin").join("java");

        let jar = sidecar_dir.join("db2sidecar.jar");

        if !jar.exists() {
            return Err(anyhow::anyhow!(
                "db2_sidecar_not_found: {}",
                jar.display()
            ));
        }

        let mut cmd = tokio::process::Command::new(&java_bin);
        cmd.arg("-jar")
            .arg(&jar)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(&sidecar_dir);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "db2_sidecar_not_found: java not found at {}",
                    java_bin.display()
                )
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
    pub async fn send(&self, req: Value) -> Result<Value> {
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');

        let mut io = self.io.lock().await;
        io.stdin.write_all(line.as_bytes()).await?;
        io.stdin.flush().await?;

        for _ in 0..8 {
            let mut resp_bytes = Vec::new();
            timeout(
                Duration::from_secs(20),
                io.stdout.read_until(b'\n', &mut resp_bytes),
            )
            .await
            .map_err(|_| anyhow::anyhow!("db2_sidecar_timeout: no response in 20s"))??;

            if resp_bytes.is_empty() {
                return Err(anyhow::anyhow!("db2_sidecar_died: EOF on stdout"));
            }

            let resp_line = match String::from_utf8(resp_bytes) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
            };
            let cleaned = resp_line.trim().trim_start_matches('\u{feff}');
            if cleaned.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(cleaned) {
                return Ok(v);
            }
        }

        Err(anyhow::anyhow!(
            "db2_sidecar_invalid_json: did not receive parseable JSON line"
        ))
    }
}

pub struct Db2SidecarState {
    client: Mutex<Option<Arc<Db2SidecarClient>>>,
    sidecar_dir: PathBuf,
}

impl Db2SidecarState {
    pub fn new(sidecar_dir: PathBuf) -> Self {
        Self {
            client: Mutex::new(None),
            sidecar_dir,
        }
    }

    /// Clears the cached client so the next `get_client` call re-spawns the sidecar.
    pub async fn reset(&self) {
        *self.client.lock().await = None;
    }

    /// Returns the shared sidecar client, spawning it on first call.
    pub async fn get_client(&self) -> Result<Arc<Db2SidecarClient>> {
        let mut guard = self.client.lock().await;
        if let Some(ref c) = *guard {
            return Ok(c.clone());
        }
        let c = Arc::new(Db2SidecarClient::spawn(self.sidecar_dir.clone())?);
        *guard = Some(c.clone());
        Ok(c)
    }
}
```

- [ ] **Step 2: Update `src-tauri/src/lib.rs` — detect sidecar DIRECTORY**

Replace the `sidecar_path` block (lines 59–134). The logic changes from finding a binary file to finding a directory. Check for `db2sidecar.jar` inside to confirm it's valid.

Windows block (replace lines 60–93):
```rust
#[cfg(target_os = "windows")]
{
    let exe_dir = std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("parent dir")
        .to_path_buf();
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let candidates = [
        // Production: resources bundle into exe_dir/db2-sidecar/
        exe_dir.join("db2-sidecar"),
        // Dev: binaries dir
        manifest_dir
            .join("binaries")
            .join("db2-sidecar-win-x64"),
    ];

    candidates
        .into_iter()
        .find(|p| p.join("db2sidecar.jar").exists())
        .unwrap_or_else(|| exe_dir.join("db2-sidecar"))
}
```

macOS block (replace lines 94–127):
```rust
#[cfg(target_os = "macos")]
{
    let exe_dir = std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("parent dir")
        .to_path_buf();

    let contents_dir = exe_dir.parent()
        .expect("Contents dir")
        .to_path_buf();

    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    #[cfg(target_arch = "aarch64")]
    let dev_subdir = "db2-sidecar-mac-arm64";
    #[cfg(target_arch = "x86_64")]
    let dev_subdir = "db2-sidecar-mac-x64";

    let candidates = [
        // Production: Tauri resources land in Contents/Resources/db2-sidecar/
        contents_dir.join("Resources").join("db2-sidecar"),
        // Dev: local build output
        manifest_dir
            .join("binaries")
            .join(dev_subdir),
    ];

    candidates
        .into_iter()
        .find(|p| p.join("db2sidecar.jar").exists())
        .unwrap_or_else(|| contents_dir.join("Resources").join("db2-sidecar"))
}
```

- [ ] **Step 3: Update `src-tauri/src/commands/db.rs:134-138` — change connection string format**

Find the DB2 connection string at lines 134–138:
```rust
DbType::Db2 => {
    let cs = format!(
        "Server={}:{};Database={};",
        conn.host, conn.port, conn.database
    );
```

Change to:
```rust
DbType::Db2 => {
    let cs = format!(
        "jdbc:db2://{}:{}/{}",
        conn.host, conn.port, conn.database
    );
```

- [ ] **Step 4: Verify Rust compiles**

```bash
cd src-tauri && cargo check 2>&1 | tail -5
```

Expected: `Finished` with no errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/db2_sidecar.rs src-tauri/src/lib.rs src-tauri/src/commands/db.rs
git commit -m "feat(db2): update Rust sidecar spawn to launch Java jar"
```

---

### Task 7: Rewrite macOS setup script

**Files:**
- Modify: `scripts/setup-db2-mac.sh`

Remove all clidriver, GCC runtime, and install_name_tool logic. Add Maven build + Temurin JRE download.

- [ ] **Step 1: Replace `scripts/setup-db2-mac.sh`**

```bash
#!/usr/bin/env bash
# Setup DB2 Java sidecar for macOS (Apple Silicon ARM64)
# Run once from the workspace root: bash scripts/setup-db2-mac.sh

set -euo pipefail

DEST="src-tauri/binaries/db2-sidecar-mac-arm64"

echo "==> Cleaning output directory: $DEST"
rm -rf "$DEST"
mkdir -p "$DEST"

echo "==> Building db2sidecar.jar via Maven..."
(cd db2-sidecar-java && mvn package -q --no-transfer-progress)
test -f "db2-sidecar-java/target/db2sidecar.jar" || {
  echo "ERROR: mvn package produced no jar at db2-sidecar-java/target/db2sidecar.jar"
  exit 1
}
cp "db2-sidecar-java/target/db2sidecar.jar" "$DEST/db2sidecar.jar"
echo "  Copied db2sidecar.jar"

echo "==> Downloading Eclipse Temurin 21 JRE (macOS ARM64)..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

JRE_URL="https://api.adoptium.net/v3/binary/latest/21/ga/mac/aarch64/jre/hotspot/normal/eclipse"
curl -L --fail "$JRE_URL" -o "$TMP/jre.tar.gz"

echo "==> Extracting JRE..."
tar -xzf "$TMP/jre.tar.gz" -C "$TMP"

# Temurin extracts as jdk-21.*-jre/ — find the extracted directory
JRE_DIR=$(find "$TMP" -maxdepth 1 -name "jdk-*-jre" -type d | head -1)
if [[ -z "$JRE_DIR" ]]; then
  # Some builds extract as a .jre directory
  JRE_DIR=$(find "$TMP" -maxdepth 1 -name "*.jre" -type d | head -1)
fi
if [[ -z "$JRE_DIR" ]]; then
  echo "ERROR: Could not locate extracted JRE directory in $TMP"
  ls "$TMP"
  exit 1
fi

# On macOS, Temurin JRE contains a Contents/Home structure
if [[ -d "$JRE_DIR/Contents/Home" ]]; then
  JRE_HOME="$JRE_DIR/Contents/Home"
else
  JRE_HOME="$JRE_DIR"
fi

mkdir -p "$DEST/jre"
cp -R "$JRE_HOME/." "$DEST/jre/"

echo "==> Verifying java binary..."
"$DEST/jre/bin/java" -version 2>&1 | head -1

echo ""
echo "Done. DB2 Java sidecar ready at: $DEST/"
echo "  db2sidecar.jar          (IBM JDBC fat jar)"
echo "  jre/bin/java            (Temurin 21 ARM64)"
echo ""
echo "Run 'npm run tauri:dev' to start the app."
```

- [ ] **Step 2: Run the script and verify**

```bash
bash scripts/setup-db2-mac.sh
```

Expected:
- No errors
- `src-tauri/binaries/db2-sidecar-mac-arm64/db2sidecar.jar` exists
- `src-tauri/binaries/db2-sidecar-mac-arm64/jre/bin/java` exists and prints version

- [ ] **Step 3: Quick integration test with live DB2**

```bash
SIDECAR="src-tauri/binaries/db2-sidecar-mac-arm64"
echo '{"id":"t1","cmd":"connect","conn_id":"c1","conn_string":"jdbc:db2://172.19.2.83:25000/LBOTHODC","username":"db2admin","password":"!qaz2wsx"}' \
  | "$SIDECAR/jre/bin/java" -jar "$SIDECAR/db2sidecar.jar"
```

Expected: `{"id":"t1","ok":true,"execution_time_ms":...}`

- [ ] **Step 4: Test list_schemas**

```bash
SIDECAR="src-tauri/binaries/db2-sidecar-mac-arm64"
{
  echo '{"id":"t1","cmd":"connect","conn_id":"c1","conn_string":"jdbc:db2://172.19.2.83:25000/LBOTHODC","username":"db2admin","password":"!qaz2wsx"}'
  echo '{"id":"t2","cmd":"list_schemas","conn_id":"c1"}'
} | "$SIDECAR/jre/bin/java" -jar "$SIDECAR/db2sidecar.jar"
```

Expected: Two JSON lines, second has `"ok":true` with `"columns":["SCHEMANAME"]` and rows.

- [ ] **Step 5: Commit**

```bash
git add scripts/setup-db2-mac.sh
git commit -m "feat(db2): rewrite macOS setup script for Java sidecar"
```

---

### Task 8: Windows setup script

**Files:**
- Create: `scripts/setup-db2-win.ps1`

- [ ] **Step 1: Create `scripts/setup-db2-win.ps1`**

```powershell
# Setup DB2 Java sidecar for Windows x64
# Run once from the workspace root: powershell -ExecutionPolicy Bypass -File scripts\setup-db2-win.ps1

$ErrorActionPreference = "Stop"

$DEST = "src-tauri\binaries\db2-sidecar-win-x64"

Write-Host "==> Cleaning output directory: $DEST"
if (Test-Path $DEST) { Remove-Item $DEST -Recurse -Force }
New-Item $DEST -ItemType Directory | Out-Null

Write-Host "==> Building db2sidecar.jar via Maven..."
Push-Location "db2-sidecar-java"
mvn package -q --no-transfer-progress
Pop-Location

$JAR = "db2-sidecar-java\target\db2sidecar.jar"
if (-not (Test-Path $JAR)) {
    Write-Error "ERROR: mvn package produced no jar at $JAR"
    exit 1
}
Copy-Item $JAR "$DEST\db2sidecar.jar"
Write-Host "  Copied db2sidecar.jar"

Write-Host "==> Downloading Eclipse Temurin 21 JRE (Windows x64)..."
$TMP = New-TemporaryFile | ForEach-Object { Remove-Item $_; New-Item -ItemType Directory $_.FullName }
$JRE_URL = "https://api.adoptium.net/v3/binary/latest/21/ga/windows/x64/jre/hotspot/normal/eclipse"
$JRE_ZIP = "$TMP\jre.zip"
Invoke-WebRequest -Uri $JRE_URL -OutFile $JRE_ZIP -UseBasicParsing

Write-Host "==> Extracting JRE..."
Expand-Archive -Path $JRE_ZIP -DestinationPath $TMP

# Temurin extracts as jdk-21.*-jre\ on Windows
$JRE_DIR = Get-ChildItem $TMP -Directory | Where-Object { $_.Name -match "jdk-.*-jre" } | Select-Object -First 1
if (-not $JRE_DIR) {
    Write-Error "ERROR: Could not locate extracted JRE directory in $TMP"
    Get-ChildItem $TMP
    exit 1
}

New-Item "$DEST\jre" -ItemType Directory | Out-Null
Copy-Item "$($JRE_DIR.FullName)\*" "$DEST\jre\" -Recurse

Write-Host "==> Verifying java.exe..."
& "$DEST\jre\bin\java.exe" -version

Remove-Item $TMP -Recurse -Force

Write-Host ""
Write-Host "Done. DB2 Java sidecar ready at: $DEST\"
Write-Host "  db2sidecar.jar          (IBM JDBC fat jar)"
Write-Host "  jre\bin\java.exe        (Temurin 21 x64)"
Write-Host ""
Write-Host "Run 'npm run tauri:dev' to start the app."
```

- [ ] **Step 2: Commit (no run needed — verify on Windows CI)**

```bash
git add scripts/setup-db2-win.ps1
git commit -m "feat(db2): add Windows setup script for Java sidecar"
```

---

### Task 9: Tauri config — fix resource destination paths

**Files:**
- Modify: `src-tauri/tauri.macos.conf.json`
- Modify: `src-tauri/tauri.windows.conf.json`

Currently both map the binary directory to `"."` (bundle root). With Java we need a named subdirectory so `db2sidecar.jar` and `jre/` don't collide with app files.

- [ ] **Step 1: Update `src-tauri/tauri.macos.conf.json`**

Current:
```json
{
  "bundle": {
    "externalBin": [],
    "resources": {
      "binaries/db2-sidecar-mac-arm64": "."
    }
  }
}
```

Change to:
```json
{
  "bundle": {
    "externalBin": [],
    "resources": {
      "binaries/db2-sidecar-mac-arm64": "db2-sidecar"
    }
  }
}
```

This places files at `Contents/Resources/db2-sidecar/db2sidecar.jar` and `Contents/Resources/db2-sidecar/jre/bin/java` — matching the `lib.rs` production path.

- [ ] **Step 2: Update `src-tauri/tauri.windows.conf.json`**

Current:
```json
{
  "bundle": {
    "externalBin": [],
    "resources": {
      "binaries/db2-sidecar-win-x64": "."
    }
  }
}
```

Change to:
```json
{
  "bundle": {
    "externalBin": [],
    "resources": {
      "binaries/db2-sidecar-win-x64": "db2-sidecar"
    }
  }
}
```

This places files at `exe_dir/db2-sidecar/db2sidecar.jar` and `exe_dir/db2-sidecar/jre/bin/java.exe` — matching the `lib.rs` Windows production path.

- [ ] **Step 3: Verify Tauri builds without error (macOS)**

```bash
npm run tauri:build 2>&1 | tail -20
```

Expected: Build succeeds and bundle contains `db2-sidecar/db2sidecar.jar`.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tauri.macos.conf.json src-tauri/tauri.windows.conf.json
git commit -m "fix(tauri): map db2-sidecar resources to named subdirectory"
```

---

## Self-Review

### Spec coverage
- ✅ Java project with IBM JDBC: Task 1
- ✅ Data models matching C# protocol: Task 2
- ✅ ConnectionManager with JDBC DriverManager: Task 3
- ✅ All 8 commands with exact SQL: Task 4
- ✅ stdin/stdout loop with snake_case JSON: Task 5
- ✅ Rust spawn updated to java -jar: Task 6 (db2_sidecar.rs)
- ✅ Rust path detection updated to directory: Task 6 (lib.rs)
- ✅ Connection string format updated: Task 6 (commands/db.rs)
- ✅ macOS setup script rewritten: Task 7
- ✅ Windows setup script created: Task 8
- ✅ Tauri resource destination fixed: Task 9

### Type consistency
- `Request.connId` (Java camelCase) → JSON `conn_id` via SNAKE_CASE strategy ✅
- `Response.executionTimeMs` → JSON `execution_time_ms` ✅
- Rust `Db2SidecarState::new(sidecar_dir)` called from `lib.rs` with directory PathBuf ✅
- Rust `build_adapter` passes JDBC URL `jdbc:db2://host:port/db` to sidecar `connect` command ✅
