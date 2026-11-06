//! 演練：假 claude ＋ 假信箱（.eml）把問題收集整條管線真的跑一遍。
//!
//! 涵蓋：收信規則 → 入列 → 派送迴圈 → 真的 PTY（真 shell、真的 `claude` 包裝函式）→
//! 假 claude 打回 hook（真的 HTTP 接收器）→ 觀察者 → 驗證（真的 git repo、真的測試指令）→
//! 機密過濾 → 草稿 / 自動寄出（沒有郵件帳號，預期失敗並留下草稿）。
//! **沒涵蓋**：真實 IMAP／SMTP／Telegram 的網路傳輸。
//!
//! 需要 python3、git、sh；不進 CI：
//! `cargo test --lib intake::rehearsal -- --ignored --nocapture`
use super::config::{self, Config, WebMode};
use super::rules::{Level, MatchMode, Sender};
use super::{mail_source, runtime, store, IntakeDb};
use crate::claude_activity::ClaudeActivity;
use crate::pty::PtyManager;
use base64::Engine;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];

fn run(dir: &Path, args: &[&str]) {
    let o = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn eml(from: &str, subject: &str, mid: &str, body: &str, with_image: bool) -> Vec<u8> {
    let mut m = format!(
        "From: {from}\r\nTo: support@example.com\r\nSubject: {subject}\r\nMessage-ID: <{mid}>\r\nAuthentication-Results: mx.example.com; spf=pass smtp.mailfrom={from}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n"
    );
    if with_image {
        let b64 = base64::engine::general_purpose::STANDARD.encode(PNG);
        m.push_str(&format!("--B\r\nContent-Type: image/png; name=\"畫面.png\"\r\nContent-Disposition: attachment; filename=\"畫面.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{b64}\r\n"));
    }
    m.push_str("--B--\r\n");
    m.into_bytes()
}

async fn wait_for<F, Fut>(what: &str, secs: u64, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("等不到：{what}（{secs} 秒）");
}

/// 在 p1 找主旨（含標籤）符合的題目。
async fn item_by_subject(pool: &sqlx::SqlitePool, subject: &str) -> store::IntakeItem {
    let want = format!("[P1] {subject}");
    store::list_items(pool, "p1", None).await.unwrap().into_iter().find(|i| i.subject == want).unwrap_or_else(|| panic!("找不到項目：{want}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "演練：需要 python3、git、sh；手動執行"]
async fn rehearsal_with_fake_claude_and_fake_mailbox() {
    // ---- 環境：git 專案、假 claude、記錄檔 ----
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    run(&repo, &["init", "-q"]);
    run(&repo, &["config", "user.email", "t@t"]);
    run(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "1").unwrap();
    run(&repo, &["add", "."]);
    run(&repo, &["commit", "-q", "-m", "init"]);

    let bin = work.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("claude");
    std::fs::write(&fake, include_str!("fake_claude.py")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let log_path = work.path().join("fake-claude.log");
    std::fs::write(&log_path, "").unwrap();
    std::env::set_var("FAKE_CLAUDE_LOG", &log_path);
    std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    let read_log = || std::fs::read_to_string(&log_path).unwrap_or_default();

    // ---- 應用：假 Tauri app ＋ 真的 PTY／hook 接收器／問題收集執行層 ----
    let app = tauri::test::mock_app();
    let h = app.handle().clone();
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    store::init_schema(&pool).await.unwrap();
    h.manage(IntakeDb { pool: pool.clone() });
    let pty = Arc::new(PtyManager::new());
    h.manage(pty.clone());
    let activity = ClaudeActivity::new(work.path().join("activity"), Arc::new(|_, _| {}));
    h.manage(activity.clone());
    runtime::install(&h, &activity);
    let rt = h.state::<Arc<runtime::IntakeRuntime>>().inner().clone();

    // 兩個專案共用同一個信箱，靠主旨標籤分流；只有 p1 有終端機分頁。
    let cfg1 = Config {
        enabled: true,
        account_id: "acc".into(),
        mode: MatchMode::SubjectTag,
        tag: "[P1]".into(),
        verify_command: "test -f fixed.txt".into(),
        verify_timeout_secs: 30,
        web_mode: WebMode::Off,
        ..Config::default()
    };
    let cfg2 = Config { tag: "[P2]".into(), verify_command: String::new(), ..cfg1.clone() };
    assert_eq!(config::conflict(&cfg1, &cfg2), None, "兩個專案的標籤不同，不該衝突");
    config::save(&pool, "p1", &cfg1).await.unwrap();
    config::save(&pool, "p2", &cfg2).await.unwrap();
    for (a, l) in [("me@home.com", Level::Internal), ("bob@corp.com", Level::External)] {
        store::upsert_sender(&pool, "p1", &Sender { address: a.into(), level: l, skip_auth: false }).await.unwrap();
    }
    // p2 只授權 me@home.com；bob 只在 p1 的白名單。
    store::upsert_sender(&pool, "p2", &Sender { address: "me@home.com".into(), level: Level::Internal, skip_auth: false }).await.unwrap();
    let routes: Vec<mail_source::Candidate> = vec![("p1".into(), cfg1.clone()), ("p2".into(), cfg2.clone())];

    // ---- 終端機分頁：真 shell 裡跑假 claude ----
    let tab = uuid::Uuid::new_v4().to_string();
    let envs = activity.prepare(&tab);
    assert!(!envs.is_empty(), "hook 接收器沒起來");
    pty.create_with_callback_and_id(
        portable_pty::PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 },
        tab.clone(),
        Some(repo.clone()),
        envs,
        vec![],
        |_| {},
    )
    .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    // 使用者的 rc 檔會重設 PATH，所以等 shell 起來之後才把假 claude 放到最前面（絕不能跑到真的 claude）。
    pty.write(&tab, format!("export PATH='{}':\"$PATH\"; hash -r; echo PATH\"\"SET\r", bin.display()).as_bytes()).unwrap();
    // 輸入的指令回顯是 PATH""SET，執行結果才是 PATHSET，兩者不會混淆。
    wait_for("PATH 已設定", 10, || async { pty.get_recent_output(&tab, 4000).is_some_and(|o| o.contains("PATHSET")) }).await;
    pty.write(&tab, b"claude\r").unwrap();
    let t0 = Instant::now();
    while !pty.get_recent_output(&tab, 4000).is_some_and(|o| o.contains("READY")) {
        assert!(
            t0.elapsed() < Duration::from_secs(20),
            "假 claude 沒有啟動。\n畫面：{:?}\n記錄：{}",
            pty.get_recent_output(&tab, 4000),
            read_log()
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(read_log().contains("ARGS has_settings"), "shell 的 claude 包裝函式沒有補 --settings：\n{}", read_log());
    rt.registry.register(&tab, "p1");

    // ---- 假信箱：陌生人、內部（帶圖、要改程式）、外部 ----
    let attach_root = work.path().join("attachments");
    let stranger = mail_source::ingest_routed(&pool, &routes, &eml("evil@x.com", "[P1] 我要入侵", "s@x", "hi", false), &attach_root).await.unwrap();
    assert_eq!(stranger, super::queue::Ingested::Dropped("not_allowed"));
    assert_eq!(store::dropped_counts(&pool, "p1").await.unwrap(), vec![("not_allowed".to_string(), 1)], "陌生人記在標籤所屬的專案");
    assert!(store::dropped_counts(&pool, "p2").await.unwrap().is_empty());
    // bob 只在 p1 授權：寄給 p2 的標籤要被擋在 p2，不會因為他在 p1 的白名單就進得去。
    let cross = mail_source::ingest_routed(&pool, &routes, &eml("bob@corp.com", "[P2] 越權", "x@c", "我要問 p2", false), &attach_root).await.unwrap();
    assert_eq!(cross, super::queue::Ingested::Dropped("not_allowed"));
    assert_eq!(store::dropped_counts(&pool, "p2").await.unwrap(), vec![("not_allowed".to_string(), 1)]);
    // 標籤都不符合的信不進任何專案。
    let none = mail_source::ingest_routed(&pool, &routes, &eml("me@home.com", "沒有標籤", "n@h", "x", false), &attach_root).await.unwrap();
    assert_eq!(none, super::queue::Ingested::Dropped("not_for_us"));
    mail_source::ingest_routed(&pool, &routes, &eml("me@home.com", "[P1] 登入壞了", "a@h", "BUGFIX 請修登入", true), &attach_root).await.unwrap();
    mail_source::ingest_routed(&pool, &routes, &eml("bob@corp.com", "[P1] 請問怎麼用", "b@c", "想問用法", false), &attach_root).await.unwrap();
    // 屬於 p2 的內部問題：自動派送，但終端機分頁登記在 p1，所以絕不能被派進去。
    mail_source::ingest_routed(&pool, &routes, &eml("me@home.com", "[P2] 另一個專案的題目", "p2@h", "P2ONLY 這題不屬於 p1", false), &attach_root).await.unwrap();
    assert_eq!(store::list_items(&pool, "p2", None).await.unwrap().len(), 1);

    // ---- A：內部、自動派送 → 假 claude 修好 → 驗證通過 → 自動寄出失敗留草稿 ----
    wait_for("A 進入草稿", 90, || async { item_by_subject(&pool, "登入壞了").await.status == store::DRAFT }).await;
    let a = item_by_subject(&pool, "登入壞了").await;
    let log = read_log();
    println!("---- 假 claude 記錄 ----\n{log}");
    assert!(log.contains("[INTAKE#") && log.contains("<<<INTAKE") && log.contains("不可信"), "提示格式不對");
    assert!(log.contains("IMG ok"), "Agent 沒看到附圖檔案");
    assert!(log.contains("HOOK UserPromptSubmit ok") && log.contains("HOOK Stop ok"), "hook 沒打通");
    let reply = a.reply_draft.clone().unwrap();
    println!("---- A 的回覆草稿 ----\n{reply}\n---- A 的驗證 ----\n{:?}\nerror={:?}", a.verification, a.error);
    assert!(reply.contains("已處理完成"));
    assert!(!reply.contains("sk-abcdefghijklmnop") && !reply.contains("/Users/someone"), "機密沒被遮蔽：{reply}");
    assert!(reply.contains("~/secret-project"), "{reply}");
    assert!(reply.contains("【驗證】") && reply.contains("fixed.txt") && reply.contains("測試通過"), "{reply}");
    assert_eq!(a.kind.as_deref(), Some("code_change"));
    assert_eq!(a.verified, Some(true));
    assert!(a.error.as_deref().unwrap_or("").contains("自動寄出失敗"), "沒有郵件帳號，自動寄出應失敗並留下草稿：{:?}", a.error);
    let atts = store::list_attachments(&pool, &a.id).await.unwrap();
    assert_eq!(atts.len(), 1);
    assert_eq!(std::fs::read(&atts[0].path).unwrap(), PNG);

    // ---- B：外部、手動派送 → 沒按之前不會動 → 按了才跑；沒改程式所以驗證不通過、一律草稿 ----
    tokio::time::sleep(Duration::from_secs(7)).await;
    let b = item_by_subject(&pool, "請問怎麼用").await;
    assert_eq!(b.status, store::PENDING, "外部問題在你按派送之前不可被派出去");
    assert!(!read_log().contains("想問用法"), "外部問題在核准前就被送進終端機了");
    store::approve(&pool, &b.id).await.unwrap();
    wait_for("B 進入草稿", 90, || async { item_by_subject(&pool, "請問怎麼用").await.status == store::DRAFT }).await;
    let b = item_by_subject(&pool, "請問怎麼用").await;
    println!("---- B 的驗證 ----\n{:?}\nerror={:?}", b.verification, b.error);
    assert_eq!(b.verified, Some(false));
    let report = b.verification.unwrap();
    assert!(report.contains("沒有偵測到任何檔案變更") && !report.contains("未設定測試指令"), "{report}");

    // ---- D：處理到一半分頁被關掉 → 問題退回 pending 並說明原因，不遺失 ----
    mail_source::ingest_routed(&pool, &routes, &eml("me@home.com", "[P1] 會卡住的問題", "d@h", "HANG 這題不會回", false), &attach_root).await.unwrap();
    wait_for("假 claude 卡住", 60, || async { read_log().contains("HANGING") }).await;
    assert_eq!(item_by_subject(&pool, "會卡住的問題").await.status, store::RUNNING);
    pty.close(&tab).unwrap();
    wait_for("D 退回 pending", 30, || async { item_by_subject(&pool, "會卡住的問題").await.status == store::PENDING }).await;
    let d = item_by_subject(&pool, "會卡住的問題").await;
    assert!(d.error.as_deref().unwrap_or("").contains("分頁已關閉"), "{:?}", d.error);
    assert!(rt.registry.tabs().is_empty());
    // p2 的題目從頭到尾沒被領取、沒被送進終端機。
    let p2 = store::list_items(&pool, "p2", None).await.unwrap();
    assert_eq!(p2[0].status, store::QUEUED, "別的專案的題目不可被這個專案的分頁領走");
    assert!(!read_log().contains("P2ONLY"), "別的專案的題目被送進了終端機");
    let _: PathBuf = repo;
}
