//! Turning a queued card into a live dispatch: compose the prompt text,
//! spawn a visible PTY tab running the configured `claude` command, wait for
//! it to settle, then type the prompt in using the same CR-terminated /
//! done-marker-instruction sequencing `coordination_ops::send_input` uses.

use std::time::Duration;

use portable_pty::PtySize;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::config::{ConfigStore, TierMapping};
use crate::pty::PtyManager;
use crate::pty::session::done_marker_instruction;
use crate::tasks::store::TaskRow;

/// Body text plus, if any attachments, one trailing line pointing `claude` at
/// their on-disk paths (they've already been copied into the task dir).
pub fn build_prompt(body: &str, attachment_paths: &[String]) -> String {
    if attachment_paths.is_empty() {
        return body.to_string();
    }
    let list = attachment_paths.join("、");
    format!("{body}\n\n（相關附件：{list}）")
}

/// 派工前要不要注入 Claude Bridge 的環境變數，以及要不要先把這張卡指定
/// 的帳號組合快照套用成全域橋接設定。
///
/// 吃純值（`bridge_port`/`bridge_token`）而不是 `Arc<BridgeState>`/
/// `Arc<SecretStore>`——呼叫端自己解析好再傳進來，這個函式才不需要真的
/// 橋接 server 或 OS keychain 就能完整測試。降級邏輯跟 `pty_create`
/// command（`pty/commands.rs`）現有邏輯逐字一致：server 沒在跑就不注入。
#[derive(serde::Deserialize)]
struct BridgeTierSnapshot {
    opus: Option<TierMapping>,
    sonnet: Option<TierMapping>,
    haiku: Option<TierMapping>,
}

pub fn resolve_bridge_env(
    config: &ConfigStore,
    task: &TaskRow,
    bridge_port: Option<u16>,
    bridge_token: Option<String>,
) -> Option<(u16, String)> {
    if task.use_bridge {
        if let Some(json) = &task.bridge_tiers {
            if let Ok(snap) = serde_json::from_str::<BridgeTierSnapshot>(json) {
                // `update` 就算磁碟寫入失敗，記憶體內的設定仍然會被改到
                // （`ConfigStore::update` 的實作先套用閉包、才存檔）——這裡
                // 在乎的是「這次派工當下」讀到的即時設定，忽略持久化失敗
                // 比讓整個派工失敗更合理。
                let _ = config.update(|c| {
                    c.claude_bridge.opus = snap.opus;
                    c.claude_bridge.sonnet = snap.sonnet;
                    c.claude_bridge.haiku = snap.haiku;
                });
            }
        }
    }
    match (task.use_bridge, bridge_port) {
        (true, Some(port)) => bridge_token.map(|t| (port, t)),
        _ => None,
    }
}

/// 這個設定值看起來是不是 Claude Code 本身。
///
/// `claude_command` 是使用者可改的設定，而這個檔案的 `NO_TUI_QUIET_MS`
/// 路徑是刻意為「設定成不是全螢幕 TUI 的指令」留的。`--session-id` 是
/// claude 專屬旗標，接到別的指令上會讓它直接啟動失敗——所以只在第一個
/// token 的檔名確實是 `claude` / `claude.exe` 時才給 session id。
///
/// 認錯的代價是不對稱的：漏認只是這張卡片退回舊的 transcript.txt，誤認
/// 是整個派工壞掉。所以比對用相等而不是包含（`claude-code` 必須是 false）。
///
/// 分隔符自己切，不用 `Path::file_name()`：那個是平台相依的，在 macOS 上
/// `Path::new(r"C:\claude.exe").file_name()` 會回傳整串（`\` 不是 Unix 的
/// 分隔符），Windows 路徑因此在 mac 的 CI 上永遠對不上。同一個理由讓
/// `session_log::encode_project_dir` 也自己處理 `/` 與 `\`。
///
/// **已知限制**：路徑含空格時（例如 `C:\Program Files\claude.exe`）
/// `split_whitespace` 會在空格處切斷，拿到 `C:\Program`，於是回傳 false。
/// 加引號也沒用——這裡沒有引號感知。刻意不處理：要正確處理得引進一個
/// shell 語法的 tokenizer，而這個情況的失敗方向是安全的（那張卡片退回
/// transcript.txt，派工照樣跑），不值得為它擴大範圍。
///
/// 比對前轉小寫是為了 Windows（`CLAUDE.EXE` 跟 `claude.exe` 是同一個檔案）。
/// Unix 的檔名大小寫敏感，所以嚴格說這在 Unix 上偏寬——真的有人把不相干的
/// 執行檔命名為 `Claude` 的話會被誤認。實務上碰不到，刻意接受，記在這裡是
/// 為了讓讀的人知道這是權衡過的，不是漏看的。
pub fn looks_like_claude(command: &str) -> bool {
    let Some(first) = command.split_whitespace().next() else {
        return false;
    };
    let name = first
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(first)
        .to_lowercase();
    name == "claude" || name == "claude.exe"
}

/// 真正要送進終端機的那一行。有 session id 就接上 `--session-id <uuid>`，
/// 沒有就是指令原樣。
///
/// 呼叫端負責先用 `looks_like_claude` 決定要不要給 session id——這裡不再
/// 判斷一次，免得兩處規則漂移。
pub fn launch_command(command: &str, session_id: Option<&str>) -> String {
    match session_id {
        Some(sid) => format!("{command} --session-id {sid}"),
        None => command.to_string(),
    }
}

/// 信任畫面上「我信任這個資料夾」那一個選項的文字，**已去掉所有空白**。
/// Claude Code 的 TUI 用游標移動排版，PTY 輸出裡一個空白位元組都沒有
/// （實測），所以比對只能在去空白之後做。
const TRUST_YES_OPTION: &str = "Yes,Itrustthisfolder";
/// 目前選中的那一行的游標符號。
const TRUST_CURSOR: char = '❯';

/// 這個畫面停在資料夾信任提示上時，回傳「把選擇移到『我信任這個資料夾』
/// 並確認」要送的按鍵；不是那個畫面、或定位不到就回 `None`。
///
/// `screen` 是 `PtyManager::get_recent_output` 的輸出（ANSI 已剝除）。
///
/// 為什麼不寫死「往下一次再 Enter」：預設選中的是 `No, exit`，而選項順序
/// 隨時可能被 Claude Code 改掉或對調。寫死方向的實作在那一天會反過來按到
/// `No, exit`，把使用者的派工直接殺掉。改成在畫面上同時定位游標行與 Yes
/// 那一行、算相對位移，任何一個找不到就回 `None`——什麼都不送，退回現行
/// 行為（卡住偵測收掉）。風險因此從「可能誤殺派工」降到「可能不生效」。
pub fn trust_prompt_keys(screen: &str) -> Option<Vec<u8>> {
    // 去掉所有空白之後才比對；空行整行丟掉，這樣「游標行」與「Yes 行」
    // 的距離就是實際的按鍵次數，不會被排版用的空行灌水。
    let lines: Vec<String> = screen
        .lines()
        .map(|l| l.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        .filter(|l| !l.is_empty())
        .collect();

    // 選項那一行就只有選項本身（可能前面帶游標符號），不是夾在句子裡的
    // 一段話——後者代表有人剛好提到這句話，不是信任畫面。
    let yes = lines.iter().position(|l| {
        l.trim_start_matches(TRUST_CURSOR) == TRUST_YES_OPTION
    })?;
    let cursor = lines.iter().position(|l| l.starts_with(TRUST_CURSOR))?;

    let mut keys = Vec::new();
    let (step, times) = if yes >= cursor {
        (&b"\x1b[B"[..], yes - cursor)
    } else {
        (&b"\x1b[A"[..], cursor - yes)
    };
    for _ in 0..times {
        keys.extend_from_slice(step);
    }
    keys.extend_from_slice(b"\r");
    Some(keys)
}

/// Max wait for `claude` to finish its cold start (spec measured ~3.7s) before
/// we type the prompt. If it's still noisy at this point we send anyway.
const SETTLE_TIMEOUT_MS: u64 = 30_000;
/// The session is "settled" once it has produced no output for this long.
const SETTLE_QUIET_MS: u64 = 800;
/// Fallback for a configured command that never enters the alternate screen
/// (i.e. isn't a full-screen TUI at all). Quiet for this much longer counts
/// as settled on its own, so a non-TUI `claude_command` still dispatches
/// instead of waiting out `SETTLE_TIMEOUT_MS`.
const NO_TUI_QUIET_MS: u64 = 5_000;
const POLL_MS: u64 = 250;
/// Same as `coordination_ops::DONE_MARKER_WAIT_SECONDS` — how long to wait for
/// `claude` to ring a fresh bell (signalling it finished reading the prompt)
/// before sending the optional done-marker instruction.
const DONE_MARKER_WAIT_SECONDS: u64 = 15;
/// Gap between writing the prompt body and writing the standalone `\r` that
/// submits it (see `run_on_session`). Long enough that the two writes don't
/// land in the same PTY read() burst on the target's side; short enough not
/// to add noticeable dispatch latency.
const SUBMIT_DELAY_MS: u64 = 120;

/// bell/marker counts captured right after the prompt (and optional
/// instruction) were written — the baseline the monitor compares fresh
/// counts against to detect "claude replied to *this* prompt".
#[derive(Debug, Clone, Copy)]
pub struct DispatchResult {
    pub bell_baseline: u64,
    pub marker_baseline: u64,
}

/// Payload for `mcp-coordination-tab-spawned` — the event the frontend
/// already listens for to adopt a backend-spawned session as a visible tab.
/// Same field names as `coordination_ops`'s copy.
#[derive(Serialize, Clone)]
struct TabSpawnedEvent {
    session_id: String,
    command: Option<String>,
}

/// Entering the alternate screen buffer. Claude Code emits this at the very
/// start of the session, as any full-screen TUI does — the codebase already
/// relies on that empirically (see the mode-restore test in
/// `pty/session.rs`). Seeing it is proof the TUI actually launched, which
/// plain quiet is not.
const ALT_SCREEN_ENTER: &[u8] = b"\x1b[?1049h";

/// Wait until `claude` has actually started and then gone quiet, so the
/// prompt is typed into a live REPL rather than into a process that hasn't
/// taken over the terminal yet.
///
/// Quiet alone is NOT a readiness signal: `PtySession::last_output_at` is
/// seeded at spawn time (see that field's doc), so "hasn't printed anything
/// yet" and "printed everything and went idle" are indistinguishable to
/// `ms_since_output`. With a single `claude` cold-starting, its banner
/// usually lands inside `SETTLE_QUIET_MS` and keeps resetting the window
/// until it really is up — but with two of them booting at once (two task
/// board projects dispatching together) the gap exceeds it, we declare
/// "settled" early, and the typed prompt is swallowed. Observed live.
///
/// So: require the alternate-screen sequence *and* quiet. Commands that
/// never enter it (a `claude_command` that isn't a TUI) fall back to the
/// longer `NO_TUI_QUIET_MS` window rather than waiting out the deadline.
///
/// 另外在同一個迴圈裡順便處理資料夾信任提示。沒信任過的目錄會讓 `claude`
/// 停在「Quick safety check」畫面上，而那時替代畫面序列早就送過、畫面也
/// 安靜了——兩個條件都成立，所以不特別處理的話提示詞會被打進對話框裡，
/// 卡片看起來在執行、實際上什麼都沒發生。見 `trust_prompt_keys`。
async fn wait_until_settled(pty: &PtyManager, tab_id: &str) {
    let mut deadline = tokio::time::Instant::now() + Duration::from_millis(SETTLE_TIMEOUT_MS);
    let mut tui_started = false;
    // 只送一次。信任畫面的位元組會留在輸出環裡，接受之後再比對還是會命中，
    // 不 latch 就會一直重送按鍵。
    let mut trust_handled = false;
    loop {
        // Latch it: the raw ring is bounded, and a chatty TUI can push the
        // sequence out of the window it was found in.
        if !tui_started {
            tui_started = pty
                .get_recent_raw(tab_id, 256 * 1024)
                .is_some_and(|b| {
                    b.windows(ALT_SCREEN_ENTER.len()).any(|w| w == ALT_SCREEN_ENTER)
                });
        }

        // 信任提示的檢查一定要排在 settled 判斷之前：那個畫面出現時 TUI
        // 已經啟動、畫面也已經安靜，兩個條件都成立，先判 settled 就會把
        // 提示詞打進信任對話框裡。
        if !trust_handled {
            if let Some(keys) =
                pty.get_recent_output(tab_id, 16 * 1024).as_deref().and_then(trust_prompt_keys)
            {
                let _ = pty.write(tab_id, &keys);
                trust_handled = true;
                // 接受之後 claude 才真正開始啟動，重新給它完整的等待預算。
                deadline = tokio::time::Instant::now() + Duration::from_millis(SETTLE_TIMEOUT_MS);
                tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
                continue;
            }
        }

        let quiet = pty.ms_since_output(tab_id).unwrap_or(u64::MAX);
        let settled = if tui_started { quiet >= SETTLE_QUIET_MS } else { quiet >= NO_TUI_QUIET_MS };
        if settled || tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}

/// Writes `text` then, after `SUBMIT_DELAY_MS`, a standalone `\r` — never
/// `format!("{text}\r")` as one burst. A raw-mode TUI receiving a burst that
/// either contains embedded newlines OR is simply long commonly treats the
/// whole thing as a paste, which fills the input box but does NOT
/// auto-submit even if that same burst happens to end in `\r`. Confirmed
/// live twice: an attachment-bearing task's multi-line prompt (embedded
/// `\n`, from `build_prompt`'s blank line before the attachment note, or
/// just a multi-line task body) sat typed-but-unsubmitted until the
/// 120s-stuck timeout failed it; separately, the done-marker instruction —
/// a single long line with no embedded `\n` at all — sat typed-but-
/// unsubmitted the same way, so burst *size* alone is enough to trigger it,
/// not just embedded newlines. A short delay before the standalone `\r`
/// write keeps the two writes from being coalesced back into one read() on
/// the far end.
async fn write_then_submit(pty: &PtyManager, tab_id: &str, text: &str) -> Result<(), String> {
    pty.write(tab_id, text.as_bytes()).map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_millis(SUBMIT_DELAY_MS)).await;
    pty.write(tab_id, b"\r").map_err(|e| e.to_string())
}

/// Type `prompt` into an already-running session (a `claude` REPL, normally)
/// via `write_then_submit`. Then, if `request_done_marker`, wait for a fresh
/// bell and send `done_marker_instruction` as a further call to
/// `write_then_submit` (same two-writes-not-one reasoning applies to it too
/// — see that function's doc comment). Returns the post-write bell/marker
/// baselines.
pub async fn run_on_session(
    pty: &PtyManager,
    tab_id: &str,
    prompt: &str,
    request_done_marker: bool,
) -> Result<DispatchResult, String> {
    write_then_submit(pty, tab_id, prompt).await?;

    if request_done_marker {
        // Wait for a bell as a best-effort signal that `claude` finished
        // reading the prompt and is ready for more input — but send the
        // instruction regardless of whether one actually arrived. A real
        // `claude` CLI in this environment has been observed to complete a
        // whole task without ever ringing a single bell (see the
        // coordination-done-marker design doc). Gating the instruction send
        // on `became_idle`, as an earlier version of this function did,
        // meant a `claude` session that simply doesn't bell was NEVER even
        // asked to print the completion marker — confirmed live: a task
        // that actually finished (visible in its own tab) still landed on
        // the monitor's 120s-stuck path and got marked failed, because
        // neither signal it's waiting for ever fired. The wait still has
        // value (gives `claude` time to become idle before we write more,
        // same reasoning `coordination_ops::send_input` documents), it's
        // just not treated as a gate on whether to bother at all.
        let bell_before = pty.bell_count(tab_id).unwrap_or(0);
        crate::mcp_server::coordination_ops::wait_for_new_bell(
            pty,
            tab_id,
            bell_before,
            Duration::from_secs(DONE_MARKER_WAIT_SECONDS),
        )
        .await;
        let instr = done_marker_instruction(tab_id);
        write_then_submit(pty, tab_id, &instr).await?;
    }

    Ok(DispatchResult {
        bell_baseline: pty.bell_count(tab_id).unwrap_or(0),
        marker_baseline: pty.marker_count(tab_id).unwrap_or(0),
    })
}

/// Full dispatch: create a visible tab in `project_dir` running
/// `claude_command`, emit the adopt event, wait for it to settle, type the
/// prompt in. Returns `(tab_id, DispatchResult)`.
pub async fn spawn_and_run(
    app: &AppHandle,
    pty: &PtyManager,
    project_dir: &str,
    claude_command: &str,
    session_id: Option<&str>,
    prompt: &str,
    request_done_marker: bool,
    bridge_env: Option<(u16, String)>,
) -> Result<(String, DispatchResult), String> {
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let tab_id = crate::pty::create_with_app(
        pty,
        app.clone(),
        size,
        Some(std::path::PathBuf::from(project_dir)),
        bridge_env,
    )
    .map_err(|e| e.to_string())?;

    // 送進終端機的是接好旗標的版本；下面的事件送的是原本的指令——分頁
    // 標題是 `Agent: <command>`（TerminalApp.tsx），把 UUID 塞進去會讓
    // 每個派工分頁的標題都拖著一串亂碼。
    let launch = launch_command(claude_command, session_id);
    if let Err(e) = pty.write(&tab_id, format!("{launch}\r").as_bytes()) {
        let _ = pty.close(&tab_id);
        return Err(e.to_string());
    }
    if let Err(e) = app.emit(
        "mcp-coordination-tab-spawned",
        TabSpawnedEvent { session_id: tab_id.clone(), command: Some(claude_command.to_string()) },
    ) {
        eprintln!("emit mcp-coordination-tab-spawned failed: {e}");
    }

    wait_until_settled(pty, &tab_id).await;
    let result = run_on_session(pty, &tab_id, prompt, request_done_marker).await?;
    Ok((tab_id, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    mod bridge_env_tests {
        // `ConfigStore`/`TierMapping`/`TaskRow` 已經透過 `use super::*` 從外層
        // （`dispatch.rs` 本體在 Step 3 加的 imports）帶進來，這裡不用重複 import。
        use super::*;
        use crate::tasks::store;
        use sqlx::sqlite::SqlitePoolOptions;

        async fn mem_pool() -> sqlx::SqlitePool {
            let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
            crate::tasks::init_schema(&pool).await.unwrap();
            pool
        }

        async fn task_with_bridge(
            pool: &sqlx::SqlitePool,
            use_bridge: bool,
            tiers: Option<&str>,
        ) -> store::TaskRow {
            let id = store::create_task(pool, "t", "", "/r", true, false).await.unwrap();
            store::set_bridge_config(pool, &id, use_bridge, tiers.map(|s| s.to_string()))
                .await
                .unwrap();
            store::get_task(pool, &id).await.unwrap().unwrap()
        }

        fn temp_config() -> (tempfile::TempDir, ConfigStore) {
            let dir = tempfile::tempdir().unwrap();
            let config = ConfigStore::new_at(dir.path().join("config.toml"));
            (dir, config)
        }

        #[tokio::test]
        async fn no_env_when_task_does_not_want_bridge() {
            let pool = mem_pool().await;
            let task = task_with_bridge(&pool, false, None).await;
            let (_dir, config) = temp_config();

            let env = resolve_bridge_env(&config, &task, Some(8317), Some("tok".to_string()));
            assert_eq!(env, None);
        }

        #[tokio::test]
        async fn no_env_when_bridge_server_not_running() {
            let pool = mem_pool().await;
            let task = task_with_bridge(&pool, true, None).await;
            let (_dir, config) = temp_config();

            let env = resolve_bridge_env(&config, &task, None, None);
            assert_eq!(env, None, "server 沒在跑就不該注入，跟 pty_create 現有行為一致");
        }

        #[tokio::test]
        async fn returns_port_and_token_when_wanted_and_server_running() {
            let pool = mem_pool().await;
            let task = task_with_bridge(&pool, true, None).await;
            let (_dir, config) = temp_config();

            let env = resolve_bridge_env(&config, &task, Some(8317), Some("tok".to_string()));
            assert_eq!(env, Some((8317, "tok".to_string())));
        }

        #[tokio::test]
        async fn snapshot_overwrites_global_tier_config_before_dispatch() {
            let pool = mem_pool().await;
            let snapshot =
                r#"{"opus":{"provider_id":"acct-b","model":"m-b"},"sonnet":null,"haiku":null}"#;
            let task = task_with_bridge(&pool, true, Some(snapshot)).await;
            let (_dir, config) = temp_config();
            config
                .update(|c| {
                    c.claude_bridge.opus =
                        Some(TierMapping { provider_id: "acct-a".into(), model: "m-a".into() });
                })
                .unwrap();

            resolve_bridge_env(&config, &task, Some(8317), Some("tok".to_string()));

            let opus = config.get().claude_bridge.opus.unwrap();
            assert_eq!(opus.provider_id, "acct-b");
            assert_eq!(opus.model, "m-b");
        }

        #[tokio::test]
        async fn null_snapshot_leaves_global_tier_config_untouched() {
            let pool = mem_pool().await;
            let task = task_with_bridge(&pool, true, None).await;
            let (_dir, config) = temp_config();
            config
                .update(|c| {
                    c.claude_bridge.opus =
                        Some(TierMapping { provider_id: "acct-a".into(), model: "m-a".into() });
                })
                .unwrap();

            resolve_bridge_env(&config, &task, Some(8317), Some("tok".to_string()));

            let opus = config.get().claude_bridge.opus.unwrap();
            assert_eq!(opus.provider_id, "acct-a", "bridge_tiers 是 None 時不該覆寫既有設定");
        }
    }

    #[test]
    fn prompt_is_just_the_body_when_there_are_no_attachments() {
        assert_eq!(build_prompt("Do the thing", &[]), "Do the thing");
    }

    #[test]
    fn prompt_appends_one_line_listing_attachment_paths() {
        let p = build_prompt(
            "Refactor per the spec",
            &["/data/tasks/x/attachments/spec.md".into(), "/data/tasks/x/attachments/before.png".into()],
        );
        assert!(p.starts_with("Refactor per the spec"));
        assert!(p.contains("/data/tasks/x/attachments/spec.md"));
        assert!(p.contains("/data/tasks/x/attachments/before.png"));
        // Attachment note is on its own line, after a blank line.
        assert!(p.contains("\n\n"));
    }

    #[test]
    fn blank_body_still_produces_the_attachment_note() {
        let p = build_prompt("", &["/a/b.txt".into()]);
        assert!(p.contains("/a/b.txt"));
    }

    #[test]
    fn a_plain_claude_command_gets_a_session_id() {
        assert!(looks_like_claude("claude"));
    }

    /// 帶旗標的指令也算——使用者常設成 `claude --dangerously-skip-permissions`。
    #[test]
    fn a_claude_command_with_flags_still_counts() {
        assert!(looks_like_claude("claude --dangerously-skip-permissions"));
    }

    /// 絕對路徑與 Windows 的 `.exe` 都算。
    ///
    /// Windows 那一條在 mac 上也必須綠——分隔符是自己切的，不靠平台相依的
    /// `Path::file_name()`（見 `looks_like_claude` 的註解）。這個斷言就是
    /// 那個決定的守門員：改回 `file_name()` 的話它會在 mac 上紅。
    #[test]
    fn an_absolute_path_to_claude_counts() {
        assert!(looks_like_claude("/opt/homebrew/bin/claude"));
        assert!(looks_like_claude(r"C:\tools\claude.exe --verbose"));
    }

    /// 已知限制，寫成測試而不是留在註解裡：路徑含空格時會被
    /// `split_whitespace` 切斷，於是認不出來。
    ///
    /// 這是**刻意**接受的——失敗方向是安全的（那張卡片退回 transcript.txt，
    /// 派工照樣跑），而正確處理要引進 shell 語法的 tokenizer。把它釘成測試
    /// 是為了讓日後有人真的去修時，是主動改掉一條紅線，而不是意外碰到一個
    /// 沒人知道存在的行為。
    #[test]
    fn a_path_with_spaces_is_not_recognised_and_that_is_accepted() {
        assert!(!looks_like_claude(r"C:\Program Files\claude.exe"));
        assert!(!looks_like_claude(r#""C:\Program Files\claude.exe""#));
    }

    /// 這是這個函式存在的理由：非 claude 的指令不能被接上旗標，否則直接
    /// 啟動失敗。名字相近的必須是 false——前綴碰撞（`claude-code`）與
    /// 後綴碰撞（`notclaude`）各要一個案例：
    ///
    /// - 只用 `contains("claude")` 的實作會在 `claude-code` 上壞掉
    /// - 不切分隔符、改用 `ends_with("claude")` 的實作會在 `notclaude`
    ///   上壞掉，而且那個實作能讓其餘每一條斷言都通過（實測過）
    #[test]
    fn a_non_claude_command_does_not_get_one() {
        assert!(!looks_like_claude("codex"));
        assert!(!looks_like_claude("bash -lc 'echo hi'"));
        assert!(!looks_like_claude("claude-code"));
        assert!(!looks_like_claude("notclaude"));
        assert!(!looks_like_claude(r"/usr/local/bin/notclaude"));
        assert!(!looks_like_claude(""));
    }

    #[test]
    fn launch_command_appends_the_flag_when_a_session_id_is_given() {
        assert_eq!(
            launch_command("claude --verbose", Some("3f2a")),
            "claude --verbose --session-id 3f2a"
        );
    }

    #[test]
    fn launch_command_is_the_command_verbatim_without_a_session_id() {
        assert_eq!(launch_command("codex", None), "codex");
    }

    /// 真實抓下來的信任畫面：實跑 `claude` 進一個未信任的資料夾、擷取原始
    /// PTY bytes、過 `strip_ansi` 之後的結果。
    ///
    /// 字與字之間真的沒有空白——Claude Code 的 TUI 用游標移動排版，整份
    /// 輸出裡一個 0x20 都沒有（實測）。所以 `contains("Yes, I trust this
    /// folder")` 這種帶空白的比對永遠不會命中。
    const REAL_TRUST_SCREEN: &str = "\
────────────────────────────────────────────────────────────────────────────────
Accessingworkspace:

/private/tmp/probe/trustprobe-23565

Quicksafetycheck:Isthisaprojectyoucreatedoroneyoutrust?(Likeyour
owncode,awell-knownopensourceproject,orworkfromyourteam).Ifnot,
takeamomenttoreviewwhat'sinthisfolderfirst.

ClaudeCode'llbeabletoread,edit,andexecutefileshere.

Securityguide

❯No,exit
Yes,Itrustthisfolder

Entertoconfirm·Esctocancel
";

    const DOWN: &[u8] = b"\x1b[B";
    const UP: &[u8] = b"\x1b[A";

    /// 真實畫面：游標在 No 上、Yes 在下一行 → 往下一次再 Enter。
    #[test]
    fn moves_down_to_reach_yes_on_the_real_screen() {
        assert_eq!(
            trust_prompt_keys(REAL_TRUST_SCREEN),
            Some([DOWN, b"\r"].concat())
        );
    }

    /// 選項對調（Yes 在上、游標停在下面的 No）→ 必須往**上**。
    /// 寫死「往下一次」的實作會在這裡按到 No, exit，把派工直接殺掉——
    /// 這個測試就是為了擋那件事而存在的。
    #[test]
    fn moves_up_when_the_options_are_swapped() {
        let swapped = REAL_TRUST_SCREEN
            .replace("❯No,exit\nYes,Itrustthisfolder", "Yes,Itrustthisfolder\n❯No,exit");
        assert_eq!(trust_prompt_keys(&swapped), Some([UP, b"\r"].concat()));
    }

    /// 游標已經停在 Yes 上 → 只送 Enter，一次都不要動。
    #[test]
    fn just_confirms_when_the_cursor_is_already_on_yes() {
        let already = REAL_TRUST_SCREEN
            .replace("❯No,exit\nYes,Itrustthisfolder", "No,exit\n❯Yes,Itrustthisfolder");
        assert_eq!(trust_prompt_keys(&already), Some(b"\r".to_vec()));
    }

    /// 不是信任畫面就什麼都不送。
    #[test]
    fn sends_nothing_on_an_ordinary_screen() {
        assert_eq!(trust_prompt_keys("$ ls\nsrc  target\n"), None);
        assert_eq!(trust_prompt_keys(""), None);
    }

    /// 認得出 Yes 那一行、卻找不到游標（UI 改版換掉了 `❯`）→ 什麼都不送。
    /// 猜一個方向亂按有可能按到 No, exit；不送最壞只是退回現行行為
    /// （卡住偵測收掉），永遠不會弄壞東西。
    #[test]
    fn sends_nothing_when_the_cursor_cannot_be_located() {
        let no_cursor = REAL_TRUST_SCREEN.replace('❯', " ");
        assert_eq!(trust_prompt_keys(&no_cursor), None);
    }

    /// 只是有人在聊天裡提到這句話，不該被當成信任畫面——畫面上必須同時
    /// 有游標與那一整行選項。
    #[test]
    fn does_not_fire_on_a_mention_of_the_phrase_in_prose() {
        let prose = "❯somethingelse\nItoldyouYes,Itrustthisfolderisthewording\n";
        assert_eq!(trust_prompt_keys(prose), None);
    }

    /// 去空白這一步是**防禦性**的，而上面每個 fixture 都忠實反映「今天的
    /// Claude Code 一個空白都不送」——所以把 filter 整段拿掉，那些測試
    /// 照樣全綠（實測過）。這條就是補那個洞。
    ///
    /// 為什麼值得補：拿掉 filter 是個很合理的未來修改。後人看到程式碼裡
    /// 的 filter 配上「這個畫面沒有空白」的註解，很自然會想「那還 filter
    /// 幹嘛」而順手刪掉——然後哪天 Claude Code 的 TUI 改成用空白排版，
    /// 信任畫面就再也認不出來，而且是安靜地失效。
    ///
    /// 這裡刻意用一份**帶空白**的畫面：它不是實測到的樣子，而是「萬一
    /// 上游改了排版方式」的樣子。認得出來才是正確行為。
    #[test]
    fn still_recognises_the_screen_if_claude_code_ever_renders_with_spaces() {
        let with_spaces = "\
Quick safety check: Is this a project you created or one you trust?

❯ No, exit
  Yes, I trust this folder

Enter to confirm · Esc to cancel
";
        assert_eq!(
            trust_prompt_keys(with_spaces),
            Some([DOWN, b"\r"].concat()),
            "排版改用空白之後就認不出來了——去空白那一步是不是被拿掉了？"
        );
    }

    use crate::pty::PtyManager;

    fn settle_size() -> PtySize {
        PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 }
    }

    /// A session that has printed its shell prompt and then gone quiet is
    /// exactly what a slow `claude` cold start looks like from the outside —
    /// and `ms_since_output` cannot tell it apart from "printed everything
    /// and is now idle", because `last_output_at` is seeded at spawn time
    /// (see that field's doc in pty/session.rs). The old implementation
    /// settled on quiet alone and typed the prompt into a REPL that wasn't
    /// up yet, so the input was swallowed.
    ///
    /// Observed live: with two `claude` processes cold-starting at once
    /// (two task-board projects dispatching together), the gap before the
    /// banner appears exceeds SETTLE_QUIET_MS and the prompt is lost.
    ///
    /// Proves it by racing against a timeout well past SETTLE_QUIET_MS —
    /// "still running after 2s" is an observed fact, not a guess about
    /// timing.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn does_not_settle_while_the_tui_has_not_started_yet() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(settle_size(), |_| {}).unwrap();
        // Let the shell draw its prompt and fall quiet — no TUI ever starts.
        tokio::time::sleep(Duration::from_millis(SETTLE_QUIET_MS + 400)).await;

        let fut = wait_until_settled(&pty, &tab);
        tokio::pin!(fut);
        let still_waiting = tokio::time::timeout(Duration::from_millis(2_000), &mut fut)
            .await
            .is_err();
        assert!(
            still_waiting,
            "settled while nothing had started — the prompt would be typed into a REPL that isn't up"
        );
    }

    /// The other half: once the TUI has actually entered the alternate
    /// screen and gone quiet, we must proceed promptly — waiting the full
    /// SETTLE_TIMEOUT_MS would add 30s to every dispatch.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn settles_once_the_tui_is_up_and_quiet() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(settle_size(), |_| {}).unwrap();
        pty.write(&tab, b"printf '\\033[?1049h'\n").unwrap();

        let settled = tokio::time::timeout(
            Duration::from_millis(NO_TUI_QUIET_MS),
            wait_until_settled(&pty, &tab),
        )
        .await;
        assert!(settled.is_ok(), "did not settle even though the TUI was up and quiet");
    }

    /// A configured `claude_command` that is not a full-screen TUI never
    /// emits the alternate-screen sequence. It must still dispatch — after
    /// the longer no-TUI quiet window, not after the 30s hard deadline.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn a_non_tui_command_still_settles_on_the_longer_quiet_window() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(settle_size(), |_| {}).unwrap();

        let settled = tokio::time::timeout(
            Duration::from_millis(NO_TUI_QUIET_MS + 2_000),
            wait_until_settled(&pty, &tab),
        )
        .await;
        assert!(settled.is_ok(), "a non-TUI command never settled");
    }

    /// 停在信任畫面時不可以判定 settled——那樣會把提示詞打進信任對話框裡。
    ///
    /// 手法照抄同檔案的 `run_on_session_sends_a_multiline_prompt_verbatim_
    /// then_a_standalone_cr`：把 pty 切進 raw 模式關掉回音，再 `od` 讀出
    /// 我們實際寫進去的位元組。不看「回音」是因為 canonical 模式的終端機會
    /// 把 ESC 顯示成 `^[`，那樣斷言驗到的是終端機的顯示規則，不是我們寫了
    /// 什麼——見 feedback「沒有失敗訊號不等於正確」。
    ///
    /// 畫面先印替代畫面序列讓 `tui_started` latch 起來（模擬 claude 已經
    /// 啟動），再印真實信任畫面的三行。
    ///
    /// `❯` 直接寫字面的 UTF-8 字元、用 `%s` 印，**不要**寫成 `\342\235\257`
    /// 配 `%b`：`printf %b` 的八進位解碼是殼相依的。zsh 的內建 printf
    /// 不解碼（實測 `zsh -c "printf '%b\n' '\342\235\257X'"` 吐出的是字面
    /// 的 `\342\235\257X`），而 `pty/shell.rs` 的 `unix_default_shell()`
    /// 讀的是 `$SHELL`——macOS 從 Catalina 起預設就是 zsh。所以那個寫法會
    /// 讓游標行永遠定位不到，測試在多數 mac 上恆紅，而且是「因為畫面根本
    /// 沒印對」而紅，不是因為實作有問題。字面字元三種殼都會原樣輸出。
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn accepts_the_trust_prompt_before_declaring_the_session_settled() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(settle_size(), |_| {}).unwrap();

        // `'MARK''READY'` 用串接寫，這樣被回音出來的指令本身不含連續的
        // marker（同上，照抄那個測試的手法）。N = 4：`\x1b[B` 加 `\r`。
        #[cfg(not(windows))]
        pty.write(
            &tab,
            b"stty raw -echo; printf '\\033[?1049h'; \
              printf '%s\\n' 'Quicksafetycheck:' '\xe2\x9d\xafNo,exit' 'Yes,Itrustthisfolder'; \
              printf 'MARK''READY'; od -An -tx1 -N 4\n",
        )
        .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let out = pty.get_recent_output(&tab, 16 * 1024).unwrap_or_default();
            if out.contains("MARKREADY") {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "信任畫面沒有印出來：{out}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let _ = tokio::time::timeout(
            Duration::from_millis(NO_TUI_QUIET_MS + 2_000),
            wait_until_settled(&pty, &tab),
        )
        .await;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let hex: Vec<String> = loop {
            let out = pty.get_recent_output(&tab, 16 * 1024).unwrap_or_default();
            let after = out.split("MARKREADY").nth(1).unwrap_or("").to_string();
            let hex: Vec<String> = after
                .split_whitespace()
                .filter(|t| t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_string)
                .collect();
            if hex.len() >= 4 {
                break hex;
            }
            assert!(tokio::time::Instant::now() < deadline, "沒有送出任何按鍵：{out}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        // ESC [ B CR — 往下一格（Yes 在 No 下面）再確認。
        assert_eq!(&hex[..4], ["1b", "5b", "42", "0d"], "送出的按鍵不對：{hex:?}");
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn run_on_session_types_the_prompt_into_an_existing_session() {
        let pty = PtyManager::new();
        // A plain shell stands in for `claude` — it echoes typed lines back.
        let tab_id = pty
            .create_with_callback(
                portable_pty::PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 },
                |_| {},
            )
            .unwrap();

        let res = run_on_session(&pty, &tab_id, "echo TASKBOARD_PROMPT_MARKER", false)
            .await
            .unwrap();
        let _ = res.marker_baseline; // field exists / no panic

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let out = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
            if out.contains("TASKBOARD_PROMPT_MARKER") {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "prompt never echoed: {out}");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Regression test for a real bug: a multi-line prompt (e.g. `build_prompt`
    /// with attachments, or just a multi-line task body) sat typed-but-never-
    /// submitted in Claude Code's input box, because the old code sent the
    /// whole thing — embedded newlines and the submitting `\r` — as one single
    /// `pty.write`. Mirrors `coordination_ops::send_input_terminates_the_line_
    /// with_cr_not_lf`'s technique: put the pty in raw mode and dump the exact
    /// bytes that arrive, rather than trusting a canonical-mode shell's
    /// tolerance for either terminator. This proves our own write pipeline
    /// delivers the multi-line body byte-for-byte (embedded LF preserved, not
    /// converted or dropped) followed by a real standalone CR — it cannot
    /// prove Claude Code's specific TUI then submits on it (that needs a live
    /// check against the real binary), but it locks in the one thing this
    /// commit actually controls: two real, distinct writes with the right
    /// bytes, not one burst that smuggled the `\r` inside embedded-newline
    /// content.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn run_on_session_sends_a_multiline_prompt_verbatim_then_a_standalone_cr() {
        let pty = PtyManager::new();
        let tab_id = pty
            .create_with_callback(
                portable_pty::PtySize { rows: 24, cols: 300, pixel_width: 0, pixel_height: 0 },
                |_| {},
            )
            .unwrap();

        // Same setup as the coordination_ops.rs test this mirrors: flip the
        // pty into raw mode, print a marker (built via concatenation so the
        // *echoed* setup command itself never contains the contiguous
        // marker), then block reading exactly N raw bytes and dump them as
        // hex. N = len("line one\nline two") + 1 (the trailing CR).
        let prompt = "line one\nline two";
        let expect_len = prompt.len() + 1;
        #[cfg(not(windows))]
        pty.write(
            &tab_id,
            format!("stty raw -echo; printf 'MARK''READY'; od -An -tx1 -N {expect_len}\n").as_bytes(),
        )
        .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
            if output.contains("MARKREADY") {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "byte-dump setup never completed: {output}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        run_on_session(&pty, &tab_id, prompt, false).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let hex_bytes: Vec<String> = loop {
            let output = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
            let after_marker = output.split("MARKREADY").nth(1).unwrap_or("").to_string();
            let hex_bytes: Vec<String> = after_marker
                .split_whitespace()
                .filter(|tok| tok.len() == 2 && tok.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_string)
                .collect();
            if hex_bytes.len() >= expect_len {
                break hex_bytes;
            }
            assert!(tokio::time::Instant::now() < deadline, "{expect_len}-byte hex dump never appeared: {output}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        let expected: Vec<String> = prompt
            .bytes()
            .chain(std::iter::once(b'\r'))
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            &hex_bytes[..expect_len],
            expected.as_slice(),
            "expected the multi-line prompt's exact bytes (embedded 0a preserved) followed by a standalone 0d — got: {hex_bytes:?}"
        );
    }

    /// Regression test for a real bug found live: a task that genuinely
    /// finished (visible completing in its own tab) still got marked failed
    /// by the monitor's 120s-stuck path, because the done-marker instruction
    /// was never sent — the old code only sent it after observing a fresh
    /// bell, and a real `claude` CLI has been observed to complete a whole
    /// turn without ever ringing one. No bell is injected here at all;
    /// the instruction (which mentions the tab_id) must still be sent once
    /// the wait elapses. Runs for real at ~DONE_MARKER_WAIT_SECONDS (15s),
    /// same as the sibling test this mirrors in coordination_ops.rs.
    /// `tauri::test::{mock_builder, mock_context, noop_assets}` (which would
    /// let this test drive `spawn_and_run` directly, the thing that actually
    /// gains a new parameter in this change) needs the `tauri` crate's `test`
    /// feature, which isn't enabled on this project's `tauri` dependency
    /// (`features = []` in Cargo.toml) — confirmed via a real compile
    /// attempt (`error[E0432]: ... the item is gated here`, pointing at
    /// `#[cfg_attr(docsrs, doc(cfg(feature = "test")))] pub mod test;`).
    /// Enabling it would mean an extra Cargo.toml dependency-feature change
    /// beyond this task's two listed files, so this exercises the same
    /// underlying mechanism one level down, against `run_on_session`
    /// (unchanged by this task, already parameterized) instead — it pairs
    /// with `run_on_session_sends_the_done_marker_instruction_even_when_the_
    /// target_never_bells` above, which covers the `true` side of the same
    /// parameter. `spawn_and_run` forwarding its own new parameter straight
    /// into `run_on_session` is a one-line, easily eyeballed change in the
    /// implementation below.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn run_on_session_does_not_send_the_done_marker_instruction_when_not_requested() {
        let pty = PtyManager::new();
        let tab_id = pty
            .create_with_callback(
                portable_pty::PtySize { rows: 24, cols: 300, pixel_width: 0, pixel_height: 0 },
                |_| {},
            )
            .unwrap();

        run_on_session(&pty, &tab_id, "echo hi", false).await.unwrap();

        tokio::time::sleep(Duration::from_secs(2)).await;
        let out = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
        assert!(!out.contains(&tab_id), "done-marker instruction was sent despite request_done_marker=false: {out}");
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn run_on_session_sends_the_done_marker_instruction_even_when_the_target_never_bells() {
        let pty = PtyManager::new();
        let tab_id = pty
            .create_with_callback(
                portable_pty::PtySize { rows: 24, cols: 300, pixel_width: 0, pixel_height: 0 },
                |_| {},
            )
            .unwrap();

        run_on_session(&pty, &tab_id, "echo hi", true).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
            if output.contains(&tab_id) {
                break; // the instruction mentions its own tab_id
            }
            assert!(tokio::time::Instant::now() < deadline, "done-marker instruction was never sent: {output}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
