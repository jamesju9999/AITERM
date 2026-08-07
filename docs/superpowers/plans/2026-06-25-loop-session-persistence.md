# Loop Session Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow Loop Studio sessions to be paused, persisted to SQLite, and resumed after app restart.

**Architecture:** Follow the exact same pattern as `DesignDb` — a dedicated SQLite file in the app data dir, accessed via sqlx pool stored as Tauri state. Auto-save after each iteration (at clean boundaries). The session list is shown at the top of Loop Studio; clicking a past session loads its config, history, and trace to resume.

**Tech Stack:** Rust / sqlx / SQLite, Tauri 2 IPC, React 19, TypeScript

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Create | `src-tauri/src/db/loop_sessions.rs` | SQLite pool, schema init, CRUD functions |
| Modify | `src-tauri/src/db/mod.rs` | expose `loop_sessions` module |
| Modify | `src-tauri/src/lib.rs` | init `LoopSessionDb`, register new commands |
| Create | `src-tauri/src/commands/loop_session.rs` | 4 Tauri commands |
| Modify | `src-tauri/src/commands/mod.rs` | expose new command module |
| Create | `src/ipc/loopSession.ts` | TypeScript invoke wrappers |
| Modify | `src/hooks/useOrchestratorLoop.ts` | auto-save hook, resume from snapshot |
| Create | `src/components/LoopStudio/SessionPicker.tsx` | session list dropdown at top |
| Modify | `src/components/LoopStudio/index.tsx` | wire SessionPicker + resume flow |
| Modify | `src/components/LoopStudio/styles.css` | SessionPicker styles |

---

## Task 1: Rust DB layer — `loop_sessions.rs`

**Files:**
- Create: `src-tauri/src/db/loop_sessions.rs`

- [ ] Create the file with pool struct, schema init, and CRUD:

```rust
// src-tauri/src/db/loop_sessions.rs
use sqlx::{SqlitePool, FromRow};
use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use std::fs;

pub struct LoopSessionDb {
    pub pool: SqlitePool,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct LoopSessionRow {
    pub id: String,
    pub goal: String,
    pub status: String,          // "running" | "paused" | "completed" | "failed"
    pub iteration: i64,
    pub config_json: String,     // full LoopConfig as JSON
    pub history_json: String,    // orchestratorHistory as JSON
    pub shared_context: String,
    pub trace_json: String,      // trace entries as JSON
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct LoopSessionSummary {
    pub id: String,
    pub goal: String,
    pub status: String,
    pub iteration: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl LoopSessionDb {
    pub async fn new() -> Self {
        let app_data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AITERM");
        fs::create_dir_all(&app_data_dir).ok();
        let db_path = app_data_dir.join("loop_sessions.db");
        let url = format!("sqlite:{}", db_path.to_string_lossy());
        let pool = SqlitePool::connect(&url).await.unwrap_or_else(|_| {
            SqlitePool::connect_lazy("sqlite::memory:").unwrap()
        });
        let db = Self { pool };
        db.init().await.ok();
        db
    }

    async fn init(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS loop_sessions (
                id TEXT PRIMARY KEY NOT NULL,
                goal TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running',
                iteration INTEGER NOT NULL DEFAULT 0,
                config_json TEXT NOT NULL DEFAULT '{}',
                history_json TEXT NOT NULL DEFAULT '[]',
                shared_context TEXT NOT NULL DEFAULT '',
                trace_json TEXT NOT NULL DEFAULT '[]',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;
        Ok(())
    }
}

pub async fn upsert_loop_session(
    pool: &SqlitePool,
    id: &str,
    goal: &str,
    status: &str,
    iteration: i64,
    config_json: &str,
    history_json: &str,
    shared_context: &str,
    trace_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO loop_sessions (id, goal, status, iteration, config_json, history_json, shared_context, trace_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           goal = excluded.goal,
           status = excluded.status,
           iteration = excluded.iteration,
           config_json = excluded.config_json,
           history_json = excluded.history_json,
           shared_context = excluded.shared_context,
           trace_json = excluded.trace_json,
           updated_at = CURRENT_TIMESTAMP"
    )
    .bind(id).bind(goal).bind(status).bind(iteration)
    .bind(config_json).bind(history_json).bind(shared_context).bind(trace_json)
    .execute(pool).await?;
    Ok(())
}

pub async fn list_loop_sessions(pool: &SqlitePool) -> Result<Vec<LoopSessionSummary>, sqlx::Error> {
    let rows = sqlx::query_as::<_, LoopSessionSummary>(
        "SELECT id, goal, status, iteration, created_at, updated_at
         FROM loop_sessions ORDER BY updated_at DESC"
    ).fetch_all(pool).await?;
    Ok(rows)
}

pub async fn load_loop_session(pool: &SqlitePool, id: &str) -> Result<LoopSessionRow, sqlx::Error> {
    sqlx::query_as::<_, LoopSessionRow>(
        "SELECT id, goal, status, iteration, config_json, history_json, shared_context, trace_json, created_at, updated_at
         FROM loop_sessions WHERE id = ?"
    ).bind(id).fetch_one(pool).await
}

pub async fn delete_loop_session(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM loop_sessions WHERE id = ?")
        .bind(id).execute(pool).await?;
    Ok(())
}
```

- [ ] Verify it compiles:
```bash
cd src-tauri && cargo check --no-default-features 2>&1 | grep "^error"
```
Expected: no errors.

---

## Task 2: Expose module and register state in `lib.rs`

**Files:**
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] In `src-tauri/src/db/mod.rs`, add:
```rust
pub mod loop_sessions;
```

- [ ] In `src-tauri/src/lib.rs`, add the import near the existing DesignDb import:
```rust
use db::{design::DesignDb, loop_sessions::LoopSessionDb, manager::DbManager, Db2SidecarState};
```

- [ ] In `lib.rs` `run()`, init the DB alongside `design_db`:
```rust
let loop_session_db = tauri::async_runtime::block_on(async { LoopSessionDb::new().await });
```

- [ ] In the `.manage()` chain, add:
```rust
.manage(loop_session_db)
```

- [ ] Verify:
```bash
cargo check --no-default-features 2>&1 | grep "^error"
```

---

## Task 3: Tauri commands — `loop_session.rs`

**Files:**
- Create: `src-tauri/src/commands/loop_session.rs`

- [ ] Create the file with 4 commands:

```rust
// src-tauri/src/commands/loop_session.rs
use tauri::State;
use serde::{Serialize, Deserialize};
use crate::db::loop_sessions::{
    LoopSessionDb, LoopSessionRow, LoopSessionSummary,
    upsert_loop_session, list_loop_sessions, load_loop_session, delete_loop_session,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct LoopSessionSaveArgs {
    pub id: String,
    pub goal: String,
    pub status: String,
    pub iteration: i64,
    pub config_json: String,
    pub history_json: String,
    pub shared_context: String,
    pub trace_json: String,
}

#[tauri::command]
pub async fn loop_session_save(
    args: LoopSessionSaveArgs,
    db: State<'_, LoopSessionDb>,
) -> Result<(), String> {
    upsert_loop_session(
        &db.pool,
        &args.id, &args.goal, &args.status,
        args.iteration,
        &args.config_json, &args.history_json,
        &args.shared_context, &args.trace_json,
    ).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn loop_session_list(
    db: State<'_, LoopSessionDb>,
) -> Result<Vec<LoopSessionSummary>, String> {
    list_loop_sessions(&db.pool).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn loop_session_load(
    id: String,
    db: State<'_, LoopSessionDb>,
) -> Result<LoopSessionRow, String> {
    load_loop_session(&db.pool, &id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn loop_session_delete(
    id: String,
    db: State<'_, LoopSessionDb>,
) -> Result<(), String> {
    delete_loop_session(&db.pool, &id).await.map_err(|e| e.to_string())
}
```

- [ ] In `src-tauri/src/commands/mod.rs`, add:
```rust
pub mod loop_session;
```

- [ ] In `src-tauri/src/lib.rs`, add the use import:
```rust
use commands::loop_session::{
    loop_session_save, loop_session_list, loop_session_load, loop_session_delete,
};
```

- [ ] Register commands in `invoke_handler`:
```rust
// Loop Sessions
loop_session_save,
loop_session_list,
loop_session_load,
loop_session_delete,
```

- [ ] Verify:
```bash
cargo check --no-default-features 2>&1 | grep "^error"
```

---

## Task 4: Full Rust build

- [ ] Build to confirm everything links:
```bash
touch src/lib.rs && cargo build --no-default-features 2>&1 | grep -E "^error|Compiling|Finished"
```
Expected: `Finished dev profile`.

---

## Task 5: TypeScript IPC wrappers

**Files:**
- Create: `src/ipc/loopSession.ts`

- [ ] Create the file:

```typescript
// src/ipc/loopSession.ts
import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage } from "./ai";
import type { TraceEntry, LoopConfig } from "../hooks/useOrchestratorLoop";

export interface LoopSessionSummary {
  id: string;
  goal: string;
  status: "running" | "paused" | "completed" | "failed";
  iteration: number;
  created_at: string;
  updated_at: string;
}

export interface LoopSessionData extends LoopSessionSummary {
  config_json: string;
  history_json: string;
  shared_context: string;
  trace_json: string;
}

export interface LoopSessionSnapshot {
  config: LoopConfig;
  orchestratorHistory: ChatMessage[];
  sharedContext: string;
  trace: TraceEntry[];
  iteration: number;
}

export function loopSessionSave(
  id: string,
  goal: string,
  status: LoopSessionSummary["status"],
  iteration: number,
  snapshot: Omit<LoopSessionSnapshot, "iteration">,
): Promise<void> {
  return invoke("loop_session_save", {
    args: {
      id,
      goal,
      status,
      iteration,
      config_json: JSON.stringify(snapshot.config),
      history_json: JSON.stringify(snapshot.orchestratorHistory),
      shared_context: snapshot.sharedContext,
      trace_json: JSON.stringify(snapshot.trace),
    },
  });
}

export function loopSessionList(): Promise<LoopSessionSummary[]> {
  return invoke("loop_session_list");
}

export function loopSessionLoad(id: string): Promise<LoopSessionData> {
  return invoke("loop_session_load", { id });
}

export function loopSessionDelete(id: string): Promise<void> {
  return invoke("loop_session_delete", { id });
}

export function parseLoopSessionData(data: LoopSessionData): LoopSessionSnapshot {
  return {
    config: JSON.parse(data.config_json) as LoopConfig,
    orchestratorHistory: JSON.parse(data.history_json) as ChatMessage[],
    sharedContext: data.shared_context,
    trace: JSON.parse(data.trace_json) as TraceEntry[],
    iteration: data.iteration,
  };
}
```

- [ ] Type-check:
```bash
npx tsc --noEmit 2>&1 | grep "^src/ipc/loopSession"
```
Expected: no output.

---

## Task 6: Auto-save in `useOrchestratorLoop.ts`

**Files:**
- Modify: `src/hooks/useOrchestratorLoop.ts`

The hook needs to:
1. Accept an optional `loopSessionId` in `LoopConfig` (or generate one at start)
2. Auto-save after each iteration's Verifier result
3. Save with status `"paused"` on user stop, `"completed"` on done, `"failed"` on error

- [ ] Add `loopSessionId?: string` to `LoopConfig`:
```typescript
export interface LoopConfig {
  // ... existing fields ...
  loopSessionId?: string;  // if provided, auto-save to this session; if absent, generate new UUID
}
```

- [ ] Add import at top of hook file:
```typescript
import { loopSessionSave } from "../ipc/loopSession";
```

- [ ] At the start of the `start` callback, generate or reuse session ID:
```typescript
const loopSessionId = config.loopSessionId ?? crypto.randomUUID();
```

- [ ] After each Verifier result is added to trace (look for `addTrace` with `kind: "verifier_result"`), add the auto-save. Find the block that calls `addTrace` for verifier result inside the iter loop and add after it:
```typescript
// Auto-save after each iteration
void loopSessionSave(
  loopSessionId,
  config.goal,
  "running",
  iter,
  { config, orchestratorHistory, sharedContext, trace: /* see note */ [] },
);
```

**Note:** `trace` is React state (`TraceEntry[]`), not available inside the callback directly. Instead, collect a mutable `traceBuffer: TraceEntry[]` ref alongside `orchestratorHistory`. Every `addTrace` call also appends to `traceBuffer`. This lets us pass `traceBuffer` to `loopSessionSave`.

Concrete implementation:

```typescript
// Add near top of start callback, after abortRef reset:
const traceBuffer: TraceEntry[] = [];
const addTraceBuffered = (entry: Omit<TraceEntry, "id" | "timestamp">) => {
  const full: TraceEntry = { ...entry, id: traceId(), timestamp: Date.now() };
  traceBuffer.push(full);
  setTrace(prev => [...prev, full]);
};
```

Then replace all `addTrace(` calls within `start` with `addTraceBuffered(`. The `addTrace` defined outside `start` is only used for `useCallback` — keep it as-is for other callers.

Helper to save current snapshot:
```typescript
const saveSnapshot = (status: "running" | "paused" | "completed" | "failed", currentIter: number) => {
  void loopSessionSave(
    loopSessionId,
    config.goal,
    status,
    currentIter,
    { config, orchestratorHistory, sharedContext, trace: traceBuffer },
  ).catch(() => {}); // never block the loop on save failure
};
```

- [ ] Call `saveSnapshot("running", iter)` after Verifier result is added (end of each iter block, before `continue` / `break` check).

- [ ] Call `saveSnapshot("paused", iter)` when `abortRef.current` is true and loop exits via user stop.

- [ ] Call `saveSnapshot("completed", iter)` when Verifier returns `done: true`.

- [ ] Call `saveSnapshot("failed", iter)` in the outer `catch` block.

- [ ] Initial save with `status: "running", iter: 0` right after preflight passes, so the session appears in the list immediately.

- [ ] Type-check:
```bash
npx tsc --noEmit 2>&1 | head -20
```

---

## Task 7: Resume support in `useOrchestratorLoop`

The hook needs to expose a way to resume from a saved snapshot.

- [ ] Add `resume` to `UseOrchestratorLoopResult`:
```typescript
export interface UseOrchestratorLoopResult {
  trace: TraceEntry[];
  isRunning: boolean;
  iteration: number;
  start: (config: LoopConfig) => Promise<void>;
  stop: () => void;
  resume: (sessionId: string) => Promise<void>;
}
```

- [ ] Implement `resume` in the hook:
```typescript
const resume = useCallback(async (sessionId: string) => {
  const { loopSessionLoad, parseLoopSessionData } = await import("../ipc/loopSession");
  const data = await loopSessionLoad(sessionId);
  const snapshot = parseLoopSessionData(data);

  // Restore trace to UI
  setTrace(snapshot.trace);
  setIteration(snapshot.iteration);

  // Start loop with restored state — pass loopSessionId so saves overwrite same record
  await start({ ...snapshot.config, loopSessionId: sessionId });
}, [start]);
```

Wait — this won't work because `start` resets trace and history. Instead, we need `start` itself to detect a resumption from `loopSessionId` being set AND the orchestratorHistory being pre-populated.

Better approach: add `resumeSnapshot?: LoopSessionSnapshot` to `LoopConfig`. When present, `start` uses it to seed `orchestratorHistory`, `sharedContext`, `traceBuffer`, and the initial iteration counter.

- [ ] Add to `LoopConfig`:
```typescript
resumeSnapshot?: {
  orchestratorHistory: ChatMessage[];
  sharedContext: string;
  trace: TraceEntry[];
  startIteration: number;
};
```

- [ ] In `start`, after `setTrace([])` / `setIteration(0)`, check for resume:
```typescript
const traceBuffer: TraceEntry[] = config.resumeSnapshot?.trace ?? [];
setTrace(traceBuffer);

const orchestratorHistory: ChatMessage[] = config.resumeSnapshot?.orchestratorHistory ?? [
  { role: "system", content: buildOrchestratorSystemPrompt(config, "") },
  { role: "user", content: `請開始執行目標：${config.goal}` },
];

let sharedContext = config.resumeSnapshot?.sharedContext ?? "";
const startIter = config.resumeSnapshot?.startIteration ?? 1;
```

- [ ] Change `for (let iter = 1; ...` to `for (let iter = startIter; ...`.

- [ ] Implement `resume`:
```typescript
const resume = useCallback(async (sessionId: string) => {
  const { loopSessionLoad, parseLoopSessionData } = await import("../ipc/loopSession");
  const data = await loopSessionLoad(sessionId);
  const snap = parseLoopSessionData(data);
  await start({
    ...snap.config,
    loopSessionId: sessionId,
    resumeSnapshot: {
      orchestratorHistory: snap.orchestratorHistory,
      sharedContext: snap.sharedContext,
      trace: snap.trace,
      startIteration: snap.iteration + 1,
    },
  });
}, [start]);
```

- [ ] Type-check:
```bash
npx tsc --noEmit 2>&1 | head -20
```

---

## Task 8: `SessionPicker` component

**Files:**
- Create: `src/components/LoopStudio/SessionPicker.tsx`

- [ ] Create the component:

```tsx
// src/components/LoopStudio/SessionPicker.tsx
import { useState, useEffect, useCallback } from "react";
import { loopSessionList, loopSessionDelete, type LoopSessionSummary } from "../../ipc/loopSession";

interface SessionPickerProps {
  onResume: (sessionId: string) => void;
  isRunning: boolean;
}

const STATUS_LABEL: Record<string, string> = {
  running: "執行中",
  paused: "已暫停",
  completed: "已完成",
  failed: "失敗",
};

function formatDate(iso: string): string {
  const d = new Date(iso);
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function SessionPicker({ onResume, isRunning }: SessionPickerProps) {
  const [sessions, setSessions] = useState<LoopSessionSummary[]>([]);
  const [open, setOpen] = useState(false);

  const refresh = useCallback(() => {
    loopSessionList().then(setSessions).catch(() => setSessions([]));
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  const handleDelete = useCallback(async (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    await loopSessionDelete(id).catch(() => {});
    refresh();
  }, [refresh]);

  if (sessions.length === 0) return null;

  return (
    <div className="ls-session-picker">
      <button
        type="button"
        className="ls-session-toggle"
        onClick={() => setOpen(o => !o)}
        disabled={isRunning}
      >
        📋 過去的 Sessions ({sessions.length})
        <span className="ls-session-chevron">{open ? "▲" : "▼"}</span>
      </button>

      {open && (
        <div className="ls-session-list">
          {sessions.map(s => (
            <div key={s.id} className={`ls-session-item status-${s.status}`}>
              <div className="ls-session-info">
                <span className="ls-session-status">{STATUS_LABEL[s.status] ?? s.status}</span>
                <span className="ls-session-goal">{s.goal.slice(0, 60)}{s.goal.length > 60 ? "..." : ""}</span>
                <span className="ls-session-meta">第 {s.iteration} 輪・{formatDate(s.updated_at)}</span>
              </div>
              <div className="ls-session-actions">
                {(s.status === "paused" || s.status === "running") && (
                  <button
                    type="button"
                    className="ls-session-resume-btn"
                    onClick={() => { setOpen(false); onResume(s.id); }}
                    disabled={isRunning}
                  >
                    ▶ 繼續
                  </button>
                )}
                <button
                  type="button"
                  className="ls-session-delete-btn"
                  onClick={e => handleDelete(e, s.id)}
                  title="刪除"
                >
                  ×
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
```

- [ ] Type-check:
```bash
npx tsc --noEmit 2>&1 | head -20
```

---

## Task 9: Wire SessionPicker into `index.tsx`

**Files:**
- Modify: `src/components/LoopStudio/index.tsx`

- [ ] Import at top:
```typescript
import { SessionPicker } from "./SessionPicker";
import { loopSessionLoad, parseLoopSessionData } from "../../ipc/loopSession";
```

- [ ] Add `handleResume` callback inside `LoopStudioView`:
```typescript
const handleResume = useCallback(async (sessionId: string) => {
  try {
    const data = await loopSessionLoad(sessionId);
    const snap = parseLoopSessionData(data);
    // Restore roster from saved config so UI reflects the resumed session
    const cfg = snap.config;
    updateRoster({
      goal: cfg.goal,
      stoppingCondition: cfg.stoppingCondition,
      orchestratorProvider: cfg.orchestrator.providerId,
      verifierProvider: cfg.verifier.providerId,
      orchestratorName: cfg.orchestrator.name,
      verifierName: cfg.verifier.name,
      subAgents: cfg.subAgents,
      maxLoops: cfg.maxLoops,
      maxOrchestratorSteps: cfg.maxOrchestratorSteps,
      maxInnerIterations: cfg.maxInnerIterations,
    });
    await loop.resume(sessionId);
  } catch (err) {
    console.error("Resume failed:", err);
  }
}, [loop, updateRoster]);
```

- [ ] Add `SessionPicker` inside `ls-left > ls-header` div (just below the title), before the first `ls-section`:
```tsx
<SessionPicker onResume={handleResume} isRunning={loop.isRunning} />
```

- [ ] Type-check:
```bash
npx tsc --noEmit 2>&1 | head -20
```

---

## Task 10: CSS for SessionPicker

**Files:**
- Modify: `src/components/LoopStudio/styles.css`

- [ ] Append these rules to `styles.css`:

```css
/* Session Picker */
.ls-session-picker {
  margin: 8px 0 12px;
}

.ls-session-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 6px 10px;
  background: var(--ls-surface, #1e1e2e);
  border: 1px solid var(--ls-border, #333);
  border-radius: 6px;
  color: var(--ls-text-muted, #888);
  font-size: 12px;
  cursor: pointer;
  text-align: left;
}
.ls-session-toggle:hover:not(:disabled) { border-color: var(--ls-accent, #7c3aed); color: var(--ls-text, #ccc); }
.ls-session-toggle:disabled { opacity: 0.5; cursor: default; }
.ls-session-chevron { margin-left: auto; }

.ls-session-list {
  margin-top: 4px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 220px;
  overflow-y: auto;
}

.ls-session-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 10px;
  background: var(--ls-surface, #1e1e2e);
  border: 1px solid var(--ls-border, #333);
  border-radius: 6px;
  font-size: 12px;
}
.ls-session-item.status-paused  { border-left: 3px solid #f59e0b; }
.ls-session-item.status-running { border-left: 3px solid #3b82f6; }
.ls-session-item.status-completed { border-left: 3px solid #22c55e; }
.ls-session-item.status-failed  { border-left: 3px solid #ef4444; }

.ls-session-info { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
.ls-session-status { font-size: 10px; font-weight: 600; text-transform: uppercase; color: var(--ls-text-muted, #888); }
.ls-session-goal { color: var(--ls-text, #ccc); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.ls-session-meta { font-size: 10px; color: var(--ls-text-muted, #888); }

.ls-session-actions { display: flex; gap: 4px; flex-shrink: 0; }
.ls-session-resume-btn {
  padding: 3px 8px;
  background: var(--ls-accent, #7c3aed);
  color: #fff;
  border: none;
  border-radius: 4px;
  font-size: 11px;
  cursor: pointer;
}
.ls-session-resume-btn:hover:not(:disabled) { opacity: 0.85; }
.ls-session-resume-btn:disabled { opacity: 0.5; cursor: default; }

.ls-session-delete-btn {
  padding: 3px 7px;
  background: transparent;
  border: 1px solid var(--ls-border, #333);
  border-radius: 4px;
  color: var(--ls-text-muted, #888);
  font-size: 13px;
  cursor: pointer;
  line-height: 1;
}
.ls-session-delete-btn:hover { border-color: #ef4444; color: #ef4444; }
```

---

## Task 11: Final build and smoke test

- [ ] Full TypeScript check:
```bash
npx tsc --noEmit 2>&1 && echo "TS OK"
```

- [ ] Full Rust build:
```bash
cd src-tauri && touch src/lib.rs && cargo build --no-default-features 2>&1 | grep -E "^error|Finished"
```

- [ ] Restart dev server and manually verify:
  1. Open Loop Studio — no sessions list visible yet (empty, hidden)
  2. Start a loop with a simple goal
  3. After iteration 1 completes, stop the loop → should appear in Sessions list as "已暫停"
  4. Click "▶ 繼續" → loop resumes from the next iteration
  5. Let a loop complete → appears in Sessions list as "已完成"
  6. Delete a session → it disappears from the list

- [ ] Commit:
```bash
git add src-tauri/src/db/loop_sessions.rs \
        src-tauri/src/db/mod.rs \
        src-tauri/src/lib.rs \
        src-tauri/src/commands/loop_session.rs \
        src-tauri/src/commands/mod.rs \
        src/ipc/loopSession.ts \
        src/hooks/useOrchestratorLoop.ts \
        src/components/LoopStudio/SessionPicker.tsx \
        src/components/LoopStudio/index.tsx \
        src/components/LoopStudio/styles.css
git commit -m "feat(loop-studio): persist sessions to SQLite with pause/resume support"
```
