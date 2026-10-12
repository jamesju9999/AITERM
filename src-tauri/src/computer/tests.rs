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
        })
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().clone()
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
        Ok(vec![
            AppInfo { name: "AITerm".into(), bundle_id: Some("com.aiterm.app".into()), pid: OWN_PID, frontmost: false },
            AppInfo { name: "Notes".into(), bundle_id: Some("com.apple.Notes".into()), pid: 42, frontmost: true },
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
    async fn type_text(&self, t: &str) -> Result<(), String> { self.calls.lock().push(format!("type {t}")); Ok(()) }
    async fn focus_element(&self, _: i32, id: &str) -> Result<(), String> { self.calls.lock().push(format!("focus {id}")); Ok(()) }
    async fn key(&self, c: &KeyCombo) -> Result<(), String> { self.calls.lock().push(format!("key {}", c.key)); Ok(()) }
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
    Rig { svc, backend, approver, config, _dir: dir }
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
    let e = r.svc.type_text("Notes", "hi", None).await.unwrap_err();
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
    assert!(r.backend.calls().contains(&"key s".to_string()));
    assert!(!r.backend.calls().contains(&"key delete".to_string()));
}

#[tokio::test]
async fn type_with_element_focuses_it_first_and_rejects_unknown_ids() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.snapshot("Notes", None).await.unwrap();
    assert!(r.svc.type_text("Notes", "x", Some("9.9")).await.unwrap_err().contains("computer_snapshot"));
    r.svc.type_text("Notes", "hello", Some("0.0")).await.unwrap();
    let c = r.backend.calls();
    let f = c.iter().position(|x| x == "focus 0.0").unwrap();
    let t = c.iter().position(|x| x == "type hello").unwrap();
    assert!(f < t);
}

#[tokio::test]
async fn type_never_logs_the_typed_text() {
    let r = rig(&[Decision::AllowAlways]);
    r.svc.type_text("Notes", "my-secret-password", None).await.unwrap();
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
