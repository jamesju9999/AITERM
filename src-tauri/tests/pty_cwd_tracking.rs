//! Integration test for e2e cwd tracking: spawn a real shell, send `cd`
//! commands, verify `PtyManager::get_cwd` matches the shell's actual cwd.
//!
//! This is the "真金火煉" test mentioned in spec §8.2 — unit tests only cover
//! the string parsing; this verifies the write-hook actually fires for a live
//! shell on this OS.

use aiterm_lib::pty::session::PtySession;
use aiterm_lib::pty::shell::default_shell;
use aiterm_lib::pty::PtyManager;
use portable_pty::PtySize;
use std::sync::mpsc;
use std::time::Duration;

fn small_size() -> PtySize {
    PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }
}

/// How long to keep polling before giving up. Generous on purpose: this bounds
/// how slow a machine may be, not how long the operation should take. A run on
/// an idle laptop returns in milliseconds either way.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Poll `f` until it returns true, or the timeout elapses.
///
/// Replaces the fixed sleeps this test used to rely on. Those encoded a guess
/// about how long a shell takes to start, run a command, and print a prompt
/// carrying its OSC 133 exit-code marker — and the guess (100 ms) was wrong on
/// a loaded Windows CI runner, where pwsh alone can take over a second to reach
/// its first prompt. Every run of this test on CI since it was added has failed
/// there for that reason.
///
/// This does not weaken the assertions: the caller still asserts the condition
/// afterwards, so a genuinely broken cwd hook still fails the test — it just
/// takes the full timeout to say so instead of failing instantly on a machine
/// that was merely busy.
fn wait_until(mut f: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + SETTLE_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

/// Upper bound on shell cold start. Much larger than `SETTLE_TIMEOUT` because
/// it bounds how slow the machine may be, not how long anything should take:
/// measured on a busy Windows CI runner (2026-08-11), pwsh produced no output
/// at all for 8.1 s and did not print its first prompt until 11.3 s.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

/// Wait until the shell is *actually* ready — i.e. it has emitted its own
/// OSC 133 A (prompt start) and gone quiet.
///
/// Why "the first chunk of output" is NOT a readiness signal: on Windows the
/// first chunk is ConPTY's own `\e[?9001h\e[?1004h`, which arrives ~15 ms after
/// spawn — seconds before pwsh even starts. Anything written before the shell
/// attaches is dropped and never reaches it.
///
/// That alone would just be flakiness. The reason it went unnoticed is worse:
/// the test still *passed* most of the time, because the shell's own startup
/// prompt emits `133;D;0`, and `confirm_pending_cds_from_output` pops the
/// pending queue on any exit code it sees — so that marker confirmed a `cd`
/// the shell had never run. Whether this test passed came down to whether that
/// free marker landed inside the 10 s window; it never once exercised the
/// write-hook it exists to verify. Measured timeline of a passing run before
/// this fix: write at 15 ms, no echo ever, `133;D;0` at 5872 ms, done at
/// 5883 ms. Failing runs were simply the ones where cold start exceeded 10 s.
///
/// Waiting for readiness also makes the assertions mean something: by the time
/// we stage the `cd`, the startup marker has already been consumed against an
/// empty queue, so the only marker that can confirm it is the one its own
/// execution produces.
///
/// All four integrations emit 133;A (pwsh, cmd's PROMPT, zsh, bash — see
/// `pty::shell`), so this needs no per-platform branch.
///
/// The quiet period matters: 133;A is emitted *before* the prompt text is
/// printed, at which point the shell is not yet reading input. Output going
/// quiet is what says it is waiting for us.
fn wait_for_shell_ready(rx: &mpsc::Receiver<Vec<u8>>) -> bool {
    const PROMPT_START: &[u8] = b"\x1b]133;A";
    let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
    let mut seen: Vec<u8> = Vec::new();
    let mut prompt_started = false;

    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => {
                seen.extend_from_slice(&chunk);
                if !prompt_started && seen.windows(PROMPT_START.len()).any(|w| w == PROMPT_START) {
                    prompt_started = true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) if prompt_started => return true,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
    false
}

#[test]
fn tracks_cd_through_real_shell() {
    let manager = PtyManager::new();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();

    let id = manager
        .create_with_callback(small_size(), move |chunk| {
            let _ = tx.send(chunk);
        })
        .expect("create session");

    // Wait for the shell to actually be ready to read input. Not "some output
    // arrived" — see wait_for_shell_ready for why that is a trap on Windows.
    assert!(
        wait_for_shell_ready(&rx),
        "shell never printed a prompt within {STARTUP_TIMEOUT:?}; anything \
         written before it is ready is dropped and never reaches it",
    );

    let initial = manager.get_cwd(&id).expect("initial cwd");

    // Platform-specific: cmd.exe on Windows, sh on Unix. default_shell() in
    // M0 prefers pwsh → powershell → cmd on Windows. Our parser handles all
    // three, so we just send a parent-dir change that works everywhere.
    #[cfg(windows)]
    manager.write(&id, b"cd ..\r\n").unwrap();
    #[cfg(not(windows))]
    manager.write(&id, b"cd ..\n").unwrap();

    // On Bash/Pwsh the cd is only staged when written; it is committed once the
    // shell's own OSC 133 marker reports the exit code. That round trip is what
    // has to complete here, and its duration is a property of the machine.
    let parent = initial.parent().expect("initial had a parent").to_path_buf();
    wait_until(|| manager.get_cwd(&id).as_deref() == Some(parent.as_path()));

    let after = manager.get_cwd(&id).expect("after cwd");
    assert_ne!(after, initial, "cwd should have changed after `cd ..`");
    assert_eq!(
        after,
        initial.parent().expect("initial had a parent").to_path_buf(),
        "cwd should now be the parent of the initial cwd"
    );

    // Now cd back into a child and verify.
    let child_name = initial
        .file_name()
        .expect("initial had a file name")
        .to_string_lossy()
        .to_string();
    #[cfg(windows)]
    manager
        .write(&id, format!("cd {}\r\n", child_name).as_bytes())
        .unwrap();
    #[cfg(not(windows))]
    manager
        .write(&id, format!("cd {}\n", child_name).as_bytes())
        .unwrap();

    wait_until(|| manager.get_cwd(&id).as_deref() == Some(initial.as_path()));
    assert_eq!(manager.get_cwd(&id).unwrap(), initial);

    manager.close(&id).ok();
}

/// End-to-end regression test for the reported bug: `cd "~"` (quoted) is
/// parsed identically to unquoted `cd ~` by cd_parser (quotes are stripped
/// before matching), but a real shell does NOT tilde-expand a quoted
/// argument, so the command genuinely fails *unless* a directory literally
/// named `~` happens to exist in the cwd (which it must not, for this test
/// to actually exercise the failure path — hence the fresh tempdir with a
/// single, differently-named child directory, rather than relying on
/// whatever happens to be in the crate's own working directory). Before the
/// exit-code confirmation fix, the tracked cwd was optimistically updated to
/// $HOME anyway, permanently desyncing from the real shell and making every
/// subsequent relative cd fail too. Unix-only: this is specifically about
/// bash/zsh tilde-quoting behavior; Windows shells don't have this construct.
#[cfg(not(windows))]
#[test]
fn failed_quoted_tilde_cd_does_not_desync_tracked_cwd() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let initial = tmp.path().canonicalize().expect("canonicalize tempdir");
    assert!(
        !initial.join("~").exists(),
        "test precondition: no literal '~' entry in the fresh tempdir"
    );
    std::fs::create_dir(initial.join("child")).expect("create child dir");

    let shell = default_shell().expect("a default shell is available");
    let (tx, _rx) = mpsc::channel::<Vec<u8>>();
    let session = PtySession::spawn(shell, small_size(), Some(initial.clone()), move |chunk| {
        let _ = tx.send(chunk);
    })
    .expect("spawn pty");

    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(session.get_cwd(), initial, "sanity: session starts in the tempdir");

    // Quoted tilde: real shell fails ("no such file or directory: ~").
    session.write(b"cd \"~\"\n").unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        session.get_cwd(),
        initial,
        "tracked cwd must not change when the real shell reports cd failed"
    );

    // A real subsequent cd must still resolve correctly against the
    // unchanged base — proving the tracker didn't desync.
    session.write(b"cd child\n").unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        session.get_cwd(),
        initial.join("child"),
        "a real cd after a failed one must resolve against the correct (unchanged) base"
    );

    drop(session);
}
