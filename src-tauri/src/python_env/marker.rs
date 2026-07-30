//! Records which profiles are installed in the managed venv.
//!
//! Keyed by the sha256 of the requirements file, so editing a requirements
//! file re-installs that profile and nothing else. A missing or unreadable
//! marker means "nothing is installed" — re-installing is cheap and correct,
//! guessing is not.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use super::profiles::Profile;

pub const MARKER_FILE: &str = ".aiterm-profiles.json";

/// True when `profile`'s requirements differ from what's recorded.
pub fn needs_install(venv: &Path, profile: Profile, requirements: &Path) -> Result<bool> {
    let want = hash_file(requirements)?;
    Ok(read_marker(venv).get(profile.marker_key()) != Some(&want))
}

/// Record `profile` as installed at the requirements file's current contents.
pub fn record_installed(venv: &Path, profile: Profile, requirements: &Path) -> Result<()> {
    let hash = hash_file(requirements)?;
    let mut marker = read_marker(venv);
    marker.insert(profile.marker_key().to_string(), hash);

    std::fs::create_dir_all(venv).with_context(|| format!("creating {}", venv.display()))?;
    let body = serde_json::to_string_pretty(&marker)?;
    std::fs::write(venv.join(MARKER_FILE), body).with_context(|| format!("writing {MARKER_FILE}"))
}

/// Profiles with a recorded hash, whatever it is. Used for status display, so
/// a stale hash still counts as "installed" — `ensure` re-checks properly.
pub fn installed_profiles(venv: &Path) -> Vec<Profile> {
    let marker = read_marker(venv);
    Profile::ALL
        .into_iter()
        .filter(|p| marker.contains_key(p.marker_key()))
        .collect()
}

fn read_marker(venv: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(venv.join(MARKER_FILE))
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default()
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::python_env::profiles::Profile;
    use tempfile::tempdir;

    fn write_requirements(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("requirements.txt");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn install_is_needed_when_no_marker_exists() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        assert!(needs_install(dir.path(), Profile::DocCore, &req).unwrap());
    }

    #[test]
    fn install_is_skipped_once_recorded() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");

        record_installed(dir.path(), Profile::DocCore, &req).unwrap();

        assert!(!needs_install(dir.path(), Profile::DocCore, &req).unwrap());
    }

    #[test]
    fn install_is_needed_again_after_requirements_change() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        record_installed(dir.path(), Profile::DocCore, &req).unwrap();

        write_requirements(dir.path(), "markitdown>=0.2.0\n");

        assert!(needs_install(dir.path(), Profile::DocCore, &req).unwrap());
    }

    #[test]
    fn a_corrupt_marker_is_treated_as_nothing_installed() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        std::fs::write(dir.path().join(MARKER_FILE), b"{not json").unwrap();

        assert!(needs_install(dir.path(), Profile::DocCore, &req).unwrap());
    }

    #[test]
    fn recording_one_profile_leaves_the_others_untouched() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        record_installed(dir.path(), Profile::DocCore, &req).unwrap();
        record_installed(dir.path(), Profile::DocMedia, &req).unwrap();

        assert!(!needs_install(dir.path(), Profile::DocCore, &req).unwrap());
        assert!(!needs_install(dir.path(), Profile::DocMedia, &req).unwrap());
        assert!(needs_install(dir.path(), Profile::ApiDocs, &req).unwrap());
    }

    #[test]
    fn installed_profiles_lists_only_recorded_ones() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        record_installed(dir.path(), Profile::DocCore, &req).unwrap();

        assert_eq!(installed_profiles(dir.path()), vec![Profile::DocCore]);
    }
}
