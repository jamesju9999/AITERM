//! Windows 的 PATH 在行程啟動時就固定了：使用者照提示裝完 git，系統設定裡有了新路徑，
//! 但已經在跑的 AITerm 讀不到，「重新檢查」與「git init」都還是找不到 git。
//! 這裡從系統設定（機器層＋使用者層）重新讀 PATH，把行程裡沒有的路徑補上。
//! 其他平台不做事。

/// `reg query` 的輸出裡取出值：`    Path    REG_EXPAND_SZ    C:\a;%SystemRoot%\b`。
fn parse_reg_value(output: &str) -> Option<String> {
    output.lines().find_map(|l| {
        let l = l.trim();
        if !l.get(..4).is_some_and(|n| n.eq_ignore_ascii_case("path")) {
            return None;
        }
        ["REG_EXPAND_SZ", "REG_SZ"].iter().find_map(|ty| {
            let at = l.find(ty)?;
            let v = l[at + ty.len()..].trim();
            (!v.is_empty()).then(|| v.to_string())
        })
    })
}

/// 展開 `%NAME%`；查不到的變數原樣保留。
fn expand_vars(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => match lookup(&after[..end]) {
                Some(v) => {
                    out.push_str(&v);
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn norm(p: &str) -> String {
    p.trim().trim_end_matches(['\\', '/']).to_lowercase()
}

/// 把 `fresh` 裡 `current` 還沒有的路徑接在後面（不分大小寫、忽略結尾斜線）；沒有新的就回 `None`。
fn merge_path(current: &str, fresh: &[String]) -> Option<String> {
    let mut have: Vec<String> = current.split(';').map(norm).filter(|p| !p.is_empty()).collect();
    let mut merged = current.trim_end_matches(';').to_string();
    let mut changed = false;
    for entry in fresh.iter().flat_map(|v| v.split(';')) {
        let n = norm(entry);
        if n.is_empty() || have.contains(&n) {
            continue;
        }
        have.push(n);
        if !merged.is_empty() {
            merged.push(';');
        }
        merged.push_str(entry.trim());
        changed = true;
    }
    changed.then_some(merged)
}

#[cfg(windows)]
fn read_registry_path(key: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg")
        .args(["query", key, "/v", "Path"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let v = parse_reg_value(&String::from_utf8_lossy(&out.stdout))?;
    Some(expand_vars(&v, |n| std::env::var(n).ok()))
}

/// 重新讀系統設定的 PATH，補進目前行程。
#[cfg(windows)]
pub async fn refresh() {
    let _ = tokio::task::spawn_blocking(|| {
        let fresh: Vec<String> = [
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            r"HKCU\Environment",
        ]
        .iter()
        .filter_map(|k| read_registry_path(k))
        .collect();
        let current = std::env::var("PATH").unwrap_or_default();
        if let Some(merged) = merge_path(&current, &fresh) {
            std::env::set_var("PATH", merged);
        }
    })
    .await;
}

#[cfg(not(windows))]
pub async fn refresh() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_value_out_of_reg_query_output() {
        let out = "\r\nHKEY_CURRENT_USER\\Environment\r\n    Path    REG_EXPAND_SZ    C:\\Users\\a b\\bin;%USERPROFILE%\\x\r\n\r\n";
        assert_eq!(parse_reg_value(out).as_deref(), Some("C:\\Users\\a b\\bin;%USERPROFILE%\\x"));
        assert_eq!(parse_reg_value("    PATH    REG_SZ    C:\\a").as_deref(), Some("C:\\a"));
        assert_eq!(parse_reg_value("    Temp    REG_SZ    C:\\t"), None);
        assert_eq!(parse_reg_value(""), None);
    }

    #[test]
    fn expands_known_variables_and_keeps_unknown_ones() {
        let look = |n: &str| (n == "SystemRoot").then(|| "C:\\Windows".to_string());
        assert_eq!(expand_vars("%SystemRoot%\\System32;%Nope%\\x", look), "C:\\Windows\\System32;%Nope%\\x");
        assert_eq!(expand_vars("100%;a", look), "100%;a");
        assert_eq!(expand_vars("", look), "");
    }

    #[test]
    fn merge_appends_only_paths_the_process_does_not_have() {
        let cur = "C:\\Windows;C:\\Tools\\";
        let fresh = vec!["c:\\windows;C:\\Program Files\\Git\\cmd".to_string(), "C:\\tools".to_string()];
        assert_eq!(merge_path(cur, &fresh).as_deref(), Some("C:\\Windows;C:\\Tools\\;C:\\Program Files\\Git\\cmd"));
        assert_eq!(merge_path(cur, &["C:\\WINDOWS".to_string()]), None, "沒有新路徑就不動");
        assert_eq!(merge_path("", &["C:\\a".to_string()]).as_deref(), Some("C:\\a"));
    }
}
