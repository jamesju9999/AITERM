//! 桌面操作會和人搶同一組滑鼠鍵盤，所以要知道「現在有沒有人在用電腦」：
//! 無人執行前先確認閒置、執行中偵測到別人的輸入就暫停。

use std::time::Duration;

/// 無人執行至少要閒置這麼久才開始。
pub const IDLE_NEEDED: Duration = Duration::from_secs(60);
/// 還沒閒置時，每隔多久再查一次。
pub const DEFER_STEP: Duration = Duration::from_secs(30);
/// 累計延後超過這麼久就放棄（記為失敗並通知）。
pub const MAX_DEFER: Duration = Duration::from_secs(30 * 60);
/// 比對「使用者有沒有動」時容許的誤差（系統時鐘與事件時間戳的差）。
pub const TOLERANCE: Duration = Duration::from_millis(1500);

/// 讀取目前的使用者閒置狀態。取不到的環境（例如 Wayland）回 `None`，功能降級。
pub trait InputMonitor: Send + Sync {
    /// 距離上一次**任何**輸入（含 AITerm 自己送的）過了幾秒。
    fn idle(&self) -> Option<Duration>;
    /// 螢幕是否鎖定（鎖定時 GUI 無法操作）。
    fn screen_locked(&self) -> Option<bool>;
}

/// 什麼都偵測不到：不做延後也不做衝突偵測。
pub struct NullMonitor;

impl InputMonitor for NullMonitor {
    fn idle(&self) -> Option<Duration> {
        None
    }
    fn screen_locked(&self) -> Option<bool> {
        None
    }
}

/// 上一個自己的動作之後，是不是有別人動過輸入裝置。
/// `since_own_action`：距離上一次自己送出輸入過了多久；`idle`：目前距離最近一次任何輸入多久。
/// 若沒人動，`idle` 會跟 `since_own_action` 幾乎一樣長；明顯更短代表其間有別的輸入。
pub fn user_interfered(idle: Duration, since_own_action: Duration, tolerance: Duration) -> bool {
    idle + tolerance < since_own_action
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defer {
    /// 可以開始了。
    Go,
    /// 還不行，過 `DEFER_STEP` 再查。
    Wait(&'static str),
    /// 等太久，放棄。
    GiveUp,
}

/// 無人執行前的判定。偵測不到（`None`）一律視為可以開始。
pub fn defer_decision(locked: Option<bool>, idle: Option<Duration>, waited: Duration) -> Defer {
    let blocked = if locked == Some(true) {
        Some("螢幕鎖定中")
    } else if idle.is_some_and(|i| i < IDLE_NEEDED) {
        Some("使用者正在操作電腦")
    } else {
        None
    };
    match blocked {
        None => Defer::Go,
        Some(_) if waited >= MAX_DEFER => Defer::GiveUp,
        Some(why) => Defer::Wait(why),
    }
}

#[cfg(test)]
pub struct FakeMonitor {
    pub idle: parking_lot::Mutex<Option<Duration>>,
    pub locked: parking_lot::Mutex<Option<bool>>,
}

#[cfg(test)]
impl FakeMonitor {
    pub fn new(idle: Option<Duration>, locked: Option<bool>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { idle: parking_lot::Mutex::new(idle), locked: parking_lot::Mutex::new(locked) })
    }
}

#[cfg(test)]
impl InputMonitor for FakeMonitor {
    fn idle(&self) -> Option<Duration> {
        *self.idle.lock()
    }
    fn screen_locked(&self) -> Option<bool> {
        *self.locked.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn nobody_touched_the_machine_means_idle_matches_time_since_our_action() {
        assert!(!user_interfered(S(5), S(5), TOLERANCE));
        assert!(!user_interfered(S(4), S(5), TOLERANCE), "差在容許誤差內");
    }

    #[test]
    fn a_shorter_idle_than_time_since_our_action_means_someone_else_typed() {
        assert!(user_interfered(S(1), S(5), TOLERANCE));
        assert!(user_interfered(Duration::ZERO, S(3), TOLERANCE));
    }

    #[test]
    fn just_after_our_own_action_is_never_interference() {
        assert!(!user_interfered(Duration::ZERO, Duration::ZERO, TOLERANCE));
        assert!(!user_interfered(Duration::from_millis(300), Duration::from_millis(900), TOLERANCE));
    }

    #[test]
    fn defers_while_locked_or_the_user_is_active() {
        assert_eq!(defer_decision(Some(true), Some(S(999)), S(0)), Defer::Wait("螢幕鎖定中"));
        assert_eq!(defer_decision(Some(false), Some(S(10)), S(0)), Defer::Wait("使用者正在操作電腦"));
    }

    #[test]
    fn goes_once_idle_long_enough_and_unlocked() {
        assert_eq!(defer_decision(Some(false), Some(IDLE_NEEDED), S(0)), Defer::Go);
        assert_eq!(defer_decision(Some(false), Some(S(600)), S(100)), Defer::Go);
    }

    #[test]
    fn gives_up_after_the_maximum_wait() {
        assert_eq!(defer_decision(Some(true), None, MAX_DEFER), Defer::GiveUp);
        assert_eq!(defer_decision(Some(false), Some(S(1)), MAX_DEFER - S(1)), Defer::Wait("使用者正在操作電腦"));
        assert_eq!(defer_decision(Some(false), Some(S(1)), MAX_DEFER), Defer::GiveUp);
    }

    #[test]
    fn unknown_state_never_blocks() {
        assert_eq!(defer_decision(None, None, S(0)), Defer::Go);
        assert_eq!(defer_decision(None, Some(S(5)), S(0)), Defer::Wait("使用者正在操作電腦"));
    }
}
