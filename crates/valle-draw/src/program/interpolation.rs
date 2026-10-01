//! Author gradient interpolation, lowered once into floating-point working-space stops.
//! Both Skia and CanvasKit consume the same result; neither reinterprets author color spaces.

use serde::{Deserialize, Serialize};

use super::{
    AuthorColor, GradientStop, LinearColor,
    paint::{decode_srgb_channel, encode_srgb_channel, from_linear_srgb},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "space", rename_all = "camelCase", deny_unknown_fields)]
pub enum GradientInterpolation {
    LinearSrgb,
    Srgb,
    Oklab,
    Oklch { hue: HueDirection },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum HueDirection {
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

/// Interpolate unassociated author colors without a frame-time 8-bit round trip. The formulas
/// and hue rules are shared with CSS gradient lowering, and all nonlinear math uses `libm`.
pub fn interpolate_author_colors(
    a: AuthorColor,
    b: AuthorColor,
    t: f64,
    space: GradientInterpolation,
) -> AuthorColor {
    if t == 0.0 {
        return a;
    }
    if t == 1.0 {
        return b;
    }
    let alpha = (a.alpha + (b.alpha - a.alpha) * t).clamp(0.0, 1.0);
    if alpha == 0.0 {
        return AuthorColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha,
        };
    }
    let convert = |color: AuthorColor| {
        let linear = [color.red, color.green, color.blue];
        match space {
            GradientInterpolation::Srgb => linear.map(encode_srgb_channel),
            GradientInterpolation::LinearSrgb => linear,
            GradientInterpolation::Oklab => linear_to_oklab(linear),
            GradientInterpolation::Oklch { .. } => {
                let lab = linear_to_oklab(linear);
                [
                    lab[0],
                    crate::math::hypot(lab[1], lab[2]),
                    crate::math::atan2(lab[2], lab[1])
                        .to_degrees()
                        .rem_euclid(360.0),
                ]
            }
        }
    };
    let mut ca = convert(a);
    let mut cb = convert(b);
    if let GradientInterpolation::Oklch { hue } = space {
        if ca[1] < 0.000004 {
            ca[2] = cb[2];
        }
        if cb[1] < 0.000004 {
            cb[2] = ca[2];
        }
        fix_hues(&mut ca[2], &mut cb[2], hue);
    }
    let mut mixed =
        core::array::from_fn(|i| (ca[i] * a.alpha * (1.0 - t) + cb[i] * b.alpha * t) / alpha);
    let linear = match space {
        GradientInterpolation::LinearSrgb => mixed,
        GradientInterpolation::Srgb => mixed.map(decode_srgb_channel),
        GradientInterpolation::Oklab => oklab_to_linear(mixed),
        GradientInterpolation::Oklch { .. } => {
            mixed[2] = ca[2] + (cb[2] - ca[2]) * t;
            let (sin, cos) = crate::math::sin_cos(mixed[2].to_radians());
            oklab_to_linear([mixed[0], mixed[1] * cos, mixed[1] * sin])
        }
    };
    // Motion author colors currently target the sRGB delivery gamut. Clip the result while it
    // is still straight linear sRGB; otherwise the generic output transform's optional chroma
    // compression changes the specified CSS interpolation midpoint at rasterization time.
    AuthorColor {
        red: linear[0].clamp(0.0, 1.0),
        green: linear[1].clamp(0.0, 1.0),
        blue: linear[2].clamp(0.0, 1.0),
        alpha,
    }
}

/// Subdivide until quarter/midpoint errors are below 1/16384 in premultiplied linear
/// Rec.2020. Hard stops retain both colors. Bounded subdivision fails rather than silently
/// degrading quality or allocating an unbounded stop table.
pub(super) fn lower_stops(
    source: &[super::recording::GradientStop],
    interpolation: GradientInterpolation,
    opacity: f32,
) -> Result<Vec<GradientStop>, &'static str> {
    const LIMIT: usize = super::validate::MAX_GRADIENT_STOPS;
    if source.len() > LIMIT {
        return Err("gradient stop budget exceeded");
    }
    let mut out = Vec::new();
    let Some(first) = source.first() else {
        return Ok(out);
    };
    out.push(GradientStop {
        offset: first.offset as f32,
        color: first.color.to_working().scale_opacity(opacity),
    });
    for pair in source.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let sample = |t: f64| {
            match (a.color, b.color) {
                (
                    super::recording::GradientStopColor::Byte(a),
                    super::recording::GradientStopColor::Byte(b),
                ) => interpolate(a, b, t, interpolation),
                (a, b) => interpolate_author_colors(a.to_author(), b.to_author(), t, interpolation)
                    .to_working(),
            }
            .scale_opacity(opacity)
        };
        let endpoint = sample(1.0);
        if a.offset < b.offset && interpolation != GradientInterpolation::LinearSrgb {
            subdivide(
                &mut out,
                &sample,
                a.offset,
                b.offset,
                0.0,
                1.0,
                sample(0.0),
                endpoint,
                0,
            )?;
        } else {
            if out.len() == LIMIT {
                return Err("gradient stop budget exceeded");
            }
            out.push(GradientStop {
                offset: b.offset as f32,
                color: endpoint,
            });
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn subdivide(
    out: &mut Vec<GradientStop>,
    sample: &impl Fn(f64) -> LinearColor,
    start: f64,
    end: f64,
    a: f64,
    b: f64,
    ca: LinearColor,
    cb: LinearColor,
    depth: u32,
) -> Result<(), &'static str> {
    let mut split = false;
    for fraction in [0.25, 0.5, 0.75] {
        let actual = sample(a + (b - a) * fraction);
        for (x, y, actual) in [
            (ca.red, cb.red, actual.red),
            (ca.green, cb.green, actual.green),
            (ca.blue, cb.blue, actual.blue),
            (ca.alpha, cb.alpha, actual.alpha),
        ] {
            split |= (actual - (x + (y - x) * fraction as f32)).abs() > 1.0 / 16384.0;
        }
    }
    if split {
        if depth == 16 {
            return Err("gradient interpolation subdivision budget exceeded");
        }
        let mid = (a + b) * 0.5;
        let color = sample(mid);
        subdivide(out, sample, start, end, a, mid, ca, color, depth + 1)?;
        subdivide(out, sample, start, end, mid, b, color, cb, depth + 1)?;
    } else {
        if out.len() == super::validate::MAX_GRADIENT_STOPS {
            return Err("gradient stop budget exceeded");
        }
        out.push(GradientStop {
            offset: (start + (end - start) * b) as f32,
            color: cb,
        });
    }
    Ok(())
}

fn interpolate(
    a: crate::Rgba,
    b: crate::Rgba,
    t: f64,
    space: GradientInterpolation,
) -> LinearColor {
    if t == 0.0 {
        return LinearColor::from_srgb8(a);
    }
    if t == 1.0 {
        return LinearColor::from_srgb8(b);
    }
    let alpha_a = f64::from(a.a) / 255.0;
    let alpha_b = f64::from(b.a) / 255.0;
    let alpha = alpha_a + (alpha_b - alpha_a) * t;
    if alpha == 0.0 {
        return LinearColor::new(0.0, 0.0, 0.0, 0.0);
    }
    let convert = |color: crate::Rgba| {
        let encoded = [color.r, color.g, color.b].map(|v| f64::from(v) / 255.0);
        if space == GradientInterpolation::Srgb {
            return encoded;
        }
        let linear = encoded.map(decode_srgb_channel);
        if space == GradientInterpolation::LinearSrgb {
            return linear;
        }
        let lab = linear_to_oklab(linear);
        if matches!(space, GradientInterpolation::Oklch { .. }) {
            [
                lab[0],
                crate::math::hypot(lab[1], lab[2]),
                crate::math::atan2(lab[2], lab[1])
                    .to_degrees()
                    .rem_euclid(360.0),
            ]
        } else {
            lab
        }
    };
    let mut ca = convert(a);
    let mut cb = convert(b);
    if let GradientInterpolation::Oklch { hue } = space {
        // Achromatic endpoints have powerless hue, borrowed from the other endpoint.
        if ca[1] < 0.000004 {
            ca[2] = cb[2];
        }
        if cb[1] < 0.000004 {
            cb[2] = ca[2];
        }
        fix_hues(&mut ca[2], &mut cb[2], hue);
    }
    let mut mixed =
        core::array::from_fn(|i| (ca[i] * alpha_a * (1.0 - t) + cb[i] * alpha_b * t) / alpha);
    let linear = match space {
        GradientInterpolation::LinearSrgb => mixed,
        GradientInterpolation::Srgb => mixed.map(decode_srgb_channel),
        GradientInterpolation::Oklab => oklab_to_linear(mixed),
        GradientInterpolation::Oklch { .. } => {
            // CSS polar hue is interpolated without alpha premultiplication.
            mixed[2] = ca[2] + (cb[2] - ca[2]) * t;
            let (sin, cos) = crate::math::sin_cos(mixed[2].to_radians());
            oklab_to_linear([mixed[0], mixed[1] * cos, mixed[1] * sin])
        }
    };
    from_linear_srgb(linear, alpha as f32)
}

fn fix_hues(a: &mut f64, b: &mut f64, direction: HueDirection) {
    let delta = *b - *a;
    match direction {
        HueDirection::Shorter if delta > 180.0 => *a += 360.0,
        HueDirection::Shorter if delta < -180.0 => *b += 360.0,
        HueDirection::Longer if delta > 0.0 && delta < 180.0 => *a += 360.0,
        HueDirection::Longer if delta <= 0.0 && delta > -180.0 => *b += 360.0,
        HueDirection::Increasing if delta < 0.0 => *b += 360.0,
        HueDirection::Decreasing if delta > 0.0 => *a += 360.0,
        _ => {}
    }
}

// CSS Color 4 sample conversion matrices, D65. Keep all non-polynomial math in libm.
fn linear_to_oklab([r, g, b]: [f64; 3]) -> [f64; 3] {
    let l = crate::math::cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
    let m = crate::math::cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
    let s = crate::math::cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

fn oklab_to_linear([l, a, b]: [f64; 3]) -> [f64; 3] {
    let cube = |v: f64| v * v * v;
    let lp = cube(l + 0.3963377774 * a + 0.2158037573 * b);
    let mp = cube(l - 0.1055613458 * a - 0.0638541728 * b);
    let sp = cube(l - 0.0894841775 * a - 1.2914855480 * b);
    [
        4.0767416621 * lp - 3.3077115913 * mp + 0.2309699292 * sp,
        -1.2684380046 * lp + 2.6097574011 * mp - 0.3413193965 * sp,
        -0.0041960863 * lp - 0.7034186147 * mp + 1.7076147010 * sp,
    ]
}

/// Bit corpus shared by the native and Wasm deterministic-math runners.
pub(crate) fn determinism_samples() -> Vec<crate::math::DeterminismSample> {
    let mut samples = Vec::new();
    for space in [
        GradientInterpolation::Srgb,
        GradientInterpolation::LinearSrgb,
        GradientInterpolation::Oklab,
        GradientInterpolation::Oklch {
            hue: HueDirection::Shorter,
        },
        GradientInterpolation::Oklch {
            hue: HueDirection::Longer,
        },
        GradientInterpolation::Oklch {
            hue: HueDirection::Increasing,
        },
        GradientInterpolation::Oklch {
            hue: HueDirection::Decreasing,
        },
    ] {
        for alpha in [0, 64, 255] {
            for t in [0.0, 0.125, 0.5, 0.875, 1.0] {
                let color = interpolate(
                    crate::Rgba::new(255, 0, 0, alpha),
                    crate::Rgba::new(0, 0, 255, 255),
                    t,
                    space,
                );
                samples.push(crate::math::DeterminismSample {
                    operation: "gradientInterpolation",
                    inputs: vec![
                        serde_json::to_string(&space).unwrap(),
                        alpha.to_string(),
                        format!("{:016x}", t.to_bits()),
                    ],
                    output: [color.red, color.green, color.blue, color.alpha]
                        .iter()
                        .map(|v| format!("{:08x}", v.to_bits()))
                        .collect(),
                });
                let author = interpolate_author_colors(
                    AuthorColor::from_srgb8(crate::Rgba::new(0x21, 0x40, 0xff, alpha)),
                    AuthorColor::from_srgb8(crate::Rgba::rgb(0xff, 0xd0, 0x00)),
                    t,
                    space,
                );
                let working = author.to_working();
                let mut output = [author.red, author.green, author.blue, author.alpha]
                    .iter()
                    .map(|v| format!("{:016x}", v.to_bits()))
                    .collect::<Vec<_>>();
                output.extend(
                    [working.red, working.green, working.blue, working.alpha]
                        .iter()
                        .map(|v| format!("{:08x}", v.to_bits())),
                );
                samples.push(crate::math::DeterminismSample {
                    operation: "authorColorInterpolation",
                    inputs: vec![
                        serde_json::to_string(&space).unwrap(),
                        alpha.to_string(),
                        format!("{:016x}", t.to_bits()),
                    ],
                    output: output.concat(),
                });
            }
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rgba;

    #[test]
    fn author_oklch_midpoint_stays_float_until_paint() {
        let blue = AuthorColor::from_srgb8(Rgba::rgb(0x21, 0x40, 0xff));
        let yellow = AuthorColor::from_srgb8(Rgba::rgb(0xff, 0xd0, 0x00));
        let midpoint = interpolate_author_colors(
            blue,
            yellow,
            0.5,
            GradientInterpolation::Oklch {
                hue: HueDirection::Shorter,
            },
        );
        let actual = midpoint.to_srgb8();
        for (actual, expected) in [(actual.r, 0), (actual.g, 196), (actual.b, 159)] {
            assert!(
                (i16::from(actual) - expected).abs() <= 6,
                "{actual} != {expected}"
            );
        }
        let srgb =
            interpolate_author_colors(blue, yellow, 0.5, GradientInterpolation::Srgb).to_srgb8();
        assert_eq!((srgb.r, srgb.g), (144, 136));
        assert!((i16::from(srgb.b) - 128).abs() <= 1);
        assert_eq!(midpoint.alpha, 1.0);
        assert!(midpoint.to_working().red.is_finite());
    }

    fn close(a: [f64; 3], b: [f64; 3], tolerance: f64) {
        for (a, b) in a.into_iter().zip(b) {
            assert!((a - b).abs() < tolerance, "{a} != {b}");
        }
    }

    fn linear_srgb(color: LinearColor) -> [f64; 3] {
        let a = f64::from(color.alpha);
        let [r, g, b] = [color.red, color.green, color.blue].map(|v| f64::from(v) / a);
        [
            1.6604910021084345 * r - 0.5876411387885495 * g - 0.07284986331988493 * b,
            -0.12455047452159074 * r + 1.1328998971259598 * g - 0.008349422604369476 * b,
            -0.018150763354905303 * r - 0.1005788980080074 * g + 1.118729661362913 * b,
        ]
    }

    #[test]
    fn known_oklab_primaries_and_css_gradient_midpoint() {
        close(
            linear_to_oklab([1.0, 0.0, 0.0]),
            [0.62795536, 0.22486306, 0.12584630],
            1e-7,
        );
        close(
            linear_to_oklab([0.0, 0.0, 1.0]),
            [0.45201372, -0.03245698, -0.31152815],
            1e-7,
        );
        for rgb in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.2, 0.5, 0.8],
        ] {
            close(oklab_to_linear(linear_to_oklab(rgb)), rgb, 2e-7);
        }
        let red = Rgba::rgb(255, 0, 0);
        let blue = Rgba::rgb(0, 0, 255);
        let srgb =
            linear_srgb(interpolate(red, blue, 0.5, GradientInterpolation::Oklab)).map(|v| {
                255.0
                    * if v <= 0.0031308 {
                        v * 12.92
                    } else {
                        1.055 * v.powf(1.0 / 2.4) - 0.055
                    }
            });
        close(srgb, [140.0, 83.0, 162.0], 0.6);
        close(
            linear_srgb(interpolate(
                red,
                blue,
                0.5,
                GradientInterpolation::LinearSrgb,
            )),
            [0.5, 0.0, 0.5],
            1e-6,
        );
        close(
            linear_srgb(interpolate(red, blue, 0.5, GradientInterpolation::Srgb)),
            [0.21404114, 0.0, 0.21404114],
            1e-6,
        );
    }

    #[test]
    fn alpha_is_premultiplied_and_hue_directions_are_distinct() {
        for space in [
            GradientInterpolation::Oklab,
            GradientInterpolation::Srgb,
            GradientInterpolation::LinearSrgb,
        ] {
            let mid = interpolate(Rgba::new(255, 0, 0, 0), Rgba::rgb(0, 0, 255), 0.5, space);
            assert_eq!(mid.alpha, 0.5);
            close(linear_srgb(mid), [0.0, 0.0, 1.0], 1e-6);
        }
        let polar = |hue| {
            interpolate(
                Rgba::rgb(255, 0, 0),
                Rgba::rgb(0, 0, 255),
                0.5,
                GradientInterpolation::Oklch { hue },
            )
        };
        assert_eq!(
            polar(HueDirection::Shorter),
            polar(HueDirection::Decreasing)
        );
        assert_eq!(polar(HueDirection::Longer), polar(HueDirection::Increasing));
        assert_ne!(polar(HueDirection::Shorter), polar(HueDirection::Longer));
    }

    #[test]
    fn adaptive_stops_preserve_hard_edges_alpha_and_dense_sample_accuracy() {
        let stop = |offset, color| super::super::recording::GradientStop {
            offset,
            color: super::super::recording::GradientStopColor::Byte(color),
        };
        for (a, b) in [
            (Rgba::rgb(255, 0, 0), Rgba::rgb(0, 0, 255)),
            (Rgba::new(255, 255, 0, 13), Rgba::new(0, 255, 255, 237)),
        ] {
            for space in [
                GradientInterpolation::Srgb,
                GradientInterpolation::Oklab,
                GradientInterpolation::Oklch {
                    hue: HueDirection::Longer,
                },
            ] {
                let stops = lower_stops(&[stop(0.0, a), stop(1.0, b)], space, 0.7).unwrap();
                assert!(stops.len() < 1024);
                for sample in 0..1001 {
                    let t = f64::from(sample) / 1000.0;
                    let interval = stops
                        .windows(2)
                        .find(|s| f64::from(s[1].offset) >= t)
                        .unwrap();
                    let fraction =
                        (t as f32 - interval[0].offset) / (interval[1].offset - interval[0].offset);
                    let actual = interpolate(a, b, t, space).scale_opacity(0.7);
                    let ca = interval[0].color;
                    let cb = interval[1].color;
                    for (a, b, actual) in [
                        (ca.red, cb.red, actual.red),
                        (ca.green, cb.green, actual.green),
                        (ca.blue, cb.blue, actual.blue),
                    ] {
                        assert!((a + (b - a) * fraction - actual).abs() < 1.0 / 8192.0);
                    }
                }
            }
        }
        let stops = lower_stops(
            &[
                stop(0.0, Rgba::rgb(255, 0, 0)),
                stop(0.5, Rgba::rgb(255, 0, 0)),
                stop(0.5, Rgba::rgb(0, 0, 255)),
                stop(1.0, Rgba::rgb(0, 0, 255)),
            ],
            GradientInterpolation::Oklab,
            1.0,
        )
        .unwrap();
        let edge: Vec<_> = stops.iter().filter(|s| s.offset == 0.5).collect();
        assert_eq!(edge.len(), 2);
        assert_eq!(edge[0].color, LinearColor::from_srgb8(Rgba::rgb(255, 0, 0)));
        assert_eq!(edge[1].color, LinearColor::from_srgb8(Rgba::rgb(0, 0, 255)));
        let oversized =
            vec![stop(0.0, Rgba::rgb(0, 0, 0)); super::super::validate::MAX_GRADIENT_STOPS + 1];
        assert!(lower_stops(&oversized, GradientInterpolation::Oklab, 1.0).is_err());
    }
}
