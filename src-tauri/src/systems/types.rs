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
