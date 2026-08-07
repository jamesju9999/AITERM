//! 供應商工具呼叫的不透明中繼資料快取。
//!
//! Gemini 的 OpenAI 相容端點會在 tool_call 的串流片段裡夾帶
//! `extra_content.google.thought_signature`；第二輪請求把 assistant 的
//! tool_call 回送時若沒有原樣帶回，上游回 400（"missing a thought_signature"）。
//! Anthropic 的 `tool_use` 區塊沒有欄位能承載這種 provider 專屬資料，所以
//! 改用伺服器端快取，以工具呼叫的 `id` 為鍵——`id` 是協定強制要原樣回送的
//! （`tool_result.tool_use_id` 靠它對應上游），這條路徑有保證；在 Anthropic
//! 區塊上塞自訂欄位則要賭 Claude Code 會保留未知欄位，未經驗證。
//!
//! 存整個 `extra_content` 的 JSON 值，不只存 `thought_signature`：這樣它是
//! provider 無關的「這個工具呼叫上游附了什麼不透明資料，回送時原樣帶回」，
//! 不是寫死 Gemini 的欄位路徑。
//!
//! 有界、無 TTL：容量到了就淘汰最舊的一筆。快取沒命中時的行為等於「沒有
//! 這個功能」——退回今天的 400，不會更糟，所以不需要 TTL 兜底。

use std::collections::{HashMap, VecDeque};

use parking_lot::Mutex;
use serde_json::Value;

/// 每筆 extra_content 約 400 bytes，512 筆約 200KB，遠低於需要在意的量級。
pub const DEFAULT_CAPACITY: usize = 512;

#[derive(Debug)]
pub struct ToolMetaCache {
    inner: Mutex<Inner>,
}

#[derive(Debug)]
struct Inner {
    map: HashMap<String, Value>,
    /// 插入順序，滿了之後從最前面（最舊）淘汰。
    order: VecDeque<String>,
    capacity: usize,
}

impl ToolMetaCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                order: VecDeque::new(),
                capacity,
            }),
        }
    }

    pub fn get(&self, id: &str) -> Option<Value> {
        self.inner.lock().map.get(id).cloned()
    }

    /// 覆蓋既有 id 不佔用額外容量、不影響淘汰順序；新 id 在滿了的時候會
    /// 先淘汰最舊的一筆再插入。
    pub fn insert(&self, id: String, value: Value) {
        let mut inner = self.inner.lock();
        if inner.map.contains_key(&id) {
            inner.map.insert(id, value);
            return;
        }
        if inner.capacity > 0 && inner.map.len() >= inner.capacity {
            if let Some(oldest) = inner.order.pop_front() {
                inner.map.remove(&oldest);
            }
        }
        inner.order.push_back(id.clone());
        inner.map.insert(id, value);
    }
}

impl Default for ToolMetaCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn missing_id_returns_none() {
        let cache = ToolMetaCache::new(4);
        assert_eq!(cache.get("nope"), None);
    }

    #[test]
    fn insert_then_get_roundtrips() {
        let cache = ToolMetaCache::new(4);
        cache.insert("id1".into(), json!({"a": 1}));
        assert_eq!(cache.get("id1"), Some(json!({"a": 1})));
    }

    #[test]
    fn overwriting_an_existing_id_updates_the_value() {
        let cache = ToolMetaCache::new(4);
        cache.insert("id1".into(), json!("first"));
        cache.insert("id1".into(), json!("second"));
        assert_eq!(cache.get("id1"), Some(json!("second")));
    }

    #[test]
    fn overwriting_does_not_consume_extra_capacity() {
        // 覆蓋不應該把同一個 id 算兩次容量，否則會提早淘汰不相關的項目。
        let cache = ToolMetaCache::new(2);
        cache.insert("id1".into(), json!(1));
        cache.insert("id1".into(), json!(2)); // 覆蓋，不佔用第二格
        cache.insert("id2".into(), json!(3));
        assert_eq!(cache.get("id1"), Some(json!(2)));
        assert_eq!(cache.get("id2"), Some(json!(3)));
    }

    #[test]
    fn exceeding_capacity_evicts_the_oldest_entry() {
        let cache = ToolMetaCache::new(2);
        cache.insert("id1".into(), json!(1));
        cache.insert("id2".into(), json!(2));
        cache.insert("id3".into(), json!(3)); // 應該淘汰 id1
        assert_eq!(cache.get("id1"), None, "最舊的一筆應該被淘汰");
        assert_eq!(cache.get("id2"), Some(json!(2)));
        assert_eq!(cache.get("id3"), Some(json!(3)));
    }

    #[test]
    fn is_thread_safe_behind_an_arc() {
        let cache = Arc::new(ToolMetaCache::new(64));
        let mut handles = Vec::new();
        for i in 0..16 {
            let cache = cache.clone();
            handles.push(thread::spawn(move || {
                let id = format!("id{i}");
                cache.insert(id.clone(), json!(i));
                cache.get(&id)
            }));
        }
        for (i, h) in handles.into_iter().enumerate() {
            assert_eq!(h.join().unwrap(), Some(json!(i)));
        }
    }
}
