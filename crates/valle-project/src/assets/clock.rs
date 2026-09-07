//! UTC timestamps using the civil-from-days algorithm. `VALLE_ASSETS_NOW` overrides epoch
//! milliseconds for deterministic tests.

/// Current UTC milliseconds, overridden by `VALLE_ASSETS_NOW` when set.
pub fn now_millis() -> i64 {
    if let Some(v) = std::env::var_os("VALLE_ASSETS_NOW") {
        if let Ok(ms) = v.to_string_lossy().trim().parse::<i64>() {
            return ms;
        }
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(_) => 0,
    }
}

/// Convert days since the Unix epoch to year, month, and day.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

/// Format an ISO 8601 UTC timestamp with milliseconds.
pub fn iso8601(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000) as u32;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let h = (sod / 3600) as u32;
    let mi = (sod % 3600 / 60) as u32;
    let s = (sod % 60) as u32;
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{millis:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_epoch_formats() {
        assert_eq!(iso8601(1_784_197_800_123), "2026-07-16T10:30:00.123Z");
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
    }
}
