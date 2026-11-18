//! 檔案上傳的路徑檢查：AI 只能上傳「可上傳資料夾」裡的檔案。
//! 路徑先解開 `..` 與符號連結（canonicalize）再比對，所以繞不出資料夾。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// 檢查並回傳解開後的絕對路徑。任何一個不合格就整批拒絕。
pub fn check_paths(dirs: &[String], paths: &[String]) -> Result<Vec<PathBuf>, String> {
    let roots: Vec<PathBuf> = dirs.iter().filter(|d| !d.trim().is_empty()).filter_map(|d| std::fs::canonicalize(d.trim()).ok()).collect();
    if roots.is_empty() {
        return Err("這個系統沒有設定「可上傳資料夾」，不能上傳檔案。請使用者在系統連線的編輯畫面設定。".into());
    }
    if paths.is_empty() {
        return Err("paths 不可空白：請給要上傳的檔案完整路徑".into());
    }
    let mut out = Vec::new();
    for p in paths {
        let real = std::fs::canonicalize(Path::new(p)).map_err(|_| format!("找不到檔案：{p}"))?;
        if !real.is_file() {
            return Err(format!("不是檔案：{p}"));
        }
        if !roots.iter().any(|r| real.starts_with(r)) {
            return Err(format!("不在可上傳資料夾內，已拒絕：{p}"));
        }
        out.push(real);
    }
    Ok(out)
}

/// 呼叫前處理 `sys_file_upload` 的參數：檢查路徑，換成解開後的路徑，並把檔名寫進 intent
/// （核准卡片與操作記錄才看得到實際上傳什麼）。
pub fn prepare(dirs: &[String], args: &mut Value) -> Result<(), String> {
    let paths: Vec<String> = args["paths"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let real = check_paths(dirs, &paths)?;
    let shown: Vec<String> = real.iter().map(|p| p.display().to_string()).collect();
    let intent = args["intent"].as_str().unwrap_or("").to_string();
    args["intent"] = json!(format!("{intent}（上傳：{}）", shown.join("、")));
    args["paths"] = json!(shown);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, String) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("in")).unwrap();
        std::fs::write(d.path().join("in/a.txt"), "a").unwrap();
        std::fs::write(d.path().join("secret.txt"), "s").unwrap();
        let dir = d.path().join("in").display().to_string();
        (d, dir)
    }

    #[test]
    fn files_inside_the_folder_are_allowed() {
        let (d, dir) = setup();
        let ok = check_paths(&[dir], &[d.path().join("in/a.txt").display().to_string()]).unwrap();
        assert_eq!(ok.len(), 1);
    }

    #[test]
    fn files_outside_or_escaping_with_dotdot_are_refused() {
        let (d, dir) = setup();
        let outside = d.path().join("secret.txt").display().to_string();
        assert!(check_paths(&[dir.clone()], &[outside]).unwrap_err().contains("不在可上傳資料夾內"));
        let sneaky = format!("{}/../secret.txt", dir);
        assert!(check_paths(&[dir], &[sneaky]).is_err(), "用 .. 繞出去也不行");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_outside_is_refused() {
        let (d, dir) = setup();
        let link = d.path().join("in/link.txt");
        std::os::unix::fs::symlink(d.path().join("secret.txt"), &link).unwrap();
        assert!(check_paths(&[dir], &[link.display().to_string()]).is_err());
    }

    #[test]
    fn no_folder_configured_or_one_bad_path_in_a_batch_refuses_everything() {
        let (d, dir) = setup();
        let good = d.path().join("in/a.txt").display().to_string();
        assert!(check_paths(&[], &[good.clone()]).unwrap_err().contains("沒有設定"));
        assert!(check_paths(&[dir], &[good, "/nonexistent/x".into()]).is_err());
    }

    #[test]
    fn prepare_adds_the_file_names_to_the_intent() {
        let (d, dir) = setup();
        let mut args = json!({ "intent": "寫：上傳報表", "paths": [d.path().join("in/a.txt").display().to_string()] });
        prepare(&[dir], &mut args).unwrap();
        assert!(args["intent"].as_str().unwrap().contains("a.txt") && args["intent"].as_str().unwrap().starts_with("寫：上傳報表"));
    }
}
