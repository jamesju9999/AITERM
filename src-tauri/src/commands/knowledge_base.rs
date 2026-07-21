use tauri::State;
use crate::db::knowledge_base::{
    KnowledgeBaseDb, NotebookRow,
    create_notebook, list_notebooks, delete_notebook,
};

#[tauri::command]
pub async fn kb_create_notebook(
    name: String,
    folder_path: String,
    embed_provider_id: Option<String>,
    embed_model: Option<String>,
    db: State<'_, KnowledgeBaseDb>,
) -> Result<NotebookRow, String> {
    create_notebook(
        &db.pool, &name, &folder_path,
        embed_provider_id.as_deref(), embed_model.as_deref(),
    ).await.map_err(|e| e.to_string())
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
use tauri::{AppHandle, Emitter, Manager};
use serde::Serialize;

use crate::config::{ConfigStore, ProviderType};
use crate::secret::SecretStore;
use crate::db::knowledge_base as kb_db;
use crate::knowledge_base::embedding::{Embedder, EmbedderConfig, HttpEmbedder};
use crate::knowledge_base::ingest::{sync_notebook, DocumentConverter, SyncProgress, SyncSummary};

struct MarkItDownConverter {
    app: AppHandle,
    vision_provider_id: Option<String>,
}

#[async_trait]
impl DocumentConverter for MarkItDownConverter {
    async fn convert(&self, path: &Path) -> Result<String, String> {
        let config = self.app.state::<Arc<ConfigStore>>();
        let secrets = self.app.state::<Arc<SecretStore>>();
        crate::commands::markitdown::markitdown_convert(
            self.app.clone(),
            path.to_string_lossy().to_string(),
            self.vision_provider_id.clone(),
            config,
            secrets,
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
    let api_key = secrets.get(provider_id).ok().flatten();

    let base_url = match cfg.provider_type {
        ProviderType::Ollama => cfg.base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
        ProviderType::Openai => cfg.base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        ProviderType::OpenaiCompatible => cfg.base_url
            .ok_or_else(|| "OpenAI 相容 provider 缺少 base_url".to_string())?,
        other => return Err(format!("{other} 不支援 embedding")),
    };

    Ok(EmbedderConfig {
        provider_type: cfg.provider_type,
        base_url,
        api_key,
        model: cfg.model,
    })
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
        .await.map_err(|e| e.to_string())?;

    let provider_id = notebook.embed_provider_id.clone()
        .ok_or_else(|| "此筆記本尚未設定 embedding provider".to_string())?;
    let model = notebook.embed_model.clone()
        .ok_or_else(|| "此筆記本尚未設定 embedding model".to_string())?;

    let mut embedder_cfg = resolve_embedder_config(&config, &secrets, &provider_id)?;
    embedder_cfg.model = model;
    let embedder = HttpEmbedder::new(embedder_cfg);

    let converter = MarkItDownConverter {
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
    kb_db::mark_synced(&db.pool, &notebook_id, now).await.map_err(|e| e.to_string())?;

    let _ = app.emit("kb-sync-event", KbSyncEvent::Done {
        notebook_id: notebook_id.clone(),
        indexed: summary.indexed,
        failed: summary.failed,
        deleted: summary.deleted,
    });

    Ok(summary)
}
