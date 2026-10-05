//! UTC calendar math shared by ids and note properties.
//!
//! Howard Hinnant's `civil_from_days` / `days_from_civil` algorithms.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// `(year, month, day, hour, minute, second)` in UTC.
pub fn utc_parts(time: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    (
        year,
        month,
        day,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

pub(crate) fn format_rfc3339(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(time);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

pub(crate) fn parse_rfc3339(text: &str) -> Option<SystemTime> {
    let text = text.trim();
    if text.len() != 20 || !text.ends_with('Z') || text.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let year: i64 = text[0..4].parse().ok()?;
    let month: u32 = text[5..7].parse().ok()?;
    let day: u32 = text[8..10].parse().ok()?;
    let hour: u32 = text[11..13].parse().ok()?;
    let minute: u32 = text[14..16].parse().ok()?;
    let second: u32 = text[17..19].parse().ok()?;
    if text.as_bytes()[4] != b'-'
        || text.as_bytes()[7] != b'-'
        || text.as_bytes()[13] != b':'
        || text.as_bytes()[16] != b':'
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let (y, m, d) = civil_from_days(days);
    if (y, m, d) != (year, month, day) {
        return None;
    }
    let secs = days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
    if secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(secs as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp(
        year: i64,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> SystemTime {
        UNIX_EPOCH
            + Duration::from_secs(
                (days_from_civil(year, month, day) * 86_400
                    + i64::from(hour) * 3_600
                    + i64::from(minute) * 60
                    + i64::from(second)) as u64,
            )
    }

    #[test]
    fn utc_parts_formats_epoch_and_day_boundaries() {
        assert_eq!(utc_parts(UNIX_EPOCH), (1970, 1, 1, 0, 0, 0));
        assert_eq!(
            utc_parts(timestamp(2024, 2, 29, 23, 59, 59)),
            (2024, 2, 29, 23, 59, 59)
        );
        assert_eq!(
            utc_parts(timestamp(2000, 3, 1, 0, 0, 0)),
            (2000, 3, 1, 0, 0, 0)
        );
    }

    #[test]
    fn rfc3339_round_trips_leap_days_and_midnight() {
        for (year, month, day, hour, minute, second) in [
            (1970, 1, 1, 0, 0, 0),
            (2000, 2, 29, 12, 34, 56),
            (2024, 2, 29, 23, 59, 59),
            (2026, 9, 28, 14, 22, 33),
        ] {
            let time = timestamp(year, month, day, hour, minute, second);
            let encoded = format_rfc3339(time);
            assert_eq!(parse_rfc3339(&encoded), Some(time), "{encoded}");
        }
    }

    #[test]
    fn rfc3339_parser_accepts_outer_whitespace_but_rejects_invalid_dates() {
        assert_eq!(
            parse_rfc3339("  2026-09-28T14:22:33Z\n"),
            Some(timestamp(2026, 9, 28, 14, 22, 33))
        );
        for invalid in [
            "",
            "2026-02-29T00:00:00Z",
            "2026-04-31T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:60:00Z",
            "2026-01-01T00:00:61Z",
            "2026/01/01T00:00:00Z",
            "2026-01-01 00:00:00Z",
            "not-a-date",
        ] {
            assert_eq!(parse_rfc3339(invalid), None, "accepted {invalid:?}");
        }
    }

    #[test]
    fn pre_epoch_times_are_clamped_for_display_and_rejected_for_properties() {
        assert_eq!(
            utc_parts(UNIX_EPOCH - Duration::from_secs(1)),
            (1970, 1, 1, 0, 0, 0)
        );
        assert_eq!(parse_rfc3339("1969-12-31T23:59:59Z"), None);
    }
}
