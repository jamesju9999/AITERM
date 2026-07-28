/// Decides whether the in-app updater can service this install.
///
/// AppImage is the only self-updatable Linux bundle we ship — every other Linux
/// packaging we produce (`.deb`, `.rpm`, Snap, Flatpak) is served by `latest.json`'s
/// AppImage URL, which would be the wrong artifact. `APPIMAGE` is exported by the
/// AppImage type-2 runtime embedded in every AppImage per the AppImage spec itself,
/// not something Tauri adds — so its presence is a durable, format-level signal
/// rather than a plugin implementation detail that could change on upgrade. It's
/// also the same signal `tauri-plugin-updater` uses to decide whether it can
/// install on Linux, so this gate mirrors the plugin's real capability rather than
/// guessing at it.
///
/// Kept env-free so it is testable without mutating process state: that's a race
/// under cargo's parallel test threads, and as of Rust 2024 `std::env::set_var` is
/// `unsafe` besides.
fn supported_for(is_linux: bool, appimage_env: Option<&str>) -> bool {
    !is_linux || appimage_env.is_some()
}

#[tauri::command]
pub fn updater_supported() -> bool {
    let appimage = std::env::var("APPIMAGE").ok();
    supported_for(cfg!(target_os = "linux"), appimage.as_deref())
}

#[cfg(test)]
mod tests {
    use super::supported_for;

    #[test]
    fn non_linux_is_always_supported() {
        assert!(supported_for(false, None));
        assert!(supported_for(false, Some("/tmp/AITerm.AppImage")));
    }

    #[test]
    fn linux_appimage_is_supported() {
        assert!(supported_for(true, Some("/tmp/AITerm.AppImage")));
    }

    #[test]
    fn linux_without_appimage_env_is_not_supported() {
        assert!(!supported_for(true, None));
    }
}
