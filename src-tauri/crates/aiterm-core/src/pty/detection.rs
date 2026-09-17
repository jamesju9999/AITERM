use super::cd_parser::ShellVariant;

/// 掃 `recent_output`（已 ANSI-stripped，來自 `PtySession::get_recent_output`）
/// 尾端的一小段，判斷是否像是「剛執行完的指令因權限不足失敗」。
///
/// 只看**最後一段非空白內容**，不是整個 scrollback——避免使用者自己
/// `echo "Access is denied."` 或訊息出現在 scrollback 中段時誤判。
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
}
