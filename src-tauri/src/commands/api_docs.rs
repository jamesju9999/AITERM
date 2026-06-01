// src-tauri/src/commands/api_docs.rs
use std::sync::Arc;
use tauri::AppHandle;

use crate::api_docs::{fetcher_script_path, runner::run_fetcher, types::{AuthStatus, DocNode, ExtractionOptions}};
use crate::secret::SecretStore;

fn cookie_key(domain: &str) -> String {
    format!("api-docs-cookies-{domain}")
}

fn extract_domain(url: &str) -> String {
    url.trim_start_matches("https://")
       .trim_start_matches("http://")
       .split('/')
       .next()
       .unwrap_or(url)
       .to_string()
}

/// Detect the platform type of an API docs website.
/// Returns JSON string: `{"platform":"mintlify-next","confidence":"high"}`
#[tauri::command]
pub async fn api_docs_detect(
    app: AppHandle,
    url: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let script = fetcher_script_path(&app);
    let domain = extract_domain(&url);
    let cookies = secrets.get(&cookie_key(&domain)).unwrap_or(None).unwrap_or_default();

    let mut args = vec!["--url", url.as_str()];
    let cookies_owned;
    if !cookies.is_empty() {
        cookies_owned = cookies.clone();
        args.push("--cookies");
        args.push(&cookies_owned);
    }

    run_fetcher(&app, &script, "detect", &args).await?;
    // Detection result was emitted as "api-docs-detected" event; return ok
    Ok("ok".to_string())
}

/// Fetch the document tree for a site.
/// Returns `Vec<DocNode>` serialised as JSON.
#[tauri::command]
pub async fn api_docs_fetch_tree(
    app: AppHandle,
    url: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<Vec<DocNode>, String> {
    let script = fetcher_script_path(&app);
    let domain = extract_domain(&url);
    let cookies = secrets.get(&cookie_key(&domain)).unwrap_or(None).unwrap_or_default();

    let mut args = vec!["--url", url.as_str()];
    let cookies_owned;
    if !cookies.is_empty() {
        cookies_owned = cookies.clone();
        args.push("--cookies");
        args.push(&cookies_owned);
    }

    let tree = run_fetcher(&app, &script, "tree", &args).await?;
    tree.ok_or_else(|| "fetcher.py did not emit a tree".to_string())
}

/// Extract selected pages and emit progress events.
/// Fires `api-docs-progress`, `api-docs-log`, and `api-docs-done` events during execution.
#[tauri::command]
pub async fn api_docs_extract(
    app: AppHandle,
    options: ExtractionOptions,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let script = fetcher_script_path(&app);
    let domain = extract_domain(&options.url);
    let stored_cookies = secrets.get(&cookie_key(&domain)).unwrap_or(None).unwrap_or_default();

    // Prefer caller-supplied cookies; fall back to stored
    let effective_cookies = if !options.cookies.is_empty() {
        options.cookies.clone()
    } else {
        stored_cookies
    };

    let pages_json = serde_json::to_string(&options.pages)
        .map_err(|e| e.to_string())?;
    let keep_json = serde_json::to_string(&options.keep)
        .map_err(|e| e.to_string())?;
    let merge_str = if options.merge { "true" } else { "false" };

    let mut args = vec![
        "--url", options.url.as_str(),
        "--pages", pages_json.as_str(),
        "--output-dir", options.output_dir.as_str(),
        "--merge", merge_str,
        "--keep", keep_json.as_str(),
    ];
    let cookies_owned;
    if !effective_cookies.is_empty() {
        cookies_owned = effective_cookies.clone();
        args.push("--cookies");
        args.push(&cookies_owned);
    }

    run_fetcher(&app, &script, "extract", &args).await?;
    Ok(())
}
