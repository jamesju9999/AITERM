use crate::ai::RiskLevel;
use super::GuardReason;

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
pub fn evaluate(program: &str, args: &[String]) -> Option<(RiskLevel, GuardReason)> {
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
            return Some((RiskLevel::Blocked, GuardReason::DeleteRoot));
        } else if has_recursive && has_force {
            return Some((RiskLevel::Dangerous, GuardReason::RecursiveForceDelete));
        } else {
            return Some((RiskLevel::NeedsConfirm, GuardReason::FileDelete));
        }
    }

    // 2. Privilege Escalation (sudo, su)
    if lower_prog == "sudo" || lower_prog == "su" {
        return Some((RiskLevel::Dangerous, GuardReason::AdminPrivilege));
    }

    // 3. Disk formatting (format, mkfs)
    if lower_prog == "format" || lower_prog == "mkfs" {
        return Some((RiskLevel::Blocked, GuardReason::DiskFormat));
    }

    // 4. Power / Process Management
    if lower_prog == "shutdown" || lower_prog == "reboot" {
        return Some((RiskLevel::Dangerous, GuardReason::Power));
    }

    if lower_prog == "kill" || lower_prog == "pkill" || lower_prog == "killall" {
        if args.iter().any(|a| a == "-9" || a == "-sigkill") {
            return Some((RiskLevel::Dangerous, GuardReason::ForceKill));
        }
        return Some((RiskLevel::NeedsConfirm, GuardReason::Kill));
    }

    // 5. System permissions
    if lower_prog == "chmod" {
        if args.iter().any(|a| a == "777" || a == "-R" || a == "--recursive") {
            return Some((RiskLevel::Dangerous, GuardReason::DangerousChmod));
        }
        return Some((RiskLevel::NeedsConfirm, GuardReason::Chmod));
    }

    if lower_prog == "chown" {
        return Some((RiskLevel::Dangerous, GuardReason::Chown));
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
            return Some((RiskLevel::NeedsConfirm, GuardReason::PackageChange));
        }
    }

    // 7. Git (whitelist some, catch others)
    if lower_prog == "git" {
        if args.iter().any(|a| {
            let al = a.to_lowercase();
            al == "commit" || al == "push" || al == "reset" || al == "rebase" || al == "clean"
        }) {
            return Some((RiskLevel::NeedsConfirm, GuardReason::GitChange));
        }
        // Git fetch/pull/diff/log are caught by the whitelist if no matches here,
        // Wait, git is in whitelist. If not matched here, it will be Safe.
    }

    // 8. Remote fetching execution
    if lower_prog == "curl" || lower_prog == "wget" {
        if args.iter().any(|a| a.contains("unix.sh") || a.contains("install.sh")) {
            return Some((RiskLevel::Dangerous, GuardReason::RemoteScript));
        }
        return Some((RiskLevel::NeedsConfirm, GuardReason::Download));
    }

    // 9. PowerShell rules
    if lower_prog == "remove-item" {
        let has_recurse = args.iter().any(|a| a.to_lowercase() == "-recurse");
        if has_recurse {
            return Some((RiskLevel::Dangerous, GuardReason::RecursiveRemoveItem));
        }
        return Some((RiskLevel::NeedsConfirm, GuardReason::RemoveItem));
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
