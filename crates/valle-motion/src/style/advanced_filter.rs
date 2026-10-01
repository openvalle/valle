//! Advanced filters share the ordinary CSS filter cascade and evaluated string grammar.
use valle_draw::{
    Rgba,
    program::{AuthorColor, recording::FilterOp},
};

pub(crate) const SLOT: &str = "--tw-motion-filter";
pub(crate) const IMPORTANT_SLOT: &str = "--tw-important-motion-filter";

/// Numeric positions in the public filter grammar, shared by parsing and
/// compile-time range diagnostics. Colors are intentionally not numeric slots.
#[derive(Clone, Copy)]
pub struct Parameter {
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f64,
    pub max: f64,
}

pub fn parameters(name: &str) -> &'static [Parameter] {
    const fn p(name: &'static str, unit: &'static str, min: f64, max: f64) -> Parameter {
        Parameter {
            name,
            unit,
            min,
            max,
        }
    }
    match name {
        "glow" => const { &[p("radius", "px", 0.0, 128.0), p("intensity", "", 0.0, 4.0)] },
        "chromatic-aberration" => const { &[p("offset", "px", -256.0, 256.0)] },
        "radial-blur" => {
            const {
                &[
                    p("center x", "px", -10_000_000.0, 10_000_000.0),
                    p("center y", "px", -10_000_000.0, 10_000_000.0),
                    p("amount", "px", 0.0, 128.0),
                ]
            }
        }
        "film-grain" => {
            const {
                &[
                    p("seed", "", 0.0, u32::MAX as f64),
                    p("amount", "", 0.0, 1.0),
                    p("size", "px", 1.0, 64.0),
                ]
            }
        }
        "lens-distortion" => const { &[p("k1", "", -0.5, 0.5), p("k2", "", -0.5, 0.5)] },
        _ => &[],
    }
}

pub fn parse(value: &str) -> Result<Option<FilterOp>, String> {
    let value = value.trim();
    let Some((name, body)) = value.split_once('(') else {
        return Ok(None);
    };
    if !matches!(
        name,
        "glow" | "chromatic-aberration" | "radial-blur" | "film-grain" | "lens-distortion"
    ) {
        return Ok(None);
    }
    let invalid = || {
        format!(
            "invalid {name}() parameters: {}",
            match name {
                "glow" => "radius 0..=128px, intensity 0..=4, and color",
                "chromatic-aberration" => "one finite pixel offset within ±256px",
                "radial-blur" => "center x/y within ±10000000px and amount 0..=128px",
                "film-grain" => "unsigned integer seed, amount 0..=1, size 1..=64px",
                _ => "two finite coefficients within ±0.5",
            }
        )
    };
    let body = body.strip_suffix(')').ok_or_else(invalid)?;
    let parts = body.split_whitespace().collect::<Vec<_>>();
    let number = |index: usize, px: bool, low: f64, high: f64| -> Result<f64, String> {
        let raw = *parts.get(index).ok_or_else(invalid)?;
        let raw = if px {
            raw.strip_suffix("px").ok_or_else(invalid)?
        } else {
            raw
        };
        let value = raw.parse::<f64>().map_err(|_| {
            format!(
                "{name} {} expects a finite number, got {raw:?}",
                parameters(name)[index].name
            )
        })?;
        if !value.is_finite() || !(low..=high).contains(&value) {
            let parameter = parameters(name)[index];
            return Err(format!(
                "{name} {} = {value}{} is outside {low}..={high}{}; use clamp() explicitly if intended",
                parameter.name, parameter.unit, parameter.unit
            ));
        }
        Ok(value)
    };
    Ok(Some(match name {
        "glow" if parts.len() >= 3 => FilterOp::Glow {
            radius: number(0, true, 0.0, 128.0)?,
            intensity: number(1, false, 0.0, 4.0)?,
            color: AuthorColor::from_srgb8(Rgba::parse(&parts[2..].join(" ")).ok_or_else(invalid)?),
        },
        "chromatic-aberration" if parts.len() == 1 => FilterOp::ChromaticAberration {
            offset_x: number(0, true, -256.0, 256.0)?,
            offset_y: 0.0,
        },
        "radial-blur" if parts.len() == 3 => FilterOp::RadialBlur {
            center_x: number(0, true, -10_000_000.0, 10_000_000.0)?,
            center_y: number(1, true, -10_000_000.0, 10_000_000.0)?,
            amount: number(2, true, 0.0, 128.0)?,
        },
        "film-grain" if parts.len() == 3 => {
            let seed = number(0, false, 0.0, u32::MAX as f64)?;
            if seed.fract() != 0.0 {
                return Err(invalid());
            }
            FilterOp::FilmGrain {
                seed: seed as u32,
                amount: number(1, false, 0.0, 1.0)?,
                size: number(2, true, 1.0, 64.0)?,
            }
        }
        "lens-distortion" if parts.len() == 2 => FilterOp::LensDistortion {
            k1: number(0, false, -0.5, 0.5)?,
            k2: number(1, false, -0.5, 0.5)?,
        },
        _ => return Err(invalid()),
    }))
}
