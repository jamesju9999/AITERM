//! Agent 協作定時器。見 docs/superpowers/specs/2026-11-16-agent-canvas-scheduler-design.md。
use chrono::{Datelike, Duration, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

pub const KIND_DAILY: &str = "daily";
pub const KIND_WEEKLY: &str = "weekly";
pub const KIND_INTERVAL: &str = "interval";
pub const MAX_TIMES: usize = 12;
pub const MIN_INTERVAL_MINUTES: u32 = 5;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub from: String,
    pub to: String,
}

/// `schedule_json` 的內容；三種模式共用一個結構，各自只用自己的欄位。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    #[serde(default)]
    pub times: Vec<String>,
    /// 1=週一 … 7=週日。
    #[serde(default)]
    pub days: Vec<u32>,
    #[serde(default)]
    pub every_minutes: u32,
    #[serde(default)]
    pub window: Option<Window>,
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

pub fn validate(kind: &str, spec: &Spec) -> Result<(), String> {
    let check_times = |spec: &Spec| -> Result<(), String> {
        if spec.times.is_empty() {
            return Err("至少要設定一個時間點".into());
        }
        if spec.times.len() > MAX_TIMES {
            return Err(format!("時間點最多 {MAX_TIMES} 個"));
        }
        for t in &spec.times {
            parse_time(t).ok_or_else(|| format!("時間格式不對（要 HH:MM）：{t}"))?;
        }
        Ok(())
    };
    match kind {
        KIND_DAILY => check_times(spec),
        KIND_WEEKLY => {
            check_times(spec)?;
            if spec.days.is_empty() || spec.days.iter().any(|d| !(1..=7).contains(d)) {
                return Err("每週模式要勾選星期（1–7）".into());
            }
            Ok(())
        }
        KIND_INTERVAL => {
            if spec.every_minutes < MIN_INTERVAL_MINUTES {
                return Err(format!("間隔不能小於 {MIN_INTERVAL_MINUTES} 分鐘"));
            }
            if let Some(w) = &spec.window {
                let (f, t) = (parse_time(&w.from), parse_time(&w.to));
                match (f, t) {
                    (Some(f), Some(t)) if f < t => {}
                    _ => return Err("時段要是 HH:MM，且起點早於終點".into()),
                }
            }
            Ok(())
        }
        other => Err(format!("不認得的週期模式：{other}")),
    }
}

/// 從 `last` 之後到 `now`（含）之間，是否有一個預定時間點已經過了。
/// `last` 是上次觸發（或建立／啟用）的本地時間。全是本地時間，沒有時區問題。
pub fn due(kind: &str, spec: &Spec, last: NaiveDateTime, now: NaiveDateTime) -> bool {
    if now <= last {
        return false;
    }
    match kind {
        KIND_INTERVAL => {
            if now < last + Duration::minutes(i64::from(spec.every_minutes)) {
                return false;
            }
            match &spec.window {
                None => true,
                Some(w) => match (parse_time(&w.from), parse_time(&w.to)) {
                    (Some(f), Some(t)) => now.time() >= f && now.time() <= t,
                    _ => false,
                },
            }
        }
        KIND_DAILY | KIND_WEEKLY => {
            let end = now.date();
            // 隔太久沒開 App 時只往回看一年：更早的時間點一定也在這範圍內有對應的，結論相同。
            let mut day = last.date().max(end - Duration::days(400));
            while day <= end {
                let weekday_ok = kind == KIND_DAILY || spec.days.contains(&day.weekday().number_from_monday());
                if weekday_ok {
                    for t in spec.times.iter().filter_map(|t| parse_time(t)) {
                        let slot = day.and_time(t);
                        if slot > last && slot <= now {
                            return true;
                        }
                    }
                }
                let Some(next) = day.succ_opt() else { break };
                day = next;
            }
            false
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }
    fn daily(times: &[&str]) -> Spec {
        Spec { times: times.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn daily_does_not_fire_one_minute_before_the_slot() {
        assert!(!due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-15 20:00"), dt("2026-11-16 07:59")));
    }

    #[test]
    fn daily_fires_at_and_after_the_slot() {
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-15 20:00"), dt("2026-11-16 08:00")));
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-15 20:00"), dt("2026-11-16 08:01")));
    }

    #[test]
    fn daily_does_not_fire_twice_for_the_same_slot() {
        // 08:00 的那一次已在 08:00 觸發過，之後的檢查不再成立。
        assert!(!due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-16 08:00"), dt("2026-11-16 08:01")));
        assert!(!due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-16 08:00"), dt("2026-11-16 23:59")));
    }

    #[test]
    fn daily_fires_again_next_day_and_after_a_gap_only_reports_due_once() {
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-16 08:00"), dt("2026-11-17 08:00")));
        // 隔了三天沒開 App：仍只是「該觸發」一次（呼叫端把 last 設成 now）。
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-16 08:00"), dt("2026-11-19 09:00")));
    }

    #[test]
    fn daily_multiple_times_fire_each() {
        let s = daily(&["08:00", "18:30"]);
        assert!(!due(KIND_DAILY, &s, dt("2026-11-16 08:00"), dt("2026-11-16 18:29")));
        assert!(due(KIND_DAILY, &s, dt("2026-11-16 08:00"), dt("2026-11-16 18:30")));
    }

    #[test]
    fn daily_crosses_month_boundary() {
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-30 08:00"), dt("2026-12-01 08:00")));
    }

    #[test]
    fn weekly_only_fires_on_selected_days() {
        // 2026-11-16 是週一。
        let s = Spec { times: vec!["09:00".into()], days: vec![1, 2, 3, 4, 5], ..Default::default() };
        assert!(due(KIND_WEEKLY, &s, dt("2026-11-14 09:00"), dt("2026-11-16 09:00"))); // 六 → 一
        assert!(!due(KIND_WEEKLY, &s, dt("2026-11-14 09:00"), dt("2026-11-15 12:00"))); // 週日不觸發
        let sat_only = Spec { times: vec!["09:00".into()], days: vec![6], ..Default::default() };
        assert!(!due(KIND_WEEKLY, &sat_only, dt("2026-11-16 09:00"), dt("2026-11-20 12:00")));
        assert!(due(KIND_WEEKLY, &sat_only, dt("2026-11-16 09:00"), dt("2026-11-21 09:00")));
    }

    #[test]
    fn interval_fires_after_the_period() {
        let s = Spec { every_minutes: 120, ..Default::default() };
        assert!(!due(KIND_INTERVAL, &s, dt("2026-11-16 08:00"), dt("2026-11-16 09:59")));
        assert!(due(KIND_INTERVAL, &s, dt("2026-11-16 08:00"), dt("2026-11-16 10:00")));
    }

    #[test]
    fn interval_respects_the_window() {
        let s = Spec { every_minutes: 60, window: Some(Window { from: "09:00".into(), to: "18:00".into() }), ..Default::default() };
        assert!(!due(KIND_INTERVAL, &s, dt("2026-11-16 17:00"), dt("2026-11-16 18:01"))); // 窗外
        assert!(due(KIND_INTERVAL, &s, dt("2026-11-16 17:00"), dt("2026-11-16 18:00")));
        // 夜裡到期，隔天進入時段才觸發。
        assert!(!due(KIND_INTERVAL, &s, dt("2026-11-16 17:30"), dt("2026-11-16 23:00")));
        assert!(due(KIND_INTERVAL, &s, dt("2026-11-16 17:30"), dt("2026-11-17 09:00")));
    }

    #[test]
    fn validate_rejects_bad_specs() {
        assert!(validate(KIND_DAILY, &Spec::default()).is_err());
        assert!(validate(KIND_DAILY, &daily(&["25:00"])).is_err());
        assert!(validate(KIND_DAILY, &daily(&["08:00"])).is_ok());
        let many: Vec<&str> = vec!["08:00"; 13];
        assert!(validate(KIND_DAILY, &daily(&many)).is_err());
        assert!(validate(KIND_WEEKLY, &daily(&["08:00"])).is_err()); // 沒勾星期
        assert!(validate(KIND_WEEKLY, &Spec { times: vec!["08:00".into()], days: vec![8], ..Default::default() }).is_err());
        assert!(validate(KIND_INTERVAL, &Spec { every_minutes: 4, ..Default::default() }).is_err());
        assert!(validate(KIND_INTERVAL, &Spec { every_minutes: 5, ..Default::default() }).is_ok());
        let bad_window = Spec { every_minutes: 60, window: Some(Window { from: "18:00".into(), to: "09:00".into() }), ..Default::default() };
        assert!(validate(KIND_INTERVAL, &bad_window).is_err());
        assert!(validate("monthly", &Spec::default()).is_err());
    }
}