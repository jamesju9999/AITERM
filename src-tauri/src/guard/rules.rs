use crate::ai::RiskLevel;

/// Whitelist of safe programs that only read data or have no lasting side effects.
pub fn is_safe_program(program: &str) -> bool {
    let lower = program.to_lowercase();
    matches!(
        lower.as_str(),
        "ls" | "pwd" | "cat" | "git" | "echo" | "which" | "ps" | "df" | "type" | "dir" | "cd" | "clear" | "cls" | "history" | "ping" | "whoami" | "uptime" | "top" | "htop" | "date"
    )
}

/// Evaluates known dangerous or state-mutating programs.
/// Returns Some((RiskLevel, Reason)) if the command matches a heuristic.
pub fn evaluate(program: &str, args: &[String]) -> Option<(RiskLevel, &'static str)> {
    let lower_prog = program.to_lowercase();

    // 1. Deletion (rm, del)
    if lower_prog == "rm" || lower_prog == "del" {
        let has_recursive = args.iter().any(|a| {
            let al = a.to_lowercase();
            al == "-r" || al == "-R" || al == "-rf" || al == "-fr" || al == "/s" || al == "--recursive"
        });
        let has_force = args.iter().any(|a| {
            let al = a.to_lowercase();
            al == "-f" || al == "-rf" || al == "-fr" || al == "/q" || al == "--force"
        });
        let has_root = args.iter().any(|a| {
            a == "/" || a == "/*" || a.to_lowercase() == "c:\\"
        });

        if has_root && has_recursive && has_force {
            return Some((RiskLevel::Blocked, "刪除系統根目錄"));
        } else if has_recursive && has_force {
            return Some((RiskLevel::Dangerous, "遞迴強制刪除，無法復原"));
        } else {
            return Some((RiskLevel::NeedsConfirm, "檔案刪除操作"));
        }
    }

    // 2. Privilege Escalation (sudo, su)
    if lower_prog == "sudo" || lower_prog == "su" {
        return Some((RiskLevel::Dangerous, "包含管理員系統權限操作"));
    }

    // 3. Disk formatting (format, mkfs)
    if lower_prog == "format" || lower_prog == "mkfs" {
        return Some((RiskLevel::Blocked, "磁碟格式化操作"));
    }

    // 4. Power / Process Management
    if lower_prog == "shutdown" || lower_prog == "reboot" {
        return Some((RiskLevel::Dangerous, "系統電源操作"));
    }

    if lower_prog == "kill" || lower_prog == "pkill" || lower_prog == "killall" {
        if args.iter().any(|a| a == "-9" || a == "-sigkill") {
            return Some((RiskLevel::Dangerous, "強制結束處理程序"));
        }
        return Some((RiskLevel::NeedsConfirm, "結束處理程序"));
    }

    // 5. System permissions
    if lower_prog == "chmod" {
        if args.iter().any(|a| a == "777" || a == "-R" || a == "--recursive") {
            return Some((RiskLevel::Dangerous, "危險的權限變更"));
        }
        return Some((RiskLevel::NeedsConfirm, "變更檔案權限"));
    }

    if lower_prog == "chown" {
        return Some((RiskLevel::Dangerous, "變更檔案擁有者"));
    }

    // 6. Package Managers
    if lower_prog == "npm"
        || lower_prog == "yarn"
        || lower_prog == "pnpm"
        || lower_prog == "pip"
        || lower_prog == "cargo"
        || lower_prog == "apt"
        || lower_prog == "brew"
        || lower_prog == "apk"
    {
        if args.iter().any(|a| {
            let al = a.to_lowercase();
            al == "install" || al == "add" || al == "remove" || al == "uninstall" || al == "update" || al == "upgrade" || al == "i"
        }) {
            return Some((RiskLevel::NeedsConfirm, "套件管理系統修改操作"));
        }
    }

    // 7. Git (whitelist some, catch others)
    if lower_prog == "git" {
        if args.iter().any(|a| {
            let al = a.to_lowercase();
            al == "commit" || al == "push" || al == "reset" || al == "rebase" || al == "clean"
        }) {
            return Some((RiskLevel::NeedsConfirm, "對版本庫進行修改操作"));
        }
        // Git fetch/pull/diff/log are caught by the whitelist if no matches here,
        // Wait, git is in whitelist. If not matched here, it will be Safe.
    }

    // 8. Remote fetching execution
    if lower_prog == "curl" || lower_prog == "wget" {
        if args.iter().any(|a| a.contains("unix.sh") || a.contains("install.sh")) {
            return Some((RiskLevel::Dangerous, "載入並可能執行外部網際網路腳本"));
        }
        return Some((RiskLevel::NeedsConfirm, "網路下載操作"));
    }

    // 9. PowerShell rules
    if lower_prog == "remove-item" {
        let has_recurse = args.iter().any(|a| a.to_lowercase() == "-recurse");
        if has_recurse {
            return Some((RiskLevel::Dangerous, "遞迴刪除項目"));
        }
        return Some((RiskLevel::NeedsConfirm, "刪除項目"));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rm_rf() {
        let res = evaluate("rm", &["-rf".into(), "folder".into()]);
        assert_eq!(res.unwrap().0, RiskLevel::Dangerous);
    }

    #[test]
    fn test_rm_rf_root() {
        let res = evaluate("rm", &["-rf".into(), "/".into()]);
        assert_eq!(res.unwrap().0, RiskLevel::Blocked);
    }

    #[test]
    fn test_sudo() {
        let res = evaluate("sudo", &["ls".into()]);
        assert_eq!(res.unwrap().0, RiskLevel::Dangerous);
    }

    #[test]
    fn test_npm_install() {
        let res = evaluate("npm", &["install".into(), "react".into()]);
        assert_eq!(res.unwrap().0, RiskLevel::NeedsConfirm);
    }
}
