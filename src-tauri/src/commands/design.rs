// src-tauri/src/commands/design.rs
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use crate::db::design::{DesignDb, create_design_session, get_design_session, delete_design_session, DesignSession};
use crate::ai::{
    context, router::AiRouter, AiError, ChatMessage, GenerateChunk,
    GenerateRequest, QueryMode,
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

/// Returns a focused instruction block when the user clicks "▶ 產生" for a specific stage.
/// This is injected into the AI prompt to ensure strict adherence to OpenSpec templates.
pub fn build_stage_instruction(stage: &str) -> &'static str {
    match stage {
        "proposal" => r#"【階段指令：產生提案 (Proposal)】
請嚴格依照以下 OpenSpec 結構產出提案，並用 [UPDATE_PROPOSAL] 標籤包覆完整內容：

[UPDATE_PROPOSAL]
## Why
（問題陳述：為什麼需要這個變更？要解決什麼問題？）

## What Changes
（具體變更描述：要新增、修改、移除什麼？）

## Capabilities
### New
- capability-name — 簡述此新功能
### Modified
- existing-capability — 變更說明

## Impact
（對程式碼、API、資料庫、依賴套件的影響範圍）
[/UPDATE_PROPOSAL]

請根據目前的對話內容產出完整提案。"#,
        "spec" => r#"【階段指令：產生規格 (Spec)】
請嚴格依照以下 OpenSpec 結構產出規格，並用 [UPDATE_SPEC] 標籤包覆完整內容。
為提案中的每個 Capability 各建一個章節：

[UPDATE_SPEC]
## Capability: capability-name

### Requirement: 需求名稱
需求描述文字

#### Scenario: 場景名稱
WHEN（觸發條件）
THEN（預期結果）

#### Scenario: 另一個場景
WHEN ...
THEN ...

（重複上述結構，每個 Capability 一個 ## 區塊）
[/UPDATE_SPEC]

重要：
- 使用 delta 標記（ADDED / MODIFIED / REMOVED）標註需求狀態
- Scenario 必須使用四級標題 ####，使用 WHEN/THEN 格式
- 請根據提案的 Capabilities 區塊逐一產出"#,
        "sdd" => r#"【階段指令：產生設計 (Design)】
請嚴格依照以下 OpenSpec 結構產出技術設計，並用 [UPDATE_SDD] 標籤包覆完整內容：

[UPDATE_SDD]
## Context
（背景與現狀：目前系統的相關架構與限制）

## Goals / Non-Goals
### Goals
- ...
### Non-Goals
- ...

## Decisions
（關鍵技術決策及其理由）

## Risks / Trade-offs
（已知風險與設計取捨）
[/UPDATE_SDD]

重要：
- 所有決策必須可追溯回已核准的規格
- 包含 Mermaid 架構圖時，節點名稱含括號或特殊字元須用雙引號包覆
- `end` 只配對 `subgraph`，`graph TD` 不需要 `end`"#,
        "plan" => r#"【階段指令：產生任務 (Tasks)】
請嚴格依照以下 OpenSpec 結構產出任務清單，並用 [UPDATE_PLAN] 標籤包覆完整內容：

[UPDATE_PLAN]
## 1. 第一組任務名稱
- [ ] 1.1 任務描述
- [ ] 1.2 任務描述

## 2. 第二組任務名稱
- [ ] 2.1 任務描述
- [ ] 2.2 任務描述
[/UPDATE_PLAN]

重要：
- 每個任務必須是 checkbox 格式：`- [ ] X.Y 描述`
- 按依賴順序分組編號
- 每個任務必須對齊設計文件中的某個組件或介面
- 任務的驗收標準必須與原規格的場景一致"#,
        _ => "",
    }
}

pub fn build_design_prompt(session: &DesignSession, snapshot: &crate::ai::EnvSnapshot) -> String {
    let proposal = session.current_proposal_draft.as_deref().unwrap_or("尚未建立提案。");
    let spec = session.current_spec_draft.as_deref().unwrap_or("尚未建立規格。");
    let sdd = session.current_sdd_draft.as_deref().unwrap_or("尚未建立設計。");
    let plan = session.current_plan_draft.as_deref().unwrap_or("尚未建立任務。");
    let summary = session.context_summary.as_deref().unwrap_or("尚無對話摘要。");

    let role_instruction = match session.status.as_str() {
        "draft" => r#"你現在的角色是「產品經理 (Product Manager)」。
你的任務：透過提問來釐清使用者意圖，並協助建立提案 (Proposal)。
目標：完善右側的「提案 (Proposal)」文件——釐清 Why、What Changes、Capabilities、Impact。
限制：在提案被核准前，請勿跳到規格或技術設計。"#,
        "proposal_approved" => r#"你現在的角色是「產品經理 (Product Manager)」。
你的任務：基於「已核准的提案」，為每個 Capability 定義詳細需求與驗收場景。
目標：完善右側的「規格 (Spec)」文件。
核心守則：
1. 每個 Capability 必須有明確的需求描述與 WHEN/THEN 場景。
2. 使用 delta 標記（ADDED / MODIFIED / REMOVED）標註需求狀態。
3. 嚴禁加入提案未提及的功能。"#,
        "spec_approved" => r#"你現在的角色是「軟體架構師 (Software Architect)」。
你的任務：基於「已核准的規格」，進行技術選型、模組劃分、API 設計與資料庫 Schema 設計。
目標：完善右側的「設計 (Design)」文件。
核心守則：
1. 所有架構決策必須 100% 追溯回已核准的規格。
2. 撰寫時儘可能註明「(對應規格 Capability: X)」。
3. 嚴禁在設計中加入規格未提及的功能或擴充性。"#,
        "sdd_approved" => r#"你現在的角色是「技術主管 (Tech Lead)」。
你的任務：將「已核准的設計」拆解為具體的 checkbox 任務清單。
目標：完善右側的「任務 (Tasks)」文件。
核心守則：
1. 每個 Task 必須與設計文件中的某個組件或介面對齊。
2. Task 的驗收標準必須與原規格的場景一致。
3. 每個任務必須是 `- [ ] X.Y 描述` 格式。"#,
        _ => "你是一位專業的軟體工程專家。",
    };

    format!(
r#"你是一位專業的軟體需求分析師與架構師，正在協助使用者進行「規格驅動開發 (Spec-Driven Development)」，遵循 OpenSpec 框架。

目前進度階段：{status_label}
{role_instruction}

目前的專案摘要：
{summary}

目前右側面板的內容：
---
[提案 (Proposal)]
{proposal}

[規格 (Spec)]
{spec}

[設計 (Design)]
{sdd}

[任務 (Tasks)]
{plan}
---

目前的終端機環境：
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}

規則：
1. 請以「繁體中文 (zh-TW)」進行對話。
2. 你的目標是透過提問來釐清需求，並逐步完善右側的文件。
3. 當你決定更新或建立文件時，請務必將內容完整地放在對應的標籤塊中：
   - 提案請用：[UPDATE_PROPOSAL] ```markdown ...內容... ```
   - 規格請用：[UPDATE_SPEC] ```markdown ...內容... ```
   - 設計請用：[UPDATE_SDD] ```markdown ...內容... ```
   - 任務請用：[UPDATE_PLAN] ```markdown ...內容... ```
4. 【極度重要】你欲寫入文件的「所有內容」（特別是 Mermaid 架構圖！）都必須嚴格放在上述的標籤塊內部。寫在標籤塊外部的內容不會被保存進文件，右側面板將無法顯示圖表！
5. 【Mermaid 語法注意】在繪製 Mermaid 架構圖時，節點名稱如果包含括號或特殊字元，必須使用雙引號包覆，例如：`UI["前端介面 (React/Vue)"]`，嚴禁使用 `UI[前端介面 (React/Vue)]`，否則會導致語法解析錯誤！
6. 【Mermaid end 配對】`end` 關鍵字只能用來關閉 `subgraph` 區塊，且必須一對一配對。`graph TD` 本身不需要 `end`。連線語句（如 `A --> B`）放在所有 subgraph 之後、不要再加多餘的 `end`。
7. 即使你使用了標籤更新草稿，仍然要在對話中向使用者說明你做了哪些變動。"#,
        status_label = match session.status.as_str() {
            "draft" => "1. 提案探索中 (Proposal)",
            "proposal_approved" => "2. 規格定義中 (Spec)",
            "spec_approved" => "3. 技術設計中 (Design)",
            "sdd_approved" => "4. 任務規劃中 (Tasks)",
            _ => "已完成"
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
    let base_prompt = build_design_prompt(&session, &snapshot);
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

    Ok(AiChatReply { content: Some(buf), tool_calls: vec![], tool_calling_unsupported: false })
}