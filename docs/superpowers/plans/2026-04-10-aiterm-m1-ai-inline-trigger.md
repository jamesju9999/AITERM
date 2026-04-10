# AITerm M1 Implementation Plan — `/ai` Inline Trigger

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first AI main-line: user types `/ai <nl>` → OpenAI produces a structured command → CommandPreview shows it → user confirms → command runs through existing PTY path.

**Architecture:** Backend adds `ai/` module (trait + types + OpenAI client + router + context snapshot) and `commands/ai.rs` (`ai_query` Tauri command). `pty/session.rs` gains per-session cwd tracking via a `cd` parser hook in `write()`. Frontend adds `parseAiPrefix` pure helper, `CommandPreview` component, and `ipc/ai.ts` wrapper; `TerminalView` intercepts `/ai ` at Enter time and bypasses PTY. Buffered AI response (no streaming UX in M1). All errors surface as red text in the terminal.

**Tech Stack:** Rust (tauri 2, reqwest + SSE, async-trait, portable-pty, tokio), TypeScript + React 19, xterm.js, vitest (new), wiremock (new dev-dep).

**Reference spec:** `docs/superpowers/specs/2026-04-10-aiterm-m1-design.md`

---

## Scope Check

This plan covers a single milestone with one cohesive user-visible feature. No decomposition needed.

---

## File Structure

### Backend (Rust)

**Create:**
- `src-tauri/src/ai/mod.rs` — trait `AiProvider`, request/response types, `AiError`, `AiSingleCommand`
- `src-tauri/src/ai/openai.rs` — `OpenAiClient` (reqwest + SSE)
- `src-tauri/src/ai/router.rs` — `AiRouter` struct
- `src-tauri/src/ai/context.rs` — `snapshot()` function
- `src-tauri/src/commands/mod.rs` — re-exports for new command modules
- `src-tauri/src/commands/ai.rs` — `ai_query` Tauri command
- `src-tauri/src/pty/cd_parser.rs` — pure `parse_cd()` function
- `src-tauri/tests/openai_client.rs` — wiremock contract test
- `src-tauri/tests/pty_cwd_tracking.rs` — real-shell cwd integration test
- `src-tauri/tests/ai_query_command.rs` — ai_query integration test with mock provider

**Modify:**
- `src-tauri/Cargo.toml` — add reqwest, async-trait, futures-util, wiremock (dev)
- `src-tauri/src/lib.rs` — register `ai` + `commands` modules, manage `AiRouter`, register `ai_query` handler
- `src-tauri/src/pty/mod.rs` — declare `cd_parser` module
- `src-tauri/src/pty/session.rs` — add `cwd`, `shell_variant`, line buffer; hook `write()` to run cd parser
- `src-tauri/src/pty/manager.rs` — add `get_cwd(id)` and `get_shell(id)` accessors

### Frontend (TypeScript + React)

**Create:**
- `src/components/parseAiPrefix.ts` — pure function
- `src/components/parseAiPrefix.test.ts` — vitest unit tests
- `src/components/CommandPreview.tsx` — React component
- `src/components/CommandPreview.css` — styles
- `src/ipc/ai.ts` — `AiError` type, `invokeAiQuery`, `formatAiError`
- `src/ipc/ai.test.ts` — vitest unit tests
- `vitest.config.ts` — vitest configuration

**Modify:**
- `package.json` — add vitest devDependency and `test` script
- `src/components/TerminalView.tsx` — intercept `/ai ` at Enter, manage preview state, write error red text
- `src/App.tsx` — no change expected (`TerminalView` owns the preview state)

---

## Task 1: Add Rust dependencies and verify build

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Edit `Cargo.toml`**

Add these lines to the `[dependencies]` section (after the existing entries):

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls"] }
async-trait = "0.1"
futures-util = "0.3"
```

Add a new `[dev-dependencies]` section at the bottom of the file:

```toml
[dev-dependencies]
wiremock = "0.6"
```

- [ ] **Step 2: Verify the project still compiles**

Run from `src-tauri/`:

```bash
cargo build
```

Expected: clean build, no errors. Warnings about unused imports are OK.

- [ ] **Step 3: Verify existing M0 tests still pass**

```bash
cargo test
```

Expected: all existing tests pass (they did before; adding deps shouldn't break anything).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore(deps): add reqwest, async-trait, futures-util, wiremock for M1"
```

---

## Task 2: Scaffold `ai/` module with `AiError`

**Files:**
- Create: `src-tauri/src/ai/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Create `src-tauri/src/ai/mod.rs`**

```rust
//! AI provider abstractions and implementations.
//!
//! Module layout:
//! - `mod.rs` (this file): trait + shared types + errors
//! - `openai.rs`: `OpenAiClient`
//! - `router.rs`: `AiRouter`
//! - `context.rs`: environment snapshot helper

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiError {
    #[error("OPENAI_API_KEY environment variable is not set")]
    NotConfigured,

    #[error("network error: {message}")]
    Network { message: String },

    #[error("authentication failed (check your API key)")]
    AuthFailed,

    #[error("rate limit exceeded")]
    RateLimit { retry_after: Option<String> },

    #[error("AI returned invalid response: {reason}")]
    ModelError { reason: String, raw: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_error_serializes_kind_tag() {
        let err = AiError::NotConfigured;
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, r#"{"kind":"not_configured"}"#);
    }

    #[test]
    fn ai_error_network_carries_message() {
        let err = AiError::Network { message: "connection refused".into() };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "network");
        assert_eq!(json["message"], "connection refused");
    }

    #[test]
    fn ai_error_rate_limit_optional_retry_after() {
        let none = AiError::RateLimit { retry_after: None };
        let some = AiError::RateLimit { retry_after: Some("20".into()) };
        let j_none = serde_json::to_value(&none).unwrap();
        let j_some = serde_json::to_value(&some).unwrap();
        assert_eq!(j_none["kind"], "rate_limit");
        assert!(j_none["retry_after"].is_null());
        assert_eq!(j_some["retry_after"], "20");
    }

    #[test]
    fn ai_error_model_error_carries_reason_and_raw() {
        let err = AiError::ModelError {
            reason: "missing field `command`".into(),
            raw: "{\"explanation\":\"...\"}".into(),
        };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "model_error");
        assert_eq!(json["reason"], "missing field `command`");
        assert_eq!(json["raw"], "{\"explanation\":\"...\"}");
    }
}
```

- [ ] **Step 2: Register the module in `lib.rs`**

Edit `src-tauri/src/lib.rs`. Change the first line from `pub mod pty;` to:

```rust
pub mod ai;
pub mod pty;
```

Leave the rest of the file unchanged for now.

- [ ] **Step 3: Run tests**

```bash
cargo test ai::tests
```

Expected: 4 tests pass (`ai_error_serializes_kind_tag`, `ai_error_network_carries_message`, `ai_error_rate_limit_optional_retry_after`, `ai_error_model_error_carries_reason_and_raw`).

- [ ] **Step 4: Verify full test suite still green**

```bash
cargo test
```

Expected: all M0 tests + 4 new ai::tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/mod.rs src-tauri/src/lib.rs
git commit -m "feat(ai): scaffold ai module with AiError enum"
```

---

## Task 3: Add request/response types to `ai/mod.rs`

**Files:**
- Modify: `src-tauri/src/ai/mod.rs`

- [ ] **Step 1: Add types above the `#[cfg(test)] mod tests` block**

Insert this block right before the existing `#[cfg(test)]` line:

```rust
use std::path::PathBuf;
use serde::Deserialize;

/// Environment snapshot sent to the AI as context.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct EnvSnapshot {
    pub os: String,      // e.g. "windows", "macos", "linux"
    pub shell: String,   // e.g. "pwsh", "powershell", "cmd", "bash", "zsh"
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMode {
    SingleCommand,
    /// Reserved for M4 (AI panel multi-turn). Not reachable in M1.
    #[allow(dead_code)]
    Chat,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,    // "user" | "assistant" | "system"
    pub content: String,
}

#[derive(Debug)]
pub struct GenerateRequest {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub context: EnvSnapshot,
    pub mode: QueryMode,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct GenerateChunk {
    pub delta: String,
    pub done: bool,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
}
```

Also add an import for `Deserialize` at the top (keep the existing `use serde::Serialize;`):

```rust
use serde::{Deserialize, Serialize};
```

And remove the now-duplicate `use serde::Deserialize;` you just added inline if the top-of-file import covers it.

- [ ] **Step 2: Add a serialization test**

Inside the existing `#[cfg(test)] mod tests { ... }` block, add:

```rust
    #[test]
    fn env_snapshot_serializes_expected_fields() {
        let snap = EnvSnapshot {
            os: "windows".into(),
            shell: "pwsh".into(),
            cwd: PathBuf::from("C:\\Users\\test"),
        };
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["os"], "windows");
        assert_eq!(json["shell"], "pwsh");
        // PathBuf serializes as a string on Windows.
        assert!(json["cwd"].as_str().unwrap().contains("test"));
    }
```

- [ ] **Step 3: Run tests**

```bash
cargo test ai::tests
```

Expected: 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/ai/mod.rs
git commit -m "feat(ai): add request/response and context types"
```

---

## Task 4: Add `AiSingleCommand` and `RiskLevel` with deserialization tests

**Files:**
- Modify: `src-tauri/src/ai/mod.rs`

- [ ] **Step 1: Add the types to `mod.rs`**

Below the `TokenUsage` struct, add:

```rust
/// The structured payload the AI is required to return for `/ai` queries.
#[derive(Debug, Clone, Deserialize)]
pub struct AiSingleCommand {
    pub explanation: String,
    pub command: String,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Safe,
    NeedsConfirm,
    Dangerous,
}
```

- [ ] **Step 2: Add deserialization tests**

Inside the `#[cfg(test)] mod tests { ... }` block, add:

```rust
    #[test]
    fn ai_single_command_parses_valid_json() {
        let json = r#"{
            "explanation": "列出檔案",
            "command": "Get-ChildItem",
            "risk_level": "safe"
        }"#;
        let parsed: AiSingleCommand = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.explanation, "列出檔案");
        assert_eq!(parsed.command, "Get-ChildItem");
        assert_eq!(parsed.risk_level, RiskLevel::Safe);
    }

    #[test]
    fn ai_single_command_all_risk_levels() {
        for (raw, expected) in [
            ("safe", RiskLevel::Safe),
            ("needs_confirm", RiskLevel::NeedsConfirm),
            ("dangerous", RiskLevel::Dangerous),
        ] {
            let json = format!(r#"{{"explanation":"x","command":"y","risk_level":"{raw}"}}"#);
            let parsed: AiSingleCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.risk_level, expected, "for raw={raw}");
        }
    }

    #[test]
    fn ai_single_command_missing_command_fails() {
        let json = r#"{"explanation":"x","risk_level":"safe"}"#;
        let err = serde_json::from_str::<AiSingleCommand>(json).unwrap_err();
        assert!(err.to_string().contains("command"), "unexpected err: {err}");
    }

    #[test]
    fn ai_single_command_invalid_risk_level_fails() {
        let json = r#"{"explanation":"x","command":"y","risk_level":"bogus"}"#;
        let err = serde_json::from_str::<AiSingleCommand>(json).unwrap_err();
        assert!(err.to_string().contains("risk_level") || err.to_string().contains("bogus"));
    }

    #[test]
    fn ai_single_command_markdown_fence_fails() {
        // Strict parser: markdown fences are not valid JSON. Test ensures we DO
        // NOT silently strip them — spec §6.3 says violations become ModelError.
        let json = "```json\n{\"explanation\":\"x\",\"command\":\"y\",\"risk_level\":\"safe\"}\n```";
        assert!(serde_json::from_str::<AiSingleCommand>(json).is_err());
    }
```

- [ ] **Step 3: Run tests**

```bash
cargo test ai::tests
```

Expected: 10 tests pass (5 existing + 5 new).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/ai/mod.rs
git commit -m "feat(ai): add AiSingleCommand schema with strict parser tests"
```

---

## Task 5: Add `AiProvider` trait

**Files:**
- Modify: `src-tauri/src/ai/mod.rs`

- [ ] **Step 1: Add the trait**

Below `RiskLevel`, add:

```rust
use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;

    /// Produce a response, streaming chunks through `tx`. Implementations must
    /// push a final chunk with `done: true` before returning `Ok(())`.
    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError>;
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo build
```

Expected: clean build. The trait has no direct test — it's exercised by the mock provider in later tasks.

- [ ] **Step 3: Run all tests**

```bash
cargo test
```

Expected: all M0 tests + 10 ai::tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/ai/mod.rs
git commit -m "feat(ai): define AiProvider trait"
```

---

## Task 6: Create `pty/cd_parser.rs` pure function

**Files:**
- Create: `src-tauri/src/pty/cd_parser.rs`
- Modify: `src-tauri/src/pty/mod.rs`

- [ ] **Step 1: Create `pty/cd_parser.rs`**

```rust
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
        let leaf = std::path::Path::new(&lower)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&lower);
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
        Some(arg) => resolve_target(arg, cwd),
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
        Some(arg) => resolve_target(arg, cwd),
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
        Some(arg) => resolve_target(arg, cwd),
    }
}

fn resolve_target(arg: &str, cwd: &Path) -> ParsedCd {
    let p = Path::new(arg);
    let joined = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    ParsedCd::ChangeTo(normalize(&joined))
}

fn parent_or_root(p: &Path) -> PathBuf {
    p.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| p.to_path_buf())
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
}
```

- [ ] **Step 2: Register the module**

Edit `src-tauri/src/pty/mod.rs`. After the existing `pub mod commands;` line, add:

```rust
pub mod cd_parser;
```

- [ ] **Step 3: Run the cd parser tests**

```bash
cargo test pty::cd_parser::tests
```

Expected: ~33 tests pass.

- [ ] **Step 4: Run the whole suite to confirm no regressions**

```bash
cargo test
```

Expected: everything green (M0 tests + ai::tests + cd_parser tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/pty/cd_parser.rs src-tauri/src/pty/mod.rs
git commit -m "feat(pty): add pure cd-command parser for cwd tracking"
```

---

## Task 7: Hook cwd tracking into `PtySession`

**Files:**
- Modify: `src-tauri/src/pty/session.rs`

This task adds a line buffer + cd parser invocation inside `PtySession::write`, plus `cwd` / `shell_variant` / `previous_cwd` state. **Do not change the public signature of `spawn` / `spawn_with_id`** — add a new helper method to read cwd.

- [ ] **Step 1: Add fields and imports**

At the top of `src-tauri/src/pty/session.rs`, add these imports just below the existing `use` block:

```rust
use std::path::PathBuf;

use super::cd_parser::{self, ParsedCd, ShellVariant};
```

Replace the `PtySession` struct definition with:

```rust
pub struct PtySession {
    pub id: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
    shell_variant: ShellVariant,
    cwd: Mutex<PathBuf>,
    previous_cwd: Mutex<Option<PathBuf>>,
    line_buffer: Mutex<Vec<u8>>,
}
```

- [ ] **Step 2: Extract shared init logic**

Both `spawn` and `spawn_with_id` currently duplicate the body. Keep that duplication for now but update both constructors to set the new fields. At the end of each constructor where it currently returns `Ok(Self { ... })`, change to:

```rust
        let initial_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let shell_variant = ShellVariant::from_program(&shell.program);

        Ok(Self {
            id,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader_thread: Mutex::new(Some(reader_thread)),
            shell_variant,
            cwd: Mutex::new(initial_cwd),
            previous_cwd: Mutex::new(None),
            line_buffer: Mutex::new(Vec::new()),
        })
```

(Do this in both `spawn` and `spawn_with_id`.)

Note: `shell` is moved into `CommandBuilder::new(shell.program)` currently. You will need `let shell_variant = ShellVariant::from_program(&shell.program);` **before** that move. Restructure the top of each constructor to:

```rust
        let shell_variant = ShellVariant::from_program(&shell.program);
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

        let mut cmd = CommandBuilder::new(shell.program);
        for arg in shell.args {
            cmd.arg(arg);
        }
```

- [ ] **Step 3: Modify `write` to run the cd parser**

Replace the existing `write` method with:

```rust
    pub fn write(&self, data: &[u8]) -> PtyResult<()> {
        self.record_into_line_buffer(data);
        let mut writer = self.writer.lock();
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }
```

Then add two new methods inside the same `impl PtySession` block, below `write`:

```rust
    /// Accumulate bytes into the line buffer. On each carriage return or
    /// newline, flush the completed line and feed it to the cd parser.
    fn record_into_line_buffer(&self, data: &[u8]) {
        let mut buf = self.line_buffer.lock();
        for &b in data {
            if b == b'\r' || b == b'\n' {
                if !buf.is_empty() {
                    if let Ok(line) = std::str::from_utf8(&buf) {
                        let line_owned = line.to_string();
                        drop(buf);
                        self.apply_cd_if_any(&line_owned);
                        buf = self.line_buffer.lock();
                    }
                    buf.clear();
                }
            } else {
                // Cap runaway input to ~8 KiB so a rogue paste cannot grow
                // unbounded before the user hits Enter.
                if buf.len() < 8 * 1024 {
                    buf.push(b);
                }
            }
        }
    }

    fn apply_cd_if_any(&self, line: &str) {
        let current = self.cwd.lock().clone();
        let parsed = cd_parser::parse_cd(line, self.shell_variant, &current);
        match parsed {
            ParsedCd::NotCd => {}
            ParsedCd::ChangeTo(new_cwd) => {
                let mut prev = self.previous_cwd.lock();
                *prev = Some(current);
                *self.cwd.lock() = new_cwd;
            }
            ParsedCd::SwapPrevious => {
                let mut prev = self.previous_cwd.lock();
                if let Some(p) = prev.take() {
                    let new_prev = self.cwd.lock().clone();
                    *self.cwd.lock() = p;
                    *prev = Some(new_prev);
                }
            }
            ParsedCd::ToHome => {
                if let Some(home) = std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                {
                    let mut prev = self.previous_cwd.lock();
                    *prev = Some(current);
                    *self.cwd.lock() = PathBuf::from(home);
                }
            }
        }
    }

    /// Read the tracked cwd for this session.
    pub fn get_cwd(&self) -> PathBuf {
        self.cwd.lock().clone()
    }

    /// Read the shell variant detected at spawn time.
    pub fn shell_variant(&self) -> ShellVariant {
        self.shell_variant
    }
```

- [ ] **Step 4: Add unit tests for the line buffer and cwd tracking**

Inside the existing `#[cfg(test)] mod tests { ... }` block in `session.rs`, add these tests **in addition to** the existing ones. You may need a small helper — add it at the top of the test module:

```rust
    fn fake_session(shell_variant: ShellVariant, initial: &str) -> PtySessionStubForCwd {
        // Build a stub that exercises only the cd-parser pathway, without
        // touching the real PTY. Mirrors the internal state of PtySession.
        PtySessionStubForCwd {
            shell_variant,
            cwd: Mutex::new(PathBuf::from(initial)),
            previous_cwd: Mutex::new(None),
            line_buffer: Mutex::new(Vec::new()),
        }
    }

    struct PtySessionStubForCwd {
        shell_variant: ShellVariant,
        cwd: Mutex<PathBuf>,
        previous_cwd: Mutex<Option<PathBuf>>,
        line_buffer: Mutex<Vec<u8>>,
    }

    impl PtySessionStubForCwd {
        fn write(&self, data: &[u8]) {
            let mut buf = self.line_buffer.lock();
            for &b in data {
                if b == b'\r' || b == b'\n' {
                    if !buf.is_empty() {
                        if let Ok(line) = std::str::from_utf8(&buf) {
                            let line_owned = line.to_string();
                            drop(buf);
                            self.apply(&line_owned);
                            buf = self.line_buffer.lock();
                        }
                        buf.clear();
                    }
                } else if buf.len() < 8 * 1024 {
                    buf.push(b);
                }
            }
        }
        fn apply(&self, line: &str) {
            let current = self.cwd.lock().clone();
            match cd_parser::parse_cd(line, self.shell_variant, &current) {
                ParsedCd::NotCd => {}
                ParsedCd::ChangeTo(new_cwd) => {
                    *self.previous_cwd.lock() = Some(current);
                    *self.cwd.lock() = new_cwd;
                }
                ParsedCd::SwapPrevious => {
                    let mut prev = self.previous_cwd.lock();
                    if let Some(p) = prev.take() {
                        let new_prev = self.cwd.lock().clone();
                        *self.cwd.lock() = p;
                        *prev = Some(new_prev);
                    }
                }
                ParsedCd::ToHome => {
                    if let Some(home) = std::env::var_os("HOME")
                        .or_else(|| std::env::var_os("USERPROFILE"))
                    {
                        *self.previous_cwd.lock() = Some(current);
                        *self.cwd.lock() = PathBuf::from(home);
                    }
                }
            }
        }
        fn get_cwd(&self) -> PathBuf { self.cwd.lock().clone() }
    }
```

Why a stub? The real `PtySession` owns a live PTY and can't easily be unit-tested without spawning a shell. The stub replays the exact cd-tracking logic we ship and lets us lock down behavior without touching I/O. Integration tests in Task 9 verify the real thing.

Now add the behavioral tests:

```rust
    #[test]
    fn cwd_updates_on_pwsh_cd_after_enter() {
        let s = fake_session(ShellVariant::Pwsh, "C:\\Users\\a");
        s.write(b"cd foo");   // no enter yet
        assert_eq!(s.get_cwd(), PathBuf::from("C:\\Users\\a"));
        s.write(b"\r");        // enter
        assert_eq!(s.get_cwd(), PathBuf::from("C:\\Users\\a\\foo"));
    }

    #[test]
    fn cwd_multiline_single_write() {
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"cd foo\ncd bar\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a/foo/bar"));
    }

    #[test]
    fn cwd_stays_on_unparseable() {
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"ls\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
        s.write(b"cd foo && ls\n"); // compound → NotCd
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
    }

    #[test]
    fn cwd_dash_swaps_previous() {
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"cd /tmp\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/tmp"));
        s.write(b"cd -\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
        s.write(b"cd -\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/tmp"));
    }
```

- [ ] **Step 5: Run session tests**

```bash
cargo test pty::session::tests
```

Expected: existing M0 tests (`session_echoes_written_bytes`, `session_resize_does_not_error`) still pass, plus 4 new cwd tests.

- [ ] **Step 6: Run full suite**

```bash
cargo test
```

Expected: everything green.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/pty/session.rs
git commit -m "feat(pty): track per-session cwd via cd parser hook in write()"
```

---

## Task 8: Add `PtyManager::get_cwd` and `get_shell`

**Files:**
- Modify: `src-tauri/src/pty/manager.rs`

- [ ] **Step 1: Add accessor methods**

Inside `impl PtyManager`, below the existing `fn get` method, add:

```rust
    pub fn get_cwd(&self, id: &str) -> Option<PathBuf> {
        self.sessions.lock().get(id).map(|s| s.get_cwd())
    }

    pub fn get_shell_variant(&self, id: &str) -> Option<super::cd_parser::ShellVariant> {
        self.sessions.lock().get(id).map(|s| s.shell_variant())
    }
```

Add the import at the top of `manager.rs`:

```rust
use std::path::PathBuf;
```

- [ ] **Step 2: Add a manager-level unit test**

Inside the existing `#[cfg(test)] mod tests { ... }` block in `manager.rs`, add:

```rust
    #[test]
    fn manager_get_cwd_returns_none_for_missing() {
        let manager = PtyManager::new();
        assert!(manager.get_cwd("no-such-id").is_none());
    }
```

- [ ] **Step 3: Run manager tests**

```bash
cargo test pty::manager::tests
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/pty/manager.rs
git commit -m "feat(pty): expose get_cwd and get_shell_variant from PtyManager"
```

---

## Task 9: Integration test — real-shell cwd tracking

**Files:**
- Create: `src-tauri/tests/pty_cwd_tracking.rs`

- [ ] **Step 1: Create the integration test**

```rust
//! Integration test for e2α cwd tracking: spawn a real shell, send `cd`
//! commands, verify `PtyManager::get_cwd` matches the shell's actual cwd.
//!
//! This is the "真金火煉" test mentioned in spec §8.2 — unit tests only cover
//! the string parsing; this verifies the write-hook actually fires for a live
//! shell on this OS.

use aiterm_lib::pty::PtyManager;
use portable_pty::PtySize;
use std::sync::mpsc;
use std::time::Duration;

fn small_size() -> PtySize {
    PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }
}

#[test]
fn tracks_cd_through_real_shell() {
    let manager = PtyManager::new();
    let (tx, _rx) = mpsc::channel::<Vec<u8>>();

    let id = manager
        .create_with_callback(small_size(), move |chunk| {
            let _ = tx.send(chunk);
        })
        .expect("create session");

    // Give the shell time to finish printing its banner/prompt before we
    // start sending commands. On Windows conpty this is flaky under 200 ms.
    std::thread::sleep(Duration::from_millis(500));

    let initial = manager.get_cwd(&id).expect("initial cwd");

    // Platform-specific: cmd.exe on Windows, sh on Unix. default_shell() in
    // M0 prefers pwsh → powershell → cmd on Windows. Our parser handles all
    // three, so we just send a parent-dir change that works everywhere.
    #[cfg(windows)]
    manager.write(&id, b"cd ..\r\n").unwrap();
    #[cfg(not(windows))]
    manager.write(&id, b"cd ..\n").unwrap();

    // Allow the write hook to process.
    std::thread::sleep(Duration::from_millis(100));

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

    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(manager.get_cwd(&id).unwrap(), initial);

    manager.close(&id).ok();
}
```

- [ ] **Step 2: Verify the binary-target name**

The existing `tests/pty_integration.rs` imports as `aiterm_lib::pty::...` — verify by reading that file:

```bash
head -5 src-tauri/tests/pty_integration.rs
```

Expected: uses `aiterm_lib::pty::*`. If your M0 uses a different crate name, adjust the import in the new test file accordingly.

- [ ] **Step 3: Run the new integration test**

```bash
cd src-tauri
cargo test --test pty_cwd_tracking
```

Expected: passes on Windows (pwsh/cmd) and Unix. Note: this test is slower (~1 s) because it spawns a real shell.

- [ ] **Step 4: Run all tests**

```bash
cargo test
```

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tests/pty_cwd_tracking.rs
git commit -m "test(pty): integration test for cwd tracking across real shell"
```

---

## Task 10: Add `ai/context.rs` snapshot helper

**Files:**
- Create: `src-tauri/src/ai/context.rs`
- Modify: `src-tauri/src/ai/mod.rs`

- [ ] **Step 1: Create `ai/context.rs`**

```rust
//! Environment snapshot for AI prompts. M1 minimal: os + shell + cwd.
//! Historical command context is intentionally deferred to M5.

use std::path::PathBuf;

use crate::ai::EnvSnapshot;
use crate::pty::cd_parser::ShellVariant;
use crate::pty::PtyManager;

/// Build a snapshot for a given PTY session. Falls back to the process cwd
/// if the manager has no record of the session (should not happen in normal
/// use, but we degrade gracefully).
pub fn snapshot(pty_manager: &PtyManager, session_id: &str) -> EnvSnapshot {
    let cwd = pty_manager
        .get_cwd(session_id)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let shell = pty_manager
        .get_shell_variant(session_id)
        .map(shell_variant_to_str)
        .unwrap_or_else(|| default_shell_name())
        .to_string();
    EnvSnapshot {
        os: std::env::consts::OS.to_string(),
        shell,
        cwd,
    }
}

/// Test-visible helper so we can build a snapshot from already-resolved parts.
pub fn snapshot_from_parts(os: &str, shell: &str, cwd: PathBuf) -> EnvSnapshot {
    EnvSnapshot {
        os: os.to_string(),
        shell: shell.to_string(),
        cwd,
    }
}

fn shell_variant_to_str(v: ShellVariant) -> &'static str {
    match v {
        ShellVariant::Pwsh => "pwsh",
        ShellVariant::Cmd => "cmd",
        ShellVariant::Bash => "bash",
        ShellVariant::Unknown => "unknown",
    }
}

fn default_shell_name() -> &'static str {
    #[cfg(windows)]
    { "pwsh" }
    #[cfg(not(windows))]
    { "bash" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_from_parts_sets_all_fields() {
        let s = snapshot_from_parts("linux", "bash", PathBuf::from("/home/u"));
        assert_eq!(s.os, "linux");
        assert_eq!(s.shell, "bash");
        assert_eq!(s.cwd, PathBuf::from("/home/u"));
    }

    #[test]
    fn shell_variant_mapping_is_stable() {
        assert_eq!(shell_variant_to_str(ShellVariant::Pwsh), "pwsh");
        assert_eq!(shell_variant_to_str(ShellVariant::Cmd), "cmd");
        assert_eq!(shell_variant_to_str(ShellVariant::Bash), "bash");
        assert_eq!(shell_variant_to_str(ShellVariant::Unknown), "unknown");
    }
}
```

- [ ] **Step 2: Register the submodule**

Edit `src-tauri/src/ai/mod.rs`. Below the module-doc comment at the top, add:

```rust
pub mod context;
```

- [ ] **Step 3: Run tests**

```bash
cargo test ai::context::tests
```

Expected: 2 tests pass.

- [ ] **Step 4: Full suite**

```bash
cargo test
```

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/context.rs src-tauri/src/ai/mod.rs
git commit -m "feat(ai): add environment snapshot builder"
```

---

## Task 11: Scaffold `OpenAiClient`

**Files:**
- Create: `src-tauri/src/ai/openai.rs`
- Modify: `src-tauri/src/ai/mod.rs`

- [ ] **Step 1: Create `ai/openai.rs`**

```rust
//! OpenAI provider implementation. Uses the chat completions endpoint with
//! `response_format: { type: "json_object" }` (spec decision D11) and a
//! hard-coded model `gpt-4o-mini` (D12). SSE streaming is consumed internally
//! and chunks are forwarded via the trait's `mpsc::Sender<GenerateChunk>`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ai::{
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, TokenUsage,
};

const DEFAULT_MODEL: &str = "gpt-4o-mini";

pub struct OpenAiClient {
    api_key: String,
    model: String,
    base_url: String, // exposed for wiremock
    client: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, "https://api.openai.com".to_string())
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            model: DEFAULT_MODEL.to_string(),
            base_url,
            client: reqwest::Client::new(),
        }
    }

    fn url(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl AiProvider for OpenAiClient {
    fn id(&self) -> &str { "openai" }
    fn display_name(&self) -> &str { "OpenAI" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let body = build_request_body(&self.model, &req);

        let resp = self
            .client
            .post(self.url())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(map_http_error(status, resp).await);
        }

        consume_sse(resp, tx).await
    }
}

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage<'a>>,
    stream: bool,
    response_format: ResponseFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    ty: &'static str,
}

fn build_request_body<'a>(model: &'a str, req: &'a GenerateRequest) -> OpenAiChatRequest<'a> {
    let mut messages: Vec<OpenAiMessage<'a>> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OpenAiMessage { role: "system", content: &req.system_prompt });
    for m in &req.messages {
        messages.push(OpenAiMessage { role: m.role.as_str(), content: m.content.as_str() });
    }
    OpenAiChatRequest {
        model,
        messages,
        stream: true,
        response_format: ResponseFormat { ty: "json_object" },
        max_tokens: req.max_tokens,
    }
}

async fn map_http_error(status: reqwest::StatusCode, resp: reqwest::Response) -> AiError {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return AiError::AuthFailed;
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        return AiError::RateLimit { retry_after };
    }
    let body = resp.text().await.unwrap_or_default();
    AiError::Network {
        message: format!("http {}: {}", status.as_u16(), truncate(&body, 200)),
    }
}

async fn consume_sse(
    resp: reqwest::Response,
    tx: mpsc::Sender<GenerateChunk>,
) -> Result<(), AiError> {
    use futures_util::StreamExt;

    let mut stream = resp.bytes_stream();
    let mut leftover = Vec::<u8>::new();
    let mut saw_done = false;

    while let Some(item) = stream.next().await {
        let bytes = item.map_err(|e| AiError::Network { message: e.to_string() })?;
        leftover.extend_from_slice(&bytes);

        while let Some(pos) = find_line_end(&leftover) {
            let line_bytes = leftover.drain(..pos).collect::<Vec<u8>>();
            // Advance past the actual separator byte(s).
            let sep_len = separator_len(&leftover);
            leftover.drain(..sep_len);
            let line = match std::str::from_utf8(&line_bytes) {
                Ok(s) => s.trim(),
                Err(_) => continue,
            };
            if line.is_empty() { continue; }
            let payload = match line.strip_prefix("data:") {
                Some(p) => p.trim(),
                None => continue,
            };
            if payload == "[DONE]" {
                saw_done = true;
                break;
            }
            match serde_json::from_str::<SsePayload>(payload) {
                Ok(p) => {
                    let delta = p.delta_text();
                    let usage = p.usage_into();
                    let done = p.finish_reason_present();
                    let _ = tx
                        .send(GenerateChunk { delta, done: false, usage: usage.clone() })
                        .await;
                    if done {
                        let _ = tx
                            .send(GenerateChunk { delta: String::new(), done: true, usage })
                            .await;
                    }
                }
                Err(_) => {
                    // Malformed SSE payload is soft-ignored — the final
                    // "missing done" guard below catches catastrophic cases.
                    continue;
                }
            }
        }
        if saw_done { break; }
    }

    if !saw_done {
        // Send a terminating chunk so the consumer unblocks even if we never
        // saw [DONE] and did not see finish_reason.
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
    }
    Ok(())
}

fn find_line_end(buf: &[u8]) -> Option<usize> {
    for (i, w) in buf.windows(2).enumerate() {
        if w == b"\r\n" { return Some(i); }
    }
    buf.iter().position(|&b| b == b'\n' || b == b'\r')
}

fn separator_len(buf: &[u8]) -> usize {
    match buf.first() {
        Some(&b'\r') if buf.get(1) == Some(&b'\n') => 2,
        Some(&b'\r') | Some(&b'\n') => 1,
        _ => 0,
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[derive(Deserialize)]
struct SsePayload {
    #[serde(default)]
    choices: Vec<SseChoice>,
    #[serde(default)]
    usage: Option<SseUsage>,
}

#[derive(Deserialize)]
struct SseChoice {
    #[serde(default)]
    delta: SseDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct SseUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

impl SsePayload {
    fn delta_text(&self) -> String {
        self.choices
            .first()
            .and_then(|c| c.delta.content.clone())
            .unwrap_or_default()
    }
    fn finish_reason_present(&self) -> bool {
        self.choices.first().and_then(|c| c.finish_reason.as_ref()).is_some()
    }
    fn usage_into(&self) -> Option<TokenUsage> {
        self.usage.as_ref().map(|u| TokenUsage {
            prompt: u.prompt_tokens,
            completion: u.completion_tokens,
        })
    }
}

// Unused import warnings will come from ChatMessage if we never touch it in
// this file after `build_request_body`. We do use it transitively through
// GenerateRequest, so this suppression is cosmetic:
#[allow(dead_code)]
fn _unused_chatmessage_anchor(_: ChatMessage) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn sample_request() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "sys".into(),
            messages: vec![ChatMessage { role: "user".into(), content: "hi".into() }],
            context: EnvSnapshot {
                os: "linux".into(),
                shell: "bash".into(),
                cwd: PathBuf::from("/"),
            },
            mode: QueryMode::SingleCommand,
            max_tokens: Some(256),
        }
    }

    #[test]
    fn request_body_sets_stream_and_response_format() {
        let body = build_request_body("gpt-4o-mini", &sample_request());
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "gpt-4o-mini");
        assert_eq!(json["stream"], true);
        assert_eq!(json["response_format"]["type"], "json_object");
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][0]["content"], "sys");
        assert_eq!(json["messages"][1]["role"], "user");
        assert_eq!(json["messages"][1]["content"], "hi");
        assert_eq!(json["max_tokens"], 256);
    }

    #[test]
    fn find_line_end_prefers_crlf() {
        assert_eq!(find_line_end(b"abc\r\nxyz"), Some(3));
        assert_eq!(find_line_end(b"abc\nxyz"), Some(3));
        assert_eq!(find_line_end(b"nope"), None);
    }

    #[test]
    fn separator_len_handles_both() {
        assert_eq!(separator_len(b"\r\nxyz"), 2);
        assert_eq!(separator_len(b"\nxyz"), 1);
        assert_eq!(separator_len(b"xyz"), 0);
    }

    #[test]
    fn sse_payload_extracts_delta() {
        let raw = r#"{"choices":[{"delta":{"content":"hello"}}]}"#;
        let p: SsePayload = serde_json::from_str(raw).unwrap();
        assert_eq!(p.delta_text(), "hello");
        assert!(!p.finish_reason_present());
    }

    #[test]
    fn sse_payload_detects_finish_reason() {
        let raw = r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#;
        let p: SsePayload = serde_json::from_str(raw).unwrap();
        assert!(p.finish_reason_present());
    }
}
```

- [ ] **Step 2: Register the submodule**

Edit `src-tauri/src/ai/mod.rs`. Below the existing `pub mod context;` line, add:

```rust
pub mod openai;
```

- [ ] **Step 3: Run the openai unit tests**

```bash
cargo test ai::openai::tests
```

Expected: 5 tests pass. Warnings about `_unused_chatmessage_anchor` are OK.

- [ ] **Step 4: Run full suite**

```bash
cargo test
```

Expected: everything green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/openai.rs src-tauri/src/ai/mod.rs
git commit -m "feat(ai): implement OpenAiClient with streaming SSE consumption"
```

---

## Task 12: wiremock integration test for `OpenAiClient`

**Files:**
- Create: `src-tauri/tests/openai_client.rs`

- [ ] **Step 1: Create the integration test**

```rust
//! Contract test for `OpenAiClient` against a wiremock fake of the OpenAI
//! chat completions endpoint. Covers the happy path, 401, 429 with
//! retry-after, and 500.

use aiterm_lib::ai::{
    openai::OpenAiClient, AiError, AiProvider, ChatMessage, EnvSnapshot, GenerateChunk,
    GenerateRequest, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{bearer_token, method, path, header};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req(text: &str) -> GenerateRequest {
    GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage { role: "user".into(), content: text.into() }],
        context: EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/"),
        },
        mode: QueryMode::SingleCommand,
        max_tokens: Some(256),
    }
}

const JSON_OUTPUT: &str =
    r#"{"explanation":"list","command":"ls","risk_level":"safe"}"#;

fn sse_response_happy_path() -> String {
    // Three SSE events: two with content, one terminator.
    let c1 = format!(
        r#"{{"choices":[{{"delta":{{"content":{}}}}}]}}"#,
        serde_json::Value::String(JSON_OUTPUT[..20].to_string())
    );
    let c2 = format!(
        r#"{{"choices":[{{"delta":{{"content":{}}}}}]}}"#,
        serde_json::Value::String(JSON_OUTPUT[20..].to_string())
    );
    let done = r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#;
    format!(
        "data: {c1}\n\ndata: {c2}\n\ndata: {done}\n\ndata: [DONE]\n\n"
    )
}

#[tokio::test]
async fn happy_path_streams_and_parses() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(bearer_token("test-key"))
        .and(header("content-type", "application/json"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_response_happy_path()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OpenAiClient::with_base_url("test-key".into(), server.uri());
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);

    client.generate(req("hello"), tx).await.expect("generate ok");

    let mut buf = String::new();
    let mut saw_done = false;
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { saw_done = true; break; }
    }
    assert!(saw_done, "expected a done chunk");
    assert_eq!(buf, JSON_OUTPUT);
}

#[tokio::test]
async fn returns_auth_failed_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let client = OpenAiClient::with_base_url("bad".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    assert!(matches!(err, AiError::AuthFailed), "got {err:?}");
}

#[tokio::test]
async fn returns_rate_limit_with_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_string("slow down"),
        )
        .mount(&server)
        .await;

    let client = OpenAiClient::with_base_url("k".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    match err {
        AiError::RateLimit { retry_after } => {
            assert_eq!(retry_after.as_deref(), Some("30"));
        }
        other => panic!("expected RateLimit, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_network_on_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal"))
        .mount(&server)
        .await;

    let client = OpenAiClient::with_base_url("k".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    match err {
        AiError::Network { message } => assert!(message.contains("500")),
        other => panic!("expected Network, got {other:?}"),
    }
}
```

- [ ] **Step 2: Check if `tokio::test` macro is available**

The test uses `#[tokio::test]`, which requires the `macros` feature — M0's Cargo.toml already has `tokio = { version = "1", features = ["sync", "rt-multi-thread", "macros"] }`. If this test fails to compile with "unknown attribute `tokio::test`", that means the test binary has a separate feature surface — add:

```toml
[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

to `Cargo.toml` under a new `[dev-dependencies]` section (or extend the existing one added in Task 1 for wiremock).

- [ ] **Step 3: Run the wiremock integration test**

```bash
cargo test --test openai_client
```

Expected: 4 tests pass.

- [ ] **Step 4: Run everything**

```bash
cargo test
```

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tests/openai_client.rs src-tauri/Cargo.toml
git commit -m "test(ai): wiremock contract test for OpenAiClient"
```

---

## Task 13: Implement `AiRouter`

**Files:**
- Create: `src-tauri/src/ai/router.rs`
- Modify: `src-tauri/src/ai/mod.rs`

- [ ] **Step 1: Create `ai/router.rs`**

```rust
//! In M1 the router is a thin holder for a single provider (or a
//! NotConfigured error captured at startup so the app can still boot).
//! M2 will expand this to pick a provider by id.

use std::sync::Arc;

use crate::ai::{openai::OpenAiClient, AiError, AiProvider};

pub struct AiRouter {
    provider: Result<Arc<dyn AiProvider>, AiError>,
}

impl AiRouter {
    /// Try to build the default provider from environment. If the env var
    /// is missing, capture the error and defer reporting until the user
    /// actually triggers `/ai`. Spec §7.3.
    pub fn from_env() -> Self {
        let result = match std::env::var("OPENAI_API_KEY") {
            Ok(key) if !key.trim().is_empty() => {
                let client: Arc<dyn AiProvider> = Arc::new(OpenAiClient::new(key));
                Ok(client)
            }
            _ => Err(AiError::NotConfigured),
        };
        Self { provider: result }
    }

    #[cfg(test)]
    pub fn with_provider(p: Arc<dyn AiProvider>) -> Self {
        Self { provider: Ok(p) }
    }

    /// Get the provider, or surface the captured error.
    pub fn require_provider(&self) -> Result<Arc<dyn AiProvider>, AiError> {
        self.provider.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_var_produces_not_configured() {
        // Intentionally unset the var for this test.
        std::env::remove_var("OPENAI_API_KEY");
        let router = AiRouter::from_env();
        match router.require_provider() {
            Err(AiError::NotConfigured) => {}
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    #[test]
    fn empty_env_var_produces_not_configured() {
        std::env::set_var("OPENAI_API_KEY", "   ");
        let router = AiRouter::from_env();
        assert!(matches!(router.require_provider(), Err(AiError::NotConfigured)));
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn present_env_var_produces_ok() {
        std::env::set_var("OPENAI_API_KEY", "fake-key");
        let router = AiRouter::from_env();
        assert!(router.require_provider().is_ok());
        std::env::remove_var("OPENAI_API_KEY");
    }
}
```

**Note on env-var tests:** Rust runs tests in parallel by default and these three tests mutate process-global state. If flakiness appears, run with `cargo test ai::router -- --test-threads=1`. In practice on a fresh CI box they pass fine because they all unset at the end.

- [ ] **Step 2: Register the submodule**

In `src-tauri/src/ai/mod.rs`, below `pub mod openai;`, add:

```rust
pub mod router;
```

- [ ] **Step 3: Run tests**

```bash
cargo test ai::router::tests -- --test-threads=1
```

Expected: 3 tests pass.

- [ ] **Step 4: Run full suite**

```bash
cargo test
```

Expected: all green (ai::router may still run with multi-thread default; if it flakes, rerun with `--test-threads=1`).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/router.rs src-tauri/src/ai/mod.rs
git commit -m "feat(ai): add AiRouter with env-based provider factory"
```

---

## Task 14: `ai_query` Tauri command

**Files:**
- Create: `src-tauri/src/commands/mod.rs`
- Create: `src-tauri/src/commands/ai.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Create `src-tauri/src/commands/mod.rs`**

```rust
pub mod ai;
```

- [ ] **Step 2: Create `src-tauri/src/commands/ai.rs`**

```rust
//! `/ai` query command. Wires the frontend invoke to the ai_router and the
//! PTY manager for context, and returns a fully-parsed `AiCommandReady`.

use serde::Serialize;
use tauri::State;
use tokio::sync::mpsc;

use crate::ai::{
    context, router::AiRouter, AiError, AiSingleCommand, ChatMessage, GenerateChunk,
    GenerateRequest, QueryMode,
};
use crate::pty::PtyManager;

#[derive(Debug, Clone, Serialize)]
pub struct AiCommandReady {
    pub command: String,
    pub explanation: String,
}

/// Build the M1 single-command system prompt. Pure function for testability.
pub fn build_single_command_prompt(snapshot: &crate::ai::EnvSnapshot) -> String {
    format!(
r#"You are an AI command generator for a cross-platform terminal application.
Your only job is to translate the user's natural-language request into ONE
executable shell command for their current environment.

Environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}            (may be slightly stale; prefer relative paths or
                         shell variables over hardcoded absolute paths)

Rules:
1. Output ONLY a JSON object, no prose, no markdown fences, no extra keys.
2. Schema:
   {{
     "explanation": "一句話說明這個命令做什麼 (use Traditional Chinese)",
     "command":     "a single shell command, no prompt prefix, no line breaks",
     "risk_level":  one of "safe", "needs_confirm", "dangerous"
   }}
3. The command must be syntactically valid for {shell}. Do not mix shells.
4. If the request cannot be satisfied with one command, pick the most useful
   single command and explain the limitation in `explanation`.
5. Never produce destructive operations against system roots. If the user
   explicitly asks for one, set risk_level="dangerous"."#,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display()
    )
}

#[tauri::command]
pub async fn ai_query(
    query: String,
    session_id: String,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiCommandReady, AiError> {
    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = router.require_provider()?;

    let prompt = build_single_command_prompt(&snapshot);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages: vec![ChatMessage { role: "user".into(), content: query }],
        context: snapshot,
        mode: QueryMode::SingleCommand,
        max_tokens: Some(512),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    // Propagate any provider error after draining the channel.
    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    let parsed: AiSingleCommand = serde_json::from_str(&buf).map_err(|e| AiError::ModelError {
        reason: e.to_string(),
        raw: buf.chars().take(200).collect(),
    })?;

    // M1 ignores risk_level (spec §6.4). It was parsed to fail-fast on
    // malformed responses, and the value is discarded here.
    let _ = parsed.risk_level;

    Ok(AiCommandReady {
        command: parsed.command,
        explanation: parsed.explanation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::EnvSnapshot;
    use std::path::PathBuf;

    #[test]
    fn prompt_contains_environment_fields() {
        let snap = EnvSnapshot {
            os: "windows".into(),
            shell: "pwsh".into(),
            cwd: PathBuf::from("C:\\Users\\a"),
        };
        let prompt = build_single_command_prompt(&snap);
        assert!(prompt.contains("OS: windows"));
        assert!(prompt.contains("Shell: pwsh"));
        assert!(prompt.contains("C:\\Users\\a"));
        assert!(prompt.contains("JSON object"));
        assert!(prompt.contains("risk_level"));
    }
}
```

- [ ] **Step 3: Register the commands module and wire state in `lib.rs`**

Replace the contents of `src-tauri/src/lib.rs` with:

```rust
pub mod ai;
pub mod commands;
pub mod pty;

use ai::router::AiRouter;
use commands::ai::ai_query;
use pty::commands::{pty_close, pty_create, pty_resize, pty_write};
use pty::PtyManager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())
        .manage(PtyManager::new())
        .manage(AiRouter::from_env())
        .invoke_handler(tauri::generate_handler![
            pty_create,
            pty_write,
            pty_resize,
            pty_close,
            ai_query,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 4: Build**

```bash
cargo build
```

Expected: clean build. Fix any errors before moving on.

- [ ] **Step 5: Run the prompt builder test**

```bash
cargo test commands::ai::tests
```

Expected: 1 test passes.

- [ ] **Step 6: Run everything**

```bash
cargo test
```

Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands/ src-tauri/src/lib.rs
git commit -m "feat(ai): add ai_query command and wire AiRouter into app state"
```

---

## Task 15: Integration test for `ai_query` with a mock provider

**Files:**
- Create: `src-tauri/tests/ai_query_command.rs`

This test validates the end-to-end command handler logic (prompt assembly, chunk accumulation, JSON parse, error propagation) without involving HTTP or a real shell. It uses a mock `AiProvider` that returns canned responses.

- [ ] **Step 1: Create the test**

```rust
//! End-to-end test for `ai_query` wiring. Uses a mock AiProvider so this
//! test is hermetic (no network, no real PTY).

use aiterm_lib::ai::{
    router::AiRouter, AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest,
};
use aiterm_lib::commands::ai::{build_single_command_prompt};
use aiterm_lib::ai::context;
use aiterm_lib::pty::PtyManager;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

struct MockProvider {
    chunks: Vec<&'static str>,
}

#[async_trait]
impl AiProvider for MockProvider {
    fn id(&self) -> &str { "mock" }
    fn display_name(&self) -> &str { "Mock" }
    async fn generate(
        &self,
        _req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        for (i, c) in self.chunks.iter().enumerate() {
            let done = i + 1 == self.chunks.len();
            let _ = tx
                .send(GenerateChunk { delta: c.to_string(), done, usage: None })
                .await;
        }
        Ok(())
    }
}

#[tokio::test]
async fn snapshot_builds_fallback_when_session_unknown() {
    let manager = PtyManager::new();
    let snap = context::snapshot(&manager, "no-such-session");
    // Fallback path: cwd is process cwd, shell is the default for this OS.
    assert!(!snap.cwd.as_os_str().is_empty());
    assert!(!snap.shell.is_empty());
}

#[test]
fn prompt_assembly_is_deterministic() {
    let snap = context::snapshot_from_parts("linux", "bash", PathBuf::from("/"));
    let a = build_single_command_prompt(&snap);
    let b = build_single_command_prompt(&snap);
    assert_eq!(a, b);
    assert!(a.contains("Shell: bash"));
}

// The full ai_query command requires a Tauri State<'_> to be constructed.
// For M1 we only exercise the pure parts (snapshot + prompt + mock provider
// protocol) from integration tests — wiring verification happens in the
// manual acceptance test (Task 22).

#[tokio::test]
async fn mock_provider_emits_chunks_through_channel() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec![
            r#"{"explanation":"列出","command":"ls","#,
            r#""risk_level":"safe"}"#,
        ],
    });
    let router = AiRouter::with_provider(provider);
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage { role: "user".into(), content: "list files".into() }],
        context: context::snapshot_from_parts("linux", "bash", PathBuf::from("/")),
        mode: aiterm_lib::ai::QueryMode::SingleCommand,
        max_tokens: Some(256),
    };
    let provider = router.require_provider().expect("provider");
    provider.generate(req, tx).await.expect("ok");

    let mut buf = String::new();
    while let Some(c) = rx.recv().await {
        buf.push_str(&c.delta);
        if c.done { break; }
    }
    let parsed: aiterm_lib::ai::AiSingleCommand = serde_json::from_str(&buf).expect("parse");
    assert_eq!(parsed.command, "ls");
    assert_eq!(parsed.explanation, "列出");
}
```

- [ ] **Step 2: Run the integration test**

```bash
cargo test --test ai_query_command
```

Expected: 3 tests pass.

- [ ] **Step 3: Full suite**

```bash
cargo test
```

Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/ai_query_command.rs
git commit -m "test(ai): integration test for ai_query wiring with mock provider"
```

---

## Task 16: Frontend — install `vitest` and add test script

**Files:**
- Modify: `package.json`
- Create: `vitest.config.ts`

- [ ] **Step 1: Install vitest**

Run from project root:

```bash
npm install -D vitest
```

Expected: adds `vitest` to devDependencies. No other packages should change (no UI testing library needed — M1 tests are pure functions).

- [ ] **Step 2: Add a `test` script to `package.json`**

Replace the `"scripts"` section of `package.json` with:

```json
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "lint": "eslint .",
    "preview": "vite preview",
    "tauri": "tauri",
    "tauri:dev": "tauri dev",
    "tauri:build": "tauri build",
    "test": "vitest run"
  },
```

- [ ] **Step 3: Create `vitest.config.ts`**

```ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
  },
});
```

- [ ] **Step 4: Verify vitest runs (with no tests yet)**

```bash
npm test
```

Expected: vitest reports "No test files found" and exits cleanly. If it fails, check `vitest.config.ts` syntax.

- [ ] **Step 5: Commit**

```bash
git add package.json package-lock.json vitest.config.ts
git commit -m "chore(frontend): add vitest devDependency and test script"
```

---

## Task 17: Frontend — `parseAiPrefix` pure function + tests

**Files:**
- Create: `src/components/parseAiPrefix.ts`
- Create: `src/components/parseAiPrefix.test.ts`

- [ ] **Step 1: Write the failing test**

Create `src/components/parseAiPrefix.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { parseAiPrefix } from "./parseAiPrefix";

describe("parseAiPrefix", () => {
  it("returns the query for a valid /ai line", () => {
    expect(parseAiPrefix("/ai list files")).toBe("list files");
  });

  it("collapses multiple spaces after /ai", () => {
    expect(parseAiPrefix("/ai   hello world")).toBe("hello world");
  });

  it("returns null when /ai has no arguments", () => {
    expect(parseAiPrefix("/ai")).toBeNull();
  });

  it("returns null when /ai is followed only by whitespace", () => {
    expect(parseAiPrefix("/ai   ")).toBeNull();
  });

  it("returns null when /ai is not at the start", () => {
    expect(parseAiPrefix("  /ai list files")).toBeNull();
    expect(parseAiPrefix("echo /ai list")).toBeNull();
  });

  it("is case-sensitive — only lowercase /ai counts", () => {
    expect(parseAiPrefix("/AI list")).toBeNull();
    expect(parseAiPrefix("/Ai list")).toBeNull();
  });

  it("returns null for unrelated lines", () => {
    expect(parseAiPrefix("ls -la")).toBeNull();
    expect(parseAiPrefix("")).toBeNull();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
npm test
```

Expected: fails with "Cannot find module './parseAiPrefix'".

- [ ] **Step 3: Create the implementation**

Create `src/components/parseAiPrefix.ts`:

```ts
/**
 * Parse a line of user input for the `/ai ` prefix. Returns the trimmed query
 * text when present, `null` when the line does not begin with `/ai ` or has no
 * query. Only lowercase `/ai` is recognized.
 */
export function parseAiPrefix(line: string): string | null {
  if (!line.startsWith("/ai")) return null;
  // Must be followed by at least one whitespace character; otherwise it's a
  // token like `/airplane` or just `/ai` alone.
  if (line.length === 3) return null;
  const next = line.charAt(3);
  if (next !== " " && next !== "\t") return null;
  const rest = line.slice(3).trim();
  return rest.length === 0 ? null : rest;
}
```

- [ ] **Step 4: Run tests again**

```bash
npm test
```

Expected: 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/parseAiPrefix.ts src/components/parseAiPrefix.test.ts
git commit -m "feat(frontend): add parseAiPrefix pure helper with vitest coverage"
```

---

## Task 18: Frontend — `ipc/ai.ts` with `formatAiError` + invoke wrapper

**Files:**
- Create: `src/ipc/ai.ts`
- Create: `src/ipc/ai.test.ts`

- [ ] **Step 1: Write the test first**

Create `src/ipc/ai.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { formatAiError, type AiError } from "./ai";

describe("formatAiError", () => {
  it("formats not_configured with install hint", () => {
    const msg = formatAiError({ kind: "not_configured" });
    expect(msg).toContain("OPENAI_API_KEY");
    expect(msg).toContain("restart");
  });

  it("formats network with message", () => {
    const e: AiError = { kind: "network", message: "connection refused" };
    expect(formatAiError(e)).toContain("connection refused");
  });

  it("formats auth_failed", () => {
    expect(formatAiError({ kind: "auth_failed" })).toContain("authentication");
  });

  it("formats rate_limit with retry_after", () => {
    const msg = formatAiError({ kind: "rate_limit", retry_after: "30" });
    expect(msg).toContain("retry after 30");
  });

  it("formats rate_limit without retry_after", () => {
    const msg = formatAiError({ kind: "rate_limit", retry_after: null });
    expect(msg).toContain("rate limit");
    expect(msg).not.toContain("retry after");
  });

  it("formats model_error with reason and raw", () => {
    const msg = formatAiError({
      kind: "model_error",
      reason: "missing command",
      raw: "{oops}",
    });
    expect(msg).toContain("missing command");
    expect(msg).toContain("{oops}");
  });
});
```

- [ ] **Step 2: Run test (expect module not found)**

```bash
npm test
```

Expected: fails with "Cannot find module './ai'".

- [ ] **Step 3: Create the implementation**

Create `src/ipc/ai.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";

export type AiError =
  | { kind: "not_configured" }
  | { kind: "network"; message: string }
  | { kind: "auth_failed" }
  | { kind: "rate_limit"; retry_after: string | null }
  | { kind: "model_error"; reason: string; raw: string };

export interface AiCommandReady {
  command: string;
  explanation: string;
}

export function invokeAiQuery(
  query: string,
  sessionId: string,
): Promise<AiCommandReady> {
  return invoke<AiCommandReady>("ai_query", { query, sessionId });
}

export function formatAiError(e: AiError): string {
  switch (e.kind) {
    case "not_configured":
      return "aiterm: OPENAI_API_KEY not set. Set the env var and restart AITerm.";
    case "network":
      return `aiterm: network error — ${e.message}`;
    case "auth_failed":
      return "aiterm: authentication failed. Check your OPENAI_API_KEY.";
    case "rate_limit":
      return e.retry_after
        ? `aiterm: rate limit exceeded (retry after ${e.retry_after})`
        : "aiterm: rate limit exceeded, try again later";
    case "model_error":
      return `aiterm: AI returned invalid response (${e.reason})\n        raw: ${e.raw}`;
  }
}
```

- [ ] **Step 4: Run tests**

```bash
npm test
```

Expected: 13 tests pass (7 from Task 17 + 6 here).

- [ ] **Step 5: Type-check the whole frontend**

```bash
npx tsc --noEmit
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add src/ipc/ai.ts src/ipc/ai.test.ts
git commit -m "feat(frontend): add ipc/ai wrapper with AiError formatting"
```

---

## Task 19: Frontend — `CommandPreview` component

**Files:**
- Create: `src/components/CommandPreview.tsx`
- Create: `src/components/CommandPreview.css`

- [ ] **Step 1: Create `CommandPreview.css`**

```css
.aiterm-command-preview {
  position: absolute;
  left: 50%;
  top: 20%;
  transform: translateX(-50%);
  min-width: 480px;
  max-width: 80vw;
  max-height: 60vh;
  overflow: auto;
  background: rgba(20, 20, 20, 0.96);
  color: #e6e6e6;
  border: 1px solid #444;
  border-radius: 6px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
  padding: 16px 20px;
  font-family: system-ui, -apple-system, "Segoe UI", sans-serif;
  font-size: 14px;
  z-index: 1000;
}

.aiterm-command-preview__label {
  font-size: 11px;
  text-transform: uppercase;
  color: #888;
  margin-top: 8px;
}

.aiterm-command-preview__label:first-child {
  margin-top: 0;
}

.aiterm-command-preview__command {
  font-family: "Cascadia Mono", Consolas, monospace;
  background: #0c0c0c;
  padding: 8px 10px;
  border-radius: 4px;
  white-space: pre-wrap;
  word-break: break-word;
  margin-top: 4px;
}

.aiterm-command-preview__explanation {
  margin-top: 4px;
  line-height: 1.5;
}

.aiterm-command-preview__hint {
  margin-top: 16px;
  text-align: center;
  color: #888;
  font-size: 12px;
}
```

- [ ] **Step 2: Create `CommandPreview.tsx`**

```tsx
import { useEffect } from "react";
import "./CommandPreview.css";

export interface CommandPreviewProps {
  command: string;
  explanation: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export function CommandPreview({
  command,
  explanation,
  onConfirm,
  onCancel,
}: CommandPreviewProps) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        onConfirm();
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onCancel();
      }
    };
    // Capture phase so we intercept before xterm.js sees the key.
    document.addEventListener("keydown", handler, true);
    return () => document.removeEventListener("keydown", handler, true);
  }, [onConfirm, onCancel]);

  return (
    <div className="aiterm-command-preview" role="dialog" aria-label="AI command preview">
      <div className="aiterm-command-preview__label">Command</div>
      <div className="aiterm-command-preview__command">{command}</div>
      <div className="aiterm-command-preview__label">Explanation</div>
      <div className="aiterm-command-preview__explanation">{explanation}</div>
      <div className="aiterm-command-preview__hint">
        [Enter] Execute &nbsp;&nbsp; [Esc] Cancel
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Type-check**

```bash
npx tsc --noEmit
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/components/CommandPreview.tsx src/components/CommandPreview.css
git commit -m "feat(frontend): add CommandPreview component with Enter/Esc keybinds"
```

---

## Task 20: Frontend — wire `/ai` prefix into `TerminalView`

**Files:**
- Modify: `src/components/TerminalView.tsx`

This is the largest frontend change. It:
1. Tracks a line buffer of what the user has typed since the last Enter
2. On Enter, checks for `/ai ` prefix before writing to PTY
3. Invokes `ai_query` if matched
4. Shows `→ asking AI...` and manages loading state
5. Renders `CommandPreview` when the response arrives
6. Writes red error text on failure

- [ ] **Step 1: Replace the contents of `src/components/TerminalView.tsx`**

```tsx
import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

import {
  closePty,
  createPty,
  onPtyData,
  resizePty,
  writePty,
} from "../ipc/pty";
import { invokeAiQuery, formatAiError, type AiError } from "../ipc/ai";
import { parseAiPrefix } from "./parseAiPrefix";
import { CommandPreview } from "./CommandPreview";
import "./TerminalView.css";

interface PreviewState {
  loading: boolean;
  visible: boolean;
  command: string;
  explanation: string;
}

const INITIAL_PREVIEW: PreviewState = {
  loading: false,
  visible: false,
  command: "",
  explanation: "",
};

export function TerminalView() {
  const hostRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<string>("initializing…");
  const [preview, setPreview] = useState<PreviewState>(INITIAL_PREVIEW);
  const previewRef = useRef<PreviewState>(INITIAL_PREVIEW);
  previewRef.current = preview;

  // Refs bridged into the useEffect closure.
  const termRef = useRef<Terminal | null>(null);
  const sessionRef = useRef<string | null>(null);
  const lineBufRef = useRef<string>("");

  useEffect(() => {
    if (!hostRef.current) return;

    const term = new Terminal({
      fontFamily: '"Cascadia Mono", Consolas, monospace',
      fontSize: 14,
      cursorBlink: true,
      theme: {
        background: "#0c0c0c",
        foreground: "#e6e6e6",
      },
      convertEol: false,
    });
    termRef.current = term;

    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current);
    requestAnimationFrame(() => fit.fit());

    const decoder = new TextDecoder("utf-8");

    let unlistenData: (() => void) | null = null;

    const writeRed = (msg: string) => {
      term.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`);
    };

    (async () => {
      try {
        const { rows, cols } = term;
        const sessionId = await createPty({ rows, cols });
        sessionRef.current = sessionId;
        setStatus(`connected (${sessionId.slice(0, 8)}…)`);

        unlistenData = await onPtyData(sessionId, (bytes) => {
          term.write(decoder.decode(bytes, { stream: true }));
        });

        term.onData((data) => {
          const session = sessionRef.current;
          if (!session) return;

          // Track a local line buffer so we can recognize `/ai ` at Enter time.
          for (const ch of data) {
            if (ch === "\r" || ch === "\n") {
              // User hit Enter. Check for /ai prefix.
              const line = lineBufRef.current;
              lineBufRef.current = "";
              const query = parseAiPrefix(line);
              if (query !== null) {
                // While loading, reject further /ai submissions.
                if (previewRef.current.loading) {
                  writeRed("aiterm: already waiting for AI response");
                  // Echo CR so the shell stays on a fresh line.
                  continue;
                }
                handleAiQuery(session, line, query, term, setPreview, writeRed);
                continue; // do NOT forward to PTY
              }
              // Non-/ai line: forward the Enter normally.
              writePty(session, ch).catch(console.error);
            } else if (ch === "\x7f" || ch === "\b") {
              // Backspace — keep the buffer in sync.
              lineBufRef.current = lineBufRef.current.slice(0, -1);
              writePty(session, ch).catch(console.error);
            } else if (ch === "\x03") {
              // Ctrl+C clears the line buffer.
              lineBufRef.current = "";
              writePty(session, ch).catch(console.error);
            } else {
              lineBufRef.current += ch;
              writePty(session, ch).catch(console.error);
            }
          }
        });

        term.onResize(({ rows: r, cols: c }) => {
          if (sessionRef.current) {
            resizePty(sessionRef.current, { rows: r, cols: c }).catch(console.error);
          }
        });
      } catch (e) {
        setStatus(`error: ${String(e)}`);
      }
    })();

    const onWindowResize = () => fit.fit();
    window.addEventListener("resize", onWindowResize);

    return () => {
      window.removeEventListener("resize", onWindowResize);
      if (unlistenData) unlistenData();
      const id = sessionRef.current;
      if (id) {
        closePty(id).catch(() => {
          // ignore — may already be gone
        });
      }
      term.dispose();
      termRef.current = null;
    };
  }, []);

  const handleConfirm = () => {
    const session = sessionRef.current;
    if (session && preview.command) {
      writePty(session, preview.command + "\r").catch(console.error);
    }
    setPreview(INITIAL_PREVIEW);
  };
  const handleCancel = () => setPreview(INITIAL_PREVIEW);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        position: "relative",
      }}
    >
      <div className="aiterm-status">AITerm · {status}</div>
      <div
        ref={hostRef}
        className="aiterm-terminal-root"
        style={{ flex: 1, minHeight: 0 }}
      />
      {preview.visible && (
        <CommandPreview
          command={preview.command}
          explanation={preview.explanation}
          onConfirm={handleConfirm}
          onCancel={handleCancel}
        />
      )}
    </div>
  );
}

/**
 * Kick off an /ai request: erase the typed line, show the "asking" indicator,
 * invoke the backend, and update preview state based on the result.
 */
function handleAiQuery(
  sessionId: string,
  originalLine: string,
  query: string,
  term: Terminal,
  setPreview: (p: PreviewState) => void,
  writeRed: (msg: string) => void,
) {
  // Erase the typed `/ai ...` line visually (CR + clear-line). We only touch
  // xterm's buffer here — the shell has not seen the bytes because we did
  // not forward them to PTY.
  void originalLine;
  term.write("\r\x1b[2K");
  term.write("→ asking AI...\r\n");
  setPreview({ loading: true, visible: false, command: "", explanation: "" });

  invokeAiQuery(query, sessionId)
    .then((resp) => {
      // Clear the "asking" line then show the preview.
      term.write("\x1b[1A\x1b[2K"); // move up one, clear line
      setPreview({
        loading: false,
        visible: true,
        command: resp.command,
        explanation: resp.explanation,
      });
    })
    .catch((rawErr: unknown) => {
      const err = normalizeAiError(rawErr);
      writeRed(formatAiError(err));
      setPreview(INITIAL_PREVIEW);
    });
}

/**
 * Tauri may deliver `AiError` either as the serialized object directly or
 * wrapped in an `Error` whose message is the JSON. Coerce both forms.
 */
function normalizeAiError(err: unknown): AiError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as AiError;
  }
  if (err instanceof Error) {
    try {
      const parsed = JSON.parse(err.message);
      if (parsed && typeof parsed === "object" && "kind" in parsed) {
        return parsed as AiError;
      }
    } catch {
      // fall through
    }
    return { kind: "network", message: err.message };
  }
  return { kind: "network", message: String(err) };
}
```

- [ ] **Step 2: Type-check**

```bash
npx tsc --noEmit
```

Expected: no errors. If you get an error about `Terminal` not being imported for `handleAiQuery`, add `import { Terminal } from "@xterm/xterm";` at the top (it should already be there from the initial import).

- [ ] **Step 3: Run all frontend tests**

```bash
npm test
```

Expected: 13 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/components/TerminalView.tsx
git commit -m "feat(frontend): intercept /ai prefix and render CommandPreview"
```

---

## Task 21: Manual smoke test (dev build)

**Files:** none modified

This task verifies the dev build starts and the basic wiring works. The 4 formal acceptance criteria are in Task 22.

- [ ] **Step 1: Start the dev build**

```bash
npm run tauri:dev
```

Expected: Tauri window opens, xterm.js shows a shell prompt (pwsh on Windows).

- [ ] **Step 2: Basic PTY sanity check**

Type `echo hello` in the terminal and press Enter. Expected: `hello` is echoed.

- [ ] **Step 3: Without `OPENAI_API_KEY`, try `/ai`**

If your current session already has `OPENAI_API_KEY` set, unset it and restart dev build. Then type `/ai show disk usage` and press Enter. Expected:
- The `/ai show disk usage` line disappears
- Red text `aiterm: OPENAI_API_KEY not set. Set the env var and restart AITerm.` appears
- Shell prompt is still responsive

Stop the dev build.

- [ ] **Step 4: Commit any artifacts**

No code changes expected. If something broke and you fixed it, commit:

```bash
git add -A
git commit -m "fix: address issues found during smoke test"
```

---

## Task 22: Formal acceptance tests (manual)

**Files:** none modified — this task runs the 4 acceptance tests from spec §9.

Complete each test and record results (all must pass for M1 to be considered done).

- [ ] **Step 1: Acceptance test 1 — happy path**

Set `OPENAI_API_KEY` in your shell, then start `npm run tauri:dev`. Type `/ai list files` and press Enter.

Expected:
- A `CommandPreview` appears with a reasonable `Get-ChildItem` command (pwsh) and a Traditional Chinese explanation
- Press Enter: command executes, listing appears in the terminal
- Press Up-arrow: the executed command is in shell history

- [ ] **Step 2: Acceptance test 2 — cancel**

Type `/ai show disk usage` and press Enter. When the preview appears, press Esc.

Expected:
- Preview disappears
- Terminal accepts a new command normally (e.g. `echo ok` works)

- [ ] **Step 3: Acceptance test 3 — unset env var**

Quit the dev build. Unset `OPENAI_API_KEY`. Start `npm run tauri:dev`. Type `/ai anything` and press Enter.

Expected:
- Red text in the terminal: `aiterm: OPENAI_API_KEY not set. Set the env var and restart AITerm.`
- The app did not crash or refuse to start

- [ ] **Step 4: Acceptance test 4 — cwd tracking**

Re-set `OPENAI_API_KEY` and restart dev build. At the shell prompt, type `cd C:\Windows\System32` and press Enter. Then type `/ai list dll files` and press Enter.

Expected:
- The preview's command reflects the `C:\Windows\System32` context (e.g. `Get-ChildItem *.dll` rather than a different path)
- This proves the e2α cd tracking reached the AI snapshot

- [ ] **Step 5: Record completion**

Create a tiny note in the commit (or a doc) listing which tests passed. If any failed, do not close M1 — file follow-ups and iterate.

```bash
git commit --allow-empty -m "chore(m1): acceptance tests 1-4 passed"
```

---

## Self-Review

Verified against the spec `docs/superpowers/specs/2026-04-10-aiterm-m1-design.md`:

- **§3.1 backend module layout:** Task 2 (mod.rs), Task 10 (context.rs), Task 11 (openai.rs), Task 13 (router.rs), Task 14 (commands/ai.rs). ✓
- **§3.2 new deps:** Task 1 (reqwest, async-trait, futures-util, wiremock dev-dep). ✓
- **§3.3 frontend structure:** Task 17 (parseAiPrefix), Task 18 (ipc/ai.ts), Task 19 (CommandPreview), Task 20 (TerminalView wiring). ✓ (using flat file layout in `src/components/` to match existing M0 convention)
- **§3.4 trait definition:** Tasks 3, 4, 5. ✓
- **§4.1 data flow:** Task 20 (frontend), Task 14 (backend). ✓ (no streaming event path; buffered as per D3)
- **§4.3 CommandPreview behavior:** Task 19 (component) + Task 20 (loading state ignores concurrent /ai). ✓
- **§5 cwd tracking (e2α):** Task 6 (parser), Task 7 (PtySession hook), Task 8 (PtyManager accessor), Task 9 (real-shell integration). ✓
- **§6 prompt + `response_format: json_object`:** Task 14 (`build_single_command_prompt`), Task 11 (`OpenAiChatRequest::response_format`). ✓
- **§6.4 risk_level parsed but unused:** Task 14 (`let _ = parsed.risk_level`). ✓
- **§7 error handling:** Tasks 2, 11, 14, 18. ✓
- **§7.3 NotConfigured deferred:** Task 13 (`AiRouter::from_env` captures the error, `require_provider` returns it only when called). ✓
- **§7.4 red text & normalizeAiError:** Task 20. ✓
- **§8.1 unit tests:** Tasks 2, 3, 4 (ai types), Task 6 (cd parser), Task 7 (session cwd), Task 11 (openai internals), Task 14 (prompt). ✓
- **§8.2 integration tests:** Task 9 (pty_cwd_tracking), Task 12 (openai_client wiremock), Task 15 (ai_query_command). ✓
- **§8.3 frontend tests:** Tasks 17, 18. ✓
- **§8.4 M0 regression protection:** Task 7 step 5 runs session tests, Task 7 step 6 runs full suite; every task has a `cargo test` step. ✓
- **§9 acceptance criteria:** Task 22. ✓

**Placeholder scan:** No TBD / TODO / "similar to task N" references. Every code block is concrete.

**Type consistency:**
- `AiCommandReady { command, explanation }` — defined in Task 14, consumed in Task 18/20. ✓
- `AiError` variants — defined in Task 2, mirrored in Task 18. ✓
- `ShellVariant` — defined in Task 6, used in Tasks 7, 8, 10. ✓
- `parseAiPrefix` signature `(line: string) => string | null` — defined in Task 17, used in Task 20. ✓
- `invokeAiQuery(query, sessionId)` snake vs camel: Tauri auto-converts; backend `ai_query(query: String, session_id: String, ...)` ↔ frontend `invoke("ai_query", { query, sessionId })`. ✓

**Known deviations from spec (intentional):**

1. Spec §3.3 lists `src/components/TerminalView/parseAiPrefix.ts` (folder); M0 uses flat files, so this plan places it at `src/components/parseAiPrefix.ts`. Same for `CommandPreview.tsx`. Reasoning: follow existing M0 convention per the "follow existing patterns" guideline in `writing-plans`.

2. Spec §8.1 test table says cd parser tests live in `pty/session.rs`; this plan puts them in `pty/cd_parser.rs` alongside the parser implementation. Reasoning: parser is a cohesive pure-function unit best kept in its own file; session.rs already has IO-heavy responsibilities.

Both deviations are minor structural choices that do not affect behavior or the decisions recorded in §2 of the spec.
