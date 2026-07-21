use std::path::Path;

const EXCLUDED_DIRS: &[&str] = &[
    "node_modules", ".git", "target", "dist", "build",
    "__pycache__", ".next", ".nuxt", "vendor", ".svn", ".hg",
    "coverage", ".cache", ".parcel-cache",
];

const EXCLUDED_EXTENSIONS: &[&str] = &[
    "lock", "bin", "exe", "dll", "so", "dylib",
    "png", "jpg", "jpeg", "gif", "ico", "webp", "bmp", "svg",
    "mp4", "mp3", "wav", "mov", "avi",
    "zip", "tar", "gz", "rar", "7z",
    "jar", "war", "ear", "aar",
    "pdf", "doc", "docx", "xls", "xlsx",
    "woff", "woff2", "ttf", "eot",
    "pyc", "class", "o", "db", "dat",
];

/// Returns true if `path` should be hidden from the AI (not listed, not readable).
pub fn is_excluded(path: &Path, project_root: &Path) -> bool {
    let relative = match path.strip_prefix(project_root) {
        Ok(r) => r,
        Err(_) => path,
    };

    // Check every component of the relative path
    for component in relative.components() {
        let name = component.as_os_str().to_string_lossy();
        if EXCLUDED_DIRS.contains(&name.as_ref()) {
            return true;
        }
    }

    // Exclude files/dirs inside hidden directories, or hidden directories themselves.
    // Walk the parent chain: if any ancestor directory name starts with '.', exclude.
    // This also catches hidden dirs themselves when they appear as intermediates.
    let parent = relative.parent().unwrap_or(Path::new(""));
    for component in parent.components() {
        let name = component.as_os_str().to_string_lossy();
        if name.starts_with('.') {
            return true;
        }
    }
    // Exclude hidden directories themselves (the final component, when it IS a directory)
    if path.is_dir() {
        if let Some(last) = relative.file_name() {
            if last.to_string_lossy().starts_with('.') {
                return true;
            }
        }
    }

    // Exclude files with excluded extensions
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        if EXCLUDED_EXTENSIONS.contains(&ext_str.as_ref()) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn excludes_node_modules() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/node_modules/react/index.js");
        assert!(is_excluded(&path, &root));
    }

    #[test]
    fn excludes_git() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/.git/config");
        assert!(is_excluded(&path, &root));
    }

    #[test]
    fn excludes_png() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/src/icon.png");
        assert!(is_excluded(&path, &root));
    }

    #[test]
    fn allows_rust_source() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/src/main.rs");
        assert!(!is_excluded(&path, &root));
    }

    #[test]
    fn allows_typescript_source() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/src/App.tsx");
        assert!(!is_excluded(&path, &root));
    }

    #[test]
    fn excludes_file_inside_hidden_dir() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/.myapp/config.json");
        assert!(is_excluded(&path, &root));
    }
}
