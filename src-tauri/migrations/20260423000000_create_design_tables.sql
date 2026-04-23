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
