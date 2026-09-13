//! Prepare-time pie angle allocation. Output is numeric slice identity, not path geometry.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PieSlice {
    pub index: usize,
    pub value: f64,
    pub start_angle: f64,
    pub end_angle: f64,
    pub mid_angle: f64,
    pub fraction: f64,
}

/// Allocate visible start/end angles in input order. Zeros keep their index with a zero sweep.
pub fn pie(
    values: &[f64],
    start_angle: f64,
    end_angle: f64,
    pad_angle: f64,
) -> Result<Vec<PieSlice>, String> {
    if !start_angle.is_finite() || !end_angle.is_finite() || !pad_angle.is_finite() {
        return Err("pie angles must be finite".into());
    }
    if pad_angle < 0.0 {
        return Err("pie padAngle must be non-negative".into());
    }
    let span = end_angle - start_angle;
    if span < 0.0 || span > core::f64::consts::TAU {
        return Err("pie span must be non-negative and at most one turn".into());
    }
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err("pie values must be finite and non-negative".into());
    }
    if values.is_empty() {
        return Ok(Vec::new());
    }

    let total: f64 = values.iter().sum();
    if !total.is_finite() {
        return Err("pie values sum must be finite".into());
    }
    let positive = values.iter().filter(|value| **value > 0.0).count();
    let pad_total = positive as f64 * pad_angle;
    if positive > 0 && pad_total >= span {
        return Err("pie padAngle exceeds the available span".into());
    }
    let available = if positive == 0 { 0.0 } else { span - pad_total };

    let mut cursor = start_angle;
    let mut slices = Vec::with_capacity(values.len());
    for (index, value) in values.iter().copied().enumerate() {
        if value == 0.0 {
            slices.push(PieSlice {
                index,
                value,
                start_angle: cursor,
                end_angle: cursor,
                mid_angle: cursor,
                fraction: 0.0,
            });
            continue;
        }
        // Normalize first: multiplying a finite large value by the angle can overflow.
        let fraction = value / total;
        let sweep = available * fraction;
        let visible_start = cursor + pad_angle / 2.0;
        let visible_end = visible_start + sweep;
        slices.push(PieSlice {
            index,
            value,
            start_angle: visible_start,
            end_angle: visible_end,
            mid_angle: visible_start + sweep / 2.0,
            fraction,
        });
        cursor = visible_end + pad_angle / 2.0;
    }
    Ok(slices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pie_keeps_input_order_zeros_and_fractions() {
        let slices = pie(&[1.0, 0.0, 3.0], 0.0, core::f64::consts::TAU, 0.0).unwrap();
        assert_eq!(slices.len(), 3);
        assert_eq!(slices[0].index, 0);
        assert_eq!(slices[1].value, 0.0);
        assert_eq!(slices[1].start_angle, slices[1].end_angle);
        assert_eq!(slices[0].fraction, 0.25);
        assert_eq!(slices[2].fraction, 0.75);
        assert!((slices[2].end_angle - core::f64::consts::TAU).abs() < 1e-12);
        assert!(slices[0].end_angle <= slices[2].start_angle + 1e-12);
    }

    #[test]
    fn pie_pad_skips_zeros_and_rejects_overfill() {
        let slices = pie(&[1.0, 0.0, 1.0], 0.0, 2.0, 0.2).unwrap();
        assert!((slices[0].end_angle - slices[0].start_angle - 0.8).abs() < 1e-12);
        assert_eq!(slices[1].start_angle, slices[1].end_angle);
        assert!(slices[0].end_angle <= slices[2].start_angle);
        assert!(pie(&[1.0, 1.0], 0.0, 1.0, 0.6).is_err());
        assert!(pie(&[-1.0], 0.0, 1.0, 0.0).is_err());
        assert!(pie(&[], 0.0, 1.0, 0.0).unwrap().is_empty());
        let zeros = pie(&[0.0, 0.0], 0.5, 1.5, 0.0).unwrap();
        assert!(zeros.iter().all(|slice| slice.fraction == 0.0
            && slice.start_angle == 0.5
            && slice.end_angle == 0.5));
    }

    #[test]
    fn pie_rejects_sum_overflow_but_accepts_large_finite_totals() {
        assert!(
            pie(&[1e308, 1e308], 0.0, 1.0, 0.0)
                .unwrap_err()
                .contains("sum")
        );
        let slices = pie(
            &[f64::MAX / 2.0, f64::MAX / 2.0],
            0.0,
            core::f64::consts::TAU,
            0.0,
        )
        .unwrap();
        for slice in &slices {
            assert_eq!(slice.fraction, 0.5);
            assert_eq!(slice.end_angle - slice.start_angle, core::f64::consts::PI);
            assert!(slice.mid_angle.is_finite());
        }
        let tiny = pie(&[f64::MIN_POSITIVE, f64::MIN_POSITIVE], 0.0, 1.0, 0.0).unwrap();
        assert_eq!(tiny[0].fraction, 0.5);
        assert_eq!(tiny[1].end_angle, 1.0);
    }
}
