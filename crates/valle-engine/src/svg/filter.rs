//! SVG filter primitives retain their named inputs until the complete definition is known.
//! The resolved graph is a DAG: inputs can only name a standard source or an earlier result.
pub mod convolve;
pub mod image;
pub mod lighting;
pub mod turbulence;

use std::collections::HashMap;
use std::rc::Rc;

use super::{Element, Length};
use crate::style::{Style, color};
use crate::template::{Tag, V};

#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub user_space: bool,
    pub primitive_user_space: bool,
    pub region: [Length; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Primitive {
    pub inputs: [Option<Rc<str>>; 2],
    pub result: Option<Rc<str>>,
    pub region: [Option<Length>; 4],
    pub op: Operation,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Blur([f64; 2]),
    Offset([f64; 2]),
    Flood,
    Tile,
    Image(image::Image),
    Lighting(lighting::Lighting),
    Light(lighting::Light),
    Convolve(convolve::Kernel),
    Turbulence(turbulence::Noise),
    Matrix([f64; 20]),
    Blend(u32),
    Composite { operator: u32, k: [f64; 4] },
    Merge,
    MergeNode,
    Morphology { dilate: bool, radius: [f64; 2] },
    Displacement { scale: f64, channels: [u32; 2] },
    Transfer,
    Channel(usize, Transfer),
    Shadow { offset: [f64; 2], sigma: [f64; 2] },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Transfer {
    Identity,
    Table(Vec<f64>, bool),
    Linear([f64; 2]),
    Gamma([f64; 3]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// 0 = SourceGraphic, 1 = SourceAlpha; subsequent values index earlier nodes + 2.
    pub inputs: Vec<usize>,
    pub region: [Option<Length>; 4],
    pub op: Operation,
    pub linear: bool,
    pub flood: color::Rgba,
    pub transfer: [Transfer; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    pub definition: Definition,
    pub nodes: Vec<Node>,
}

pub fn definition(attributes: &[(&str, &V)]) -> Result<Definition, String> {
    let a = Attributes(attributes);
    let space = |name, default| match a.string(name).as_deref() {
        None => Ok(default),
        Some("userSpaceOnUse") => Ok(true),
        Some("objectBoundingBox") => Ok(false),
        _ => Err(format!(
            "{name}: expected userSpaceOnUse or objectBoundingBox"
        )),
    };
    let mut region = [Length {
        em: 0.0,
        units: 0.0,
        percent: -10.0,
    }; 4];
    for (i, name) in ["x", "y", "width", "height"].into_iter().enumerate() {
        region[i] = a.length(name)?.unwrap_or(Length {
            em: 0.0,
            units: 0.0,
            percent: if i < 2 { -10.0 } else { 120.0 },
        });
        if i >= 2 {
            region[i].nonnegative(name)?;
        }
    }
    Ok(Definition {
        user_space: space("filterUnits", false)?,
        primitive_user_space: space("primitiveUnits", true)?,
        region,
    })
}

pub fn primitive(tag: Tag, attributes: &[(&str, &V)]) -> Result<Primitive, String> {
    let a = Attributes(attributes);
    let name = tag.name();
    let pair = |key, default| a.pair(key, default, true);
    let op = match tag {
        Tag::FeGaussianBlur => {
            if a.string("edgeMode").is_some_and(|v| v != "none") {
                return Err("feGaussianBlur edgeMode: only none is supported".into());
            }
            Operation::Blur(pair("stdDeviation", 0.0)?)
        }
        Tag::FeOffset => Operation::Offset([a.number("dx", 0.0)?, a.number("dy", 0.0)?]),
        Tag::FeFlood => Operation::Flood,
        Tag::FeTile => Operation::Tile,
        Tag::FeImage => Operation::Image(image::Image::parse(&a)?),
        Tag::FeDiffuseLighting | Tag::FeSpecularLighting => {
            Operation::Lighting(lighting::Lighting::parse(tag, &a)?)
        }
        Tag::FeDistantLight | Tag::FePointLight | Tag::FeSpotLight => {
            Operation::Light(lighting::Light::parse(tag, &a)?)
        }
        Tag::FeConvolveMatrix => Operation::Convolve(convolve::Kernel::parse(&a)?),
        Tag::FeTurbulence => {
            let octaves = a.number("numOctaves", 1.)?;
            if octaves.fract() != 0. || !(0. ..=f64::from(u32::MAX)).contains(&octaves) {
                return Err("numOctaves: expected a nonnegative integer".into());
            }
            Operation::Turbulence(turbulence::Noise {
                frequency: pair("baseFrequency", 0.)?,
                octaves: octaves as u32,
                fractal: a.choice("type", &["turbulence", "fractalNoise"], 0)? == 1,
                stitch: a.choice("stitchTiles", &["noStitch", "stitch"], 0)? == 1,
                table: turbulence::table(a.number("seed", 0.)?),
            })
        }
        Tag::FeBlend => Operation::Blend(a.choice(
            "mode",
            &[
                "normal",
                "multiply",
                "screen",
                "overlay",
                "darken",
                "lighten",
                "color-dodge",
                "color-burn",
                "hard-light",
                "soft-light",
                "difference",
                "exclusion",
                "hue",
                "saturation",
                "color",
                "luminosity",
            ],
            0,
        )?),
        Tag::FeComposite => Operation::Composite {
            operator: a.choice(
                "operator",
                &["over", "in", "out", "atop", "xor", "arithmetic"],
                0,
            )?,
            k: [
                a.number("k1", 0.0)?,
                a.number("k2", 0.0)?,
                a.number("k3", 0.0)?,
                a.number("k4", 0.0)?,
            ],
        },
        Tag::FeMerge => Operation::Merge,
        Tag::FeMergeNode => Operation::MergeNode,
        Tag::FeMorphology => Operation::Morphology {
            dilate: a.choice("operator", &["erode", "dilate"], 0)? == 1,
            radius: pair("radius", 0.0)?,
        },
        Tag::FeDisplacementMap => Operation::Displacement {
            scale: a.number("scale", 0.0)?,
            channels: [
                a.choice("xChannelSelector", &["R", "G", "B", "A"], 3)?,
                a.choice("yChannelSelector", &["R", "G", "B", "A"], 3)?,
            ],
        },
        Tag::FeDropShadow => Operation::Shadow {
            offset: [a.number("dx", 2.0)?, a.number("dy", 2.0)?],
            sigma: pair("stdDeviation", 2.0)?,
        },
        Tag::FeComponentTransfer => Operation::Transfer,
        Tag::FeFuncR | Tag::FeFuncG | Tag::FeFuncB | Tag::FeFuncA => {
            let channel = match tag {
                Tag::FeFuncR => 0,
                Tag::FeFuncG => 1,
                Tag::FeFuncB => 2,
                _ => 3,
            };
            let transfer = match a.choice(
                "type",
                &["identity", "table", "discrete", "linear", "gamma"],
                0,
            )? {
                1 | 2 => Transfer::Table(
                    a.numbers("tableValues")?.unwrap_or_default(),
                    a.string("type").as_deref() == Some("discrete"),
                ),
                3 => Transfer::Linear([a.number("slope", 1.0)?, a.number("intercept", 0.0)?]),
                4 => Transfer::Gamma([
                    a.number("amplitude", 1.0)?,
                    a.number("exponent", 1.0)?,
                    a.number("offset", 0.0)?,
                ]),
                _ => Transfer::Identity,
            };
            Operation::Channel(channel, transfer)
        }
        Tag::FeColorMatrix => Operation::Matrix(matrix(&a)?),
        _ => return Err(format!("{name} is not a filter primitive")),
    };
    let mut region = [None; 4];
    for (i, key) in ["x", "y", "width", "height"].into_iter().enumerate() {
        region[i] = a.length(key)?;
        if i >= 2
            && let Some(v) = region[i]
        {
            v.nonnegative(key)?;
        }
    }
    Ok(Primitive {
        inputs: [a.string("in"), a.string("in2")].map(|v| v.map(Rc::from)),
        result: a.string("result").map(Rc::from),
        region,
        op,
    })
}

struct Attributes<'a>(&'a [(&'a str, &'a V)]);
impl Attributes<'_> {
    fn get(&self, name: &str) -> Option<&V> {
        self.0.iter().find_map(|(n, v)| (*n == name).then_some(*v))
    }
    fn string(&self, name: &str) -> Option<String> {
        self.get(name).map(|v| v.text().trim().to_string())
    }
    fn length(&self, name: &str) -> Result<Option<Length>, String> {
        self.get(name)
            .map(super::length)
            .transpose()
            .map_err(|e| format!("{name}: {e}"))
    }
    fn number(&self, name: &str, default: f64) -> Result<f64, String> {
        let Some(v) = self.get(name) else {
            return Ok(default);
        };
        let n = v
            .text()
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("{name}: expected a number"))?;
        if n.is_finite() {
            Ok(n)
        } else {
            Err(format!("{name}: expected a finite number"))
        }
    }
    fn numbers(&self, name: &str) -> Result<Option<Vec<f64>>, String> {
        self.string(name)
            .map(|s| {
                crate::path::numbers(&s)
                    .map(|v| v.into_iter().map(f64::from).collect())
                    .ok_or_else(|| format!("{name}: expected numbers"))
            })
            .transpose()
    }
    fn pair(&self, name: &str, default: f64, nonnegative: bool) -> Result<[f64; 2], String> {
        let v = self.numbers(name)?.unwrap_or_else(|| vec![default]);
        let v = match v[..] {
            [a] => [a, a],
            [a, b] => [a, b],
            _ => return Err(format!("{name}: expected one or two numbers")),
        };
        if nonnegative && v.iter().any(|v| *v < 0.0) {
            return Err(format!("{name}: must not be negative"));
        }
        Ok(v)
    }
    fn choice(&self, name: &str, choices: &[&str], default: u32) -> Result<u32, String> {
        self.string(name).map_or(Ok(default), |s| {
            choices
                .iter()
                .position(|c| *c == s)
                .map(|i| i as u32)
                .ok_or_else(|| format!("{name}: {s:?} is not one of {}", choices.join(", ")))
        })
    }
}

fn matrix(a: &Attributes) -> Result<[f64; 20], String> {
    let identity = [
        1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0.,
    ];
    let values = a.numbers("values")?;
    match a.choice(
        "type",
        &["matrix", "saturate", "hueRotate", "luminanceToAlpha"],
        0,
    )? {
        0 => values.map_or(Ok(identity), |v| {
            v.try_into()
                .map_err(|_| "feColorMatrix: matrix needs 20 values".into())
        }),
        kind @ (1 | 2) => {
            let v = values.unwrap_or_else(|| vec![if kind == 1 { 1.0 } else { 0.0 }]);
            let [amount] = v[..] else {
                return Err("feColorMatrix: saturate/hueRotate needs one value".into());
            };
            let (c, s) = if kind == 1 {
                (amount, 0.0)
            } else {
                (amount.to_radians().cos(), amount.to_radians().sin())
            };
            Ok([
                0.213 + 0.787 * c - 0.213 * s,
                0.715 - 0.715 * c - 0.715 * s,
                0.072 - 0.072 * c + 0.928 * s,
                0.,
                0.,
                0.213 - 0.213 * c + 0.143 * s,
                0.715 + 0.285 * c + 0.140 * s,
                0.072 - 0.072 * c - 0.283 * s,
                0.,
                0.,
                0.213 - 0.213 * c - 0.787 * s,
                0.715 - 0.715 * c + 0.715 * s,
                0.072 + 0.928 * c + 0.072 * s,
                0.,
                0.,
                0.,
                0.,
                0.,
                1.,
                0.,
            ])
        }
        _ => Ok([
            0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0.2125, 0.7154, 0.0721, 0.,
            0.,
        ]),
    }
}

/// Build every definition once per computed style tree. Forward/missing named results are
/// errors with the authored name, instead of silently sampling an unrelated intermediate.
pub(crate) fn prepare(styles: &mut [Style], parents: &[Option<u32>]) -> Result<(), String> {
    let mut local_images = false;
    for i in 0..styles.len() {
        let Some(Element::Filter(definition)) = styles[i].svg.as_deref().map(|c| &c.element) else {
            continue;
        };
        let definition = definition.clone();
        let mut names = HashMap::<Rc<str>, usize>::new();
        let mut nodes = Vec::new();
        for k in i + 1..styles.len() {
            if parents[k] != Some(i as u32) {
                continue;
            }
            let Some(Element::Primitive(p)) = styles[k].svg.as_deref().map(|c| &c.element) else {
                continue;
            };
            let previous = if nodes.is_empty() { 0 } else { nodes.len() + 1 };
            let input = |v: Option<&str>| -> Result<usize, String> {
                match v {
                    None => Ok(previous),
                    Some("SourceGraphic") => Ok(0),
                    Some("SourceAlpha") => Ok(1),
                    Some("BackgroundImage" | "BackgroundAlpha" | "FillPaint" | "StrokePaint") => {
                        Err(
                            "filter input supports SourceGraphic, SourceAlpha or an earlier result"
                                .into(),
                        )
                    }
                    Some(v) => names.get(v).copied().ok_or_else(|| {
                        format!("filter input {v:?} does not name an earlier result")
                    }),
                }
            };
            let mut inputs = match p.op {
                Operation::Flood | Operation::Turbulence(_) | Operation::Image(_) => Vec::new(),
                Operation::Merge => Vec::new(),
                Operation::Blend(_)
                | Operation::Composite { .. }
                | Operation::Displacement { .. } => vec![
                    input(p.inputs[0].as_deref())?,
                    input(p.inputs[1].as_deref())?,
                ],
                _ => vec![input(p.inputs[0].as_deref())?],
            };
            let mut op = p.op.clone();
            if let Operation::Image(image) = &mut op {
                image.node = k;
                local_images |= image.href.starts_with('#');
            }
            let mut transfer = std::array::from_fn(|_| Transfer::Identity);
            for child in k + 1..styles.len() {
                if parents[child] != Some(k as u32) {
                    continue;
                }
                if let Some(Element::Primitive(c)) =
                    styles[child].svg.as_deref().map(|c| &c.element)
                {
                    match &c.op {
                        Operation::Light(source) => {
                            if let Operation::Lighting(lighting) = &mut op
                                && lighting.light.replace(source.clone()).is_some()
                            {
                                return Err("lighting needs exactly one light source".into());
                            }
                        }
                        Operation::MergeNode => inputs.push(input(c.inputs[0].as_deref())?),
                        Operation::Channel(channel, value) => transfer[*channel] = value.clone(),
                        _ => {}
                    }
                }
            }
            let style = &styles[k];
            if let Operation::Lighting(lighting) = &mut op {
                if lighting.light.is_none() {
                    return Err("lighting needs exactly one light source".into());
                }
                lighting.color = color::resolve(&style.lighting_color, style.color);
            }
            let mut flood = color::resolve(&style.flood_color, style.color);
            flood[3] *= style.flood_opacity;
            nodes.push(Node {
                inputs,
                region: p.region,
                op,
                linear: style.filter_linear,
                flood,
                transfer,
            });
            if let Some(result) = &p.result {
                if result.is_empty() {
                    return Err("filter result must not be empty".into());
                }
                names.insert(result.clone(), nodes.len() + 1);
            }
        }
        styles[i].filter_graph = Some(Rc::new(Graph { definition, nodes }));
    }
    if local_images {
        image::validate(styles, parents)?;
    }
    Ok(())
}

impl Graph {
    /// Standard sources and earlier results which contribute to the final primitive.
    pub fn needed(&self) -> Vec<bool> {
        let mut needed = vec![false; self.nodes.len() + 2];
        let mut pending = vec![needed.len() - 1];
        while let Some(i) = pending.pop() {
            if std::mem::replace(&mut needed[i], true) {
                continue;
            }
            if i >= 2 {
                pending.extend(self.nodes[i - 2].inputs.iter().copied());
            }
        }
        needed
    }

    pub fn region(&self, bbox: [f64; 4], viewport: [f64; 2]) -> [f64; 4] {
        std::array::from_fn(|i| {
            coordinate(
                self.definition.region[i],
                i,
                self.definition.user_space,
                bbox,
                viewport,
            )
        })
    }

    pub fn regions(&self, bbox: [f64; 4], viewport: [f64; 2]) -> Vec<[f64; 4]> {
        let region = self.region(bbox, viewport);
        let mut regions = vec![region; 2];
        for n in &self.nodes {
            let default = if matches!(n.op, Operation::Tile)
                || n.inputs.is_empty()
                || n.inputs.iter().any(|i| *i < 2)
            {
                region
            } else {
                n.inputs
                    .iter()
                    .map(|i| regions[*i])
                    .reduce(union)
                    .unwrap_or(region)
            };
            let sub = std::array::from_fn(|i| {
                n.region[i].map_or(default[i], |v| {
                    coordinate(v, i, self.definition.primitive_user_space, bbox, viewport)
                })
            });
            regions.push(intersection(sub, region));
        }
        regions
    }
}

fn coordinate(v: Length, i: usize, user: bool, bbox: [f64; 4], viewport: [f64; 2]) -> f64 {
    let axis = i % 2;
    if user {
        v.resolve(viewport[axis])
    } else {
        (if i < 2 { bbox[axis] } else { 0.0 }) + (v.units + v.percent / 100.0) * bbox[axis + 2]
    }
}
fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let x = a[0].min(b[0]);
    let y = a[1].min(b[1]);
    [
        x,
        y,
        (a[0] + a[2]).max(b[0] + b[2]) - x,
        (a[1] + a[3]).max(b[1] + b[3]) - y,
    ]
}
fn intersection(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    [
        x,
        y,
        ((a[0] + a[2]).min(b[0] + b[2]) - x).max(0.0),
        ((a[1] + a[3]).min(b[1] + b[3]) - y).max(0.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{Kind, Node, Prop};
    use crate::testing::{node, styles::sheet};

    fn svg(tag: Tag, attrs: &[(&str, &str)], children: &[u32]) -> Node {
        node(
            Kind::Svg {
                tag,
                attributes: attrs
                    .iter()
                    .map(|(k, v)| (Rc::from(*k), Prop::Const(V::Str((*v).into()))))
                    .collect(),
                geometry: None,
            },
            &[],
            children,
        )
    }

    #[test]
    fn named_branches_defaults_and_dynamic_parameters_survive_seeking() {
        let mut blur = svg(Tag::FeGaussianBlur, &[("result", "soft")], &[]);
        if let Kind::Svg { attributes, .. } = &mut blur.kind {
            attributes.push(("stdDeviation".into(), Prop::Expr(0)));
        }
        let sheet = sheet(vec![
            svg(Tag::Svg, &[("width", "100"), ("height", "100")], &[1]),
            svg(
                Tag::Filter,
                &[("id", "f"), ("colorInterpolationFilters", "sRGB")],
                &[2, 3, 4, 5],
            ),
            blur,
            svg(
                Tag::FeOffset,
                &[("in", "soft"), ("result", "soft"), ("dx", "2")],
                &[],
            ),
            svg(Tag::FeBlend, &[("in", "soft"), ("in2", "soft")], &[]),
            svg(Tag::FeMerge, &[], &[6, 7]),
            svg(Tag::FeMergeNode, &[], &[]),
            svg(Tag::FeMergeNode, &[("in", "SourceAlpha")], &[]),
        ])
        .unwrap();
        for sigma in [0., 3., 1., 0.] {
            let styles = sheet
                .compute(&[V::Num(sigma)], true, [100., 100.], &[])
                .unwrap();
            let graph = styles[1].filter_graph.as_ref().unwrap();
            assert_eq!(
                graph
                    .nodes
                    .iter()
                    .map(|n| n.inputs.clone())
                    .collect::<Vec<_>>(),
                [vec![0], vec![2], vec![3, 3], vec![4, 1]]
            );
            assert_eq!(graph.nodes[0].op, Operation::Blur([sigma; 2]));
            assert!(graph.nodes.iter().all(|n| !n.linear));
        }
    }

    #[test]
    fn regions_use_filter_units_primitive_units_and_input_union() {
        let sheet = sheet(vec![
            svg(Tag::Svg, &[], &[1]),
            svg(
                Tag::Filter,
                &[("primitiveUnits", "objectBoundingBox")],
                &[2, 3, 4],
            ),
            svg(
                Tag::FeFlood,
                &[
                    ("x", "25%"),
                    ("y", "25%"),
                    ("width", "50%"),
                    ("height", "50%"),
                    ("result", "a"),
                ],
                &[],
            ),
            svg(
                Tag::FeFlood,
                &[
                    ("x", "0"),
                    ("y", "0"),
                    ("width", "0.25"),
                    ("height", "0.25"),
                ],
                &[],
            ),
            svg(Tag::FeBlend, &[("in2", "a")], &[]),
        ])
        .unwrap();
        let styles = sheet.compute(&[], true, [200., 100.], &[]).unwrap();
        let graph = styles[1].filter_graph.as_ref().unwrap();
        assert_eq!(
            graph.regions([20., 10., 80., 40.], [200., 100.]),
            [
                [12., 6., 96., 48.],
                [12., 6., 96., 48.],
                [40., 20., 40., 20.],
                [20., 10., 20., 10.],
                [20., 10., 60., 30.],
            ]
        );
    }

    #[test]
    fn lighting_requires_one_source_and_resolves_current_color() {
        for sources in [0, 1, 2] {
            let mut nodes = vec![
                svg(Tag::Svg, &[], &[1]),
                svg(Tag::Filter, &[], &[2]),
                svg(
                    Tag::FeDiffuseLighting,
                    &[("lightingColor", "currentColor"), ("color", "#804020")],
                    &(3..3 + sources).collect::<Vec<_>>(),
                ),
            ];
            for _ in 0..sources {
                nodes.push(svg(Tag::FeDistantLight, &[("elevation", "90")], &[]));
            }
            let styles = sheet(nodes).unwrap().compute(&[], true, [100., 100.], &[]);
            if sources == 1 {
                let styles = styles.unwrap();
                let graph = styles[1].filter_graph.as_ref().unwrap();
                let Operation::Lighting(lighting) = &graph.nodes[0].op else {
                    panic!("expected lighting");
                };
                assert_eq!(lighting.light, Some(lighting::Light::Distant([0., 90.])));
                assert_eq!(lighting.color, [128. / 255., 64. / 255., 32. / 255., 1.]);
            } else {
                assert!(styles.unwrap_err().contains("exactly one light source"));
            }
        }
    }

    #[test]
    fn invalid_primitives_and_unavailable_inputs_report_authored_names() {
        for (tag, attrs, message) in [
            (
                Tag::FeGaussianBlur,
                vec![("stdDeviation", "-1")],
                "must not be negative",
            ),
            (
                Tag::FeTurbulence,
                vec![("numOctaves", "1.5")],
                "nonnegative integer",
            ),
            (
                Tag::FeTurbulence,
                vec![("baseFrequency", "-1")],
                "must not be negative",
            ),
            (
                Tag::FeTurbulence,
                vec![("stitchTiles", "repeat")],
                "not one of",
            ),
            (
                Tag::FeDiffuseLighting,
                vec![("diffuseConstant", "-1")],
                "must not be negative",
            ),
            (
                Tag::FeDiffuseLighting,
                vec![("kernelUnitLength", "0")],
                "positive",
            ),
            (
                Tag::FeSpecularLighting,
                vec![("specularExponent", "0")],
                "between 1 and 128",
            ),
            (
                Tag::FeSpecularLighting,
                vec![("specularExponent", "129")],
                "between 1 and 128",
            ),
            (
                Tag::FeSpecularLighting,
                vec![("specularConstant", "-1")],
                "must not be negative",
            ),
            (Tag::FeMorphology, vec![("radius", "1 2 3")], "one or two"),
            (Tag::FeColorMatrix, vec![("values", "1 2")], "20 values"),
            (
                Tag::FeDisplacementMap,
                vec![("xChannelSelector", "Z")],
                "not one of",
            ),
        ] {
            let values: Vec<_> = attrs
                .iter()
                .map(|(k, v)| (*k, V::Str((*v).into())))
                .collect();
            let refs: Vec<_> = values.iter().map(|(k, v)| (*k, v)).collect();
            assert!(primitive(tag, &refs).unwrap_err().contains(message));
        }
        let sheet = sheet(vec![
            svg(Tag::Svg, &[], &[1]),
            svg(Tag::Filter, &[], &[2]),
            svg(Tag::FeOffset, &[("in", "future")], &[]),
        ])
        .unwrap();
        assert!(
            sheet
                .compute(&[], true, [100., 100.], &[])
                .unwrap_err()
                .contains("future")
        );
    }
}
