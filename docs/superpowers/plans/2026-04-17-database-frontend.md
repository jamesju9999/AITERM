# Database Tab — Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the full database tab UI to AITerm: a `NewTabPicker` dropdown on the `+` button, a `DatabaseView` with Browse / AI Chat / SQL Editor sub-tabs, a connection selector overlay, and a settings page for managing database connections.

**Architecture:** Extend `Tab` with a `type` field. `TerminalApp` shows `NewTabPicker` on `+` and renders `DatabaseView` for database tabs. `DatabaseView` uses three sub-components (Browser, SqlEditor, AiChat) toggled by sub-tabs. Settings gains a `DatabaseConnectionsPage`. All DB operations go through `src/ipc/db.ts`.

**Tech Stack:** React 19, TypeScript, Vitest + Testing Library (existing pattern), Tauri `invoke` (existing pattern). Run tests with `npm run test`.

**Prerequisite:** The backend plan (`2026-04-17-database-backend.md`) must be complete and `cargo build` must succeed before starting this plan.

---

## File Map

**Create:**
- `src/ipc/db.ts`
- `src/components/NewTabPicker/index.tsx`
- `src/components/NewTabPicker/index.css`
- `src/components/DatabaseView/index.tsx`
- `src/components/DatabaseView/ConnectionSelector.tsx`
- `src/components/DatabaseView/DatabaseBrowser.tsx`
- `src/components/DatabaseView/DatabaseSqlEditor.tsx`
- `src/components/DatabaseView/DatabaseAiChat.tsx`
- `src/components/DatabaseView/index.css`
- `src/components/Settings/DatabaseConnectionsPage.tsx`
- `src/components/DatabaseView/DatabaseView.test.tsx`
- `src/components/NewTabPicker/NewTabPicker.test.tsx`
- `src/components/Settings/DatabaseConnectionsPage.test.tsx`

**Modify:**
- `src/components/TabBar/index.tsx` — add `type` and `dbConnectionId` to `Tab` interface
- `src/components/TerminalApp.tsx` — show `NewTabPicker` on `+`, render `DatabaseView` for db tabs
- `src/components/Settings/SettingsView.tsx` — add "資料庫連線" nav item

---

### Task 13: Frontend IPC bindings

**Files:**
- Create: `src/ipc/db.ts`

- [ ] **Step 1: Write the IPC module**

```typescript
import { invoke } from "@tauri-apps/api/core";

export type DbType = "postgresql" | "mysql" | "sqlite" | "mssql" | "db2";

export interface DbConnectionInfo {
  id: string;
  name: string;
  db_type: DbType;
  host: string;
  port: number;
  database: string;
  username: string;
  is_connected: boolean;
}

export interface DbConnectionInput {
  id?: string;
  name: string;
  db_type: DbType;
  host: string;
  port: number;
  database: string;
  username: string;
  password: string;
}

export interface TableInfo {
  name: string;
  table_type: "table" | "view";
}

export interface ColumnInfo {
  name: string;
  data_type: string;
  nullable: boolean;
  default: string | null;
}

export interface QueryResult {
  columns: string[];
  rows: (string | number | boolean | null)[][];
  affected_rows: number | null;
  execution_time_ms: number;
  error: string | null;
}

export const DB_TYPE_LABELS: Record<DbType, string> = {
  postgresql: "PostgreSQL",
  mysql: "MySQL",
  sqlite: "SQLite",
  mssql: "MSSQL",
  db2: "DB2",
};

export const DB_DEFAULT_PORTS: Record<DbType, number> = {
  postgresql: 5432,
  mysql: 3306,
  sqlite: 0,
  mssql: 1433,
  db2: 50000,
};

export function dbListConnections(): Promise<DbConnectionInfo[]> {
  return invoke("db_list_connections");
}

export function dbAddConnection(input: DbConnectionInput): Promise<string> {
  return invoke("db_add_connection", { input });
}

export function dbUpdateConnection(input: DbConnectionInput): Promise<void> {
  return invoke("db_update_connection", { input });
}

export function dbRemoveConnection(id: string): Promise<void> {
  return invoke("db_remove_connection", { id });
}

export function dbTestConnection(input: DbConnectionInput): Promise<void> {
  return invoke("db_test_connection", { input });
}

export function dbConnect(id: string): Promise<void> {
  return invoke("db_connect", { id });
}

export function dbDisconnect(id: string): Promise<void> {
  return invoke("db_disconnect", { id });
}

export function dbListSchemas(connectionId: string): Promise<string[]> {
  return invoke("db_list_schemas", { connectionId });
}

export function dbListTables(connectionId: string, schema: string): Promise<TableInfo[]> {
  return invoke("db_list_tables", { connectionId, schema });
}

export function dbGetTableSchema(connectionId: string, schema: string, table: string): Promise<ColumnInfo[]> {
  return invoke("db_get_table_schema", { connectionId, schema, table });
}

export function dbExecuteQuery(connectionId: string, sql: string): Promise<QueryResult> {
  return invoke("db_execute_query", { connectionId, sql });
}
```

- [ ] **Step 2: Compile check**

```bash
npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors in `src/ipc/db.ts`.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/db.ts
git commit -m "feat(db): add frontend IPC bindings"
```

---

### Task 14: Extend Tab type

**Files:**
- Modify: `src/components/TabBar/index.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/components/NewTabPicker/NewTabPicker.test.tsx`:

```typescript
import { describe, it, expect } from "vitest";
import { Tab } from "../TabBar";

describe("Tab type", () => {
  it("accepts terminal type", () => {
    const tab: Tab = { id: "1", title: "Terminal", type: "terminal" };
    expect(tab.type).toBe("terminal");
  });

  it("accepts database type with connection id", () => {
    const tab: Tab = { id: "2", title: "DB", type: "database", dbConnectionId: "conn-1" };
    expect(tab.type).toBe("database");
    expect(tab.dbConnectionId).toBe("conn-1");
  });
});
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
npm run test -- NewTabPicker.test 2>&1 | tail -10
```

Expected: type error — `type` property does not exist on `Tab`.

- [ ] **Step 3: Update the Tab interface**

In `src/components/TabBar/index.tsx`, change:

```typescript
export interface Tab {
  id: string;
  title: string;
}
```

to:

```typescript
export interface Tab {
  id: string;
  title: string;
  type: "terminal" | "database";
  dbConnectionId?: string;
}
```

- [ ] **Step 4: Fix the one place that creates a Tab in TerminalApp.tsx**

In `src/components/TerminalApp.tsx`, find the initial `useState<Tab[]>` and add `type: "terminal"`:

```typescript
const [tabs, setTabs] = useState<Tab[]>(() => [
  { id: crypto.randomUUID(), title: "Terminal", type: "terminal" },
]);
```

Also in `handleCloseTab` where it creates a fallback new tab:
```typescript
return [{ id: newId, title: "Terminal", type: "terminal" }];
```

- [ ] **Step 5: Run tests**

```bash
npm run test -- NewTabPicker.test 2>&1 | tail -10
```

Expected: both tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/components/TabBar/index.tsx src/components/TerminalApp.tsx src/components/NewTabPicker/NewTabPicker.test.tsx
git commit -m "feat(tabs): extend Tab type with type and dbConnectionId fields"
```

---

### Task 15: NewTabPicker component

**Files:**
- Create: `src/components/NewTabPicker/index.tsx`
- Create: `src/components/NewTabPicker/index.css`

- [ ] **Step 1: Write tests**

Add to `src/components/NewTabPicker/NewTabPicker.test.tsx`:

```typescript
import { render, screen, fireEvent } from "@testing-library/react";
import { NewTabPicker } from "./index";

describe("NewTabPicker", () => {
  it("renders two options when open", () => {
    render(<NewTabPicker onSelect={() => {}} onClose={() => {}} />);
    expect(screen.getByText("終端機")).toBeInTheDocument();
    expect(screen.getByText("資料庫")).toBeInTheDocument();
  });

  it("calls onSelect with terminal when 終端機 clicked", () => {
    const onSelect = vi.fn();
    render(<NewTabPicker onSelect={onSelect} onClose={() => {}} />);
    fireEvent.click(screen.getByText("終端機"));
    expect(onSelect).toHaveBeenCalledWith("terminal");
  });

  it("calls onSelect with database when 資料庫 clicked", () => {
    const onSelect = vi.fn();
    render(<NewTabPicker onSelect={onSelect} onClose={() => {}} />);
    fireEvent.click(screen.getByText("資料庫"));
    expect(onSelect).toHaveBeenCalledWith("database");
  });

  it("calls onClose when Escape pressed", () => {
    const onClose = vi.fn();
    render(<NewTabPicker onSelect={() => {}} onClose={onClose} />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
npm run test -- NewTabPicker.test 2>&1 | tail -5
```

Expected: import error or render failure.

- [ ] **Step 3: Create the component**

Create `src/components/NewTabPicker/index.tsx`:

```typescript
import { useEffect, useRef } from "react";
import "./index.css";

interface Props {
  onSelect: (type: "terminal" | "database") => void;
  onClose: () => void;
}

export function NewTabPicker({ onSelect, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("keydown", handleKey);
    document.addEventListener("mousedown", handleClick);
    return () => {
      document.removeEventListener("keydown", handleKey);
      document.removeEventListener("mousedown", handleClick);
    };
  }, [onClose]);

  return (
    <div className="new-tab-picker" ref={ref}>
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("terminal"); onClose(); }}
      >
        <span className="new-tab-picker__icon">⬛</span>
        <div>
          <div className="new-tab-picker__label">終端機</div>
          <div className="new-tab-picker__desc">開啟新 Shell Session</div>
        </div>
      </button>
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("database"); onClose(); }}
      >
        <span className="new-tab-picker__icon">🗄️</span>
        <div>
          <div className="new-tab-picker__label">資料庫</div>
          <div className="new-tab-picker__desc">連接資料庫並瀏覽</div>
        </div>
      </button>
    </div>
  );
}
```

Create `src/components/NewTabPicker/index.css`:

```css
.new-tab-picker {
  position: absolute;
  top: 100%;
  left: 0;
  z-index: 200;
  background: #1a1a1a;
  border: 1px solid #2a2a2a;
  border-radius: 6px;
  padding: 4px;
  min-width: 200px;
  box-shadow: 0 4px 16px rgba(0,0,0,0.5);
}

.new-tab-picker__item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  background: transparent;
  border: none;
  border-radius: 4px;
  padding: 10px 12px;
  cursor: pointer;
  color: #e6e6e6;
  text-align: left;
  transition: background 0.1s;
}

.new-tab-picker__item:hover {
  background: #2a2a2a;
}

.new-tab-picker__icon {
  font-size: 20px;
  flex-shrink: 0;
}

.new-tab-picker__label {
  font-size: 13px;
  font-weight: 500;
  color: #e6e6e6;
}

.new-tab-picker__desc {
  font-size: 11px;
  color: #888;
  margin-top: 2px;
}
```

- [ ] **Step 4: Run tests**

```bash
npm run test -- NewTabPicker.test 2>&1 | tail -10
```

Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/NewTabPicker/
git commit -m "feat(ui): add NewTabPicker dropdown component"
```

---

### Task 16: Update TerminalApp to handle tab types

**Files:**
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: Update handleAddTab to show picker**

In `src/components/TerminalApp.tsx`:

1. Add state for picker visibility at the top of `TerminalApp`:

```typescript
const [pickerOpen, setPickerOpen] = useState(false);
```

2. Replace `handleAddTab` with:

```typescript
const handleAddTab = useCallback(() => {
  setPickerOpen(true);
}, []);

const handlePickerSelect = useCallback((type: "terminal" | "database") => {
  const newId = crypto.randomUUID();
  const title = type === "terminal" ? "Terminal" : "資料庫";
  setTabs((prev) => [...prev, { id: newId, title, type }]);
  setActiveId(newId);
  setPickerOpen(false);
}, []);
```

3. Add import:

```typescript
import { NewTabPicker } from "./NewTabPicker";
import { DatabaseView } from "./DatabaseView";
```

4. In the JSX, wrap the `TabBar` in a relative container and add the picker:

```typescript
<div style={{ position: "relative" }}>
  <TabBar
    tabs={tabs}
    activeId={activeId}
    onSelect={setActiveId}
    onClose={handleCloseTab}
    onAdd={handleAddTab}
    onRename={handleRename}
    isSidebarOpen={isSidebarOpen}
    onToggle={toggleSidebar}
    width={sidebarWidth}
  />
  {pickerOpen && (
    <NewTabPicker
      onSelect={handlePickerSelect}
      onClose={() => setPickerOpen(false)}
    />
  )}
</div>
```

5. In the tab rendering section, replace the single `<TerminalView>` render with a type-based switch:

```typescript
{tabs.map((tab) => {
  const isActive = tab.id === activeId;
  return (
    <div
      key={tab.id}
      style={{
        position: "absolute", top: 0, left: 0, right: 0, bottom: 0,
        visibility: isActive ? "visible" : "hidden",
        zIndex: isActive ? 1 : -1,
        pointerEvents: isActive ? "auto" : "none",
      }}
    >
      {tab.type === "database" ? (
        <DatabaseView
          tabId={tab.id}
          isActive={isActive}
          dbConnectionId={tab.dbConnectionId}
          onConnectionSelected={(connId) => {
            setTabs((prev) =>
              prev.map((t) => t.id === tab.id ? { ...t, dbConnectionId: connId } : t)
            );
          }}
        />
      ) : (
        <TerminalView isActive={isActive} />
      )}
    </div>
  );
})}
```

- [ ] **Step 2: Compile check**

```bash
npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors (DatabaseView doesn't exist yet — add a stub if needed).

Create a temporary stub `src/components/DatabaseView/index.tsx` so TypeScript is happy:

```typescript
export interface DatabaseViewProps {
  tabId: string;
  isActive: boolean;
  dbConnectionId?: string;
  onConnectionSelected: (connId: string) => void;
}

export function DatabaseView(_props: DatabaseViewProps) {
  return <div style={{ color: "#888", padding: "20px" }}>Database view coming soon...</div>;
}
```

- [ ] **Step 3: Verify app starts**

```bash
npm run tauri:dev 2>&1 &
sleep 10 && echo "running"
```

Expected: app opens, clicking `+` shows the picker with Terminal / 資料庫 options.

- [ ] **Step 4: Commit**

```bash
git add src/components/TerminalApp.tsx src/components/DatabaseView/index.tsx
git commit -m "feat(tabs): wire NewTabPicker into TerminalApp, add DatabaseView stub"
```

---

### Task 17: DatabaseConnectionsPage (Settings)

**Files:**
- Create: `src/components/Settings/DatabaseConnectionsPage.tsx`

- [ ] **Step 1: Write the test**

Create `src/components/Settings/DatabaseConnectionsPage.test.tsx`:

```typescript
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";

vi.mock("../../ipc/db", () => ({
  dbListConnections: vi.fn().mockResolvedValue([]),
  dbAddConnection: vi.fn().mockResolvedValue("new-id"),
  dbRemoveConnection: vi.fn().mockResolvedValue(undefined),
  dbTestConnection: vi.fn().mockResolvedValue(undefined),
  DB_TYPE_LABELS: { postgresql: "PostgreSQL", mysql: "MySQL", sqlite: "SQLite", mssql: "MSSQL", db2: "DB2" },
  DB_DEFAULT_PORTS: { postgresql: 5432, mysql: 3306, sqlite: 0, mssql: 1433, db2: 50000 },
}));

describe("DatabaseConnectionsPage", () => {
  it("renders empty state and add button", async () => {
    render(<DatabaseConnectionsPage />);
    await waitFor(() => expect(screen.getByText("+ 新增連線")).toBeInTheDocument());
  });

  it("shows form when + button clicked", async () => {
    render(<DatabaseConnectionsPage />);
    await waitFor(() => fireEvent.click(screen.getByText("+ 新增連線")));
    expect(screen.getByPlaceholderText("我的資料庫")).toBeInTheDocument();
  });

  it("shows DB2 ODBC notice when DB2 selected", async () => {
    render(<DatabaseConnectionsPage />);
    await waitFor(() => fireEvent.click(screen.getByText("+ 新增連線")));
    const select = screen.getByRole("combobox");
    fireEvent.change(select, { target: { value: "db2" } });
    expect(screen.getByText(/IBM DB2 ODBC Driver/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
npm run test -- DatabaseConnectionsPage.test 2>&1 | tail -5
```

Expected: import error.

- [ ] **Step 3: Create the component**

Create `src/components/Settings/DatabaseConnectionsPage.tsx`:

```typescript
import { useEffect, useState } from "react";
import {
  dbListConnections, dbAddConnection, dbUpdateConnection, dbRemoveConnection,
  dbTestConnection, DbConnectionInfo, DbConnectionInput, DbType,
  DB_TYPE_LABELS, DB_DEFAULT_PORTS,
} from "../../ipc/db";

type FormState = Omit<DbConnectionInput, "id"> & { id?: string };

const EMPTY_FORM: FormState = {
  name: "", db_type: "postgresql", host: "localhost",
  port: 5432, database: "", username: "", password: "",
};

export function DatabaseConnectionsPage() {
  const [connections, setConnections] = useState<DbConnectionInfo[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [testStatus, setTestStatus] = useState<"idle" | "testing" | "ok" | "error">("idle");
  const [testMsg, setTestMsg] = useState("");
  const [saving, setSaving] = useState(false);

  const load = () => dbListConnections().then(setConnections).catch(console.error);
  useEffect(() => { load(); }, []);

  const handleDbTypeChange = (db_type: DbType) => {
    setForm((f) => ({ ...f, db_type, port: DB_DEFAULT_PORTS[db_type] }));
  };

  const handleTest = async () => {
    setTestStatus("testing");
    setTestMsg("");
    try {
      await dbTestConnection({ ...form, password: form.password });
      setTestStatus("ok");
      setTestMsg("連線成功");
    } catch (e: unknown) {
      setTestStatus("error");
      setTestMsg(String(e));
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      if (form.id) {
        await dbUpdateConnection({ ...form, id: form.id });
      } else {
        await dbAddConnection(form as DbConnectionInput);
      }
      setShowForm(false);
      setForm(EMPTY_FORM);
      load();
    } catch (e) {
      console.error(e);
    } finally {
      setSaving(false);
    }
  };

  const handleEdit = (conn: DbConnectionInfo) => {
    setForm({
      id: conn.id, name: conn.name, db_type: conn.db_type,
      host: conn.host, port: conn.port, database: conn.database,
      username: conn.username, password: "",
    });
    setShowForm(true);
    setTestStatus("idle");
  };

  const handleDelete = async (id: string) => {
    if (!confirm("確定刪除此連線？")) return;
    await dbRemoveConnection(id);
    load();
  };

  return (
    <div style={{ maxWidth: 600 }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <h2 style={{ margin: 0, fontSize: 16, color: "#e6e6e6" }}>資料庫連線</h2>
        {!showForm && (
          <button
            onClick={() => { setForm(EMPTY_FORM); setShowForm(true); setTestStatus("idle"); }}
            style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 5, padding: "6px 14px", cursor: "pointer", fontSize: 13 }}
          >
            + 新增連線
          </button>
        )}
      </div>

      {/* Connection list */}
      {!showForm && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {connections.length === 0 && (
            <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>尚無資料庫連線。點擊「+ 新增連線」開始。</div>
          )}
          {connections.map((conn) => (
            <div key={conn.id} style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "10px 14px", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <div>
                <div style={{ color: "#e6e6e6", fontSize: 13, fontWeight: 500 }}>{conn.name}</div>
                <div style={{ color: "#888", fontSize: 11, marginTop: 2 }}>
                  {DB_TYPE_LABELS[conn.db_type]} · {conn.host}:{conn.port} / {conn.database}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                {conn.is_connected && <span style={{ color: "#34d399", fontSize: 11 }}>● 已連線</span>}
                <button onClick={() => handleEdit(conn)} style={btnStyle}>編輯</button>
                <button onClick={() => handleDelete(conn.id)} style={{ ...btnStyle, color: "#f87171", borderColor: "#f87171" }}>刪除</button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Add/Edit form */}
      {showForm && (
        <div style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 8, padding: 20 }}>
          <h3 style={{ margin: "0 0 16px", fontSize: 14, color: "#e6e6e6" }}>
            {form.id ? "編輯連線" : "新增連線"}
          </h3>
          <div style={{ display: "grid", gridTemplateColumns: "120px 1fr", gap: "10px 12px", alignItems: "center" }}>
            <label style={labelStyle}>名稱</label>
            <input
              placeholder="我的資料庫"
              value={form.name}
              onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
              style={inputStyle}
            />
            <label style={labelStyle}>類型</label>
            <select
              value={form.db_type}
              onChange={(e) => handleDbTypeChange(e.target.value as DbType)}
              style={inputStyle}
            >
              {(Object.entries(DB_TYPE_LABELS) as [DbType, string][]).map(([k, v]) => (
                <option key={k} value={k}>{v}</option>
              ))}
            </select>

            {form.db_type === "sqlite" ? (
              <>
                <label style={labelStyle}>檔案路徑</label>
                <input
                  placeholder="/path/to/database.db"
                  value={form.host}
                  onChange={(e) => setForm((f) => ({ ...f, host: e.target.value }))}
                  style={inputStyle}
                />
              </>
            ) : (
              <>
                <label style={labelStyle}>Host</label>
                <input
                  placeholder="localhost"
                  value={form.host}
                  onChange={(e) => setForm((f) => ({ ...f, host: e.target.value }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>Port</label>
                <input
                  type="number"
                  value={form.port}
                  onChange={(e) => setForm((f) => ({ ...f, port: Number(e.target.value) }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>Database</label>
                <input
                  value={form.database}
                  onChange={(e) => setForm((f) => ({ ...f, database: e.target.value }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>Username</label>
                <input
                  value={form.username}
                  onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>Password</label>
                <input
                  type="password"
                  placeholder={form.id ? "留空表示不變更" : ""}
                  value={form.password}
                  onChange={(e) => setForm((f) => ({ ...f, password: e.target.value }))}
                  style={inputStyle}
                />
              </>
            )}
          </div>

          {/* DB2 ODBC notice */}
          {form.db_type === "db2" && (
            <div style={{ background: "#2a1a00", border: "1px solid #f9a825", borderRadius: 5, padding: "10px 14px", marginTop: 12, fontSize: 12, color: "#f9a825" }}>
              ⚠️ DB2 需要 IBM DB2 ODBC Driver。Windows / macOS: 安裝 IBM Data Server Driver Package。
              <br />Host 欄位請填寫 DSN 名稱或完整 ODBC 連線字串。
            </div>
          )}

          <div style={{ display: "flex", gap: 8, marginTop: 16, alignItems: "center" }}>
            <button onClick={handleTest} disabled={testStatus === "testing"} style={btnStyle}>
              {testStatus === "testing" ? "測試中..." : "測試連線"}
            </button>
            {testStatus === "ok" && <span style={{ color: "#34d399", fontSize: 12 }}>✓ {testMsg}</span>}
            {testStatus === "error" && <span style={{ color: "#f87171", fontSize: 12 }}>✗ {testMsg}</span>}
            <div style={{ flex: 1 }} />
            <button onClick={() => setShowForm(false)} style={btnStyle}>取消</button>
            <button onClick={handleSave} disabled={saving} style={{ ...btnStyle, background: "#1e3a2e", borderColor: "#34d399", color: "#34d399" }}>
              {saving ? "儲存中..." : "儲存"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

const btnStyle: React.CSSProperties = {
  background: "transparent", border: "1px solid #3a3a3a", color: "#ccc",
  borderRadius: 4, padding: "4px 12px", cursor: "pointer", fontSize: 12,
};
const labelStyle: React.CSSProperties = { color: "#888", fontSize: 12 };
const inputStyle: React.CSSProperties = {
  background: "#0f0f0f", border: "1px solid #2a2a2a", color: "#e6e6e6",
  borderRadius: 4, padding: "6px 8px", fontSize: 13, width: "100%", boxSizing: "border-box",
};
```

- [ ] **Step 4: Run tests**

```bash
npm run test -- DatabaseConnectionsPage.test 2>&1 | tail -10
```

Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/DatabaseConnectionsPage.tsx src/components/Settings/DatabaseConnectionsPage.test.tsx
git commit -m "feat(settings): add DatabaseConnectionsPage"
```

---

### Task 18: Add DB nav to SettingsView

**Files:**
- Modify: `src/components/Settings/SettingsView.tsx`

- [ ] **Step 1: Update SettingsView**

In `src/components/Settings/SettingsView.tsx`:

1. Change the `SettingsTab` type:

```typescript
type SettingsTab = "providers" | "general" | "databases";
```

2. Add the import:

```typescript
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";
```

3. Add the nav button between "AI Providers" and "一般":

```typescript
<button
  className={`sidebar-item ${tab === "databases" ? "sidebar-item--active" : ""}`}
  onClick={() => setTab("databases")}
>
  🗄️ 資料庫連線
</button>
```

4. Add the content case:

```typescript
{tab === "databases" && <DatabaseConnectionsPage />}
```

- [ ] **Step 2: Verify app starts and settings page shows the new nav item**

```bash
npx tsc --noEmit 2>&1 | head -10
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/components/Settings/SettingsView.tsx
git commit -m "feat(settings): add database connections nav item"
```

---

### Task 19: ConnectionSelector + DatabaseView shell

**Files:**
- Create: `src/components/DatabaseView/ConnectionSelector.tsx`
- Modify: `src/components/DatabaseView/index.tsx` (replace stub)
- Create: `src/components/DatabaseView/index.css`
- Create: `src/components/DatabaseView/DatabaseView.test.tsx`

- [ ] **Step 1: Write tests**

Create `src/components/DatabaseView/DatabaseView.test.tsx`:

```typescript
import { render, screen, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { DatabaseView } from "./index";

vi.mock("../../ipc/db", () => ({
  dbListConnections: vi.fn().mockResolvedValue([
    { id: "c1", name: "My PG", db_type: "postgresql", host: "localhost",
      port: 5432, database: "mydb", username: "postgres", is_connected: false },
  ]),
  dbConnect: vi.fn().mockResolvedValue(undefined),
  dbListSchemas: vi.fn().mockResolvedValue(["public"]),
  dbListTables: vi.fn().mockResolvedValue([]),
  DB_TYPE_LABELS: { postgresql: "PostgreSQL" },
}));

describe("DatabaseView", () => {
  it("shows connection selector when no connection is set", async () => {
    render(
      <DatabaseView tabId="t1" isActive={true} onConnectionSelected={() => {}} />
    );
    await waitFor(() => expect(screen.getByText("選擇資料庫連線")).toBeInTheDocument());
    expect(screen.getByText("My PG")).toBeInTheDocument();
  });

  it("shows sub-tabs after connection is provided", async () => {
    render(
      <DatabaseView tabId="t1" isActive={true} dbConnectionId="c1" onConnectionSelected={() => {}} />
    );
    await waitFor(() => expect(screen.getByText("瀏覽")).toBeInTheDocument());
    expect(screen.getByText("AI Chat")).toBeInTheDocument();
    expect(screen.getByText("SQL Editor")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to confirm failure**

```bash
npm run test -- DatabaseView.test 2>&1 | tail -5
```

Expected: test fails (stub doesn't match).

- [ ] **Step 3: Create ConnectionSelector**

Create `src/components/DatabaseView/ConnectionSelector.tsx`:

```typescript
import { useEffect, useState } from "react";
import { dbListConnections, DbConnectionInfo, DB_TYPE_LABELS } from "../../ipc/db";
import { useNavigate } from "react-router-dom";

interface Props {
  onSelect: (connId: string) => void;
}

export function ConnectionSelector({ onSelect }: Props) {
  const navigate = useNavigate();
  const [connections, setConnections] = useState<DbConnectionInfo[]>([]);

  useEffect(() => {
    dbListConnections().then(setConnections).catch(console.error);
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", gap: 16 }}>
      <div style={{ color: "#888", fontSize: 14 }}>選擇資料庫連線</div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8, width: 280 }}>
        {connections.map((conn) => (
          <button
            key={conn.id}
            onClick={() => onSelect(conn.id)}
            style={{
              background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6,
              padding: "10px 14px", display: "flex", justifyContent: "space-between",
              alignItems: "center", cursor: "pointer", color: "#e6e6e6",
            }}
          >
            <span style={{ fontSize: 13 }}>{conn.name}</span>
            <span style={{ fontSize: 11, color: "#888" }}>{DB_TYPE_LABELS[conn.db_type]}</span>
          </button>
        ))}
        <button
          onClick={() => navigate("/settings")}
          style={{
            border: "1px dashed #333", background: "transparent", borderRadius: 6,
            padding: "10px 14px", color: "#555", fontSize: 12, cursor: "pointer",
          }}
        >
          ⚙ 新增 / 管理連線...
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Replace DatabaseView stub**

Replace the content of `src/components/DatabaseView/index.tsx`:

```typescript
import { useEffect, useState } from "react";
import { dbConnect, dbListSchemas } from "../../ipc/db";
import { ConnectionSelector } from "./ConnectionSelector";
import { DatabaseBrowser } from "./DatabaseBrowser";
import { DatabaseSqlEditor } from "./DatabaseSqlEditor";
import { DatabaseAiChat } from "./DatabaseAiChat";
import "./index.css";

export interface DatabaseViewProps {
  tabId: string;
  isActive: boolean;
  dbConnectionId?: string;
  onConnectionSelected: (connId: string) => void;
}

type SubTab = "browse" | "ai" | "sql";

export function DatabaseView({ tabId, isActive, dbConnectionId, onConnectionSelected }: DatabaseViewProps) {
  const [subTab, setSubTab] = useState<SubTab>("browse");
  const [schemas, setSchemas] = useState<string[]>([]);
  const [activeSchema, setActiveSchema] = useState<string>("");
  const [connectError, setConnectError] = useState<string | null>(null);

  useEffect(() => {
    if (!dbConnectionId) return;
    dbConnect(dbConnectionId)
      .then(() => dbListSchemas(dbConnectionId))
      .then((s) => {
        setSchemas(s);
        setActiveSchema(s[0] ?? "");
        setConnectError(null);
      })
      .catch((e: unknown) => setConnectError(String(e)));
  }, [dbConnectionId]);

  if (!dbConnectionId) {
    return <ConnectionSelector onSelect={onConnectionSelected} />;
  }

  if (connectError) {
    const isOdbc = connectError.includes("odbc_driver_not_found");
    return (
      <div style={{ padding: 24, color: "#f87171", fontSize: 13 }}>
        {isOdbc ? (
          <>
            <div style={{ marginBottom: 8 }}>⚠️ DB2 ODBC Driver 未安裝</div>
            <div style={{ color: "#888", fontSize: 12 }}>
              請安裝 IBM Data Server Driver Package，然後重新嘗試連線。
            </div>
          </>
        ) : (
          <>
            <div style={{ marginBottom: 8 }}>連線失敗</div>
            <div style={{ color: "#888", fontSize: 12 }}>{connectError}</div>
            <button
              onClick={() => { setConnectError(null); }}
              style={{ marginTop: 12, background: "#1a1a1a", border: "1px solid #3a3a3a", color: "#ccc", borderRadius: 4, padding: "6px 14px", cursor: "pointer" }}
            >
              重新連線
            </button>
          </>
        )}
      </div>
    );
  }

  return (
    <div className="db-view">
      <div className="db-view__subtabs">
        {([["browse", "瀏覽"], ["ai", "AI Chat"], ["sql", "SQL Editor"]] as [SubTab, string][]).map(([key, label]) => (
          <button
            key={key}
            className={`db-view__subtab ${subTab === key ? "db-view__subtab--active" : ""}`}
            onClick={() => setSubTab(key)}
          >
            {label}
          </button>
        ))}
        {schemas.length > 1 && (
          <select
            value={activeSchema}
            onChange={(e) => setActiveSchema(e.target.value)}
            style={{ marginLeft: "auto", background: "#1a1a1a", border: "1px solid #2a2a2a", color: "#888", fontSize: 11, borderRadius: 3, padding: "2px 6px" }}
          >
            {schemas.map((s) => <option key={s} value={s}>{s}</option>)}
          </select>
        )}
      </div>
      <div className="db-view__content">
        {subTab === "browse" && dbConnectionId && (
          <DatabaseBrowser connectionId={dbConnectionId} schema={activeSchema} />
        )}
        {subTab === "ai" && dbConnectionId && (
          <DatabaseAiChat connectionId={dbConnectionId} schema={activeSchema} />
        )}
        {subTab === "sql" && dbConnectionId && (
          <DatabaseSqlEditor connectionId={dbConnectionId} />
        )}
      </div>
    </div>
  );
}
```

Create `src/components/DatabaseView/index.css`:

```css
.db-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: #0c0c0c;
}

.db-view__subtabs {
  display: flex;
  align-items: center;
  background: #111;
  border-bottom: 1px solid #1e1e1e;
  padding: 0 8px;
  flex-shrink: 0;
}

.db-view__subtab {
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: #888;
  font-size: 12px;
  padding: 8px 14px;
  cursor: pointer;
  transition: color 0.1s;
}

.db-view__subtab:hover { color: #ccc; }

.db-view__subtab--active {
  color: #34d399;
  border-bottom-color: #34d399;
}

.db-view__content {
  flex: 1;
  overflow: hidden;
  display: flex;
}
```

- [ ] **Step 5: Create stubs for sub-components** (so it compiles before implementing them)

Create `src/components/DatabaseView/DatabaseBrowser.tsx`:

```typescript
export function DatabaseBrowser({ connectionId, schema }: { connectionId: string; schema: string }) {
  return <div style={{ padding: 16, color: "#888", fontSize: 13 }}>Browser: {connectionId} / {schema}</div>;
}
```

Create `src/components/DatabaseView/DatabaseSqlEditor.tsx`:

```typescript
export function DatabaseSqlEditor({ connectionId }: { connectionId: string }) {
  return <div style={{ padding: 16, color: "#888", fontSize: 13 }}>SQL Editor: {connectionId}</div>;
}
```

Create `src/components/DatabaseView/DatabaseAiChat.tsx`:

```typescript
export function DatabaseAiChat({ connectionId, schema }: { connectionId: string; schema: string }) {
  return <div style={{ padding: 16, color: "#888", fontSize: 13 }}>AI Chat: {connectionId} / {schema}</div>;
}
```

- [ ] **Step 6: Run tests**

```bash
npm run test -- DatabaseView.test 2>&1 | tail -10
```

Expected: both tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/components/DatabaseView/
git commit -m "feat(db): add DatabaseView shell with ConnectionSelector and sub-tab routing"
```

---

### Task 20: DatabaseBrowser (Browse sub-tab)

**Files:**
- Modify: `src/components/DatabaseView/DatabaseBrowser.tsx`

- [ ] **Step 1: Replace stub with full implementation**

```typescript
import { useEffect, useState } from "react";
import { dbListTables, dbGetTableSchema, dbExecuteQuery, TableInfo, ColumnInfo, QueryResult } from "../../ipc/db";

interface Props {
  connectionId: string;
  schema: string;
}

type ViewMode = "data" | "structure";

export function DatabaseBrowser({ connectionId, schema }: Props) {
  const [tables, setTables] = useState<TableInfo[]>([]);
  const [selectedTable, setSelectedTable] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("data");
  const [queryResult, setQueryResult] = useState<QueryResult | null>(null);
  const [columns, setColumns] = useState<ColumnInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(0);
  const PAGE_SIZE = 100;

  useEffect(() => {
    if (!schema) return;
    dbListTables(connectionId, schema).then(setTables).catch(console.error);
    setSelectedTable(null);
    setQueryResult(null);
  }, [connectionId, schema]);

  const selectTable = async (name: string) => {
    setSelectedTable(name);
    setPage(0);
    setLoading(true);
    try {
      if (viewMode === "data") {
        const result = await dbExecuteQuery(connectionId, `SELECT * FROM "${schema}"."${name}" LIMIT ${PAGE_SIZE} OFFSET 0`);
        setQueryResult(result);
      } else {
        const cols = await dbGetTableSchema(connectionId, schema, name);
        setColumns(cols);
      }
    } finally {
      setLoading(false);
    }
  };

  const switchMode = async (mode: ViewMode) => {
    setViewMode(mode);
    if (!selectedTable) return;
    setLoading(true);
    try {
      if (mode === "data") {
        const result = await dbExecuteQuery(connectionId, `SELECT * FROM "${schema}"."${selectedTable}" LIMIT ${PAGE_SIZE} OFFSET ${page * PAGE_SIZE}`);
        setQueryResult(result);
      } else {
        const cols = await dbGetTableSchema(connectionId, schema, selectedTable);
        setColumns(cols);
      }
    } finally {
      setLoading(false);
    }
  };

  const loadPage = async (newPage: number) => {
    if (!selectedTable) return;
    setPage(newPage);
    setLoading(true);
    try {
      const result = await dbExecuteQuery(connectionId, `SELECT * FROM "${schema}"."${selectedTable}" LIMIT ${PAGE_SIZE} OFFSET ${newPage * PAGE_SIZE}`);
      setQueryResult(result);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ display: "flex", height: "100%", overflow: "hidden" }}>
      {/* Object tree */}
      <div style={{ width: 200, flexShrink: 0, background: "#111", borderRight: "1px solid #1e1e1e", overflowY: "auto", padding: "8px 0" }}>
        <div style={{ color: "#666", fontSize: 10, letterSpacing: 1, padding: "4px 12px", marginBottom: 4 }}>TABLES</div>
        {tables.map((t) => (
          <div
            key={t.name}
            onClick={() => selectTable(t.name)}
            style={{
              padding: "5px 12px", cursor: "pointer", fontSize: 12,
              color: selectedTable === t.name ? "#34d399" : "#ccc",
              background: selectedTable === t.name ? "#1a2a1a" : "transparent",
              display: "flex", alignItems: "center", gap: 6,
            }}
          >
            <span style={{ fontSize: 10, color: "#555" }}>{t.table_type === "view" ? "👁" : "▤"}</span>
            {t.name}
          </div>
        ))}
      </div>

      {/* Content area */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        {selectedTable && (
          <div style={{ background: "#111", borderBottom: "1px solid #1e1e1e", padding: "6px 12px", display: "flex", gap: 0, alignItems: "center" }}>
            <button onClick={() => switchMode("data")} style={{ ...modeBtn, ...(viewMode === "data" ? modeBtnActive : {}) }}>資料</button>
            <button onClick={() => switchMode("structure")} style={{ ...modeBtn, ...(viewMode === "structure" ? modeBtnActive : {}) }}>結構</button>
            <span style={{ color: "#555", fontSize: 11, marginLeft: "auto" }}>{selectedTable}</span>
          </div>
        )}

        {loading && (
          <div style={{ padding: 16, color: "#888", fontSize: 12 }}>載入中...</div>
        )}

        {!loading && !selectedTable && (
          <div style={{ padding: 24, color: "#555", fontSize: 13 }}>← 從左側選擇資料表</div>
        )}

        {!loading && selectedTable && viewMode === "data" && queryResult && (
          <DataGrid result={queryResult} page={page} pageSize={PAGE_SIZE} onPageChange={loadPage} />
        )}

        {!loading && selectedTable && viewMode === "structure" && (
          <StructureView columns={columns} />
        )}
      </div>
    </div>
  );
}

function DataGrid({ result, page, pageSize, onPageChange }: { result: QueryResult; page: number; pageSize: number; onPageChange: (p: number) => void }) {
  if (result.error) {
    return <div style={{ padding: 16, color: "#f87171", fontSize: 12 }}>錯誤：{result.error}</div>;
  }
  return (
    <div style={{ flex: 1, overflow: "auto" }}>
      <table style={{ borderCollapse: "collapse", width: "100%", fontSize: 12, fontFamily: "monospace" }}>
        <thead>
          <tr style={{ background: "#151515", position: "sticky", top: 0 }}>
            {result.columns.map((col) => (
              <th key={col} style={{ padding: "6px 10px", textAlign: "left", color: "#888", borderBottom: "1px solid #222", whiteSpace: "nowrap", fontWeight: "normal" }}>
                {col}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {result.rows.map((row, i) => (
            <tr key={i} style={{ borderBottom: "1px solid #111", background: i % 2 === 0 ? "transparent" : "#0f0f0f" }}>
              {row.map((cell, j) => (
                <td key={j} style={{ padding: "4px 10px", color: cell === null ? "#444" : typeof cell === "number" ? "#f9a825" : "#ccc", whiteSpace: "nowrap", maxWidth: 300, overflow: "hidden", textOverflow: "ellipsis" }}>
                  {cell === null ? "NULL" : String(cell)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <div style={{ padding: "8px 12px", display: "flex", gap: 8, alignItems: "center", borderTop: "1px solid #1e1e1e", fontSize: 11, color: "#888" }}>
        <span>{result.rows.length} 列 · {result.execution_time_ms}ms</span>
        <div style={{ flex: 1 }} />
        {page > 0 && <button onClick={() => onPageChange(page - 1)} style={pageBtn}>← 上一頁</button>}
        <span>第 {page + 1} 頁</span>
        {result.rows.length === 100 && <button onClick={() => onPageChange(page + 1)} style={pageBtn}>下一頁 →</button>}
      </div>
    </div>
  );
}

function StructureView({ columns }: { columns: ColumnInfo[] }) {
  return (
    <div style={{ flex: 1, overflow: "auto" }}>
      <table style={{ borderCollapse: "collapse", width: "100%", fontSize: 12 }}>
        <thead>
          <tr style={{ background: "#151515" }}>
            {["欄位名稱", "類型", "可為 NULL", "預設值"].map((h) => (
              <th key={h} style={{ padding: "6px 10px", textAlign: "left", color: "#888", borderBottom: "1px solid #222", fontWeight: "normal" }}>{h}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {columns.map((col, i) => (
            <tr key={col.name} style={{ borderBottom: "1px solid #111", background: i % 2 === 0 ? "transparent" : "#0f0f0f" }}>
              <td style={{ padding: "4px 10px", color: "#e6e6e6" }}>{col.name}</td>
              <td style={{ padding: "4px 10px", color: "#60a5fa", fontFamily: "monospace" }}>{col.data_type}</td>
              <td style={{ padding: "4px 10px", color: col.nullable ? "#34d399" : "#f87171" }}>{col.nullable ? "YES" : "NO"}</td>
              <td style={{ padding: "4px 10px", color: "#888" }}>{col.default ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const modeBtn: React.CSSProperties = { background: "transparent", border: "none", color: "#888", fontSize: 12, padding: "4px 12px", cursor: "pointer" };
const modeBtnActive: React.CSSProperties = { color: "#34d399", borderBottom: "2px solid #34d399" };
const pageBtn: React.CSSProperties = { background: "#1a1a1a", border: "1px solid #2a2a2a", color: "#888", fontSize: 11, padding: "2px 10px", borderRadius: 3, cursor: "pointer" };
```

- [ ] **Step 2: Compile check**

```bash
npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/components/DatabaseView/DatabaseBrowser.tsx
git commit -m "feat(db): implement DatabaseBrowser with data grid and structure view"
```

---

### Task 21: DatabaseSqlEditor

**Files:**
- Modify: `src/components/DatabaseView/DatabaseSqlEditor.tsx`

- [ ] **Step 1: Replace stub with implementation**

```typescript
import { useState, useRef } from "react";
import { dbExecuteQuery, QueryResult } from "../../ipc/db";

interface Props {
  connectionId: string;
}

export function DatabaseSqlEditor({ connectionId }: Props) {
  const [sql, setSql] = useState("SELECT 1;");
  const [result, setResult] = useState<QueryResult | null>(null);
  const [running, setRunning] = useState(false);

  const run = async () => {
    if (!sql.trim()) return;
    setRunning(true);
    try {
      const r = await dbExecuteQuery(connectionId, sql);
      setResult(r);
    } catch (e: unknown) {
      setResult({ columns: [], rows: [], affected_rows: null, execution_time_ms: 0, error: String(e) });
    } finally {
      setRunning(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Ctrl+Enter or Cmd+Enter to run
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      run();
    }
    // Tab → insert 2 spaces
    if (e.key === "Tab") {
      e.preventDefault();
      const el = e.currentTarget;
      const start = el.selectionStart;
      const end = el.selectionEnd;
      const newVal = sql.slice(0, start) + "  " + sql.slice(end);
      setSql(newVal);
      requestAnimationFrame(() => {
        el.selectionStart = el.selectionEnd = start + 2;
      });
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden" }}>
      {/* Editor area */}
      <div style={{ flex: "0 0 40%", display: "flex", flexDirection: "column", borderBottom: "1px solid #1e1e1e" }}>
        <div style={{ background: "#111", padding: "6px 12px", display: "flex", alignItems: "center", gap: 8, borderBottom: "1px solid #1e1e1e" }}>
          <span style={{ color: "#888", fontSize: 11 }}>SQL Editor</span>
          <div style={{ flex: 1 }} />
          <button
            onClick={run}
            disabled={running}
            style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 4, padding: "4px 14px", cursor: "pointer", fontSize: 12 }}
          >
            {running ? "執行中..." : "▶ 執行 (Ctrl+Enter)"}
          </button>
        </div>
        <textarea
          value={sql}
          onChange={(e) => setSql(e.target.value)}
          onKeyDown={handleKeyDown}
          spellCheck={false}
          style={{
            flex: 1, background: "#0c0c0c", color: "#e6e6e6", border: "none",
            resize: "none", padding: "12px 14px", fontFamily: '"Cascadia Mono", Consolas, monospace',
            fontSize: 13, lineHeight: 1.6, outline: "none",
          }}
        />
      </div>

      {/* Results area */}
      <div style={{ flex: 1, overflow: "auto" }}>
        {!result && !running && (
          <div style={{ padding: 20, color: "#555", fontSize: 12 }}>執行 SQL 後結果將顯示於此</div>
        )}
        {running && (
          <div style={{ padding: 20, color: "#888", fontSize: 12 }}>執行中...</div>
        )}
        {result && !running && (
          <>
            {result.error && (
              <div style={{ padding: "12px 16px", color: "#f87171", fontSize: 12, fontFamily: "monospace" }}>
                ✗ {result.error}
              </div>
            )}
            {!result.error && result.affected_rows !== null && (
              <div style={{ padding: "12px 16px", color: "#34d399", fontSize: 12 }}>
                ✓ {result.affected_rows} 列受影響 ({result.execution_time_ms}ms)
              </div>
            )}
            {!result.error && result.columns.length > 0 && (
              <>
                <div style={{ padding: "6px 12px", color: "#888", fontSize: 11, borderBottom: "1px solid #1e1e1e", background: "#111" }}>
                  {result.rows.length} 列 · {result.execution_time_ms}ms
                </div>
                <table style={{ borderCollapse: "collapse", width: "100%", fontSize: 12, fontFamily: "monospace" }}>
                  <thead>
                    <tr style={{ background: "#151515" }}>
                      {result.columns.map((col) => (
                        <th key={col} style={{ padding: "6px 10px", textAlign: "left", color: "#888", borderBottom: "1px solid #222", fontWeight: "normal", whiteSpace: "nowrap" }}>
                          {col}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {result.rows.map((row, i) => (
                      <tr key={i} style={{ borderBottom: "1px solid #111", background: i % 2 === 0 ? "transparent" : "#0f0f0f" }}>
                        {row.map((cell, j) => (
                          <td key={j} style={{ padding: "4px 10px", color: cell === null ? "#444" : typeof cell === "number" ? "#f9a825" : "#ccc", whiteSpace: "nowrap" }}>
                            {cell === null ? "NULL" : String(cell)}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Compile check**

```bash
npx tsc --noEmit 2>&1 | head -10
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/components/DatabaseView/DatabaseSqlEditor.tsx
git commit -m "feat(db): implement SQL Editor sub-tab"
```

---

### Task 22: DatabaseAiChat

**Files:**
- Modify: `src/components/DatabaseView/DatabaseAiChat.tsx`

- [ ] **Step 1: Replace stub with implementation**

```typescript
import { useState, useEffect, useRef } from "react";
import { dbListTables, dbExecuteQuery, TableInfo, QueryResult } from "../../ipc/db";
import { aiChat } from "../../ipc/ai";

interface Props {
  connectionId: string;
  schema: string;
}

interface Message {
  role: "user" | "assistant";
  text: string;
  sql?: string;
  result?: QueryResult;
  executing?: boolean;
}

/** Extract the first ```sql ... ``` block from AI text. */
function extractSql(text: string): string | null {
  const m = text.match(/```sql\s*([\s\S]*?)```/i);
  return m ? m[1].trim() : null;
}

export function DatabaseAiChat({ connectionId, schema }: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [tables, setTables] = useState<TableInfo[]>([]);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (schema) {
      dbListTables(connectionId, schema).then(setTables).catch(console.error);
    }
  }, [connectionId, schema]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const buildSystemPrompt = () => {
    const tableList = tables.map((t) => t.name).join(", ");
    return `你是一個資料庫助手，正在連接資料庫 schema「${schema}」。
可用的資料表：${tableList || "（載入中）"}。
請用繁體中文回答，並以 \`\`\`sql ... \`\`\` 格式提供 SQL 語句。`;
  };

  const executeMessageSql = async (msgIndex: number, sql: string, retryCount = 0): Promise<void> => {
    setMessages((prev) =>
      prev.map((m, i) => (i === msgIndex ? { ...m, executing: true } : m))
    );
    const result = await dbExecuteQuery(connectionId, sql);

    // Auto-retry on error (max 2 retries) by asking AI to fix
    if (result.error && retryCount < 2) {
      const retryPrompt = `SQL 執行錯誤：${result.error}\n原始 SQL：${sql}\n請修正 SQL。`;
      setSending(true);
      try {
        const fixResp = await aiChat(retryPrompt, buildSystemPrompt(), []);
        const fixedSql = extractSql(fixResp);
        if (fixedSql) {
          await executeMessageSql(msgIndex, fixedSql, retryCount + 1);
          return;
        }
      } finally {
        setSending(false);
      }
    }

    setMessages((prev) =>
      prev.map((m, i) =>
        i === msgIndex ? { ...m, sql, result, executing: false } : m
      )
    );
  };

  const send = async () => {
    if (!input.trim() || sending) return;
    const userMsg = input.trim();
    setInput("");
    setSending(true);

    const newMessages: Message[] = [...messages, { role: "user", text: userMsg }];
    setMessages(newMessages);

    try {
      const history = newMessages.map((m) => ({ role: m.role, content: m.text }));
      const reply = await aiChat(userMsg, buildSystemPrompt(), history.slice(0, -1));
      const sql = extractSql(reply);
      const msgIndex = newMessages.length;
      setMessages((prev) => [...prev, { role: "assistant", text: reply, sql: sql ?? undefined }]);

      if (sql) {
        await executeMessageSql(msgIndex, sql);
      }
    } catch (e: unknown) {
      setMessages((prev) => [...prev, { role: "assistant", text: `錯誤：${String(e)}` }]);
    } finally {
      setSending(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ flex: 1, overflowY: "auto", padding: "12px 16px", display: "flex", flexDirection: "column", gap: 12 }}>
        {messages.length === 0 && (
          <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>
            用自然語言描述你想查詢的資料，例如：「查詢最近 10 筆訂單」
          </div>
        )}
        {messages.map((msg, i) => (
          <div key={i} style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: msg.role === "user" ? "flex-end" : "flex-start" }}>
            <div style={{
              background: msg.role === "user" ? "#1a2a3e" : "#1a1a1a",
              border: `1px solid ${msg.role === "user" ? "#60a5fa33" : "#2a2a2a"}`,
              borderRadius: 8, padding: "8px 12px", maxWidth: "80%", fontSize: 13, color: "#e6e6e6",
            }}>
              {msg.text}
            </div>
            {msg.sql && (
              <div style={{ background: "#0f0f0f", border: "1px solid #2a2a2a", borderRadius: 6, padding: "8px 12px", fontFamily: "monospace", fontSize: 12, color: "#34d399", maxWidth: "90%" }}>
                {msg.sql}
              </div>
            )}
            {msg.executing && (
              <div style={{ color: "#888", fontSize: 11 }}>執行中...</div>
            )}
            {msg.result && !msg.executing && (
              <ResultInline result={msg.result} />
            )}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>

      <div style={{ borderTop: "1px solid #1e1e1e", padding: "10px 12px", display: "flex", gap: 8, alignItems: "flex-end", background: "#111" }}>
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); }
          }}
          placeholder="用自然語言描述查詢... (Enter 送出)"
          rows={2}
          style={{
            flex: 1, background: "#0c0c0c", border: "1px solid #2a2a2a", color: "#e6e6e6",
            borderRadius: 6, padding: "8px 10px", fontSize: 13, resize: "none", outline: "none",
            fontFamily: "inherit",
          }}
        />
        <button
          onClick={send}
          disabled={sending || !input.trim()}
          style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 6, padding: "8px 14px", cursor: "pointer", fontSize: 12 }}
        >
          {sending ? "..." : "✨ 送出"}
        </button>
      </div>
    </div>
  );
}

function ResultInline({ result }: { result: QueryResult }) {
  if (result.error) {
    return <div style={{ color: "#f87171", fontSize: 11, padding: "4px 12px" }}>✗ {result.error}</div>;
  }
  if (result.affected_rows !== null) {
    return <div style={{ color: "#34d399", fontSize: 11 }}>✓ {result.affected_rows} 列受影響 ({result.execution_time_ms}ms)</div>;
  }
  if (result.columns.length === 0) return null;
  return (
    <div style={{ overflowX: "auto", maxWidth: "90%" }}>
      <div style={{ color: "#888", fontSize: 11, marginBottom: 4 }}>{result.rows.length} 列 · {result.execution_time_ms}ms</div>
      <table style={{ borderCollapse: "collapse", fontSize: 11, fontFamily: "monospace" }}>
        <thead>
          <tr>{result.columns.map((c) => <th key={c} style={{ padding: "3px 8px", color: "#888", borderBottom: "1px solid #222", textAlign: "left", fontWeight: "normal" }}>{c}</th>)}</tr>
        </thead>
        <tbody>
          {result.rows.slice(0, 20).map((row, i) => (
            <tr key={i} style={{ background: i % 2 === 0 ? "transparent" : "#0f0f0f" }}>
              {row.map((cell, j) => (
                <td key={j} style={{ padding: "3px 8px", color: cell === null ? "#444" : "#ccc", borderBottom: "1px solid #111" }}>
                  {cell === null ? "NULL" : String(cell)}
                </td>
              ))}
            </tr>
          ))}
          {result.rows.length > 20 && (
            <tr><td colSpan={result.columns.length} style={{ padding: "4px 8px", color: "#555", fontSize: 11 }}>... 還有 {result.rows.length - 20} 列</td></tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
```

- [ ] **Step 2: Add `aiChat` export to ipc/ai.ts**

Check `src/ipc/ai.ts` for an existing chat function. If there's already `invokeAiChat` or similar, import and re-export under `aiChat`. If not, add:

```typescript
export function aiChat(
  message: string,
  systemPrompt: string,
  history: { role: string; content: string }[]
): Promise<string> {
  return invoke("ai_chat", { message, systemPrompt, history });
}
```

- [ ] **Step 3: Compile check**

```bash
npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 4: Run all frontend tests**

```bash
npm run test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/DatabaseView/DatabaseAiChat.tsx src/ipc/ai.ts
git commit -m "feat(db): implement AI Chat sub-tab with SQL extraction and auto-retry"
```

---

### Task 23: Final integration check

- [ ] **Step 1: Run all tests**

```bash
npm run test 2>&1 | tail -20
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 2: Build Tauri app**

```bash
npm run tauri:dev 2>&1 &
sleep 15 && echo "started"
```

Manual verification checklist:
- [ ] Clicking `+` shows Terminal / 資料庫 dropdown
- [ ] Selecting Terminal creates a working terminal tab
- [ ] Selecting 資料庫 creates a DB tab showing connection selector
- [ ] Settings → 🗄️ 資料庫連線 page shows form with all 5 DB types
- [ ] Selecting DB2 shows ODBC notice
- [ ] Sub-tabs (瀏覽 / AI Chat / SQL Editor) switch correctly

- [ ] **Step 3: Final commit**

```bash
git add -p  # review any remaining changes
git commit -m "feat(db): complete database tab feature — frontend integration"
```
