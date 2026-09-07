//! Clock helpers owned by the Timeline revision store.

/// Current UTC time in milliseconds.
///
/// `VALLE_PROJECT_NOW` is a deterministic test hook. Production callers do
/// not accept timestamps from mutation payloads; the store always issues the
/// timestamp itself through this function.
pub(super) fn now_millis() -> i64 {
    if let Some(value) = std::env::var_os("VALLE_PROJECT_NOW") {
        if let Ok(ms) = value.to_string_lossy().trim().parse::<i64>() {
            return ms;
        }
    }

    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as i64,
        Err(_) => 0,
    }
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn parts(ms: i64) -> (i64, u32, u32, u32, u32, u32, u32) {
    let seconds = ms.div_euclid(1000);
    let milliseconds = ms.rem_euclid(1000) as u32;
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = (seconds_of_day / 3600) as u32;
    let minute = (seconds_of_day % 3600 / 60) as u32;
    let second = (seconds_of_day % 60) as u32;
    (year, month, day, hour, minute, second, milliseconds)
}

/// Canonical millisecond-precision UTC timestamp used by revision metadata.
pub(super) fn iso8601(ms: i64) -> String {
    let (year, month, day, hour, minute, second, milliseconds) = parts(ms);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milliseconds:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_to_millisecond_precision() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(1_784_197_800_123), "2026-07-16T10:30:00.123Z");
    }
}
