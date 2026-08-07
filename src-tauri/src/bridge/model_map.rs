//! Claude Code 送來的 `model` 欄位 → AITerm 的 (provider_id, model)。
//!
//! 我們注入的是 `ANTHROPIC_DEFAULT_OPUS_MODEL=aiterm:opus` 這類哨兵字串，
//! Claude Code 會原樣放進請求的 `model`。這比比對 `claude-opus-4-...` 穩定
//! ——真實型號會隨 Claude Code 版本改變。子字串比對只是使用者手動覆寫時
//! 的後備路徑。

use crate::config::types::{ClaudeBridgeConfig, TierMapping};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Opus,
    Sonnet,
    Haiku,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Opus => "opus",
            Tier::Sonnet => "sonnet",
            Tier::Haiku => "haiku",
        }
    }
}

pub fn tier_for_model(model: &str) -> Option<Tier> {
    match model {
        "aiterm:opus" => return Some(Tier::Opus),
        "aiterm:sonnet" => return Some(Tier::Sonnet),
        "aiterm:haiku" => return Some(Tier::Haiku),
        _ => {}
    }
    // haiku 先比：它是最便宜的一層，誤判成 opus 的代價比反過來大。
    let lower = model.to_ascii_lowercase();
    if lower.contains("haiku") {
        Some(Tier::Haiku)
    } else if lower.contains("sonnet") {
        Some(Tier::Sonnet)
    } else if lower.contains("opus") {
        Some(Tier::Opus)
    } else {
        None
    }
}

/// 查出這個 model 字串該打到哪。錯誤訊息會直接回給 Claude Code 顯示，
/// 所以要寫成使用者看得懂、且指向設定頁的句子。
///
/// 回傳擁有權而非借用：借用會把生命週期綁在 `cfg` 上，對之後要在 async
/// handler 裡使用的呼叫端沒有幫助，而 `TierMapping` 已經是 `Clone`。
pub fn resolve(cfg: &ClaudeBridgeConfig, model: &str) -> Result<TierMapping, String> {
    let Some(tier) = tier_for_model(model) else {
        return Err(format!(
            "AITerm 橋接無法判斷模型「{model}」屬於哪一層。請在設定 → Claude Code 橋接檢查模型映射。"
        ));
    };
    let slot = match tier {
        Tier::Opus => &cfg.opus,
        Tier::Sonnet => &cfg.sonnet,
        Tier::Haiku => &cfg.haiku,
    };
    slot.clone().ok_or_else(|| {
        format!(
            "AITerm 橋接的 {} 層尚未設定供應商。請到設定 → Claude Code 橋接指定。",
            tier.as_str()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{ClaudeBridgeConfig, TierMapping};

    fn cfg_with_sonnet() -> ClaudeBridgeConfig {
        ClaudeBridgeConfig {
            sonnet: Some(TierMapping {
                provider_id: "local-qwen".into(),
                model: "Qwen3.6-35B".into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn sentinel_strings_win() {
        assert_eq!(tier_for_model("aiterm:opus"), Some(Tier::Opus));
        assert_eq!(tier_for_model("aiterm:sonnet"), Some(Tier::Sonnet));
        assert_eq!(tier_for_model("aiterm:haiku"), Some(Tier::Haiku));
    }

    #[test]
    fn falls_back_to_substring_for_real_model_names() {
        // 使用者手動覆寫了環境變數，或未來版本的 Claude Code 送真實型號。
        assert_eq!(tier_for_model("claude-opus-4-20250514"), Some(Tier::Opus));
        assert_eq!(tier_for_model("claude-3-5-haiku-latest"), Some(Tier::Haiku));
    }

    #[test]
    fn haiku_checked_before_sonnet_and_opus() {
        // 假想的複合名稱不能因為比對順序而誤判。
        assert_eq!(tier_for_model("sonnet-and-haiku-mix"), Some(Tier::Haiku));
    }

    #[test]
    fn unknown_model_yields_none() {
        assert_eq!(tier_for_model("gpt-4o"), None);
    }

    #[test]
    fn resolve_returns_mapping_for_configured_tier() {
        let m = resolve(&cfg_with_sonnet(), "aiterm:sonnet").unwrap();
        assert_eq!(m.provider_id, "local-qwen");
        assert_eq!(m.model, "Qwen3.6-35B");
    }

    #[test]
    fn resolve_errors_when_tier_unmapped() {
        let err = resolve(&cfg_with_sonnet(), "aiterm:opus").unwrap_err();
        assert!(err.contains("opus"), "訊息要指出是哪一層沒設定：{err}");
    }

    #[test]
    fn resolve_errors_on_unrecognised_model() {
        let err = resolve(&cfg_with_sonnet(), "gpt-4o").unwrap_err();
        assert!(err.contains("gpt-4o"));
    }
}
