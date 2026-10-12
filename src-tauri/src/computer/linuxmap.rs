//! Linux（X11）後端用到的純邏輯：不連 X server、不啟動任何行程，所以在任何平台都能測試。

use super::backend::{KeyCombo, ScreenText};

// ── 按鍵：我們的標準名稱 → X11 keysym ──────────────────────────────────────────────

pub const XK_SHIFT_L: u32 = 0xffe1;
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_ALT_L: u32 = 0xffe9;

/// 標準按鍵名稱（見 `policy::parse_key`）→ keysym。`delete` 是我們的「往回刪」＝ BackSpace。
pub fn keysym_for_key(key: &str) -> Option<u32> {
    let named = match key {
        "return" => 0xff0d,
        "tab" => 0xff09,
        "space" => 0x20,
        "escape" => 0xff1b,
        "delete" => 0xff08,
        "forwarddelete" => 0xffff,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        _ => 0,
    };
    if named != 0 {
        return Some(named);
    }
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=12).contains(&n) {
            return Some(0xffbe + n - 1);
        }
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        // 可印出的 ASCII：keysym 就是字元本身（字母用小寫）。
        (Some(c), None) if c.is_ascii_graphic() => Some(c.to_ascii_lowercase() as u32),
        _ => None,
    }
}

/// 要先按下的修飾鍵 keysym。`cmd` 在 Linux 對應 Ctrl（模型習慣寫 `cmd+s`），與 `ctrl` 並用時只按一次。
pub fn modifier_keysyms(c: &KeyCombo) -> Vec<u32> {
    let mut v = Vec::new();
    if c.cmd || c.ctrl {
        v.push(XK_CONTROL_L);
    }
    if c.alt {
        v.push(XK_ALT_L);
    }
    if c.shift {
        v.push(XK_SHIFT_L);
    }
    v
}

/// 任意字元的 keysym：Latin-1 範圍直接用字碼，其餘用 Unicode keysym（`0x01000000 | 碼位`）。
pub fn char_keysym(c: char) -> u32 {
    let cp = c as u32;
    if (0x20..=0x7e).contains(&cp) || (0xa0..=0xff).contains(&cp) {
        cp
    } else {
        0x0100_0000 | cp
    }
}

// ── 滑鼠 ──────────────────────────────────────────────────────────────────────────────

/// X11 的滾輪是按鈕事件：4 上、5 下、6 左、7 右。`dy` 正＝往上、`dx` 正＝往右。
/// 回傳 `(按鈕, 次數)`，次數已取絕對值並限制在合理範圍。
pub fn scroll_buttons(dx: i32, dy: i32) -> Vec<(u8, u32)> {
    let mut out = Vec::new();
    if dy != 0 {
        out.push((if dy > 0 { 4 } else { 5 }, dy.unsigned_abs().min(200)));
    }
    if dx != 0 {
        out.push((if dx > 0 { 7 } else { 6 }, dx.unsigned_abs().min(200)));
    }
    out
}

// ── 工作階段 ─────────────────────────────────────────────────────────────────────────

/// 這個桌面工作階段能不能用 X11 的截圖與合成輸入。Wayland 刻意不允許一般程式擷取別人的畫面或
/// 注入輸入，所以明確判定為不支援（即使有 XWayland 的 DISPLAY，也只看得到 X 視窗）。
pub fn session_support(session_type: Option<&str>, display: Option<&str>, wayland_display: Option<&str>) -> Result<(), String> {
    let wayland = session_type.map(|s| s.eq_ignore_ascii_case("wayland")).unwrap_or(false)
        || (wayland_display.is_some() && session_type.map_or(true, |s| s.is_empty()));
    if wayland {
        return Err(
            "目前只支援 X11 桌面。Wayland 不允許應用程式擷取其他視窗的畫面或模擬輸入；請改用「X11」工作階段登入（登入畫面的齒輪選單）。"
                .to_string(),
        );
    }
    if display.map(|d| d.trim().is_empty()).unwrap_or(true) {
        return Err("找不到 X11 顯示（沒有 DISPLAY）。電腦操作需要在圖形桌面的 X11 工作階段裡執行。".to_string());
    }
    Ok(())
}

// ── OCR：解析 tesseract 的 TSV 輸出 ─────────────────────────────────────────────────

/// `tesseract <圖> stdout tsv` 的輸出 → 每一行文字與外框（圖片像素座標）。
/// 欄位：level page_num block_num par_num line_num word_num left top width height conf text。
/// 只收 level 5（單字）的資料，依 (block, par, line) 組成行；信心為負或文字為空的略過。
pub fn parse_tesseract_tsv(tsv: &str) -> Vec<ScreenText> {
    use std::collections::BTreeMap;
    type Key = (u32, u32, u32, u32);
    let mut lines: BTreeMap<Key, (Vec<String>, f64, f64, f64, f64)> = BTreeMap::new();
    for row in tsv.lines().skip(1) {
        let f: Vec<&str> = row.split('\t').collect();
        if f.len() < 12 || f[0] != "5" {
            continue;
        }
        let text = f[11].trim();
        let conf: f64 = f[10].parse().unwrap_or(-1.0);
        if text.is_empty() || conf < 0.0 {
            continue;
        }
        let nums: Vec<Option<f64>> = f[6..10].iter().map(|v| v.parse::<f64>().ok()).collect();
        let (Some(left), Some(top), Some(w), Some(h)) = (nums[0], nums[1], nums[2], nums[3]) else { continue };
        let key = (
            f[1].parse().unwrap_or(0),
            f[2].parse().unwrap_or(0),
            f[3].parse().unwrap_or(0),
            f[4].parse().unwrap_or(0),
        );
        let e = lines.entry(key).or_insert((Vec::new(), f64::MAX, f64::MAX, f64::MIN, f64::MIN));
        e.0.push(text.to_string());
        e.1 = e.1.min(left);
        e.2 = e.2.min(top);
        e.3 = e.3.max(left + w);
        e.4 = e.4.max(top + h);
    }
    lines
        .into_values()
        .filter(|(words, x0, y0, x1, y1)| !words.is_empty() && x1 > x0 && y1 > y0)
        .map(|(words, x0, y0, x1, y1)| ScreenText { text: words.join(" "), x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
        .collect()
}

/// `/proc/<pid>/comm` 的內容 → App 名稱（去掉結尾換行）。
pub fn app_name_from_comm(comm: &str) -> Option<String> {
    let n = comm.trim();
    (!n.is_empty()).then(|| n.to_string())
}

/// WM_CLASS 的「class」部分（第二個，由 NUL 分隔）當作 App 名稱的後備，例如 `Navigator\0firefox` → `firefox`。
pub fn app_name_from_wm_class(raw: &[u8]) -> Option<String> {
    let parts: Vec<&str> = raw.split(|b| *b == 0).filter_map(|p| std::str::from_utf8(p).ok()).filter(|p| !p.is_empty()).collect();
    parts.get(1).or_else(|| parts.first()).map(|s| s.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::policy::parse_key;

    #[test]
    fn named_letter_digit_function_and_punctuation_keys_have_keysyms() {
        assert_eq!(keysym_for_key("return"), Some(0xff0d));
        assert_eq!(keysym_for_key("delete"), Some(0xff08)); // 往回刪 = BackSpace
        assert_eq!(keysym_for_key("forwarddelete"), Some(0xffff));
        assert_eq!(keysym_for_key("a"), Some(0x61));
        assert_eq!(keysym_for_key("0"), Some(0x30));
        assert_eq!(keysym_for_key("f1"), Some(0xffbe));
        assert_eq!(keysym_for_key("f12"), Some(0xffc9));
        assert_eq!(keysym_for_key("`"), Some(0x60));
        assert_eq!(keysym_for_key("f13"), None);
        assert_eq!(keysym_for_key("nope"), None);
        assert_eq!(keysym_for_key(""), None);
    }

    #[test]
    fn every_key_the_parser_accepts_has_a_keysym_on_linux() {
        for k in ["a", "z", "0", "9", "return", "tab", "space", "escape", "delete", "forwarddelete", "left", "right", "up",
                  "down", "home", "end", "pageup", "pagedown", "f1", "f12", "-", "=", "[", "]", "\\", ";", "'", ",", ".", "/", "`"] {
            assert!(parse_key(k).is_ok(), "{k}");
            assert!(keysym_for_key(k).is_some(), "{k}");
        }
    }

    #[test]
    fn cmd_maps_to_ctrl_and_is_not_pressed_twice() {
        let m = |s: &str| modifier_keysyms(&parse_key(s).unwrap());
        assert_eq!(m("cmd+s"), vec![XK_CONTROL_L]);
        assert_eq!(m("cmd+ctrl+s"), vec![XK_CONTROL_L]);
        assert_eq!(m("ctrl+alt+delete"), vec![XK_CONTROL_L, XK_ALT_L]);
        assert_eq!(m("cmd+shift+left"), vec![XK_CONTROL_L, XK_SHIFT_L]);
        assert!(m("return").is_empty());
    }

    #[test]
    fn characters_map_to_latin1_or_unicode_keysyms() {
        assert_eq!(char_keysym('a'), 0x61);
        assert_eq!(char_keysym(' '), 0x20);
        assert_eq!(char_keysym('é'), 0xe9);
        assert_eq!(char_keysym('你'), 0x0100_4f60);
        assert_eq!(char_keysym('😀'), 0x0101_f600);
        assert_eq!(char_keysym('\n'), 0x0100_000a); // 控制字元走 Unicode keysym（呼叫端會改送 Return）
    }

    #[test]
    fn scroll_uses_buttons_4_to_7_with_up_and_right_positive() {
        assert_eq!(scroll_buttons(0, 3), vec![(4, 3)]);
        assert_eq!(scroll_buttons(0, -2), vec![(5, 2)]);
        assert_eq!(scroll_buttons(2, 0), vec![(7, 2)]);
        assert_eq!(scroll_buttons(-1, 1), vec![(4, 1), (6, 1)]);
        assert!(scroll_buttons(0, 0).is_empty());
        assert_eq!(scroll_buttons(0, i32::MAX), vec![(4, 200)]); // 上限
        assert_eq!(scroll_buttons(0, i32::MIN), vec![(5, 200)]); // 取絕對值不溢位
    }

    #[test]
    fn wayland_sessions_are_refused_with_a_clear_instruction() {
        let e = session_support(Some("wayland"), Some(":0"), Some("wayland-0")).unwrap_err();
        assert!(e.contains("Wayland") && e.contains("X11"));
        // 沒有 XDG_SESSION_TYPE 但有 WAYLAND_DISPLAY：也算 Wayland
        assert!(session_support(None, Some(":0"), Some("wayland-0")).is_err());
        assert!(session_support(Some(""), Some(":0"), Some("wayland-0")).is_err());
        // X11 工作階段
        assert!(session_support(Some("x11"), Some(":0"), None).is_ok());
        assert!(session_support(None, Some(":1"), None).is_ok());
        // 沒有 DISPLAY
        assert!(session_support(Some("x11"), None, None).unwrap_err().contains("DISPLAY"));
        assert!(session_support(Some("x11"), Some("  "), None).is_err());
    }

    #[test]
    fn tesseract_tsv_words_are_grouped_into_lines_with_union_boxes() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t800\t600\t-1\t\n\
4\t1\t1\t1\t1\t0\t10\t20\t200\t18\t-1\t\n\
5\t1\t1\t1\t1\t1\t10\t20\t60\t18\t96.5\tHello\n\
5\t1\t1\t1\t1\t2\t80\t22\t100\t16\t91\tworld\n\
5\t1\t1\t1\t2\t1\t10\t60\t90\t18\t88\t搜尋\n\
5\t1\t1\t1\t2\t2\t110\t60\t40\t18\t-1\tgarbage\n\
5\t1\t1\t1\t3\t1\t10\t100\t50\t18\t90\t   \n";
        let lines = parse_tesseract_tsv(tsv);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!((lines[0].x, lines[0].y, lines[0].w, lines[0].h), (10.0, 20.0, 170.0, 18.0));
        assert_eq!(lines[1].text, "搜尋"); // 信心為負的字與空白字被略過
        assert!(parse_tesseract_tsv("").is_empty());
        assert!(parse_tesseract_tsv("header only\n").is_empty());
        assert!(parse_tesseract_tsv("a\nb\tc\n5\tnot\tnumbers\n").is_empty()); // 壞資料不 panic
    }

    #[test]
    fn app_names_come_from_proc_comm_or_wm_class() {
        assert_eq!(app_name_from_comm("firefox\n").as_deref(), Some("firefox"));
        assert_eq!(app_name_from_comm("  \n"), None);
        assert_eq!(app_name_from_wm_class(b"Navigator\0firefox\0").as_deref(), Some("firefox"));
        assert_eq!(app_name_from_wm_class(b"gnome-terminal\0").as_deref(), Some("gnome-terminal"));
        assert_eq!(app_name_from_wm_class(b"").as_deref(), None);
    }
}
