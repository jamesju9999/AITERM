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

    // Write-then-rename rather than writing in place: every profile shares this
    // one file, so a write interrupted midway would leave JSON that reads as
    // "nothing installed" and force a reinstall of everything already set up,
    // not just the profile being recorded. rename is atomic within a filesystem,
    // and the temp file sits in the same directory to guarantee that.
    let tmp = venv.join(format!("{MARKER_FILE}.tmp"));
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, venv.join(MARKER_FILE))
        .with_context(|| format!("replacing {MARKER_FILE}"))
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

        // Write-then-rename must not leave a temp file behind after two
        // consecutive writes to the shared marker file.
        assert!(!dir.path().join(format!("{MARKER_FILE}.tmp")).exists());
    }

    #[test]
    fn installed_profiles_lists_only_recorded_ones() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        record_installed(dir.path(), Profile::DocCore, &req).unwrap();

        assert_eq!(installed_profiles(dir.path()), vec![Profile::DocCore]);
    }

    #[test]
    fn needs_install_errors_when_the_requirements_file_is_missing() {
        // Task 6 calls this and defaults to "install" on Err. That only stays a
        // sane default if a missing requirements file is an Err and not Ok(true)
        // — the two mean different things (packaging bug vs. first run), and
        // collapsing them here would hide the former.
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.txt");
        assert!(needs_install(dir.path(), Profile::DocCore, &missing).is_err());
    }

    #[test]
    fn a_corrupt_marker_also_reads_as_nothing_installed_for_the_listing() {
        // The "corrupt means nothing installed" guarantee has to hold on both
        // public read paths, not just needs_install.
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(MARKER_FILE), b"{not json").unwrap();
        assert!(installed_profiles(dir.path()).is_empty());
    }
}
