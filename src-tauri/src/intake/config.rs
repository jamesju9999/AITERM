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

const KEY: &str = "config";

pub async fn load(pool: &SqlitePool) -> Config {
    match store::get_setting(pool, KEY).await {
        Ok(Some(s)) => serde_json::from_str(&s).unwrap_or_default(),
        _ => Config::default(),
    }
}

pub async fn save(pool: &SqlitePool, c: &Config) -> Result<(), sqlx::Error> {
    store::set_setting(pool, KEY, &serde_json::to_string(c).unwrap_or_default()).await
}
