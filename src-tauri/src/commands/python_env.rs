//! Tauri surface for the managed Python environment.

use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::config::ConfigStore;
use crate::python_env::{self, profiles::Profile, EnvStatus};

#[tauri::command]
pub fn python_env_status(app: AppHandle) -> EnvStatus {
    python_env::status(&app)
}

#[tauri::command]
pub async fn python_env_ensure(app: AppHandle, profile: Profile) -> Result<(), String> {
    python_env::ensure(&app, profile).await.map(|_| ()).map_err(String::from)
}

#[tauri::command]
pub async fn python_env_reset(app: AppHandle, purge_runtimes: bool) -> Result<(), String> {
    python_env::reset(&app, purge_runtimes).await.map_err(String::from)
}

#[tauri::command]
pub fn python_env_set_interpreter(
    path: Option<String>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<(), String> {
    let normalized = path.as_ref().map(|p| p.trim().to_string()).filter(|p| !p.is_empty());

    // Verify before storing: the settings page shows the new source immediately,
    // so an unusable path would leave the UI claiming an interpreter that only
    // fails later, when a rebuild finally hands it to uv. MarkItDown needs 3.10+.
    //
    // Asking it to print the version (rather than checking the exit status) is
    // what separates "not a Python at all" from "a Python that's too old" — any
    // executable can exit non-zero on unfamiliar arguments, so an exit code
    // alone would misreport something like /bin/ls as an outdated Python.
    if let Some(candidate) = &normalized {
        let mut cmd = std::process::Command::new(candidate);
        cmd.arg("-c")
            .arg("import sys; print(sys.version_info[0], sys.version_info[1])");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        match cmd.output() {
            Err(e) => {
                return Err(format!("無法執行這個路徑，請確認它是 Python 執行檔：{e}"));
            }
            Ok(out) => {
                let printed = String::from_utf8_lossy(&out.stdout);
                let mut parts = printed.split_whitespace();
                let version = parts
                    .next()
                    .and_then(|m| m.parse::<u32>().ok())
                    .zip(parts.next().and_then(|m| m.parse::<u32>().ok()));
                match version {
                    None => return Err("這個路徑不是 Python 執行檔".to_string()),
                    Some((major, minor)) if (major, minor) < (3, 10) => {
                        return Err(format!("這個 Python 是 {major}.{minor}，需要 3.10 或更新版本"));
                    }
                    Some(_) => {}
                }
            }
        }
    }

    config
        .update(|cfg| cfg.python_interpreter = normalized)
        .map_err(|e| format!("儲存設定失敗：{e}"))
}

/// Trim/normalize `url`, rejecting anything that isn't shaped like one. Split
/// out so it's testable without a `ConfigStore` — same reasoning as
/// `resolve_requirements` in `python_env/mod.rs`.
///
/// Same argument as `python_env_set_interpreter` above, applied to the index
/// instead of the interpreter: the settings page shows the new value
/// immediately, so a typo (a missing scheme, most likely) would otherwise only
/// surface as a connection error the next time uv installs packages.
///
/// Deliberately shape-only — this doesn't connect to the URL the way
/// `set_interpreter` runs the candidate interpreter. Actually reaching it
/// would be slow, and would misclassify a valid mirror that's merely
/// unreachable right now (offline dev, a VPN not yet up) as invalid.
fn validate_index_url(url: Option<String>) -> Result<Option<String>, String> {
    let normalized = url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    match &normalized {
        Some(u) if !(u.starts_with("http://") || u.starts_with("https://")) => {
            Err("Index URL 必須以 http:// 或 https:// 開頭".to_string())
        }
        _ => Ok(normalized),
    }
}

#[tauri::command]
pub fn python_env_set_index_url(
    url: Option<String>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<(), String> {
    let normalized = validate_index_url(url)?;
    config
        .update(|cfg| cfg.python_index_url = normalized)
        .map_err(|e| format!("儲存設定失敗：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_valid_https_url() {
        assert_eq!(
            validate_index_url(Some("https://pypi.mycompany.com/simple".to_string())),
            Ok(Some("https://pypi.mycompany.com/simple".to_string()))
        );
    }

    #[test]
    fn accepts_a_valid_http_url() {
        assert_eq!(
            validate_index_url(Some("http://pypi.mycompany.com/simple".to_string())),
            Ok(Some("http://pypi.mycompany.com/simple".to_string()))
        );
    }

    #[test]
    fn rejects_a_url_missing_a_scheme() {
        // The exact typo this exists for: a pasted host/path with the
        // `https://` accidentally left off.
        assert!(validate_index_url(Some("pypi.mycompany.com/simple".to_string())).is_err());
    }

    #[test]
    fn empty_input_clears_the_setting_rather_than_erroring() {
        assert_eq!(validate_index_url(Some("   ".to_string())), Ok(None));
        assert_eq!(validate_index_url(None), Ok(None));
    }

    #[test]
    fn trims_whitespace_around_a_valid_url() {
        assert_eq!(
            validate_index_url(Some("  https://pypi.mycompany.com/simple  ".to_string())),
            Ok(Some("https://pypi.mycompany.com/simple".to_string()))
        );
    }
}
