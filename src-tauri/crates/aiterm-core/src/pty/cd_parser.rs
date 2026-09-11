//! Pure-function cd-command parser used by `PtySession` to track cwd across
//! user-initiated directory changes. All functions here are synchronous,
//! pure, and have no I/O — they are fully unit-tested without spawning shells.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellVariant {
    Pwsh,     // pwsh.exe, powershell.exe
    Cmd,      // cmd.exe
    Bash,     // bash, sh, zsh, dash, etc.
    Unknown,
}

impl ShellVariant {
    pub fn from_program(program: &str) -> Self {
        let lower = program.to_ascii_lowercase();
        // Split on '/' or '\' manually rather than via std::path::Path: Path's
        // separator is determined by the HOST compile target, not by which
        // shell produced this string. On a Unix-compiled binary, `\` is not
        // recognized as a separator, so a Windows-style program path would
        // never split into a leaf name.
        let leaf = lower.rsplit(['/', '\\']).next().unwrap_or(&lower);
        let stem = leaf.strip_suffix(".exe").unwrap_or(leaf);
        match stem {
            "pwsh" | "powershell" => ShellVariant::Pwsh,
            "cmd" => ShellVariant::Cmd,
            "bash" | "sh" | "zsh" | "dash" | "ash" | "fish" => ShellVariant::Bash,
            _ => ShellVariant::Unknown,
        }
    }
}

/// Outcome of parsing one command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCd {
    /// Not a cd command, or could not be parsed — cwd must stay unchanged.
    NotCd,
    /// A cd with an absolute or already-resolved target.
    ChangeTo(PathBuf),
    /// bash `cd -` — caller should swap current and previous cwd.
    SwapPrevious,
    /// bash `cd` with no args — caller should resolve to home.
    ToHome,
}

/// Parse a single line (one user-entered command) for this shell.
/// `current_cwd` is used to resolve relative paths.
pub fn parse_cd(line: &str, shell: ShellVariant, current_cwd: &Path) -> ParsedCd {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ParsedCd::NotCd;
    }

    // Conservative guard: reject any line that contains shell-control
    // operators or substitutions. These indicate compound commands, subshells,
    // or variable expansion we do not attempt to interpret.
    if contains_uninterpretable(trimmed) {
        return ParsedCd::NotCd;
    }

    let tokens = match tokenize(trimmed) {
        Some(t) => t,
        None => return ParsedCd::NotCd,
    };
    if tokens.is_empty() {
        return ParsedCd::NotCd;
    }

    match shell {
        ShellVariant::Pwsh => parse_pwsh(&tokens, current_cwd),
        ShellVariant::Cmd => parse_cmd(&tokens, current_cwd),
        ShellVariant::Bash => parse_bash(&tokens, current_cwd),
        ShellVariant::Unknown => ParsedCd::NotCd,
    }
}

fn contains_uninterpretable(line: &str) -> bool {
    // Pipes, logical operators, command separators, subshells, substitutions,
    // backticks, redirects, and variable expansion.
    let bad = ['|', '&', ';', '`', '$', '<', '>', '(', ')', '{', '}'];
    line.chars().any(|c| bad.contains(&c))
}

/// Minimal tokenizer: splits on whitespace, respecting single and double quotes.
/// Returns None if quoting is unbalanced.
fn tokenize(line: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_single {
            if c == '\'' { in_single = false; } else { cur.push(c); }
        } else if in_double {
            if c == '"' { in_double = false; } else { cur.push(c); }
        } else if c == '\'' {
            in_single = true;
        } else if c == '"' {
            in_double = true;
        } else if c.is_whitespace() {
            if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
        } else {
            cur.push(c);
        }
    }
    if in_single || in_double { return None; }
    if !cur.is_empty() { out.push(cur); }
    Some(out)
}

fn parse_pwsh(tokens: &[String], cwd: &Path) -> ParsedCd {
    let cmd = tokens[0].to_ascii_lowercase();
    let is_cd_like = matches!(cmd.as_str(),
        "cd" | "cd.." | "set-location" | "sl" | "pushd"
    );
    if !is_cd_like {
        return ParsedCd::NotCd;
    }
    // `cd..` is a pwsh shortcut (yes, really) → parent.
    if cmd == "cd.." {
        return ParsedCd::ChangeTo(parent_or_root(cwd));
    }
    match tokens.get(1) {
        None => ParsedCd::NotCd, // `cd` alone in pwsh is a no-op (prints cwd)
        Some(arg) => resolve_target(arg, cwd, ShellVariant::Pwsh),
    }
}

fn parse_cmd(tokens: &[String], cwd: &Path) -> ParsedCd {
    let cmd = tokens[0].to_ascii_lowercase();
    if !matches!(cmd.as_str(), "cd" | "chdir") {
        return ParsedCd::NotCd;
    }
    // Handle `cd /d <path>` by skipping the `/d` flag.
    let mut idx = 1;
    if tokens.get(idx).map(|t| t.eq_ignore_ascii_case("/d")).unwrap_or(false) {
        idx += 1;
    }
    match tokens.get(idx) {
        None => ParsedCd::NotCd, // `cd` alone in cmd prints current dir, no change
        Some(arg) => resolve_target(arg, cwd, ShellVariant::Cmd),
    }
}

fn parse_bash(tokens: &[String], cwd: &Path) -> ParsedCd {
    if tokens[0] != "cd" {
        return ParsedCd::NotCd;
    }
    match tokens.get(1) {
        None => ParsedCd::ToHome,            // `cd`     → $HOME
        Some(arg) if arg == "-" => ParsedCd::SwapPrevious, // `cd -`
        Some(arg) if arg == "~" => ParsedCd::ToHome,       // `cd ~`
        Some(arg) if arg.starts_with("~/") => {
            // ~/foo → caller's home + "foo"
            let rest = &arg[2..];
            if let Some(home) = home_dir() {
                ParsedCd::ChangeTo(normalize(&home.join(rest)))
            } else {
                ParsedCd::NotCd
            }
        }
        Some(arg) => resolve_target(arg, cwd, ShellVariant::Bash),
    }
}

/// Resolve `arg` against `cwd` using the path syntax of `shell`, not the host
/// compile target's native syntax. Windows-flavored shells (pwsh/cmd) need
/// `\`-separated joining and drive-letter absolute detection regardless of
/// whether this binary is compiled for macOS/Linux/Windows — the shell being
/// parsed may be running on a different machine entirely (e.g. over SSH).
fn resolve_target(arg: &str, cwd: &Path, shell: ShellVariant) -> ParsedCd {
    if matches!(shell, ShellVariant::Pwsh | ShellVariant::Cmd) {
        let cwd_str = cwd.to_string_lossy();
        let joined = if is_windows_absolute(arg) {
            arg.to_string()
        } else {
            windows_join(&cwd_str, arg)
        };
        return ParsedCd::ChangeTo(PathBuf::from(windows_normalize(&joined)));
    }
    let p = Path::new(arg);
    let joined = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    ParsedCd::ChangeTo(normalize(&joined))
}

/// Windows-style absolute path: drive letter (`C:\`, `C:/`) or UNC (`\\server\share`).
fn is_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    let has_drive_letter = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    has_drive_letter || path.starts_with("\\\\") || path.starts_with("//")
}

/// Join a Windows-style relative segment onto a Windows-style base path,
/// as plain string concatenation (no host Path semantics involved).
fn windows_join(base: &str, rest: &str) -> String {
    if base.ends_with('\\') || base.ends_with('/') {
        format!("{base}{rest}")
    } else {
        format!("{base}\\{rest}")
    }
}

/// Collapse `.`/`..` components in a Windows-style path string, keeping the
/// drive-letter prefix (if any) rooted so `..` can never pop past it.
fn windows_normalize(path: &str) -> String {
    let (prefix, rest) = match path.as_bytes() {
        [_, b':', ..] => (&path[..2], &path[2..]),
        _ => ("", path),
    };
    let mut stack: Vec<&str> = Vec::new();
    for part in rest.split(['\\', '/']) {
        match part {
            "" | "." => {}
            ".." => { stack.pop(); }
            p => stack.push(p),
        }
    }
    format!("{prefix}\\{}", stack.join("\\"))
}

/// Parent of a Windows-style path (one level up). A drive root (`C:\`) has
/// no parent and is returned unchanged, matching Windows shell behavior.
fn windows_parent(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rfind(['\\', '/']) {
        Some(idx) => {
            let head = &trimmed[..idx];
            if head.ends_with(':') { format!("{head}\\") } else { head.to_string() }
        }
        None => path.to_string(),
    }
}

fn parent_or_root(p: &Path) -> PathBuf {
    PathBuf::from(windows_parent(&p.to_string_lossy()))
}

/// Normalize a path by collapsing `.` and `..` components without touching the
/// filesystem (no `canonicalize`, so no symlink resolution or I/O errors).
pub fn normalize(p: &Path) -> PathBuf {
    let mut stack: Vec<Component> = Vec::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                } else {
                    stack.push(comp);
                }
            }
            c => stack.push(c),
        }
    }
    let mut out = PathBuf::new();
    for c in stack { out.push(c.as_os_str()); }
    out
}

fn home_dir() -> Option<PathBuf> {
    // Keep zero-dep: read HOME (Unix) or USERPROFILE (Windows) directly.
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Scans raw incoming PTY output for OSC 133 `D` (command-finished) markers —
/// `ESC ] 1 3 3 ; D [ ; <exit-code> ] BEL` — and returns the exit code of each
/// one found, in order. Used to confirm whether a `cd` the write-path parser
/// staged actually succeeded in the real shell, instead of assuming every
/// typed `cd` line took effect (see `PtySession::pending_cds`).
///
/// Only bash/zsh and PowerShell's injected shell integration (see
/// `pty::shell::inject_shell_integration` / `inject_powershell_integration`)
/// emit a `D` marker with an exit code and terminate it with BEL (`\x07`).
/// cmd.exe's `PROMPT`-based marker has no exit code and uses a different
/// terminator (`ESC \`) — callers should not use this for `ShellVariant::Cmd`.
pub fn find_exit_codes(data: &[u8]) -> Vec<i32> {
    const MARKER: &[u8] = b"\x1b]133;D";
    let mut codes = Vec::new();
    let mut pos = 0;
    while let Some(start) = find_subslice(&data[pos..], MARKER) {
        let after_marker = pos + start + MARKER.len();
        // Scan forward for BEL, collecting an optional ";<digits>" in between.
        // Anything else before BEL (e.g. a second, unrelated OSC starting up)
        // means this wasn't a well-formed marker — skip past it and keep looking.
        let mut i = after_marker;
        let mut exit_code: i32 = 0;
        let mut has_digits = false;
        let mut malformed = false;
        if data.get(i) == Some(&b';') {
            i += 1;
            let digits_start = i;
            while data.get(i).map(u8::is_ascii_digit).unwrap_or(false) {
                i += 1;
            }
            if i == digits_start {
                malformed = true; // ';' with no digits after it
            } else {
                has_digits = true;
                exit_code = std::str::from_utf8(&data[digits_start..i])
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
        }
        if !malformed && data.get(i) == Some(&0x07) {
            codes.push(exit_code);
            pos = i + 1;
        } else {
            let _ = has_digits;
            pos = after_marker; // keep scanning past this malformed occurrence
        }
    }
    codes
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Scans a chunk of raw PTY output for a "bare" bell byte (`0x07`) — one that
/// is NOT the terminator of an OSC (`ESC ]`) escape sequence. Needed because
/// this codebase's own OSC 133 shell-integration hooks (see `pty::shell`) are
/// themselves BEL-terminated (as are some shells' OSC 0/2 window-title
/// sequences) — none of those represent a coding agent's own terminal-bell
/// notification, which is what `PtySession::bell_count` needs to detect.
/// Treats ANY OSC-introduced sequence generically (not just OSC 133), since
/// BEL-termination is a general, long-standing xterm convention for OSC, not
/// specific to shell-integration markers.
pub fn contains_bare_bell(data: &[u8]) -> bool {
    let mut i = 0;
    let mut in_osc = false;
    while i < data.len() {
        if !in_osc && data[i] == 0x1b && data.get(i + 1) == Some(&b']') {
            in_osc = true;
            i += 2;
            continue;
        }
        if data[i] == 0x07 {
            if in_osc {
                in_osc = false; // BEL terminates the OSC sequence — consumed, not a real bell
            } else {
                return true; // bare bell — a real notification
            }
        } else if in_osc && data[i] == 0x1b && data.get(i + 1) == Some(&b'\\') {
            in_osc = false; // ST also terminates an OSC sequence
            i += 1;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd(s: &str) -> PathBuf { PathBuf::from(s) }

    // --- ShellVariant::from_program ---

    #[test]
    fn shell_variant_from_program() {
        assert_eq!(ShellVariant::from_program("pwsh.exe"), ShellVariant::Pwsh);
        assert_eq!(ShellVariant::from_program("PowerShell.EXE"), ShellVariant::Pwsh);
        assert_eq!(ShellVariant::from_program("C:\\Windows\\System32\\cmd.exe"), ShellVariant::Cmd);
        assert_eq!(ShellVariant::from_program("/bin/bash"), ShellVariant::Bash);
        assert_eq!(ShellVariant::from_program("/usr/bin/zsh"), ShellVariant::Bash);
        assert_eq!(ShellVariant::from_program("sh"), ShellVariant::Bash);
        assert_eq!(ShellVariant::from_program("unknown-shell"), ShellVariant::Unknown);
    }

    // --- pwsh ---

    #[test]
    fn pwsh_cd_absolute() {
        let r = parse_cd("cd C:\\Windows", ShellVariant::Pwsh, &cwd("C:\\Users\\a"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\Windows")));
    }

    #[test]
    fn pwsh_cd_relative() {
        let r = parse_cd("cd foo", ShellVariant::Pwsh, &cwd("C:\\Users\\a"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\Users\\a\\foo")));
    }

    #[test]
    fn pwsh_set_location_synonym() {
        let r = parse_cd("Set-Location C:\\temp", ShellVariant::Pwsh, &cwd("C:\\"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\temp")));
    }

    #[test]
    fn pwsh_sl_alias() {
        let r = parse_cd("sl C:\\temp", ShellVariant::Pwsh, &cwd("C:\\"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\temp")));
    }

    #[test]
    fn pwsh_pushd() {
        let r = parse_cd("pushd C:\\temp", ShellVariant::Pwsh, &cwd("C:\\"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\temp")));
    }

    #[test]
    fn pwsh_cd_dotdot_shortcut() {
        let r = parse_cd("cd..", ShellVariant::Pwsh, &cwd("C:\\Users\\a"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\Users")));
    }

    #[test]
    fn pwsh_cd_dotdot_with_space() {
        let r = parse_cd("cd ..", ShellVariant::Pwsh, &cwd("C:\\Users\\a"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\Users")));
    }

    #[test]
    fn pwsh_cd_alone_is_noop() {
        // In pwsh, `cd` with no args prints current location — cwd does not change.
        let r = parse_cd("cd", ShellVariant::Pwsh, &cwd("C:\\Users\\a"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    #[test]
    fn pwsh_quoted_path_with_spaces() {
        let r = parse_cd("cd \"C:\\Program Files\"", ShellVariant::Pwsh, &cwd("C:\\"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\Program Files")));
    }

    #[test]
    fn pwsh_unrelated_command() {
        let r = parse_cd("Get-ChildItem", ShellVariant::Pwsh, &cwd("C:\\"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    // --- cmd ---

    #[test]
    fn cmd_cd_absolute() {
        let r = parse_cd("cd C:\\Windows", ShellVariant::Cmd, &cwd("C:\\Users\\a"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\Windows")));
    }

    #[test]
    fn cmd_cd_slash_d() {
        let r = parse_cd("cd /d D:\\projects", ShellVariant::Cmd, &cwd("C:\\"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("D:\\projects")));
    }

    #[test]
    fn cmd_chdir_synonym() {
        let r = parse_cd("chdir C:\\Windows", ShellVariant::Cmd, &cwd("C:\\Users"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\Windows")));
    }

    #[test]
    fn cmd_cd_alone_is_noop() {
        // In cmd, `cd` without args prints the current directory.
        let r = parse_cd("cd", ShellVariant::Cmd, &cwd("C:\\Users\\a"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    #[test]
    fn cmd_quoted_path() {
        let r = parse_cd("cd \"C:\\Program Files\"", ShellVariant::Cmd, &cwd("C:\\"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("C:\\Program Files")));
    }

    // --- bash ---

    #[test]
    fn bash_cd_absolute() {
        let r = parse_cd("cd /tmp", ShellVariant::Bash, &cwd("/home/a"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("/tmp")));
    }

    #[test]
    fn bash_cd_relative() {
        let r = parse_cd("cd foo", ShellVariant::Bash, &cwd("/home/a"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("/home/a/foo")));
    }

    #[test]
    fn bash_cd_parent() {
        let r = parse_cd("cd ..", ShellVariant::Bash, &cwd("/home/a"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("/home")));
    }

    #[test]
    fn bash_cd_alone_goes_home() {
        let r = parse_cd("cd", ShellVariant::Bash, &cwd("/tmp"));
        assert_eq!(r, ParsedCd::ToHome);
    }

    #[test]
    fn bash_cd_dash_swaps_previous() {
        let r = parse_cd("cd -", ShellVariant::Bash, &cwd("/tmp"));
        assert_eq!(r, ParsedCd::SwapPrevious);
    }

    #[test]
    fn bash_cd_tilde_goes_home() {
        let r = parse_cd("cd ~", ShellVariant::Bash, &cwd("/tmp"));
        assert_eq!(r, ParsedCd::ToHome);
    }

    #[test]
    fn bash_cd_quoted_path_with_spaces() {
        let r = parse_cd("cd '/tmp/my docs'", ShellVariant::Bash, &cwd("/"));
        assert_eq!(r, ParsedCd::ChangeTo(cwd("/tmp/my docs")));
    }

    #[test]
    fn bash_unrelated_command() {
        let r = parse_cd("ls -la", ShellVariant::Bash, &cwd("/"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    // --- conservative guards ---

    #[test]
    fn compound_with_ampersand_rejected() {
        let r = parse_cd("cd foo && ls", ShellVariant::Bash, &cwd("/"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    #[test]
    fn compound_with_semicolon_rejected() {
        let r = parse_cd("cd foo; ls", ShellVariant::Bash, &cwd("/"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    #[test]
    fn subshell_rejected() {
        let r = parse_cd("(cd foo && ls)", ShellVariant::Bash, &cwd("/"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    #[test]
    fn variable_expansion_rejected() {
        let r = parse_cd("cd $HOME", ShellVariant::Bash, &cwd("/"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    #[test]
    fn unbalanced_quotes_rejected() {
        let r = parse_cd("cd \"oops", ShellVariant::Bash, &cwd("/"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    #[test]
    fn empty_line_is_notcd() {
        let r = parse_cd("   ", ShellVariant::Bash, &cwd("/"));
        assert_eq!(r, ParsedCd::NotCd);
    }

    // --- normalize ---

    #[test]
    fn normalize_dotdot() {
        assert_eq!(normalize(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
    }

    #[test]
    fn normalize_curdir() {
        assert_eq!(normalize(Path::new("/a/./b")), PathBuf::from("/a/b"));
    }

    #[test]
    fn normalize_leading_dotdot_preserved() {
        // No cwd context → parent-of-root is left as-is (parser's caller should
        // never send an unrooted path into normalize anyway).
        assert_eq!(normalize(Path::new("../a")), PathBuf::from("../a"));
    }

    // --- find_exit_codes ---

    #[test]
    fn find_exit_codes_success() {
        let data = b"\x1b]133;D;0\x07";
        assert_eq!(find_exit_codes(data), vec![0]);
    }

    #[test]
    fn find_exit_codes_failure() {
        let data = b"\x1b]133;D;1\x07";
        assert_eq!(find_exit_codes(data), vec![1]);
    }

    #[test]
    fn find_exit_codes_multi_digit() {
        let data = b"\x1b]133;D;127\x07";
        assert_eq!(find_exit_codes(data), vec![127]);
    }

    #[test]
    fn find_exit_codes_no_exit_code_defaults_to_zero() {
        // Matches the shell-integration script's own fallback when $? is
        // somehow empty — treat a bare "D" (no ";N") as success.
        let data = b"\x1b]133;D\x07";
        assert_eq!(find_exit_codes(data), vec![0]);
    }

    #[test]
    fn find_exit_codes_embedded_in_real_output() {
        let data = b"total 24\r\ndrwxr-xr-x  1 a  staff\r\n\x1b]133;D;0\x07\x1b]133;A\x07jamesju@mac ~ % ";
        assert_eq!(find_exit_codes(data), vec![0]);
    }

    #[test]
    fn find_exit_codes_multiple_in_one_chunk() {
        let data = b"\x1b]133;D;0\x07...\x1b]133;D;1\x07";
        assert_eq!(find_exit_codes(data), vec![0, 1]);
    }

    #[test]
    fn find_exit_codes_no_marker_returns_empty() {
        let data = b"just some regular output, no markers here\r\n";
        assert_eq!(find_exit_codes(data), Vec::<i32>::new());
    }

    #[test]
    fn find_exit_codes_ignores_malformed_marker() {
        // ';' with no digits after it before BEL — not a well-formed marker.
        let data = b"\x1b]133;D;\x07";
        assert_eq!(find_exit_codes(data), Vec::<i32>::new());
    }

    #[test]
    fn find_exit_codes_ignores_marker_missing_bel() {
        let data = b"\x1b]133;D;0 no bel here";
        assert_eq!(find_exit_codes(data), Vec::<i32>::new());
    }

    // --- contains_bare_bell ---

    #[test]
    fn contains_bare_bell_true_for_a_plain_bell_byte() {
        assert!(contains_bare_bell(b"hello\x07world"));
    }

    #[test]
    fn contains_bare_bell_false_for_osc133_a_marker() {
        assert!(!contains_bare_bell(b"\x1b]133;A\x07"));
    }

    #[test]
    fn contains_bare_bell_false_for_osc133_c_marker() {
        assert!(!contains_bare_bell(b"\x1b]133;C\x07"));
    }

    #[test]
    fn contains_bare_bell_false_for_osc133_d_marker_with_exit_code() {
        assert!(!contains_bare_bell(b"\x1b]133;D;0\x07"));
    }

    #[test]
    fn contains_bare_bell_true_when_a_real_bell_follows_an_osc133_marker() {
        // Realistic case: shell draws its prompt (OSC133 A, BEL-terminated,
        // not a real notification), then later the agent inside genuinely
        // rings the bell to say it's waiting for input.
        assert!(contains_bare_bell(b"\x1b]133;A\x07some output\x07"));
    }

    #[test]
    fn contains_bare_bell_false_for_st_terminated_osc_sequence_with_later_bare_bell_absent() {
        // OSC sequences can also be terminated with ST (ESC \\) instead of BEL.
        // No bell anywhere here at all.
        assert!(!contains_bare_bell(b"\x1b]0;window title\x1b\\"));
    }

    #[test]
    fn contains_bare_bell_false_for_empty_input() {
        assert!(!contains_bare_bell(b""));
    }
}
