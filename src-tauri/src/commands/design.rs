// src-tauri/src/commands/design.rs
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use crate::db::design::{DesignDb, create_design_session, get_design_session, delete_design_session, DesignSession};
use crate::ai::{
    context, router::AiRouter, AiError, ChatMessage, GenerateChunk,
    GenerateRequest, Locale, QueryMode,
};
use crate::commands::ai::{AiChatReply, AiStreamEvent, AiStreamKind};
use crate::pty::PtyManager;

#[tauri::command]
pub async fn design_start_session(
    design_db: State<'_, DesignDb>,
    title: String,
) -> Result<String, String> {
    create_design_session(&design_db.pool, &title)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn design_load_session(
    design_db: State<'_, DesignDb>,
    id: String,
) -> Result<DesignSession, String> {
    get_design_session(&design_db.pool, &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn design_list_sessions(
    design_db: State<'_, DesignDb>,
) -> Result<Vec<DesignSession>, String> {
    sqlx::query_as::<_, DesignSession>(
        "SELECT id, title, current_proposal_draft, current_spec_draft, current_sdd_draft, current_plan_draft, context_summary, status FROM design_sessions ORDER BY updated_at DESC"
    )
    .fetch_all(&design_db.pool)
    .await
    .map_err(|e| e.to_string())
}

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

#[tauri::command]
pub async fn design_delete_session(
    design_db: State<'_, DesignDb>,
    session_id: String,
) -> Result<bool, String> {
    delete_design_session(&design_db.pool, &session_id)
        .await
        .map(|_| true)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn design_advance_stage(
    design_db: State<'_, DesignDb>,
    session_id: String,
    next_status: String,
) -> Result<bool, String> {
    sqlx::query("UPDATE design_sessions SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&next_status)
        .bind(&session_id)
        .execute(&design_db.pool)
        .await
        .map(|_| true)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn design_list_messages(
    design_db: State<'_, DesignDb>,
    session_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let messages = crate::db::design::get_design_messages(&design_db.pool, &session_id)
        .await
        .map_err(|e| e.to_string())?;
    
    Ok(messages.into_iter().map(|m| ChatMessage {
        role: m.role,
        content: serde_json::Value::String(m.content),
        tool_call_id: None,
        tool_calls: None,
    }).collect())
}

#[tauri::command]
pub async fn design_update_draft(
    design_db: State<'_, DesignDb>,
    session_id: String,
    field: String,
    content: String,
) -> Result<bool, String> {
    crate::db::design::update_design_draft(&design_db.pool, &session_id, &field, &content)
        .await
        .map(|_| true)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn design_save_file(
    file_path: String,
    content: String,
) -> Result<bool, String> {
    if let Some(parent) = std::path::Path::new(&file_path).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    tokio::fs::write(&file_path, content)
        .await
        .map(|_| true)
        .map_err(|e| format!("Failed to save file: {}", e))
}

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
    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }

    // 1. Load session context from DB
    let session = get_design_session(&design_db.pool, &session_id)
        .await
        .map_err(|e| AiError::ModelError { reason: e.to_string(), raw: String::new() })?;

    // Auto-update title from first user message if still default
    if session.title == "新需求討論" {
        if let Some(first_user) = messages.iter().find(|m| m.role == "user") {
            let content_str = first_user.content.as_str().unwrap_or("").trim();
            // Skip auto-generated [GENERATE:xxx] messages
            if !content_str.starts_with("請根據目前的討論內容產生") {
                let new_title: String = content_str.chars().take(30).collect();
                let _ = sqlx::query("UPDATE design_sessions SET title = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
                    .bind(&new_title)
                    .bind(&session_id)
                    .execute(&design_db.pool)
                    .await;
            }
        }
    }

    // 2. Get environment snapshot
    let snapshot = context::snapshot(&pty_manager, &session_id);
    
    // 3. Resolve AI provider (allow manual override via provider_id)
    let provider = match provider_id.as_deref() {
        Some(id) => router.resolve_by_id(id).await?,
        None => router.resolve().await?,
    };

    // 4. Build specialized design prompt, with optional stage instruction injection
    let base_prompt = build_design_prompt(&session, &snapshot, locale);
    let last_content_owned: String = messages.last()
        .and_then(|m| m.content.as_str())
        .unwrap_or("")
        .to_owned();
    let last_content = last_content_owned.as_str();
    let stage_inject = if let Some(start) = last_content.find("[GENERATE:") {
        let rest = &last_content[start + 10..];
        if let Some(end) = rest.find(']') {
            let stage = &rest[..end];
            build_stage_instruction(stage)
        } else { "" }
    } else { "" };
    let prompt = if stage_inject.is_empty() {
        base_prompt
    } else {
        format!("{base_prompt}\n\n{stage_inject}")
    };

    let req = GenerateRequest {
        system_prompt: prompt,
        messages: messages.clone(),
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: None,
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        // We use kind: Chat for now, but the frontend will know it's from the design tab via session_id
        let _ = app.emit("ai-stream", AiStreamEvent {
            session_id: session_id.clone(),
            kind: AiStreamKind::Chat,
            delta: chunk.delta.clone(),
            done: chunk.done,
        });
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    // Persist history: save the user's last message and the assistant's reply
    if let Some(last_user_msg) = messages.last() {
        let content_str = last_user_msg.content.as_str().unwrap_or("");
        let _ = crate::db::design::create_design_message(&design_db.pool, &session_id, &last_user_msg.role, content_str).await;
    }
    let _ = crate::db::design::create_design_message(&design_db.pool, &session_id, "assistant", &buf).await;

    Ok(AiChatReply { content: Some(buf), tool_calls: vec![], tool_calling_unsupported: false, raw_tool_calls: None })
}

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
