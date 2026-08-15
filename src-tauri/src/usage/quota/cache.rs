//! 配額快照的記憶體快取。
//!
//! 配額變動以分鐘計，但 UI 每次開下拉就要顯示、又有 5 分鐘的背景輪詢，
//! 多個視圖同時掛載時若不快取會重複打上游。60 秒 TTL 讓「多視圖同時掛載」
//! 只產生一次實際請求。

use super::{ProviderQuota, QuotaSource};
use crate::ai::AiError;
#[cfg(test)]
use async_trait::async_trait;
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Arc;
use std::sync::Mutex;

const TTL_SECS: i64 = 60;

#[derive(Default)]
pub struct QuotaCache {
    entries: Mutex<HashMap<String, ProviderQuota>>,
}

impl QuotaCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 快取命中且未過期就直接回；否則呼叫 `source.fetch()`。
    /// `now` 由呼叫端注入以便測試。
    pub async fn get_or_fetch(
        &self,
        provider_id: &str,
        source: &dyn QuotaSource,
        force: bool,
        now: i64,
    ) -> Result<ProviderQuota, AiError> {
        if !force {
            let guard = self.entries.lock().unwrap();
            if let Some(hit) = guard.get(provider_id) {
                if now - hit.fetched_at < TTL_SECS {
                    return Ok(hit.clone());
                }
            }
        }
        let mut fresh = source.fetch().await?;
        // fetched_at 以呼叫端的時鐘為準，避免 adapter 與快取用到不同時間源。
        fresh.fetched_at = now;
        self.entries
            .lock()
            .unwrap()
            .insert(provider_id.to_string(), fresh.clone());
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSource {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl QuotaSource for CountingSource {
        async fn fetch(&self) -> Result<ProviderQuota, AiError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ProviderQuota {
                provider_id: "p".into(), plan: None, windows: vec![], fetched_at: 0,
            })
        }
    }

    #[tokio::test]
    async fn second_call_within_ttl_does_not_refetch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = QuotaCache::new();
        let src = CountingSource { calls: calls.clone() };

        cache.get_or_fetch("p", &src, false, 1_000).await.expect("first");
        cache.get_or_fetch("p", &src, false, 1_030).await.expect("second");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "30 秒內不該重打");
    }

    #[tokio::test]
    async fn call_after_ttl_refetches() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = QuotaCache::new();
        let src = CountingSource { calls: calls.clone() };

        cache.get_or_fetch("p", &src, false, 1_000).await.expect("first");
        cache.get_or_fetch("p", &src, false, 1_061).await.expect("second");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "超過 60 秒要重打");
    }

    #[tokio::test]
    async fn force_bypasses_cache() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = QuotaCache::new();
        let src = CountingSource { calls: calls.clone() };

        cache.get_or_fetch("p", &src, false, 1_000).await.expect("first");
        cache.get_or_fetch("p", &src, true, 1_001).await.expect("forced");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn different_providers_have_separate_entries() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = QuotaCache::new();
        let src = CountingSource { calls: calls.clone() };

        cache.get_or_fetch("a", &src, false, 1_000).await.expect("a");
        cache.get_or_fetch("b", &src, false, 1_001).await.expect("b");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "不同 provider 不共用快取");
    }
}
