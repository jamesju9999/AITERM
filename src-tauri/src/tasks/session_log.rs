//! Claude Code 自己寫的 session 記錄：找到它、複製它、渲染它。
//!
//! Claude Code 每一輪 user / assistant / 工具呼叫都會即時寫進
//! `~/.claude/projects/<編碼後的 cwd>/<session-id>.jsonl`。派工時我們用
//! `--session-id` 指定那個 UUID，所以任務結束後不必猜哪個檔案對應哪張卡片。
//!
//! 這個模組除了最外層的 `copy_session_log` 之外全是純函式（吃字串、吐
//! 字串），與檔案系統和 Tauri state 無關。
//!
//! 見 docs/superpowers/specs/2026-09-07-full-task-transcript-design.md。

use std::path::Path;

/// 把絕對路徑編成 Claude Code 用的資料夾名：每一個路徑分隔符換成 `-`。
///
/// 路徑裡既有的 `-` 原樣保留（`/private/tmp/-Users-x` → `-private-tmp--Users-x`），
/// 所以這個轉換不可逆——不需要可逆，我們只需要拼得出同一個資料夾名。
///
/// Windows 的 `\` 與磁碟機代號後面的 `:` 一樣算分隔符，否則 Windows 上
/// 永遠對不到目錄。
pub fn encode_project_dir(dir: &Path) -> String {
    dir.to_string_lossy()
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == ':' { '-' } else { c })
        .collect()
}

#[cfg(test)]
mod encode_tests {
    use super::*;

    #[test]
    fn every_slash_becomes_a_dash() {
        assert_eq!(
            encode_project_dir(Path::new("/Users/jamesju/Documents/GitHub/AITERM")),
            "-Users-jamesju-Documents-GitHub-AITERM"
        );
    }

    /// 路徑本身既有的 `-` 必須原樣保留，不可以被合併或跳脫——這是唯一能
    /// 分辨「換掉斜線」與「換掉斜線後又去正規化連字號」兩種實作的案例。
    #[test]
    fn existing_dashes_in_the_path_are_left_alone() {
        assert_eq!(
            encode_project_dir(Path::new("/private/tmp/-Users-x")),
            "-private-tmp--Users-x"
        );
    }

    /// Windows 路徑也要能編碼——反斜線與磁碟機代號都算路徑分隔，
    /// 否則 Windows 上永遠找不到 session 檔。
    #[test]
    fn windows_separators_and_drive_letters_become_dashes() {
        assert_eq!(encode_project_dir(Path::new(r"C:\Users\j\repo")), "C--Users-j-repo");
    }
}
