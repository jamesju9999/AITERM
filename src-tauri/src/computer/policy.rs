//! 純邏輯的安全判斷：什麼算破壞性、按鍵字串怎麼解析。

use super::backend::KeyCombo;

/// 標題／描述含這些字樣的元件，點擊前每次都要使用者確認。
/// 比對不分大小寫，中英文都有；寧可多問，不要漏問。
const DANGEROUS_WORDS: &[&str] = &[
    "刪除", "移除", "清空", "送出", "發送", "傳送", "付款", "購買", "支付", "結帳", "確認付款", "格式化",
    "delete", "remove", "erase", "empty trash", "send", "pay", "purchase", "buy", "submit",
    "checkout", "place order", "format", "wipe", "reset",
];

pub fn is_dangerous_label(label: &str) -> bool {
    let l = label.to_lowercase();
    DANGEROUS_WORDS.iter().any(|w| l.contains(w))
}

/// 本身就帶破壞性的按鍵（Finder 的 Cmd+Delete 會把檔案丟進垃圾桶）。
pub fn is_dangerous_key(c: &KeyCombo) -> bool {
    c.cmd && matches!(c.key.as_str(), "delete" | "backspace" | "forwarddelete")
}

const NAMED_KEYS: &[&str] = &[
    "return", "enter", "tab", "space", "escape", "delete", "backspace", "forwarddelete",
    "left", "right", "up", "down", "home", "end", "pageup", "pagedown",
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
];

/// 解析 `cmd+shift+s`、`return`、`ctrl+alt+left`。修飾鍵別名：
/// `command`／`cmd`／`meta`、`control`／`ctrl`、`option`／`alt`／`opt`、`shift`。
pub fn parse_key(spec: &str) -> Result<KeyCombo, String> {
    let mut combo = KeyCombo { key: String::new(), cmd: false, ctrl: false, alt: false, shift: false };
    let parts: Vec<&str> = spec.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err("按鍵不可為空".to_string());
    }
    for (i, raw) in parts.iter().enumerate() {
        let p = raw.to_lowercase();
        let last = i == parts.len() - 1;
        match p.as_str() {
            "cmd" | "command" | "meta" if !last => combo.cmd = true,
            "ctrl" | "control" if !last => combo.ctrl = true,
            "alt" | "option" | "opt" if !last => combo.alt = true,
            "shift" if !last => combo.shift = true,
            _ if last => {
                let single = p.chars().count() == 1 && p.chars().all(|c| c.is_ascii_alphanumeric() || "-=[];',./`\\".contains(c));
                if single || NAMED_KEYS.contains(&p.as_str()) {
                    combo.key = if p == "enter" { "return".to_string() } else if p == "backspace" { "delete".to_string() } else { p };
                } else {
                    return Err(format!("不認得的按鍵「{raw}」"));
                }
            }
            _ => return Err(format!("不認得的修飾鍵「{raw}」")),
        }
    }
    if combo.key.is_empty() {
        return Err("缺少主按鍵".to_string());
    }
    Ok(combo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_labels_match_in_both_languages_and_any_case() {
        for l in ["刪除", "Delete", "DELETE FOREVER", "Send message", "確認付款", "Empty Trash…", "Place Order"] {
            assert!(is_dangerous_label(l), "{l}");
        }
    }

    #[test]
    fn harmless_labels_pass() {
        for l in ["Save", "儲存", "Cancel", "OK", "Open", ""] {
            assert!(!is_dangerous_label(l), "{l}");
        }
    }

    #[test]
    fn parses_modifiers_aliases_and_canonical_names() {
        let c = parse_key("Command+Shift+S").unwrap();
        assert_eq!((c.cmd, c.shift, c.ctrl, c.alt, c.key.as_str()), (true, true, false, false, "s"));
        assert_eq!(parse_key("enter").unwrap().key, "return");
        assert_eq!(parse_key("ctrl+option+left").unwrap().alt, true);
    }

    #[test]
    fn rejects_unknown_and_malformed_keys() {
        assert!(parse_key("").is_err());
        assert!(parse_key("cmd+").is_err());
        assert!(parse_key("cmd+shift").is_err()); // 只有修飾鍵
        assert!(parse_key("hyper+s").is_err());
        assert!(parse_key("cmd+bogus").is_err());
    }

    #[test]
    fn cmd_delete_is_dangerous_plain_delete_is_not() {
        assert!(is_dangerous_key(&parse_key("cmd+delete").unwrap()));
        assert!(is_dangerous_key(&parse_key("cmd+backspace").unwrap()));
        assert!(!is_dangerous_key(&parse_key("delete").unwrap()));
        assert!(!is_dangerous_key(&parse_key("cmd+s").unwrap()));
    }
}
