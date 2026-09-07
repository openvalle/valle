//! Workspace floating-point JSON roundtrips must preserve f64 bits.

/// Require serde_json float_roundtrip so native parsing matches correctly rounded Web JSON parsing.
#[test]
fn float_json_roundtrip_is_bit_exact() {
    // cos(15 degrees) is a regression sample sensitive to one-ulp parser drift.
    let x: f64 = 15.0f64.to_radians().cos();
    let s = serde_json::to_string(&x).unwrap();
    assert_eq!(
        serde_json::from_str::<f64>(&s).unwrap().to_bits(),
        x.to_bits(),
        "cos(15 degrees) lost one ulp during JSON roundtrip; check float_roundtrip"
    );
    assert_eq!(
        serde_json::from_str::<f64>(&s).unwrap().to_bits(),
        s.parse::<f64>().unwrap().to_bits(),
        "serde_json and str::parse must agree"
    );

    // Check a deterministic LCG sample of doubles in [0, 1).
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for i in 0..20_000 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let x = f64::from_bits((state >> 12) | 0x3FF0_0000_0000_0000) - 1.0;
        let back: f64 = serde_json::from_str(&serde_json::to_string(&x).unwrap()).unwrap();
        assert_eq!(
            back.to_bits(),
            x.to_bits(),
            "sample {i} ({x:?}) changed during roundtrip"
        );
    }
}
