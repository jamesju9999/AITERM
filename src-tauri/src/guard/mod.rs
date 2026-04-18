use crate::ai::RiskLevel;

pub mod rules;

pub struct CommandGuard;

impl CommandGuard {
    /// Evaluates a user command and returns the classification.
    /// Returns a tuple of (RiskLevel, Option<reason>).
    pub fn classify(command: &str) -> (RiskLevel, Option<String>) {
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
                    Some("複合指令中包含管理員權限操作 (sudo)".to_string()),
                );
            }
            if lower.contains(" su ") || lower.starts_with("su ") {
                return (
                    RiskLevel::Dangerous,
                    Some("複合指令中包含用戶切換操作 (su)".to_string()),
                );
            }
            if lower.contains("rm -rf") || lower.contains("rm -fr") {
                return (
                    RiskLevel::Dangerous,
                    Some("複合指令中包含遞迴強制刪除操作".to_string()),
                );
            }
            return (
                RiskLevel::NeedsConfirm,
                Some("指令包含管線或多重執行邏輯".to_string()),
            );
        }

        // 2. Tokenize the command using shell-words
        let words = match shell_words::split(cmd_trim) {
            Ok(w) => w,
            Err(_) => {
                return (
                    RiskLevel::NeedsConfirm,
                    Some("指令包含無法安全解析的引號".to_string()),
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
            return (level, Some(reason.to_string()));
        }

        // 4. Fallback safe whitelist
        if rules::is_safe_program(program) {
            // Even if the program is safe, if it contains variable substitutions
            // or subshells, we should ask for confirmation.
            if args.iter().any(|a| a.starts_with('$') || a.starts_with('`')) {
                return (
                    RiskLevel::NeedsConfirm,
                    Some("包含變數或子殼層替換".to_string()),
                );
            }
            return (RiskLevel::Safe, None);
        }

        // 5. Default
        (
            RiskLevel::NeedsConfirm,
            Some("無法辨識該指令是否為安全查詢".to_string()),
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
