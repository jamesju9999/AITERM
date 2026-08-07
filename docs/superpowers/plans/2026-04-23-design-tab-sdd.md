# Design Tab SDD Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a dedicated "Design" tab in AITERM for Spec-Driven Development (SDD), featuring dual interaction modes (Chat/Wizard), strict SDD phasing, SQLite-backed session persistence with context compression, and hybrid file saving.

**Architecture:** 
- **Backend (Rust):** SQLite tables `design_sessions` and `design_messages` via SQLx. Tauri commands for session lifecycle, message handling with AI integration, and file saving. A background compression mechanism to summarize long contexts for local models.
- **Frontend (React):** A new `DesignView` component with a toggle for Chat/Wizard modes. A `useDesignSession` hook manages state via Tauri IPC. A `SpecPreview` component renders Markdown tabs for Spec, Architecture, and Plan.
- **Data Flow:** UI -> `useDesignSession` -> `src/ipc/design.ts` -> Tauri Commands -> SQLx DB & AI Router.

**Tech Stack:** React 19, TypeScript, Tauri 2, Rust, SQLx (SQLite), xterm.js (existing), Vitest, Cargo Test.

---

### Task 1: Database Schema & Models (Rust)

**Files:**
- Create: `src-tauri/migrations/20260423000000_create_design_tables.sql`
- Create: `src-tauri/src/db/design.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Test: `src-tauri/tests/db_design_integration.rs`

- [ ] **Step 1: Write the migration file**

```sql
-- src-tauri/migrations/20260423000000_create_design_tables.sql
CREATE TABLE IF NOT EXISTS design_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    current_spec_draft TEXT,
    current_sdd_draft TEXT,
    current_plan_draft TEXT,
    context_summary TEXT,
    status TEXT NOT NULL DEFAULT 'draft', -- draft, review, approved
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS design_messages (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL, -- user, assistant, system
    content TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (session_id) REFERENCES design_sessions (id) ON DELETE CASCADE
);
```

- [ ] **Step 2: Write failing tests for DB operations**

```rust
// src-tauri/tests/db_design_integration.rs
use aiterm::db::design::{create_design_session, get_design_session};
use aiterm::db::DbManager;

#[tokio::test]
async fn test_create_and_get_design_session() {
    let db = DbManager::new_in_memory().await.unwrap();
    let session_id = create_design_session(&db.pool, "New Feature").await.unwrap();
    
    let session = get_design_session(&db.pool, &session_id).await.unwrap();
    assert_eq!(session.title, "New Feature");
    assert_eq!(session.status, "draft");
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test db_design_integration`
Expected: FAIL (unresolved imports, missing functions)

- [ ] **Step 4: Implement DB models and functions**

```rust
// src-tauri/src/db/design.rs
use sqlx::{SqlitePool, Row};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct DesignSession {
    pub id: String,
    pub title: String,
    pub current_spec_draft: Option<String>,
    pub current_sdd_draft: Option<String>,
    pub current_plan_draft: Option<String>,
    pub context_summary: Option<String>,
    pub status: String,
}

pub async fn create_design_session(pool: &SqlitePool, title: &str) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO design_sessions (id, title, status) VALUES (?, ?, 'draft')")
        .bind(&id)
        .bind(title)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn get_design_session(pool: &SqlitePool, id: &str) -> Result<DesignSession, sqlx::Error> {
    let row = sqlx::query_as!(DesignSession, "SELECT id, title, current_spec_draft, current_sdd_draft, current_plan_draft, context_summary, status FROM design_sessions WHERE id = ?", id)
        .fetch_one(pool)
        .await?;
    Ok(row)
}
```
*(Also add `pub mod design;` to `src-tauri/src/db/mod.rs`)*

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --test db_design_integration`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/migrations/ src-tauri/src/db/ src-tauri/tests/db_design_integration.rs
git commit -m "feat(db): add design_sessions and design_messages schema and basic ops"
```

### Task 2: Tauri Commands for Session Management

**Files:**
- Create: `src-tauri/src/commands/design.rs`
- Modify: `src-tauri/src/main.rs` (to register commands)
- Test: `src-tauri/tests/design_commands.rs`

- [ ] **Step 1: Write failing tests for Tauri commands**

```rust
// src-tauri/tests/design_commands.rs
use aiterm::commands::design::{design_start_session, design_load_session};

// Note: Test setup using Tauri's mock builder if applicable, or direct function calls.
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test design_commands`

- [ ] **Step 3: Implement Tauri Commands**

```rust
// src-tauri/src/commands/design.rs
use tauri::State;
use crate::db::DbManager;
use crate::db::design::{create_design_session, get_design_session, DesignSession};

#[tauri::command]
pub async fn design_start_session(db: State<'_, DbManager>, title: String) -> Result<String, String> {
    create_design_session(&db.pool, &title).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn design_load_session(db: State<'_, DbManager>, id: String) -> Result<DesignSession, String> {
    get_design_session(&db.pool, &id).await.map_err(|e| e.to_string())
}
```

- [ ] **Step 4: Register Commands**

```rust
// src-tauri/src/main.rs (Snippet addition)
// .invoke_handler(tauri::generate_handler![
//     ...,
//     aiterm::commands::design::design_start_session,
//     aiterm::commands::design::design_load_session
// ])
```

- [ ] **Step 5: Run tests to verify**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/design.rs src-tauri/src/main.rs src-tauri/tests/design_commands.rs
git commit -m "feat(ipc): add tauri commands for design session lifecycle"
```

### Task 3: Frontend IPC Wrappers

**Files:**
- Create: `src/ipc/design.ts`
- Modify: `src/ipc/index.ts`
- Test: `src/ipc/design.test.ts`

- [ ] **Step 1: Write failing tests**

```typescript
// src/ipc/design.test.ts
import { describe, it, expect, vi } from 'vitest';
import { designStartSession } from './design';
import { invoke } from '@tauri-apps/api/core';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

describe('designStartSession', () => {
    it('invokes design_start_session with title', async () => {
        vi.mocked(invoke).mockResolvedValue('session-123');
        const id = await designStartSession('New Spec');
        expect(invoke).toHaveBeenCalledWith('design_start_session', { title: 'New Spec' });
        expect(id).toBe('session-123');
    });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- design.test.ts`

- [ ] **Step 3: Implement IPC wrappers**

```typescript
// src/ipc/design.ts
import { invoke } from '@tauri-apps/api/core';

export interface DesignSession {
    id: string;
    title: string;
    current_spec_draft: string | null;
    current_sdd_draft: string | null;
    current_plan_draft: string | null;
    context_summary: string | null;
    status: string;
}

export async function designStartSession(title: string): Promise<string> {
    return invoke('design_start_session', { title });
}

export async function designLoadSession(id: string): Promise<DesignSession> {
    return invoke('design_load_session', { id });
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- design.test.ts`

- [ ] **Step 5: Commit**

```bash
git add src/ipc/design.ts src/ipc/design.test.ts
git commit -m "feat(frontend): add ipc wrappers for design sessions"
```

### Task 4: Frontend UI Component - DesignView Shell & Preview

**Files:**
- Create: `src/components/DesignView/DesignView.tsx`
- Create: `src/components/DesignView/SpecPreview.tsx`
- Modify: `src/components/TerminalApp.tsx` (Add routing/tab)

- [ ] **Step 1: Create SpecPreview Component**

```tsx
// src/components/DesignView/SpecPreview.tsx
import React, { useState } from 'react';
import { MarkdownBlock } from '../MermaidBlock'; // Assuming Markdown component exists

export function SpecPreview({ spec, sdd, plan }: { spec: string | null, sdd: string | null, plan: string | null }) {
    const [activeTab, setActiveTab] = useState<'spec' | 'sdd' | 'plan'>('spec');

    return (
        <div className="spec-preview">
            <div className="tabs">
                <button onClick={() => setActiveTab('spec')}>Spec</button>
                <button onClick={() => setActiveTab('sdd')}>Architecture</button>
                <button onClick={() => setActiveTab('plan')}>Plan</button>
            </div>
            <div className="content">
                {activeTab === 'spec' && <MarkdownBlock content={spec || '*No spec yet*'} />}
                {activeTab === 'sdd' && <MarkdownBlock content={sdd || '*No architecture yet*'} />}
                {activeTab === 'plan' && <MarkdownBlock content={plan || '*No plan yet*'} />}
            </div>
        </div>
    );
}
```

- [ ] **Step 2: Create DesignView Component**

```tsx
// src/components/DesignView/DesignView.tsx
import React, { useState } from 'react';
import { SpecPreview } from './SpecPreview';

export function DesignView() {
    const [mode, setMode] = useState<'chat' | 'wizard'>('chat');

    return (
        <div className="design-view" style={{ display: 'flex', height: '100%' }}>
            <div className="left-panel" style={{ flex: 1, borderRight: '1px solid #ccc' }}>
                <div className="mode-toggle">
                    <button onClick={() => setMode('chat')}>Chat Mode</button>
                    <button onClick={() => setMode('wizard')}>Wizard Mode</button>
                </div>
                <div className="chat-area">
                    {/* Chat or Wizard UI will go here */}
                    <p>Interaction Mode: {mode}</p>
                </div>
            </div>
            <div className="right-panel" style={{ flex: 1 }}>
                <SpecPreview spec="## Draft Spec" sdd={null} plan={null} />
            </div>
        </div>
    );
}
```

- [ ] **Step 3: Integrate into TerminalApp**

*(Modify `TerminalApp.tsx` to add the Design Tab alongside Terminal/Database)*

- [ ] **Step 4: Verify visually and Type Check**

Run: `npx tsc --noEmit`

- [ ] **Step 5: Commit**

```bash
git add src/components/DesignView/ src/components/TerminalApp.tsx
git commit -m "feat(ui): add DesignView shell and SpecPreview components"
```
