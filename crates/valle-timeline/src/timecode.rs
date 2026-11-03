//! SMPTE ST 12 timecode labels: frame numbers as `HH:MM:SS:FF` at a rate's nominal whole frames
//! per second, and drop-frame `HH:MM:SS;FF` at 29.97 and 59.94 (labels 0 and 1, or 0–3, skipped
//! each minute but every tenth, so the label keeps up with the clock). 23.976 counts at 24,
//! non-drop. A timecode is only a label: it never places anything.

/// How a rate counts: its nominal frames a second, and the labels dropped each minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Count {
    nominal: i64,
    drop: i64,
}

impl Count {
    /// Drop-frame for the NTSC rates 30000/1001 and 60000/1001 (and 29.97 written so), non-drop
    /// otherwise.
    fn of(fps: f64) -> Count {
        let nominal = (fps.round() as i64).max(1);
        let ntsc = nominal % 30 == 0 && (fps - nominal as f64 * 1000.0 / 1001.0).abs() < 1e-3;
        Count {
            nominal,
            drop: if ntsc { nominal / 15 } else { 0 },
        }
    }
}

/// The label of frame `frame` (0 is 00:00:00:00; past 24 hours it wraps round).
pub fn format(frame: i64, fps: f64) -> String {
    let c = Count::of(fps);
    let day = c.nominal * 86_400 - c.drop * (1440 - 144);
    let mut n = frame.rem_euclid(day);
    if c.drop > 0 {
        let ten = c.nominal * 600 - c.drop * 9;
        let minute = c.nominal * 60 - c.drop;
        let (d, m) = (n / ten, n % ten);
        n += c.drop * 9 * d;
        if m >= c.drop {
            n += c.drop * ((m - c.drop) / minute);
        }
    }
    let (f, s) = (n % c.nominal, n / c.nominal);
    let sep = if c.drop > 0 { ';' } else { ':' };
    format!(
        "{:02}:{:02}:{:02}{sep}{:02}",
        s / 3600,
        s / 60 % 60,
        s % 60,
        f
    )
}

/// The frame a label names: `HH:MM:SS:FF`, or `;` before the frames for drop-frame, which the
/// rate must count.
pub fn parse(tc: &str, fps: f64) -> Result<i64, String> {
    let c = Count::of(fps);
    let bad = || format!("timecode {tc:?}: HH:MM:SS:FF (drop-frame HH:MM:SS;FF)");
    let parts: Vec<i64> = (tc.split([':', ';']))
        .map(|p| p.parse::<i64>().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    let [h, m, s, f] = parts[..] else {
        return Err(bad());
    };
    if m >= 60 || s >= 60 || f >= c.nominal || [h, m, s, f].iter().any(|v| *v < 0) {
        return Err(format!(
            "timecode {tc:?}: a field out of range at {fps} fps"
        ));
    }
    if tc.contains(';') && c.drop == 0 {
        return Err(format!(
            "timecode {tc:?}: drop-frame (;) counts 29.97 and 59.94, not {fps} fps"
        ));
    }
    let minutes = 60 * h + m;
    if c.drop > 0 && s == 0 && f < c.drop && m % 10 != 0 {
        return Err(format!(
            "timecode {tc:?}: drop-frame skips frames 0-{} of each minute but every tenth",
            c.drop - 1
        ));
    }
    let n = (minutes * 60 + s) * c.nominal + f;
    Ok(n - c.drop * (minutes - minutes / 10))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NTSC: f64 = 30000.0 / 1001.0;

    #[test]
    fn drop_frame_labels_keep_up_with_the_clock() {
        assert_eq!(format(0, NTSC), "00:00:00;00");
        assert_eq!(format(1799, NTSC), "00:00:59;29");
        assert_eq!(format(1800, NTSC), "00:01:00;02");
        assert_eq!(format(17982, NTSC), "00:10:00;00");
        assert_eq!(format(107892, NTSC), "01:00:00;00");
        assert_eq!(format(107892, 29.97), "01:00:00;00");
        assert_eq!(parse("00:01:00;02", NTSC), Ok(1800));
        assert!(parse("00:01:00;01", NTSC).is_err());
        assert_eq!(format(3600 + 3596, 60000.0 / 1001.0), "00:02:00;04");
        for n in (0..300_000).step_by(7) {
            assert_eq!(parse(&format(n, NTSC), NTSC), Ok(n), "frame {n}");
        }
    }

    #[test]
    fn other_rates_count_every_label() {
        assert_eq!(format(86_423, 24000.0 / 1001.0), "01:00:00:23");
        assert_eq!(format(90_000, 25.0), "01:00:00:00");
        assert_eq!(parse("00:00:01:15", 30.0), Ok(45));
        assert!(parse("00:00:01;15", 30.0).is_err());
        assert!(parse("00:00:01:30", 30.0).is_err());
        assert_eq!(format(-1, 30.0), "23:59:59:29");
    }
}
