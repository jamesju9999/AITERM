use aiterm_lib::commands::exec::run_command;

#[tokio::test]
async fn runs_simple_command_and_captures_stdout() {
    let r = run_command("echo hello_exec", None, 10_000).await.unwrap();
    assert!(r.stdout.contains("hello_exec"));
    assert_eq!(r.exit_code, Some(0));
    assert!(!r.timed_out);
}

#[tokio::test]
async fn reports_nonzero_exit_code() {
    let r = run_command("exit 3", None, 10_000).await.unwrap();
    assert_eq!(r.exit_code, Some(3));
    assert!(!r.timed_out);
}

#[tokio::test]
async fn captures_stderr() {
    #[cfg(windows)]
    let cmd = "echo oops 1>&2";
    #[cfg(not(windows))]
    let cmd = "echo oops >&2";
    let r = run_command(cmd, None, 10_000).await.unwrap();
    assert!(r.stderr.contains("oops"));
}

#[tokio::test]
async fn kills_on_timeout() {
    #[cfg(windows)]
    let cmd = "ping -n 30 127.0.0.1 >NUL";
    #[cfg(not(windows))]
    let cmd = "sleep 30";
    let start = std::time::Instant::now();
    let r = run_command(cmd, None, 500).await.unwrap();
    assert!(r.timed_out);
    assert!(start.elapsed().as_secs() < 5, "kill must not wait for the child");
}

#[cfg(not(windows))]
#[tokio::test]
async fn timeout_returns_promptly_despite_orphaned_pipe_holder() {
    let start = std::time::Instant::now();
    let r = run_command("( sleep 5 & ) ; sleep 6", None, 500).await.unwrap();
    assert!(r.timed_out);
    assert!(start.elapsed().as_secs() < 4, "must not wait for orphaned descendant");
}

#[tokio::test]
async fn respects_cwd() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let cmd = "cd";
    #[cfg(not(windows))]
    let cmd = "pwd";
    let r = run_command(cmd, Some(dir.path().to_str().unwrap()), 10_000).await.unwrap();
    // canonical 路徑在 macOS 可能有 /private 前綴，用資料夾名比對
    let dir_name = dir.path().file_name().unwrap().to_str().unwrap();
    assert!(r.stdout.contains(dir_name));
}
