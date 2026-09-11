//! 認證失敗的來源退避。
//!
//! 不是為了防爆破（256-bit 金鑰不需要），是為了擋 log flooding 與握手階段的
//! 資源耗用——CLI host 的埠常常直接暴露在網路上。

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// 失敗幾次之後開始延遲。前幾次不罰，因為使用者貼錯一次金鑰是常態。
const FREE_ATTEMPTS: u32 = 3;
/// 每次延遲的基數；實際延遲是 `BASE * 2^(failures - FREE_ATTEMPTS)`，上限 `MAX`。
const BASE: Duration = Duration::from_millis(500);
const MAX: Duration = Duration::from_secs(30);
/// 多久沒有新的失敗就把紀錄清掉。
const FORGET_AFTER: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct AuthBackoff {
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl AuthBackoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// 這個來源現在該被延遲多久。
    pub fn delay_for(&self, ip: IpAddr, now: Instant) -> Duration {
        let mut map = self.failures.lock();
        map.retain(|_, (_, last)| now.duration_since(*last) < FORGET_AFTER);
        let Some((count, _)) = map.get(&ip) else { return Duration::ZERO };
        if *count <= FREE_ATTEMPTS {
            return Duration::ZERO;
        }
        let exp = (*count - FREE_ATTEMPTS).min(16);
        BASE.saturating_mul(1u32 << exp).min(MAX)
    }

    pub fn record_failure(&self, ip: IpAddr, now: Instant) {
        let mut map = self.failures.lock();
        let entry = map.entry(ip).or_insert((0, now));
        entry.0 += 1;
        entry.1 = now;
    }

    /// 認證成功就清掉這個來源的紀錄。
    pub fn record_success(&self, ip: IpAddr) {
        self.failures.lock().remove(&ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, n])
    }

    #[test]
    fn a_fresh_source_is_not_delayed() {
        let b = AuthBackoff::new();
        assert_eq!(b.delay_for(ip(1), Instant::now()), Duration::ZERO);
    }

    #[test]
    fn the_first_few_failures_are_free() {
        // 貼錯一次金鑰是常態，不該讓使用者等。
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..FREE_ATTEMPTS {
            b.record_failure(ip(1), now);
        }
        assert_eq!(b.delay_for(ip(1), now), Duration::ZERO);
    }

    #[test]
    fn the_delay_grows_after_the_free_attempts() {
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..(FREE_ATTEMPTS + 1) {
            b.record_failure(ip(1), now);
        }
        let first = b.delay_for(ip(1), now);
        assert!(first > Duration::ZERO, "got {first:?}");

        b.record_failure(ip(1), now);
        let second = b.delay_for(ip(1), now);
        assert!(second > first, "delay must grow: {first:?} -> {second:?}");
    }

    #[test]
    fn the_delay_is_capped() {
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..100 {
            b.record_failure(ip(1), now);
        }
        assert_eq!(b.delay_for(ip(1), now), MAX);
    }

    #[test]
    fn one_source_does_not_delay_another() {
        // 沒有這條的話，任何人都能用一台機器狂試金鑰，把合法使用者一起鎖住。
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..50 {
            b.record_failure(ip(1), now);
        }
        assert_eq!(b.delay_for(ip(2), now), Duration::ZERO);
    }

    #[test]
    fn a_success_clears_the_record() {
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..50 {
            b.record_failure(ip(1), now);
        }
        b.record_success(ip(1));
        assert_eq!(b.delay_for(ip(1), now), Duration::ZERO);
    }

    #[test]
    fn old_records_are_forgotten() {
        let b = AuthBackoff::new();
        let start = Instant::now();
        for _ in 0..50 {
            b.record_failure(ip(1), start);
        }
        let much_later = start + FORGET_AFTER + Duration::from_secs(1);
        assert_eq!(b.delay_for(ip(1), much_later), Duration::ZERO);
    }
}
