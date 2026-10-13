//! GUI 控制與截圖。設計見 docs/superpowers/specs/2026-10-12-computer-use-design.md。
//!
//! 所有入口（目前是 MCP tool server）只能呼叫 [`ComputerService`]：
//! 總開關 → 權限 → 找 App → 拒絕 AITerm 自己 → 首次授權 → 破壞性確認 → 後端 → 稽核。

pub mod approval;
pub mod audit;
pub mod backend;
pub mod cleanup;
pub mod icons;
pub mod linuxmap;
#[cfg(target_os = "linux")]
pub mod linux_wayland;
#[cfg(target_os = "linux")]
pub mod pipewire_capture;
pub mod waylandmap;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;
pub mod policy;
pub mod winmap;
pub mod tools;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::config::ConfigStore;
use approval::{ApprovalKind, ApprovalRequest, Approver, Decision};
use audit::AuditLog;
use backend::{AppInfo, Backend, ElementNode, IconRect, KeyCombo, MouseButton, ScreenText};
use icons::Region;

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
    // 不支援的平台不給模型看工具：看得到卻每個都失敗，只會浪費它的回合。
    if cfg.mcp_tool_server.computer_enabled && platform_supported() { tools::tool_infos() } else { vec![] }
}

/// 這個作業系統有沒有電腦操作後端。
pub fn os_supported(os: &str) -> bool {
    matches!(os, "macos" | "windows" | "linux")
}

pub fn platform_supported() -> bool {
    os_supported(std::env::consts::OS)
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
    /// App 清單快取：每個工具呼叫都要先找目標 App，而列清單約要 1 秒（見 `fetch_apps`）。
    apps: Mutex<Option<(Instant, Vec<AppInfo>)>>,
    apps_ttl: Mutex<Duration>,
    /// 動作與下一步（或畫面觀察）之間等 UI 反應的時間；測試設為 0。
    settle: Mutex<Duration>,
}

/// App 清單快取的有效時間。夠短，使用者剛開啟／關閉 App 時不會等太久；
/// 目標不在快取裡時會立刻重新讀一次，所以新開的 App 不受影響。
const APPS_TTL: Duration = Duration::from_secs(2);
const SETTLE: Duration = Duration::from_millis(200);
/// 動作後附上的畫面現況：只看淺層、精簡、行數有上限。
const OBSERVE_DEPTH: usize = 4;
const OBSERVE_MAX_LINES: usize = 60;
/// 無障礙樹裡「有內容的節點」少於這個數，就視為這個 App 幾乎沒有無障礙資訊（Chromium／Qt）。
/// 關閉掉無障礙的 Electron 視窗大約只剩三顆視窗控制鈕，所以門檻設在 4。
const SPARSE_CONTENT_NODES: usize = 4;
const MAX_SCREEN_TEXT_LINES: usize = 120;
const OBSERVE_OCR_LINES: usize = 40;
/// 這些角色是 App 一定會有的骨架（選單列、視窗外框），不算「有內容」。
const SCAFFOLD_ROLES: &[&str] = &[
    "AXApplication", "AXMenuBar", "AXMenuBarItem", "AXMenu", "AXMenuItem", "AXWindow", "AXWebArea",
];

/// 精簡元件樹時，標題為空就省略的「純容器」角色（孩子仍會列出，id 不變）。
const GENERIC_ROLES: &[&str] = &[
    "AXGroup", "AXUnknown", "AXScrollArea", "AXSplitGroup", "AXLayoutArea", "AXLayoutItem", "AXSplitter",
];

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
        Self {
            backend,
            approver,
            config,
            audit,
            own_pid,
            seen: Mutex::new(HashMap::new()),
            apps: Mutex::new(None),
            apps_ttl: Mutex::new(APPS_TTL),
            settle: Mutex::new(SETTLE),
        }
    }

    pub fn set_settle(&self, d: Duration) {
        *self.settle.lock() = d;
    }

    pub fn settle(&self) -> Duration {
        *self.settle.lock()
    }

    pub fn set_apps_ttl(&self, ttl: Duration) {
        *self.apps_ttl.lock() = ttl;
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
            // 開關開著，但這個環境不能用（例如 Wayland）：直接說明，不要讓使用者看到一堆「缺少權限」。
            match self.backend.unsupported_reason() {
                Some(reason) => Err(reason),
                None => Ok(()),
            }
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

    /// 一定重新讀並更新快取。
    async fn fetch_apps(&self) -> Result<Vec<AppInfo>, String> {
        let list = self.backend.list_apps().await?;
        *self.apps.lock() = Some((Instant::now(), list.clone()));
        Ok(list)
    }

    fn cached_apps(&self) -> Option<Vec<AppInfo>> {
        let ttl = *self.apps_ttl.lock();
        self.apps.lock().as_ref().filter(|(at, _)| at.elapsed() < ttl).map(|(_, l)| l.clone())
    }

    async fn resolve_app(&self, query: &str) -> Result<AppInfo, String> {
        let q = query.trim().to_lowercase();
        let find = |apps: &[AppInfo]| {
            apps.iter()
                .find(|a| {
                    a.bundle_id.as_deref().map(|b| b.to_lowercase()) == Some(q.clone())
                        || a.name.to_lowercase() == q
                        || a.localized.as_deref().map(|l| l.to_lowercase()) == Some(q.clone())
                })
                .cloned()
        };
        // 先看快取；沒有、過期、或目標不在裡面（可能剛開啟）才重新讀。
        if let Some(app) = self.cached_apps().as_deref().and_then(find) {
            return Ok(app);
        }
        let apps = self.fetch_apps().await?;
        // 只有「整個桌面」可操作的環境（Wayland）：模型照習慣講 App 名稱（如 Firefox）時，指的就是這個畫面。
        let whole_desktop = || match apps.as_slice() {
            [only] if only.pid == backend::WHOLE_DESKTOP_PID => Some(only.clone()),
            _ => None,
        };
        find(&apps).or_else(whole_desktop).ok_or_else(|| {
            let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
            format!("找不到 App「{query}」。目前可操作的有：{}", names.join("、"))
        })
    }

    async fn authorize(&self, key: String, display: &str, action: &str) -> Result<(), String> {
        self.authorize_any(vec![key], display, action).await
    }

    /// `keys` 任何一個已在「永遠允許」清單裡就放行；選「永遠允許」時存第一個。
    /// 同一個 App 可能用 bundle id、行程名稱、系統語言名稱被指到，所以要接受多個別名。
    async fn authorize_any(&self, keys: Vec<String>, display: &str, action: &str) -> Result<(), String> {
        let allowed = self.config.get().computer_allowed_apps;
        if keys.iter().any(|k| allowed.contains(k)) {
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
                let key = keys[0].clone();
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
        // 以 bundle id（優先）、行程名稱、系統語言名稱任一個比對；先用名稱開啟過的 App 也算數。
        let mut keys = vec![app_key(&app), app.name.to_lowercase()];
        keys.extend(app.localized.as_ref().map(|l| l.to_lowercase()));
        self.authorize_any(keys, &app.name, action).await?;
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

    /// 動作成功後附上的畫面現況（精簡元件樹）。這是剛成功的動作結果的一部分，所以不另外要授權、
    /// 也不寫稽核；任何失敗都回 `None`，不影響動作本身。同時刷新元件 id，讓下一步可以直接用。
    pub async fn observe(&self, app: &str) -> Option<String> {
        self.require_enabled().ok()?;
        self.require_accessibility().ok()?;
        let settle = self.settle();
        if !settle.is_zero() {
            tokio::time::sleep(settle).await; // 等 UI 對剛才的動作有反應
        }
        let info = self.resolve_app(app).await.ok()?;
        if info.pid == self.own_pid {
            return None;
        }
        let tree = self.backend.snapshot(info.pid, OBSERVE_DEPTH).await.ok()?;
        let mut lines = Vec::new();
        {
            // 鎖只在這個區塊內持有：之後還有 await（OCR），guard 不能跨過它。
            let mut seen = self.seen.lock();
            seen.retain(|(pid, _), _| *pid != info.pid);
            render_tree(&tree, 0, 0, true, info.pid, &mut seen, &mut lines);
        }
        let truncated = lines.len() > OBSERVE_MAX_LINES;
        lines.truncate(OBSERVE_MAX_LINES);
        let mut text = lines.join("\n");
        if truncated {
            text.push_str("\n…（已截斷；需要更多請呼叫 computer_snapshot 或 computer_find）");
        }
        // 幾乎沒有無障礙資訊的 App：樹裡只有選單列，對模型沒用。改附畫面文字（OCR），
        // 模型才「看得見」動作的結果。只有這種 App 才付 OCR 的時間，一般 App 不受影響。
        if count_content_nodes(&tree) < SPARSE_CONTENT_NODES && self.backend.permissions().screen_recording {
            if let Ok(mut items) = self.backend.read_screen(Some(info.pid)).await {
                sort_reading_order(&mut items);
                if !items.is_empty() {
                    let shown: Vec<String> = items.iter().take(OBSERVE_OCR_LINES).map(format_screen_text).collect();
                    text.push_str("\n畫面文字（OCR）：\n");
                    text.push_str(&shown.join("\n"));
                    if items.len() > OBSERVE_OCR_LINES {
                        text.push_str("\n…（更多請呼叫 computer_read_screen）");
                    }
                }
            }
        }
        Some(text)
    }

    pub async fn list_apps(&self) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            let apps = self.fetch_apps().await?;
            let lines: Vec<String> = apps
                .iter()
                .filter(|a| a.pid != self.own_pid)
                .map(|a| {
                    format!(
                        "{}{}{} [{}] pid={}",
                        a.name,
                        a.localized.as_deref().map(|l| format!("（{l}）")).unwrap_or_default(),
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
        self.snapshot_opts(app, max_depth, true).await
    }

    /// `compact`：省略沒有標題的純容器與零大小（看不見）的節點，大幅縮短輸出；id 不變。
    pub async fn snapshot_opts(&self, app: &str, max_depth: Option<usize>, compact: bool) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            self.require_accessibility()?;
            let info = self.gate(app, "讀取畫面元件").await?;
            let depth = max_depth.unwrap_or(6).clamp(1, 12);
            let tree = self.backend.snapshot(info.pid, depth).await?;
            let mut seen = self.seen.lock();
            seen.retain(|(pid, _), _| *pid != info.pid);
            let mut lines = Vec::new();
            render_tree(&tree, 0, 0, compact, info.pid, &mut seen, &mut lines);
            drop(seen);
            let truncated = lines.len() > MAX_TREE_LINES;
            lines.truncate(MAX_TREE_LINES);
            let mut text = lines.join("\n");
            if truncated {
                text.push_str("\n…（已截斷；請縮小 max_depth 或只看需要的部分）");
            }
            if count_content_nodes(&tree) < SPARSE_CONTENT_NODES {
                text.push_str(
                    "\n\n注意：這個 App 幾乎沒有提供無障礙資訊（常見於 Chromium／Qt 的應用程式，例如 LINE），\
                     上面看不到它的按鈕與文字。請改用 computer_read_screen 讀畫面上的字，\
                     並用 computer_click_text 直接點文字；輸入文字與按鍵照常可用。",
                );
            }
            Ok(ToolOutput::Text(text))
        }
        .await;
        self.finish("computer_snapshot", app, "", r)
    }

    /// 讀畫面上的文字（OCR）。給沒有無障礙資訊的 App 用；有座標，可直接拿去點。
    pub async fn read_screen(&self, app: &str) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            self.require_screen_recording()?;
            let info = self.gate(app, "讀取畫面文字").await?;
            // 先切到前景：視窗被擋住或在背景時，截到的會是別人的畫面。
            self.backend.activate(info.pid).await?;
            let mut items = self.backend.read_screen(Some(info.pid)).await?;
            sort_reading_order(&mut items);
            if items.is_empty() {
                return Ok(ToolOutput::Text("沒有辨識到任何文字（視窗可能被遮住、最小化，或畫面上沒有字）。".to_string()));
            }
            let total = items.len();
            let mut lines: Vec<String> = items.iter().take(MAX_SCREEN_TEXT_LINES).map(format_screen_text).collect();
            if total > MAX_SCREEN_TEXT_LINES {
                lines.push(format!("…（僅列出前 {MAX_SCREEN_TEXT_LINES} 段，共 {total} 段）"));
            }
            Ok(ToolOutput::Text(lines.join("\n")))
        }
        .await;
        self.finish("computer_read_screen", app, "", r)
    }

    /// 找畫面上的某段文字並點它的中心。比對不到時列出畫面上實際有的字，讓模型能修正。
    pub async fn click_text(
        &self,
        app: &str,
        query: &str,
        index: Option<usize>,
        button: MouseButton,
        double: bool,
    ) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            self.require_accessibility()?;
            self.require_screen_recording()?;
            if query.trim().is_empty() {
                return Err("text 不可為空".to_string());
            }
            let info = self.gate(app, "點擊畫面文字").await?;
            // 先切到前景再辨識：座標要對應「這個 App 的視窗」，而不是蓋在它上面的別的視窗。
            self.backend.activate(info.pid).await?;
            let mut items = self.backend.read_screen(Some(info.pid)).await?;
            sort_reading_order(&mut items);
            let hits = match_screen_text(&items, query);
            if hits.is_empty() {
                let seen: Vec<&str> = items.iter().take(30).map(|t| t.text.as_str()).collect();
                return Err(format!(
                    "畫面上找不到「{query}」。目前辨識到的文字有：{}",
                    if seen.is_empty() { "（沒有）".to_string() } else { seen.join("、") }
                ));
            }
            let pick = match (hits.len(), index) {
                (1, _) => hits[0],
                (n, None) => {
                    let list: Vec<String> =
                        hits.iter().enumerate().map(|(i, t)| format!("{}. {}", i + 1, format_screen_text(t))).collect();
                    return Err(format!("畫面上有 {n} 處符合「{query}」，請用 index（從 1 開始）指定哪一個：\n{}", list.join("\n")));
                }
                (n, Some(i)) if i >= 1 && i <= n => hits[i - 1],
                (n, Some(i)) => return Err(format!("index {i} 超出範圍（共 {n} 處符合）")),
            };
            if policy::is_dangerous_label(&pick.text) {
                self.confirm_destructive(&info, "點擊", &format!("文字「{}」", pick.text)).await?;
            }
            // 確認框會把 AITerm 叫到前景；點擊前要再切回目標 App（已在最前面時幾乎沒有成本）。
            self.backend.activate(info.pid).await?;
            let (x, y) = pick.center();
            self.backend.click_at(x, y, button, double).await?;
            Ok(ToolOutput::Text(format!("已點擊文字「{}」（座標 {x:.0},{y:.0}）", pick.text)))
        }
        .await;
        self.finish("computer_click_text", app, &format!("text={query}"), r)
    }

    /// 開啟前的授權：App 已在執行就沿用它的所有別名（bundle id、行程名稱、系統語言名稱），
    /// 操作過再開啟（或反過來）不會被問第二次；還沒執行就只能用名稱。
    async fn authorize_open(&self, name: &str, action: &str) -> Result<(), String> {
        let mut keys = vec![name.to_lowercase()];
        if let Ok(app) = self.resolve_app(name).await {
            if app.pid == self.own_pid {
                return Err("不可操作 AITerm 自己的視窗".to_string());
            }
            keys = vec![app_key(&app), app.name.to_lowercase()];
            keys.extend(app.localized.as_ref().map(|l| l.to_lowercase()));
            keys.push(name.to_lowercase());
        }
        self.authorize_any(keys, name, action).await
    }

    fn is_self_name(&self, name: &str) -> bool {
        let n = name.trim().to_lowercase();
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()));
        n == "aiterm" || exe.as_deref() == Some(n.as_str())
    }

    /// 開啟後等 App 出現在清單裡（剛啟動的 App 不在舊快取裡，也可能還要幾秒才出現）。
    /// 並清掉快取，讓下一個動作一定看得到它。`settle` 為 0（測試）時不等。
    async fn wait_for_app(&self, name: &str) -> bool {
        *self.apps.lock() = None;
        let q = name.trim().to_lowercase();
        let timeout = if self.settle().is_zero() { Duration::ZERO } else { Duration::from_secs(6) };
        let start = Instant::now();
        loop {
            if let Ok(apps) = self.fetch_apps().await {
                if apps.iter().any(|a| {
                    a.name.to_lowercase() == q
                        || a.localized.as_deref().map(str::to_lowercase).as_deref() == Some(q.as_str())
                        || a.bundle_id.as_deref().map(str::to_lowercase).as_deref() == Some(q.as_str())
                }) {
                    return true;
                }
            }
            if start.elapsed() >= timeout {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// 啟動（或帶到前景）一個 App。給 MCP 對話用：沒有這個工具時，模型只能寫出 shell 指令，
    /// 而對話模式不會執行指令。
    pub async fn open_app(&self, name: &str) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            let name = valid_app_name(name)?;
            if self.is_self_name(&name) {
                return Err("不可操作 AITerm 自己的視窗".to_string());
            }
            self.authorize_open(&name, "開啟這個 App").await?;
            self.backend.open_app(&name).await?;
            let seen = self.wait_for_app(&name).await;
            Ok(ToolOutput::Text(if seen {
                format!("已開啟 {name}")
            } else {
                format!("已送出開啟 {name} 的要求，但還沒在 App 清單看到它（可能還在啟動，稍後再操作；名稱也可能不對）")
            }))
        }
        .await;
        self.finish("computer_open_app", name, "", r)
    }

    /// 用預設瀏覽器（或指定的 App）開啟網址。只允許 http／https——其他協定（file、javascript、tel、
    /// line…）可能觸發撥號、讀檔或其他 App 的動作，不是「開網頁」。
    pub async fn open_url(&self, url: &str, app: Option<&str>) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            let url = valid_http_url(url)?;
            let app = match app.map(str::trim).filter(|a| !a.is_empty()) {
                Some(a) => Some(valid_app_name(a)?),
                None => None,
            };
            match &app {
                Some(a) => {
                    if self.is_self_name(a) {
                        return Err("不可操作 AITerm 自己的視窗".to_string());
                    }
                    self.authorize_open(a, "開啟網址").await?;
                }
                None => self.authorize("*default-browser*".to_string(), "預設瀏覽器", "開啟網址").await?,
            }
            self.backend.open_url(&url, app.as_deref()).await?;
            if let Some(a) = &app {
                self.wait_for_app(a).await;
            }
            Ok(ToolOutput::Text(match &app {
                Some(a) => format!("已用 {a} 開啟 {url}"),
                None => format!("已用預設瀏覽器開啟 {url}"),
            }))
        }
        .await;
        self.finish("computer_open_url", app.unwrap_or(""), url, r)
    }

    /// 找視窗某區域裡「只有圖形」的按鈕，由左到右編號（也標出從右數第幾個）。
    pub async fn find_icons(&self, app: &str, region: &str) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            self.require_screen_recording()?;
            let region = Region::parse(region)?;
            let info = self.gate(app, "尋找圖示").await?;
            self.backend.activate(info.pid).await?;
            let icons = self.backend.find_icons(info.pid, region).await?;
            if icons.is_empty() {
                return Ok(ToolOutput::Text(format!(
                    "{}沒有找到圖示（可能視窗被遮住、區域選錯，或圖示和背景對比太低）。可改用 region=all 或 computer_read_screen。",
                    region.label()
                )));
            }
            let total = icons.len();
            let mut lines = vec![format!("{}找到 {total} 個圖示（由左到右）：", region.label())];
            for (i, ic) in icons.iter().enumerate() {
                let (cx, cy) = ic.center();
                lines.push(format!("{}. ({cx:.0},{cy:.0}) 大小 {:.0}x{:.0}（從右數第 {}）", i + 1, ic.w, ic.h, total - i));
            }
            lines.push("圖示只有圖形、沒有文字，這裡無法知道每個代表什麼；請依該 App 的版面判斷，用 computer_click_icon 點選。".to_string());
            Ok(ToolOutput::Text(lines.join("\n")))
        }
        .await;
        self.finish("computer_find_icons", app, &format!("region={region}"), r)
    }

    /// 點視窗某區域裡的第 N 個圖示（可由右數）。圖示沒有文字、也沒有無障礙資訊，用途無從判斷，
    /// 所以一律逐次確認（與破壞性操作同等級）。
    pub async fn click_icon(&self, app: &str, region: &str, index: usize, from_right: bool, double: bool) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            self.require_accessibility()?;
            self.require_screen_recording()?;
            let region = Region::parse(region)?;
            if index < 1 {
                return Err("index 要從 1 開始".to_string());
            }
            let info = self.gate(app, "點擊圖示").await?;
            self.backend.activate(info.pid).await?;
            let icons = self.backend.find_icons(info.pid, region).await?;
            if index > icons.len() {
                return Err(format!(
                    "{}只有 {} 個圖示，沒有 index {index}。先用 computer_find_icons 看看有哪些。",
                    region.label(),
                    icons.len()
                ));
            }
            let pick = if from_right { &icons[icons.len() - index] } else { &icons[index - 1] };
            let side = if from_right { "右" } else { "左" };
            self.confirm_destructive(
                &info,
                "點擊圖示",
                &format!("點擊 {} 視窗{}由{side}數第 {index} 個圖示（只有圖形、沒有文字，無法確認用途）", info.name, region.label()),
            )
            .await?;
            // 確認框會把 AITerm 叫到前景；點擊前要再切回目標 App。
            self.backend.activate(info.pid).await?;
            let (x, y) = pick.center();
            self.backend.click_at(x, y, MouseButton::Left, double).await?;
            Ok(ToolOutput::Text(format!("已點擊{}由{side}數第 {index} 個圖示（座標 {x:.0},{y:.0}）", region.label())))
        }
        .await;
        self.finish("computer_click_icon", app, &format!("{region} #{index}{}", if from_right { " 從右" } else { "" }), r)
    }

    /// 依文字（標題／角色）找元件，不必讀整棵樹；找到的 id 可以直接拿去點。
    pub async fn find(&self, app: &str, query: &str, role: Option<&str>, max: usize) -> Result<ToolOutput, String> {
        let r = async {
            self.require_enabled()?;
            self.require_accessibility()?;
            let q = query.trim().to_lowercase();
            let role = role.map(str::trim).filter(|r| !r.is_empty());
            if q.is_empty() && role.is_none() {
                return Err("請提供 query（要找的文字）或 role（例如 AXButton）".to_string());
            }
            let info = self.gate(app, "查找元件").await?;
            let tree = self.backend.snapshot(info.pid, 12).await?;
            let mut seen = self.seen.lock();
            seen.retain(|(pid, _), _| *pid != info.pid);
            let mut all = Vec::new();
            collect_nodes(&tree, info.pid, &mut seen, &mut all);
            drop(seen);
            let max = max.clamp(1, 100);
            let hits: Vec<&ElementNode> = all
                .iter()
                .filter(|n| n.w > 0.0 && n.h > 0.0)
                .filter(|n| role.map_or(true, |r| n.role.eq_ignore_ascii_case(r)))
                .filter(|n| q.is_empty() || n.title.to_lowercase().contains(&q) || n.role.to_lowercase().contains(&q))
                .collect();
            if hits.is_empty() {
                return Ok(ToolOutput::Text(format!(
                    "沒有找到符合「{query}」的元件。可改用 computer_snapshot 看整體畫面。"
                )));
            }
            let mut lines: Vec<String> = hits.iter().take(max).map(|n| format_node(n)).collect();
            if hits.len() > max {
                lines.push(format!("…（僅列出前 {max} 筆，共 {} 筆符合；請用更精確的 query）", hits.len()));
            }
            Ok(ToolOutput::Text(lines.join("\n")))
        }
        .await;
        self.finish("computer_find", app, &format!("query={query}"), r)
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

/// App 名稱要能安全地當作 `open -a` 的參數：非空、不是旗標（`-` 開頭）、沒有控制字元。
fn valid_app_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    // 路徑分隔字元與引號也拒絕：授權對話框顯示的是「名稱」，不能讓名稱其實是任意路徑（Windows 的
    // ShellExecute 會把 `C:\\x\\evil.exe` 當成要執行的檔案）。
    if n.is_empty()
        || n.starts_with('-')
        || n.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '"' | '<' | '>' | '|' | '*' | '?'))
        || n.len() > 200
    {
        return Err("App 名稱不正確（不可為空、不可以 - 開頭、不可含控制字元、路徑分隔符號或引號）".to_string());
    }
    Ok(n.to_string())
}

/// 只接受 http／https 網址：有主機、沒有空白、控制字元或雙引號（Windows 的 ShellExecute 參數會用引號包起來）、長度合理。
fn valid_http_url(url: &str) -> Result<String, String> {
    let u = url.trim();
    let lower = u.to_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
    let bad = || "網址不正確：只接受 http:// 或 https:// 開頭、含主機名稱、沒有空白的網址".to_string();
    let Some(rest) = rest else { return Err(bad()) };
    if u.len() > 2048 || rest.is_empty() || rest.starts_with('/') || u.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"') {
        return Err(bad());
    }
    Ok(u.to_string())
}

fn count_content_nodes(n: &ElementNode) -> usize {
    let me = usize::from(
        !SCAFFOLD_ROLES.contains(&n.role.as_str()) && !GENERIC_ROLES.contains(&n.role.as_str()) && n.w > 0.0 && n.h > 0.0,
    );
    me + n.children.iter().map(count_content_nodes).sum::<usize>()
}

/// 由上到下、由左到右排序（同一行的 y 差小於字高的一半視為同一行）。
fn sort_reading_order(items: &mut [ScreenText]) {
    items.sort_by(|a, b| {
        let same_row = (a.y - b.y).abs() < a.h.min(b.h) / 2.0;
        if same_row { a.x.total_cmp(&b.x) } else { a.y.total_cmp(&b.y) }
    });
}

fn format_screen_text(t: &ScreenText) -> String {
    format!("\"{}\" ({:.0},{:.0},{:.0},{:.0})", t.text, t.x, t.y, t.w, t.h)
}

/// 在辨識結果裡找符合 `query` 的文字：完全相同優先，其次是包含（都不分大小寫）。
fn match_screen_text<'a>(items: &'a [ScreenText], query: &str) -> Vec<&'a ScreenText> {
    let q = query.trim().to_lowercase();
    let exact: Vec<&ScreenText> = items.iter().filter(|t| t.text.trim().to_lowercase() == q).collect();
    if !exact.is_empty() {
        return exact;
    }
    items.iter().filter(|t| t.text.to_lowercase().contains(&q)).collect()
}

fn format_node(n: &ElementNode) -> String {
    let title = if n.title.is_empty() { String::new() } else { format!(" \"{}\"", n.title) };
    format!("{} {}{} ({:.0},{:.0},{:.0},{:.0})", n.id, n.role, title, n.x, n.y, n.w, n.h)
}

/// 把整棵樹攤平，並把每個節點都登記進 `seen`（即使精簡輸出省略了它，id 仍可用）。
fn collect_nodes(
    n: &ElementNode,
    pid: i32,
    seen: &mut HashMap<(i32, String), (String, String)>,
    out: &mut Vec<ElementNode>,
) {
    seen.insert((pid, n.id.clone()), (n.role.clone(), n.title.clone()));
    out.push(ElementNode { children: vec![], ..n.clone() });
    for c in &n.children {
        collect_nodes(c, pid, seen, out);
    }
}

/// 精簡模式要不要省略這一行（孩子仍會處理）。根節點永遠保留。
fn omit_in_compact(n: &ElementNode, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    (n.w <= 0.0 || n.h <= 0.0) || (n.title.is_empty() && GENERIC_ROLES.contains(&n.role.as_str()))
}

fn render_tree(
    n: &ElementNode,
    depth: usize,
    indent: usize,
    compact: bool,
    pid: i32,
    seen: &mut HashMap<(i32, String), (String, String)>,
    out: &mut Vec<String>,
) {
    seen.insert((pid, n.id.clone()), (n.role.clone(), n.title.clone()));
    let omit = compact && omit_in_compact(n, depth);
    if !omit {
        out.push(format!("{}{}", "  ".repeat(indent), format_node(n)));
    }
    let next_indent = if omit { indent } else { indent + 1 };
    for c in &n.children {
        render_tree(c, depth + 1, next_indent, compact, pid, seen, out);
    }
}

#[cfg(test)]
mod tests;
