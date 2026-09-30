//! 對外的 OpenAI 相容 server：把 `/v1/chat/completions` 請求翻譯到 AITerm
//! 已設定的任一 AI 供應商。與 Claude Code 橋接（`bridge::server`）平行、
//! 互不影響，但共用同一套上游 adapter 與中立事件（`upstream::UpstreamEvent`）。
//!
//! 設計見 `docs/superpowers/specs/2026-09-30-openai-compatible-server-design.md`。

pub mod alias_map;
pub mod anthropic_events;
pub mod request;
pub mod response;
pub mod server;

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::types::OpenAiServerConfig;
use crate::config::ConfigStore;
use crate::secret::SecretStore;
use crate::bridge::tool_meta::ToolMetaCache;

/// server 的生命週期：持有目前執行中 server 的 handle，能 start/stop。
/// 結構與 `bridge::BridgeState` 相同，但綁定位址可選 loopback 或全介面。
#[derive(Default)]
pub struct OpenAiServerState {
    running: Mutex<Option<Running>>,
}

struct Running {
    port: u16,
    lan: bool,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl OpenAiServerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.port)
    }

    pub fn lan(&self) -> bool {
        self.running.lock().as_ref().is_some_and(|r| r.lan)
    }

    /// 已經在跑就先停掉（換埠或換綁定範圍時會用到）。埠被占用時回錯誤而不是換一個。
    pub async fn start(
        &self,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        token: String,
        port: u16,
        allow_lan: bool,
    ) -> anyhow::Result<()> {
        self.stop();

        let addr = SocketAddr::from((bind_ip(allow_lan), port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| anyhow::anyhow!("無法綁定 {addr}（{e}）。請在設定裡換一個埠。"))?;

        let app = server::router(
            server::AppState {
                config,
                secrets,
                token: Arc::new(token),
                tool_meta: Arc::new(ToolMetaCache::default()),
            },
            allow_lan,
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let served = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
            if let Err(e) = served {
                log::error!("openai-server 結束於錯誤：{e}");
            }
        });

        *self.running.lock() = Some(Running { port, lan: allow_lan, shutdown: tx });
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
        }
    }
}

/// 綁定位址。抽成純函式是為了能測：測試裡真的綁 0.0.0.0 會讓 macOS 跳防火牆對話框。
fn bind_ip(allow_lan: bool) -> [u8; 4] {
    if allow_lan { [0, 0, 0, 0] } else { [127, 0, 0, 1] }
}

/// 存檔前的驗證。錯誤訊息直接顯示在設定頁。
pub fn validate(cfg: &OpenAiServerConfig) -> Result<(), String> {
    if cfg.port < 1024 {
        return Err("埠必須在 1024–65535 之間。".into());
    }
    let mut seen = std::collections::HashSet::new();
    for a in &cfg.aliases {
        if a.alias.trim().is_empty() {
            return Err("別名不可空白。".into());
        }
        if a.provider_id.trim().is_empty() || a.model.trim().is_empty() {
            return Err(format!("別名「{}」尚未選擇供應商與模型。", a.alias));
        }
        if !seen.insert(a.alias.as_str()) {
            return Err(format!("別名「{}」重複了。", a.alias));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::ModelAlias;

    fn alias(a: &str, p: &str, m: &str) -> ModelAlias {
        ModelAlias { alias: a.into(), provider_id: p.into(), model: m.into() }
    }

    #[test]
    fn default_config_is_valid() {
        assert!(validate(&OpenAiServerConfig::default()).is_ok());
    }

    #[test]
    fn rejects_privileged_ports() {
        let cfg = OpenAiServerConfig { port: 80, ..Default::default() };
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn rejects_duplicate_and_blank_aliases() {
        let dup = OpenAiServerConfig {
            aliases: vec![alias("a", "p", "m"), alias("a", "q", "n")],
            ..Default::default()
        };
        assert!(validate(&dup).unwrap_err().contains("重複"));

        let blank = OpenAiServerConfig { aliases: vec![alias("  ", "p", "m")], ..Default::default() };
        assert!(validate(&blank).is_err());

        let no_model = OpenAiServerConfig { aliases: vec![alias("a", "p", "")], ..Default::default() };
        assert!(validate(&no_model).is_err());
    }

    #[test]
    fn bind_ip_is_loopback_unless_lan_allowed() {
        assert_eq!(bind_ip(false), [127, 0, 0, 1]);
        assert_eq!(bind_ip(true), [0, 0, 0, 0]);
    }

    /// 真的綁一次 loopback 埠，證明起得來、停得掉。
    #[tokio::test]
    async fn starts_and_stops_on_a_real_socket() {
        let dir = tempfile::tempdir().unwrap();
        let state = OpenAiServerState::new();
        let config = Arc::new(ConfigStore::new_at(dir.path().join("config.toml")));
        let secrets = Arc::new(SecretStore::new());

        // 0 埠會被 validate 擋掉，但 start 本身接受，讓 OS 分配空閒埠。
        state.start(config, secrets, "t".into(), 0, false).await.unwrap();
        assert!(!state.lan());
        assert_eq!(state.port(), Some(0));
        state.stop();
        assert_eq!(state.port(), None);
    }
}
