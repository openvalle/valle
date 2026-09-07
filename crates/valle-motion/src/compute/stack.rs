//! Generic matrix stacking for prepared chart geometry.
//!
//! The input is series-major (`values[series][category]`). The result is still only numbers:
//! `layers[series][category] = [low, high]`. There is no Series, Axis, paint or scene concept here.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackOffset {
    Zero,
    Expand,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackResult {
    pub layers: Vec<Vec<[f64; 2]>>,
    pub positive_totals: Vec<f64>,
    pub negative_totals: Vec<f64>,
}

/// Stack a finite rectangular matrix without reordering its rows or columns.
///
/// `Zero` supports diverging values by maintaining independent positive and negative baselines.
/// `Expand` normalizes non-negative categories to `[0, 1]`; negative values are rejected rather
/// than silently inventing a normalization convention.
pub fn stack(values: &[Vec<f64>], offset: StackOffset) -> Result<StackResult, String> {
    let categories = values.first().map_or(0, Vec::len);
    if values.is_empty() || categories == 0 {
        return Err("stack values must contain at least one series and one category".into());
    }
    if values.iter().any(|series| series.len() != categories) {
        return Err(
            "stack values must be rectangular; every series needs the same category count".into(),
        );
    }
    if values.iter().flatten().any(|value| !value.is_finite()) {
        return Err("stack values must contain only finite numbers".into());
    }
    if offset == StackOffset::Expand && values.iter().flatten().any(|value| *value < 0.0) {
        return Err("stack offset `expand` accepts non-negative values only".into());
    }

    let mut positive_totals = vec![0.0; categories];
    let mut negative_totals = vec![0.0; categories];
    for series in values {
        for (category, value) in series.iter().copied().enumerate() {
            if value >= 0.0 {
                positive_totals[category] += value;
            } else {
                negative_totals[category] += value;
            }
        }
    }

    let mut positive = vec![0.0; categories];
    let mut negative = vec![0.0; categories];
    let mut layers = Vec::with_capacity(values.len());
    for series in values {
        let mut layer = Vec::with_capacity(categories);
        for (category, raw) in series.iter().copied().enumerate() {
            let value = match offset {
                StackOffset::Zero => raw,
                StackOffset::Expand => {
                    let total = positive_totals[category];
                    if total == 0.0 { 0.0 } else { raw / total }
                }
            };
            if value >= 0.0 {
                let low = positive[category];
                positive[category] += value;
                layer.push([low, positive[category]]);
            } else {
                let high = negative[category];
                negative[category] += value;
                layer.push([negative[category], high]);
            }
        }
        layers.push(layer);
    }

    Ok(StackResult {
        layers,
        positive_totals,
        negative_totals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_offset_keeps_input_order_and_diverging_baselines() {
        let result = stack(
            &[vec![2.0, -3.0], vec![5.0, -7.0], vec![1.0, 4.0]],
            StackOffset::Zero,
        )
        .unwrap();
        assert_eq!(
            result.layers,
            vec![
                vec![[0.0, 2.0], [-3.0, 0.0]],
                vec![[2.0, 7.0], [-10.0, -3.0]],
                vec![[7.0, 8.0], [0.0, 4.0]],
            ]
        );
        assert_eq!(result.positive_totals, vec![8.0, 4.0]);
        assert_eq!(result.negative_totals, vec![0.0, -10.0]);
    }

    #[test]
    fn expand_normalizes_each_non_negative_category() {
        let result = stack(&[vec![1.0, 0.0], vec![3.0, 0.0]], StackOffset::Expand).unwrap();
        assert_eq!(result.layers[0], vec![[0.0, 0.25], [0.0, 0.0]]);
        assert_eq!(result.layers[1], vec![[0.25, 1.0], [0.0, 0.0]]);
    }

    #[test]
    fn malformed_matrices_fail_closed() {
        assert!(stack(&[], StackOffset::Zero).is_err());
        assert!(stack(&[vec![1.0], vec![2.0, 3.0]], StackOffset::Zero).is_err());
        assert!(stack(&[vec![-1.0]], StackOffset::Expand).is_err());
    }
}
