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

use tauri::{WebviewWindowBuilder, WebviewUrl};

/// Open an embedded WebView window for the user to log in.
/// Monitors navigation; when the URL returns to the docs domain, extracts
/// document.cookie via JS eval and stores it in the OS keyring.
/// Returns the cookie string on success.
#[tauri::command]
pub async fn api_docs_login(
    app: AppHandle,
    url: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let domain = extract_domain(&url);
    let domain_clone = domain.clone();

    // Build a unique window label so multiple calls don't conflict
    let label = format!("api-docs-login-{}", uuid::Uuid::new_v4().simple());

    let webview_url = WebviewUrl::External(
        url.parse::<tauri::Url>().map_err(|e| e.to_string())?
    );

    let window = WebviewWindowBuilder::new(&app, &label, webview_url)
        .title(format!("Login — {domain}"))
        .inner_size(900.0, 700.0)
        .build()
        .map_err(|e| e.to_string())?;

    // Poll navigation until we're back on the docs domain (login success)
    // We check every 500ms for up to 5 minutes
    let (tx, mut rx) = tokio::sync::oneshot::channel::<String>();
    let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
    let domain_for_closure = domain_clone.clone();

    let window_clone = window.clone();
    let tx_clone = tx.clone();
    window.on_navigation(move |nav_url| {
        let nav_host = nav_url.host_str().unwrap_or("");
        if nav_host.contains(&domain_for_closure) || nav_host.ends_with(&domain_for_closure) {
            // We're back on the docs domain — extract cookies via JS
            let win = window_clone.clone();
            let tx2 = tx_clone.clone();
            tauri::async_runtime::spawn(async move {
                // Small delay so the page can set cookies
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                let cookies = win.eval("document.cookie")
                    .unwrap_or_else(|_| "\"\"".into());
                // eval returns a JS value; strip surrounding quotes if present
                let cookies = cookies.trim_matches('"').to_string();
                if let Some(sender) = tx2.lock().await.take() {
                    let _ = sender.send(cookies);
                }
            });
        }
        true  // allow navigation
    });

    // Wait for cookie signal (timeout after 5 min)
    let cookies = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        async { rx.await.unwrap_or_default() }
    )
    .await
    .unwrap_or_default();

    // Close the login window
    let _ = window.close();

    if cookies.is_empty() {
        return Err("Login window closed without detecting a successful login".to_string());
    }

    // Persist to keyring
    secrets.set(&cookie_key(&domain), &cookies)
        .map_err(|e| e.to_string())?;

    Ok(cookies)
}

/// Clear stored cookies for a domain.
#[tauri::command]
pub async fn api_docs_logout(
    domain: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    secrets.delete(&cookie_key(&domain))
        .map_err(|e| e.to_string())
}

/// Check whether cookies are stored for a domain.
#[tauri::command]
pub async fn api_docs_auth_status(
    domain: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<AuthStatus, String> {
    let key = cookie_key(&domain);
    match secrets.get(&key) {
        Ok(Some(cookies)) if !cookies.is_empty() => {
            // Try to extract an email-like token from the cookies as a display name
            let account = cookies
                .split(';')
                .filter_map(|pair| {
                    let kv: Vec<&str> = pair.splitn(2, '=').collect();
                    if kv.len() == 2 { Some(kv[1].trim().to_string()) } else { None }
                })
                .find(|v| v.contains('@'))
                .unwrap_or_default();
            Ok(AuthStatus { logged_in: true, account })
        }
        _ => Ok(AuthStatus { logged_in: false, account: String::new() }),
    }
}
