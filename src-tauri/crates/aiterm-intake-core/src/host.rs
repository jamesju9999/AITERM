//! 與平台有關的三個接口：祕密儲存、資料變動通知、一次性 AI 補全。
//! AITERM 與 server 各自實作，core 只認這些 trait。
use std::sync::Arc;

/// 祕密儲存（LINE 憑證、郵件密碼、AI 金鑰）。AITERM 用系統鑰匙圈，server 用檔案金鑰。
pub trait Secrets: Send + Sync {
    fn get(&self, id: &str) -> Option<String>;
    fn set(&self, id: &str, value: &str) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// 資料變動通知（AITERM：emit 事件；server：推播給 WebSocket）。
pub trait Notifier: Send + Sync {
    fn changed(&self);
}

/// 一次性文字補全（系統提示加使用者文字），供題目分類與「是不是新問題」使用。
#[async_trait::async_trait]
pub trait AiComplete: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<String, String>;
}

pub type SharedSecrets = Arc<dyn Secrets>;
pub type SharedNotifier = Arc<dyn Notifier>;
pub type SharedAi = Arc<dyn AiComplete>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Count(AtomicUsize);
    impl Notifier for Count {
        fn changed(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn a_shared_notifier_can_be_called_through_the_trait_object() {
        let n = Arc::new(Count(AtomicUsize::new(0)));
        let shared: SharedNotifier = n.clone();
        shared.changed();
        shared.changed();
        assert_eq!(n.0.load(Ordering::SeqCst), 2);
    }
}
