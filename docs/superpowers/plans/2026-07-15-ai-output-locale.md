# AI 輸出語言跟隨系統 Locale Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every AI-directed prompt in AITerm (single-command `/ai`, AiPanel chat, DesignView SDD assistant, CrossDb/Database SQL assistants, LoopStudio Orchestrator/Sub-agent) respond in the language matching the current UI locale (`en` / `zh-TW`) instead of being hardcoded to Traditional Chinese.

**Architecture:** Add a `Locale` type on both the Rust and TypeScript sides plus a `language_name()` / `languageDirective()` helper that maps `Locale` → a human-readable language name. Every prompt-building function keeps its instruction text in English (language-neutral — the AI understands the instructions regardless of language) and appends exactly one dynamic "respond in {language}" rule. The frontend threads `useLocale().locale` explicitly through every relevant Tauri `invoke` call and through `LoopConfig`; no global backend locale state.

**Tech Stack:** Rust (Tauri commands, serde), TypeScript/React (Tauri IPC wrappers, hooks), Vitest, `cargo test`.

**Reference spec:** `docs/superpowers/specs/2026-07-15-ai-output-locale-design.md`

---

## Task 1: `Locale` type + `language_name()` helper (Rust)

**Files:**
- Modify: `src-tauri/src/ai/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `src-tauri/src/ai/mod.rs` (after the last existing test, before the closing `}` of `mod tests`):

```rust
    #[test]
    fn locale_deserializes_from_frontend_strings() {
        let en: Locale = serde_json::from_str("\"en\"").unwrap();
        let zh: Locale = serde_json::from_str("\"zh-TW\"").unwrap();
        assert_eq!(en, Locale::En);
        assert_eq!(zh, Locale::ZhTw);
    }

    #[test]
    fn language_name_maps_locale_to_readable_name() {
        assert_eq!(language_name(Locale::En), "English");
        assert_eq!(language_name(Locale::ZhTw), "Traditional Chinese (繁體中文)");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test locale_deserializes_from_frontend_strings language_name_maps_locale_to_readable_name`
Expected: compile error — `Locale` and `language_name` are not defined.

- [ ] **Step 3: Implement `Locale` and `language_name()`**

In `src-tauri/src/ai/mod.rs`, add this block right after the `QueryMode` enum definition (after line 70, before the `ChatMessage` struct):

```rust
/// UI locale, mirrors the frontend's `Locale` type (`"en" | "zh-TW"`).
/// Threaded explicitly through AI commands so prompt builders know what
/// language to instruct the model to respond in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Locale {
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-TW")]
    ZhTw,
}

/// Human-readable language name for the "respond in X" prompt rule.
pub fn language_name(locale: Locale) -> &'static str {
    match locale {
        Locale::En => "English",
        Locale::ZhTw => "Traditional Chinese (繁體中文)",
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test locale_deserializes_from_frontend_strings language_name_maps_locale_to_readable_name`
Expected: PASS (2 passed)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/mod.rs
git commit -m "feat(ai): add Locale type and language_name helper"
```

---

## Task 2: `languageDirective()` helper (TypeScript)

**Files:**
- Modify: `src/lib/i18n.ts`
- Create: `src/lib/i18n.test.ts`

- [ ] **Step 1: Write the failing test**

Create `src/lib/i18n.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { languageDirective } from "./i18n";

describe("languageDirective", () => {
  it("returns English for en locale", () => {
    expect(languageDirective("en")).toBe("English");
  });

  it("returns Traditional Chinese for zh-TW locale", () => {
    expect(languageDirective("zh-TW")).toBe("Traditional Chinese (繁體中文)");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/lib/i18n.test.ts`
Expected: FAIL — `languageDirective` is not exported by `./i18n`.

- [ ] **Step 3: Implement `languageDirective()`**

In `src/lib/i18n.ts`, add right after the existing two lines at the top of the file (`export type Locale = "zh-TW" | "en";` and `export const LOCALE_STORAGE_KEY = "aiterm_locale";`):

```ts
export function languageDirective(locale: Locale): string {
  return locale === "zh-TW" ? "Traditional Chinese (繁體中文)" : "English";
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/lib/i18n.test.ts`
Expected: PASS (2 passed)

- [ ] **Step 5: Commit**

```bash
git add src/lib/i18n.ts src/lib/i18n.test.ts
git commit -m "feat(i18n): add languageDirective helper"
```

---

## Task 3: `ai_query` / `build_single_command_prompt` follows locale

**Files:**
- Modify: `src-tauri/src/commands/ai.rs:12-15,101-140,189-206`
- Modify: `src-tauri/tests/ai_query_command.rs`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/tests/ai_query_command.rs`, add `Locale` to the import on line 4-6:

```rust
use aiterm_lib::ai::{
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, Locale,
};
```

Update the two existing calls to `build_single_command_prompt` to pass a locale — change line 55 and line 103:

```rust
    let a = build_single_command_prompt(&snap, Locale::ZhTw);
    let b = build_single_command_prompt(&snap, Locale::ZhTw);
```

```rust
    let prompt = build_single_command_prompt(&snap, Locale::ZhTw);
```

Then append a new test at the end of the file:

```rust
#[test]
fn single_command_prompt_language_hint_follows_locale() {
    let snap = context::snapshot_from_parts("linux", "bash", PathBuf::from("/"));
    let en_prompt = build_single_command_prompt(&snap, Locale::En);
    let zh_prompt = build_single_command_prompt(&snap, Locale::ZhTw);
    assert!(en_prompt.contains("use English"), "en prompt: {en_prompt}");
    assert!(zh_prompt.contains("use Traditional Chinese"), "zh prompt: {zh_prompt}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --test ai_query_command`
Expected: compile error — `build_single_command_prompt` takes 1 argument, 2 supplied.

- [ ] **Step 3: Implement locale-aware `build_single_command_prompt` and thread it through `ai_query`**

In `src-tauri/src/commands/ai.rs`, update the import block (lines 12-15) to include `Locale`:

```rust
use crate::ai::{
    context, router::AiRouter, AiError, AiSingleCommand, ChatMessage, GenerateChunk,
    GenerateRequest, Locale, McpToolDefinition, QueryMode, RiskLevel,
};
```

Change the `build_single_command_prompt` signature (line 101) and the schema line (line 126) to take and use `locale`:

```rust
pub fn build_single_command_prompt(snapshot: &crate::ai::EnvSnapshot, locale: Locale) -> String {
    let recent_section = snapshot.recent_output.as_deref().map(|o| {
        let trimmed = if o.len() > 2000 { &o[o.len() - 2000..] } else { o };
        format!("\nRecent terminal output (last ~50 lines):\n```\n{trimmed}\n```")
    }).unwrap_or_default();

    let dir_section = snapshot.dir_listing.as_deref().map(|d| {
        format!("\nDirectory listing ({}):\n```\n{d}\n```", snapshot.cwd.display())
    }).unwrap_or_default();

    format!(
r#"You are an AI command generator for a cross-platform terminal application.
Your only job is to translate the user's natural-language request (or execution goal) into ONE
executable shell command for their current environment.

Environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}            (may be slightly stale; prefer relative paths or
                         shell variables over hardcoded absolute paths){recent_section}{dir_section}

Rules:
1. Output ONLY a JSON object, no prose, no markdown fences, no extra keys.
2. Schema:
   {{
     "explanation": "one-sentence description of what this command does, or a summary of the completed result (use {language})",
     "command":     "a single shell command, no prompt prefix, no line breaks. SET TO 'DONE' IF GOAL IS FULLY MET.",
     "risk_level":  one of "safe", "needs_confirm", "dangerous"
   }}
3. The command must be syntactically valid for {shell}. Do not mix shells.
4. If the request cannot be satisfied with one command, pick the most useful
   single command to progress further.
5. If the user provides an execution history and it shows their ultimate goal is achieved, you MUST set "command" to "DONE".
6. Never produce destructive operations against system roots. If the user
   explicitly asks for one, set risk_level="dangerous"."#,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display(),
        language = crate::ai::language_name(locale),
    )
}
```

Add a `locale: Locale` parameter to the `ai_query` command (line 190) and pass it through (line 199):

```rust
#[tauri::command]
pub async fn ai_query(
    query: String,
    session_id: String,
    locale: Locale,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiCommandReady, AiError> {
    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = router.resolve().await?;
    let prompt = build_single_command_prompt(&snapshot, locale);
```

(No other lines in `ai_query` change.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --test ai_query_command`
Expected: PASS (5 passed)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/ai.rs src-tauri/tests/ai_query_command.rs
git commit -m "feat(ai): make ai_query's language hint follow locale"
```

---

## Task 4: `ai_chat` / `build_chat_prompt` follows locale

**Files:**
- Modify: `src-tauri/src/commands/ai.rs:142-187,296-334`
- Modify: `src-tauri/tests/ai_chat_command.rs`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/tests/ai_chat_command.rs`, add `Locale` to the import (line 4-6):

```rust
use aiterm_lib::ai::{
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, Locale, QueryMode,
};
```

Give the `run_chat_loop` helper (line 84-117) a `locale` parameter:

```rust
async fn run_chat_loop(
    provider: Arc<dyn AiProvider>,
    messages: Vec<ChatMessage>,
    locale: Locale,
) -> Result<String, AiError> {
    let snapshot = context::snapshot_from_parts(
        "linux",
        "bash",
        std::path::PathBuf::from("/"),
    );
    let prompt = build_chat_prompt(&snapshot, locale);
```

(rest of the function body is unchanged)

Update all four existing call sites to pass `Locale::ZhTw` (lines 136, 158, 174, 202):

```rust
    let content = run_chat_loop(provider, vec![user("列出所有檔案")], Locale::ZhTw)
```
```rust
    run_chat_loop(provider, history.clone(), Locale::ZhTw).await.expect("ok");
```
```rust
    run_chat_loop(provider, vec![user("hi")], Locale::ZhTw).await.expect("ok");
```
```rust
    let err = run_chat_loop(provider, vec![user("hi")], Locale::ZhTw).await.unwrap_err();
```

Then append a new test at the end of the file:

```rust
#[tokio::test]
async fn chat_prompt_language_rule_follows_locale() {
    let mock = MockProvider::new(vec!["x"]);
    let captured = mock.last_request.clone();
    let provider: Arc<dyn AiProvider> = Arc::new(mock);

    run_chat_loop(provider, vec![user("hi")], Locale::En).await.expect("ok");

    let got = captured.lock().unwrap().clone().unwrap();
    assert!(got.system_prompt.contains("Respond in English."), "prompt: {}", got.system_prompt);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --test ai_chat_command`
Expected: compile error — `build_chat_prompt` takes 1 argument, 2 supplied / `run_chat_loop` argument count mismatch.

- [ ] **Step 3: Implement locale-aware `build_chat_prompt` and thread it through `ai_chat`**

In `src-tauri/src/commands/ai.rs`, change `build_chat_prompt`'s signature (line 145) and Rule 1 (line 173):

```rust
pub fn build_chat_prompt(snapshot: &crate::ai::EnvSnapshot, locale: Locale) -> String {
    let recent_section = snapshot.recent_output.as_deref().map(|o| {
        let trimmed = if o.len() > 2000 {
            let start = o.len() - 2000;
            let start = (start..=o.len())
                .find(|&i| o.is_char_boundary(i))
                .unwrap_or(o.len());
            &o[start..]
        } else {
            o
        };
        format!("\nRecent terminal output (last ~50 lines):\n```\n{trimmed}\n```")
    }).unwrap_or_default();

    let dir_section = snapshot.dir_listing.as_deref().map(|d| {
        format!("\nDirectory listing ({}):\n```\n{d}\n```", snapshot.cwd.display())
    }).unwrap_or_default();

    format!(
r#"You are an AI terminal assistant. The user is in an interactive terminal
session and you can see their OS, shell, cwd, and recent output.

Environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}{recent_section}{dir_section}

Rules:
1. Respond in {language}.
2. When you want to suggest a runnable shell command, wrap it in
   <cmd>...</cmd> tags. The user can click the tag to execute it.
3. You may include multiple <cmd> tags in one reply if needed.
4. Each <cmd> must contain a command valid for {shell}. Prefer single-line
   commands; multi-line commands will ask the user for confirmation before
   executing.
5. Free-form explanation outside <cmd> tags is encouraged.
6. Never produce destructive operations against system roots unless the
   user explicitly asks; if you do, mark it clearly in prose."#,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display(),
        language = crate::ai::language_name(locale),
    )
}
```

Add a `locale: Locale` parameter to the `ai_chat` command signature (line 297-306) and pass it through at the `build_chat_prompt` call (line 327):

```rust
#[tauri::command]
pub async fn ai_chat(
    messages: Vec<ChatMessage>,
    session_id: String,
    provider_id: Option<String>,
    use_mcp: bool,
    locale: Locale,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
    mcp_manager: State<'_, std::sync::Arc<tokio::sync::Mutex<crate::mcp::McpManager>>>,
    config: State<'_, std::sync::Arc<crate::config::ConfigStore>>,
) -> Result<AiChatReply, AiError> {
```

```rust
    let prompt = build_chat_prompt(&snapshot, locale);
```

(No other lines in `ai_chat` change — the MCP tool-calling path already reuses `req.system_prompt` via `fallback_req`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --test ai_chat_command`
Expected: PASS (5 passed)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/ai.rs src-tauri/tests/ai_chat_command.rs
git commit -m "feat(ai): make ai_chat's response-language rule follow locale"
```

---

## Task 5: `design_chat` / `build_design_prompt` / `build_stage_instruction` follow locale

**Files:**
- Modify: `src-tauri/src/commands/design.rs:1-234,306-315,350-351`

- [ ] **Step 1: Write the failing tests**

Add a `#[cfg(test)] mod tests` block at the end of `src-tauri/src/commands/design.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{EnvSnapshot, Locale};

    fn fixture_session(status: &str) -> DesignSession {
        DesignSession {
            id: "s1".into(),
            title: "t".into(),
            current_proposal_draft: None,
            current_spec_draft: None,
            current_sdd_draft: None,
            current_plan_draft: None,
            context_summary: None,
            status: status.into(),
        }
    }

    fn fixture_snapshot() -> EnvSnapshot {
        EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: std::path::PathBuf::from("/"),
            ..Default::default()
        }
    }

    #[test]
    fn design_prompt_language_rule_follows_locale() {
        let session = fixture_session("draft");
        let snapshot = fixture_snapshot();
        let en_prompt = build_design_prompt(&session, &snapshot, Locale::En);
        let zh_prompt = build_design_prompt(&session, &snapshot, Locale::ZhTw);
        assert!(en_prompt.contains("Respond in English."), "en prompt: {en_prompt}");
        assert!(zh_prompt.contains("Respond in Traditional Chinese (繁體中文)."), "zh prompt: {zh_prompt}");
    }

    #[test]
    fn design_prompt_role_instruction_matches_status() {
        let snapshot = fixture_snapshot();
        let pm_prompt = build_design_prompt(&fixture_session("draft"), &snapshot, Locale::En);
        assert!(pm_prompt.contains("Product Manager"));
        let architect_prompt = build_design_prompt(&fixture_session("spec_approved"), &snapshot, Locale::En);
        assert!(architect_prompt.contains("Software Architect"));
    }

    #[test]
    fn build_stage_instruction_is_language_neutral() {
        let proposal = build_stage_instruction("proposal");
        assert!(proposal.contains("UPDATE_PROPOSAL"));
        assert!(!proposal.is_empty());
        assert_eq!(build_stage_instruction("unknown"), "");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib design`
Expected: compile error — `build_design_prompt` takes 2 arguments, 3 supplied.

- [ ] **Step 3: Rewrite `build_stage_instruction` and `build_design_prompt` in language-neutral English + dynamic locale rule**

In `src-tauri/src/commands/design.rs`, add `Locale` to the import block at the top (lines 5-8):

```rust
use crate::ai::{
    context, router::AiRouter, AiError, ChatMessage, GenerateChunk,
    GenerateRequest, Locale, QueryMode,
};
```

Replace the entire `build_stage_instruction` function (lines 46-138) with:

```rust
/// Returns a focused instruction block when the user clicks "Generate" for a specific stage.
/// This is injected into the AI prompt to ensure strict adherence to OpenSpec templates.
pub fn build_stage_instruction(stage: &str) -> &'static str {
    match stage {
        "proposal" => r#"[Stage Instruction: Generate Proposal]
Produce the proposal strictly following this OpenSpec structure, wrapped in an [UPDATE_PROPOSAL] tag:

[UPDATE_PROPOSAL]
## Why
(Problem statement: why is this change needed? What problem does it solve?)

## What Changes
(Concrete description of the change: what is being added, modified, or removed?)

## Capabilities
### New
- capability-name — brief description of this new capability
### Modified
- existing-capability — description of the change

## Impact
(Scope of impact on code, APIs, database, dependencies)
[/UPDATE_PROPOSAL]

Produce the full proposal based on the current conversation."#,
        "spec" => r#"[Stage Instruction: Generate Spec]
Produce the spec strictly following this OpenSpec structure, wrapped in an [UPDATE_SPEC] tag.
Create one section per Capability from the proposal:

[UPDATE_SPEC]
## Capability: capability-name

### Requirement: requirement name
Requirement description text

#### Scenario: scenario name
WHEN (trigger condition)
THEN (expected result)

#### Scenario: another scenario
WHEN ...
THEN ...

(repeat this structure, one ## block per Capability)
[/UPDATE_SPEC]

Important:
- Use delta markers (ADDED / MODIFIED / REMOVED) to tag requirement status
- Scenarios must use a level-4 heading (####) with the WHEN/THEN format
- Produce these based on the proposal's Capabilities section, one at a time"#,
        "sdd" => r#"[Stage Instruction: Generate Design]
Produce the technical design strictly following this OpenSpec structure, wrapped in an [UPDATE_SDD] tag:

[UPDATE_SDD]
## Context
(Background and current state: relevant existing architecture and constraints)

## Goals / Non-Goals
### Goals
- ...
### Non-Goals
- ...

## Decisions
(Key technical decisions and their rationale)

## Risks / Trade-offs
(Known risks and design trade-offs)
[/UPDATE_SDD]

Important:
- Every decision must be traceable back to the approved spec
- When including Mermaid diagrams, node names containing parentheses or special characters must be wrapped in double quotes
- `end` only pairs with `subgraph`; `graph TD` itself does not need an `end`"#,
        "plan" => r#"[Stage Instruction: Generate Tasks]
Produce the task list strictly following this OpenSpec structure, wrapped in an [UPDATE_PLAN] tag:

[UPDATE_PLAN]
## 1. First task group name
- [ ] 1.1 Task description
- [ ] 1.2 Task description

## 2. Second task group name
- [ ] 2.1 Task description
- [ ] 2.2 Task description
[/UPDATE_PLAN]

Important:
- Every task must use the checkbox format: `- [ ] X.Y description`
- Number groups in dependency order
- Every task must map to a component or interface in the design document
- A task's acceptance criteria must match a scenario in the original spec"#,
        _ => "",
    }
}
```

Replace the entire `build_design_prompt` function (lines 140-234) with:

```rust
pub fn build_design_prompt(session: &DesignSession, snapshot: &crate::ai::EnvSnapshot, locale: Locale) -> String {
    let proposal = session.current_proposal_draft.as_deref().unwrap_or("No proposal yet.");
    let spec = session.current_spec_draft.as_deref().unwrap_or("No spec yet.");
    let sdd = session.current_sdd_draft.as_deref().unwrap_or("No design yet.");
    let plan = session.current_plan_draft.as_deref().unwrap_or("No tasks yet.");
    let summary = session.context_summary.as_deref().unwrap_or("No conversation summary yet.");

    let role_instruction = match session.status.as_str() {
        "draft" => r#"Your current role is Product Manager.
Your task: clarify user intent through questions, and help build the Proposal.
Goal: flesh out the "Proposal" document on the right — clarify Why, What Changes, Capabilities, Impact.
Constraint: do not jump ahead to spec or technical design before the proposal is approved."#,
        "proposal_approved" => r#"Your current role is Product Manager.
Your task: based on the "Approved Proposal", define detailed requirements and acceptance scenarios for each Capability.
Goal: flesh out the "Spec" document on the right.
Core rules:
1. Every Capability must have a clear requirement description and WHEN/THEN scenarios.
2. Use delta markers (ADDED / MODIFIED / REMOVED) to tag requirement status.
3. Never add functionality not mentioned in the proposal."#,
        "spec_approved" => r#"Your current role is Software Architect.
Your task: based on the "Approved Spec", perform technology selection, module breakdown, API design, and database schema design.
Goal: flesh out the "Design" document on the right.
Core rules:
1. Every architectural decision must be 100% traceable back to the approved spec.
2. Where possible, note "(maps to Spec Capability: X)" when writing.
3. Never add functionality or extensibility not mentioned in the spec."#,
        "sdd_approved" => r#"Your current role is Tech Lead.
Your task: break the "Approved Design" down into concrete checkbox task items.
Goal: flesh out the "Tasks" document on the right.
Core rules:
1. Every task must map to a component or interface in the design document.
2. A task's acceptance criteria must match a scenario in the original spec.
3. Every task must use the `- [ ] X.Y description` format."#,
        _ => "You are a professional software engineering expert.",
    };

    format!(
r#"You are a professional software requirements analyst and architect helping the user with Spec-Driven Development, following the OpenSpec framework.

Current stage: {status_label}
{role_instruction}

Current project summary:
{summary}

Current content of the right-hand panel:
---
[Proposal]
{proposal}

[Spec]
{spec}

[Design]
{sdd}

[Tasks]
{plan}
---

Current terminal environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}

Rules:
1. Respond in {language}.
2. Your goal is to clarify requirements through questions, and progressively flesh out the documents on the right.
3. When you decide to update or create a document, you MUST place the full content inside the matching tag block:
   - Proposal: [UPDATE_PROPOSAL] ```markdown ...content... ```
   - Spec: [UPDATE_SPEC] ```markdown ...content... ```
   - Design: [UPDATE_SDD] ```markdown ...content... ```
   - Tasks: [UPDATE_PLAN] ```markdown ...content... ```
4. [CRITICAL] ALL content you intend to write to a document (especially Mermaid diagrams!) MUST be strictly inside the tag blocks above. Content written outside the tag blocks will NOT be saved, and the right panel will not be able to render the diagram!
5. [Mermaid syntax] When drawing Mermaid diagrams, if a node name contains parentheses or special characters, it MUST be wrapped in double quotes, e.g. `UI["Frontend (React/Vue)"]` — never `UI[Frontend (React/Vue)]`, or it will cause a syntax error!
6. [Mermaid `end` pairing] The `end` keyword only closes a `subgraph` block, and must be paired one-to-one. `graph TD` itself does not need an `end`. Connection statements (like `A --> B`) go after all subgraphs — do not add an extra `end`.
7. Even when you use a tag to update a draft, still explain in the conversation what changes you made."#,
        status_label = match session.status.as_str() {
            "draft" => "1. Exploring Proposal",
            "proposal_approved" => "2. Defining Spec",
            "spec_approved" => "3. Designing (Design)",
            "sdd_approved" => "4. Planning Tasks",
            _ => "Completed"
        },
        role_instruction = role_instruction,
        summary = summary,
        proposal = proposal,
        spec = spec,
        sdd = sdd,
        plan = plan,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display(),
        language = crate::ai::language_name(locale),
    )
}
```

Add a `locale: Locale` parameter to the `design_chat` command signature (around line 307-315) and pass it at the `build_design_prompt` call (around line 351):

```rust
#[tauri::command]
pub async fn design_chat(
    session_id: String,
    messages: Vec<ChatMessage>,
    provider_id: Option<String>,
    locale: Locale,
    app: AppHandle,
    design_db: State<'_, DesignDb>,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiChatReply, AiError> {
```

```rust
    let base_prompt = build_design_prompt(&session, &snapshot, locale);
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib design`
Expected: PASS (3 passed)

- [ ] **Step 5: Run the full Rust test suite to check for regressions**

Run: `cd src-tauri && cargo test`
Expected: all tests PASS (no other file calls `build_design_prompt`, `build_stage_instruction`, or `design_chat` directly)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/design.rs
git commit -m "feat(design): rewrite design_chat prompts in English with locale-driven response language"
```

---

## Task 6: TypeScript IPC wrappers accept `locale`

**Files:**
- Modify: `src/ipc/ai.ts:1-2,82-118`
- Modify: `src/ipc/design.ts:1-3,69-75`

- [ ] **Step 1: Write the failing test**

Add to `src/hooks/useAiChat.test.ts`, inside the `describe("useAiChat", ...)` block (after the last existing `it(...)`, before the closing `});`):

```ts
  it("passes the default locale to invokeAiChat when none is threaded", async () => {
    invokeMock.mockResolvedValueOnce({ content: "ok" });
    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.send("hi");
    });

    const lastCall = invokeMock.mock.calls[invokeMock.mock.calls.length - 1];
    const args = lastCall[1] as { locale: string };
    expect(args.locale).toBe("zh-TW");
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/hooks/useAiChat.test.ts`
Expected: FAIL — `args.locale` is `undefined`, not `"zh-TW"`.

- [ ] **Step 3: Add `locale` params to the IPC wrappers**

In `src/ipc/ai.ts`, update the import on line 2 to bring in the `Locale` type:

```ts
import { LOCALE_STORAGE_KEY, translations, type Locale } from "../lib/i18n";
```

Replace `invokeAiChat`, `aiChat`, and `invokeAiQuery` (lines 82-118) with:

```ts
export function invokeAiChat(
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string,
  useMcp = false,
  locale: Locale = "zh-TW",
): Promise<AiChatReply> {
  return invoke<AiChatReply>("ai_chat", { messages, sessionId, providerId: providerId ?? null, useMcp, locale });
}

export const aiChat = (
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string,
  useMcp = false,
  locale: Locale = "zh-TW",
): Promise<AiChatReply> =>
  invoke("ai_chat", {
    messages,
    sessionId,
    providerId: providerId ?? null,
    useMcp,
    locale,
  });

export type AiStreamKind = "query" | "chat";

export interface AiStreamEvent {
  session_id: string;
  kind: AiStreamKind;
  delta: string;
  done: boolean;
}

export function invokeAiQuery(
  query: string,
  sessionId: string,
  locale: Locale = "zh-TW",
): Promise<AiCommandReady> {
  return invoke<AiCommandReady>("ai_query", { query, sessionId, locale });
}
```

In `src/ipc/design.ts`, update the import on line 3 to bring in `Locale`:

```ts
import type { ChatMessage } from './ai';
import type { Locale } from '../lib/i18n';
```

Replace `designChat` (lines 69-75) with:

```ts
export async function designChat(
  sessionId: string,
  messages: ChatMessage[],
  providerId?: string,
  locale: Locale = 'zh-TW',
): Promise<DesignChatReply> {
  return invoke('design_chat', { sessionId, messages, providerId: providerId ?? null, locale });
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/hooks/useAiChat.test.ts`
Expected: PASS (all tests in the file, including the new one)

- [ ] **Step 5: Run the full frontend test suite and type check**

Run: `npm run test && npx tsc --noEmit`
Expected: all tests PASS, no type errors

- [ ] **Step 6: Commit**

```bash
git add src/ipc/ai.ts src/ipc/design.ts src/hooks/useAiChat.test.ts
git commit -m "feat(ipc): thread locale through ai_query/ai_chat/design_chat invoke wrappers"
```

---

## Task 7: Wire `locale` through frontend call sites

**Files:**
- Modify: `src/hooks/useAiChat.ts:76-152`
- Modify: `src/components/TerminalView.tsx:1124-1150,1399-1411`
- Modify: `src/components/AiPanel/index.tsx:54,188-257`
- Modify: `src/components/DesignView/DesignView.tsx:34,267-348`
- Modify: `src/components/CrossDbView/CrossDbAiChat.tsx:269-273,300-304,377-381`
- Modify: `src/components/DatabaseView/DatabaseAiChat.tsx:294-298,348-352`
- Modify: `src/components/LoopStudio/AgentRoster.tsx:120-133`

This task has no new automated test (all target components already have coverage gaps for direct AI-call assertions, or are exercised indirectly through Task 6's test); it is verified by the full test suite + typecheck in Step 2, plus manual verification in Task 11.

- [ ] **Step 1: Wire `locale` into every call site**

**`src/hooks/useAiChat.ts`** — the hook already calls `const { t } = useLocale();` at line 77. Change it to also destructure `locale`, and pass it to `invokeAiChat`:

```ts
  const { t, locale } = useLocale();
```

```ts
  const invokeChat = useCallback(
    async (msgs: ChatMessage[], useMcp = false) => {
      setStreamBuf("");
      setIsStreaming(true);
      setError(null);
      setToolCallingUnsupported(false);
      try {
        const reply: AiChatReply = await invokeAiChat(msgs, sessionId, undefined, useMcp, locale);
        if (!mountedRef.current) return;
        setMessages([...msgs, { role: "assistant", content: reply.content ?? "" }]);
        setToolCallingUnsupported(reply.tool_calling_unsupported ?? false);
      } catch (e) {
        if (!mountedRef.current) return;
        setError(normalizeAiError(e));
        // Do NOT roll back `msgs` — user message stays so UI can show a retry.
      } finally {
        if (mountedRef.current) {
          setStreamBuf("");
          setIsStreaming(false);
        }
      }
    },
    [sessionId, locale],
  );
```

**`src/components/TerminalView.tsx`** — add `import type { Locale } from "../lib/i18n";` next to the existing `useLocale` import (line 3), give `handleAiQuery` a `locale` parameter, and use it at the `invokeAiQuery` call (line 1149):

```ts
function handleAiQuery(
  t: any,
  locale: Locale,
  sessionId: string,
  originalLine: string,
  query: string,
  term: Terminal,
  setPreview: (p: PreviewState) => void,
  setStreamText: React.Dispatch<React.SetStateAction<string>>,
  streamingRef: React.MutableRefObject<boolean>,
  executionModeRef: React.MutableRefObject<ExecutionMode>,
  writeRed: (msg: string) => void,
  submitCommand: (cmd: string, onComplete?: (block: import("../hooks/useTerminalBlocks").TerminalBlock) => void) => void,
  onDone?: (explanation?: string) => void,
  agentActive = false,
  onCommandComplete?: (block: import("../hooks/useTerminalBlocks").TerminalBlock) => void,
  onAiError?: (err: AiError) => void,
  onWebAction?: (type: "search" | "fetch", value: string) => void
) {
  void originalLine;
  term.write("\r\x1b[2K");
  term.write("→ asking AI...\r\n");
  setStreamText("");
  streamingRef.current = true;
  setPreview({ loading: true, visible: false, command: "", explanation: "", riskLevel: "safe" });

  invokeAiQuery(query, sessionId, locale)
```

Update the call site (line 1400) to pass `locale` right after `t`:

```ts
  handleAiQuery(
    t,
    locale,
    sessionId,
```

(`locale` is already available at the call site via the component-level `const { t } = useLocale();` at line 108 — change that line to `const { t, locale } = useLocale();`.)

**`src/components/AiPanel/index.tsx`** — change line 54 to also destructure `locale`, pass it to the `invokeAiChat` call at line 211, and add it to `runAgentLoop`'s dependency array (line 257):

```ts
  const { t, locale } = useLocale();
```

```ts
      const replyObj = await invokeAiChat(agentMessages, sessionId, undefined, false, locale);
```

```ts
  }, [chat, onExecuteCommand, sessionId, locale]);
```

**`src/components/DesignView/DesignView.tsx`** — change line 34 to also destructure `locale`, pass it to the `designChat` call at line 288, and add it to `handleSendMessage`'s dependency array (line 348):

```ts
  const { t, locale } = useLocale();
```

```ts
      const response = await designChat(session.id, combinedMessages, providerId, locale);
```

```ts
  }, [inputValue, session, messages, isStreaming, refreshSession, providerId, t, locale]);
```

**`src/components/CrossDbView/CrossDbAiChat.tsx`** — `locale` is already destructured at line 127 (`const { t, locale } = useLocale();`). Add it as the 4th/5th argument to all three `aiChat(...)` calls:

```ts
        const aiResult = await aiChat(
          [{ role: "system" as const, content: buildSystemPrompt() }, ...loopHistory.slice(0, -1), { role: "user" as const, content: lastUserContent }],
          "crossdb",
          selectedProviderId || undefined,
          false,
          locale,
        );
```

```ts
          const summaryResult1 = await aiChat(
            [{ role: "system" as const, content: buildSystemPrompt() }, ...loopHistory, { role: "user" as const, content: t.cdb_ai_summarizing_prompt }],
            "crossdb",
            selectedProviderId || undefined,
            false,
            locale,
          );
```

```ts
          const summaryResult2 = await aiChat(
            [{ role: "system" as const, content: buildSystemPrompt() }, ...loopHistory, { role: "user" as const, content: t.cdb_ai_summarizing_prompt }],
            "crossdb",
            selectedProviderId || undefined,
            false,
            locale,
          );
```

**`src/components/DatabaseView/DatabaseAiChat.tsx`** — `locale` is already destructured at line 131 (`const { t, locale } = useLocale();`). Add it to both `aiChat(...)` calls:

```ts
        const aiResult = await aiChat(
          [{ role: "system" as const, content: buildSystemPrompt(userMsg) }, ...loopHistory.slice(0, -1), { role: "user" as const, content: lastUserContent }],
          `db-${connectionId}`,
          selectedProviderId || undefined,
          false,
          locale,
        );
```

```ts
          const summaryResult = await aiChat(
            [{ role: "system" as const, content: buildSystemPrompt(userMsg) }, ...loopHistory, { role: "user" as const, content: t.db_ai_summarizing_prompt }],
            `db-${connectionId}`,
            selectedProviderId || undefined,
            false,
            locale,
          );
```

**`src/components/LoopStudio/AgentRoster.tsx`** — add `locale` to the `useLocale()` destructure (currently `const { t } = useLocale();`, need to find its exact line near the top of the component) and pass it to the `invokeAiChat` call at line 120:

```ts
  const { t, locale } = useLocale();
```

```ts
      const reply = await invokeAiChat(
        [
          {
            role: "system",
            content: t.ls_role_generator_system,
          },
          {
            role: "user",
            content: t.ls_role_generator_user(agent.name),
          },
        ],
        "roster-agent-gen",
        providers[0]?.id,
        false,
        locale,
      );
```

- [ ] **Step 2: Run the full frontend test suite and type check**

Run: `npm run test && npx tsc --noEmit`
Expected: all tests PASS, no type errors

- [ ] **Step 3: Commit**

```bash
git add src/hooks/useAiChat.ts src/components/TerminalView.tsx src/components/AiPanel/index.tsx src/components/DesignView/DesignView.tsx src/components/CrossDbView/CrossDbAiChat.tsx src/components/DatabaseView/DatabaseAiChat.tsx src/components/LoopStudio/AgentRoster.tsx
git commit -m "feat(ai): thread UI locale through all AI chat/query call sites"
```

---

## Task 8: LoopStudio Sub-agent prompt follows locale

**Files:**
- Modify: `src/hooks/useSubAgentLoop.ts`
- Create: `src/hooks/useSubAgentLoop.test.ts`

- [ ] **Step 1: Write the failing test**

Create `src/hooks/useSubAgentLoop.test.ts`:

```ts
import { describe, expect, it, vi, beforeEach } from "vitest";

const agentChatMock = vi.fn();
vi.mock("../ipc/ai", async () => {
  const actual = await vi.importActual<typeof import("../ipc/ai")>("../ipc/ai");
  return { ...actual, agentChat: (...args: unknown[]) => agentChatMock(...args) };
});
vi.mock("../ipc/fs", () => ({
  readFile: vi.fn(),
  writeTextFile: vi.fn(),
  listDirectory: vi.fn(),
  getSessionCwd: vi.fn().mockResolvedValue("/project"),
}));
vi.mock("../ipc/exec", () => ({ agentExec: vi.fn() }));

import { runSubAgent, type AgentDefinition } from "./useSubAgentLoop";

const agent: AgentDefinition = {
  name: "Coder",
  providerId: "prov1",
  roleDescription: "You write code.",
  tools: [],
};

describe("runSubAgent locale", () => {
  beforeEach(() => {
    agentChatMock.mockReset();
    agentChatMock.mockResolvedValue({ content: "done", tool_calls: [], tool_calling_unsupported: false });
  });

  it("system prompt asks for an English report when locale is en", async () => {
    await runSubAgent("s1", agent, "do the task", { locale: "en" });
    const [, history] = agentChatMock.mock.calls[0];
    const systemMsg = (history as { role: string; content: string }[]).find((m) => m.role === "system");
    expect(systemMsg?.content).toContain("report in English");
  });

  it("system prompt asks for a Traditional Chinese report by default", async () => {
    await runSubAgent("s1", agent, "do the task", {});
    const [, history] = agentChatMock.mock.calls[0];
    const systemMsg = (history as { role: string; content: string }[]).find((m) => m.role === "system");
    expect(systemMsg?.content).toContain("report in Traditional Chinese (繁體中文)");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/hooks/useSubAgentLoop.test.ts`
Expected: FAIL — `RunSubAgentOptions` has no `locale` property (type error) and/or the system prompt doesn't contain "report in English".

- [ ] **Step 3: Implement locale-aware sub-agent prompt and English tool-error strings**

In `src/hooks/useSubAgentLoop.ts`, add the import at the top:

```ts
import { agentChat, type AgentToolDefinition, type ChatMessage } from "../ipc/ai";
import { readFile, writeTextFile, listDirectory, getSessionCwd } from "../ipc/fs";
import { agentExec } from "../ipc/exec";
import { classifyCommand, commandWritesOutsideRoot } from "../lib/commandRisk";
import { isPathInside } from "../lib/pathUtils";
import { languageDirective, type Locale } from "../lib/i18n";
```

Replace the Chinese error strings inside `executeTool` (these become `tool` result content fed back to the AI, so they must stay language-neutral English regardless of locale, matching the rest of the tool-calling machinery):

```ts
      case "write_file": {
        if (!ctx.effectiveRoot) {
          return {
            result: "Error: could not determine the project directory; refusing to write for safety. Please set a project directory in Loop Studio.",
            isError: true,
          };
        }
        if (!isPathInside(args.path, ctx.effectiveRoot)) {
          return {
            result: `Error: writing outside the project directory (${ctx.effectiveRoot}) is not allowed: ${args.path}. Please use a path inside the project directory instead.`,
            isError: true,
          };
        }
        await writeTextFile(args.path, args.content);
        return { result: `Successfully wrote ${args.path}`, isError: false };
      }
```

```ts
      case "execute_command": {
        if (!ctx.effectiveRoot) {
          return {
            result: "Error: could not determine the project directory; refusing to execute commands for safety. Please set a project directory in Loop Studio.",
            isError: true,
          };
        }
        const root = ctx.effectiveRoot;
        const isDangerous = classifyCommand(args.command) === "dangerous" || commandWritesOutsideRoot(args.command, root);
        if (isDangerous) {
          if (!ctx.onConfirmNeeded) {
            return { result: "Error: this command was classified as dangerous, and dangerous commands are not allowed in this execution environment.", isError: true };
          }
          const approved = await ctx.onConfirmNeeded(args.command);
          if (!approved) {
            return { result: "The user declined to run this command. Please find another way to complete the task.", isError: true };
          }
        }
```

Add `locale?: Locale` to `RunSubAgentOptions`:

```ts
export interface RunSubAgentOptions {
  onAction?: (action: SubAgentAction) => void;
  onConfirmNeeded?: (command: string) => Promise<boolean>;
  maxInnerIterations?: number;
  sharedContext?: string;
  projectDir?: string;
  locale?: Locale;
}
```

Rewrite `runSubAgent`'s prompt assembly:

```ts
export async function runSubAgent(
  sessionId: string,
  agent: AgentDefinition,
  task: string,
  options: RunSubAgentOptions = {},
): Promise<SubAgentResult> {
  const { onAction, onConfirmNeeded, maxInnerIterations = 30, sharedContext = "", projectDir, locale = "zh-TW" } = options;

  const cwd = projectDir
    || await getSessionCwd(sessionId).catch(() => null)
    || null;

  const contextSection = sharedContext
    ? `\n## Accumulated Context From Previous Iterations\nBelow is the record of what has and hasn't been completed for this task so far — avoid repeating completed work:\n${sharedContext}\n`
    : "";

  const systemPrompt = `${agent.roleDescription}

Current working directory: ${cwd ?? "(unknown)"}
All file operations and shell commands MUST be performed under this directory. Use absolute paths when writing files.
${contextSection}
## Instructions
Use the available tools to complete the assigned task. When done, report in ${languageDirective(locale)}:
1. What you did (concrete actions)
2. What the result was (concrete output or findings)
3. Whether there were any issues or incomplete parts

Do not repeat work already marked as completed in the Context.`;

  const history: ChatMessage[] = [
    { role: "system", content: systemPrompt },
    { role: "user", content: task },
  ];

  return runToolLoop(
    agent.providerId,
    history,
    agent.tools,
    { sessionId, effectiveRoot: cwd, onConfirmNeeded },
    maxInnerIterations,
    onAction,
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/hooks/useSubAgentLoop.test.ts`
Expected: PASS (2 passed)

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useSubAgentLoop.ts src/hooks/useSubAgentLoop.test.ts
git commit -m "feat(loop-studio): sub-agent prompt follows locale, tool errors are English"
```

---

## Task 9: LoopStudio Orchestrator prompts follow locale

**Files:**
- Modify: `src/hooks/useOrchestratorLoop.ts`

This task has no new automated test — `useOrchestratorLoop`'s `start()` drives a long async loop with many `agentChat`/`runSubAgent` calls and is not currently covered by any test (verified: no existing test file references `useOrchestratorLoop`). Adding a full harness is out of scope for this change; the rewrite is verified by TypeScript's structural typing (Step 2) and by the manual LoopStudio smoke test in Task 11.

- [ ] **Step 1: Rewrite all AI-bound prompt/message text to English + dynamic locale rule**

In `src/hooks/useOrchestratorLoop.ts`, add the import:

```ts
import { useState, useCallback, useRef } from "react";
import { agentChat, type AgentToolDefinition, type ChatMessage } from "../ipc/ai";
import { runSubAgent, runToolLoop, serializeError, type AgentDefinition, type SubAgentAction } from "./useSubAgentLoop";
import { loopSessionSave, loopSessionLoad, parseLoopSessionData } from "../ipc/loopSession";
import { languageDirective, type Locale } from "../lib/i18n";
```

Add `locale?: Locale` to `LoopConfig` (right after `fullAuto?: boolean;`):

```ts
export interface LoopConfig {
  goal: string;
  stoppingCondition: string;
  orchestrator: OrchestratorAgent;
  verifier: OrchestratorAgent;
  subAgents: OrchestratorAgent[];
  maxLoops: number;
  /** 0 = unlimited */
  maxOrchestratorSteps: number;
  /** 0 = unlimited */
  maxInnerIterations: number;
  sessionId: string;
  /** Absolute path all sub-agents should treat as the working directory */
  projectDir?: string;
  /** true = 跳過危險指令確認，全自動執行 */
  fullAuto?: boolean;
  /** UI locale at the time the loop was started — controls AI-bound prompt language */
  locale?: Locale;
  loopSessionId?: string;
  resumeSnapshot?: {
    orchestratorHistory: ChatMessage[];
    sharedContext: string;
    trace: TraceEntry[];
    startIteration: number;
  };
}
```

Rewrite `buildOrchestratorSystemPrompt` to take `locale` and produce an English scaffold with a dynamic final rule:

```ts
function buildOrchestratorSystemPrompt(config: LoopConfig, sharedContext: string, locale: Locale): string {
  const agentDescriptions = config.subAgents.map(a => {
    const toolList = a.tools.length > 0 ? a.tools.join(", ") : "none";
    return `- ${a.name}: ${a.roleDescription} (tools: ${toolList})`;
  }).join("\n");

  const contextSection = sharedContext
    ? `\n## Accumulated Context From Previous Iterations\n${sharedContext}\n`
    : "";

  const dirSection = config.projectDir
    ? `\n## Working Directory\nAll file operations and commands must be performed under: ${config.projectDir}\nAlways instruct sub-agents to work within this directory.\n`
    : "";

  return `You are an Orchestrator AI.

## Goal
${config.goal}
${dirSection}${contextSection}
## Available Sub-Agents
${agentDescriptions}

## Instructions
1. Review the accumulated context above to understand what has already been done and what remains.
2. Delegate tasks to sub-agents using the call_agent tool. Avoid repeating work already completed.
3. When all sub-tasks are done, respond with a final summary WITHOUT calling any tools.
4. Always write your final response in ${languageDirective(locale)}.`;
}
```

Rewrite `buildVerifierSystemPrompt` to take `locale`:

```ts
function buildVerifierSystemPrompt(stoppingCondition: string, subAgentNames: string[], locale: Locale): string {
  const agentList = subAgentNames.length > 0
    ? subAgentNames.map(n => `"${n}"`).join(", ")
    : "(no agents available)";

  return `You are a Verifier AI. Your job is to objectively evaluate progress toward a goal.

## Stopping Condition
${stoppingCondition}

## Available Sub-Agents (ONLY use these names in your suggestion)
${agentList}

## Verification Tools
You have read-only tools (read_file, list_directory). Before concluding, you may
use them to inspect the actual files and verify the Orchestrator's claims.

## Instructions
Analyze the Orchestrator's report and respond ONLY with a JSON object in this exact format:
{
  "done": true or false,
  "summary": "one-sentence summary of overall progress so far",
  "accomplished": ["specific completed item 1", "specific completed item 2"],
  "remaining": ["specific incomplete item 1", "specific incomplete item 2"],
  "suggestion": "concrete next-step suggestion for the Orchestrator, using only the agent names listed above"
}

Rules:
- "accomplished" and "remaining" must be specific and verifiable, not vague
- "suggestion" MUST only reference agents from the available list above — never invent new agent names
- If done is true, "remaining" should be empty and "suggestion" can be empty
- Write all values in ${languageDirective(locale)}
- Do NOT include any text outside the JSON object`;
}
```

Rewrite `buildSharedContextUpdate`'s labels to English:

```ts
function buildSharedContextUpdate(iter: number, result: VerifierResult, subAgentSummaries: string[]): string {
  const lines = [`### Iteration #${iter} Result`];
  if (subAgentSummaries.length > 0) {
    lines.push("**Agent Execution Summary:**");
    subAgentSummaries.forEach(s => lines.push(`  - ${s}`));
  }
  if (result.accomplished.length > 0) {
    lines.push("**Accomplished:**");
    result.accomplished.forEach(a => lines.push(`  ✓ ${a}`));
  }
  if (result.remaining.length > 0) {
    lines.push("**Remaining:**");
    result.remaining.forEach(r => lines.push(`  ✗ ${r}`));
  }
  return lines.join("\n");
}
```

Inside `start()`, right after `abortRef.current = false;`, normalize the locale once for the whole run:

```ts
  const start = useCallback(async (config: LoopConfig) => {
    abortRef.current = false;
    const locale: Locale = config.locale ?? "zh-TW";
    const loopSessionId = config.loopSessionId ?? crypto.randomUUID();
```

Update the initial kickoff history to pass `locale` and use an English kickoff message:

```ts
    const orchestratorHistory: ChatMessage[] = config.resumeSnapshot?.orchestratorHistory ?? [
      { role: "system", content: buildOrchestratorSystemPrompt(config, sharedContext, locale) },
      { role: "user", content: `Begin working toward the goal: ${config.goal}` },
    ];
```

Update the per-iteration system-prompt refresh (in the `for (let iter = ...)` loop) to pass `locale`:

```ts
        // Update orchestrator system prompt with latest shared context each iteration
        if (iter > 1 && sharedContext) {
          orchestratorHistory[0] = {
            role: "system",
            content: buildOrchestratorSystemPrompt(config, sharedContext, locale),
          };
        }
```

Rewrite the preflight test messages to English:

```ts
      const agentNames = config.subAgents.map(a => a.name);
      const preflightMessages: ChatMessage[] = [
        { role: "system", content: `You are an Orchestrator AI. You must use the call_agent tool to delegate tasks. Available agents: ${agentNames.join(", ")}.` },
        { role: "user", content: `[Preflight test] Immediately call the call_agent tool, assigning any simple task to any available agent. Call the tool directly — do not output any explanatory text.` },
      ];
```

Fix the "looks like a plan, not a tool call" heuristic (currently Chinese-only) to also catch English planning language, and rewrite the correction prompt to English:

```ts
            const agentNames = config.subAgents.map(a => a.name);
            const looksLikePlan = agentNames.some(n => responseText.includes(n)) ||
              /應該|需要|建議|請|should|need to|will now|let me|plan to|call_agent|delegate|assign/i.test(responseText) ||
              responseText.includes("<tool_call>");

            if (looksLikePlan && noToolCallRetries < 2) {
              noToolCallRetries++;
              orchestratorHistory.push({ role: "assistant", content: responseText });
              orchestratorHistory.push({
                role: "user",
                content: `You just described a plan but did not call the call_agent tool. You MUST immediately use the call_agent tool to delegate a task to a sub-agent — do not output any more explanatory text. Available agents: ${agentNames.join(", ")}.`,
              });
```

Rewrite the "agent not found" tool-result error (fed back to the AI as a `tool` message) to English:

```ts
            let subResult: string;
            if (!targetAgent) {
              const available = config.subAgents.map(a => `"${a.name}"`).join(", ");
              subResult = `Error: agent "${args.agent_name}" does not exist in the roster. Available agents: ${available}. Please use one of these instead.`;
```

Pass `locale` into the `runSubAgent` options:

```ts
              const result = await runSubAgent(
                config.sessionId,
                targetAgent,
                args.task,
                {
                  onAction: (action) => addTraceBuffered({ kind: "sub_agent_action", agentName: args.agent_name, text: action.tool, actions: [action], iteration: iter }),
                  onConfirmNeeded: confirmFn,
                  maxInnerIterations: config.maxInnerIterations,
                  sharedContext,
                  projectDir: config.projectDir,
                  locale,
                },
              );
```

Rewrite the Verifier user message headers to English and pass `locale` to `buildVerifierSystemPrompt`:

```ts
        // Run Verifier with full context
        const verifierMessages: ChatMessage[] = [
          { role: "system", content: buildVerifierSystemPrompt(config.stoppingCondition, config.subAgents.map(a => a.name), locale) },
          {
            role: "user",
            content: [
              `## Goal\n${config.goal}`,
              sharedContext ? `## Accumulated Context From Previous Iterations\n${sharedContext}` : "",
              `## Orchestrator Report This Round (Iteration #${iter})\n${orchestratorFinalAnswer}`,
            ].filter(Boolean).join("\n\n"),
          },
        ];
```

Rewrite the "Verifier response unparseable" fallback message (fed back to the AI) to English:

```ts
          orchestratorHistory.push({
            role: "user",
            content: "The Verifier's response could not be parsed. Please continue trying to achieve the goal.",
          });
```

Rewrite the Verifier feedback block (fed back into `orchestratorHistory`) to English:

```ts
        // Feed structured feedback to orchestrator for next iteration
        const feedbackMsg = [
          `## Verifier Feedback (Iteration #${iter})`,
          `**Summary:** ${verifierResult.summary}`,
          verifierResult.accomplished.length > 0
            ? `**Accomplished:**\n${verifierResult.accomplished.map(a => `- ${a}`).join("\n")}`
            : "",
          verifierResult.remaining.length > 0
            ? `**Remaining:**\n${verifierResult.remaining.map(r => `- ${r}`).join("\n")}`
            : "",
          verifierResult.suggestion
            ? `**Next-step suggestion:** ${verifierResult.suggestion}`
            : "",
        ].filter(Boolean).join("\n\n");
```

Everything else in the file (all `addTraceBuffered({ ..., text: "..." })` calls, e.g. "🔍 正在測試...", "✓ 目標達成", "⚠ 偵測到重複迴圈...") is UI trace text only — it is never sent to the AI and stays as-is (out of scope, per the design spec).

- [ ] **Step 2: Run the full frontend test suite and type check**

Run: `npm run test && npx tsc --noEmit`
Expected: all tests PASS, no type errors (the rewritten functions keep the same call signatures used elsewhere except the added `locale` parameters, all of which are now supplied)

- [ ] **Step 3: Commit**

```bash
git add src/hooks/useOrchestratorLoop.ts
git commit -m "feat(loop-studio): orchestrator/verifier prompts follow locale, fix plan-detection heuristic for English"
```

---

## Task 10: Wire `locale` into `LoopConfig` from the LoopStudio UI

**Files:**
- Modify: `src/components/LoopStudio/index.tsx:115,209-245`

- [ ] **Step 1: Pass `useLocale().locale` into the config built by `handleStart`**

In `src/components/LoopStudio/index.tsx`, change line 115 to also destructure `locale`:

```ts
  const { t, locale } = useLocale();
```

Add `locale` to the `config` object inside `handleStart` and to its `useCallback` dependency array:

```ts
    const config: LoopConfig = {
      goal: roster.goal,
      stoppingCondition: roster.stoppingCondition || roster.goal,
      orchestrator,
      verifier,
      subAgents: roster.subAgents,
      maxLoops: roster.maxLoops,
      maxOrchestratorSteps: roster.maxOrchestratorSteps,
      maxInnerIterations: roster.maxInnerIterations,
      sessionId: ptySessionId,
      projectDir: roster.projectDir || undefined,
      fullAuto: roster.fullAuto ?? false,
      locale,
    };

    void loop.start(config);
  }, [loop, ptySessionId, roster, locale]);
```

- [ ] **Step 2: Run the full frontend test suite and type check**

Run: `npm run test && npx tsc --noEmit`
Expected: all tests PASS, no type errors

- [ ] **Step 3: Commit**

```bash
git add src/components/LoopStudio/index.tsx
git commit -m "feat(loop-studio): pass UI locale into LoopConfig when starting a run"
```

---

## Task 11: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: all tests PASS

- [ ] **Step 2: Run the full frontend test suite**

Run: `npm run test`
Expected: all tests PASS

- [ ] **Step 3: Type check**

Run: `npx tsc --noEmit`
Expected: no errors

- [ ] **Step 4: Lint**

Run: `npm run lint`
Expected: no errors

- [ ] **Step 5: Manual smoke test**

Run: `npm run tauri:dev`

1. Open Settings → General, switch language to **English**.
2. In a terminal tab, type `/ai list files in this directory` — confirm the returned explanation is in English.
3. Open the AI Panel (Ctrl+I), send a chat message — confirm the reply is in English.
4. Open a Design tab (SDD), send a message — confirm the reply is in English.
5. Open a Database tab or CrossDB tab with a connection configured, ask a question that needs a SQL query — confirm the reply is in English.
6. Open LoopStudio, configure a small goal with one sub-agent, run it — confirm the Orchestrator's final summary and the Sub-agent's report are in English.
7. Switch language back to **繁體中文** in Settings, repeat steps 2–4 — confirm all responses are back in Traditional Chinese (regression check).

- [ ] **Step 6: Report results to the user**

Summarize which steps passed / any deviations observed during the manual smoke test.

---

## Self-Review Notes

- **Spec coverage:** Task 1–2 cover the core mechanism; Task 3 covers `ai_query`; Task 4 covers `ai_chat` (and therefore `CrossDbAiChat`/`DatabaseAiChat`'s language-conflict fix, resolved in Task 7 by passing the same `locale`); Task 5 covers `design_chat` (including `build_stage_instruction`, discovered during planning as also being AI-bound content within the same command — included per the spec's "design_chat" scope); Tasks 8–10 cover LoopStudio Orchestrator/Sub-agent AI-bound content, including the English plan-detection regex fix flagged in the spec. UI trace strings and `vcs_query`/`vcs_agent_step` are explicitly left untouched, matching the spec's exclusions.
- **Type consistency:** `Locale` (Rust) / `Locale` (TS, from `lib/i18n.ts`) are the two sources of truth; every function signature introduced in one task (`build_single_command_prompt(snapshot, locale)`, `build_chat_prompt(snapshot, locale)`, `build_design_prompt(session, snapshot, locale)`, `buildOrchestratorSystemPrompt(config, sharedContext, locale)`, `buildVerifierSystemPrompt(stoppingCondition, names, locale)`, `runSubAgent(sessionId, agent, task, { locale })`) is used with matching argument order in every later task that calls it.
