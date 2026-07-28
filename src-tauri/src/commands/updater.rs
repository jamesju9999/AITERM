/// Decides whether the in-app updater can service this install.
///
/// AppImage is the only self-updatable Linux bundle we ship — `.deb` installs
/// are served by `latest.json`'s AppImage URL, which would be the wrong artifact.
/// Tauri's AppImage launcher exports `APPIMAGE`, so its presence identifies the bundle.
///
/// Kept env-free so it is testable without mutating process state, which races
/// under cargo's parallel test threads.
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
