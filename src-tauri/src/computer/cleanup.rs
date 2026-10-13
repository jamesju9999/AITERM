//! 啟動時清掉電腦操作留在系統暫存資料夾的殘檔。
//!
//! 截圖、文字辨識、圖示偵測的暫存圖在每次操作結束就會刪除；只有 App 在那一瞬間被強制結束或當機，
//! 才會留下。這裡把超過一段時間的殘檔掃掉，免得長期累積。只認我們自己的檔名前綴與 `.png` 副檔名。

use std::path::Path;
use std::time::{Duration, SystemTime};

/// 我們自己建立的暫存檔前綴（見 macos.rs／linux.rs）。
const PREFIXES: [&str; 3] = ["aiterm-shot-", "aiterm-ocr-", "aiterm-icons-"];

/// 超過這個時間還在的暫存圖一定是殘檔（單次操作不會跑這麼久）。
pub const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

fn is_ours(name: &str) -> bool {
    name.ends_with(".png") && PREFIXES.iter().any(|p| name.starts_with(p))
}

/// 刪除 `dir` 裡修改時間早於 `now - max_age` 的殘檔，回傳刪掉的數量。目錄讀不到、單檔刪不掉都略過。
pub fn sweep_stale_temp(dir: &Path, max_age: Duration, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_ours(name) {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else { continue };
        let age = now.duration_since(modified).unwrap_or_default();
        if age >= max_age && std::fs::remove_file(e.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 啟動時呼叫：掃系統暫存資料夾。放到背景執行緒，不拖慢啟動。
pub fn spawn_startup_sweep() {
    std::thread::spawn(|| {
        let n = sweep_stale_temp(&std::env::temp_dir(), STALE_AFTER, SystemTime::now());
        if n > 0 {
            log::info!("已清除 {n} 個電腦操作留下的暫存圖");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn make(dir: &Path, name: &str, age: Duration) {
        let f = File::create(dir.join(name)).unwrap();
        f.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn removes_only_old_files_with_our_prefixes() {
        let d = tempfile::tempdir().unwrap();
        let old = Duration::from_secs(2 * 3600);
        let fresh = Duration::from_secs(60);
        make(d.path(), "aiterm-shot-1.png", old);
        make(d.path(), "aiterm-ocr-2.png", old);
        make(d.path(), "aiterm-icons-3.png", old);
        make(d.path(), "aiterm-shot-4-s.png", fresh); // 可能是正在進行的操作
        make(d.path(), "holiday.png", old); // 不是我們的
        make(d.path(), "aiterm-ocr-5.txt", old); // 副檔名不對
        make(d.path(), "other-aiterm-shot-6.png", old); // 前綴只能在開頭
        let n = sweep_stale_temp(d.path(), STALE_AFTER, SystemTime::now());
        assert_eq!(n, 3);
        let mut left: Vec<String> = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, vec!["aiterm-ocr-5.txt", "aiterm-shot-4-s.png", "holiday.png", "other-aiterm-shot-6.png"]);
    }

    #[test]
    fn leaves_directories_alone_and_survives_a_missing_dir() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("aiterm-shot-dir.png")).unwrap();
        assert_eq!(sweep_stale_temp(d.path(), Duration::ZERO, SystemTime::now()), 0);
        assert_eq!(sweep_stale_temp(&d.path().join("nope"), Duration::ZERO, SystemTime::now()), 0);
    }
}
