// src-tauri/src/commands/api_docs.rs
use std::sync::Arc;
use tauri::{AppHandle, Manager};

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

    let mut args = vec![
        "--url", options.url.as_str(),
        "--pages", pages_json.as_str(),
        "--output", options.output_dir.as_str(),
        "--keep", keep_json.as_str(),
    ];
    if options.merge {
        args.push("--merge");
    }
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
/// Monitors navigation; when the URL returns to the docs domain, reads cookies
/// via the Tauri WebviewWindow cookie API and stores them in the OS keyring.
/// Returns the cookie string (key=value; key=value) on success.
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

    // Captured cookies stored as (name, value) pairs — Vec<(String,String)> is Send,
    // allowing it to cross from run_on_main_thread back to the tokio task via a channel.
    let captured: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
    let tx_clone = tx.clone();
    let captured_for_nav = captured.clone();
    let app_for_nav = app.clone();
    let label_for_nav = label.clone();
    let domain_for_nav = domain_clone.clone();

    // Detect login by tracking the initial host (e.g. "docs.developer.swift.com").
    // Many OAuth flows (including SWIFT) stay within the same parent domain, so tracking
    // "left parent zone / returned" doesn't work. Instead we track the exact initial host:
    // once the user navigates away from it (to a sign-in page) and comes back, login is done.
    let initial_host = domain_clone.clone(); // "docs.developer.swift.com"
    let initial_host_for_closure = initial_host.clone();
    let has_left_initial = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let has_left_initial_clone = has_left_initial.clone();
    let has_triggered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let has_triggered_clone = has_triggered.clone();

    let window = WebviewWindowBuilder::new(&app, &label, webview_url)
        .title(format!("Login — {domain}"))
        .inner_size(900.0, 700.0)
        .on_navigation(move |nav_url: &tauri::Url| {
            let nav_host = nav_url.host_str().unwrap_or("");
            if nav_host.is_empty() { return true; }

            let left = has_left_initial_clone.load(std::sync::atomic::Ordering::Relaxed);
            eprintln!("[api-docs-login] nav={nav_host} initial={initial_host_for_closure} has_left={left}");

            if nav_host != initial_host_for_closure {
                // Navigated away from the initial docs host (e.g. to sign-in page)
                has_left_initial_clone.store(true, std::sync::atomic::Ordering::Relaxed);
            } else if has_left_initial_clone.load(std::sync::atomic::Ordering::Relaxed)
                && !has_triggered_clone.swap(true, std::sync::atomic::Ordering::Relaxed)
            {
                // Returned to initial host after having left — login complete.
                eprintln!("[api-docs-login] trigger fired — waiting 2s then reading cookies");
                // Returned to parent domain after OAuth — trigger exactly once.
                // Spawn a tokio task that:
                //   1. Waits for the page to settle (2 s)
                //   2. Dispatches cookie reading to the main thread via run_on_main_thread
                //      (WKWebView's getAllCookies callback requires the main run-loop)
                //   3. Closes the window and signals the waiting command.
                let tx2 = tx_clone.clone();
                let app2 = app_for_nav.clone();
                let label2 = label_for_nav.clone();
                let domain2 = domain_for_nav.clone();
                let captured2 = captured_for_nav.clone();

                tauri::async_runtime::spawn(async move {
                    // Give the page time to finish loading and setting cookies
                    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

                    // Read cookies on the main thread — required on macOS (WKWebView)
                    let (ch_tx, ch_rx) =
                        tokio::sync::oneshot::channel::<Vec<(String, String)>>();
                    let app_m = app2.clone();
                    let label_m = label2.clone();
                    let domain_m = domain2.clone();
                    let _ = app2.run_on_main_thread(move || {
                        let mut pairs: Vec<(String, String)> = Vec::new();
                        let win_exists = app_m.get_webview_window(&label_m).is_some();
                        eprintln!("[api-docs-login] run_on_main_thread: window_exists={win_exists}");
                        if let Some(win) = app_m.get_webview_window(&label_m) {
                            let parts: Vec<String> =
                                domain_m.split('.').map(String::from).collect();
                            let mut urls = vec![format!("https://{domain_m}/")];
                            for skip in 1..parts.len().saturating_sub(1) {
                                let parent = parts[skip..].join(".");
                                urls.push(format!("https://{parent}/"));
                                urls.push(format!("http://{parent}/"));
                            }
                            let mut seen = std::collections::HashSet::new();
                            for u in &urls {
                                if let Ok(parsed) = u.parse::<tauri::Url>() {
                                    if let Ok(cookies) = win.cookies_for_url(parsed)
                                        .map(|v: Vec<tauri::webview::Cookie<'static>>| v)
                                    {
                                        for c in cookies {
                                            let name = c.name().to_string();
                                            if seen.insert(name.clone()) {
                                                pairs.push((name, c.value().to_string()));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        eprintln!("[api-docs-login] cookies read: {} pairs", pairs.len());
                        for (k, _) in &pairs {
                            eprintln!("[api-docs-login]   cookie name={k}");
                        }
                        let _ = ch_tx.send(pairs);
                    });

                    if let Ok(pairs) = ch_rx.await {
                        eprintln!("[api-docs-login] captured {} cookies", pairs.len());
                        *captured2.lock().unwrap() = pairs;
                    }

                    // Close the login window (also via main thread)
                    let app_c = app2.clone();
                    let label_c = label2.clone();
                    let _ = app2.run_on_main_thread(move || {
                        if let Some(win) = app_c.get_webview_window(&label_c) {
                            let _ = win.close();
                        }
                    });

                    if let Some(sender) = tx2.lock().await.take() {
                        let _ = sender.send(());
                    }
                });
            }
            true
        })
        .build()
        .map_err(|e| e.to_string())?;

    // Wait for login signal (timeout after 5 min)
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        async { rx.await.ok() }
    )
    .await;

    // Ensure window is closed (no-op if already closed)
    let _ = window.close();

    let pairs: Vec<(String, String)> = captured.lock().unwrap().drain(..).collect();

    if pairs.is_empty() {
        return Err("登入視窗關閉但未偵測到認證 Cookie。請確認已成功完成登入流程後再關閉視窗。".to_string());
    }

    let cookie_str = pairs.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<String>>()
        .join("; ");

    secrets.set(&cookie_key(&domain), &cookie_str)
        .map_err(|e| e.to_string())?;

    Ok(cookie_str)
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
