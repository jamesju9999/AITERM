//! 專案顯示名稱 → 可安全建立的資料夾名稱。
//!
//! 一律套用 Windows 的規則，即使在 macOS/Linux 上也一樣——這樣同一個
//! 專案名在三個平台產生相同的資料夾名，專案資料夾複製到別台機器
//! 不會因為平台差異而變成兩個不同的名字。

/// Windows 保留的裝置名稱（不分大小寫，且含副檔名時同樣保留）。
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

pub fn safe_folder_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if (c as u32) < 0x20 => '-',
            c => c,
        })
        .collect();

    // Windows 無法建立以 '.' 或空白結尾的資料夾；開頭空白也一併去掉。
    let trimmed = replaced.trim().trim_end_matches(['.', ' ']).trim();

    if trimmed.is_empty() || trimmed.chars().all(|c| c == '-') {
        return "project".to_string();
    }

    // 保留裝置名稱：整個名稱（或副檔名前的部分）等於保留字時加底線。
    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return format!("{trimmed}_");
    }

    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_an_already_safe_name() {
        assert_eq!(safe_folder_name("makemoney"), "makemoney");
        assert_eq!(safe_folder_name("賺錢計畫"), "賺錢計畫");
    }

    #[test]
    fn replaces_windows_illegal_characters() {
        assert_eq!(safe_folder_name("a/b\\c:d*e?f\"g<h>i|j"), "a-b-c-d-e-f-g-h-i-j");
    }

    #[test]
    fn escapes_windows_reserved_device_names_case_insensitively() {
        assert_eq!(safe_folder_name("CON"), "CON_");
        assert_eq!(safe_folder_name("con"), "con_");
        assert_eq!(safe_folder_name("COM1"), "COM1_");
        assert_eq!(safe_folder_name("lpt9"), "lpt9_");
        // 只是開頭像保留名稱的一般名稱不受影響
        assert_eq!(safe_folder_name("console"), "console");
    }

    #[test]
    fn trims_trailing_dots_and_spaces() {
        // Windows 無法建立以 . 或空白結尾的資料夾
        assert_eq!(safe_folder_name("name. "), "name");
        assert_eq!(safe_folder_name("  name  "), "name");
    }

    #[test]
    fn an_empty_or_all_illegal_name_falls_back() {
        assert_eq!(safe_folder_name(""), "project");
        assert_eq!(safe_folder_name("///"), "project");
        assert_eq!(safe_folder_name("..."), "project");
    }
}
