//! 一般終端機分頁裡手動跑的 Claude：subagent 協作側欄的後端。
//!
//! 流程：`pty_create` → `prepare` 為這個 session 寫一份 hooks 設定檔、把路徑放進
//! `AITERM_CLAUDE_SETTINGS`（shell 整合腳本的 `claude` 函式會補上 `--settings`）→
//! Claude 的 HTTP hook 打到本機接收器 → `Tree` 狀態機 → emit 完整快照給前端。

pub mod commands;
pub mod tree;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use axum::extract::{Path as AxPath, State};
use axum::routing::post;
use parking_lot::Mutex;
use serde_json::Value;

use tree::{Kind, Node, Tree};

/// 子行程環境變數：該 session 的 hooks 設定檔路徑。
pub const SETTINGS_ENV: &str = "AITERM_CLAUDE_SETTINGS";
/// 殘留（app 當掉沒清到）的設定檔多久後清掉。
const STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);

/// 狀態變更時的通知：(session id, 完整快照)。正式版 emit Tauri 事件，測試收進 Vec。
pub type Emitter = Arc<dyn Fn(&str, &[Node]) + Send + Sync>;

/// 交給 `claude --settings` 的設定檔內容；`base` 是 `http://127.0.0.1:<port>/activity/<secret>/<sid>`。
pub fn settings_json(base: &str) -> String {
    let hook = |path: &str| serde_json::json!([{ "hooks": [{ "type": "http", "url": format!("{base}/{path}") }] }]);
    serde_json::json!({
        "hooks": {
            "PreToolUse": hook("pre"),
            "PostToolUse": hook("post"),
            "SubagentStart": hook("sub-start"),
            "SubagentStop": hook("sub-stop"),
            "SessionEnd": hook("session-end"),
        }
    })
    .to_string()
}

fn purge_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > STALE_AFTER);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct HookServer {
    port: u16,
    secret: String,
}

struct HandlerState {
    activity: Arc<ClaudeActivity>,
    secret: String,
}

pub struct ClaudeActivity {
    dir: PathBuf,
    emit: Emitter,
    sessions: Mutex<HashMap<String, Tree>>,
    /// 第一次 `prepare` 時才啟動接收器；啟動失敗＝`None`（分頁照開，只是沒有側欄）。
    server: OnceLock<Option<HookServer>>,
}

impl ClaudeActivity {
    pub fn new(dir: PathBuf, emit: Emitter) -> Arc<Self> {
        purge_stale(&dir);
        Arc::new(Self {
            dir,
            emit,
            sessions: Mutex::new(HashMap::new()),
            server: OnceLock::new(),
        })
    }

    /// 為新 PTY 準備：登記 session、寫設定檔，回傳要加進子行程的環境變數。
    /// 任何一步失敗都回空（不注入比注入一個壞掉的設定檔更好除錯）。
    pub fn prepare(self: &Arc<Self>, session_id: &str) -> Vec<(String, String)> {
        let Some(server) = self.server.get_or_init(|| self.start_server()).as_ref() else {
            return Vec::new();
        };
        let base = format!("http://127.0.0.1:{}/activity/{}/{}", server.port, server.secret, session_id);
        let path = self.dir.join(format!("{session_id}.json"));
        let written = std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(&path, settings_json(&base)));
        if let Err(e) = written {
            eprintln!("claude activity: write settings failed: {e}");
            return Vec::new();
        }
        self.sessions.lock().insert(session_id.to_owned(), Tree::default());
        vec![(SETTINGS_ENV.to_owned(), path.to_string_lossy().into_owned())]
    }

    /// 套用一個 hook 事件；沒登記過的 session 一律丟棄（本機其他程式亂打也不會污染）。
    pub fn ingest(&self, session_id: &str, kind: Kind, body: &Value, now_ms: u64) {
        let snapshot = {
            let mut map = self.sessions.lock();
            let Some(tree) = map.get_mut(session_id) else { return };
            if !tree.apply(kind, body, now_ms) {
                return;
            }
            tree.nodes().to_vec()
        };
        (self.emit)(session_id, &snapshot);
    }

    pub fn snapshot(&self, session_id: &str) -> Vec<Node> {
        self.sessions.lock().get(session_id).map(|t| t.nodes().to_vec()).unwrap_or_default()
    }

    /// PTY 關閉：丟掉狀態並刪設定檔。只處理登記過的 id，所以不能拿路徑字元亂刪檔。
    pub fn forget(&self, session_id: &str) {
        if self.sessions.lock().remove(session_id).is_some() {
            let _ = std::fs::remove_file(self.dir.join(format!("{session_id}.json")));
        }
    }

    fn start_server(self: &Arc<Self>) -> Option<HookServer> {
        // 同步綁埠（`prepare` 從同步的 Tauri 指令呼叫，此時不在 async runtime 裡），
        // 之後交給 Tauri 的 runtime 去 serve。
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
        std_listener.set_nonblocking(true).ok()?;
        let port = std_listener.local_addr().ok()?.port();
        let secret = uuid::Uuid::new_v4().simple().to_string();
        let state = Arc::new(HandlerState { activity: self.clone(), secret: secret.clone() });
        let app = axum::Router::new()
            .route("/activity/{secret}/{sid}/{kind}", post(handle))
            .with_state(state);
        tauri::async_runtime::spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(std_listener) else { return };
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("claude activity server stopped: {e}");
            }
        });
        Some(HookServer { port, secret })
    }
}

async fn handle(
    State(st): State<Arc<HandlerState>>,
    AxPath((secret, sid, kind)): AxPath<(String, String, String)>,
    body: axum::body::Bytes,
) -> &'static str {
    // 只接受自己發出去的網址；本機其他程式猜不到密鑰。
    if secret == st.secret {
        if let Some(kind) = Kind::parse(&kind) {
            let json: Value = serde_json::from_slice(&body).unwrap_or_default();
            st.activity.ingest(&sid, kind, &json, now_ms());
        }
    }
    // Claude Code 會把回應當成 hook 輸出解析；空物件 = 沒有意見。
    "{}"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aiterm-claude-activity-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    type Seen = Arc<StdMutex<Vec<(String, Vec<Node>)>>>;
    fn activity(tag: &str) -> (Arc<ClaudeActivity>, Seen) {
        let seen: Seen = Arc::default();
        let sink = seen.clone();
        let emit: Emitter = Arc::new(move |sid, nodes| sink.lock().unwrap().push((sid.to_owned(), nodes.to_vec())));
        (ClaudeActivity::new(temp_dir(tag), emit), seen)
    }

    #[test]
    fn settings_json_points_every_hook_at_its_own_path() {
        let v: serde_json::Value = serde_json::from_str(&settings_json("http://h/activity/s/sid")).unwrap();
        for (event, path) in [
            ("PreToolUse", "pre"),
            ("PostToolUse", "post"),
            ("SubagentStart", "sub-start"),
            ("SubagentStop", "sub-stop"),
            ("SessionEnd", "session-end"),
        ] {
            let h = &v["hooks"][event][0]["hooks"][0];
            assert_eq!(h["type"], "http", "{event}");
            assert_eq!(h["url"], format!("http://h/activity/s/sid/{path}"), "{event}");
        }
    }

    #[test]
    fn prepare_writes_a_settings_file_and_returns_its_path_in_env() {
        let (a, _) = activity("prepare");
        let env = a.prepare("sess-1");
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].0, SETTINGS_ENV);
        let text = std::fs::read_to_string(&env[0].1).unwrap();
        assert!(text.contains("/activity/"), "{text}");
        assert!(text.contains("/sess-1/pre"), "{text}");
    }

    #[test]
    fn ingest_updates_the_snapshot_and_emits_once_per_change() {
        let (a, seen) = activity("ingest");
        a.prepare("s1");
        let start = json!({ "agent_id": "a1", "agent_type": "general-purpose" });
        a.ingest("s1", Kind::SubStart, &start, 1);
        a.ingest("s1", Kind::SubStart, &start, 2); // 重複：不再通知
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "s1");
        assert_eq!(a.snapshot("s1").len(), 1);
    }

    #[test]
    fn events_for_unregistered_sessions_are_dropped() {
        let (a, seen) = activity("unknown");
        a.ingest("ghost", Kind::SubStart, &json!({ "agent_id": "a1" }), 1);
        assert!(seen.lock().unwrap().is_empty());
        assert!(a.snapshot("ghost").is_empty());
    }

    #[test]
    fn forget_drops_state_and_deletes_the_settings_file() {
        let (a, _) = activity("forget");
        let env = a.prepare("s1");
        a.forget("s1");
        assert!(!std::path::Path::new(&env[0].1).exists());
        a.ingest("s1", Kind::SubStart, &json!({ "agent_id": "a1" }), 1);
        assert!(a.snapshot("s1").is_empty(), "忘記之後的事件不再被接受");
    }

    #[test]
    fn forget_ignores_ids_that_were_never_registered() {
        let (a, _) = activity("forget-unknown");
        let victim = a.dir.join("../victim.json");
        std::fs::write(&victim, "x").ok();
        a.forget("../victim");
        assert!(victim.exists(), "沒登記過的 id 不能拿來刪檔");
        std::fs::remove_file(&victim).ok();
    }

    #[test]
    fn stale_settings_files_are_purged_on_startup_but_fresh_ones_stay() {
        let dir = temp_dir("purge");
        let old = dir.join("old.json");
        let fresh = dir.join("fresh.json");
        std::fs::write(&old, "{}").unwrap();
        std::fs::write(&fresh, "{}").unwrap();
        let f = std::fs::File::options().write(true).open(&old).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 3600)).unwrap();
        purge_stale(&dir);
        assert!(!old.exists());
        assert!(fresh.exists());
    }

    /// 真的走一次 HTTP：驗證密鑰、session id、事件種類都從網址解出來，
    /// 而且錯誤的密鑰不會被接受。
    #[tokio::test]
    async fn the_http_receiver_accepts_the_real_urls_and_rejects_a_wrong_secret() {
        let (a, seen) = activity("http");
        let env = a.prepare("s1");
        let settings: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&env[0].1).unwrap()).unwrap();
        let url = settings["hooks"]["SubagentStart"][0]["hooks"][0]["url"].as_str().unwrap().to_owned();
        let client = reqwest::Client::new();

        let bad = url.replace("/activity/", "/activity/x");
        client.post(&bad).json(&json!({ "agent_id": "a1" })).send().await.unwrap();
        assert!(seen.lock().unwrap().is_empty(), "密鑰錯誤不能被接受");

        let resp = client
            .post(&url)
            .json(&json!({ "agent_id": "a1", "agent_type": "general-purpose" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.text().await.unwrap(), "{}", "Claude 會把回應當 hook 輸出解析，空物件＝沒意見");
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(a.snapshot("s1")[0].id, "a1");
    }
}
