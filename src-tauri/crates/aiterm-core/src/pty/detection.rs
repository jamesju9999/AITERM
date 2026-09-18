use super::cd_parser::ShellVariant;

/// Win32 `ERROR_ELEVATION_REQUIRED`——子行程明確表示「這個動作需要提升權限
/// 才能執行」時的 exit code，跟語言、shell 種類都無關（cmd.exe 的
/// `%ERRORLEVEL%` 與 PowerShell 的 `$LASTEXITCODE` 都是直接反映子行程自己
/// 回報的這個數字）。
///
/// 這個訊號比 [`looks_like_permission_denied`] 的文字比對更可靠：實測中，
/// 非英文（例如繁體中文）Windows 上執行 `DISM /Online /Cleanup-Image
/// /RestoreHealth` 印出「需要提升的權限才能執行 DISM」，完全不含任何我們比
/// 對的英文關鍵字，但 exit code 確實是 740——文字比對在地化語系下全滅，
/// exit code 檢查不受影響。
pub const ERROR_ELEVATION_REQUIRED: i32 = 740;

/// 指令的 exit code 是否直接表明「需要提升權限」。刻意跟 shell variant 無
/// 關——這是子行程自己回報的 Win32 錯誤碼，不是終端機文字，任何 shell 轉發
/// 出來的都一樣。
pub fn exit_code_indicates_permission_denied(exit_code: i32) -> bool {
    exit_code == ERROR_ELEVATION_REQUIRED
}

/// 掃 `recent_output`（已 ANSI-stripped，來自 `PtySession::get_recent_output`）
/// 尾端的一小段，判斷是否像是「剛執行完的指令因權限不足失敗」。
///
/// 只看**最後一段非空白內容**，不是整個 scrollback——避免使用者自己
/// `echo "Access is denied."` 或訊息出現在 scrollback 中段時誤判。
///
/// 只認得固定的英文字串，在地化系統（例如繁體中文 Windows）上常常完全比對
/// 不到——見 [`exit_code_indicates_permission_denied`]，那個訊號才是主要防
/// 線，這個函式是文字層面的輔助訊號，兩者是 OR 的關係，呼叫端應該都檢查。
pub fn looks_like_permission_denied(recent_output: &str, shell: ShellVariant) -> bool {
    let tail = last_nonblank_tail(recent_output);
    match shell {
        ShellVariant::Cmd => {
            tail.contains("Access is denied.") || tail.contains("You do not have sufficient privilege")
        }
        ShellVariant::Pwsh => {
            tail.contains("UnauthorizedAccessException")
                || tail.contains("is denied")
                || tail.contains("requires elevation")
        }
        ShellVariant::Bash | ShellVariant::Unknown => false,
    }
}

/// 取輸出尾端最後 8 個非空白行，串成一段字串給正則掃。8 行足夠涵蓋 PowerShell
/// 例外訊息常見的多行堆疊（訊息本文 + `CategoryInfo` + `FullyQualifiedErrorId`），
/// 又不會大到把使用者自己輸入的內容一起吃進來。
fn last_nonblank_tail(output: &str) -> String {
    output
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_access_is_denied_is_detected() {
        let output = "C:\\Windows\\System32>echo test > C:\\Windows\\System32\\x.txt\nAccess is denied.\n\nC:\\Windows\\System32>";
        assert!(looks_like_permission_denied(output, ShellVariant::Cmd));
    }

    #[test]
    fn cmd_insufficient_privilege_is_detected() {
        let output = "You do not have sufficient privilege to perform this operation.";
        assert!(looks_like_permission_denied(output, ShellVariant::Cmd));
    }

    #[test]
    fn pwsh_unauthorized_access_exception_is_detected() {
        let output = "Set-Content : Access to the path 'C:\\Windows\\System32\\x.txt' is denied.\nUnauthorizedAccessException";
        assert!(looks_like_permission_denied(output, ShellVariant::Pwsh));
    }

    #[test]
    fn cmd_variant_does_not_match_pwsh_only_phrase() {
        // 反例：PowerShell 專屬用語出現在 cmd.exe 輸出裡不該觸發——這種輸出
        // 實務上不會發生，但規則本身必須是 shell-variant-scoped 而非全域字串比對。
        let output = "UnauthorizedAccessException";
        assert!(!looks_like_permission_denied(output, ShellVariant::Cmd));
    }

    #[test]
    fn echoing_the_phrase_yourself_mid_scrollback_does_not_trigger() {
        // 反例：這句話出現在很早之前的輸出裡，後面接了一大段其他輸出——
        // 代表它不是「剛執行完那條指令」的結果，只是被捲到 scrollback 中段。
        let mut output = String::from("Access is denied.\n");
        for i in 0..20 {
            output.push_str(&format!("some later unrelated output line {i}\n"));
        }
        assert!(!looks_like_permission_denied(&output, ShellVariant::Cmd));
    }

    #[test]
    fn bash_never_matches() {
        assert!(!looks_like_permission_denied("Access is denied.", ShellVariant::Bash));
    }

    #[test]
    fn empty_output_does_not_match() {
        assert!(!looks_like_permission_denied("", ShellVariant::Cmd));
    }

    #[test]
    fn exit_code_740_is_elevation_required() {
        assert!(exit_code_indicates_permission_denied(740));
    }

    #[test]
    fn exit_code_zero_is_not_elevation_required() {
        assert!(!exit_code_indicates_permission_denied(0));
    }

    #[test]
    fn exit_code_5_access_denied_is_not_treated_as_elevation_required() {
        // Win32 ERROR_ACCESS_DENIED (5) 太泛用——很多跟提權無關的失敗也會回
        // 這個碼（例如檔案被其他行程鎖住），只挑 740 這個明確表示「需要提升
        // 權限」的碼，避免對一般權限錯誤也跳提權提示。
        assert!(!exit_code_indicates_permission_denied(5));
    }
}
