use serde::{Deserialize, Serialize};

/// 寫入權限模式（每個系統各自一個）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// 非 GET 一律拒絕。
    #[default]
    ReadOnly,
    /// 每個非 GET 都等使用者核准。
    ApproveEach,
    /// 依 `rules` 判定，沒命中的等核准。
    Rules,
    /// 全部放行，只寫審計。
    Autonomous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Allow,
    Ask,
}

/// `rules` 模式的一條規則。`method` 為 `*` 代表任何方法；`url_pattern` 支援 `*` 萬用字元。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub method: String,
    pub url_pattern: String,
    pub action: RuleAction,
}

/// 系統連線的種類：網頁（Playwright）或桌面 App（電腦操作）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SystemKind {
    #[default]
    Web,
    Desktop,
}

/// 存在 AppConfig 裡的一個系統連線。不含任何憑證。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemConnection {
    pub id: String,
    pub name: String,
    pub start_url: String,
    /// 允許頂層導覽的網域。子網域也算（`example.com` 允許 `app.example.com`）。
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// AI 操作這個系統用的 Claude 模型（`haiku`／`sonnet`／`opus` 或完整名稱）。`None`＝沿用 Claude Code 的預設。
    /// 定時任務與「請 AI 操作」可以個別覆蓋；問題收集一律用這個。
    #[serde(default)]
    pub model: Option<String>,
    /// AI 可以從這些資料夾挑檔案上傳到網頁（`sys_file_upload`）。空＝不能上傳。
    #[serde(default)]
    pub upload_dirs: Vec<String>,
    /// 連線種類；舊資料沒有這個欄位，視為網頁。
    #[serde(default)]
    pub kind: SystemKind,
    /// 桌面類型：綁定的 App（名稱或套件 ID，電腦操作用的識別字串）。AI 只能操作它。
    #[serde(default)]
    pub app: String,
    /// 桌面類型：App 沒開時由 AITerm 啟動。
    #[serde(default = "default_true")]
    pub launch_app: bool,
}

impl Default for SystemConnection {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            start_url: String::new(),
            allowed_hosts: vec![],
            mode: Mode::default(),
            rules: vec![],
            enabled: true,
            model: None,
            upload_dirs: vec![],
            kind: SystemKind::Web,
            app: String::new(),
            launch_app: true,
        }
    }
}

impl SystemConnection {
    pub fn is_desktop(&self) -> bool {
        self.kind == SystemKind::Desktop
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    Disabled,
    ReadOnly,
    HostNotAllowed,
}

/// 閘門對一個請求的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", content = "reason", rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny(DenyReason),
    /// 暫停請求，等使用者核准。
    Ask,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_json_without_kind_is_a_web_system() {
        let c: SystemConnection = serde_json::from_str(
            r#"{"id":"a","name":"A","start_url":"https://x/","allowed_hosts":["x"]}"#,
        )
        .unwrap();
        assert_eq!(c.kind, SystemKind::Web);
        assert!(!c.is_desktop());
        assert!(c.launch_app && c.enabled);
        assert_eq!(c.app, "");
    }

    #[test]
    fn desktop_kind_round_trips_as_snake_case() {
        let c = SystemConnection { kind: SystemKind::Desktop, app: "Notes".into(), ..Default::default() };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["kind"], "desktop");
        let back: SystemConnection = serde_json::from_value(v).unwrap();
        assert!(back.is_desktop() && back.app == "Notes");
    }
}
