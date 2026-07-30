// src-tauri/src/commands/markitdown.rs
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncBufReadExt;
use serde::Deserialize;
use crate::config::{ConfigStore, ProviderType};
use crate::secret::SecretStore;

fn converter_script_path(app: &AppHandle) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest_dir
        .parent()
        .unwrap_or(&manifest_dir)
        .join("tools")
        .join("MarkItDown")
        .join("converter.py");
    if dev_path.exists() {
        return dev_path;
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let prod_path = resource_dir.join("MarkItDown").join("converter.py");
        if prod_path.exists() {
            return prod_path;
        }
    }
    dev_path
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PythonLine {
    Done { markdown: String },
    Error { message: String },
}

/// Resolve LLM credentials for image vision from the configured provider.
/// Returns (provider_type_str, api_key, base_url, model) or None if unavailable.
fn resolve_vision_credentials(
    config: &ConfigStore,
    secrets: &SecretStore,
    provider_id: &str,
) -> Option<(String, String, String, String)> {
    let cfg = config.get_provider(provider_id)?;
    let api_key = secrets.get(provider_id).ok().flatten().unwrap_or_default();

    let (provider_type_str, base_url) = match cfg.provider_type {
        ProviderType::Openai => (
            "openai".to_string(),
            cfg.base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        ),
        ProviderType::Anthropic => (
            "anthropic".to_string(),
            cfg.base_url.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
        ),
        ProviderType::Ollama => (
            "ollama".to_string(),
            cfg.base_url.unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
        ),
        ProviderType::OpenaiCompatible => (
            "openai-compatible".to_string(),
            cfg.base_url.unwrap_or_default(),
        ),
        // GitHub Copilot and Google AI have complex OAuth/auth flows — skip for now
        _ => return None,
    };

    Some((provider_type_str, api_key, base_url, cfg.model))
}

/// Convert a local file to Markdown using MarkItDown.
/// Auto-installs Python deps on first use (fast no-op if already installed).
/// `provider_id` is used for image vision (passes AI credentials to converter.py).
#[tauri::command]
pub async fn markitdown_convert(
    app: AppHandle,
    file_path: String,
    provider_id: Option<String>,
    config: tauri::State<'_, Arc<ConfigStore>>,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let script = converter_script_path(&app);
    let python = crate::python_env::ensure(&app, crate::python_env::profiles::Profile::DocCore)
        .await
        .map_err(String::from)?;
    let script_dir = script.parent().unwrap_or(script.as_path());

    // Resolve vision credentials (if a provider is selected)
    let vision_creds = provider_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .and_then(|id| resolve_vision_credentials(&config, &secrets, id));

    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(&script)
        .arg(&file_path)
        .current_dir(script_dir)
        .env("PYTHONIOENCODING", "utf-8")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Pass vision credentials as environment variables
    if let Some((provider_type_str, api_key, base_url, model)) = vision_creds {
        cmd.env("MARKITDOWN_LLM_PROVIDER_TYPE", provider_type_str)
           .env("MARKITDOWN_LLM_API_KEY", api_key)
           .env("MARKITDOWN_LLM_BASE_URL", base_url)
           .env("MARKITDOWN_LLM_MODEL", model);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn Python: {e}"))?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let mut result: Option<String> = None;

    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<PythonLine>(line) {
            Ok(PythonLine::Done { markdown }) => {
                result = Some(markdown);
            }
            Ok(PythonLine::Error { message }) => {
                return Err(message);
            }
            Err(_) => {}
        }
    }

    let stderr_output = stderr_task.await.unwrap_or_default();
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() && result.is_none() {
        let detail = if stderr_output.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", stderr_output.trim())
        };
        return Err(format!(
            "converter.py exited with code {:?}{}",
            status.code(),
            detail
        ));
    }

    result.ok_or_else(|| "converter.py did not emit markdown".to_string())
}

/// Open a native OS file picker and return the selected path, or None if cancelled.
#[tauri::command]
pub async fn markitdown_pick_file() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .add_filter(
            "Documents",
            &[
                "xlsx", "xls", "csv", "docx", "pdf", "pptx", "html", "htm",
                "jpg", "jpeg", "png", "gif", "webp", "epub", "msg",
                "txt", "md", "rst", "xml", "json", "yaml", "yml",
            ],
        )
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().to_string())
}
