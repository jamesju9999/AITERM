use tauri::State;
use crate::db::knowledge_base::{
    KnowledgeBaseDb, NotebookRow,
    list_notebooks, delete_notebook,
};

/// 建立筆記本。空字串的 provider/model 由 `create_notebook_verified` 擋掉，
/// 這裡只把「整個欄位沒給」翻成看得懂的訊息（維持既有的 IPC 形狀，
/// 也比 tauri 反序列化失敗的訊息好讀）。
#[tauri::command]
pub async fn kb_create_notebook(
    name: String,
    folder_path: String,
    embed_provider_id: Option<String>,
    embed_model: Option<String>,
    db: State<'_, KnowledgeBaseDb>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<NotebookRow, String> {
    let provider_id = embed_provider_id
        .ok_or_else(|| "請選擇 embedding provider".to_string())?;
    let model = embed_model
        .ok_or_else(|| "請填寫 embedding model".to_string())?;

    let mut cfg = resolve_embedder_config(&config, &secrets, &provider_id)?;
    cfg.model = model.clone();
    let embedder = HttpEmbedder::new(cfg)?;

    crate::knowledge_base::notebook::create_notebook_verified(
        &db.pool, &name, &folder_path, &provider_id, &model, &embedder,
    ).await
}

#[tauri::command]
pub async fn kb_list_notebooks(db: State<'_, KnowledgeBaseDb>) -> Result<Vec<NotebookRow>, String> {
    list_notebooks(&db.pool).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_delete_notebook(id: String, db: State<'_, KnowledgeBaseDb>) -> Result<(), String> {
    delete_notebook(&db.pool, &id).await.map_err(|e| e.to_string())
}

use std::path::Path;
use std::sync::Arc;
use async_trait::async_trait;
use tauri::{AppHandle, Emitter};
use serde::Serialize;

use crate::config::ConfigStore;
use crate::secret::SecretStore;
use crate::db::knowledge_base as kb_db;
use crate::db::kb_chat_sessions::{self, ChatSessionSummary, ChatMessageRow};
use crate::knowledge_base::embedding::{
    embedding_api, Embedder, EmbedderConfig, EmbeddingApi, HttpEmbedder,
};
use crate::knowledge_base::ingest::{sync_notebook, DocumentConverter, SyncProgress, SyncSummary};

struct RoutedConverter {
    app: AppHandle,
    vision_provider_id: Option<String>,
}

#[async_trait]
impl DocumentConverter for RoutedConverter {
    async fn convert(&self, path: &Path) -> Result<String, String> {
        crate::document_convert::convert_document(
            self.app.clone(),
            path,
            self.vision_provider_id.clone(),
        ).await
    }
}

fn resolve_embedder_config(
    config: &ConfigStore,
    secrets: &SecretStore,
    provider_id: &str,
) -> Result<EmbedderConfig, String> {
    let cfg = config.get_provider(provider_id)
        .ok_or_else(|| format!("找不到 provider: {provider_id}"))?;
    // 空字串的金鑰要當成沒有：留著會送出 `Authorization: Bearer `，換來一個
    // 看起來像「列模型壞掉」的 401。同 provider.rs 既有寫法。
    let api_key = secrets.get(provider_id).ok().flatten().filter(|v| !v.trim().is_empty());

    let base_url = match embedding_api(cfg.provider_type)? {
        EmbeddingApi::Ollama => cfg.base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
        EmbeddingApi::OpenAi => cfg.base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        EmbeddingApi::OpenAiCompatible => cfg.base_url
            .ok_or_else(|| "OpenAI 相容 provider 缺少 base_url".to_string())?,
    };

    Ok(EmbedderConfig {
        provider_type: cfg.provider_type,
        base_url,
        api_key,
        model: cfg.model,
    })
}

/// 列出某個 provider 可用的模型，給新增筆記本時的下拉選單用。
#[tauri::command]
pub async fn kb_list_embedding_models(
    provider_id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let cfg = resolve_embedder_config(&config, &secrets, &provider_id)?;
    crate::knowledge_base::embedding::list_models(&cfg).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum KbSyncEvent {
    Progress {
        notebook_id: String,
        processed: usize,
        total: usize,
        current_file: String,
    },
    Done {
        notebook_id: String,
        indexed: usize,
        failed: usize,
        deleted: usize,
    },
}

#[tauri::command]
pub async fn kb_sync_notebook(
    notebook_id: String,
    app: AppHandle,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
    config: tauri::State<'_, Arc<ConfigStore>>,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<SyncSummary, String> {
    let notebook = kb_db::get_notebook(&db.pool, &notebook_id)
        .await.map_err(|_| format!("找不到筆記本: {notebook_id}"))?;

    let provider_id = notebook.embed_provider_id.clone()
        .ok_or_else(|| "此筆記本尚未設定 embedding provider".to_string())?;
    let model = notebook.embed_model.clone()
        .ok_or_else(|| "此筆記本尚未設定 embedding model".to_string())?;

    let mut embedder_cfg = resolve_embedder_config(&config, &secrets, &provider_id)?;
    embedder_cfg.model = model;
    let embedder = HttpEmbedder::new(embedder_cfg)?;

    let converter = RoutedConverter {
        app: app.clone(),
        vision_provider_id: Some(provider_id),
    };

    let app_for_progress = app.clone();
    let nb_id_for_progress = notebook_id.clone();

    let summary = sync_notebook(
        &db.pool,
        &notebook,
        Arc::new(converter),
        Arc::new(embedder),
        move |progress: SyncProgress| {
            let _ = app_for_progress.emit("kb-sync-event", KbSyncEvent::Progress {
                notebook_id: nb_id_for_progress.clone(),
                processed: progress.processed,
                total: progress.total,
                current_file: progress.current_file,
            });
        },
    ).await?;

    let now = chrono::Utc::now().timestamp();
    if let Err(e) = kb_db::mark_synced(&db.pool, &notebook_id, now).await {
        eprintln!("Warning: failed to update last_synced_at for notebook {notebook_id}: {e}");
    }

    let _ = app.emit("kb-sync-event", KbSyncEvent::Done {
        notebook_id: notebook_id.clone(),
        indexed: summary.indexed,
        failed: summary.failed,
        deleted: summary.deleted,
    });

    Ok(summary)
}

use crate::ai::{router::AiRouter, ChatMessage, Locale};

#[tauri::command]
pub async fn kb_chat(
    notebook_id: String,
    messages: Vec<ChatMessage>,
    session_id: String,
    chat_session_id: String,
    provider_id: Option<String>,
    locale: Locale,
    app: AppHandle,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
    config: tauri::State<'_, Arc<ConfigStore>>,
    secrets: tauri::State<'_, Arc<SecretStore>>,
    router: tauri::State<'_, AiRouter>,
) -> Result<(), crate::ai::AiError> {
    use crate::ai::AiError;

    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }

    let notebook = kb_db::get_notebook(&db.pool, &notebook_id)
        .await.map_err(|_| AiError::InvalidInput { reason: format!("找不到筆記本: {notebook_id}") })?;

    let embed_provider_id = notebook.embed_provider_id.clone()
        .ok_or_else(|| AiError::InvalidInput { reason: "此筆記本尚未設定 embedding provider".into() })?;
    let embed_model = notebook.embed_model.clone()
        .ok_or_else(|| AiError::InvalidInput { reason: "此筆記本尚未設定 embedding model".into() })?;

    let mut embedder_cfg = resolve_embedder_config(&config, &secrets, &embed_provider_id)
        .map_err(|reason| AiError::InvalidInput { reason })?;
    embedder_cfg.model = embed_model;
    let embedder: Arc<dyn Embedder> = Arc::new(
        HttpEmbedder::new(embedder_cfg).map_err(|message| AiError::Network { message })?,
    );

    let chat_provider = match provider_id.as_deref() {
        Some(id) => router.resolve_by_id(id).await?,
        None => router.resolve().await?,
    };

    crate::knowledge_base::chat::run_chat(
        db.pool.clone(), notebook, messages, chat_provider, embedder, session_id, chat_session_id, locale, app,
    ).await
}

#[tauri::command]
pub async fn kb_create_chat_session(
    notebook_id: String,
    title: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<String, String> {
    kb_chat_sessions::create_chat_session(&db.pool, &notebook_id, &title)
        .await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_list_chat_sessions(
    notebook_id: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<Vec<ChatSessionSummary>, String> {
    kb_chat_sessions::list_chat_sessions(&db.pool, &notebook_id)
        .await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_load_chat_session(
    session_id: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<Vec<ChatMessageRow>, String> {
    kb_chat_sessions::load_chat_session_messages(&db.pool, &session_id)
        .await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_delete_chat_session(
    session_id: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<(), String> {
    kb_chat_sessions::delete_chat_session(&db.pool, &session_id)
        .await.map_err(|e| e.to_string())
}

/// 開啟筆記本資料夾內的某份文件（OS 預設應用程式）。
/// rel_path 來自工具呼叫結果（AI 影響的內容），開啟前一定要做邊界檢查，
/// 避免解析到筆記本資料夾以外的路徑。
#[tauri::command]
pub async fn kb_open_document(
    notebook_id: String,
    rel_path: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<(), String> {
    let notebook = kb_db::get_notebook(&db.pool, &notebook_id)
        .await.map_err(|e| e.to_string())?;

    let root = std::path::Path::new(&notebook.folder_path);
    let canonical_root = root.canonicalize()
        .map_err(|e| format!("Cannot resolve notebook folder: {e}"))?;
    let target = root.join(rel_path.trim_start_matches('/'));
    let canonical_target = target.canonicalize()
        .map_err(|e| format!("File not found: {e}"))?;

    if !canonical_target.starts_with(&canonical_root) {
        return Err("Path is outside the notebook folder".into());
    }

    open::that(canonical_target).map_err(|e| e.to_string())
}

#[cfg(test)]
mod resolve_embedder_config_tests {
    use super::*;
    use crate::config::{AppConfig, ProviderConfig, ProviderType};

    /// 只有 base_url 這條路徑會因為 provider 類型而不同，其餘欄位固定。
    fn store_with(provider_type: ProviderType, base_url: Option<&str>) -> ConfigStore {
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "p".into(),
            display_name: "P".into(),
            provider_type,
            base_url: base_url.map(str::to_string),
            oauth_client_id: None,
            model: "m".into(),
            supports_json_mode: true,
            auth_method: None,
        });
        ConfigStore::from_config(cfg)
    }

    /// keychain 裡不會有 "p" 這個 id，所以 api_key 一律是 None，不碰使用者的金鑰。
    fn resolve(config: &ConfigStore) -> Result<EmbedderConfig, String> {
        resolve_embedder_config(config, &SecretStore::new(), "p")
    }

    #[test]
    fn each_provider_type_gets_its_own_base_url_rule() {
        assert_eq!(
            resolve(&store_with(ProviderType::Ollama, None)).unwrap().base_url,
            "http://localhost:11434",
        );
        assert_eq!(
            resolve(&store_with(ProviderType::Openai, None)).unwrap().base_url,
            "https://api.openai.com/v1",
        );
        // OpenAI 相容沒有可猜的預設，缺 base_url 要當成設定錯誤而不是連到 openai.com。
        assert!(
            resolve(&store_with(ProviderType::OpenaiCompatible, None))
                .unwrap_err()
                .contains("缺少 base_url"),
        );
        // 有填就照填的走，三種類型都一樣。
        for provider_type in [ProviderType::Ollama, ProviderType::Openai, ProviderType::OpenaiCompatible] {
            assert_eq!(
                resolve(&store_with(provider_type, Some("http://host/v1"))).unwrap().base_url,
                "http://host/v1",
            );
        }
    }

    #[test]
    fn provider_types_without_embedding_are_rejected() {
        let err = resolve(&store_with(ProviderType::Anthropic, None)).unwrap_err();
        assert!(err.contains("不支援 embedding"), "unexpected error: {err}");
    }
}
