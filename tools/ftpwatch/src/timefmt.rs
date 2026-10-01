//! UTC date/time formatting from Unix seconds, so the tool needs no date
//! crate. The calendar conversion is Howard Hinnant's `civil_from_days`.

use std::time::{SystemTime, UNIX_EPOCH};

const SECONDS_PER_DAY: i64 = 86_400;

/// Current time as Unix seconds (UTC).
pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Converts days since 1970-01-01 to a (year, month, day) civil date.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Splits Unix seconds into (year, month, day, hour, minute, second), UTC.
fn split(unix: i64) -> (i64, i64, i64, i64, i64, i64) {
    let (year, month, day) = civil_from_days(unix.div_euclid(SECONDS_PER_DAY));
    let seconds = unix.rem_euclid(SECONDS_PER_DAY);
    (year, month, day, seconds / 3_600, seconds % 3_600 / 60, seconds % 60)
}

/// Formats Unix seconds as `YYYY-MM-DD HH:MM:SS` (UTC).
pub fn format_datetime(unix: i64) -> String {
    let (y, mo, d, h, mi, s) = split(unix);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// Formats Unix seconds as `YYYY-MM-DD_HHMMSS` (UTC), used in file names.
pub fn format_stamp(unix: i64) -> String {
    let (y, mo, d, h, mi, s) = split(unix);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}{mi:02}{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_datetime(0), "1970-01-01 00:00:00");
        assert_eq!(format_datetime(1_000_000_000), "2001-09-09 01:46:40");
        assert_eq!(format_datetime(1_578_182_400), "2020-01-05 00:00:00");
        assert_eq!(format_datetime(-1), "1969-12-31 23:59:59");
    }

    #[test]
    fn handles_leap_days() {
        assert_eq!(format_datetime(1_709_164_800), "2024-02-29 00:00:00");
        assert_eq!(format_datetime(1_709_251_200), "2024-03-01 00:00:00");
    }

    #[test]
    fn stamp_is_sortable_and_filename_safe() {
        assert_eq!(format_stamp(1_000_000_000), "2001-09-09_014640");
    }
}
