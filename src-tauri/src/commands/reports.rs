//! 工作報告的存取。報告是 AI 產生的 HTML 文件，存在
//! `<專案>/reports/` 底下累積成歷史——專案資料夾自成一體，所以報告
//! 會跟著專案走。
//!
//! 檔案 I/O 留在 command 層、不下放 `tasks::store`，與
//! `tasks_save_transcript` 同一慣例。

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::projects::{ProjectHandle, ProjectRegistry};

#[derive(Serialize)]
pub struct ReportInfo {
    pub filename: String,
    /// 檔案的修改時間，Unix 秒。
    pub saved_at: i64,
    /// 從 HTML 的 `<title>` 取出；沒有就是 None，前端顯示檔名。
    pub title: Option<String>,
}

fn project(reg: &ProjectRegistry, id: &str) -> Result<ProjectHandle, String> {
    reg.get(id).ok_or_else(|| format!("專案不存在或已關閉：{id}"))
}

fn reports_dir(project: &ProjectHandle) -> PathBuf {
    project.path.join("reports")
}

/// 只接受單純的檔名。路徑分隔字元、`..`、空字串一律拒絕——這個指令
/// 不能假設呼叫端守規矩。
fn safe_report_name(name: &str) -> Result<&str, String> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || PathBuf::from(name).components().count() != 1
    {
        return Err(format!("不合法的報告檔名：{name}"));
    }
    Ok(name)
}

/// `<stamp>.html`，若已存在則往後找 `-2`、`-3`。同一分鐘內產第二份
/// 不可以覆蓋第一份。
fn report_filename(existing: &[String], stamp: &str) -> String {
    let first = format!("{stamp}.html");
    if !existing.iter().any(|e| e == &first) {
        return first;
    }
    for n in 2..1000 {
        let candidate = format!("{stamp}-{n}.html");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
    }
    format!("{stamp}-{}.html", uuid::Uuid::new_v4())
}

/// 從 HTML 抓 `<title>`。刻意用最笨的字串搜尋而不是 HTML 解析器：
/// 只是為了給歷史清單一個好看的標籤，抓不到就顯示檔名，不值得為此
/// 引入一個解析器。
///
/// 用 `char_indices` 在原字串（非小寫化的字串）上找標籤邊界，
/// 避免 `to_lowercase()` 因為少數 Unicode 字元改變位元組長度而
/// 讓位移跟原字串對不上。
fn title_from_html(html: &str) -> Option<String> {
    let open_tag = find_ci(html, "<title>")?;
    let start = open_tag + "<title>".len();
    let close_tag = find_ci(&html[start..], "</title>")? + start;
    let title = html[start..close_tag].trim();
    if title.is_empty() { None } else { Some(title.to_string()) }
}

/// 在 `haystack` 裡找 `needle`（ASCII 大小寫不敏感）出現的位元組位移。
/// `needle` 全是 ASCII，所以逐窗比對即可，不需要動 `haystack` 本身
/// 的大小寫或編碼。
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() || bytes.len() < needle_bytes.len() {
        return None;
    }
    (0..=bytes.len() - needle_bytes.len()).find(|&i| {
        haystack.is_char_boundary(i)
            && bytes[i..i + needle_bytes.len()].eq_ignore_ascii_case(needle_bytes)
    })
}

fn list_filenames(dir: &PathBuf) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".html"))
        .collect()
}

#[tauri::command]
pub async fn reports_save(
    project_id: String,
    html: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let dir = reports_dir(&p);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M").to_string();
    let filename = report_filename(&list_filenames(&dir), &stamp);
    std::fs::write(dir.join(&filename), html).map_err(|e| e.to_string())?;
    Ok(filename)
}

#[tauri::command]
pub async fn reports_list(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<ReportInfo>, String> {
    let p = project(&reg, &project_id)?;
    let dir = reports_dir(&p);
    let mut out = Vec::new();
    for filename in list_filenames(&dir) {
        let path = dir.join(&filename);
        let saved_at = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let title = std::fs::read_to_string(&path).ok().and_then(|h| title_from_html(&h));
        out.push(ReportInfo { filename, saved_at, title });
    }
    // 新到舊。時間相同時用檔名遞減當第二順位，讓順序是確定的
    // （同一分鐘的 -2 排在無後綴的前面）。
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then(b.filename.cmp(&a.filename)));
    Ok(out)
}

#[tauri::command]
pub async fn reports_read(
    project_id: String,
    filename: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let safe = safe_report_name(&filename)?;
    std::fs::read_to_string(reports_dir(&p).join(safe)).map_err(|e| e.to_string())
}

/// 刪掉一份歷史報告。報告是可以重新產生的產物，所以真的刪檔、不做垃圾桶；
/// 確認的動作留在前端（原生確認框）。
///
/// 檔案已經不在時視為成功：使用者要的結果是「這份報告不在了」，而重複
/// 點兩下或兩個視窗同時刪同一份都會走到這裡，報錯只會造成困惑。
#[tauri::command]
pub async fn reports_delete(
    project_id: String,
    filename: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let safe = safe_report_name(&filename)?;
    delete_report_file(&reports_dir(&p), safe)
}

/// `reports_delete` 的檔案操作。抽出來是為了可測——指令本身綁著
/// `State<ProjectRegistry>`，沒有真的 Tauri app 起不來。
fn delete_report_file(dir: &PathBuf, safe: &str) -> Result<(), String> {
    match std::fs::remove_file(dir.join(safe)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_report_name_accepts_a_plain_filename() {
        assert!(safe_report_name("2026-09-05-1430.html").is_ok());
        assert!(safe_report_name("2026-09-05-1430-2.html").is_ok());
    }

    /// 前端只會傳 `reports_list` 給過的檔名，但這個指令不能假設呼叫端
    /// 守規矩——路徑穿越必須在這裡就被擋掉。
    #[test]
    fn safe_report_name_rejects_path_traversal() {
        for bad in [
            "../secrets.txt",
            "../../etc/passwd",
            "a/b.html",
            "a\\b.html",
            "/absolute.html",
            "..",
            "",
        ] {
            assert!(safe_report_name(bad).is_err(), "應該被拒絕：{bad}");
        }
    }

    #[test]
    fn report_filename_uses_the_timestamp() {
        let name = report_filename(&[], "2026-09-05-1430");
        assert_eq!(name, "2026-09-05-1430.html");
    }

    /// 同一分鐘內產第二份不可以覆蓋第一份。
    #[test]
    fn report_filename_avoids_collisions_within_the_same_minute() {
        let existing = vec!["2026-09-05-1430.html".to_string()];
        assert_eq!(report_filename(&existing, "2026-09-05-1430"), "2026-09-05-1430-2.html");

        let existing = vec![
            "2026-09-05-1430.html".to_string(),
            "2026-09-05-1430-2.html".to_string(),
        ];
        assert_eq!(report_filename(&existing, "2026-09-05-1430"), "2026-09-05-1430-3.html");
    }

    #[test]
    fn delete_report_file_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-09-06-0757.html");
        std::fs::write(&path, "<html></html>").unwrap();

        delete_report_file(&dir.path().to_path_buf(), "2026-09-06-0757.html").unwrap();
        assert!(!path.exists());
    }

    /// 重複點兩下、或兩個視窗同時刪同一份，都會走到「檔案已經不在」。
    /// 使用者要的結果是「這份報告不在了」，那個結果已經成立，報錯只會
    /// 造成困惑。
    #[test]
    fn delete_report_file_treats_a_missing_file_as_success() {
        let dir = tempfile::tempdir().unwrap();
        assert!(delete_report_file(&dir.path().to_path_buf(), "never-existed.html").is_ok());
    }

    #[test]
    fn title_from_html_reads_the_title_tag() {
        assert_eq!(
            title_from_html("<html><head><title>第三季進度</title></head><body></body></html>"),
            Some("第三季進度".to_string())
        );
    }

    #[test]
    fn title_from_html_is_none_when_there_is_no_title() {
        assert_eq!(title_from_html("<html><body>hi</body></html>"), None);
    }

    /// 模型寫出來的 HTML 標籤大小寫不受我們控制。
    #[test]
    fn title_from_html_is_case_insensitive_about_the_tag() {
        assert_eq!(
            title_from_html("<HTML><HEAD><TITLE>大寫標籤</TITLE></HEAD></HTML>"),
            Some("大寫標籤".to_string())
        );
    }

    /// `find_ci` 之所以在**原字串**上比對，而不是先 `to_lowercase()` 再
    /// 拿位移去切原字串，就是為了這種內容：土耳其文的 İ 小寫化之後位元組
    /// 長度會變，位移就對不上，輕則取到錯的範圍、重則從非字元邊界切下去
    /// 直接 panic。
    ///
    /// 已實際驗證這個測試會咬：把實作換回「先 to_lowercase 再取位移」，
    /// 這一條會失敗。
    #[test]
    fn title_from_html_survives_content_whose_lowercase_form_is_longer() {
        let html = "<html><title>İstanbul 專案</title></html>";
        assert_eq!(title_from_html(html), Some("İstanbul 專案".to_string()));
    }
}
