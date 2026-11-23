//! 附件：問題附的圖片與任意檔案（文件、log…）一律存到本機，Agent 與前端都從磁碟讀。
//!
//! 安全：圖片與否由**檔案內容的魔術位元組**決定，不信寄件者給的 Content-Type；
//! 檔名只取安全字元、副檔名只留短的英數；路徑由我們產生（項目 id + 序號），寄件者無法指定；
//! 檔案只當資料存放，不給執行權限，也絕不執行。
use std::path::{Path, PathBuf};

pub const MAX_FILES: usize = 10;
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
/// 一題所有附件加起來的上限，避免一封信塞滿磁碟。
pub const MAX_TOTAL_BYTES: usize = 60 * 1024 * 1024;

pub struct Raw {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub filename: String,
    pub path: String,
    pub mime: &'static str,
    pub size: i64,
}

/// 只接受常見的點陣圖。回傳 `(mime, 副檔名)`。
pub fn sniff(b: &[u8]) -> Option<(&'static str, &'static str)> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(("image/png", "png"))
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some(("image/gif", "gif"))
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

/// 看起來是純文字（UTF-8、前 8KB 沒有 NUL）：前端可以預覽、Agent 也能直接讀。
pub fn looks_like_text(b: &[u8]) -> bool {
    let head = &b[..b.len().min(8192)];
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // 8KB 剛好切在多位元組字元中間不算錯。
        Err(e) => e.error_len().is_none() && e.valid_up_to() + 4 > head.len(),
    }
}

/// 副檔名：取最後一個點之後的英數，最長 8 字、轉小寫；沒有就回 `None`。
pub fn safe_ext(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let (_, ext) = base.rsplit_once('.')?;
    (!ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric())).then(|| ext.to_ascii_lowercase())
}

/// 檔名主幹：只留英數、底線、連字號、點；其餘換底線；最長 40 字；空的給預設。
pub fn safe_stem(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    let s: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .take(40)
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "file".into() } else { s }
}

pub fn root() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM").join("intake-attachments")
}

/// 寫入磁碟。回傳成功存下的檔案與被略過的數量（空檔、單檔太大、超過個數或總量上限）。
/// 圖片以內容認定並用圖片副檔名；其他檔案依內容分成純文字或二進位，副檔名沿用寄件者的（清理過）。
pub fn save_all(root: &Path, item_id: &str, raws: Vec<Raw>) -> (Vec<Saved>, usize) {
    let dir = root.join(item_id);
    let mut saved = Vec::new();
    let mut skipped = 0;
    let mut total = 0usize;
    for raw in raws {
        if saved.len() >= MAX_FILES || raw.bytes.is_empty() || raw.bytes.len() > MAX_BYTES || total + raw.bytes.len() > MAX_TOTAL_BYTES {
            skipped += 1;
            continue;
        }
        let (mime, ext) = match sniff(&raw.bytes) {
            Some((m, e)) => (m, e.to_string()),
            None if looks_like_text(&raw.bytes) => ("text/plain", safe_ext(&raw.name).unwrap_or_else(|| "txt".into())),
            None => ("application/octet-stream", safe_ext(&raw.name).unwrap_or_else(|| "bin".into())),
        };
        let filename = format!("{}-{}.{}", saved.len() + 1, safe_stem(&raw.name), ext);
        let path = dir.join(&filename);
        if std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &raw.bytes)).is_err() {
            skipped += 1;
            continue;
        }
        total += raw.bytes.len();
        saved.push(Saved { filename, path: path.to_string_lossy().into_owned(), mime, size: raw.bytes.len() as i64 });
    }
    (saved, skipped)
}

pub fn remove_dir(root: &Path, item_id: &str) {
    let _ = std::fs::remove_dir_all(root.join(item_id));
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
    fn raw(name: &str, bytes: &[u8]) -> Raw {
        Raw { name: name.into(), bytes: bytes.to_vec() }
    }

    #[test]
    fn kind_comes_from_content_not_from_the_claimed_name() {
        assert_eq!(sniff(PNG), Some(("image/png", "png")));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(("image/jpeg", "jpg")));
        assert_eq!(sniff(b"GIF89a.."), Some(("image/gif", "gif")));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(("image/webp", "webp")));
        assert_eq!(sniff(b"<svg onload=alert(1)>"), None, "SVG 可夾腳本，不收");
        assert_eq!(sniff(b"MZ\x90\x00 exe"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn hostile_file_names_cannot_escape_or_set_the_extension() {
        assert_eq!(safe_stem("../../etc/passwd"), "passwd");
        assert_eq!(safe_stem("C:\\Users\\x\\evil.exe"), "evil");
        assert_eq!(safe_stem("截圖 2026.png"), "2026");
        assert_eq!(safe_stem(""), "file");
        assert_eq!(safe_stem("....png"), "file");
        assert!(safe_stem(&"a".repeat(200)).len() <= 40);
    }

    #[test]
    fn saves_images_and_any_other_file_with_our_own_names() {
        let d = tempfile::tempdir().unwrap();
        let (saved, skipped) = save_all(
            d.path(),
            "item1",
            vec![
                raw("../../shot.exe", PNG),
                raw("server.log", b"2026-11-06 ERROR boom\n"),
                raw("a.jpg", &[0xFF, 0xD8, 0xFF, 1]),
                raw("empty.png", b""),
                raw("報告.pdf", b"%PDF-1.7\n\0\x01binary"),
                raw("noext", b"plain text without extension"),
            ],
        );
        assert_eq!(skipped, 1, "只有空檔被略過");
        let names: Vec<&str> = saved.iter().map(|s| s.filename.as_str()).collect();
        assert_eq!(names, vec!["1-shot.png", "2-server.log", "3-a.jpg", "4-file.pdf", "5-noext.txt"]);
        let mimes: Vec<&str> = saved.iter().map(|s| s.mime).collect();
        assert_eq!(mimes, vec!["image/png", "text/plain", "image/jpeg", "application/octet-stream", "text/plain"]);
        for s in &saved {
            assert!(Path::new(&s.path).starts_with(d.path().join("item1")), "{}", s.path);
            assert!(Path::new(&s.path).exists());
        }
    }

    #[test]
    fn a_file_pretending_to_be_an_image_is_just_a_file_and_gets_no_exec_permission() {
        let d = tempfile::tempdir().unwrap();
        let (saved, _) = save_all(d.path(), "i", vec![raw("run.sh", b"#!/bin/sh\nrm -rf ~\n"), raw("fake.png", b"MZ\x90\x00 exe")]);
        assert_eq!(saved[0].mime, "text/plain");
        assert_eq!(saved[1].mime, "application/octet-stream", "不是圖片內容就不當圖片，副檔名 png 也一樣");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for s in &saved {
                assert_eq!(std::fs::metadata(&s.path).unwrap().permissions().mode() & 0o111, 0, "{}", s.filename);
            }
        }
    }

    #[test]
    fn extensions_are_cleaned_and_text_detection_tolerates_a_cut_multibyte_char() {
        assert_eq!(safe_ext("a.LOG").as_deref(), Some("log"));
        assert_eq!(safe_ext("archive.tar.gz").as_deref(), Some("gz"));
        assert_eq!(safe_ext("evil.p h p"), None, "含空白的副檔名不採用");
        assert_eq!(safe_ext("noext"), None);
        assert_eq!(safe_ext("x.verylongextension"), None);
        assert!(looks_like_text("純文字 log\n".as_bytes()));
        assert!(!looks_like_text(b"a\0b"));
        let mut long = "中".repeat(3000).into_bytes();
        long.truncate(8191); // 切在多位元組字元中間
        assert!(looks_like_text(&long));
        assert!(!looks_like_text(&[0xFF, 0xFE, 0xFD, 0x80]));
    }

    #[test]
    fn enforces_count_size_and_total_limits() {
        let d = tempfile::tempdir().unwrap();
        let many: Vec<Raw> = (0..MAX_FILES + 2).map(|i| raw(&format!("{i}.png"), PNG)).collect();
        let (saved, skipped) = save_all(d.path(), "i", many);
        assert_eq!((saved.len(), skipped), (MAX_FILES, 2));
        let mut big = PNG.to_vec();
        big.resize(MAX_BYTES + 1, 0);
        let (saved, skipped) = save_all(d.path(), "j", vec![raw("big.png", &big)]);
        assert_eq!((saved.len(), skipped), (0, 1));
        // 每個都在單檔上限內，但加起來超過總量上限：後面的被略過。
        let chunk = vec![b'x'; MAX_BYTES];
        let (saved, skipped) = save_all(d.path(), "k", (0..5).map(|i| raw(&format!("{i}.log"), &chunk)).collect());
        assert_eq!((saved.len(), skipped), (MAX_TOTAL_BYTES / MAX_BYTES, 5 - MAX_TOTAL_BYTES / MAX_BYTES));
    }
}
