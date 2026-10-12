//! Windows 後端用到的純邏輯：不呼叫任何 Win32 API，所以在任何平台（含 macOS 開發機與三個平台的 CI）
//! 都能測試。`windows.rs` 只負責把這些結果交給系統。

use super::backend::KeyCombo;

// ── UI Automation 控制項類型 → 與 macOS 相同的角色名稱 ───────────────────────────────

/// `UIA_ControlTypeIds`（50000…）→ 與 macOS 一致的 `AX…` 角色，這樣提示詞、工具說明、
/// 「純容器」「選單」等判斷都不必分平台。不認得的回 `AXUnknown`。
pub fn uia_role(control_type: i32) -> &'static str {
    match control_type {
        50000 => "AXButton",
        50002 => "AXCheckBox",
        50003 => "AXPopUpButton", // ComboBox
        50004 => "AXTextField",   // Edit
        50005 => "AXLink",
        50006 => "AXImage",
        50007 | 50029 | 50024 => "AXRow", // ListItem / DataItem / TreeItem
        50008 => "AXList",
        50009 => "AXMenu",
        50010 => "AXMenuBar",
        50011 => "AXMenuItem",
        50012 => "AXProgressIndicator",
        50013 => "AXRadioButton",
        50014 => "AXScrollBar",
        50015 => "AXSlider",
        50016 => "AXIncrementor", // Spinner
        50018 => "AXTabGroup",
        50019 => "AXTab",
        50020 => "AXStaticText", // Text
        50021 => "AXToolbar",
        50023 => "AXOutline", // Tree
        50028 | 50036 => "AXTable",
        50030 => "AXTextArea", // Document
        50031 => "AXMenuButton", // SplitButton
        50032 => "AXWindow",
        50038 => "AXSplitter",
        // Pane、Group、Custom、TitleBar、StatusBar、Header…：版面容器。
        50017 | 50025 | 50026 | 50033 | 50034 | 50035 | 50037 => "AXGroup",
        _ => "AXUnknown",
    }
}

// ── 按鍵 ──────────────────────────────────────────────────────────────────────────────

pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12; // Alt

/// 我們的標準按鍵名稱（見 `policy::parse_key`）→ 虛擬鍵碼（美式配置的 OEM 鍵）。
pub fn vk_for_key(key: &str) -> Option<u16> {
    let named = match key {
        "return" => 0x0D,
        "tab" => 0x09,
        "space" => 0x20,
        "escape" => 0x1B,
        "delete" => 0x08, // 我們的 delete 是「往回刪」＝ Backspace
        "forwarddelete" => 0x2E,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "-" => 0xBD,
        "=" => 0xBB,
        "[" => 0xDB,
        "]" => 0xDD,
        "\\" => 0xDC,
        ";" => 0xBA,
        "'" => 0xDE,
        "," => 0xBC,
        "." => 0xBE,
        "/" => 0xBF,
        "`" => 0xC0,
        _ => 0,
    };
    if named != 0 {
        return Some(named);
    }
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=12).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase() as u16),
        (Some(c), None) if c.is_ascii_digit() => Some(c as u16),
        _ => None,
    }
}

/// 要先按下的修飾鍵（依序）。`cmd` 在 Windows 對應 Ctrl（模型習慣寫 `cmd+s`），與 `ctrl` 並用時只按一次。
pub fn modifier_keys(c: &KeyCombo) -> Vec<u16> {
    let mut v = Vec::new();
    if c.cmd || c.ctrl {
        v.push(VK_CONTROL);
    }
    if c.alt {
        v.push(VK_MENU);
    }
    if c.shift {
        v.push(VK_SHIFT);
    }
    v
}

/// 方向鍵、Home／End、PageUp／Down、Delete／Insert 要帶 `KEYEVENTF_EXTENDEDKEY`，
/// 否則有些程式會把它們當成數字鍵盤的按鍵。
pub fn is_extended_key(vk: u16) -> bool {
    matches!(vk, 0x21..=0x28 | 0x2D | 0x2E)
}

// ── 滑鼠 ──────────────────────────────────────────────────────────────────────────────

/// 螢幕座標 → `SendInput` 絕對滑鼠座標（0..=65535，對應整個虛擬桌面）。
/// 虛擬桌面原點可以是負的（副螢幕在左邊或上面）。超出範圍的座標夾在邊界上。
pub fn to_absolute(x: f64, y: f64, left: i32, top: i32, width: i32, height: i32) -> (i32, i32) {
    let norm = |v: f64, origin: i32, size: i32| -> i32 {
        let span = (size.max(2) - 1) as f64;
        (((v - origin as f64) * 65535.0 / span).round()).clamp(0.0, 65535.0) as i32
    };
    (norm(x, left, width), norm(y, top, height))
}

/// `(dx, dy)` 行數 → `(垂直, 水平)` 滾輪量（每行 120）。`dy` 正＝往上、`dx` 正＝往右，
/// 與 Windows 的 `WHEEL_DELTA` 方向一致。
pub fn wheel_amounts(dx: i32, dy: i32) -> (i32, i32) {
    (dy.saturating_mul(120), dx.saturating_mul(120))
}

// ── 文字輸入 ─────────────────────────────────────────────────────────────────────────

/// 用 `KEYEVENTF_UNICODE` 逐個 UTF-16 單位輸入：不受鍵盤配置與輸入法影響（中文、emoji 都行）。
/// 回傳 `(單位, 是否為放開)`，每個單位依序一次按下與一次放開。
pub fn unicode_key_events(text: &str) -> Vec<(u16, bool)> {
    text.encode_utf16().flat_map(|u| [(u, false), (u, true)]).collect()
}

// ── App 清單 ─────────────────────────────────────────────────────────────────────────

fn file_name(path: &str) -> Option<&str> {
    path.rsplit(['\\', '/']).next().filter(|n| !n.is_empty())
}

/// `C:\Windows\System32\notepad.exe` → `notepad`（行程名稱，模型與使用者最常用的叫法）。
pub fn process_name_from_path(path: &str) -> Option<String> {
    let name = file_name(path)?;
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    (!stem.is_empty()).then(|| stem.to_string())
}

/// 小寫的完整檔名（`notepad.exe`），當作這個 App 的穩定識別（對應 macOS 的 bundle id）。
pub fn app_id_from_path(path: &str) -> Option<String> {
    file_name(path).map(|n| n.to_lowercase())
}

/// 哪些頂層視窗算「使用者看得到的 App 視窗」：可見、沒被 DWM 隱藏（cloaked）、不是工具視窗、
/// 沒有擁有者（對話框屬於它的擁有者）、有標題。
pub fn is_app_window(visible: bool, cloaked: bool, tool_window: bool, has_owner: bool, title: &str) -> bool {
    visible && !cloaked && !tool_window && !has_owner && !title.trim().is_empty()
}

/// DPI → 縮放比（96 為 100%）。偵測不到時給 1.0。
pub fn scale_from_dpi(dpi: u32) -> f64 {
    if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::policy::parse_key;

    #[test]
    fn uia_control_types_map_to_the_same_roles_macos_uses() {
        assert_eq!(uia_role(50000), "AXButton");
        assert_eq!(uia_role(50004), "AXTextField");
        assert_eq!(uia_role(50032), "AXWindow");
        assert_eq!(uia_role(50010), "AXMenuBar");
        assert_eq!(uia_role(50011), "AXMenuItem");
        assert_eq!(uia_role(50020), "AXStaticText");
        assert_eq!(uia_role(50007), "AXRow");
        // 版面容器要對到 AXGroup，精簡元件樹才會把「沒有標題的容器」省略。
        for ct in [50033, 50026, 50025, 50037] {
            assert_eq!(uia_role(ct), "AXGroup", "{ct}");
        }
        assert_eq!(uia_role(99999), "AXUnknown");
        assert_eq!(uia_role(-1), "AXUnknown");
    }

    #[test]
    fn every_role_the_windows_mapping_can_return_is_one_the_shared_code_understands() {
        // 共用邏輯（精簡樹、稀疏判斷）靠角色字串；對不上就會悄悄失效。
        for ct in 50000..=50040 {
            let r = uia_role(ct);
            assert!(r.starts_with("AX"), "{ct} -> {r}");
        }
    }

    #[test]
    fn letters_digits_function_keys_and_named_keys_have_virtual_key_codes() {
        assert_eq!(vk_for_key("a"), Some(0x41));
        assert_eq!(vk_for_key("z"), Some(0x5A));
        assert_eq!(vk_for_key("0"), Some(0x30));
        assert_eq!(vk_for_key("9"), Some(0x39));
        assert_eq!(vk_for_key("f1"), Some(0x70));
        assert_eq!(vk_for_key("f12"), Some(0x7B));
        assert_eq!(vk_for_key("return"), Some(0x0D));
        assert_eq!(vk_for_key("delete"), Some(0x08)); // 往回刪
        assert_eq!(vk_for_key("forwarddelete"), Some(0x2E));
        assert_eq!(vk_for_key("left"), Some(0x25));
        assert_eq!(vk_for_key("`"), Some(0xC0));
        assert_eq!(vk_for_key("f13"), None);
        assert_eq!(vk_for_key("nope"), None);
        assert_eq!(vk_for_key(""), None);
    }

    #[test]
    fn every_key_the_parser_accepts_has_a_virtual_key_on_windows() {
        for k in ["a", "z", "0", "9", "return", "tab", "space", "escape", "delete", "forwarddelete", "left", "right", "up",
                  "down", "home", "end", "pageup", "pagedown", "f1", "f12", "-", "=", "[", "]", "\\", ";", "'", ",", ".", "/", "`"] {
            assert!(parse_key(k).is_ok(), "{k}");
            assert!(vk_for_key(k).is_some(), "{k}");
        }
    }

    #[test]
    fn cmd_becomes_ctrl_and_is_not_pressed_twice_with_ctrl() {
        let m = |s: &str| modifier_keys(&parse_key(s).unwrap());
        assert_eq!(m("cmd+s"), vec![VK_CONTROL]);
        assert_eq!(m("ctrl+s"), vec![VK_CONTROL]);
        assert_eq!(m("cmd+ctrl+s"), vec![VK_CONTROL]);
        assert_eq!(m("ctrl+alt+delete"), vec![VK_CONTROL, VK_MENU]);
        assert_eq!(m("cmd+shift+left"), vec![VK_CONTROL, VK_SHIFT]);
        assert!(m("return").is_empty());
    }

    #[test]
    fn navigation_keys_are_extended_and_letters_are_not() {
        for vk in [0x25, 0x26, 0x27, 0x28, 0x21, 0x22, 0x23, 0x24, 0x2E] {
            assert!(is_extended_key(vk), "{vk:#x}");
        }
        for vk in [0x41, 0x0D, 0x08, 0x20] {
            assert!(!is_extended_key(vk), "{vk:#x}");
        }
    }

    #[test]
    fn absolute_mouse_coordinates_span_the_virtual_desktop_including_negative_origins() {
        // 單一 1920x1080
        assert_eq!(to_absolute(0.0, 0.0, 0, 0, 1920, 1080), (0, 0));
        assert_eq!(to_absolute(1919.0, 1079.0, 0, 0, 1920, 1080), (65535, 65535));
        let (mid, _) = to_absolute(960.0, 540.0, 0, 0, 1920, 1080);
        assert!((mid - 32784).abs() <= 1, "{mid}");
        // 副螢幕在左邊：虛擬桌面從 -1920 開始，總寬 3840
        assert_eq!(to_absolute(-1920.0, 0.0, -1920, 0, 3840, 1080).0, 0);
        assert_eq!(to_absolute(1919.0, 0.0, -1920, 0, 3840, 1080).0, 65535);
        // 超出範圍夾在邊界，不會溢位
        assert_eq!(to_absolute(-99999.0, 99999.0, 0, 0, 1920, 1080), (0, 65535));
        // 退化的尺寸不除以零
        let _ = to_absolute(5.0, 5.0, 0, 0, 0, 0);
        let _ = to_absolute(5.0, 5.0, 0, 0, 1, 1);
    }

    #[test]
    fn wheel_amounts_use_120_per_line_with_up_and_right_positive() {
        assert_eq!(wheel_amounts(0, 3), (360, 0));
        assert_eq!(wheel_amounts(-2, -1), (-120, -240));
        assert_eq!(wheel_amounts(0, 0), (0, 0));
        assert_eq!(wheel_amounts(0, i32::MAX), (i32::MAX, 0)); // 飽和，不溢位
    }

    #[test]
    fn unicode_events_come_in_down_up_pairs_and_split_surrogate_pairs() {
        assert_eq!(unicode_key_events("a"), vec![(0x61, false), (0x61, true)]);
        let e = unicode_key_events("你");
        assert_eq!(e, vec![(0x4F60, false), (0x4F60, true)]);
        // emoji 在 UTF-16 是兩個單位（代理對），要各送一對。
        let emoji = unicode_key_events("😀");
        assert_eq!(emoji.len(), 4);
        assert_eq!(emoji[0].0, 0xD83D);
        assert_eq!(emoji[2].0, 0xDE00);
        assert!(unicode_key_events("").is_empty());
    }

    #[test]
    fn process_and_app_ids_come_from_the_executable_path() {
        assert_eq!(process_name_from_path(r"C:\Windows\System32\notepad.exe").as_deref(), Some("notepad"));
        assert_eq!(process_name_from_path(r"C:\Program Files\Google\Chrome\Application\chrome.exe").as_deref(), Some("chrome"));
        assert_eq!(process_name_from_path("/usr/bin/foo").as_deref(), Some("foo"));
        assert_eq!(process_name_from_path("Calc").as_deref(), Some("Calc"));
        assert_eq!(process_name_from_path(r"C:\x\").as_deref(), None);
        assert_eq!(process_name_from_path("").as_deref(), None);
        assert_eq!(app_id_from_path(r"C:\Windows\System32\Notepad.EXE").as_deref(), Some("notepad.exe"));
        assert_eq!(app_id_from_path("").as_deref(), None);
    }

    #[test]
    fn only_real_visible_unowned_titled_windows_count_as_apps() {
        assert!(is_app_window(true, false, false, false, "無標題 - 記事本"));
        assert!(!is_app_window(false, false, false, false, "x")); // 不可見
        assert!(!is_app_window(true, true, false, false, "x")); // 被 DWM 隱藏（其他虛擬桌面、UWP 暫停）
        assert!(!is_app_window(true, false, true, false, "x")); // 工具視窗
        assert!(!is_app_window(true, false, false, true, "x")); // 對話框（有擁有者）
        assert!(!is_app_window(true, false, false, false, "  ")); // 沒標題
    }

    #[test]
    fn dpi_converts_to_a_scale_factor() {
        assert_eq!(scale_from_dpi(96), 1.0);
        assert_eq!(scale_from_dpi(144), 1.5);
        assert_eq!(scale_from_dpi(192), 2.0);
        assert_eq!(scale_from_dpi(0), 1.0);
    }
}
