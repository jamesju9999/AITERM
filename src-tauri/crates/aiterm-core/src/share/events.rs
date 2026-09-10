//! 把「有事情發生了」告訴上層的介面。
//!
//! server 本身不知道上層是 GUI（要發 Tauri 事件）還是無人值守的 CLI
//! （沒有任何 UI 要更新）。原本這裡是 `Option<tauri::AppHandle>`，那讓
//! server 綁死在 Tauri 上，headless 的 CLI host 因此不可能重用它。

use super::protocol::PendingRequestEvent;

pub trait ShareEvents: Send + Sync + 'static {
    /// 有人送出連線請求，正在等裁決。
    fn pending_request(&self, ev: &PendingRequestEvent);
    /// 觀看者名單或其存取層級變動了。
    fn viewers_changed(&self);
}

/// 什麼都不做的實作，給無人值守的 CLI host 用。
///
/// 刻意寫成一個有名字的型別而不是讓 server 收 `Option<impl ShareEvents>`：
/// 「CLI 模式下這些事件去哪了」應該在程式碼裡看得見，而不是靠一個 None。
pub struct SilentEvents;

impl ShareEvents for SilentEvents {
    fn pending_request(&self, _ev: &PendingRequestEvent) {}
    fn viewers_changed(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Default)]
    struct Counting {
        pending: AtomicUsize,
        viewers: AtomicUsize,
    }

    impl ShareEvents for Counting {
        fn pending_request(&self, _ev: &PendingRequestEvent) {
            self.pending.fetch_add(1, Ordering::SeqCst);
        }
        fn viewers_changed(&self) {
            self.viewers.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn a_share_events_impl_can_be_held_as_a_trait_object() {
        // server 會把它存成 `Arc<dyn ShareEvents>`。trait 若不是 object-safe
        // （例如哪天有人加了泛型方法），這裡會編譯失敗——那比在 server 那個
        // 大檔案裡發現要好。
        let counting = Arc::new(Counting::default());
        let as_dyn: Arc<dyn ShareEvents> = counting.clone();
        as_dyn.pending_request(&PendingRequestEvent {
            request_id: "r1".to_string(),
            tab_id: "t1".to_string(),
            display_name: "Alice".to_string(),
        });
        as_dyn.viewers_changed();
        assert_eq!(counting.pending.load(Ordering::SeqCst), 1);
        assert_eq!(counting.viewers.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn silent_events_does_nothing_and_does_not_panic() {
        let silent: Arc<dyn ShareEvents> = Arc::new(SilentEvents);
        silent.pending_request(&PendingRequestEvent {
            request_id: "r1".to_string(),
            tab_id: "t1".to_string(),
            display_name: "Alice".to_string(),
        });
        silent.viewers_changed();
    }
}
