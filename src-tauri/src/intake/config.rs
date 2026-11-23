//! 問題收集的使用者設定，以 JSON 存在 `intake_settings`。
use super::rules::{MatchMode, Settings};
use super::store;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchMode {
    Auto,
    Manual,
}

/// 問題派送到哪裡處理。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// 只給終端機分頁裡的 Claude Code。
    Terminal,
    /// 「要改程式」的問題建成工作看板卡片（需專案有程式庫路徑）；提問仍給終端機分頁。
    #[default]
    Board,
    /// 建成 Agent 協作桌布的卡：要改程式→bug 卡（審核者審查），提問→問答卡。沒有可用的工程師就退回看板。
    Canvas,
    /// 交給系統連線：用 Claude Code 無人模式讓 AI 在指定的外部網頁系統上完成，回報當回覆。
    System,
}

/// 一個管道收到的問題要怎麼處理。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handling {
    /// 只診斷與回答：一律視為提問，不做「要改程式」的初篩與修復驗證，並要求 AI 不修改檔案。
    Diagnose,
    /// 可改檔案：AI 初篩，要改程式的依派送目標處理並做修復驗證。
    Edit,
}

/// 每個管道各自的處理方式。信箱／Telegram／網頁表單維持原本行為（可改檔案），LINE 預設只診斷。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelHandling {
    pub email: Handling,
    pub telegram: Handling,
    pub line: Handling,
    pub web: Handling,
}

impl Default for ChannelHandling {
    fn default() -> Self {
        Self { email: Handling::Edit, telegram: Handling::Edit, line: Handling::Diagnose, web: Handling::Edit }
    }
}

/// 網頁表單的開放範圍。外網沒有獨立模式：`Lan` 綁 0.0.0.0，要從外網進來請自備反向代理／Tunnel。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebMode {
    Off,
    Local,
    Lan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    /// 用哪個郵件帳號收／寄（`MailAccountConfig.id`）。
    pub account_id: String,
    pub mode: MatchMode,
    pub alias: String,
    pub tag: String,
    pub auth_server: String,
    pub internal_without_auth: bool,
    pub dispatch_internal: DispatchMode,
    pub dispatch_external: DispatchMode,
    /// 內部回覆自動寄出；外部預設存草稿。
    pub reply_internal_auto: bool,
    pub reply_external_auto: bool,
    pub rate_limit_per_hour: u32,
    /// 「要改程式」的問題完成後在終端機目錄執行的測試指令（空＝不跑）。
    pub verify_command: String,
    /// 測試指令的逾時秒數。
    pub verify_timeout_secs: u64,
    /// 是否接收 Telegram 來的問題（需先在設定裡設好機器人）。
    pub telegram_enabled: bool,
    /// 是否接收 LINE 來的問題（需先設好官方帳號的憑證，並讓 LINE 連得到本機 webhook）。
    pub line_enabled: bool,
    /// LINE webhook 的本機埠號（所有專案共用）。
    pub line_port: u16,
    /// 白名單內的 LINE 群組：客戶沒 @ 時，新問題也自動開題（等客戶講完、判斷是新問題、且人員還沒回才開）。
    pub line_auto_open: bool,
    /// LINE 群組訊息在本機保存幾天（所有專案共用；0＝不保存，此時沒有前情也不能自動開題）。
    pub line_retention_days: u32,
    pub web_mode: WebMode,
    pub web_port: u16,
    /// 各管道的處理方式（只診斷／可改檔案）。
    pub channel_handling: ChannelHandling,
    /// 問題派送到哪裡處理。
    pub route: Route,
    /// 派送目標是系統連線時，要用哪個系統（`SystemConnection.id`）。
    pub system_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: String::new(),
            mode: MatchMode::AliasOrTag,
            alias: String::new(),
            tag: "[AITerm]".into(),
            auth_server: String::new(),
            internal_without_auth: false,
            dispatch_internal: DispatchMode::Auto,
            dispatch_external: DispatchMode::Manual,
            reply_internal_auto: true,
            reply_external_auto: false,
            rate_limit_per_hour: 10,
            verify_command: String::new(),
            verify_timeout_secs: 600,
            telegram_enabled: false,
            line_enabled: false,
            line_port: 8321,
            line_auto_open: true,
            line_retention_days: 30,
            web_mode: WebMode::Off,
            web_port: 8320,
            channel_handling: ChannelHandling::default(),
            route: Route::Board,
            system_id: String::new(),
        }
    }
}

impl Config {
    /// 依題目來源（`email`／`telegram`／`line`／`web`）取處理方式；不認得的來源保守沿用原行為（可改檔案）。
    pub fn handling(&self, source: &str) -> Handling {
        let h = &self.channel_handling;
        match source {
            "email" => h.email,
            "telegram" => h.telegram,
            "line" => h.line,
            "web" => h.web,
            _ => Handling::Edit,
        }
    }

    pub fn rules(&self) -> Settings {
        Settings {
            mode: self.mode,
            alias: self.alias.clone(),
            tag: self.tag.clone(),
            auth_server: self.auth_server.clone(),
            internal_without_auth: self.internal_without_auth,
        }
    }
}

/// 專案設定的存放鍵。空字串＝未歸屬（升級前的舊設定，沿用原本的鍵）。
fn key_for(project_id: &str) -> String {
    if project_id.is_empty() { "config".to_string() } else { format!("config:{project_id}") }
}

pub async fn load(pool: &SqlitePool, project_id: &str) -> Config {
    match store::get_setting(pool, &key_for(project_id)).await {
        Ok(Some(s)) => serde_json::from_str(&s).unwrap_or_default(),
        _ => Config::default(),
    }
}

pub async fn save(pool: &SqlitePool, project_id: &str, c: &Config) -> Result<(), sqlx::Error> {
    store::set_setting(pool, &key_for(project_id), &serde_json::to_string(c).unwrap_or_default()).await
}

/// 這個專案是否已有自己的設定（還沒存過的專案畫面上要提示「從既有設定複製」）。
pub async fn exists(pool: &SqlitePool, project_id: &str) -> bool {
    matches!(store::get_setting(pool, &key_for(project_id)).await, Ok(Some(_)))
}

/// 所有專案共用的設定：網頁表單只有一個伺服器、Telegram 只有一個機器人。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalConfig {
    pub web_port: u16,
    pub line_port: u16,
    pub line_retention_days: u32,
    /// 主人用 `/ask 問題` 而沒指定專案時，問題進哪個專案（空＝只有一個已啟用 Telegram 的專案時用它）。
    pub telegram_default_project: String,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self { web_port: 8320, line_port: 8321, line_retention_days: 30, telegram_default_project: String::new() }
    }
}

const GLOBAL_KEY: &str = "global";

pub async fn load_global(pool: &SqlitePool) -> GlobalConfig {
    match store::get_setting(pool, GLOBAL_KEY).await {
        Ok(Some(s)) => serde_json::from_str(&s).unwrap_or_default(),
        _ => {
            // 升級前埠號存在單一設定裡：第一次沿用它。
            let legacy = load(pool, "").await;
            GlobalConfig { web_port: legacy.web_port, ..GlobalConfig::default() }
        }
    }
}

pub async fn save_global(pool: &SqlitePool, g: &GlobalConfig) -> Result<(), sqlx::Error> {
    store::set_setting(pool, GLOBAL_KEY, &serde_json::to_string(g).unwrap_or_default()).await
}

/// 兩個專案的設定會不會讓同一封信有兩個歸屬。回傳衝突的欄位（`dedicated`／`alias`／`tag`）。
/// 只有「兩邊都啟用、用同一個信箱帳號」才可能衝突。
pub fn conflict(a: &Config, b: &Config) -> Option<&'static str> {
    use super::rules::MatchMode::*;
    if !a.enabled || !b.enabled || a.account_id.is_empty() || a.account_id != b.account_id {
        return None;
    }
    if a.mode == Dedicated || b.mode == Dedicated {
        return Some("dedicated");
    }
    let uses_alias = |m| matches!(m, Alias | AliasOrTag);
    let uses_tag = |m| matches!(m, SubjectTag | AliasOrTag);
    let same = |x: &str, y: &str| !x.trim().is_empty() && x.trim().eq_ignore_ascii_case(y.trim());
    if uses_alias(a.mode) && uses_alias(b.mode) && same(&a.alias, &b.alias) {
        return Some("alias");
    }
    if uses_tag(a.mode) && uses_tag(b.mode) && same(&a.tag, &b.tag) {
        return Some("tag");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::rules::MatchMode;

    fn c(mode: MatchMode, alias: &str, tag: &str) -> Config {
        Config { enabled: true, account_id: "acc".into(), mode, alias: alias.into(), tag: tag.into(), ..Config::default() }
    }

    #[test]
    fn handling_defaults_keep_old_channels_editable_and_make_line_diagnose_only() {
        let c = Config::default();
        assert_eq!(
            ["email", "telegram", "line", "web"].map(|s| c.handling(s)),
            [Handling::Edit, Handling::Edit, Handling::Diagnose, Handling::Edit]
        );
        assert_eq!(c.handling("something-new"), Handling::Edit, "不認得的來源沿用原行為");
    }

    #[test]
    fn a_config_saved_before_this_field_existed_still_loads_with_defaults() {
        let old: Config = serde_json::from_str(r#"{"enabled":true,"telegram_enabled":true}"#).unwrap();
        assert_eq!(old.channel_handling, ChannelHandling::default());
        let partial: Config = serde_json::from_str(r#"{"channel_handling":{"line":"edit"}}"#).unwrap();
        assert_eq!((partial.handling("line"), partial.handling("email")), (Handling::Edit, Handling::Edit));
        let json = serde_json::to_string(&Config::default()).unwrap();
        assert!(json.contains(r#""line":"diagnose""#), "{json}");
    }

    #[test]
    fn same_tag_or_alias_on_one_account_conflicts_but_different_ones_do_not() {
        assert_eq!(conflict(&c(MatchMode::SubjectTag, "", "[A]"), &c(MatchMode::SubjectTag, "", "[a]")), Some("tag"));
        assert_eq!(conflict(&c(MatchMode::Alias, "me+x@g.com", ""), &c(MatchMode::AliasOrTag, "ME+X@g.com", "[B]")), Some("alias"));
        assert_eq!(conflict(&c(MatchMode::AliasOrTag, "me+a@g.com", "[A]"), &c(MatchMode::AliasOrTag, "me+b@g.com", "[B]")), None);
    }

    #[test]
    fn a_dedicated_mailbox_cannot_be_shared() {
        assert_eq!(conflict(&c(MatchMode::Dedicated, "", ""), &c(MatchMode::SubjectTag, "", "[B]")), Some("dedicated"));
    }

    #[test]
    fn no_conflict_across_accounts_or_when_disabled_or_when_empty() {
        let a = c(MatchMode::SubjectTag, "", "[A]");
        assert_eq!(conflict(&a, &Config { account_id: "other".into(), ..a.clone() }), None);
        assert_eq!(conflict(&a, &Config { enabled: false, ..a.clone() }), None);
        assert_eq!(conflict(&c(MatchMode::AliasOrTag, "", ""), &c(MatchMode::AliasOrTag, "", "")), None, "空白欄位不算相同");
    }
}
