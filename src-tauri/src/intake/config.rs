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
    pub web_mode: WebMode,
    pub web_port: u16,
    /// 「要改程式」的問題建成看板卡片（需專案有程式庫路徑）；關掉就全部走終端機分頁。
    pub route_to_board: bool,
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
            web_mode: WebMode::Off,
            web_port: 8320,
            route_to_board: true,
        }
    }
}

impl Config {
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
    /// 主人用 `/ask 問題` 而沒指定專案時，問題進哪個專案（空＝只有一個已啟用 Telegram 的專案時用它）。
    pub telegram_default_project: String,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self { web_port: 8320, telegram_default_project: String::new() }
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
