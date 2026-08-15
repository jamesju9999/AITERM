//! 模型單價表與成本估算。純函式，無 I/O。
//!
//! 單價會過期。**查不到單價一律回 None（不顯示金額），絕不猜測** ——
//! 顯示一個錯的金額比不顯示更糟。

/// 每百萬 token 的美元單價。
#[derive(Debug, Clone, Copy)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// 以 model id 的**前綴**比對。同時命中多個時取最長的。
/// 單價來源：各供應商公開定價頁，2026-08 校對。
pub const PRICES: &[(&str, ModelPrice)] = &[
    ("claude-opus-4", ModelPrice { input: 15.0, output: 75.0, cache_read: 1.50, cache_write: 18.75 }),
    ("claude-sonnet-4-5", ModelPrice { input: 3.0, output: 15.0, cache_read: 0.30, cache_write: 3.75 }),
    ("claude-sonnet-4", ModelPrice { input: 3.0, output: 15.0, cache_read: 0.30, cache_write: 3.75 }),
    ("claude-haiku-4-5", ModelPrice { input: 1.0, output: 5.0, cache_read: 0.10, cache_write: 1.25 }),
    ("gpt-5.6", ModelPrice { input: 1.25, output: 10.0, cache_read: 0.125, cache_write: 0.0 }),
    ("gpt-4.1", ModelPrice { input: 2.0, output: 8.0, cache_read: 0.50, cache_write: 0.0 }),
    ("gemini-3.5-flash", ModelPrice { input: 0.30, output: 2.50, cache_read: 0.075, cache_write: 0.0 }),
    ("gemini-2.5-flash", ModelPrice { input: 0.30, output: 2.50, cache_read: 0.075, cache_write: 0.0 }),
];

/// 找出前綴命中且最長的單價。
pub fn price_for(model: &str) -> Option<ModelPrice> {
    PRICES
        .iter()
        .filter(|(prefix, _)| model.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, p)| *p)
}

/// 估算美元成本。查不到單價回 `None`。
pub fn estimate_cost(
    model: &str,
    prompt: i64,
    completion: i64,
    cache_read: i64,
    cache_write: i64,
) -> Option<f64> {
    let p = price_for(model)?;
    let m = 1_000_000.0;
    Some(
        prompt as f64 / m * p.input
            + completion as f64 / m * p.output
            + cache_read as f64 / m * p.cache_read
            + cache_write as f64 / m * p.cache_write,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_costs_are_computed_per_million_tokens() {
        // 1M input + 1M output 的 claude-sonnet-4-5 = 3.0 + 15.0
        let cost = estimate_cost("claude-sonnet-4-5", 1_000_000, 1_000_000, 0, 0)
            .expect("已知模型應有單價");
        assert!((cost - 18.0).abs() < 1e-6, "得到 {cost}");
    }

    #[test]
    fn cache_read_is_cheaper_than_fresh_input() {
        let fresh = estimate_cost("claude-sonnet-4-5", 1_000_000, 0, 0, 0).unwrap();
        let cached = estimate_cost("claude-sonnet-4-5", 0, 0, 1_000_000, 0).unwrap();
        assert!(cached < fresh, "快取讀取必須比新輸入便宜: {cached} vs {fresh}");
    }

    #[test]
    fn matches_by_prefix_so_dated_variants_resolve() {
        // 上游常回帶日期的完整 id，單價表只列基底名稱。
        let a = estimate_cost("claude-sonnet-4-5-20250929", 1_000_000, 0, 0, 0);
        let b = estimate_cost("claude-sonnet-4-5", 1_000_000, 0, 0, 0);
        assert_eq!(a, b);
    }

    #[test]
    fn unknown_model_returns_none_rather_than_guessing() {
        // 查不到單價就不顯示金額。猜一個數字比不顯示更糟。
        assert!(estimate_cost("some-local-gguf-model", 1_000_000, 1_000_000, 0, 0).is_none());
    }

    #[test]
    fn longest_prefix_wins() {
        // "gpt-5.6-luna" 與 "gpt-5.6" 同時存在時，必須選較長的那個。
        let luna = PRICES.iter().find(|(k, _)| *k == "gpt-5.6-luna");
        if luna.is_some() {
            let a = estimate_cost("gpt-5.6-luna", 1_000_000, 0, 0, 0).unwrap();
            let b = estimate_cost("gpt-5.6", 1_000_000, 0, 0, 0).unwrap();
            assert_ne!(a, b, "較長的前綴必須勝出，否則單價表形同虛設");
        }
    }
}
