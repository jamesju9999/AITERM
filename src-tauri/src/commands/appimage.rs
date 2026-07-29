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
