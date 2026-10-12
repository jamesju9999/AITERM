//! 平台後端的介面與資料型別。`ComputerService` 只認這個 trait，
//! 所以 policy／授權流程可以用假後端完整測試，不必真的去點別人的 App。

use async_trait::async_trait;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AppInfo {
    /// 行程名稱（可執行檔名，如 `TextEdit`、`Notes`），與舊版 osascript 清單相同，模型與使用者最常用的叫法。
    pub name: String,
    /// 系統語言下顯示的名稱（如「文字編輯」）；與 `name` 相同時為 `None`。也能拿來指定 App。
    pub localized: Option<String>,
    pub bundle_id: Option<String>,
    pub pid: i32,
    pub frontmost: bool,
}

/// 無障礙元件樹的一個節點。`id` 是從 App 根節點算起的子節點索引路徑（如 `0.3.1`）。
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct ElementNode {
    pub id: String,
    pub role: String,
    pub title: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub children: Vec<ElementNode>,
}

impl ElementNode {
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// 截圖：PNG 位元組，已縮到邏輯像素；`origin_*` 是它在螢幕座標系的左上角。
#[derive(Debug, Clone)]
pub struct Shot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub origin_x: f64,
    pub origin_y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
}

/// 已解析的按鍵組合。`key` 是小寫的標準名稱（`s`、`return`、`left`…）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCombo {
    pub key: String,
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// 螢幕上辨識到的一段文字，座標是螢幕邏輯座標（左上角為原點，與點擊座標一致）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenText {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl ScreenText {
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// 目前焦點元件的狀態，操作後回報給模型驗證結果用。
#[derive(Debug, Clone, PartialEq)]
pub struct FocusedText {
    pub role: String,
    pub text: String,
    /// 密碼欄位：內容絕不回傳。
    pub secure: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    pub accessibility: bool,
    pub screen_recording: bool,
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn permissions(&self) -> Permissions;
    async fn list_apps(&self) -> Result<Vec<AppInfo>, String>;
    /// 讀元件樹；深度由 `max_depth` 限制（根為 0）。
    async fn snapshot(&self, pid: i32, max_depth: usize) -> Result<ElementNode, String>;
    /// 只取單一節點（不含子節點），用來在點擊前確認 id 沒有過期。
    async fn element_info(&self, pid: i32, id: &str) -> Result<ElementNode, String>;
    /// 螢幕座標處的元件（跨 App），用來判斷座標點擊會點到什麼。
    async fn element_at(&self, x: f64, y: f64) -> Result<Option<ElementNode>, String>;
    async fn screenshot(&self, pid: Option<i32>) -> Result<Shot, String>;
    async fn activate(&self, pid: i32) -> Result<(), String>;
    /// 對元件做 AXPress。
    async fn press_element(&self, pid: i32, id: &str) -> Result<(), String>;
    async fn click_at(&self, x: f64, y: f64, button: MouseButton, double: bool) -> Result<(), String>;
    async fn type_text(&self, text: &str) -> Result<(), String>;
    async fn focus_element(&self, pid: i32, id: &str) -> Result<(), String>;
    async fn key(&self, combo: &KeyCombo) -> Result<(), String>;
    async fn scroll(&self, dx: i32, dy: i32, at: Option<(f64, f64)>) -> Result<(), String>;
    /// 對畫面（整個螢幕，或某個 App 的前景視窗）做文字辨識，回傳每段文字與它的螢幕座標。
    /// 給不提供無障礙資訊的 App（Chromium／Qt，如 LINE）用。
    async fn read_screen(&self, pid: Option<i32>) -> Result<Vec<ScreenText>, String>;
    /// App 目前焦點元件的角色與文字；沒有焦點元件回 `None`。
    async fn focused_text(&self, pid: i32) -> Result<Option<FocusedText>, String>;
}

/// 非 macOS：介面保留，一律回「尚未支援」。
pub struct UnsupportedBackend;

fn unsupported<T>() -> Result<T, String> {
    Err("目前只支援 macOS（Windows／Linux 尚未實作）".to_string())
}

#[async_trait]
impl Backend for UnsupportedBackend {
    fn permissions(&self) -> Permissions {
        Permissions { accessibility: false, screen_recording: false }
    }
    async fn list_apps(&self) -> Result<Vec<AppInfo>, String> { unsupported() }
    async fn snapshot(&self, _: i32, _: usize) -> Result<ElementNode, String> { unsupported() }
    async fn element_info(&self, _: i32, _: &str) -> Result<ElementNode, String> { unsupported() }
    async fn element_at(&self, _: f64, _: f64) -> Result<Option<ElementNode>, String> { unsupported() }
    async fn screenshot(&self, _: Option<i32>) -> Result<Shot, String> { unsupported() }
    async fn activate(&self, _: i32) -> Result<(), String> { unsupported() }
    async fn press_element(&self, _: i32, _: &str) -> Result<(), String> { unsupported() }
    async fn click_at(&self, _: f64, _: f64, _: MouseButton, _: bool) -> Result<(), String> { unsupported() }
    async fn type_text(&self, _: &str) -> Result<(), String> { unsupported() }
    async fn focus_element(&self, _: i32, _: &str) -> Result<(), String> { unsupported() }
    async fn key(&self, _: &KeyCombo) -> Result<(), String> { unsupported() }
    async fn scroll(&self, _: i32, _: i32, _: Option<(f64, f64)>) -> Result<(), String> { unsupported() }
    async fn read_screen(&self, _: Option<i32>) -> Result<Vec<ScreenText>, String> { unsupported() }
    async fn focused_text(&self, _: i32) -> Result<Option<FocusedText>, String> { unsupported() }
}
