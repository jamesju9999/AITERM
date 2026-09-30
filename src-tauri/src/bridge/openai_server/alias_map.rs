//! 客戶端送來的 `model`（別名）→ AITerm 的 (provider_id, model)。
//!
//! 只做精確比對：OpenAI 的模型名稱區分大小寫，模糊比對會把請求默默導到
//! 使用者沒打算用的供應商——那是花錢／耗訂閱額度的錯誤，寧可回 404。

use crate::config::types::{ModelAlias, OpenAiServerConfig, TierMapping};

pub fn resolve(cfg: &OpenAiServerConfig, model: &str) -> Option<TierMapping> {
    // 重複別名時第一個贏：設定頁會擋重複，但手改 config.toml 時行為仍要確定。
    cfg.aliases.iter().find(|a| a.alias == model).map(|a| TierMapping {
        provider_id: a.provider_id.clone(),
        model: a.model.clone(),
    })
}

pub fn list(cfg: &OpenAiServerConfig) -> &[ModelAlias] {
    &cfg.aliases
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OpenAiServerConfig {
        OpenAiServerConfig {
            aliases: vec![
                ModelAlias { alias: "gpt-4o".into(), provider_id: "gemini".into(), model: "gemini-2.5-pro".into() },
                ModelAlias { alias: "gpt-4o".into(), provider_id: "other".into(), model: "x".into() },
                ModelAlias { alias: "local".into(), provider_id: "ollama".into(), model: "qwen3".into() },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn resolves_exact_alias() {
        let m = resolve(&cfg(), "local").unwrap();
        assert_eq!(m.provider_id, "ollama");
        assert_eq!(m.model, "qwen3");
    }

    #[test]
    fn unknown_alias_is_none() {
        assert!(resolve(&cfg(), "gpt-5").is_none());
    }

    #[test]
    fn alias_match_is_case_sensitive() {
        assert!(resolve(&cfg(), "GPT-4o").is_none());
    }

    #[test]
    fn duplicate_alias_first_wins() {
        assert_eq!(resolve(&cfg(), "gpt-4o").unwrap().provider_id, "gemini");
    }

    #[test]
    fn list_returns_all_aliases_in_order() {
        let c = cfg();
        let names: Vec<_> = list(&c).iter().map(|a| a.alias.as_str()).collect();
        assert_eq!(names, ["gpt-4o", "gpt-4o", "local"]);
    }
}
