fn main() {
  // Fix read-only JRE files that were copied by a previous build with
  // permissions inherited from the original archive (r--r--r--).
  // tauri_build::copy_resources uses fs::copy which fails to overwrite
  // read-only destination files on macOS.
  #[cfg(target_os = "macos")]
  fix_target_db2_permissions();

  tauri_build::build()
}

#[cfg(target_os = "macos")]
fn fix_target_db2_permissions() {
  let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
  // OUT_DIR is something like .../target/debug/build/app-xxx/out
  // Walk up to find target/debug (or release)
  let mut p = std::path::Path::new(&out_dir);
  loop {
    let candidate = p.join("db2-sidecar");
    if candidate.is_dir() {
      fix_dir_permissions(&candidate);
      break;
    }
    match p.parent() {
      Some(parent) => p = parent,
      None => break,
    }
  }
}

#[cfg(target_os = "macos")]
fn fix_dir_permissions(dir: &std::path::Path) {
  use std::os::unix::fs::PermissionsExt;
  if let Ok(entries) = std::fs::read_dir(dir) {
    for entry in entries.flatten() {
      let path = entry.path();
      if path.is_dir() {
        fix_dir_permissions(&path);
      } else if let Ok(meta) = std::fs::metadata(&path) {
        let mode = meta.permissions().mode();
        // Ensure owner write bit is set
        if mode & 0o200 == 0 {
          let _ = std::fs::set_permissions(
            &path,
            std::fs::Permissions::from_mode(mode | 0o200),
          );
        }
      }
    }
  }
}
