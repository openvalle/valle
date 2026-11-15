//! SVG's alpha-height lighting parameters and its three light sources.
use super::Attributes;
use crate::style::color;
use crate::template::Tag;

#[derive(Clone, Debug, PartialEq)]
pub enum Light {
    Distant([f64; 2]),
    Point([f64; 3]),
    Spot {
        position: [f64; 3],
        target: [f64; 3],
        exponent: f64,
        cone: Option<f64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lighting {
    pub surface: f64,
    pub specular: Option<f64>,
    pub constant: f64,
    pub unit: Option<[f64; 2]>,
    pub light: Option<Light>,
    pub color: color::Rgba,
}

impl Lighting {
    pub(super) fn parse(tag: Tag, a: &Attributes<'_>) -> Result<Self, String> {
        let specular = (tag == Tag::FeSpecularLighting)
            .then(|| a.number("specularExponent", 1.))
            .transpose()?;
        if specular.is_some_and(|v| !(1. ..=128.).contains(&v)) {
            return Err("specularExponent: must be between 1 and 128".into());
        }
        let name = if specular.is_some() {
            "specularConstant"
        } else {
            "diffuseConstant"
        };
        let constant = a.number(name, 1.)?;
        if constant < 0. {
            return Err(format!("{name}: must not be negative"));
        }
        let unit = a
            .get("kernelUnitLength")
            .map(|_| a.pair("kernelUnitLength", 1., true))
            .transpose()?;
        if unit.is_some_and(|u| u.iter().any(|v| *v <= 0.)) {
            return Err("kernelUnitLength: must be positive".into());
        }
        Ok(Self {
            surface: a.number("surfaceScale", 1.)?,
            specular,
            constant,
            unit,
            light: None,
            color: [1.; 4],
        })
    }
}

impl Light {
    pub(super) fn parse(tag: Tag, a: &Attributes<'_>) -> Result<Self, String> {
        if tag == Tag::FeDistantLight {
            return Ok(Self::Distant([
                a.number("azimuth", 0.)?,
                a.number("elevation", 0.)?,
            ]));
        }
        let position = [a.number("x", 0.)?, a.number("y", 0.)?, a.number("z", 0.)?];
        if tag == Tag::FePointLight {
            return Ok(Self::Point(position));
        }
        let target = [
            a.number("pointsAtX", 0.)?,
            a.number("pointsAtY", 0.)?,
            a.number("pointsAtZ", 0.)?,
        ];
        let exponent = a.number("specularExponent", 1.)?;
        if exponent < 0. {
            return Err("specularExponent: must not be negative".into());
        }
        let cone = a
            .get("limitingConeAngle")
            .map(|_| a.number("limitingConeAngle", 0.))
            .transpose()?;
        Ok(Self::Spot {
            position,
            target,
            exponent,
            cone,
        })
    }

    pub fn parameters(&self, origin: [f64; 2], units: [f64; 3]) -> [f32; 12] {
        let point = |p: [f64; 3]| {
            [
                (p[0] * units[0] + origin[0]) as f32,
                (p[1] * units[1] + origin[1]) as f32,
                (p[2] * units[2]) as f32,
            ]
        };
        let mut data = [0.; 12];
        match *self {
            Self::Distant([azimuth, elevation]) => {
                let (a, e) = (azimuth.to_radians(), elevation.to_radians());
                data[1..4].copy_from_slice(&[
                    (a.cos() * e.cos()) as f32,
                    (a.sin() * e.cos()) as f32,
                    e.sin() as f32,
                ]);
            }
            Self::Point(position) => {
                data[0] = 1.;
                data[1..4].copy_from_slice(&point(position));
            }
            Self::Spot {
                position,
                target,
                exponent,
                cone,
            } => {
                data[0] = 2.;
                data[1..4].copy_from_slice(&point(position));
                data[4..7].copy_from_slice(&point(target));
                data[7] = exponent as f32;
                data[8] = cone.map_or(-1., |a| a.to_radians().cos() as f32);
            }
        }
        data
    }
}
