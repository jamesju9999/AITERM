/// The `.desktop` filename and icon name we install under. Deliberately not
/// "app": that name is far too generic to put in a shared icon theme, and it
/// would collide with any other project that ships an `app.png`.
#[cfg(any(target_os = "linux", test))]
const ENTRY_NAME: &str = "aiterm";

/// Rewrites the AppDir's `.desktop` for installation into the user's
/// applications directory.
///
/// Only `Exec=` and `Icon=` change. Everything else is carried through — the
/// bundler already generated `Name`, `Comment` and `Categories` from
/// tauri.conf.json, and rewriting them here would create a second source that
/// drifts the moment the config changes.
///
/// `StartupWMClass` in particular MUST survive untouched. It is `app`, derived
/// from the Rust binary name, and it is what GNOME matches the window against.
/// Renaming it to match ENTRY_NAME would leave a menu entry that looks correct
/// while the dock still shows a generic icon — a failure with no visible symptom.
#[cfg(any(target_os = "linux", test))]
fn rewrite_desktop(source: &str, appimage_path: &str) -> String {
    source
        .lines()
        .map(|line| {
            if line.starts_with("Exec=") {
                format!("Exec=\"{appimage_path}\" %U")
            } else if line.starts_with("Icon=") {
                format!("Icon={ENTRY_NAME}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// The path in a `.desktop`'s `Exec=` line, unquoted and without trailing
/// field codes. `None` when there is no `Exec=` line.
#[cfg(any(target_os = "linux", test))]
fn exec_path_of(desktop: &str) -> Option<String> {
    let value = desktop.lines().find_map(|l| l.strip_prefix("Exec="))?.trim();
    Some(if let Some(rest) = value.strip_prefix('"') {
        rest.split('"').next().unwrap_or("").to_string()
    } else {
        // Unquoted: the path runs until the first space, which is also where
        // any field code (%U, %F) would start.
        value.split(' ').next().unwrap_or("").to_string()
    })
}

/// Updated contents when `Exec=` no longer points at `appimage_path`, or `None`
/// when it already matches and nothing needs writing.
#[cfg(any(target_os = "linux", test))]
fn repair_exec(desktop: &str, appimage_path: &str) -> Option<String> {
    let current = exec_path_of(desktop)?;
    if current == appimage_path {
        return None;
    }
    Some(
        desktop
            .lines()
            .map(|line| {
                if line.starts_with("Exec=") {
                    format!("Exec=\"{appimage_path}\" %U")
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum IntegrationState {
    /// Not running as an AppImage — includes every non-Linux platform.
    NotAppimage,
    /// Running as an AppImage with no menu entry installed yet.
    Available,
    /// A menu entry exists; `exec_path` is what it currently points at.
    Integrated { exec_path: String },
}

#[cfg(target_os = "linux")]
mod paths {
    use std::path::PathBuf;

    pub fn desktop_file() -> Option<PathBuf> {
        Some(
            dirs::data_dir()?
                .join("applications")
                .join(format!("{}.desktop", super::ENTRY_NAME)),
        )
    }

    pub fn icon_dir() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("icons").join("hicolor"))
    }
}

#[tauri::command]
pub fn appimage_integration_state() -> IntegrationState {
    #[cfg(target_os = "linux")]
    {
        if std::env::var("APPIMAGE").is_err() {
            return IntegrationState::NotAppimage;
        }
        if let Some(path) = paths::desktop_file() {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Some(exec_path) = exec_path_of(&contents) {
                    return IntegrationState::Integrated { exec_path };
                }
            }
        }
        IntegrationState::Available
    }
    #[cfg(not(target_os = "linux"))]
    {
        IntegrationState::NotAppimage
    }
}

#[tauri::command]
pub fn appimage_integrate() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let appimage = std::env::var("APPIMAGE").map_err(|_| "not running as an AppImage".to_string())?;
        let appdir = std::env::var("APPDIR").map_err(|_| "APPDIR is not set".to_string())?;

        // The bundler already generated this with the right Name, Comment,
        // Categories and StartupWMClass — copying keeps tauri.conf.json as the
        // single source rather than duplicating that metadata here.
        let src_dir = std::path::Path::new(&appdir).join("usr/share/applications");
        let source = std::fs::read_dir(&src_dir)
            .map_err(|e| format!("cannot read {}: {e}", src_dir.display()))?
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().is_some_and(|x| x == "desktop"))
            .ok_or_else(|| format!("no .desktop found in {}", src_dir.display()))?;
        let contents = std::fs::read_to_string(source.path()).map_err(|e| e.to_string())?;

        let target = paths::desktop_file().ok_or("cannot resolve the data directory")?;
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&target, rewrite_desktop(&contents, &appimage)).map_err(|e| e.to_string())?;

        // A missing icon is not fatal: the menu entry still works, it just
        // falls back to a generic image. Failing here would be worse.
        let _ = copy_icons(&appdir);
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("only supported on Linux".to_string())
    }
}

/// Copies every hicolor size the AppDir ships, renamed to ENTRY_NAME.
#[cfg(target_os = "linux")]
fn copy_icons(appdir: &str) -> std::io::Result<()> {
    let src_root = std::path::Path::new(appdir).join("usr/share/icons/hicolor");
    let dst_root = paths::icon_dir().ok_or(std::io::ErrorKind::NotFound)?;
    for size in std::fs::read_dir(&src_root)?.filter_map(|e| e.ok()) {
        let src = size.path().join("apps");
        let Ok(entries) = std::fs::read_dir(&src) else { continue };
        for icon in entries.filter_map(|e| e.ok()) {
            let ext = icon.path().extension().map(|e| e.to_owned());
            let Some(ext) = ext else { continue };
            let dst_dir = dst_root.join(size.file_name()).join("apps");
            std::fs::create_dir_all(&dst_dir)?;
            let dst = dst_dir.join(format!("{}.{}", ENTRY_NAME, ext.to_string_lossy()));
            std::fs::copy(icon.path(), dst)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn appimage_remove_integration() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if let Some(path) = paths::desktop_file() {
            // Idempotent: already absent is success, not an error.
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
        if let Some(root) = paths::icon_dir() {
            if let Ok(sizes) = std::fs::read_dir(&root) {
                for size in sizes.filter_map(|e| e.ok()) {
                    for ext in ["png", "svg"] {
                        let p = size.path().join("apps").join(format!("{ENTRY_NAME}.{ext}"));
                        let _ = std::fs::remove_file(p);
                    }
                }
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("only supported on Linux".to_string())
    }
}

/// Called from `setup`: keeps an installed entry pointing at the AppImage's
/// current location after the user moves it or swaps in a new version.
///
/// Runs in the backend rather than the UI so it self-heals even for users who
/// dismissed the prompt and never open Settings.
pub fn repair_integration_on_startup() {
    #[cfg(target_os = "linux")]
    {
        let Ok(appimage) = std::env::var("APPIMAGE") else { return };
        let Some(path) = paths::desktop_file() else { return };
        let Ok(contents) = std::fs::read_to_string(&path) else { return };
        if let Some(updated) = repair_exec(&contents, &appimage) {
            let _ = std::fs::write(&path, updated);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the bundler actually produces inside the AppDir, verified by
    /// `cat squashfs-root/*.desktop` on a real build.
    const APPDIR_DESKTOP: &str = "\
[Desktop Entry]
Categories=Development;
Comment=AI TERM Studio
Exec=app
StartupWMClass=app
Icon=app
Name=AITerm
Terminal=false
Type=Application
";

    #[test]
    fn rewrite_points_exec_at_the_appimage() {
        let out = rewrite_desktop(APPDIR_DESKTOP, "/home/u/AITerm.AppImage");
        assert!(out.contains("Exec=\"/home/u/AITerm.AppImage\" %U"), "got:\n{out}");
    }

    #[test]
    fn rewrite_quotes_paths_containing_spaces() {
        // AppImages routinely sit in paths like ~/我的 下載/
        let out = rewrite_desktop(APPDIR_DESKTOP, "/home/u/my downloads/AITerm.AppImage");
        assert!(out.contains("Exec=\"/home/u/my downloads/AITerm.AppImage\" %U"), "got:\n{out}");
    }

    #[test]
    fn rewrite_points_icon_at_our_entry_name() {
        let out = rewrite_desktop(APPDIR_DESKTOP, "/x/A.AppImage");
        assert!(out.contains("Icon=aiterm"), "got:\n{out}");
        assert!(!out.contains("Icon=app\n"), "old icon line survived:\n{out}");
    }

    #[test]
    fn rewrite_preserves_startup_wm_class() {
        // The whole feature hinges on this staying "app" — see the doc comment.
        let out = rewrite_desktop(APPDIR_DESKTOP, "/x/A.AppImage");
        assert!(out.contains("StartupWMClass=app"), "got:\n{out}");
        assert!(!out.contains("StartupWMClass=aiterm"), "got:\n{out}");
    }

    #[test]
    fn rewrite_preserves_bundler_generated_metadata() {
        let out = rewrite_desktop(APPDIR_DESKTOP, "/x/A.AppImage");
        for line in ["Name=AITerm", "Comment=AI TERM Studio", "Categories=Development;"] {
            assert!(out.contains(line), "{line} missing from:\n{out}");
        }
    }

    #[test]
    fn exec_path_reads_a_quoted_path() {
        let d = "[Desktop Entry]\nExec=\"/home/u/my app/A.AppImage\" %U\n";
        assert_eq!(exec_path_of(d).as_deref(), Some("/home/u/my app/A.AppImage"));
    }

    #[test]
    fn exec_path_reads_an_unquoted_path() {
        // A user may have hand-edited the file.
        let d = "[Desktop Entry]\nExec=/home/u/A.AppImage %U\n";
        assert_eq!(exec_path_of(d).as_deref(), Some("/home/u/A.AppImage"));
    }

    #[test]
    fn exec_path_is_none_without_an_exec_line() {
        assert_eq!(exec_path_of("[Desktop Entry]\nName=X\n"), None);
    }

    #[test]
    fn repair_is_a_no_op_when_the_path_already_matches() {
        let d = "[Desktop Entry]\nExec=\"/x/A.AppImage\" %U\nIcon=aiterm\n";
        assert_eq!(repair_exec(d, "/x/A.AppImage"), None);
    }

    #[test]
    fn repair_rewrites_only_the_exec_line() {
        let d = "[Desktop Entry]\nExec=\"/old/A.AppImage\" %U\nStartupWMClass=app\nName=AITerm\n";
        let out = repair_exec(d, "/new/A.AppImage").expect("should need repair");
        assert!(out.contains("Exec=\"/new/A.AppImage\" %U"), "got:\n{out}");
        assert!(out.contains("StartupWMClass=app"), "got:\n{out}");
        assert!(out.contains("Name=AITerm"), "got:\n{out}");
        assert!(!out.contains("/old/"), "old path survived:\n{out}");
    }

    #[test]
    fn repair_is_none_when_there_is_nothing_to_repair() {
        // No Exec= line at all: the user mangled it, leave it alone.
        assert_eq!(repair_exec("[Desktop Entry]\nName=X\n", "/x/A.AppImage"), None);
    }
}
