//! Server 自動處理題目：為每一題啟動一個 `claude -p` 子行程（非互動），結果變成草稿。
//!
//! - **唯讀**：只給讀檔與搜尋工具；外部客戶的題目、只診斷的管道、提問類題目一律這個模式。
//! - **可改檔案**：只對內部、可改檔案的管道、要改程式的題目。在隔離的 git worktree 與專屬分支裡改，
//!   做完 commit 並跑驗證，**永不自動合併**——要管理員在 AITerm 核准（見 `merge`）。
//!
//! 認證用 server 主機上 Claude Code 自己的登入；這裡不存也不讀任何 Claude 憑證。
use crate::state::AppState;
use aiterm_intake_core::classify::{self, Kind};
use aiterm_intake_core::config::{self, Handling};
use aiterm_intake_core::dispatcher;
use aiterm_intake_core::store::{self, IntakeItem};
use aiterm_intake_core::verify;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub const SETTING_KEY: &str = "server_runner";
/// 自動處理時 server 在領取紀錄上的名字。
pub const WHO: &str = "server";
/// 多久掃一次有沒有可以自動處理的題目。
pub const TICK_EVERY: Duration = Duration::from_secs(10);
/// 處理中每隔多久向自己續約（租約 15 分鐘，留很大的餘裕）。
const HEARTBEAT_EVERY: Duration = Duration::from_secs(120);
/// 多久重新確認一次 claude 能不能用。
const AVAILABILITY_TTL: Duration = Duration::from_secs(60);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct RunnerSettings {
    /// Claude Code 執行檔；預設 `claude`（要在 server 的 PATH 上）。
    pub claude_path: String,
    /// 同時最多處理幾題。
    pub max_concurrent: usize,
    /// 每題最久執行幾秒（必須小於領取租約）。
    pub timeout_secs: u64,
    /// Claude Code 每題最多幾個回合。
    pub max_turns: u32,
}

impl Default for RunnerSettings {
    fn default() -> Self {
        Self { claude_path: "claude".into(), max_concurrent: 1, timeout_secs: 600, max_turns: 20 }
    }
}

impl RunnerSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.claude_path.trim().is_empty() {
            return Err("請填 Claude Code 的執行檔路徑".into());
        }
        if !(1..=8).contains(&self.max_concurrent) {
            return Err("同時處理的題數要在 1～8".into());
        }
        if !(30..=800).contains(&self.timeout_secs) {
            return Err("逾時要在 30～800 秒（必須小於 15 分鐘的領取租約）".into());
        }
        if !(1..=100).contains(&self.max_turns) {
            return Err("回合上限要在 1～100".into());
        }
        Ok(())
    }
}

pub async fn load(pool: &sqlx::SqlitePool) -> RunnerSettings {
    store::get_setting(pool, SETTING_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub async fn save(pool: &sqlx::SqlitePool, s: &RunnerSettings) -> Result<(), String> {
    s.validate()?;
    store::set_setting(pool, SETTING_KEY, &serde_json::to_string(s).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())
}

/// claude 能不能用（最近一次檢查的結果）。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Availability {
    pub checked: bool,
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Runner {
    pub(crate) running: AtomicUsize,
    avail: parking_lot::Mutex<Option<(std::time::Instant, String, Availability)>>,
    task: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Runner {
    pub fn running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }
}

/// 執行 `claude --version` 確認能用。
pub async fn check_claude(path: &str) -> Availability {
    let run = Command::new(path).arg("--version").stdin(Stdio::null()).kill_on_drop(true).output();
    match tokio::time::timeout(Duration::from_secs(15), run).await {
        Err(_) => Availability { checked: true, available: false, version: None, error: Some("執行 claude --version 逾時".into()) },
        Ok(Err(e)) => Availability { checked: true, available: false, version: None, error: Some(format!("找不到或無法執行 {path}：{e}")) },
        Ok(Ok(o)) if o.status.success() => Availability { checked: true, available: true, version: Some(String::from_utf8_lossy(&o.stdout).trim().to_string()), error: None },
        Ok(Ok(o)) => Availability { checked: true, available: false, version: None, error: Some(format!("claude --version 失敗：{}", String::from_utf8_lossy(&o.stderr).trim().chars().take(200).collect::<String>())) },
    }
}

/// 目前的可用狀態；`force` 或快取過期就重新檢查。
pub async fn availability(st: &AppState, force: bool) -> Availability {
    let path = load(&st.pools.intake).await.claude_path;
    {
        let g = st.runner.avail.lock();
        if let Some((at, p, a)) = g.as_ref() {
            if !force && *p == path && at.elapsed() < AVAILABILITY_TTL {
                return a.clone();
            }
        }
    }
    let a = check_claude(&path).await;
    *st.runner.avail.lock() = Some((std::time::Instant::now(), path, a.clone()));
    a
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    ReadOnly,
    /// 可改檔案；`verify_command` 是唯一額外開放的指令（可為空）。
    Edit { verify_command: String },
    /// 驗收：開放 Bash 與（有的話）瀏覽器，但不給編輯工具；`deny` 是禁止清單（每條 `Bash(...)`）。
    /// `mcp_config` 是只含瀏覽器的 MCP 設定檔路徑（沒有 Node 就沒有）。
    Verify { deny: Vec<String>, browser: bool, mcp_config: Option<String> },
}

/// 子行程只拿到它需要的環境變數：不要把 server 的其他祕密環境帶進去。
fn minimal_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| {
            matches!(k.as_str(), "PATH" | "HOME" | "USER" | "LOGNAME" | "LANG" | "SHELL" | "TMPDIR" | "TZ" | "SSL_CERT_FILE" | "SSL_CERT_DIR")
                || k.starts_with("LC_")
                || k.starts_with("XDG_")
                || k.starts_with("ANTHROPIC_")
                || k.starts_with("CLAUDE_")
        })
        .collect()
}

/// 這次呼叫要不要保留對話：預設不留（`--no-session-persistence`）；
/// 要能中途接續的長任務（Coder 做卡片）用固定的對話 id，被打斷後下一次用 `--resume` 接回去。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SessionUse {
    #[default]
    None,
    /// 開一段新的對話並指定 id（`--session-id`；id 已經用過會被 Claude Code 拒絕）。
    New(String),
    /// 接續上次的對話（`--resume`）；找不到會失敗，見 `is_session_missing`。
    Resume(String),
}

/// `--resume` 找不到那段對話（對話檔被刪、換了機器）：這種失敗不是任務本身的問題，改開新的就好。
pub fn is_session_missing(err: &str) -> bool {
    err.contains("No conversation found")
}

/// 對話 id 要能安全當檔名與參數：只收 UUID 的字元與長度。
pub fn valid_session_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// 刪掉磁碟上那段對話的檔案（`<Claude 設定資料夾>/projects/*/<id>.jsonl`）。
/// server 通常裝在使用者自己的桌面電腦上、用同一個帳號，不清的話每張卡都會在使用者平常的對話清單裡多一筆。
pub fn remove_session_file(id: &str) {
    if !valid_session_id(id) {
        return;
    }
    let base = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".claude")));
    let Some(projects) = base.map(|b| b.join("projects")) else { return };
    let Ok(dirs) = std::fs::read_dir(projects) else { return };
    for d in dirs.flatten() {
        let file = d.path().join(format!("{id}.jsonl"));
        if file.is_file() && std::fs::remove_file(&file).is_ok() {
            // 這張卡的 worktree 專屬的資料夾，空了就一起收掉（非空刪不掉，不會動到別的東西）。
            let _ = std::fs::remove_dir(d.path());
        }
    }
}

/// 殺掉上一個 server 行程留下的孤兒 Claude Code：server 被強制結束（當機、斷電、`kill -9`）時，
/// 它啟動的 `claude` 子行程不會跟著結束，還在改同一個 worktree；這時接續（`--resume`）會和它同時寫同一批檔案。
/// 靠命令列裡的對話 id（`--session-id <id>` 或 `--resume <id>`）認人，只動我們自己記過的對話；回傳殺掉幾個。
pub fn kill_orphans(session_ids: &[String]) -> usize {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    if session_ids.is_empty() {
        return 0;
    }
    let mut sys = System::new();
    // 預設不抓命令列；認人要靠它。
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always));
    let mut killed = 0;
    for p in sys.processes().values() {
        let cmd: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let ours = cmd.windows(2).any(|w| matches!(w[0].as_str(), "--session-id" | "--resume") && session_ids.iter().any(|id| *id == w[1]));
        if ours && p.pid().as_u32() != std::process::id() && p.kill() {
            killed += 1;
        }
    }
    killed
}

/// 組出 `claude -p` 的參數（提示詞走標準輸入，不放在參數裡）。
pub fn claude_args(s: &RunnerSettings, mode: &Mode, extra_dirs: &[String]) -> Vec<String> {
    claude_args_with(s, mode, extra_dirs, &SessionUse::None)
}

pub fn claude_args_with(s: &RunnerSettings, mode: &Mode, extra_dirs: &[String], session: &SessionUse) -> Vec<String> {
    let (allowed, disallowed) = match mode {
        Mode::ReadOnly => ("Read,Grep,Glob".to_string(), "Bash,Edit,Write,MultiEdit,NotebookEdit,WebFetch,WebSearch,Task".to_string()),
        Mode::Edit { verify_command } => {
            let mut a = "Read,Grep,Glob,Edit,Write,MultiEdit".to_string();
            if !verify_command.trim().is_empty() {
                a.push_str(&format!(",Bash({})", verify_command.trim()));
            }
            (a, "WebFetch,WebSearch,NotebookEdit".to_string())
        }
        Mode::Verify { deny, browser, .. } => {
            let allowed = if *browser { "Read,Grep,Glob,Bash,mcp__playwright".to_string() } else { "Read,Grep,Glob,Bash".to_string() };
            let mut dis: Vec<String> = ["Edit", "Write", "MultiEdit", "NotebookEdit", "WebFetch", "WebSearch"].iter().map(|x| x.to_string()).collect();
            dis.extend(deny.iter().cloned());
            (allowed, dis.join(","))
        }
    };
    let mut args: Vec<String> = ["-p", "--output-format", "json"].iter().map(|x| x.to_string()).collect();
    match session {
        SessionUse::None => args.push("--no-session-persistence".into()),
        SessionUse::New(id) => args.extend(["--session-id".into(), id.clone()]),
        SessionUse::Resume(id) => args.extend(["--resume".into(), id.clone()]),
    }
    args.extend(["--permission-mode".into(), "dontAsk".into()]);
    args.extend(["--max-turns".into(), s.max_turns.to_string(), "--allowedTools".into(), allowed, "--disallowedTools".into(), disallowed]);
    for d in extra_dirs {
        args.extend(["--add-dir".into(), d.clone()]);
    }
    if let Mode::Verify { mcp_config: Some(path), .. } = mode {
        args.extend(["--mcp-config".into(), path.clone(), "--strict-mcp-config".into()]);
    }
    args
}

/// 執行一次 `claude -p`，回傳最終回答的文字。逾時會終止子行程。
pub async fn exec_claude(s: &RunnerSettings, cwd: &Path, mode: &Mode, extra_dirs: &[String], prompt: &str) -> Result<String, String> {
    exec_claude_as(s, cwd, mode, extra_dirs, prompt, None).await
}

/// `--model` 的值要能安全放進參數：只收英數與 `. _ - [ ] :`，而且不能以 `-` 開頭（會被當成另一個旗標）。
pub fn valid_model(m: &str) -> bool {
    !m.is_empty() && m.len() <= 80 && !m.starts_with('-') && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '[' | ']' | ':'))
}

/// 同 `exec_claude`，可指定這次用的模型（Agent 設定的 `model`；空白或不合法就沿用預設）。
pub async fn exec_claude_as(s: &RunnerSettings, cwd: &Path, mode: &Mode, extra_dirs: &[String], prompt: &str, model: Option<&str>) -> Result<String, String> {
    exec_claude_session(s, cwd, mode, extra_dirs, prompt, model, &SessionUse::None).await
}

/// 同 `exec_claude_as`，另外可指定這次對話要不要保留、或接續上一段。
pub async fn exec_claude_session(s: &RunnerSettings, cwd: &Path, mode: &Mode, extra_dirs: &[String], prompt: &str, model: Option<&str>, session: &SessionUse) -> Result<String, String> {
    exec_claude_full(s, cwd, mode, extra_dirs, prompt, model, session).await.map(|r| r.text)
}

/// 一次 `claude -p` 的結果：最終回答，以及被權限規則（禁止清單）擋下的 Bash 命令。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClaudeRun {
    pub text: String,
    pub denied: Vec<String>,
}

/// 輸出裡被拒絕的 Bash 命令（`permission_denials` 裡 `tool_name` 是 Bash 的 `tool_input.command`）。
pub fn denied_commands(stdout: &str) -> Vec<String> {
    let t = stdout.trim();
    let Some(v) = serde_json::from_str::<serde_json::Value>(t).ok().or_else(|| serde_json::from_str(t.lines().last().unwrap_or("")).ok()) else { return Vec::new() };
    v["permission_denials"]
        .as_array()
        .map(|a| a.iter().filter(|d| d["tool_name"] == "Bash").filter_map(|d| d["tool_input"]["command"].as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub async fn exec_claude_full(s: &RunnerSettings, cwd: &Path, mode: &Mode, extra_dirs: &[String], prompt: &str, model: Option<&str>, session: &SessionUse) -> Result<ClaudeRun, String> {
    let mut args = claude_args_with(s, mode, extra_dirs, session);
    if let Some(m) = model.map(str::trim).filter(|m| valid_model(m)) {
        args.extend(["--model".to_string(), m.to_string()]);
    }
    let mut cmd = Command::new(&s.claude_path);
    cmd.args(args).current_dir(cwd).env_clear().envs(minimal_env()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("無法啟動 {}：{e}", s.claude_path))?;
    let mut stdin = child.stdin.take().ok_or("無法開啟標準輸入")?;
    let prompt = prompt.to_string();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(prompt.as_bytes()).await;
        let _ = stdin.shutdown().await;
    });
    let out = match tokio::time::timeout(Duration::from_secs(s.timeout_secs), child.wait_with_output()).await {
        Err(_) => {
            writer.abort();
            return Err(format!("處理逾時（超過 {} 秒），已終止", s.timeout_secs));
        }
        Ok(Err(e)) => return Err(format!("等待 Claude Code 失敗：{e}")),
        Ok(Ok(o)) => o,
    };
    let _ = writer.await;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let parsed = parse_output(&stdout);
    if !out.status.success() {
        // 標準輸出是空的時候，真正的原因在標準錯誤（例如 `--resume` 找不到對話只寫在 stderr）；不能只回「輸出不是預期的格式」把它吞掉。
        let stderr: String = String::from_utf8_lossy(&out.stderr).trim().chars().take(300).collect();
        let why = if stdout.trim().is_empty() && !stderr.is_empty() { stderr } else { parsed.err().unwrap_or(stderr) };
        return Err(format!("Claude Code 失敗（{}）：{why}", out.status));
    }
    Ok(ClaudeRun { text: parsed?, denied: denied_commands(&stdout) })
}

/// Claude Code 的錯誤是不是「額度或速率限制」（不是這張卡本身的問題，等一等就會好）。
/// 回傳該等到什麼時候（unix 秒）：錯誤裡帶了重置時間（`…limit reached|<unix 秒>`）就用它，否則等 20 分鐘。
pub fn quota_wait(err: &str, now: i64) -> Option<i64> {
    let low = err.to_lowercase();
    let hit = ["usage limit", "limit reached", "rate limit", "rate_limit", "too many requests", "quota", "overloaded", "429"].iter().any(|k| low.contains(k));
    if !hit {
        return None;
    }
    // 形如「Claude AI usage limit reached|1760000000」。
    let stamp = err.split('|').skip(1).filter_map(|p| p.trim().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<i64>().ok()).find(|t| (1_000_000_000..10_000_000_000).contains(t));
    Some(match stamp {
        Some(t) if t > now => t.min(now + 12 * 3600) + 5, // 重置後多等幾秒；最多等 12 小時
        Some(_) => now + 60,                               // 時間已過：很快再試
        None => now + 20 * 60,
    })
}

/// 解析 `--output-format json` 的輸出：取 `result`；`is_error` 當錯誤。
pub fn parse_output(stdout: &str) -> Result<String, String> {
    let t = stdout.trim();
    let v: serde_json::Value = serde_json::from_str(t).or_else(|_| serde_json::from_str(t.lines().last().unwrap_or(""))).map_err(|_| format!("Claude Code 的輸出不是預期的格式：{}", t.chars().take(200).collect::<String>()))?;
    let text = v["result"].as_str().unwrap_or("").to_string();
    if v["is_error"].as_bool().unwrap_or(false) {
        return Err(if text.is_empty() { "Claude Code 回報錯誤".into() } else { text.chars().take(300).collect() });
    }
    if text.trim().is_empty() {
        return Err("Claude Code 沒有給出回答".into());
    }
    Ok(text)
}

pub(crate) async fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let o = Command::new("git")
        .args(["-c", "user.name=AITerm Server", "-c", "user.email=aiterm-server@localhost"])
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("無法執行 git：{e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    } else {
        Err(format!("git {} 失敗：{}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim().chars().take(300).collect::<String>()))
    }
}

/// 這題的 worktree 與分支。
pub fn worktree_path(st: &AppState, item_id: &str) -> PathBuf {
    st.data_dir.join("worktrees").join(item_id)
}

pub fn branch_name(item_id: &str) -> String {
    format!("intake/{}", item_id.chars().take(12).collect::<String>())
}

/// 移除 worktree 與分支（找不到就算了，不報錯）。
pub async fn discard(st: &AppState, work_dir: &Path, item_id: &str, branch: &str) {
    let wt = worktree_path(st, item_id);
    let _ = git(work_dir, &["worktree", "remove", "--force", &wt.to_string_lossy()]).await;
    let _ = std::fs::remove_dir_all(&wt);
    let _ = git(work_dir, &["worktree", "prune"]).await;
    let _ = git(work_dir, &["branch", "-D", branch]).await;
}

/// 把分支以 `--no-ff` 合併進 `work_dir` 目前的分支；衝突就中止，不留半套。成功後清掉 worktree 與分支。
pub async fn merge(st: &AppState, work_dir: &Path, item_id: &str, branch: &str, subject: &str) -> Result<(), String> {
    let msg = format!("合併問題處理：{}", subject.chars().take(60).collect::<String>());
    if let Err(e) = git(work_dir, &["merge", "--no-ff", "-m", &msg, branch]).await {
        let _ = git(work_dir, &["merge", "--abort"]).await;
        return Err(format!("合併失敗（已還原，什麼都沒有改）：{e}"));
    }
    discard(st, work_dir, item_id, branch).await;
    Ok(())
}

async fn project_work_dir(st: &AppState, project_id: &str) -> String {
    sqlx::query_scalar::<_, String>("SELECT work_dir FROM projects WHERE id = ?").bind(project_id).fetch_optional(&st.pools.server).await.ok().flatten().unwrap_or_default()
}

/// 這題要用哪個模式：外部客戶一律唯讀；可改檔案只給「內部、管道可改檔案、種類是要改程式」。
/// 沒標種類時問 server 的 AI 分類；沒設 AI 或分類失敗就保守用唯讀。
pub async fn decide_mode(st: &AppState, item: &IntakeItem, cfg: &config::Config) -> Mode {
    if item.sender_level != "internal" || cfg.handling(&item.source) != Handling::Edit {
        return Mode::ReadOnly;
    }
    let kind = match item.kind.as_deref() {
        Some("question") => Kind::Question,
        Some(_) => Kind::CodeChange,
        None => match classify::classify(st.ai.as_ref(), &item.subject, &item.body).await {
            Ok(k) => k,
            Err(_) => return Mode::ReadOnly,
        },
    };
    if kind == Kind::CodeChange {
        Mode::Edit { verify_command: cfg.verify_command.clone() }
    } else {
        Mode::ReadOnly
    }
}

/// 處理一題（已經領取）。不管成敗都會把題目交回（草稿或失敗），不會留在處理中。
async fn process(st: Arc<AppState>, item: IntakeItem) {
    let beat = {
        let (pool, id) = (st.pools.intake.clone(), item.id.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(HEARTBEAT_EVERY).await;
                if !matches!(store::renew_claim(&pool, &id, WHO, crate::db::now()).await, Ok(true)) {
                    break;
                }
            }
        })
    };
    let outcome = process_inner(&st, &item).await;
    beat.abort();
    let now = crate::db::now();
    let pool = &st.pools.intake;
    match outcome {
        Ok(done) => {
            let v = done.verification.as_ref().map(|(t, ok)| (t.as_str(), *ok));
            // 分支與「待合併」要跟著變成草稿一起寫入（同一個 UPDATE），不然別人會看到「已完成卻沒有分支」。
            match store::finish_claim_with_branch(pool, &item.id, WHO, &done.reply, v, done.branch.as_deref(), now).await {
                Ok(true) => {
                    crate::audit::record(&st.pools.server, WHO, "item.auto", &item.id).await;
                }
                _ => {
                    // 租約已被收回：結果不採用，別留下沒人管的分支。
                    if let (Some(branch), Some(wd)) = (&done.branch, done.work_dir.as_ref()) {
                        discard(&st, wd, &item.id, branch).await;
                    }
                }
            }
        }
        Err(e) => {
            store::fail_claim(pool, &item.id, WHO, &e, now).await.ok();
            crate::audit::record(&st.pools.server, WHO, "item.auto_fail", &item.id).await;
        }
    }
    st.runner.running.fetch_sub(1, Ordering::SeqCst);
    use aiterm_intake_core::host::Notifier;
    st.events.changed();
}

struct Done {
    reply: String,
    verification: Option<(String, bool)>,
    branch: Option<String>,
    work_dir: Option<PathBuf>,
}

async fn process_inner(st: &Arc<AppState>, item: &IntakeItem) -> Result<Done, String> {
    let settings = load(&st.pools.intake).await;
    let cfg = config::load(&st.pools.intake, &item.project_id).await;
    let wd = project_work_dir(st, &item.project_id).await;
    if wd.trim().is_empty() {
        return Err("這個專案沒有設定工作資料夾（work_dir），server 不知道要在哪裡處理".into());
    }
    let work_dir = PathBuf::from(wd.trim());
    if !work_dir.is_dir() {
        return Err(format!("工作資料夾不存在：{}", work_dir.display()));
    }
    let internal = item.sender_level == "internal";
    let mode = decide_mode(st, item, &cfg).await;
    let attachments = store::list_attachments(&st.pools.intake, &item.id).await.unwrap_or_default();
    let refs: Vec<String> = cfg.refs_for(internal).into_iter().filter(|p| Path::new(p).is_dir()).collect();
    let mut extra: Vec<String> = refs.clone();
    if let Some(dir) = attachments.first().and_then(|a| Path::new(&a.path).parent()) {
        extra.push(dir.to_string_lossy().to_string());
    }
    let prompt = format!("{}{}", dispatcher::build_prompt(item, &attachments, matches!(mode, Mode::ReadOnly)), dispatcher::refs_note(&refs));

    match mode {
        // 驗收只由編排（`verifier::run_verify`）使用，問題收集的題目不會走到這裡。
        Mode::Verify { .. } => Err("內部錯誤：題目不應該用驗收模式處理".to_string()),
        Mode::ReadOnly => {
            let text = exec_claude(&settings, &work_dir, &mode, &extra, &prompt).await?;
            Ok(Done { reply: aiterm_intake_core::filter::redact(&text), verification: None, branch: None, work_dir: None })
        }
        Mode::Edit { .. } => {
            let branch = branch_name(&item.id);
            let wt = worktree_path(st, &item.id);
            std::fs::create_dir_all(wt.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
            git(&work_dir, &["rev-parse", "--verify", "HEAD"]).await.map_err(|_| "工作資料夾不是有提交紀錄的 git 專案，無法開隔離的 worktree".to_string())?;
            git(&work_dir, &["worktree", "add", "-b", &branch, &wt.to_string_lossy(), "HEAD"]).await?;
            let result = edit_in_worktree(st, item, &settings, &cfg, &mode, &wt, &extra, &prompt).await;
            match result {
                Ok((reply, report, changed)) => {
                    let verification = Some((aiterm_intake_core::filter::redact(&report.text()), report.passed()));
                    let reply = format!("{}\n\n{}", aiterm_intake_core::filter::redact(&reply), verification.as_ref().map(|v| v.0.clone()).unwrap_or_default());
                    if changed {
                        Ok(Done { reply, verification, branch: Some(branch), work_dir: Some(work_dir) })
                    } else {
                        discard(st, &work_dir, &item.id, &branch).await;
                        Ok(Done { reply, verification, branch: None, work_dir: None })
                    }
                }
                Err(e) => {
                    discard(st, &work_dir, &item.id, &branch).await;
                    Err(e)
                }
            }
        }
    }
}

/// 在 worktree 裡跑 Claude Code、驗證、commit；回傳（回答、驗證報告、是否有改動）。
async fn edit_in_worktree(_st: &AppState, item: &IntakeItem, s: &RunnerSettings, cfg: &config::Config, mode: &Mode, wt: &Path, extra: &[String], prompt: &str) -> Result<(String, verify::Report, bool), String> {
    let base = verify::snapshot(wt).await;
    let text = exec_claude(s, wt, mode, extra, prompt).await?;
    let report = verify::verify(wt, base.as_deref(), &cfg.verify_command, Duration::from_secs(cfg.verify_timeout_secs.clamp(1, 600))).await;
    let changed = !report.files.is_empty();
    if changed {
        git(wt, &["add", "-A"]).await?;
        git(wt, &["commit", "-q", "-m", &format!("問題處理：{}", item.subject.chars().take(60).collect::<String>())]).await?;
    }
    Ok((text, report, changed))
}

/// 掃一次：對開了自動處理的專案，在並行上限內領題目並開始處理。回傳這次開始處理的題數。
pub async fn tick_once(st: &Arc<AppState>) -> usize {
    let settings = load(&st.pools.intake).await;
    if settings.validate().is_err() {
        return 0;
    }
    let mut started = 0;
    let mut checked = false;
    for p in crate::routes_data::project_list(st).await {
        let cfg = config::load(&st.pools.intake, &p.id).await;
        if !cfg.auto_process || !cfg.enabled {
            continue;
        }
        if !checked {
            checked = true;
            if !availability(st, false).await.available {
                return 0; // claude 不能用：不領任何題目，題目留在待派送讓成員處理
            }
        }
        while st.runner.running() < settings.max_concurrent {
            let Ok(Some(item)) = store::claim_next_for(&st.pools.intake, &p.id, WHO, crate::db::now()).await else { break };
            st.runner.running.fetch_add(1, Ordering::SeqCst);
            started += 1;
            let st2 = st.clone();
            tokio::spawn(process(st2, item));
        }
    }
    started
}

pub fn start(st: &Arc<AppState>) {
    let st2 = st.clone();
    let h = tokio::spawn(async move {
        loop {
            tick_once(&st2).await;
            tokio::time::sleep(TICK_EVERY).await;
        }
    });
    *st.runner.task.lock() = Some(h);
}

pub fn stop(st: &AppState) {
    if let Some(h) = st.runner.task.lock().take() {
        h.abort();
    }
}
