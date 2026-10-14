use crate::ai::{Locale, RiskLevel};

pub mod rules;

pub struct CommandGuard;

/// 指令被判為有風險的原因。文字依介面語系產生（授權框、指令預覽的說明都會顯示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardReason {
    DeleteRoot,
    RecursiveForceDelete,
    FileDelete,
    AdminPrivilege,
    DiskFormat,
    Power,
    ForceKill,
    Kill,
    DangerousChmod,
    Chmod,
    Chown,
    PackageChange,
    GitChange,
    RemoteScript,
    Download,
    RecursiveRemoveItem,
    RemoveItem,
    CompoundSudo,
    CompoundSu,
    CompoundRecursiveDelete,
    Compound,
    UnparsableQuotes,
    Substitution,
    Unrecognized,
}

impl GuardReason {
    pub fn text(self, locale: Locale) -> &'static str {
        use GuardReason::*;
        let (zh, en) = match self {
            DeleteRoot => ("刪除系統根目錄", "deletes the system root directory"),
            RecursiveForceDelete => ("遞迴強制刪除，無法復原", "recursive forced delete, cannot be undone"),
            FileDelete => ("檔案刪除操作", "deletes files"),
            AdminPrivilege => ("包含管理員系統權限操作", "runs with administrator privileges"),
            DiskFormat => ("磁碟格式化操作", "formats a disk"),
            Power => ("系統電源操作", "shuts down or restarts the system"),
            ForceKill => ("強制結束處理程序", "force-kills processes"),
            Kill => ("結束處理程序", "ends processes"),
            DangerousChmod => ("危險的權限變更", "dangerous permission change"),
            Chmod => ("變更檔案權限", "changes file permissions"),
            Chown => ("變更檔案擁有者", "changes file ownership"),
            PackageChange => ("套件管理系統修改操作", "installs, removes or updates packages"),
            GitChange => ("對版本庫進行修改操作", "modifies the git repository"),
            RemoteScript => ("載入並可能執行外部網際網路腳本", "downloads and may run a script from the internet"),
            Download => ("網路下載操作", "downloads from the network"),
            RecursiveRemoveItem => ("遞迴刪除項目", "recursively deletes items"),
            RemoveItem => ("刪除項目", "deletes items"),
            CompoundSudo => ("複合指令中包含管理員權限操作 (sudo)", "compound command includes an administrator operation (sudo)"),
            CompoundSu => ("複合指令中包含用戶切換操作 (su)", "compound command switches user (su)"),
            CompoundRecursiveDelete => ("複合指令中包含遞迴強制刪除操作", "compound command includes a recursive forced delete"),
            Compound => ("指令包含管線或多重執行邏輯", "contains pipes or multiple commands"),
            UnparsableQuotes => ("指令包含無法安全解析的引號", "contains quotes that cannot be parsed safely"),
            Substitution => ("包含變數或子殼層替換", "contains variable or subshell substitution"),
            Unrecognized => ("無法辨識該指令是否為安全查詢", "not recognized as a read-only command"),
        };
        match locale {
            Locale::ZhTw => zh,
            Locale::En => en,
        }
    }
}

impl CommandGuard {
    /// Evaluates a user command and returns the classification.
    /// Returns a tuple of (RiskLevel, Option<reason>).
    pub fn classify(command: &str) -> (RiskLevel, Option<GuardReason>) {
        let cmd_trim = command.trim();
        if cmd_trim.is_empty() {
            return (RiskLevel::Safe, None);
        }

        // 1. Heuristic for multi-commands and pipes
        // Before returning NeedsConfirm, scan for dangerous patterns inside compound commands
        if cmd_trim.contains('|')
            || cmd_trim.contains("&&")
            || cmd_trim.contains(';')
            || cmd_trim.contains("||")
            || cmd_trim.contains('>')
            || cmd_trim.contains('<')
        {
            // Scan for dangerous keywords inside compound commands
            let lower = cmd_trim.to_lowercase();
            if lower.contains("sudo ") || lower.contains("sudo\t") || lower.starts_with("sudo") {
                return (
                    RiskLevel::Dangerous,
                    Some(GuardReason::CompoundSudo),
                );
            }
            if lower.contains(" su ") || lower.starts_with("su ") {
                return (
                    RiskLevel::Dangerous,
                    Some(GuardReason::CompoundSu),
                );
            }
            if lower.contains("rm -rf") || lower.contains("rm -fr") {
                return (
                    RiskLevel::Dangerous,
                    Some(GuardReason::CompoundRecursiveDelete),
                );
            }
            return (
                RiskLevel::NeedsConfirm,
                Some(GuardReason::Compound),
            );
        }

        // 2. Tokenize the command using shell-words
        let words = match shell_words::split(cmd_trim) {
            Ok(w) => w,
            Err(_) => {
                return (
                    RiskLevel::NeedsConfirm,
                    Some(GuardReason::UnparsableQuotes),
                );
            }
        };

        if words.is_empty() {
            return (RiskLevel::Safe, None);
        }

        let program = &words[0];
        let args = &words[1..];

        // 3. Evaluate specific blacklist/graylist rules
        if let Some((level, reason)) = rules::evaluate(program, args) {
            return (level, Some(reason));
        }

        // 4. Fallback safe whitelist
        if rules::is_safe_program(program) {
            // Even if the program is safe, if it contains variable substitutions
            // or subshells, we should ask for confirmation.
            if args.iter().any(|a| a.starts_with('$') || a.starts_with('`')) {
                return (
                    RiskLevel::NeedsConfirm,
                    Some(GuardReason::Substitution),
                );
            }
            return (RiskLevel::Safe, None);
        }

        // 5. Default
        (
            RiskLevel::NeedsConfirm,
            Some(GuardReason::Unrecognized),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipes() {
        let (level, _) = CommandGuard::classify("ls | grep foo");
        assert_eq!(level, RiskLevel::NeedsConfirm);
    }

    #[test]
    fn test_whitelist() {
        let (level, _) = CommandGuard::classify("ls -la");
        assert_eq!(level, RiskLevel::Safe);
    }

    #[test]
    fn test_unknown() {
        let (level, _) = CommandGuard::classify("some_weird_tool");
        assert_eq!(level, RiskLevel::NeedsConfirm);
    }
}
