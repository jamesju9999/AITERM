//! 附圖：問題附的圖片一律存到本機，Agent 與前端都從磁碟讀。
//!
//! 安全：副檔名與 MIME 由**檔案內容的魔術位元組**決定，不信寄件者給的檔名或 Content-Type；
//! 檔名只取安全字元；路徑由我們產生（項目 id + 序號），寄件者無法指定。
use std::path::{Path, PathBuf};

pub const MAX_FILES: usize = 5;
pub const MAX_BYTES: usize = 10 * 1024 * 1024;

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
    if s.is_empty() { "image".into() } else { s }
}

pub fn root() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM").join("intake-attachments")
}

/// 驗證並寫入磁碟。回傳成功存下的檔案與被略過的數量（格式不符、太大、超過張數上限）。
pub fn save_all(root: &Path, item_id: &str, raws: Vec<Raw>) -> (Vec<Saved>, usize) {
    let dir = root.join(item_id);
    let mut saved = Vec::new();
    let mut skipped = 0;
    for raw in raws {
        if saved.len() >= MAX_FILES || raw.bytes.is_empty() || raw.bytes.len() > MAX_BYTES {
            skipped += 1;
            continue;
        }
        let Some((mime, ext)) = sniff(&raw.bytes) else {
            skipped += 1;
            continue;
        };
        let filename = format!("{}-{}.{}", saved.len() + 1, safe_stem(&raw.name), ext);
        let path = dir.join(&filename);
        if std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &raw.bytes)).is_err() {
            skipped += 1;
            continue;
        }
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
        assert_eq!(safe_stem(""), "image");
        assert_eq!(safe_stem("....png"), "image");
        assert!(safe_stem(&"a".repeat(200)).len() <= 40);
    }

    #[test]
    fn saves_only_real_images_with_our_own_names_and_counts_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let (saved, skipped) = save_all(
            d.path(),
            "item1",
            vec![raw("../../shot.exe", PNG), raw("notes.txt", b"hello"), raw("a.jpg", &[0xFF, 0xD8, 0xFF, 1]), raw("empty.png", b"")],
        );
        assert_eq!(skipped, 2);
        assert_eq!(saved.iter().map(|s| s.filename.as_str()).collect::<Vec<_>>(), vec!["1-shot.png", "2-a.jpg"]);
        for s in &saved {
            assert!(Path::new(&s.path).starts_with(d.path().join("item1")), "{}", s.path);
            assert!(Path::new(&s.path).exists());
        }
        assert_eq!(saved[0].mime, "image/png");
    }

    #[test]
    fn enforces_count_and_size_limits() {
        let d = tempfile::tempdir().unwrap();
        let many: Vec<Raw> = (0..MAX_FILES + 2).map(|i| raw(&format!("{i}.png"), PNG)).collect();
        let (saved, skipped) = save_all(d.path(), "i", many);
        assert_eq!((saved.len(), skipped), (MAX_FILES, 2));
        let mut big = PNG.to_vec();
        big.resize(MAX_BYTES + 1, 0);
        let (saved, skipped) = save_all(d.path(), "j", vec![raw("big.png", &big)]);
        assert_eq!((saved.len(), skipped), (0, 1));
    }
}
