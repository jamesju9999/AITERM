use serde::Serialize;
use std::path::Path;

/// 一次「開新分頁」的請求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaunchRequest {
    /// 新分頁的起始目錄。
    pub cwd: Option<String>,
    /// `.command`／`.sh` 檔的路徑。前端要先讓使用者確認才會執行。
    pub script: Option<String>,
    /// `-e` 之後的 argv，shell 就緒後直接送進去。
    pub command: Option<Vec<String>>,
}

use std::path::PathBuf;

const SCRIPT_EXTENSIONS: [&str; 2] = ["command", "sh"];

fn resolve(raw: &str, invoking_cwd: Option<&Path>) -> PathBuf {
    let path = Path::new(raw);
    let joined = match invoking_cwd {
        Some(base) if path.is_relative() => base.join(path),
        _ => path.to_path_buf(),
    };
    // components() 會吃掉中間的 `.`，避免出現 `/x/.`。
    joined.components().collect()
}

fn existing_dir(raw: &str, invoking_cwd: Option<&Path>) -> Option<String> {
    let path = resolve(raw, invoking_cwd);
    path.is_dir().then(|| path.to_string_lossy().into_owned())
}

fn positional(raw: &str, invoking_cwd: Option<&Path>) -> Option<LaunchRequest> {
    let path = resolve(raw, invoking_cwd);
    if path.is_dir() {
        return Some(LaunchRequest { cwd: Some(path.to_string_lossy().into_owned()), script: None, command: None });
    }
    let is_script = path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SCRIPT_EXTENSIONS.iter().any(|s| s.eq_ignore_ascii_case(e)));
    if !is_script {
        return None;
    }
    let parent = path.parent()?.to_string_lossy().into_owned();
    Some(LaunchRequest {
        cwd: Some(parent),
        script: Some(path.to_string_lossy().into_owned()),
        command: None,
    })
}

/// 把命令列參數解析成開分頁請求。`argv[0]` 是程式本身，會被略過。
///
/// - 位置參數：資料夾 → 開在該處；`.command`／`.sh` → 開在其父目錄並記下腳本路徑。
/// - `--working-directory=X`／`--working-directory X`：起始目錄。
/// - `-e`／`--command`／`-x`：其後**所有** argv 都是要跑的指令（x-terminal-emulator 慣例）。
/// - 不存在的路徑、無法辨識的旗標一律略過。
pub fn parse_args(argv: &[String], invoking_cwd: Option<&Path>) -> Vec<LaunchRequest> {
    let mut requests = Vec::new();
    let mut working_dir: Option<String> = None;
    let mut command: Option<Vec<String>> = None;

    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].as_str();
        match arg {
            "-e" | "--command" | "-x" => {
                let rest = &argv[i + 1..];
                if !rest.is_empty() {
                    command = Some(rest.to_vec());
                }
                break;
            }
            "--working-directory" => {
                if let Some(value) = argv.get(i + 1) {
                    working_dir = existing_dir(value, invoking_cwd);
                    i += 1;
                }
            }
            _ if arg.starts_with("--working-directory=") => {
                working_dir = existing_dir(&arg["--working-directory=".len()..], invoking_cwd);
            }
            _ if arg.starts_with('-') => {}
            _ => {
                if let Some(request) = positional(arg, invoking_cwd) {
                    requests.push(request);
                }
            }
        }
        i += 1;
    }

    if command.is_some() || working_dir.is_some() {
        // 只有 `-e` 沒給目錄時，退回呼叫端的 cwd——檔案管理員叫終端機時，
        // 那才是使用者當下所在的位置。
        let cwd = if command.is_some() {
            working_dir.or_else(|| invoking_cwd.map(|p| p.to_string_lossy().into_owned()))
        } else {
            working_dir
        };
        requests.push(LaunchRequest { cwd, script: None, command });
    }
    requests
}

/// macOS `RunEvent::Opened` 給的是 `file://` URL；轉成路徑後餵給 `parse_args`。
pub fn args_from_file_urls(urls: &[url::Url]) -> Vec<String> {
    let mut argv = vec![String::from("aiterm")];
    argv.extend(
        urls.iter()
            .filter_map(|u| u.to_file_path().ok())
            .map(|p| p.to_string_lossy().into_owned()),
    );
    argv
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn argv(parts: &[&str]) -> Vec<String> {
        std::iter::once("aiterm").chain(parts.iter().copied()).map(String::from).collect()
    }

    fn p(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_directory_argument_becomes_a_cwd_request() {
        let dir = tempfile::tempdir().unwrap();
        let got = parse_args(&argv(&[&p(dir.path())]), None);
        assert_eq!(
            got,
            vec![LaunchRequest { cwd: Some(p(dir.path())), script: None, command: None }]
        );
    }

    #[test]
    fn a_command_script_opens_its_parent_dir_and_records_the_script() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("deploy.command");
        fs::write(&script, "#!/bin/sh\n").unwrap();
        let got = parse_args(&argv(&[&p(&script)]), None);
        assert_eq!(
            got,
            vec![LaunchRequest { cwd: Some(p(dir.path())), script: Some(p(&script)), command: None }]
        );
    }

    #[test]
    fn a_sh_script_is_treated_like_a_command_script() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("run.SH"); // 副檔名大小寫不分
        fs::write(&script, "").unwrap();
        let got = parse_args(&argv(&[&p(&script)]), None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].script, Some(p(&script)));
    }

    #[test]
    fn a_plain_file_that_is_not_a_script_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        fs::write(&file, "").unwrap();
        assert!(parse_args(&argv(&[&p(&file)]), None).is_empty());
    }

    #[test]
    fn a_missing_path_is_ignored() {
        assert!(parse_args(&argv(&["/definitely/not/here/aiterm-test"]), None).is_empty());
    }

    #[test]
    fn working_directory_accepts_both_spellings() {
        let dir = tempfile::tempdir().unwrap();
        let d = p(dir.path());
        let eq = parse_args(&argv(&[&format!("--working-directory={d}")]), None);
        let sp = parse_args(&argv(&["--working-directory", &d]), None);
        let want = vec![LaunchRequest { cwd: Some(d.clone()), script: None, command: None }];
        assert_eq!(eq, want);
        assert_eq!(sp, want);
    }

    #[test]
    fn dash_e_swallows_every_following_argument_as_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let d = p(dir.path());
        // `-e` 之後的 `--working-directory` 是指令自己的參數，不是我們的旗標。
        let got = parse_args(&argv(&["-e", "ls", "-la", "--working-directory", &d]), Some(dir.path()));
        assert_eq!(
            got,
            vec![LaunchRequest {
                cwd: Some(d.clone()), // 沒給 --working-directory 時退回呼叫端的 cwd
                script: None,
                command: Some(vec!["ls".into(), "-la".into(), "--working-directory".into(), d]),
            }]
        );
    }

    #[test]
    fn working_directory_and_dash_e_combine_into_one_request() {
        let wd = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let got = parse_args(
            &argv(&["--working-directory", &p(wd.path()), "-e", "htop"]),
            Some(other.path()),
        );
        assert_eq!(
            got,
            vec![LaunchRequest {
                cwd: Some(p(wd.path())),
                script: None,
                command: Some(vec!["htop".into()]),
            }]
        );
    }

    #[test]
    fn a_dash_e_with_nothing_after_it_produces_no_request() {
        assert!(parse_args(&argv(&["-e"]), None).is_empty());
    }

    #[test]
    fn relative_paths_resolve_against_the_invoking_cwd() {
        let base = tempfile::tempdir().unwrap();
        fs::create_dir(base.path().join("proj")).unwrap();
        let got = parse_args(&argv(&["proj"]), Some(base.path()));
        assert_eq!(got[0].cwd, Some(p(&base.path().join("proj"))));
        // "." 要被正規化掉，不能留下 `/x/.`
        let dot = parse_args(&argv(&["."]), Some(base.path()));
        assert_eq!(dot[0].cwd, Some(p(base.path())));
    }

    #[test]
    fn unknown_flags_and_headless_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let got = parse_args(&argv(&["--headless", "-psn_0_12345", &p(dir.path())]), None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cwd, Some(p(dir.path())));
    }

    #[test]
    fn multiple_paths_make_multiple_requests_in_order() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let got = parse_args(&argv(&[&p(a.path()), &p(b.path())]), None);
        assert_eq!(got.iter().map(|r| r.cwd.clone().unwrap()).collect::<Vec<_>>(), vec![p(a.path()), p(b.path())]);
    }

    #[cfg(unix)]
    #[test]
    fn file_urls_become_decoded_path_arguments() {
        let urls = vec![url::Url::parse("file:///tmp/a%20b").unwrap()];
        assert_eq!(args_from_file_urls(&urls), vec!["aiterm".to_string(), "/tmp/a b".to_string()]);
    }

    #[test]
    fn non_file_urls_are_dropped() {
        let urls = vec![url::Url::parse("https://example.com/x").unwrap()];
        assert_eq!(args_from_file_urls(&urls), vec!["aiterm".to_string()]);
    }
}
