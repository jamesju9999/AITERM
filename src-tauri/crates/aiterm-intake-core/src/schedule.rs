//! 定時器的週期規則（桌面版與 server 共用）：每天／每週／間隔。
//! 時間點預設是執行那台機器的本地時間；`Spec.tz` 指定 IANA 時區（例如 `Asia/Taipei`）時，改用那個時區的牆上時間。
//! 這裡只有純函式；觸發、結算與資料存取各自在桌面版與 server。
use chrono::{Datelike, Duration, Local, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::Tz;
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
    /// IANA 時區名稱；沒填（或空白）＝這台機器的本地時間。共用看板的定時器由 server 觸發，
    /// 讓使用者用自己的時區設定「每天 09:00」，不必去換算 server 主機的時區。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
}

/// 解析時區名稱；空白當作沒指定。不認得回 `Err`。
pub fn parse_tz(name: &str) -> Result<Option<Tz>, String> {
    let n = name.trim();
    if n.is_empty() {
        return Ok(None);
    }
    n.parse::<Tz>().map(Some).map_err(|_| format!("不認得的時區：{n}（要 IANA 名稱，例如 Asia/Taipei）"))
}

impl Spec {
    fn zone(&self) -> Option<Tz> {
        self.tz.as_deref().and_then(|n| parse_tz(n).ok().flatten())
    }
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
    if let Some(tz) = &spec.tz {
        parse_tz(tz)?;
    }
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


/// 某個時間點（unix 秒）在這個週期設定的時區裡的牆上時間；沒指定時區就是這台機器的本地時間。
pub fn wall(spec: &Spec, ts: i64) -> NaiveDateTime {
    match spec.zone() {
        Some(tz) => tz.timestamp_opt(ts, 0).earliest().map(|d| d.naive_local()).unwrap_or_default(),
        None => from_ts(ts),
    }
}

/// 以 unix 秒判斷到點了沒：把 `last`、`now` 都換成週期設定的時區的牆上時間，再走 `due`。
pub fn due_at(kind: &str, spec: &Spec, last_ts: i64, now_ts: i64) -> bool {
    due(kind, spec, wall(spec, last_ts), wall(spec, now_ts))
}

pub fn local_ts(dt: NaiveDateTime) -> i64 {
    Local.from_local_datetime(&dt).earliest().map(|d| d.timestamp()).unwrap_or_else(|| dt.and_utc().timestamp())
}

pub fn from_ts(ts: i64) -> NaiveDateTime {
    Local.timestamp_opt(ts, 0).single().map(|d| d.naive_local()).unwrap_or_default()
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

    fn ts(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
    }

    #[test]
    fn a_time_zone_makes_the_same_instant_a_different_wall_time() {
        let taipei = Spec { tz: Some("Asia/Taipei".into()), ..daily(&["09:00"]) };
        let ny = Spec { tz: Some("America/New_York".into()), ..daily(&["09:00"]) };
        let t = ts("2026-11-26T01:00:00Z"); // 台北 09:00、紐約 前一天 20:00
        assert_eq!(wall(&taipei, t), dt("2026-11-26 09:00"));
        assert_eq!(wall(&ny, t), dt("2026-11-25 20:00"));
    }

    #[test]
    fn daily_fires_at_the_slot_in_the_chosen_zone_not_the_machines() {
        let taipei = Spec { tz: Some("Asia/Taipei".into()), ..daily(&["09:00"]) };
        let last = ts("2026-11-25T01:00:00Z"); // 台北 11/25 09:00 剛觸發過
        assert!(!due_at(KIND_DAILY, &taipei, last, ts("2026-11-26T00:59:00Z")), "台北 08:59 還沒到");
        assert!(due_at(KIND_DAILY, &taipei, last, ts("2026-11-26T01:00:00Z")), "台北 09:00 到了");
        // 同一組時間點在紐約是另一個時刻：UTC 01:00 對紐約來說是前一天晚上，不會觸發。
        let ny = Spec { tz: Some("America/New_York".into()), ..daily(&["09:00"]) };
        let ny_last = ts("2026-11-26T00:30:00Z"); // 紐約 11/25 19:30，下一個 09:00 是 11/26
        assert!(!due_at(KIND_DAILY, &ny, ny_last, ts("2026-11-26T01:00:00Z")), "同一個時刻在紐約是前一天晚上");
        assert!(!due_at(KIND_DAILY, &ny, ny_last, ts("2026-11-26T13:59:00Z")), "紐約 08:59");
        assert!(due_at(KIND_DAILY, &ny, ny_last, ts("2026-11-26T14:00:00Z")), "紐約 09:00（冬令時間 UTC-5）到了");
    }

    #[test]
    fn daylight_saving_shifts_the_utc_instant_but_not_the_wall_time() {
        let ny = Spec { tz: Some("America/New_York".into()), ..daily(&["09:00"]) };
        // 夏令時間 UTC-4：UTC 13:00＝紐約 09:00；冬令 UTC-5：UTC 14:00＝紐約 09:00。
        assert_eq!(wall(&ny, ts("2026-07-01T13:00:00Z")), dt("2026-07-01 09:00"));
        assert_eq!(wall(&ny, ts("2026-12-01T14:00:00Z")), dt("2026-12-01 09:00"));
    }

    #[test]
    fn no_zone_keeps_using_the_machines_local_time_and_validate_rejects_unknown_zones() {
        let plain = daily(&["09:00"]);
        assert_eq!(wall(&plain, 1_800_000_000), from_ts(1_800_000_000));
        assert!(validate(KIND_DAILY, &Spec { tz: Some("Asia/Taipei".into()), ..plain.clone() }).is_ok());
        assert!(validate(KIND_DAILY, &Spec { tz: Some("  ".into()), ..plain.clone() }).is_ok(), "空白＝沒指定");
        let err = validate(KIND_DAILY, &Spec { tz: Some("Mars/Base".into()), ..plain }).unwrap_err();
        assert!(err.contains("不認得的時區") && err.contains("Mars/Base"), "{err}");
    }

    #[test]
    fn the_zone_is_part_of_schedule_json_and_absent_when_not_set() {
        let with = serde_json::to_string(&Spec { tz: Some("Asia/Taipei".into()), ..daily(&["09:00"]) }).unwrap();
        assert!(with.contains("\"tz\":\"Asia/Taipei\""));
        assert!(!serde_json::to_string(&daily(&["09:00"])).unwrap().contains("tz"), "沒設時區就不寫，舊資料與桌面版的 JSON 不變");
        let old: Spec = serde_json::from_str(r#"{"times":["09:00"]}"#).unwrap();
        assert_eq!(old.tz, None, "舊的 schedule_json 照樣讀得進來");
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
