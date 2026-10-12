//! GUI 控制與截圖。設計見 docs/superpowers/specs/2026-10-12-computer-use-design.md。
//!
//! 所有入口（目前是 MCP tool server）只能呼叫 [`ComputerService`]：
//! 總開關 → 權限 → 找 App → 拒絕 AITerm 自己 → 首次授權 → 破壞性確認 → 後端 → 稽核。

pub mod approval;
pub mod audit;
pub mod backend;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod policy;
pub mod tools;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::ConfigStore;
use approval::{ApprovalKind, ApprovalRequest, Approver, Decision};
use audit::AuditLog;
use backend::{AppInfo, Backend, ElementNode, KeyCombo, MouseButton};

/// 工具回傳：純文字，或影像加說明文字。
#[derive(Debug)]
pub enum ToolOutput {
    Text(String),
    Image { png: Vec<u8>, text: String },
}

pub enum ClickTarget {
    Element(String),
    Point(f64, f64),
}

/// 內建 Ask AI 可用的電腦操作工具；總開關關閉時一個都不給（模型根本看不到）。
pub fn builtin_tool_infos(cfg: &crate::config::AppConfig) -> Vec<crate::mcp::McpToolInfo> {
    if cfg.mcp_tool_server.computer_enabled { tools::tool_infos() } else { vec![] }
}

const DENIED_PREFIX: &str = "使用者拒絕";
const SCREEN_KEY: &str = "*screen*";
const MAX_TREE_LINES: usize = 400;
const MAX_STATE_CHARS: usize = 300;

pub struct ComputerService {
    backend: Arc<dyn Backend>,
    approver: Arc<dyn Approver>,
    config: Arc<ConfigStore>,
    audit: AuditLog,
    own_pid: i32,
    /// 最近一次 snapshot 記下的 (pid, 元件 id) → (role, title)，用來偵測 id 過期。
    seen: Mutex<HashMap<(i32, String), (String, String)>>,
}

fn app_key(app: &AppInfo) -> String {
    app.bundle_id.clone().unwrap_or_else(|| app.name.clone())
}

impl ComputerService {
    pub fn new(
        backend: Arc<dyn Backend>,
        approver: Arc<dyn Approver>,
        config: Arc<ConfigStore>,
        audit: AuditLog,
        own_pid: i32,
    ) -> Self {
        Self { backend, approver, config, audit, own_pid, seen: Mutex::new(HashMap::new()) }
    }

    pub fn permissions(&self) -> backend::Permissions {
        self.backend.permissions()
    }

    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    fn finish(&self, tool: &str, app: &str, detail: &str, r: Result<ToolOutput, String>) -> Result<ToolOutput, String> {
        let outcome = match &r {
            Ok(_) => "ok".to_string(),
            Err(e) if e.starts_with(DENIED_PREFIX) => "denied".to_string(),
            Err(e) => format!("error: {e}"),
        };
        self.audit.append(tool, app, detail, &outcome);
        r
    }

    fn require_enabled(&self) -> Result<(), String> {
        if self.config.get().mcp_tool_server.computer_enabled {
            Ok(())
        } else {
            Err("電腦操作工具未啟用。請到 AITerm 設定 → MCP Tool Server 開啟「電腦操作」。".to_string())
        }
    }

    fn require_accessibility(&self) -> Result<(), String> {
        if self.backend.permissions().accessibility {
            Ok(())
        } else {
            Err("缺少「輔助使用」權限。請到 系統設定 → 隱私權與安全性 → 輔助使用，允許 AITerm（開發版是啟動它的終端機）。".to_string())
        }
    }

    fn require_screen_recording(&self) -> Result<(), String> {
        if self.backend.permissions().screen_recording {
            Ok(())
        } else {
            Err("缺少「螢幕錄製」權限。請到 系統設定 → 隱私權與安全性 → 螢幕與系統音訊錄製，允許 AITerm。".to_string())
        }
    }

    async fn resolve_app(&self, query: &str) -> Result<AppInfo, String> {
        let apps = self.backend.list_apps().await?;
        let q = query.trim().to_lowercase();
        apps.iter()
            .find(|a| a.bundle_id.as_deref().map(|b| b.to_lowercase()) == Some(q.clone()) || a.name.to_lowercase() == q)
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
                format!("找不到 App「{query}」。目前可操作的有：{}", names.join("、"))
            })
    }

    async fn authorize(&self, key: String, display: &str, action: &str) -> Result<(), String> {
        if self.config.get().computer_allowed_apps.contains(&key) {
            return Ok(());
        }
        let req = ApprovalRequest {
            kind: ApprovalKind::FirstUse,
            app: display.to_string(),
            action: action.to_string(),
            detail: String::new(),
        };
        match self.approver.ask(&req).await {
            Decision::Deny => Err(format!("{DENIED_PREFIX}讓 AI 操作「{display}」")),
            Decision::AllowOnce => Ok(()),
            Decision::AllowAlways => {
                self.config
                    .update(|c| {
                        if !c.computer_allowed_apps.contains(&key) {
                            c.computer_allowed_apps.push(key.clone());
                        }
                    })
                    .map_err(|e| format!("無法儲存授權：{e}"))
            }
        }
    }

    /// 開關 → 找 App → 拒絕自己 → 首次授權。
    async fn gate(&self, app_query: &str, action: &str) -> Result<AppInfo, String> {
        self.require_enabled()?;
        let app = self.resolve_app(app_query).await?;
        if app.pid == self.own_pid {
            return Err("不可操作 AITerm 自己的視窗".to_string());
        }
        self.authorize(app_key(&app), &app.name, action).await?;
        Ok(app)
    }

    async fn confirm_destructive(&self, app: &AppInfo, action: &str, detail: &str) -> Result<(), String> {
        let req = ApprovalRequest {
            kind: ApprovalKind::Destructive,
            app: app.name.clone(),
            action: action.to_string(),
            detail: detail.to_string(),
        };
        match self.approver.ask(&req).await {
            Decision::Deny => Err(format!("{DENIED_PREFIX}執行「{detail}」")),
            // 破壞性操作每次都問，所以「永遠允許」在這裡只當作這一次。
            _ => Ok(()),
        }
    }

    /// 在動作結果後面附上焦點元件目前的內容，讓模型能自己驗證（例如發現「只刪了一個字」）。
    /// 讀不到就什麼都不附——狀態只是輔助，不能讓已成功的動作變成失敗。
    async fn with_state(&self, base: String, pid: i32) -> String {
        match self.backend.focused_text(pid).await {
            Ok(Some(f)) if f.secure => format!("{base}\n目前焦點：{}（密碼欄位，內容不回傳）", f.role),
            Ok(Some(f)) => {
                let shown = if f.text.is_empty() {
                    "（空）".to_string()
                } else if f.text.chars().count() > MAX_STATE_CHARS {
                    let head: String = f.text.chars().take(MAX_STATE_CHARS).collect();
                    format!("\"{head}…\"")
                } else {
                    format!("\"{}\"", f.text)
                };
                format!("{base}\n目前焦點：{}，內容：{shown}", f.role)
            }
            Ok(None) => base,
            Err(e) => {
                log::debug!("讀取焦點元件狀態失敗（不影響動作結果）：{e}");
                base
            }
        }
    }

    pub async fn list_apps(&self) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            let apps = self.backend.list_apps().await?;
            let lines: Vec<String> = apps
                .iter()
                .filter(|a| a.pid != self.own_pid)
                .map(|a| {
                    format!(
                        "{}{} [{}] pid={}",
                        a.name,
                        if a.frontmost { " (最前)" } else { "" },
                        a.bundle_id.as_deref().unwrap_or("-"),
                        a.pid
                    )
                })
                .collect();
            Ok(ToolOutput::Text(lines.join("\n")))
        }
        .await;
        self.finish("computer_list_apps", "", "", r)
    }

    pub async fn snapshot(&self, app: &str, max_depth: Option<usize>) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            self.require_accessibility()?;
            let info = self.gate(app, "讀取畫面元件").await?;
            let depth = max_depth.unwrap_or(6).clamp(1, 12);
            let tree = self.backend.snapshot(info.pid, depth).await?;
            let mut seen = self.seen.lock();
            seen.retain(|(pid, _), _| *pid != info.pid);
            let mut lines = Vec::new();
            render_tree(&tree, 0, info.pid, &mut seen, &mut lines);
            drop(seen);
            let truncated = lines.len() > MAX_TREE_LINES;
            lines.truncate(MAX_TREE_LINES);
            let mut text = lines.join("\n");
            if truncated {
                text.push_str("\n…（已截斷；請縮小 max_depth 或只看需要的部分）");
            }
            Ok(ToolOutput::Text(text))
        }
        .await;
        self.finish("computer_snapshot", app, "", r)
    }

    pub async fn screenshot(&self, app: Option<&str>) -> Result<ToolOutput, String> {
        let name = app.unwrap_or("(整個螢幕)");
        let r = async {
            self.require_enabled()?;
            self.require_screen_recording()?;
            let pid = match app {
                Some(q) => Some(self.gate(q, "截圖").await?.pid),
                None => {
                    self.authorize(SCREEN_KEY.to_string(), "整個螢幕", "截取整個螢幕").await?;
                    None
                }
            };
            let shot = self.backend.screenshot(pid).await?;
            let text = format!(
                "截圖 {}x{}（邏輯像素），左上角在螢幕座標 ({:.0},{:.0})；影像內座標加上這個原點即為點擊座標。",
                shot.width, shot.height, shot.origin_x, shot.origin_y
            );
            Ok(ToolOutput::Image { png: shot.png, text })
        }
        .await;
        self.finish("computer_screenshot", name, "", r)
    }

    pub async fn click(&self, app: &str, target: ClickTarget, button: MouseButton, double: bool) -> Result<ToolOutput, String> {
        let detail = match &target {
            ClickTarget::Element(id) => format!("element {id}"),
            ClickTarget::Point(x, y) => format!("({x:.0},{y:.0})"),
        };
        let r = async {
            self.require_enabled()?;
            self.require_accessibility()?;
            let info = self.gate(app, "點擊").await?;
            match &target {
                ClickTarget::Element(id) => {
                    let remembered = self.seen.lock().get(&(info.pid, id.clone())).cloned();
                    let Some((role, title)) = remembered else {
                        return Err("不認得這個元件 id。請先呼叫 computer_snapshot 取得最新的元件樹。".to_string());
                    };
                    let now = self.backend.element_info(info.pid, id).await?;
                    if now.role != role || now.title != title {
                        return Err("畫面已變，這個元件 id 已失效。請重新呼叫 computer_snapshot。".to_string());
                    }
                    if policy::is_dangerous_label(&title) {
                        self.confirm_destructive(&info, "點擊", &format!("{role}「{title}」")).await?;
                    }
                    self.backend.activate(info.pid).await?;
                    if button == MouseButton::Left && !double {
                        if self.backend.press_element(info.pid, id).await.is_ok() {
                            return Ok(ToolOutput::Text(format!("已點擊 {role}「{title}」")));
                        }
                    }
                    let (x, y) = now.center();
                    self.backend.click_at(x, y, button, double).await?;
                    Ok(ToolOutput::Text(format!("已點擊 {role}「{title}」（座標 {x:.0},{y:.0}）")))
                }
                ClickTarget::Point(x, y) => {
                    if let Ok(Some(el)) = self.backend.element_at(*x, *y).await {
                        if policy::is_dangerous_label(&el.title) {
                            self.confirm_destructive(&info, "點擊", &format!("{}「{}」", el.role, el.title)).await?;
                        }
                    }
                    self.backend.activate(info.pid).await?;
                    self.backend.click_at(*x, *y, button, double).await?;
                    Ok(ToolOutput::Text(format!("已點擊座標 ({x:.0},{y:.0})")))
                }
            }
        }
        .await;
        self.finish("computer_click", app, &detail, r)
    }

    pub async fn type_text(&self, app: &str, text: &str, element: Option<&str>, replace: bool) -> Result<ToolOutput, String> {
        let detail = format!("{} 個字元", text.chars().count());
        let r = async {
            self.require_enabled()?;
            self.require_accessibility()?;
            let info = self.gate(app, "輸入文字").await?;
            self.backend.activate(info.pid).await?;
            if let Some(id) = element {
                if !self.seen.lock().contains_key(&(info.pid, id.to_string())) {
                    return Err("不認得這個元件 id。請先呼叫 computer_snapshot 取得最新的元件樹。".to_string());
                }
                self.backend.focus_element(info.pid, id).await?;
            }
            if replace {
                // 先全選，輸入的文字才會取代原本的內容。
                let all = KeyCombo { key: "a".into(), cmd: true, ctrl: false, alt: false, shift: false };
                self.backend.key(&all).await?;
                // 讓應用程式先處理完全選，再送文字。
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            }
            self.backend.type_text(text).await?;
            let base = format!("已輸入 {} 個字元{}", text.chars().count(), if replace { "（已先全選並取代）" } else { "" });
            Ok(ToolOutput::Text(self.with_state(base, info.pid).await))
        }
        .await;
        self.finish("computer_type", app, &detail, r)
    }

    pub async fn key(&self, app: &str, spec: &str) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            self.require_accessibility()?;
            let combo = policy::parse_key(spec)?;
            let info = self.gate(app, "按鍵").await?;
            if policy::is_dangerous_key(&combo) {
                self.confirm_destructive(&info, "按鍵", spec).await?;
            }
            self.backend.activate(info.pid).await?;
            self.backend.key(&combo).await?;
            Ok(ToolOutput::Text(self.with_state(format!("已按下 {spec}"), info.pid).await))
        }
        .await;
        self.finish("computer_key", app, spec, r)
    }

    pub async fn scroll(&self, app: &str, dx: i32, dy: i32, at: Option<(f64, f64)>) -> Result<ToolOutput, String> {
        let detail = format!("dx={dx} dy={dy}");
        let r = async {
            self.require_enabled()?;
            self.require_accessibility()?;
            let info = self.gate(app, "捲動").await?;
            self.backend.activate(info.pid).await?;
            self.backend.scroll(dx, dy, at).await?;
            Ok(ToolOutput::Text(format!("已捲動 dx={dx} dy={dy}")))
        }
        .await;
        self.finish("computer_scroll", app, &detail, r)
    }
}

fn render_tree(
    n: &ElementNode,
    depth: usize,
    pid: i32,
    seen: &mut HashMap<(i32, String), (String, String)>,
    out: &mut Vec<String>,
) {
    seen.insert((pid, n.id.clone()), (n.role.clone(), n.title.clone()));
    let title = if n.title.is_empty() { String::new() } else { format!(" \"{}\"", n.title) };
    out.push(format!(
        "{}{} {}{} ({:.0},{:.0},{:.0},{:.0})",
        "  ".repeat(depth),
        n.id,
        n.role,
        title,
        n.x,
        n.y,
        n.w,
        n.h
    ));
    for c in &n.children {
        render_tree(c, depth + 1, pid, seen, out);
    }
}

#[cfg(test)]
mod tests;
