use std::collections::VecDeque;

use async_trait::async_trait;

use super::approval::{ApprovalKind, ApprovalRequest, Approver, Decision};
use super::backend::*;
use super::*;

const OWN_PID: i32 = 1;

fn el(id: &str, role: &str, title: &str) -> ElementNode {
    ElementNode { id: id.into(), role: role.into(), title: title.into(), x: 10.0, y: 20.0, w: 100.0, h: 40.0, children: vec![] }
}

/// 記錄所有被呼叫的動作；元件樹可在測試中途改寫來模擬「畫面變了」。
struct FakeBackend {
    perms: Permissions,
    tree: Mutex<ElementNode>,
    calls: Mutex<Vec<String>>,
    press_fails: bool,
    /// 模擬焦點文字區：(內容, 是否全選, 是否密碼欄位)。
    doc: Mutex<(String, bool, bool)>,
    list_calls: std::sync::atomic::AtomicUsize,
    /// 假的 OCR 結果（螢幕座標）。
    screen_text: Mutex<Vec<ScreenText>>,
}

fn st(text: &str, x: f64, y: f64) -> ScreenText {
    ScreenText { text: text.into(), x, y, w: 80.0, h: 20.0 }
}

impl FakeBackend {
    fn new() -> Arc<Self> {
        let mut root = el("0", "AXWindow", "Doc");
        root.children = vec![el("0.0", "AXButton", "Save"), el("0.1", "AXButton", "Delete")];
        Arc::new(Self {
            perms: Permissions { accessibility: true, screen_recording: true },
            tree: Mutex::new(root),
            calls: Mutex::new(vec![]),
            press_fails: false,
            doc: Mutex::new((String::new(), false, false)),
            list_calls: Default::default(),
            screen_text: Mutex::new(vec![]),
        })
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().clone()
    }
    fn list_calls(&self) -> usize {
        self.list_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn find(&self, id: &str) -> Option<ElementNode> {
        fn go(n: &ElementNode, id: &str) -> Option<ElementNode> {
            if n.id == id {
                return Some(ElementNode { children: vec![], ..n.clone() });
            }
            n.children.iter().find_map(|c| go(c, id))
        }
        go(&self.tree.lock(), id)
    }
}

#[async_trait]
impl Backend for FakeBackend {
    fn permissions(&self) -> Permissions { self.perms }
    async fn list_apps(&self) -> Result<Vec<AppInfo>, String> {
        self.list_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(vec![
            AppInfo { name: "AITerm".into(), localized: None, bundle_id: Some("com.aiterm.app".into()), pid: OWN_PID, frontmost: false },
            AppInfo { name: "Notes".into(), localized: Some("備忘錄".into()), bundle_id: Some("com.apple.Notes".into()), pid: 42, frontmost: true },
        ])
    }
    async fn snapshot(&self, _: i32, _: usize) -> Result<ElementNode, String> { Ok(self.tree.lock().clone()) }
    async fn element_info(&self, _: i32, id: &str) -> Result<ElementNode, String> {
        self.find(id).ok_or_else(|| "no such element".to_string())
    }
    async fn element_at(&self, x: f64, _: f64) -> Result<Option<ElementNode>, String> {
        Ok(if x < 50.0 { self.find("0.0") } else { self.find("0.1") })
    }
    async fn screenshot(&self, pid: Option<i32>) -> Result<Shot, String> {
        self.calls.lock().push(format!("screenshot {pid:?}"));
        Ok(Shot { png: vec![1, 2, 3], width: 800, height: 600, origin_x: 0.0, origin_y: 0.0 })
    }
    async fn activate(&self, pid: i32) -> Result<(), String> { self.calls.lock().push(format!("activate {pid}")); Ok(()) }
    async fn press_element(&self, _: i32, id: &str) -> Result<(), String> {
        if self.press_fails { return Err("AXPress unsupported".into()); }
        self.calls.lock().push(format!("press {id}"));
        Ok(())
    }
    async fn click_at(&self, x: f64, y: f64, _: MouseButton, double: bool) -> Result<(), String> {
        self.calls.lock().push(format!("click_at {x:.0},{y:.0} double={double}"));
        Ok(())
    }
    async fn type_text(&self, t: &str) -> Result<(), String> {
        self.calls.lock().push(format!("type {t}"));
        let mut d = self.doc.lock();
        if d.1 { d.0.clear(); d.1 = false; }
        d.0.push_str(t);
        Ok(())
    }
    async fn focus_element(&self, _: i32, id: &str) -> Result<(), String> { self.calls.lock().push(format!("focus {id}")); Ok(()) }
    async fn key(&self, c: &KeyCombo) -> Result<(), String> {
        self.calls.lock().push(format!("key {}{}", if c.cmd { "cmd+" } else { "" }, c.key));
        let mut d = self.doc.lock();
        match (c.cmd, c.key.as_str()) {
            (true, "a") => d.1 = true,
            (false, "delete") => { if d.1 { d.0.clear(); d.1 = false } else { d.0.pop(); } }
            _ => {}
        }
        Ok(())
    }
    async fn read_screen(&self, pid: Option<i32>) -> Result<Vec<ScreenText>, String> {
        self.calls.lock().push(format!("read_screen {pid:?}"));
        Ok(self.screen_text.lock().clone())
    }
    async fn focused_text(&self, _: i32) -> Result<Option<FocusedText>, String> {
        let d = self.doc.lock();
        Ok(Some(FocusedText { role: "AXTextArea".into(), text: d.0.clone(), secure: d.2 }))
    }
    async fn scroll(&self, dx: i32, dy: i32, _: Option<(f64, f64)>) -> Result<(), String> {
        self.calls.lock().push(format!("scroll {dx},{dy}"));
        Ok(())
    }
}

/// 依序回覆預設決定；沒決定了就拒絕。記錄所有被問到的請求。
struct FakeApprover {
    script: Mutex<VecDeque<Decision>>,
    asked: Mutex<Vec<(ApprovalKind, String, String)>>,
}

impl FakeApprover {
    fn new(script: &[Decision]) -> Arc<Self> {
        Arc::new(Self { script: Mutex::new(script.iter().copied().collect()), asked: Mutex::new(vec![]) })
    }
    fn asked(&self) -> Vec<(ApprovalKind, String, String)> { self.asked.lock().clone() }
}

#[async_trait]
impl Approver for FakeApprover {
    async fn ask(&self, req: &ApprovalRequest) -> Decision {
        self.asked.lock().push((req.kind, req.app.clone(), req.detail.clone()));
        self.script.lock().pop_front().unwrap_or(Decision::Deny)
    }
}

struct Rig {
    svc: ComputerService,
    backend: Arc<FakeBackend>,
    approver: Arc<FakeApprover>,
    config: Arc<ConfigStore>,
    _dir: tempfile::TempDir,
}

fn rig_with(backend: Arc<FakeBackend>, enabled: bool, script: &[Decision]) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let config = Arc::new(ConfigStore::new_at(dir.path().join("config.toml")));
    config.update(|c| c.mcp_tool_server.computer_enabled = enabled).unwrap();
    let approver = FakeApprover::new(script);
    let svc = ComputerService::new(
        backend.clone(),
        approver.clone(),
        config.clone(),
        AuditLog::new(dir.path().join("audit.jsonl")),
        OWN_PID,
    );
    svc.set_settle(std::time::Duration::ZERO);
    Rig { svc, backend, approver, config, _dir: dir }
}

fn rig_with_backend(backend: Arc<dyn Backend>, script: &[Decision]) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let config = Arc::new(ConfigStore::new_at(dir.path().join("config.toml")));
    config.update(|c| c.mcp_tool_server.computer_enabled = true).unwrap();
    let approver = FakeApprover::new(script);
    let svc = ComputerService::new(backend, approver.clone(), config.clone(), AuditLog::new(dir.path().join("audit.jsonl")), OWN_PID);
    // Rig.backend 只給需要檢查假後端的測試用；這裡的後端不是 FakeBackend，放一個空的占位。
    svc.set_settle(std::time::Duration::ZERO);
    Rig { svc, backend: FakeBackend::new(), approver, config, _dir: dir }
}

fn rig(script: &[Decision]) -> Rig {
    rig_with(FakeBackend::new(), true, script)
}

fn text(o: ToolOutput) -> String {
    match o {
        ToolOutput::Text(t) => t,
        ToolOutput::Image { text, .. } => text,
    }
}

#[tokio::test]
async fn disabled_by_default_blocks_every_tool_and_touches_nothing() {
    let r = rig_with(FakeBackend::new(), false, &[Decision::AllowAlways]);
    assert!(r.svc.list_apps().await.unwrap_err().contains("未啟用"));
    assert!(r.svc.snapshot("Notes", None).await.unwrap_err().contains("未啟用"));
    assert!(r.svc.key("Notes", "cmd+s").await.unwrap_err().contains("未啟用"));
    assert!(r.approver.asked().is_empty());
    assert!(r.backend.calls().is_empty());
}

#[tokio::test]
async fn list_apps_hides_aiterm_itself() {
    let r = rig(&[]);
    let out = text(r.svc.list_apps().await.unwrap());
    assert!(out.contains("Notes"));
    assert!(!out.contains("AITerm"));
}

#[tokio::test]
async fn refuses_to_operate_aiterm_itself_without_asking() {
    let r = rig(&[Decision::AllowAlways]);
    let e = r.svc.snapshot("AITerm", None).await.unwrap_err();
    assert!(e.contains("AITerm 自己"));
    assert!(r.approver.asked().is_empty());
}

#[tokio::test]
async fn unknown_app_lists_what_is_available() {
    let r = rig(&[]);
    let e = r.svc.snapshot("Photoshop", None).await.unwrap_err();
    assert!(e.contains("找不到") && e.contains("Notes"));
}

#[tokio::test]
async fn first_use_asks_once_and_allow_always_persists_by_bundle_id() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    assert_eq!(r.approver.asked().len(), 1);
    assert_eq!(r.approver.asked()[0].0, ApprovalKind::FirstUse);
    assert_eq!(r.config.get().computer_allowed_apps, vec!["com.apple.Notes".to_string()]);

    // 第二次不再問（腳本已空，再問就會被拒絕）。
    r.svc.snapshot("notes", None).await.unwrap();
    assert_eq!(r.approver.asked().len(), 1);
}

#[tokio::test]
async fn allow_once_is_not_remembered() {
    let r = rig(&[Decision::AllowOnce, Decision::AllowOnce]);
    r.svc.snapshot("Notes", None).await.unwrap();
    r.svc.snapshot("Notes", None).await.unwrap();
    assert_eq!(r.approver.asked().len(), 2);
    assert!(r.config.get().computer_allowed_apps.is_empty());
}

#[tokio::test]
async fn deny_stops_before_the_backend_and_is_audited_as_denied() {
    let r = rig(&[Decision::Deny]);
    let e = r.svc.type_text("Notes", "hi", None, false).await.unwrap_err();
    assert!(e.starts_with("使用者拒絕"));
    assert!(r.backend.calls().is_empty());
    let log = r.svc.audit().tail(5);
    assert_eq!(log[0].tool, "computer_type");
    assert_eq!(log[0].outcome, "denied");
}

#[tokio::test]
async fn missing_permissions_are_reported_before_anything_else() {
    let b = Arc::new(FakeBackend {
        perms: Permissions { accessibility: false, screen_recording: false },
        tree: Mutex::new(el("0", "AXWindow", "")),
        calls: Mutex::new(vec![]),
        press_fails: false,
        doc: Mutex::new((String::new(), false, false)),
        list_calls: Default::default(),
        screen_text: Mutex::new(vec![]),
    });
    let r = rig_with(b, true, &[Decision::AllowAlways]);
    assert!(r.svc.snapshot("Notes", None).await.unwrap_err().contains("輔助使用"));
    assert!(r.svc.screenshot(Some("Notes")).await.unwrap_err().contains("螢幕錄製"));
    assert!(r.approver.asked().is_empty());
}

#[tokio::test]
async fn snapshot_renders_ids_roles_titles_and_frames() {
    let r = rig(&[Decision::AllowAlways]);
    let out = text(r.svc.snapshot("Notes", None).await.unwrap());
    assert!(out.contains("0 AXWindow \"Doc\" (10,20,100,40)"), "{out}");
    assert!(out.contains("  0.0 AXButton \"Save\""), "{out}");
}

#[tokio::test]
async fn click_requires_a_snapshot_first() {
    let r = rig(&[Decision::AllowAlways]);
    let e = r.svc.click("Notes", ClickTarget::Element("0.0".into()), MouseButton::Left, false).await.unwrap_err();
    assert!(e.contains("computer_snapshot"));
}

#[tokio::test]
async fn click_presses_the_element_after_activating_the_app() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    let out = text(r.svc.click("Notes", ClickTarget::Element("0.0".into()), MouseButton::Left, false).await.unwrap());
    assert!(out.contains("Save"));
    assert_eq!(r.backend.calls(), vec!["activate 42", "press 0.0"]);
}

#[tokio::test]
async fn click_falls_back_to_coordinates_when_press_is_unsupported() {
    let mut b = FakeBackend::new();
    Arc::get_mut(&mut b).unwrap().press_fails = true;
    let r = rig_with(b, true, &[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    r.svc.click("Notes", ClickTarget::Element("0.0".into()), MouseButton::Left, false).await.unwrap();
    // 中心點 = (10 + 100/2, 20 + 40/2)
    assert_eq!(r.backend.calls(), vec!["activate 42", "click_at 60,40 double=false"]);
}

#[tokio::test]
async fn right_click_and_double_click_go_straight_to_coordinates() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    r.svc.click("Notes", ClickTarget::Element("0.0".into()), MouseButton::Right, false).await.unwrap();
    r.svc.click("Notes", ClickTarget::Element("0.0".into()), MouseButton::Left, true).await.unwrap();
    let c = r.backend.calls();
    assert!(!c.iter().any(|x| x.starts_with("press")), "{c:?}");
    assert!(c.iter().any(|x| x == "click_at 60,40 double=true"), "{c:?}");
}

#[tokio::test]
async fn stale_element_id_is_rejected_when_the_screen_changed() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    r.backend.tree.lock().children[0].title = "Something else".into();
    let e = r.svc.click("Notes", ClickTarget::Element("0.0".into()), MouseButton::Left, false).await.unwrap_err();
    assert!(e.contains("失效"), "{e}");
    assert!(!r.backend.calls().iter().any(|c| c.starts_with("press")));
}

#[tokio::test]
async fn destructive_click_asks_every_time_even_when_app_is_allowed_always() {
    // 1: 首次授權 AllowAlways；2、3: 兩次破壞性確認，第一次允許、第二次拒絕。
    let r = rig(&[Decision::AllowAlways, Decision::AllowAlways, Decision::Deny]);
    r.svc.snapshot("Notes", None).await.unwrap();
    r.svc.click("Notes", ClickTarget::Element("0.1".into()), MouseButton::Left, false).await.unwrap();
    let e = r.svc.click("Notes", ClickTarget::Element("0.1".into()), MouseButton::Left, false).await.unwrap_err();
    assert!(e.starts_with("使用者拒絕"));
    let asked = r.approver.asked();
    assert_eq!(asked.iter().filter(|a| a.0 == ApprovalKind::Destructive).count(), 2);
    assert!(asked[1].2.contains("Delete"));
    // 「永遠允許」對破壞性操作只算這一次：設定裡仍只有那個 App，沒有多出東西。
    assert_eq!(r.config.get().computer_allowed_apps.len(), 1);
    // 被拒絕的那次沒有真的按下去。
    assert_eq!(r.backend.calls().iter().filter(|c| c.starts_with("press")).count(), 1);
}

#[tokio::test]
async fn coordinate_click_on_a_dangerous_element_is_also_confirmed() {
    let r = rig(&[Decision::AllowAlways, Decision::Deny]);
    r.svc.snapshot("Notes", None).await.unwrap();
    // fake 的 element_at：x >= 50 是 Delete 按鈕
    let e = r.svc.click("Notes", ClickTarget::Point(80.0, 40.0), MouseButton::Left, false).await.unwrap_err();
    assert!(e.starts_with("使用者拒絕"));
    assert!(!r.backend.calls().iter().any(|c| c.starts_with("click_at")));
    // 安全的座標不問
    r.svc.click("Notes", ClickTarget::Point(10.0, 40.0), MouseButton::Left, false).await.unwrap();
}

#[tokio::test]
async fn key_validates_before_asking_and_cmd_delete_is_confirmed() {
    let r = rig(&[Decision::AllowAlways, Decision::Deny]);
    assert!(r.svc.key("Notes", "cmd+bogus").await.unwrap_err().contains("不認得"));
    assert!(r.approver.asked().is_empty(), "壞按鍵不該打擾使用者");

    r.svc.key("Notes", "cmd+s").await.unwrap();
    let e = r.svc.key("Notes", "cmd+delete").await.unwrap_err();
    assert!(e.starts_with("使用者拒絕"));
    assert!(r.backend.calls().contains(&"key cmd+s".to_string()));
    assert!(!r.backend.calls().contains(&"key cmd+delete".to_string()));
}

#[tokio::test]
async fn type_with_element_focuses_it_first_and_rejects_unknown_ids() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    assert!(r.svc.type_text("Notes", "x", Some("9.9"), false).await.unwrap_err().contains("computer_snapshot"));
    r.svc.type_text("Notes", "hello", Some("0.0"), false).await.unwrap();
    let c = r.backend.calls();
    let f = c.iter().position(|x| x == "focus 0.0").unwrap();
    let t = c.iter().position(|x| x == "type hello").unwrap();
    assert!(f < t);
}

#[tokio::test]
async fn type_never_logs_the_typed_text() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.type_text("Notes", "my-secret-password", None, false).await.unwrap();
    let raw = std::fs::read_to_string(r._dir.path().join("audit.jsonl")).unwrap();
    assert!(!raw.contains("my-secret-password"), "{raw}");
}

#[tokio::test]
async fn full_screen_screenshot_needs_its_own_first_use_authorization() {
    let r = rig(&[Decision::AllowAlways]);
    let out = r.svc.screenshot(None).await.unwrap();
    assert!(matches!(out, ToolOutput::Image { .. }));
    assert_eq!(r.approver.asked()[0].1, "整個螢幕");
    assert_eq!(r.config.get().computer_allowed_apps, vec!["*screen*".to_string()]);
}

#[tokio::test]
async fn app_screenshot_passes_the_pid_and_describes_the_origin() {
    let r = rig(&[Decision::AllowAlways]);
    let out = text(r.svc.screenshot(Some("Notes")).await.unwrap());
    assert!(out.contains("800x600"));
    assert_eq!(r.backend.calls(), vec!["screenshot Some(42)"]);
}

#[tokio::test]
async fn every_call_lands_in_the_audit_log_newest_first() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    let _ = r.svc.key("Notes", "nope+").await;
    let log = r.svc.audit().tail(10);
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].tool, "computer_key");
    assert!(log[0].outcome.starts_with("error"));
    assert_eq!(log[1].outcome, "ok");
}

#[tokio::test]
async fn unsupported_backend_reports_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let config = Arc::new(ConfigStore::new_at(dir.path().join("config.toml")));
    config.update(|c| c.mcp_tool_server.computer_enabled = true).unwrap();
    let svc = ComputerService::new(
        Arc::new(UnsupportedBackend),
        FakeApprover::new(&[]),
        config,
        AuditLog::new(dir.path().join("a.jsonl")),
        OWN_PID,
    );
    assert!(svc.list_apps().await.unwrap_err().contains("macOS"));
}

// ── 操作後回報焦點元件的狀態（讓模型能發現「沒清空」之類的失誤），以及 replace 選項。

#[tokio::test]
async fn type_result_reports_what_the_focused_field_now_contains() {
    let r = rig(&[Decision::AllowAlways]);
    let out = text(r.svc.type_text("Notes", "你好 AITerm", None, false).await.unwrap());
    assert!(out.contains("AXTextArea") && out.contains("你好 AITerm"), "{out}");
}

#[tokio::test]
async fn a_bare_delete_leaves_text_behind_and_the_result_shows_it() {
    // 重現真實失誤：模型只送 delete 沒先全選，文字只少一個字。結果文字要讓這一點看得出來。
    let r = rig(&[Decision::AllowAlways]);
    r.svc.type_text("Notes", "你好 AITerm", None, false).await.unwrap();
    let out = text(r.svc.key("Notes", "delete").await.unwrap());
    assert!(out.contains("你好 AITer") && !out.contains("內容：（空）"), "{out}");

    r.svc.key("Notes", "cmd+a").await.unwrap();
    let out = text(r.svc.key("Notes", "delete").await.unwrap());
    assert!(out.contains("內容：（空）"), "{out}");
}

#[tokio::test]
async fn replace_selects_all_before_typing_so_old_text_is_gone() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.type_text("Notes", "old text", None, false).await.unwrap();
    let out = text(r.svc.type_text("Notes", "new", None, true).await.unwrap());
    assert!(out.contains("\"new\"") && !out.contains("old"), "{out}");
    let c = r.backend.calls();
    let sel = c.iter().rposition(|x| x == "key cmd+a").unwrap();
    let typ = c.iter().rposition(|x| x == "type new").unwrap();
    assert!(sel < typ, "{c:?}");
}

#[tokio::test]
async fn without_replace_no_select_all_is_sent() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.type_text("Notes", "x", None, false).await.unwrap();
    assert!(!r.backend.calls().iter().any(|c| c == "key cmd+a"));
}

#[tokio::test]
async fn password_fields_are_never_echoed_back() {
    let r = rig(&[Decision::AllowAlways]);
    r.backend.doc.lock().2 = true;
    let out = text(r.svc.type_text("Notes", "hunter2", None, false).await.unwrap());
    assert!(!out.contains("hunter2"), "{out}");
    assert!(out.contains("密碼欄位"), "{out}");
}

#[tokio::test]
async fn long_field_contents_are_truncated_in_the_result() {
    let r = rig(&[Decision::AllowAlways]);
    let big = "字".repeat(1000);
    let out = text(r.svc.type_text("Notes", &big, None, false).await.unwrap());
    assert!(out.chars().count() < 600, "{}", out.chars().count());
    assert!(out.contains("…"));
}

#[tokio::test]
async fn failing_to_read_the_field_state_does_not_fail_the_action() {
    let r = rig_with(FakeBackend::new(), true, &[Decision::AllowAlways]);
    // 空的 UnsupportedBackend 式失敗：用一個 focused_text 回錯誤的後端。
    struct NoState(Arc<FakeBackend>);
    #[async_trait]
    impl Backend for NoState {
        fn permissions(&self) -> Permissions { self.0.permissions() }
        async fn list_apps(&self) -> Result<Vec<AppInfo>, String> { self.0.list_apps().await }
        async fn snapshot(&self, p: i32, d: usize) -> Result<ElementNode, String> { self.0.snapshot(p, d).await }
        async fn element_info(&self, p: i32, i: &str) -> Result<ElementNode, String> { self.0.element_info(p, i).await }
        async fn element_at(&self, x: f64, y: f64) -> Result<Option<ElementNode>, String> { self.0.element_at(x, y).await }
        async fn screenshot(&self, p: Option<i32>) -> Result<Shot, String> { self.0.screenshot(p).await }
        async fn activate(&self, p: i32) -> Result<(), String> { self.0.activate(p).await }
        async fn press_element(&self, p: i32, i: &str) -> Result<(), String> { self.0.press_element(p, i).await }
        async fn click_at(&self, x: f64, y: f64, b: MouseButton, d: bool) -> Result<(), String> { self.0.click_at(x, y, b, d).await }
        async fn type_text(&self, t: &str) -> Result<(), String> { self.0.type_text(t).await }
        async fn focus_element(&self, p: i32, i: &str) -> Result<(), String> { self.0.focus_element(p, i).await }
        async fn key(&self, c: &KeyCombo) -> Result<(), String> { self.0.key(c).await }
        async fn scroll(&self, x: i32, y: i32, a: Option<(f64, f64)>) -> Result<(), String> { self.0.scroll(x, y, a).await }
        async fn read_screen(&self, p: Option<i32>) -> Result<Vec<ScreenText>, String> { self.0.read_screen(p).await }
        async fn focused_text(&self, _: i32) -> Result<Option<FocusedText>, String> { Err("AX 讀取失敗".into()) }
    }
    drop(r);
    let r = rig_with_backend(Arc::new(NoState(FakeBackend::new())), &[Decision::AllowAlways]);
    let out = text(r.svc.type_text("Notes", "hi", None, false).await.unwrap());
    assert!(out.contains("已輸入 2 個字元") && !out.contains("目前焦點"), "{out}");
}

// ── 效能：App 清單快取、精簡元件樹、依文字找元件。

#[tokio::test]
async fn app_list_is_cached_so_each_tool_call_does_not_relist_every_app() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    r.svc.snapshot("Notes", None).await.unwrap();
    r.svc.key("Notes", "cmd+s").await.unwrap();
    assert_eq!(r.backend.list_calls(), 1);
}

#[tokio::test]
async fn an_app_missing_from_a_fresh_cache_triggers_one_refresh_then_a_clear_error() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    assert_eq!(r.backend.list_calls(), 1);
    let e = r.svc.snapshot("Calendar", None).await.unwrap_err(); // 剛開啟的 App 不在舊快取裡
    assert!(e.contains("找不到"));
    assert_eq!(r.backend.list_calls(), 2);
}

#[tokio::test]
async fn an_expired_cache_is_refreshed_and_list_apps_always_reads_fresh() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.set_apps_ttl(std::time::Duration::ZERO);
    r.svc.snapshot("Notes", None).await.unwrap();
    r.svc.snapshot("Notes", None).await.unwrap();
    assert_eq!(r.backend.list_calls(), 2);

    r.svc.set_apps_ttl(std::time::Duration::from_secs(60));
    r.svc.list_apps().await.unwrap(); // 明確要清單：不吃快取
    assert_eq!(r.backend.list_calls(), 3);
    r.svc.snapshot("Notes", None).await.unwrap(); // 但會順便更新快取
    assert_eq!(r.backend.list_calls(), 3);
}

fn noisy_tree() -> ElementNode {
    let mut root = el("0", "AXWindow", "Doc");
    let mut group = el("0.0", "AXGroup", "");
    group.children = vec![el("0.0.0", "AXButton", "Save")];
    let mut hidden = el("0.1", "AXButton", "Offscreen");
    hidden.w = 0.0;
    hidden.h = 0.0;
    root.children = vec![group, hidden, el("0.2", "AXTextField", "")];
    root
}

#[tokio::test]
async fn compact_snapshot_drops_empty_containers_and_zero_size_nodes_but_keeps_ids_stable() {
    let b = FakeBackend::new();
    *b.tree.lock() = noisy_tree();
    let r = rig_with(b, true, &[Decision::AllowAlways]);
    let compact = text(r.svc.snapshot("Notes", None).await.unwrap());
    assert!(!compact.contains("AXGroup"), "{compact}");
    assert!(!compact.contains("Offscreen"), "{compact}");
    assert!(compact.contains("0.0.0 AXButton \"Save\""), "孩子的 id 不能因為父層被省略而改變：{compact}");
    assert!(compact.contains("0.2 AXTextField"), "空標題但可互動的欄位要保留：{compact}");

    let full = text(r.svc.snapshot_opts("Notes", None, false).await.unwrap());
    assert!(full.contains("AXGroup") && full.contains("Offscreen"), "{full}");
    assert!(compact.lines().count() < full.lines().count());
}

#[tokio::test]
async fn ids_dropped_from_the_compact_view_are_still_known_for_clicks() {
    let b = FakeBackend::new();
    *b.tree.lock() = noisy_tree();
    let r = rig_with(b, true, &[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    // 0.0.0 在精簡輸出中縮排變淺，但 id 照常有效。
    r.svc.click("Notes", ClickTarget::Element("0.0.0".into()), MouseButton::Left, false).await.unwrap();
}

#[tokio::test]
async fn find_matches_title_or_role_case_insensitively_and_registers_ids() {
    let r = rig(&[Decision::AllowAlways]);
    let out = text(r.svc.find("Notes", "delete", None, 20).await.unwrap());
    assert!(out.contains("0.1 AXButton \"Delete\""), "{out}");
    assert!(!out.contains("Save"), "{out}");

    let by_role = text(r.svc.find("Notes", "", Some("AXButton"), 20).await.unwrap());
    assert!(by_role.contains("Save") && by_role.contains("Delete"), "{by_role}");

    // find 之後不必先 snapshot 就能點（id 已登記）
    r.svc.click("Notes", ClickTarget::Element("0.0".into()), MouseButton::Left, false).await.unwrap();
}

#[tokio::test]
async fn find_caps_results_and_reports_when_nothing_matches() {
    let r = rig(&[Decision::AllowAlways]);
    let none = text(r.svc.find("Notes", "no-such-thing", None, 20).await.unwrap());
    assert!(none.contains("沒有找到"), "{none}");
    let one = text(r.svc.find("Notes", "", Some("AXButton"), 1).await.unwrap());
    assert_eq!(one.lines().filter(|l| l.contains("AXButton")).count(), 1, "{one}");
    assert!(one.contains("僅列出前 1"), "{one}");
}

#[tokio::test]
async fn find_requires_a_query_or_a_role() {
    let r = rig(&[Decision::AllowAlways]);
    assert!(r.svc.find("Notes", "  ", None, 20).await.unwrap_err().contains("query"));
}

// ── 減少模型往返：動作後自動附上畫面現況、批次動作。

use super::tools::{BatchStep, Call, ComputerBatchArgs, ComputerClickArgs, ComputerKeyArgs, ComputerScrollArgs, ComputerTypeArgs};

fn click_args(app: &str, element: &str) -> ComputerClickArgs {
    ComputerClickArgs { app: app.into(), element: Some(element.into()), x: None, y: None, button: None, double: None, observe: None }
}

fn step(action: &str) -> BatchStep {
    BatchStep { action: action.into(), element: None, x: None, y: None, button: None, double: None, text: None, replace: None, key: None, dx: None, dy: None, index: None }
}

#[tokio::test]
async fn actions_attach_the_resulting_screen_by_default_and_ids_stay_usable() {
    let r = rig(&[Decision::AllowAlways]);
    tools::execute(&r.svc, Call::Snapshot(snapshot_args("Notes"))).await.unwrap();
    let out = text(tools::execute(&r.svc, Call::Click(click_args("Notes", "0.0"))).await.unwrap());
    assert!(out.contains("已點擊") && out.contains("畫面現況"), "{out}");
    assert!(out.contains("0.1 AXButton \"Delete\""), "{out}");
}

#[tokio::test]
async fn observe_false_returns_only_the_action_result() {
    let r = rig(&[Decision::AllowAlways]);
    tools::execute(&r.svc, Call::Snapshot(snapshot_args("Notes"))).await.unwrap();
    let mut a = click_args("Notes", "0.0");
    a.observe = Some(false);
    let out = text(tools::execute(&r.svc, Call::Click(a)).await.unwrap());
    assert!(!out.contains("畫面現況"), "{out}");
}

#[tokio::test]
async fn a_failed_action_does_not_attach_a_screen() {
    let r = rig(&[Decision::Deny]);
    let e = tools::execute(&r.svc, Call::Key(ComputerKeyArgs { app: "Notes".into(), key: "cmd+s".into(), observe: None })).await.unwrap_err();
    assert!(e.starts_with("使用者拒絕") && !e.contains("畫面現況"), "{e}");
}

#[tokio::test]
async fn observation_is_capped_so_a_huge_window_cannot_flood_the_reply() {
    let b = FakeBackend::new();
    {
        let mut root = el("0", "AXWindow", "Big");
        root.children = (0..300).map(|i| el(&format!("0.{i}"), "AXButton", &format!("b{i}"))).collect();
        *b.tree.lock() = root;
    }
    let r = rig_with(b, true, &[Decision::AllowAlways]);
    let out = text(tools::execute(&r.svc, Call::Key(ComputerKeyArgs { app: "Notes".into(), key: "tab".into(), observe: None })).await.unwrap());
    assert!(out.lines().count() < 100, "{}", out.lines().count());
    assert!(out.contains("已截斷"), "{out}");
}

fn snapshot_args(app: &str) -> super::tools::ComputerSnapshotArgs {
    super::tools::ComputerSnapshotArgs { app: app.into(), max_depth: None, compact: None }
}

#[tokio::test]
async fn batch_runs_steps_in_order_and_observes_once_at_the_end() {
    let r = rig(&[Decision::AllowAlways]);
    tools::execute(&r.svc, Call::Snapshot(snapshot_args("Notes"))).await.unwrap();
    let mut click = step("click");
    click.element = Some("0.0".into());
    let mut typ = step("type");
    typ.text = Some("hello".into());
    let mut key = step("key");
    key.key = Some("return".into());
    let out = text(tools::execute(&r.svc, Call::Batch(ComputerBatchArgs { app: "Notes".into(), steps: vec![click, typ, key], observe: None })).await.unwrap());
    let c = r.backend.calls();
    let (p, t, k) = (
        c.iter().position(|x| x == "press 0.0").unwrap(),
        c.iter().position(|x| x == "type hello").unwrap(),
        c.iter().position(|x| x == "key return").unwrap(),
    );
    assert!(p < t && t < k, "{c:?}");
    assert!(out.contains("步驟 1/3") && out.contains("步驟 3/3"), "{out}");
    assert_eq!(out.matches("畫面現況").count(), 1, "{out}");
}

#[tokio::test]
async fn batch_stops_at_the_first_failure_and_reports_progress() {
    let r = rig(&[Decision::AllowAlways]);
    let mut typ = step("type");
    typ.text = Some("one".into());
    let mut bad = step("key");
    bad.key = Some("cmd+bogus".into());
    let mut never = step("type");
    never.text = Some("never".into());
    let e = tools::execute(&r.svc, Call::Batch(ComputerBatchArgs { app: "Notes".into(), steps: vec![typ, bad, never], observe: None })).await.unwrap_err();
    assert!(e.contains("步驟 1/3") && e.contains("步驟 2 失敗"), "{e}");
    assert!(!r.backend.calls().iter().any(|c| c == "type never"));
}

#[tokio::test]
async fn every_batch_step_still_goes_through_the_destructive_gate() {
    let r = rig(&[Decision::AllowAlways, Decision::Deny]);
    tools::execute(&r.svc, Call::Snapshot(snapshot_args("Notes"))).await.unwrap();
    let mut safe = step("click");
    safe.element = Some("0.0".into());
    let mut risky = step("click");
    risky.element = Some("0.1".into()); // Delete
    let e = tools::execute(&r.svc, Call::Batch(ComputerBatchArgs { app: "Notes".into(), steps: vec![safe, risky], observe: None })).await.unwrap_err();
    assert!(e.contains("使用者拒絕"), "{e}");
    assert_eq!(r.backend.calls().iter().filter(|c| c.starts_with("press")).count(), 1);
    assert!(r.approver.asked().iter().any(|a| a.0 == ApprovalKind::Destructive));
}

#[tokio::test]
async fn batch_rejects_empty_oversized_and_unknown_steps_before_doing_anything() {
    let r = rig(&[Decision::AllowAlways]);
    let run = |steps: Vec<BatchStep>| tools::execute(&r.svc, Call::Batch(ComputerBatchArgs { app: "Notes".into(), steps, observe: None }));
    assert!(run(vec![]).await.unwrap_err().contains("至少"));
    assert!(run((0..11).map(|_| step("key")).collect()).await.unwrap_err().contains("最多 10"));
    let mut k = step("key");
    k.key = Some("a".into());
    assert!(run(vec![k, step("dance")]).await.unwrap_err().contains("dance"));
    assert!(r.backend.calls().is_empty(), "整批驗證通過前不能先動手：{:?}", r.backend.calls());
}

#[tokio::test]
async fn batch_step_missing_its_required_field_is_reported_with_the_step_number() {
    let r = rig(&[Decision::AllowAlways]);
    let e = tools::execute(&r.svc, Call::Batch(ComputerBatchArgs { app: "Notes".into(), steps: vec![step("type")], observe: None })).await.unwrap_err();
    assert!(e.contains("步驟 1") && e.contains("text"), "{e}");
}

#[tokio::test]
async fn type_and_scroll_calls_accept_the_new_observe_flag() {
    let r = rig(&[Decision::AllowAlways]);
    let out = text(tools::execute(&r.svc, Call::Type(ComputerTypeArgs { app: "Notes".into(), text: "x".into(), element: None, replace: None, observe: Some(false) })).await.unwrap());
    assert!(!out.contains("畫面現況") && out.contains("已輸入"));
    let out = text(tools::execute(&r.svc, Call::Scroll(ComputerScrollArgs { app: "Notes".into(), dx: None, dy: Some(3), x: None, y: None, observe: Some(false) })).await.unwrap());
    assert!(out.contains("已捲動"));
}

// ── 沒有無障礙資訊的 App（Chromium／Qt，例如 LINE）：靠螢幕文字辨識（OCR）看畫面、點文字。

fn sparse_tree() -> ElementNode {
    // 真實 LINE 的樣子：視窗 1x1、只有一個空的 AXWebArea，其餘只有選單列。
    let mut root = el("0", "AXApplication", "");
    let mut win = el("0.0", "AXWindow", "");
    win.w = 1.0;
    win.h = 1.0;
    let mut bar = el("0.1", "AXMenuBar", "");
    bar.children = vec![el("0.1.0", "AXMenuBarItem", "檔案"), el("0.1.1", "AXMenuBarItem", "編輯")];
    root.children = vec![win, bar];
    root
}

fn line_like_rig(script: &[Decision]) -> Rig {
    let b = FakeBackend::new();
    *b.tree.lock() = sparse_tree();
    *b.screen_text.lock() = vec![st("搜尋", 100.0, 60.0), st("景詳", 100.0, 200.0), st("景詳的朋友", 100.0, 260.0), st("傳送", 700.0, 700.0)];
    rig_with(b, true, script)
}

#[tokio::test]
async fn read_screen_lists_text_with_coordinates_top_to_bottom_and_needs_screen_recording() {
    let r = line_like_rig(&[Decision::AllowAlways]);
    let out = text(r.svc.read_screen("Notes").await.unwrap());
    let a = out.find("搜尋").unwrap();
    let b = out.find("景詳").unwrap();
    assert!(a < b, "要由上到下：{out}");
    assert!(out.contains("(100,60,80,20)"), "{out}");
    let c = r.backend.calls();
    assert!(c.iter().position(|x| x == "activate 42") < c.iter().position(|x| x.starts_with("read_screen")), "{c:?}");

    let b2 = Arc::new(FakeBackend {
        perms: Permissions { accessibility: true, screen_recording: false },
        tree: Mutex::new(el("0", "AXWindow", "")),
        calls: Mutex::new(vec![]),
        press_fails: false,
        doc: Mutex::new((String::new(), false, false)),
        list_calls: Default::default(),
        screen_text: Mutex::new(vec![]),
    });
    let r2 = rig_with(b2, true, &[Decision::AllowAlways]);
    assert!(r2.svc.read_screen("Notes").await.unwrap_err().contains("螢幕錄製"));
}

#[tokio::test]
async fn read_screen_with_no_text_says_so_instead_of_returning_an_empty_string() {
    let r = rig(&[Decision::AllowAlways]);
    let out = text(r.svc.read_screen("Notes").await.unwrap());
    assert!(out.contains("沒有辨識到"), "{out}");
}

#[tokio::test]
async fn click_text_clicks_the_centre_of_the_best_match_after_activating_the_app() {
    let r = line_like_rig(&[Decision::AllowAlways]);
    let out = text(r.svc.click_text("Notes", "搜尋", None, MouseButton::Left, false).await.unwrap());
    assert!(out.contains("搜尋"), "{out}");
    // 中心 = (100 + 80/2, 60 + 20/2)
    assert_eq!(r.backend.calls().iter().filter(|c| c.starts_with("click_at")).collect::<Vec<_>>(), vec!["click_at 140,70 double=false"]);
    // 先切到前景、再辨識、再點擊：順序錯了座標會對到別的視窗。
    let c = r.backend.calls();
    let act = c.iter().position(|x| x == "activate 42").unwrap();
    let ocr = c.iter().position(|x| x.starts_with("read_screen")).unwrap();
    let clk = c.iter().position(|x| x.starts_with("click_at")).unwrap();
    assert!(act < ocr && ocr < clk, "{c:?}");
}

#[tokio::test]
async fn click_text_prefers_an_exact_match_over_a_substring_one() {
    let r = line_like_rig(&[Decision::AllowAlways]);
    r.svc.click_text("Notes", "景詳", None, MouseButton::Left, false).await.unwrap();
    assert!(r.backend.calls().iter().any(|c| c == "click_at 140,210 double=false"), "{:?}", r.backend.calls());
}

#[tokio::test]
async fn click_text_with_several_equal_matches_asks_which_one_instead_of_guessing() {
    let b = FakeBackend::new();
    *b.screen_text.lock() = vec![st("OK", 10.0, 10.0), st("OK", 10.0, 300.0)];
    let r = rig_with(b, true, &[Decision::AllowAlways]);
    let e = r.svc.click_text("Notes", "OK", None, MouseButton::Left, false).await.unwrap_err();
    assert!(e.contains("2 處") && e.contains("index"), "{e}");
    assert!(!r.backend.calls().iter().any(|c| c.starts_with("click_at")));
    r.svc.click_text("Notes", "OK", Some(2), MouseButton::Left, false).await.unwrap();
    assert!(r.backend.calls().iter().any(|c| c == "click_at 50,310 double=false"));
}

#[tokio::test]
async fn click_text_reports_what_was_actually_seen_when_nothing_matches() {
    let r = line_like_rig(&[Decision::AllowAlways]);
    let e = r.svc.click_text("Notes", "不存在", None, MouseButton::Left, false).await.unwrap_err();
    assert!(e.contains("找不到") && e.contains("搜尋"), "應列出畫面上實際有的字：{e}");
}

#[tokio::test]
async fn click_text_reactivates_the_app_after_a_confirmation_dialog_stole_focus() {
    let r = line_like_rig(&[Decision::AllowAlways, Decision::AllowOnce]);
    r.svc.click_text("Notes", "傳送", None, MouseButton::Left, false).await.unwrap();
    let c = r.backend.calls();
    let last_act = c.iter().rposition(|x| x == "activate 42").unwrap();
    let clk = c.iter().position(|x| x.starts_with("click_at")).unwrap();
    assert!(last_act < clk && c.iter().filter(|x| *x == "activate 42").count() >= 2, "{c:?}");
}

#[tokio::test]
async fn click_text_on_a_dangerous_label_asks_every_time() {
    let r = line_like_rig(&[Decision::AllowAlways, Decision::Deny]);
    let e = r.svc.click_text("Notes", "傳送", None, MouseButton::Left, false).await.unwrap_err();
    assert!(e.starts_with("使用者拒絕"), "{e}");
    assert!(!r.backend.calls().iter().any(|c| c.starts_with("click_at")));
    assert!(r.approver.asked().iter().any(|a| a.0 == ApprovalKind::Destructive && a.2.contains("傳送")));
}

#[tokio::test]
async fn sparse_accessibility_apps_are_flagged_in_snapshot_and_fall_back_to_ocr_when_observing() {
    let r = line_like_rig(&[Decision::AllowAlways]);
    let snap = text(r.svc.snapshot("Notes", None).await.unwrap());
    assert!(snap.contains("computer_read_screen") && snap.contains("computer_click_text"), "{snap}");

    let out = text(tools::execute(&r.svc, Call::Key(ComputerKeyArgs { app: "Notes".into(), key: "tab".into(), observe: None })).await.unwrap());
    assert!(out.contains("畫面文字") && out.contains("景詳"), "{out}");
}

#[tokio::test]
async fn rich_accessibility_apps_do_not_pay_for_ocr() {
    let b = FakeBackend::new();
    {
        let mut root = el("0", "AXWindow", "Doc");
        root.children = (0..6).map(|i| el(&format!("0.{i}"), "AXButton", &format!("b{i}"))).collect();
        *b.tree.lock() = root;
    }
    let r = rig_with(b, true, &[Decision::AllowAlways]);
    tools::execute(&r.svc, Call::Key(ComputerKeyArgs { app: "Notes".into(), key: "tab".into(), observe: None })).await.unwrap();
    assert!(!r.backend.calls().iter().any(|c| c.starts_with("read_screen")), "{:?}", r.backend.calls());
}

#[tokio::test]
async fn batch_supports_click_text_steps() {
    let r = line_like_rig(&[Decision::AllowAlways]);
    let mut click = step("click_text");
    click.text = Some("搜尋".into());
    let mut typ = step("type");
    typ.text = Some("景詳".into());
    let out = text(tools::execute(&r.svc, Call::Batch(ComputerBatchArgs { app: "Notes".into(), steps: vec![click, typ], observe: Some(false) })).await.unwrap());
    assert!(out.contains("步驟 1/2 click_text") && out.contains("步驟 2/2 type"), "{out}");
    let c = r.backend.calls();
    assert!(c.iter().position(|x| x.starts_with("click_at")) < c.iter().position(|x| x == "type 景詳"));
}

// ── 內建 Ask AI 入口（computer::tools::call）：結果一律包成文字＋is_error，模型讀得到原因。

#[tokio::test]
async fn builtin_call_runs_a_tool_through_the_same_gate_and_returns_text() {
    let r = rig(&[Decision::AllowAlways]);
    let out = tools::call(&r.svc, "computer_snapshot", serde_json::json!({"app": "Notes"})).await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("AXButton \"Save\""));
    // 首次授權照樣被問到，且「永遠允許」寫進設定——內建入口沒有繞過任何東西。
    assert_eq!(r.approver.asked().len(), 1);
    assert_eq!(r.config.get().computer_allowed_apps, vec!["com.apple.Notes".to_string()]);
}

#[tokio::test]
async fn builtin_call_reports_denial_disabled_and_bad_arguments_as_errors_not_panics() {
    let denied = rig(&[Decision::Deny]);
    let out = tools::call(&denied.svc, "computer_key", serde_json::json!({"app": "Notes", "key": "cmd+s"})).await;
    assert!(out.is_error && out.content.starts_with("使用者拒絕"), "{}", out.content);
    assert!(denied.backend.calls().is_empty());

    let off = rig_with(FakeBackend::new(), false, &[]);
    let out = tools::call(&off.svc, "computer_list_apps", serde_json::json!({})).await;
    assert!(out.is_error && out.content.contains("未啟用"));

    let r = rig(&[]);
    let out = tools::call(&r.svc, "computer_click", serde_json::json!({"x": 1})).await; // 缺 app
    assert!(out.is_error && out.content.contains("參數錯誤"), "{}", out.content);
    let out = tools::call(&r.svc, "computer_nope", serde_json::json!({})).await;
    assert!(out.is_error && out.content.contains("未知"));
}

#[tokio::test]
async fn builtin_call_still_asks_before_a_destructive_click() {
    let r = rig(&[Decision::AllowAlways, Decision::Deny]);
    tools::call(&r.svc, "computer_snapshot", serde_json::json!({"app": "Notes"})).await;
    let out = tools::call(&r.svc, "computer_click", serde_json::json!({"app": "Notes", "element": "0.1"})).await;
    assert!(out.is_error && out.content.starts_with("使用者拒絕"), "{}", out.content);
    assert!(!r.backend.calls().iter().any(|c| c.starts_with("press")));
}

#[test]
fn builtin_tools_are_hidden_unless_the_master_switch_is_on() {
    let mut cfg = crate::config::AppConfig::default();
    assert!(builtin_tool_infos(&cfg).is_empty());
    cfg.mcp_tool_server.computer_enabled = true;
    assert_eq!(builtin_tool_infos(&cfg).len(), 11);
}

// ── 真機端對端（macOS，需要輔助使用＋螢幕錄製權限，預設不跑）：
//    cargo test -p app --lib computer::tests::live -- --ignored --nocapture
#[cfg(target_os = "macos")]
mod live {
    use super::*;
    use crate::computer::macos::MacBackend;

    fn osa(script: &str) -> String {
        let out = std::process::Command::new("osascript").args(["-e", script]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[tokio::test]
    #[ignore]
    async fn textedit_type_key_click_and_screenshot_end_to_end() {
        osa("tell application \"TextEdit\" to activate");
        osa("tell application \"TextEdit\" to make new document");
        std::thread::sleep(std::time::Duration::from_millis(800));

        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(ConfigStore::new_at(dir.path().join("config.toml")));
        config.update(|c| c.mcp_tool_server.computer_enabled = true).unwrap();
        let approver = FakeApprover::new(&[Decision::AllowOnce; 16]);
        let svc = ComputerService::new(
            Arc::new(MacBackend),
            approver.clone(),
            config,
            AuditLog::new(dir.path().join("audit.jsonl")),
            std::process::id() as i32,
        );

        let apps = text(svc.list_apps().await.unwrap());
        assert!(apps.contains("TextEdit"), "{apps}");

        let tree = text(svc.snapshot("TextEdit", Some(8)).await.unwrap());
        println!("{tree}");
        let area = tree
            .lines()
            .find(|l| l.contains("AXTextArea"))
            .and_then(|l| l.split_whitespace().next())
            .expect("找不到 AXTextArea")
            .to_string();

        let t0 = std::time::Instant::now();
        svc.type_text("TextEdit", "123 from AITerm 你好", Some(&area), false).await.unwrap();
        println!("TIMING type_text (cached apps, app already frontmost): {:?}", t0.elapsed());
        let got = osa("tell application \"TextEdit\" to get text of document 1");
        println!("document text = {got:?}");
        assert_eq!(got, "123 from AITerm 你好");

        // 只按 delete：只少一個字，而且結果文字要讓模型看得出來。
        let r = text(svc.key("TextEdit", "delete").await.unwrap());
        println!("after bare delete: {r}");
        assert!(r.contains("123 from AITerm 你"), "{r}");
        assert_eq!(osa("tell application \"TextEdit\" to get text of document 1"), "123 from AITerm 你");

        // replace：先全選再輸入，舊文字消失。
        let r = text(svc.type_text("TextEdit", "999 replaced", Some(&area), true).await.unwrap());
        println!("after replace: {r}");
        std::thread::sleep(std::time::Duration::from_millis(600));
        println!("doc after replace +600ms: {:?}", osa("tell application \"TextEdit\" to get text of document 1"));
        assert!(r.contains("\"999 replaced\""), "{r}");
        assert_eq!(osa("tell application \"TextEdit\" to get text of document 1"), "999 replaced");

        svc.key("TextEdit", "cmd+a").await.unwrap();
        let r = text(svc.key("TextEdit", "delete").await.unwrap());
        assert!(r.contains("（空）"), "{r}");
        assert_eq!(osa("tell application \"TextEdit\" to get text of document 1"), "");

        // 座標點擊文字區中央（安全元件，不應觸發確認）
        let frame = tree.lines().find(|l| l.contains("AXTextArea")).unwrap().to_string();
        println!("{frame}");

        let shot = svc.screenshot(Some("TextEdit")).await.unwrap();
        match shot {
            ToolOutput::Image { png, text } => {
                println!("{text} bytes={}", png.len());
                std::fs::write(std::env::temp_dir().join("aiterm-live-textedit.png"), &png).unwrap();
                assert!(png.len() > 1000);
            }
            _ => panic!("expected image"),
        }

        // 只有第一次使用 TextEdit 時問過使用者（AllowOnce 不記憶，所以每個動作各問一次）。
        assert!(approver.asked().iter().all(|a| a.0 == ApprovalKind::FirstUse));

        osa("tell application \"TextEdit\" to close document 1 saving no");
    }
}

#[tokio::test]
async fn apps_can_be_addressed_by_process_name_localized_name_or_bundle_id() {
    let r = rig(&[Decision::AllowAlways]);
    for q in ["Notes", "備忘錄", "com.apple.Notes", "notes"] {
        r.svc.snapshot(q, None).await.unwrap_or_else(|e| panic!("{q}: {e}"));
    }
    let listed = text(r.svc.list_apps().await.unwrap());
    assert!(listed.contains("Notes（備忘錄）"), "{listed}");
}

#[cfg(target_os = "macos")]
mod live_ocr_line {
    use super::*;
    use crate::computer::macos::MacBackend;

    /// 真機：LINE 沒有無障礙資訊，但 OCR 要讀得到畫面上的字。需要 LINE 開著且有可見視窗。
    #[tokio::test]
    #[ignore]
    async fn line_is_readable_through_ocr_even_though_its_accessibility_tree_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(ConfigStore::new_at(dir.path().join("config.toml")));
        config.update(|c| c.mcp_tool_server.computer_enabled = true).unwrap();
        let svc = ComputerService::new(
            Arc::new(MacBackend),
            FakeApprover::new(&[Decision::AllowOnce; 8]),
            config,
            AuditLog::new(dir.path().join("audit.jsonl")),
            std::process::id() as i32,
        );
        let snap = text(svc.snapshot("LINE", None).await.unwrap());
        println!("SNAPSHOT_TAIL {}", snap.lines().last().unwrap_or(""));
        assert!(snap.contains("computer_read_screen"), "稀疏的 App 應提示改用 OCR：{snap}");
        let t = std::time::Instant::now();
        let out = text(svc.read_screen("LINE").await.unwrap());
        println!("TIMING read_screen(LINE): {:?}", t.elapsed());
        for l in out.lines().take(25) {
            println!("LINEOCR {l}");
        }
        assert!(out.lines().count() > 3, "{out}");
    }
}

#[cfg(target_os = "macos")]
mod live_line_flow {
    use super::*;
    use crate::computer::macos::MacBackend;

    /// 真機、不會送出任何訊息：用 batch 點 LINE 的搜尋欄（click_text）、打入名字，
    /// 視窗開著時 LINE 有無障礙資訊（沒開時只有 1x1 的幽靈視窗）。結束後清掉搜尋文字。
    #[tokio::test]
    #[ignore]
    async fn line_search_flow_with_click_text_batch_and_ocr_observation() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(ConfigStore::new_at(dir.path().join("config.toml")));
        config.update(|c| c.mcp_tool_server.computer_enabled = true).unwrap();
        let svc = ComputerService::new(
            Arc::new(MacBackend),
            FakeApprover::new(&[Decision::AllowOnce; 16]),
            config,
            AuditLog::new(dir.path().join("audit.jsonl")),
            std::process::id() as i32,
        );
        let mut click = step("click_text");
        click.text = Some("以姓名搜尋".into());
        let mut typ = step("type");
        typ.text = Some("景詳".into());
        let t = std::time::Instant::now();
        let out = text(tools::execute(&svc, Call::Batch(ComputerBatchArgs { app: "LINE".into(), steps: vec![click, typ], observe: None })).await.unwrap());
        println!("TIMING batch(click_text + type + observe): {:?}", t.elapsed());
        for l in out.lines().take(20) {
            println!("FLOW {l}");
        }
        assert!(out.contains("步驟 1/2 click_text") && out.contains("景詳"), "{out}");
        // 清掉搜尋文字，不留痕跡。
        let mut a = step("key");
        a.key = Some("cmd+a".into());
        let mut d = step("key");
        d.key = Some("delete".into());
        let _ = tools::execute(&svc, Call::Batch(ComputerBatchArgs { app: "LINE".into(), steps: vec![a, d], observe: Some(false) })).await;
    }
}
