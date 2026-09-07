//! `tasks_read_transcript` 的來源優先序：有完整的 session 記錄就用它，
//! 沒有才退回終端機擷取。
//!
//! 見 docs/superpowers/specs/2026-09-07-full-task-transcript-design.md。

use aiterm_lib::commands::tasks::resolve_transcript;

fn write(dir: &std::path::Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.to_string_lossy().into_owned()
}

const ONE_TURN: &str =
    r#"{"type":"user","message":{"role":"user","content":"做這件事"}}"#;

#[test]
fn prefers_the_rendered_session_log() {
    let d = tempfile::tempdir().unwrap();
    let session = write(d.path(), "session.jsonl", ONE_TURN);
    let transcript = write(d.path(), "transcript.txt", "只有最後一屏");

    let out = resolve_transcript(Some(&session), Some(&transcript));
    assert!(out.contains("使用者：做這件事"), "沒有用 session 記錄：{out}");
    assert!(!out.contains("只有最後一屏"), "不該同時吐出兩份：{out}");
}

#[test]
fn falls_back_to_the_terminal_capture_when_there_is_no_session_log() {
    let d = tempfile::tempdir().unwrap();
    let transcript = write(d.path(), "transcript.txt", "只有最後一屏");
    assert_eq!(resolve_transcript(None, Some(&transcript)), "只有最後一屏");
}

/// session_path 有值但檔案被刪掉／讀不到——必須退回，不能回空字串。
#[test]
fn falls_back_when_the_session_file_is_gone() {
    let d = tempfile::tempdir().unwrap();
    let transcript = write(d.path(), "transcript.txt", "只有最後一屏");
    let missing = d.path().join("nope.jsonl").to_string_lossy().into_owned();
    assert_eq!(resolve_transcript(Some(&missing), Some(&transcript)), "只有最後一屏");
}

/// 檔案在、但裡面一句對話都渲染不出來（整份都是 mode / attachment 之類的
/// 雜訊記錄）也要退回。「讀得到檔案」不等於「有內容」。
#[test]
fn falls_back_when_the_session_log_renders_to_nothing() {
    let d = tempfile::tempdir().unwrap();
    let session = write(d.path(), "session.jsonl", "{\"type\":\"mode\"}\n");
    let transcript = write(d.path(), "transcript.txt", "只有最後一屏");
    assert_eq!(resolve_transcript(Some(&session), Some(&transcript)), "只有最後一屏");
}

#[test]
fn returns_empty_when_there_is_nothing_at_all() {
    assert_eq!(resolve_transcript(None, None), "");
}
