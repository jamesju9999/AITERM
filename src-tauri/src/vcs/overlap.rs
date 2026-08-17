//! 純函式：比對「即將要動的檔案」跟「其他人進行中功能已經動過的檔案」有沒有重疊。
//! 不碰 HTTP、不碰 git，方便完整單元測試覆蓋。

use super::types::ActiveFeature;

/// 正規化一個 repo 相對路徑，讓 Windows 產生的 `\` 路徑跟 Unix 的 `/` 路徑能對得起來。
///
/// 注意：比對時**區分大小寫**，符合 git 與 GitHub API 的語意。
/// 這是刻意的設計而非疏漏——git 內部把路徑作為大小寫敏感的位元組處理，
/// GitHub API 也完全按提交時的大小寫回傳。如果使用者輸入的路徑大小寫跟實際檔案不同，
/// 不應該錯誤地標記為重疊（在大小寫敏感的檔案系統上，兩者可能是不同檔案）。
fn normalize(path: &str) -> String {
    path.trim().replace('\\', "/")
}

/// 回傳 `features` 裡跟 `candidate_files` 有任何檔案重疊的項目。
/// `candidate_files` 是空的（使用者沒填「預計檔案」）就直接回傳空清單——
/// 沒有東西可比對，不該假裝比對出了「沒有衝突」。
///
/// 檔案路徑比對是**區分大小寫**的，以符合 git 和 GitHub API 的語意。
pub fn find_overlaps(candidate_files: &[String], features: &[ActiveFeature]) -> Vec<ActiveFeature> {
    if candidate_files.is_empty() {
        return Vec::new();
    }
    let candidates: std::collections::HashSet<String> =
        candidate_files.iter().map(|f| normalize(f)).collect();

    features
        .iter()
        .filter(|f| f.files.iter().any(|file| candidates.contains(&normalize(file))))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(author: &str, files: &[&str]) -> ActiveFeature {
        ActiveFeature {
            number: 1,
            title: format!("{author} 的功能"),
            author: author.to_string(),
            draft: true,
            url: "https://github.com/x/y/pull/1".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            head_ref: format!("feature/{author}"),
            files: files.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn detects_exact_file_overlap() {
        let features = vec![feature("bob", &["src/App.tsx", "src/index.ts"])];
        let hits = find_overlaps(&["src/App.tsx".to_string()], &features);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].author, "bob");
    }

    #[test]
    fn no_overlap_returns_empty() {
        let features = vec![feature("bob", &["src/App.tsx"])];
        let hits = find_overlaps(&["src/Other.tsx".to_string()], &features);
        assert!(hits.is_empty());
    }

    #[test]
    fn empty_candidate_list_returns_empty_without_false_positives() {
        let features = vec![feature("bob", &["src/App.tsx"])];
        let hits = find_overlaps(&[], &features);
        assert!(hits.is_empty());
    }

    #[test]
    fn normalizes_windows_backslashes_before_comparing() {
        let features = vec![feature("bob", &["src\\App.tsx"])];
        let hits = find_overlaps(&["src/App.tsx".to_string()], &features);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn does_not_match_different_files_in_same_directory() {
        let features = vec![feature("bob", &["src/App.tsx"])];
        let hits = find_overlaps(&["src/AppTest.tsx".to_string()], &features);
        assert!(hits.is_empty());
    }

    #[test]
    fn matches_against_multiple_features_independently() {
        let features = vec![
            feature("bob", &["src/App.tsx"]),
            feature("carol", &["src/Other.tsx"]),
        ];
        let hits = find_overlaps(&["src/Other.tsx".to_string()], &features);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].author, "carol");
    }

    #[test]
    fn case_differences_are_treated_as_different_files() {
        // Feature has "src/App.tsx" but candidate is "src/app.tsx" — should NOT match
        // This is intentional: git treats paths as case-sensitive, and we follow that semantics
        let features = vec![feature("bob", &["src/App.tsx"])];
        let hits = find_overlaps(&["src/app.tsx".to_string()], &features);
        assert!(hits.is_empty(), "Case-different paths should not match");
    }
}
