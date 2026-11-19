//! 本機 `claude` CLI（Claude Code）提供者——只做文字生成。
//!
//! 設計見 `docs/superpowers/specs/2026-11-19-claude-cli-provider-design.md`。
//!
//! 為什麼有這個：Anthropic 訂閱的 OAuth 憑證帶 `tools` 直連會被記到額外用量；
//! 真的 `claude` 是 Anthropic 自己的客戶端，用訂閱沒有這個限制。這裡把它當成
//! 「訂閱版的文字生成」，工具呼叫仍走 AITerm 既有的提示注入路徑
//! （`generate_with_tools` 不覆寫，預設回 `Unsupported`）。
//!
//! 不偽裝：只呼叫使用者自己安裝、自己登入的真 CLI，不碰憑證、不模仿請求簽章。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{content, AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, TokenUsage};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(180);

pub struct ClaudeCliClient {
    model: String,
    /// 測試用：直接指定執行檔（假 claude）。`None` = 去找真的。
    program: Option<PathBuf>,
    timeout: Duration,
}

impl ClaudeCliClient {
    pub fn new(model: String) -> Self {
        Self { model, program: None, timeout: DEFAULT_TIMEOUT }
    }

    #[cfg(test)]
    fn with_program(model: &str, program: PathBuf, timeout: Duration) -> Self {
        Self { model: model.into(), program: Some(program), timeout }
    }

    fn resolve_program(&self) -> Result<PathBuf, AiError> {
        if let Some(p) = &self.program {
            return Ok(p.clone());
        }
        locate_claude().ok_or(AiError::NotConfigured)
    }
}

// ───────────────────────── 找執行檔 ─────────────────────────

/// GUI App 的 PATH 不含 Homebrew／npm／`~/.local/bin`，所以 PATH 之外再補幾個常見位置。
fn candidate_dirs(path_env: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_env.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    if let Some(h) = home {
        dirs.push(h.join(".local").join("bin"));
        dirs.push(h.join(".claude").join("local"));
        dirs.push(h.join(".npm-global").join("bin"));
        dirs.push(h.join(".bun").join("bin"));
    }
    if !cfg!(windows) {
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
    }
    dirs
}

fn executable_names() -> &'static [&'static str] {
    if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] }
}

pub(crate) fn find_in(dirs: &[PathBuf], is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    for d in dirs {
        for n in executable_names() {
            let p = d.join(n);
            if is_file(&p) {
                return Some(p);
            }
        }
    }
    None
}

pub fn locate_claude() -> Option<PathBuf> {
    let home = dirs::home_dir();
    let dirs = candidate_dirs(std::env::var_os("PATH").as_deref(), home.as_deref());
    find_in(&dirs, |p| p.is_file())
}

// ───────────────────────── 組命令 ─────────────────────────

/// `claude` 的引數。提示走 stdin、系統提示走檔案，都不進 argv
/// （多行中文在各平台命令列引號會出問題，Windows 命令列也有長度上限）。
///
/// 瘦身三個旗標不可省，實測（claude 2.1.328）首次 `cache_creation_input_tokens`
/// 12,497 → 636：沒有它們，使用者的 hooks／CLAUDE.md／skills 會全被帶進每次呼叫。
/// 不能用 `--bare`：它不吃訂閱 OAuth（回 Not logged in）。
pub(crate) fn build_args(model: &str, system_prompt_file: &Path) -> Vec<String> {
    let mut a: Vec<String> = vec!["-p".into()];
    if !model.trim().is_empty() {
        a.extend(["--model".into(), model.trim().into()]);
    }
    a.extend([
        "--tools".into(),
        String::new(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
        "--system-prompt-file".into(),
        system_prompt_file.to_string_lossy().into_owned(),
        "--setting-sources".into(),
        String::new(),
        "--strict-mcp-config".into(),
        "--disable-slash-commands".into(),
        "--no-session-persistence".into(),
    ]);
    a
}

/// 把對話攤平成純文字給 stdin。單則使用者訊息就原樣送；多輪才加角色標籤。
/// `role:"system"` 的訊息不在這裡（由 [`system_text`] 併進系統提示）。
pub(crate) fn flatten_messages(messages: &[ChatMessage]) -> String {
    let turns: Vec<(&str, String)> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let label = match m.role.as_str() {
                "assistant" => "Assistant",
                "tool" => "Tool result",
                _ => "User",
            };
            let text = match &m.content {
                Value::String(s) => s.clone(),
                other => content::text_only(other),
            };
            (label, text)
        })
        .collect();
    if turns.len() == 1 && turns[0].0 == "User" {
        return turns[0].1.clone();
    }
    let mut out = String::new();
    for (label, text) in &turns {
        out.push_str(&format!("{label}:\n{text}\n\n"));
    }
    out.push_str("Reply as the assistant to the last user message.");
    out
}

pub(crate) fn system_text(req: &GenerateRequest) -> String {
    let mut s = req.system_prompt.clone();
    for m in req.messages.iter().filter(|m| m.role == "system") {
        let t = match &m.content {
            Value::String(t) => t.clone(),
            other => content::text_only(other),
        };
        if !t.is_empty() {
            if !s.is_empty() {
                s.push_str("\n\n");
            }
            s.push_str(&t);
        }
    }
    s
}

// ───────────────────────── 解析串流 ─────────────────────────

#[derive(Debug, PartialEq)]
pub(crate) enum Parsed {
    Text(String),
    Done(TokenUsage),
    Failed(AiError2),
    Ignore,
}

/// `AiError` 沒有實作 `PartialEq`；測試只需要比對種類。
#[derive(Debug, PartialEq)]
pub(crate) enum AiError2 {
    Auth,
    RateLimit,
    Model(String),
}

impl From<AiError2> for AiError {
    fn from(e: AiError2) -> Self {
        match e {
            AiError2::Auth => AiError::AuthFailed,
            AiError2::RateLimit => AiError::RateLimit { retry_after: None, body: None },
            AiError2::Model(reason) => AiError::ModelError { raw: reason.clone(), reason },
        }
    }
}

fn classify_failure(message: &str) -> AiError2 {
    let lower = message.to_lowercase();
    if lower.contains("not logged in") || lower.contains("/login") || lower.contains("invalid api key") {
        AiError2::Auth
    } else if lower.contains("rate limit") || lower.contains("usage limit") || lower.contains("limit reached") {
        AiError2::RateLimit
    } else {
        AiError2::Model(message.to_string())
    }
}

fn u32_of(v: &Value, key: &str) -> u32 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0).min(u32::MAX as u64) as u32
}

/// 解一行 stream-json。未知事件一律略過（CLI 升級會新增事件類型，不能因此報錯）。
pub(crate) fn parse_line(line: &str) -> Parsed {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
        return Parsed::Ignore;
    };
    match v.get("type").and_then(Value::as_str) {
        Some("stream_event") => {
            let ev = &v["event"];
            if ev["type"] == "content_block_delta" && ev["delta"]["type"] == "text_delta" {
                if let Some(t) = ev["delta"]["text"].as_str() {
                    return Parsed::Text(t.to_string());
                }
            }
            Parsed::Ignore
        }
        Some("result") => {
            if v["is_error"].as_bool().unwrap_or(false) || v["subtype"].as_str().is_some_and(|s| s != "success") {
                let msg = v["result"].as_str().unwrap_or("claude 回報錯誤");
                return Parsed::Failed(classify_failure(msg));
            }
            let u = &v["usage"];
            Parsed::Done(TokenUsage {
                prompt: u32_of(u, "input_tokens"),
                completion: u32_of(u, "output_tokens"),
                cache_read: u32_of(u, "cache_read_input_tokens"),
                cache_write: u32_of(u, "cache_creation_input_tokens"),
            })
        }
        _ => Parsed::Ignore,
    }
}

// ───────────────────────── 執行 ─────────────────────────

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 系統提示暫存檔，離開作用域就刪。
struct TempPromptFile(PathBuf);

impl TempPromptFile {
    fn create(text: &str) -> Result<Self, AiError> {
        let name = format!("aiterm-claude-cli-{}-{}.txt", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, text).map_err(|e| AiError::Network { message: format!("寫入暫存檔失敗：{e}") })?;
        Ok(Self(path))
    }
}

impl Drop for TempPromptFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 沒有任何 CLAUDE.md／.claude 的資料夾（系統暫存目錄）。
pub(crate) fn neutral_dir() -> PathBuf {
    std::env::temp_dir()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { s.chars().take(max).collect::<String>() + "…" }
}

impl ClaudeCliClient {
    async fn run(&self, req: &GenerateRequest, tx: &mpsc::Sender<GenerateChunk>) -> Result<(), AiError> {
        let program = self.resolve_program()?;
        let sys_file = TempPromptFile::create(&system_text(req))?;
        let prompt = flatten_messages(&req.messages);
        if prompt.trim().is_empty() {
            return Err(AiError::InvalidInput { reason: "沒有可送出的訊息".into() });
        }

        let mut cmd = Command::new(&program);
        cmd.args(build_args(&self.model, &sys_file.0))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // 在中立資料夾跑：claude 會從 cwd 往上載入專案的 CLAUDE.md，
            // `--setting-sources ""` 擋不住它（實測 AITerm 自己的 repo 下首次快取寫入 11,104 tokens，
            // 換到空資料夾 636）。Ask AI 的答案不該被當下的專案說明汙染。
            .current_dir(neutral_dir())
            // 呼叫端放棄（取消）時 future 被丟棄，行程要跟著結束，不留孤兒 claude。
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AiError::NotConfigured
            } else {
                AiError::Network { message: format!("無法啟動 claude：{e}") }
            }
        })?;

        let mut stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");

        // stdin 另開任務寫，避免提示很大、claude 又先輸出時兩邊互相等待。
        let writer = tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
        let err_task = tokio::spawn(async move {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf).await;
            buf
        });

        let read_all = async {
            let mut lines = BufReader::new(stdout).lines();
            let mut usage: Option<TokenUsage> = None;
            let mut failure: Option<AiError2> = None;
            while let Some(line) = lines.next_line().await.map_err(|e| AiError::Network { message: e.to_string() })? {
                match parse_line(&line) {
                    Parsed::Text(t) => {
                        if tx.send(GenerateChunk { delta: t, done: false, usage: None }).await.is_err() {
                            return Err(AiError::Network { message: "接收端已關閉".into() });
                        }
                    }
                    Parsed::Done(u) => usage = Some(u),
                    Parsed::Failed(f) => failure = Some(f),
                    Parsed::Ignore => {}
                }
            }
            Ok::<_, AiError>((usage, failure))
        };

        let outcome = tokio::time::timeout(self.timeout, read_all).await;
        let (usage, failure) = match outcome {
            Ok(r) => r?,
            Err(_) => {
                let _ = child.kill().await;
                return Err(AiError::Network { message: format!("claude 超過 {} 秒沒有完成", self.timeout.as_secs()) });
            }
        };
        let status = child.wait().await.map_err(|e| AiError::Network { message: e.to_string() })?;
        let _ = writer.await;
        let stderr_text = err_task.await.unwrap_or_default();

        if let Some(f) = failure {
            return Err(f.into());
        }
        let Some(usage) = usage else {
            // 沒有 result 事件：非零結束或被中途砍掉。
            let reason = if stderr_text.trim().is_empty() {
                format!("claude 沒有回傳結果（結束碼 {:?}）", status.code())
            } else {
                truncate(stderr_text.trim(), 400)
            };
            return Err(match classify_failure(&reason) {
                AiError2::Model(r) => AiError::ModelError { raw: stderr_text, reason: r },
                other => other.into(),
            });
        };
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: Some(usage) }).await;
        Ok(())
    }
}

#[async_trait]
impl AiProvider for ClaudeCliClient {
    fn id(&self) -> &str { "claude-cli" }
    fn display_name(&self) -> &str { "Claude Code CLI" }

    async fn generate(&self, req: GenerateRequest, tx: mpsc::Sender<GenerateChunk>) -> Result<(), AiError> {
        self.run(&req, &tx).await
    }

    async fn health_check(&self) -> Result<(), AiError> {
        // 一次極小的生成：同時確認找得到、跑得起來、已登入。
        let (tx, mut rx) = mpsc::channel(64);
        let req = GenerateRequest {
            system_prompt: "Reply with the single word OK.".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: Value::String("ping".into()),
                tool_call_id: None,
                tool_calls: None,
            }],
            context: Default::default(),
            mode: super::QueryMode::Chat,
            max_tokens: Some(16),
        };
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let r = self.run(&req, &tx).await;
        drop(tx);
        let _ = drain.await;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("claude_cli_fixture.jsonl");

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage { role: role.into(), content: Value::String(text.into()), tool_call_id: None, tool_calls: None }
    }

    // ── 解析：用真實錄下來的輸出 ──

    #[test]
    fn real_stream_yields_the_text_and_usage_ignoring_noise() {
        let mut text = String::new();
        let mut done = None;
        for line in FIXTURE.lines() {
            match parse_line(line) {
                Parsed::Text(t) => text.push_str(&t),
                Parsed::Done(u) => done = Some(u),
                Parsed::Failed(f) => panic!("不該失敗：{f:?}"),
                Parsed::Ignore => {}
            }
        }
        assert_eq!(text, "你好");
        let u = done.expect("要有 result");
        assert_eq!(u.prompt, 2);
        assert!(u.completion > 0);
        assert!(u.cache_write > 0 || u.cache_read > 0, "快取欄位要被讀進來");
    }

    #[test]
    fn unknown_and_malformed_lines_are_ignored() {
        assert_eq!(parse_line(""), Parsed::Ignore);
        assert_eq!(parse_line("not json"), Parsed::Ignore);
        assert_eq!(parse_line(r#"{"type":"brand_new_event","x":1}"#), Parsed::Ignore);
        assert_eq!(
            parse_line(r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"x"}}}"#),
            Parsed::Ignore,
            "thinking 不能混進回答"
        );
    }

    #[test]
    fn error_results_are_classified() {
        let line = |msg: &str| format!(r#"{{"type":"result","subtype":"success","is_error":true,"result":"{msg}"}}"#);
        assert_eq!(parse_line(&line("Not logged in · Please run /login")), Parsed::Failed(AiError2::Auth));
        assert_eq!(parse_line(&line("Claude usage limit reached")), Parsed::Failed(AiError2::RateLimit));
        assert_eq!(parse_line(&line("boom")), Parsed::Failed(AiError2::Model("boom".into())));
        // subtype 不是 success 也算失敗（例如 error_max_turns）。
        assert!(matches!(
            parse_line(r#"{"type":"result","subtype":"error_during_execution","is_error":false}"#),
            Parsed::Failed(_)
        ));
    }

    // ── 命令列 ──

    #[test]
    fn args_carry_the_lean_flags_and_keep_prompt_out_of_argv() {
        let a = build_args("haiku", Path::new("/tmp/sys.txt"));
        let has = |s: &str| a.iter().any(|x| x == s);
        for f in ["-p", "--verbose", "--include-partial-messages", "--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence"] {
            assert!(has(f), "缺 {f}：{a:?}");
        }
        let after = |flag: &str| {
            let i = a.iter().position(|x| x == flag).unwrap_or_else(|| panic!("缺 {flag}"));
            a[i + 1].clone()
        };
        assert_eq!(after("--tools"), "", "不讓 CLI 用它自己的工具");
        assert_eq!(after("--setting-sources"), "", "不載入使用者的 hooks／CLAUDE.md");
        assert_eq!(after("--output-format"), "stream-json");
        assert_eq!(after("--model"), "haiku");
        assert_eq!(after("--system-prompt-file"), "/tmp/sys.txt");
        assert!(!has("--bare"), "--bare 不吃訂閱 OAuth");
    }

    #[test]
    fn empty_model_omits_the_flag_so_cli_default_applies() {
        assert!(!build_args("  ", Path::new("x")).iter().any(|x| x == "--model"));
    }

    #[test]
    fn single_user_message_is_sent_verbatim() {
        assert_eq!(flatten_messages(&[msg("user", "你好\n第二行")]), "你好\n第二行");
    }

    #[test]
    fn multi_turn_is_labelled_and_system_messages_are_excluded() {
        let out = flatten_messages(&[msg("system", "S"), msg("user", "a"), msg("assistant", "b"), msg("user", "c")]);
        assert!(out.contains("User:\na") && out.contains("Assistant:\nb") && out.contains("User:\nc"), "{out}");
        assert!(!out.contains('S') || !out.contains("System"), "{out}");
        assert!(out.ends_with("Reply as the assistant to the last user message."));
    }

    #[test]
    fn system_messages_fold_into_the_system_prompt() {
        let req = GenerateRequest {
            system_prompt: "base".into(),
            messages: vec![msg("system", "extra"), msg("user", "hi")],
            context: Default::default(),
            mode: super::super::QueryMode::Chat,
            max_tokens: None,
        };
        assert_eq!(system_text(&req), "base\n\nextra");
    }

    // ── 找執行檔 ──

    #[test]
    fn finder_checks_path_then_well_known_dirs() {
        let home = PathBuf::from("/home/u");
        let dirs = candidate_dirs(Some(std::ffi::OsStr::new("/a:/b")), Some(&home));
        assert_eq!(&dirs[..2], &[PathBuf::from("/a"), PathBuf::from("/b")]);
        assert!(dirs.contains(&home.join(".local").join("bin")));
        let want = home.join(".local").join("bin").join(executable_names()[0]);
        assert_eq!(find_in(&dirs, |p| p == want), Some(want.clone()));
        assert_eq!(find_in(&dirs, |_| false), None);
    }

    // ── 端到端：假 claude（只在 unix；用 shell 腳本重播錄下來的輸出）──

    #[cfg(unix)]
    mod fake {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }

        fn req(text: &str) -> GenerateRequest {
            GenerateRequest {
                system_prompt: "sys".into(),
                messages: vec![msg("user", text)],
                context: Default::default(),
                mode: super::super::super::QueryMode::Chat,
                max_tokens: None,
            }
        }

        async fn collect(c: &ClaudeCliClient, r: GenerateRequest) -> (Result<(), AiError>, Vec<GenerateChunk>) {
            let (tx, mut rx) = mpsc::channel(256);
            let res = c.generate(r, tx).await;
            let mut v = vec![];
            while let Ok(ch) = rx.try_recv() {
                v.push(ch);
            }
            (res, v)
        }

        #[tokio::test]
        async fn replays_real_output_into_chunks_and_a_final_done() {
            let d = tempfile::tempdir().unwrap();
            let fx = d.path().join("fx.jsonl");
            std::fs::write(&fx, FIXTURE).unwrap();
            // 同時驗證：提示真的從 stdin 進來，系統提示檔真的存在。
            let p = script(
                d.path(),
                "claude",
                &format!(
                    r#"cat > "{d}/stdin.txt"
pwd -P > "{d}/cwd.txt"
while [ $# -gt 0 ]; do [ "$1" = "--system-prompt-file" ] && cp "$2" "{d}/sys.txt"; shift; done
cat "{fx}""#,
                    d = d.path().display(),
                    fx = fx.display()
                ),
            );
            let c = ClaudeCliClient::with_program("haiku", p, Duration::from_secs(20));
            let (res, chunks) = collect(&c, req("你好\n第二行")).await;
            res.unwrap();
            let text: String = chunks.iter().map(|c| c.delta.as_str()).collect();
            assert_eq!(text, "你好");
            let last = chunks.last().unwrap();
            assert!(last.done && last.usage.is_some());
            assert_eq!(chunks.iter().filter(|c| c.done).count(), 1);
            assert_eq!(std::fs::read_to_string(d.path().join("stdin.txt")).unwrap(), "你好\n第二行");
            assert_eq!(std::fs::read_to_string(d.path().join("sys.txt")).unwrap(), "sys");
            let cwd = std::fs::read_to_string(d.path().join("cwd.txt")).unwrap();
            assert_eq!(
                std::fs::canonicalize(cwd.trim()).unwrap(),
                std::fs::canonicalize(neutral_dir()).unwrap(),
                "要在中立資料夾跑，不能繼承 AITerm 的 cwd（會載入專案 CLAUDE.md）"
            );
        }

        #[tokio::test]
        async fn not_logged_in_maps_to_auth_failed() {
            let d = tempfile::tempdir().unwrap();
            let p = script(
                d.path(),
                "claude",
                r#"cat >/dev/null
echo '{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}'"#,
            );
            let c = ClaudeCliClient::with_program("", p, Duration::from_secs(20));
            let (res, _) = collect(&c, req("x")).await;
            assert!(matches!(res, Err(AiError::AuthFailed)), "{res:?}");
        }

        #[tokio::test]
        async fn nonzero_exit_without_result_reports_stderr() {
            let d = tempfile::tempdir().unwrap();
            let p = script(d.path(), "claude", "cat >/dev/null\necho 'kaboom' >&2\nexit 3");
            let c = ClaudeCliClient::with_program("", p, Duration::from_secs(20));
            let (res, _) = collect(&c, req("x")).await;
            match res {
                Err(AiError::ModelError { reason, .. }) => assert!(reason.contains("kaboom"), "{reason}"),
                other => panic!("{other:?}"),
            }
        }

        #[tokio::test]
        async fn missing_program_is_not_configured() {
            let c = ClaudeCliClient::with_program("", PathBuf::from("/nonexistent/claude"), Duration::from_secs(5));
            let (res, _) = collect(&c, req("x")).await;
            assert!(matches!(res, Err(AiError::NotConfigured)), "{res:?}");
        }

        #[tokio::test]
        async fn timeout_kills_the_child() {
            let d = tempfile::tempdir().unwrap();
            let pidfile = d.path().join("pid");
            let p = script(d.path(), "claude", &format!("echo $$ > \"{}\"\nexec sleep 30", pidfile.display()));
            let c = ClaudeCliClient::with_program("", p, Duration::from_millis(600));
            let (res, _) = collect(&c, req("x")).await;
            assert!(matches!(res, Err(AiError::Network { .. })), "{res:?}");
            let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            // kill -0：行程還在就回 0。
            let alive = std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().unwrap().success();
            assert!(!alive, "逾時後 claude 不能還活著");
        }

        #[tokio::test]
        async fn dropping_the_future_kills_the_child() {
            let d = tempfile::tempdir().unwrap();
            let pidfile = d.path().join("pid");
            let p = script(d.path(), "claude", &format!("echo $$ > \"{}\"\nexec sleep 30", pidfile.display()));
            let c = ClaudeCliClient::with_program("", p, Duration::from_secs(60));
            let (tx, _rx) = mpsc::channel(8);
            let r = req("x");
            let _ = tokio::time::timeout(Duration::from_millis(600), c.generate(r, tx)).await; // 取消
            let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            let alive = std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().unwrap().success();
            assert!(!alive, "取消後不能留下孤兒 claude");
        }
    }

    // ── 真 claude 實機（手動：cargo test claude_cli_real -- --ignored）──

    #[tokio::test]
    #[ignore]
    async fn claude_cli_real_roundtrip_is_lean() {
        let c = ClaudeCliClient::new("haiku".into());
        let (tx, mut rx) = mpsc::channel(256);
        let r = GenerateRequest {
            system_prompt: "只回覆兩個字".into(),
            messages: vec![msg("user", "你好")],
            context: Default::default(),
            mode: super::super::QueryMode::Chat,
            max_tokens: None,
        };
        c.generate(r, tx).await.unwrap();
        let mut text = String::new();
        let mut usage = None;
        while let Ok(ch) = rx.try_recv() {
            text.push_str(&ch.delta);
            if ch.done {
                usage = ch.usage;
            }
        }
        assert!(!text.trim().is_empty());
        let u = usage.unwrap();
        assert!(u.cache_write + u.cache_read < 5000, "瘦身旗標失效？{u:?}");
    }
}
