use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Shanghai;

/// The fixed UTC offset of Asia/Shanghai. The zone has no DST, so every local
/// day boundary maps to `local_midnight - 8h` in UTC at every instant of the year.
const BEIJING_UTC_OFFSET_SECONDS: i32 = 8 * 3600;

/// The Asia/Shanghai local date of `now`, as a `YYYY-MM-DD` day id (AR-2).
pub fn beijing_day_id(now: DateTime<Utc>) -> String {
    now.with_timezone(&Shanghai).date_naive().to_string()
}

/// The start instant (00:00:00 local) of the Asia/Shanghai calendar day that
/// contains `now`, expressed in UTC. This is the `today_start` instant shared by
/// every dashboard "today" aggregate (AD-2, BP-U3, DH-18a).
pub fn beijing_today_start_utc(now: DateTime<Utc>) -> DateTime<Utc> {
    beijing_day_start_utc(&beijing_day_id(now)).expect("current day id is a valid date")
}

/// The UTC start instant of the Asia/Shanghai day `YYYY-MM-DD`. Returns `None`
/// for a day id that is not a valid date or is outside chrono's representable
/// range.
pub fn beijing_day_start_utc(day: &str) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    let local_midnight = date
        .and_hms_opt(0, 0, 0)?
        .and_local_timezone(Shanghai)
        .single()?;
    Some(local_midnight.with_timezone(&Utc))
}

/// The UTC exclusive end instant of the Asia/Shanghai day `YYYY-MM-DD`.
pub fn beijing_day_end_utc(day: &str) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    let next = date.succ_opt()?;
    let local_end = next
        .and_hms_opt(0, 0, 0)?
        .and_local_timezone(Shanghai)
        .single()?;
    Some(local_end.with_timezone(&Utc))
}

/// The day id of the day that follows `day` in Asia/Shanghai.
pub fn beijing_next_day_id(day: &str) -> Option<String> {
    let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    let next = date.succ_opt()?;
    Some(next.to_string())
}

/// Validate a `YYYY-MM-DD` day id: it must parse as a date and the formatted
/// round-trip must be identical, rejecting `2026-9-6` and `20260906`.
pub fn is_valid_beijing_day_id(day: &str) -> bool {
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|date| date.to_string() == day)
        .unwrap_or(false)
}

/// The number of whole days between two day ids (`to` - `from`), or `None`
/// when either id is invalid.
pub fn beijing_day_distance(from: &str, to: &str) -> Option<i64> {
    let from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let to_date = NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    Some((to_date - from_date).num_days())
}

/// Iterate the day ids from `from` to `to` inclusive, ascending.
pub fn beijing_day_ids(from: &str, to: &str) -> Option<Vec<String>> {
    let mut from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let to_date = NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    if from_date > to_date {
        return None;
    }
    let mut days = Vec::new();
    while from_date <= to_date {
        days.push(from_date.to_string());
        from_date = from_date.succ_opt()?;
    }
    Some(days)
}

/// The day id `offset_days` before `day` in Asia/Shanghai.
pub fn beijing_day_shift(day: &str, offset_days: i64) -> Option<String> {
    let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    let shifted = date.checked_add_signed(Duration::days(offset_days))?;
    Some(shifted.to_string())
}

/// The UTC offset in seconds of Asia/Shanghai. Exposed for the SQL day
/// expressions that avoid timezone tables on SQLite.
pub fn beijing_utc_offset_seconds() -> i32 {
    BEIJING_UTC_OFFSET_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn day_id_uses_beijing_date_across_midnight() {
        // 2026-09-15 16:05 UTC is 2026-09-16 00:05 Beijing: the Beijing day has
        // already flipped even though the UTC date has not.
        assert_eq!(beijing_day_id(utc(2026, 9, 15, 16, 5)), "2026-09-16");
        assert_eq!(beijing_day_id(utc(2026, 9, 15, 15, 59)), "2026-09-15");
    }

    #[test]
    fn today_start_is_beijing_midnight() {
        let now = utc(2026, 9, 15, 16, 5);
        let start = beijing_today_start_utc(now);
        assert_eq!(start, utc(2026, 9, 15, 16, 0));
    }

    #[test]
    fn day_bounds_are_exact() {
        assert_eq!(
            beijing_day_start_utc("2026-09-16"),
            Some(utc(2026, 9, 15, 16, 0))
        );
        assert_eq!(
            beijing_day_end_utc("2026-09-16"),
            Some(utc(2026, 9, 16, 16, 0))
        );
    }

    #[test]
    fn day_id_validation_rejects_noncanonical_forms() {
        assert!(is_valid_beijing_day_id("2026-09-16"));
        assert!(!is_valid_beijing_day_id("2026-9-6"));
        assert!(!is_valid_beijing_day_id("20260916"));
        assert!(!is_valid_beijing_day_id("not-a-date"));
    }

    #[test]
    fn day_helpers_round_trip() {
        assert_eq!(beijing_next_day_id("2026-12-31"), Some("2027-01-01".into()));
        assert_eq!(
            beijing_day_shift("2026-09-16", -30),
            Some("2026-08-17".into())
        );
        assert_eq!(beijing_day_distance("2026-08-17", "2026-09-16"), Some(30));
        let days = beijing_day_ids("2026-09-15", "2026-09-17").expect("range");
        assert_eq!(days, vec!["2026-09-15", "2026-09-16", "2026-09-17"]);
        assert!(beijing_day_ids("2026-09-17", "2026-09-15").is_none());
    }
}
