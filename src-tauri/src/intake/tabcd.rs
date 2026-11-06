//! 替終端機分頁切換到專案目錄。
//!
//! 只在分頁**閒置在 shell 提示字元**時才送：分頁若正在跑 Claude Code、vim 或長指令，
//! 打進去的 `cd` 會變成那個程式的輸入。判斷靠 shell 整合送出的 OSC 133 標記
//! （C＝指令開始、D＝指令結束、A／B＝提示字元），沒有標記就當作無法判斷、不送。
use aiterm_core::pty::cd_parser::ShellVariant;

/// 對真的分頁做整件事：確認閒置 → 清行並送出 `cd` → 等分頁目錄真的變過去。
pub async fn cd_tab(pty: &crate::pty::PtyManager, tab_id: &str, dir: &str) -> Result<(), String> {
    let shell = pty.get_shell_variant(tab_id).ok_or("找不到這個分頁")?;
    let raw = pty.get_recent_raw(tab_id, 256 * 1024).unwrap_or_default();
    match shell_idle(&raw) {
        Some(true) => {}
        Some(false) => return Err("這個分頁正在執行程式，請先結束它再切換".into()),
        None => return Err("無法確認這個分頁是否閒置（沒有 shell 整合），請手動切換".into()),
    }
    let (clear, line) = cd_command(shell, dir)?;
    let before = exit_codes(&raw).len();
    pty.write(tab_id, format!("{clear}{line}\r").as_bytes()).map_err(|e| e.to_string())?;
    // 以 shell 自己回報的結果為準：指令結束時 shell 整合會送 `D;<結束碼>`。
    // 不靠分頁目錄的追蹤——它只是解析「打過的 cd 指令」，路徑含單引號之類就解析不了。
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let now = exit_codes(&pty.get_recent_raw(tab_id, 256 * 1024).unwrap_or_default());
        match now.get(before..).and_then(|new| new.last()) {
            Some(0) => return Ok(()),
            Some(_) => return Err(format!("切換失敗：{dir}（資料夾不存在或沒有權限？）")),
            None => {}
        }
        // 沒有結束碼的 shell（cmd）：退而看分頁目錄。
        if matches!(shell, ShellVariant::Cmd) && pty.get_cwd(tab_id).is_some_and(|c| is_under(&c.to_string_lossy(), dir)) {
            return Ok(());
        }
    }
    Err(format!("已送出切換指令，但沒有收到完成的回報：{dir}"))
}

fn exit_codes(raw: &[u8]) -> Vec<i32> {
    aiterm_core::pty::cd_parser::find_exit_codes(raw)
}

/// 看最近輸出裡**最後一個** OSC 133 標記：A、B、D 代表停在提示字元（閒置），C 代表有指令在跑。
/// 一個標記都沒有（沒有 shell 整合）回 `None`。
pub fn shell_idle(raw: &[u8]) -> Option<bool> {
    const PREFIX: &[u8] = b"\x1b]133;";
    let mut last = None;
    let mut i = 0;
    while i + PREFIX.len() < raw.len() {
        if raw[i..].starts_with(PREFIX) {
            last = Some(raw[i + PREFIX.len()]);
            i += PREFIX.len();
        } else {
            i += 1;
        }
    }
    match last? {
        b'A' | b'B' | b'D' => Some(true),
        b'C' => Some(false),
        _ => None,
    }
}

/// 要先清掉提示字元上使用者已經打了一半的字，否則 `cd` 會接在後面變成別的指令。
/// `(清行按鍵, 指令)`；路徑含控制字元、或在該 shell 下無法安全引用就回錯誤。
pub fn cd_command(shell: ShellVariant, dir: &str) -> Result<(&'static str, String), String> {
    if dir.is_empty() || dir.chars().any(|c| c.is_control()) {
        return Err("專案目錄的路徑含有無法安全送進終端機的字元".into());
    }
    match shell {
        // Ctrl-E 到行尾、Ctrl-U 清掉整行（zsh、bash、fish 都適用）。
        ShellVariant::Bash => Ok(("\x05\x15", format!("cd '{}'", dir.replace('\'', "'\\''")))),
        // PSReadLine 的 Esc 會還原整行。
        ShellVariant::Pwsh => Ok(("\x1b", format!("Set-Location -LiteralPath '{}'", dir.replace('\'', "''")))),
        ShellVariant::Cmd => {
            // cmd 會在引號內展開 %VAR%，也沒有可靠的跳脫方式：直接拒絕。
            if dir.contains('%') || dir.contains('"') {
                return Err("專案目錄的路徑含有 % 或雙引號，無法在 cmd 安全切換，請手動切換".into());
            }
            Ok(("\x1b", format!("cd /d \"{dir}\"")))
        }
        ShellVariant::Unknown => Err("不支援這個 shell，請手動切換".into()),
    }
}

/// `dir` 是 `root` 本身或在它底下（以資料夾邊界判斷，不分大小寫與斜線方向）。
pub fn is_under(dir: &str, root: &str) -> bool {
    let n = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let (d, r) = (n(dir), n(root));
    !r.is_empty() && (d == r || d.starts_with(&format!("{r}/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_follows_the_last_osc133_marker() {
        assert_eq!(shell_idle(b""), None);
        assert_eq!(shell_idle(b"plain output without any shell integration"), None);
        assert_eq!(shell_idle(b"\x1b]133;A\x07$ "), Some(true), "停在提示字元");
        assert_eq!(shell_idle(b"\x1b]133;A\x07$ claude\x1b]133;C\x07"), Some(false), "指令還在跑");
        assert_eq!(shell_idle(b"\x1b]133;C\x07out\x1b]133;D;0\x07\x1b]133;A\x07$ "), Some(true), "跑完又回到提示字元");
        assert_eq!(shell_idle(b"\x1b]133;D;1\x07"), Some(true));
        // 輸出很多時最後一個標記才算，不是第一個。
        let mut long = b"\x1b]133;A\x07".to_vec();
        long.extend(std::iter::repeat(b'x').take(100_000));
        long.extend(b"\x1b]133;C\x07vim");
        assert_eq!(shell_idle(&long), Some(false));
    }

    #[test]
    fn quoting_is_right_for_each_shell_including_spaces_quotes_and_unicode() {
        assert_eq!(cd_command(ShellVariant::Bash, "/Users/a b/記帳本").unwrap().1, "cd '/Users/a b/記帳本'");
        assert_eq!(cd_command(ShellVariant::Bash, "/x/it's").unwrap().1, "cd '/x/it'\\''s'");
        assert_eq!(cd_command(ShellVariant::Pwsh, "C:\\Work\\it's").unwrap().1, "Set-Location -LiteralPath 'C:\\Work\\it''s'");
        assert_eq!(cd_command(ShellVariant::Cmd, "C:\\Work\\App Server").unwrap().1, "cd /d \"C:\\Work\\App Server\"");
        // 每個 shell 都有清行用的前置按鍵。
        for s in [ShellVariant::Bash, ShellVariant::Pwsh, ShellVariant::Cmd] {
            assert!(!cd_command(s, "/x").unwrap().0.is_empty());
        }
    }

    #[test]
    fn dangerous_or_unsupported_input_is_refused() {
        assert!(cd_command(ShellVariant::Bash, "/x\nrm -rf ~").is_err(), "換行不能夾帶第二個指令");
        assert!(cd_command(ShellVariant::Bash, "").is_err());
        assert!(cd_command(ShellVariant::Cmd, "C:\\%USERPROFILE%").is_err());
        assert!(cd_command(ShellVariant::Cmd, "C:\\a\"b").is_err());
        assert!(cd_command(ShellVariant::Unknown, "/x").is_err());
        // 單引號包起來的路徑裡，$、反引號、分號都只是普通字元。
        assert_eq!(cd_command(ShellVariant::Bash, "/x/$(id);`id`").unwrap().1, "cd '/x/$(id);`id`'");
    }

    #[test]
    fn under_respects_boundaries_case_and_slashes() {
        assert!(is_under("/work/app/src", "/work/app"));
        assert!(is_under("/work/app", "/work/app/"));
        assert!(!is_under("/work/application", "/work/app"));
        assert!(is_under("C:\\Work\\App\\src", "c:/work/app"));
        assert!(!is_under("/x", ""));
    }
}
