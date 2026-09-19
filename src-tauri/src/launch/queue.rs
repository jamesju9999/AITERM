use super::parse::LaunchRequest;
use std::sync::Mutex;

/// 還沒被前端取走的請求。請求先存在這裡、再通知前端，前端訂閱之前入列的
/// 請求不會遺失——事件本身不承載資料。
#[derive(Default)]
pub struct LaunchQueue(Mutex<Vec<LaunchRequest>>);

impl LaunchQueue {
    pub fn push(&self, requests: Vec<LaunchRequest>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).extend(requests);
    }

    /// 取走並清空。
    pub fn take(&self) -> Vec<LaunchRequest> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(cwd: &str) -> LaunchRequest {
        LaunchRequest { cwd: Some(cwd.into()), script: None, command: None }
    }

    #[test]
    fn take_returns_pushed_requests_in_order_and_empties_the_queue() {
        let q = LaunchQueue::default();
        q.push(vec![req("/a")]);
        q.push(vec![req("/b"), req("/c")]);
        assert_eq!(q.take(), vec![req("/a"), req("/b"), req("/c")]);
        assert!(q.take().is_empty(), "second take must find nothing");
    }

    #[test]
    fn take_on_an_untouched_queue_is_empty() {
        assert!(LaunchQueue::default().take().is_empty());
    }
}
