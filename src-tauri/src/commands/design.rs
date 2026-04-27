// src-tauri/src/commands/design.rs
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use crate::db::design::{DesignDb, create_design_session, get_design_session, DesignSession};
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
        "SELECT id, title, current_spec_draft, current_sdd_draft, current_plan_draft, context_summary, status FROM design_sessions ORDER BY updated_at DESC"
    )
    .fetch_all(&design_db.pool)
    .await
    .map_err(|e| e.to_string())
}

pub fn build_design_prompt(session: &DesignSession, snapshot: &crate::ai::EnvSnapshot) -> String {
    let spec = session.current_spec_draft.as_deref().unwrap_or("尚未建立規格。");
    let sdd = session.current_sdd_draft.as_deref().unwrap_or("尚未建立系統設計。");
    let plan = session.current_plan_draft.as_deref().unwrap_or("尚未建立實作計畫。");
    let summary = session.context_summary.as_deref().unwrap_or("尚無對話摘要。");

    let role_instruction = match session.status.as_str() {
        "draft" => r#"你現在的角色是「產品經理 (Product Manager)」。
你的任務：透過提問來釐清使用者意圖、確認邊界條件與驗收標準。
目標：完善右側的「規格 (Spec)」文件。
限制：在規格被核准前，請勿討論具體的程式碼實作或模組細節。"#,
        "spec_approved" => r#"你現在的角色是「軟體架構師 (Software Architect)」。
你的任務：基於「已核准的規格」，進行技術選型、模組劃分、API 設計與資料庫 Schema 設計。
目標：完善右側的「架構 (SDD)」文件。
SDD 核心守則：
1. 你的所有架構決策必須「100% 追溯」回已核准的規格。
2. 在撰寫設計文件時，請儘可能註註明「(對應規格第 X 點)」。
3. 嚴禁在設計中加入任何規格書中未提及的功能或擴充性，保持設計的精簡與精確。"#,
        "sdd_approved" => r#"你現在的角色是「技術主管 (Tech Lead)」。
你的任務：將「已核准的架構與設計」拆解為具體任務。
目標：完善右側的「計畫 (Plan)」文件。
SDD 核心守則：
1. 每個 Task 必須與 SDD 中的某個組件或介面對齊。
2. Task 的驗收標準必須與原規格書的驗收標準一致。"#,
        _ => "你是一位專業的軟體工程專家。"
    };

    format!(
r#"你是一位專業的軟體需求分析師與架構師，正在協助使用者進行「規格驅動開發 (SDD)」。

目前進度階段：{status_label}
{role_instruction}

目前的專案摘要：
{summary}

目前右側面板的內容：
---
[規格 (Spec)]
{spec}

[架構 (SDD)]
{sdd}

[計畫 (Plan)]
{plan}
---

目前的終端機環境：
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}

規則：
1. 請以「繁體中文 (zh-TW)」進行對話。
2. 你的目標是透過提問來釐清需求，並逐步完善右側的規格與設計。
3. 當你決定更新或建立規格、架構或計畫時，請務必將內容完整地放在對應的標籤塊中：
   - 規格請用：[UPDATE_SPEC] ```markdown ...內容... ```
   - 架構請用：[UPDATE_SDD] ```markdown ...內容... ```
   - 計畫請用：[UPDATE_PLAN] ```markdown ...內容... ```
4. 【極度重要】你欲寫入文件的「所有內容」（特別是 Mermaid 架構圖！）都必須嚴格放在上述的標籤塊內部。寫在標籤塊外部的內容不會被保存進文件，右側面板將無法顯示圖表！
5. 【Mermaid 語法注意】在繪製 Mermaid 架構圖時，節點名稱如果包含括號或特殊字元，必須使用雙引號包覆，例如：`UI["前端介面 (React/Vue)"]`，嚴禁使用 `UI[前端介面 (React/Vue)]`，否則會導致語法解析錯誤！
6. 即使你使用了標籤更新草稿，仍然要在對話中向使用者說明你做了哪些變動。"#,
        status_label = match session.status.as_str() {
            "draft" => "1. 需求探索中",
            "spec_approved" => "2. 系統設計中",
            "sdd_approved" => "3. 任務規劃中",
            _ => "已完成"
        },
        role_instruction = role_instruction,
        summary = summary,
        spec = spec,
        sdd = sdd,
        plan = plan,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display(),
    )
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
        content: m.content,
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

    // 2. Get environment snapshot
    let snapshot = context::snapshot(&pty_manager, &session_id);
    
    // 3. Resolve AI provider (allow manual override via provider_id)
    let provider = match provider_id.as_deref() {
        Some(id) => router.resolve_by_id(id).await?,
        None => router.resolve().await?,
    };

    // 4. Build specialized design prompt
    let prompt = build_design_prompt(&session, &snapshot);

    let req = GenerateRequest {
        system_prompt: prompt,
        messages: messages.clone(),
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: Some(2048),
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
        let _ = crate::db::design::create_design_message(&design_db.pool, &session_id, &last_user_msg.role, &last_user_msg.content).await;
    }
    let _ = crate::db::design::create_design_message(&design_db.pool, &session_id, "assistant", &buf).await;

    Ok(AiChatReply { content: buf })
}