use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use valle_timeline::{
    RationalTime, TimeError,
    time::{ExactRational, FrameRate, RationalRate},
};

fn hash(value: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn rational_time_has_one_canonical_value_wire_and_hash() {
    let half = RationalTime::new(1, 2).unwrap();
    let alias = RationalTime::new(2, 4).unwrap();
    assert_eq!(alias, half);
    assert_eq!(alias.numerator(), 1);
    assert_eq!(alias.denominator(), 2);
    assert_eq!(hash(alias), hash(half));
    assert_eq!(serde_json::to_string(&alias).unwrap(), r#""1/2""#);

    let decoded: RationalTime = serde_json::from_str(r#""-3/4""#).unwrap();
    assert_eq!(decoded, RationalTime::new(-3, 4).unwrap());
    assert_eq!(serde_json::to_string(&decoded).unwrap(), r#""-3/4""#);
}

#[test]
fn canonical_wire_aliases_and_malformed_literals_fail_closed() {
    assert_eq!(RationalTime::new(1, 0), Err(TimeError::ZeroDenominator));
    assert_eq!(
        serde_json::from_str::<RationalTime>(r#""0/1""#).unwrap(),
        RationalTime::ZERO
    );

    for invalid in [
        r#""2/4""#,
        r#""0/2""#,
        r#""-0/1""#,
        r#""-0""#,
        r#""+1/2""#,
        r#"" 1/2""#,
        r#""1/2 ""#,
        r#""01/2""#,
        r#""1/02""#,
        r#""1/-2""#,
        r#""1/0""#,
        r#""1""#,
        r#"{"numerator":1,"denominator":2}"#,
    ] {
        assert!(
            serde_json::from_str::<RationalTime>(invalid).is_err(),
            "accepted non-canonical rational wire: {invalid}"
        );
    }
}

#[test]
fn exact_algebra_is_shared_but_domain_rates_are_positive() {
    let half = ExactRational::new(2, 4).unwrap();
    assert_eq!(half, ExactRational::new(1, 2).unwrap());
    assert_eq!(serde_json::to_string(&half).unwrap(), r#""1/2""#);
    assert!(serde_json::from_str::<ExactRational>(r#""2/4""#).is_err());

    assert_eq!(RationalRate::new(1, 1).unwrap(), RationalRate::ONE);
    assert_eq!(RationalRate::new(0, 1), Err(TimeError::InvalidRate));
    assert_eq!(FrameRate::new(0, 1), Err(TimeError::InvalidFrameRate));
    assert!(serde_json::from_str::<RationalRate>(r#""-1/1""#).is_err());
    assert!(serde_json::from_str::<FrameRate>(r#""0/1""#).is_err());

    assert_eq!(
        RationalTime::new(3, 2)
            .unwrap()
            .checked_scale(RationalRate::new(2, 3).unwrap())
            .unwrap(),
        RationalTime::ONE
    );
}

#[test]
fn checked_arithmetic_reports_overflow_and_division_by_zero() {
    let max = ExactRational::new(i64::MAX, 1).unwrap();
    assert_eq!(
        max.checked_add(ExactRational::ONE),
        Err(TimeError::Overflow)
    );
    assert_eq!(
        ExactRational::new(i64::MIN, 1).unwrap().checked_neg(),
        Err(TimeError::Overflow)
    );
    assert_eq!(
        ExactRational::ONE.checked_div(ExactRational::ZERO),
        Err(TimeError::DivisionByZero)
    );

    let max = RationalTime::new(i64::MAX, 1).unwrap();
    assert_eq!(
        max.checked_add(RationalTime::new(1, 1).unwrap()),
        Err(TimeError::Overflow)
    );
}

#[test]
fn ordering_is_exact_for_negative_and_large_values() {
    assert!(RationalTime::new(-1, 3).unwrap() < RationalTime::ZERO);
    assert!(
        RationalTime::new(i64::MAX - 1, u32::MAX).unwrap()
            < RationalTime::new(i64::MAX, u32::MAX).unwrap()
    );
}
