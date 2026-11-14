//! Shared ellipse formulas produce compact rows, without a computed CSS style per seed.

use std::rc::Rc;

use super::Formula;
use crate::style::Paint;
use crate::style::color::Rgba;
use crate::style::declaration::{Declared, presented, set};
use crate::style::property::Field;
use crate::svg::{self, Element, Shape};
use crate::template::{Instances, Prop, Tag, V};

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub shape: Shape,
    pub fill: Option<Paint>,
    pub fill_opacity: Option<f64>,
    pub even_odd: Option<bool>,
    pub color: Option<Rgba>,
    pub opacity: f64,
}

pub(crate) struct Table {
    formula: Formula,
    rows: Vec<Bound>,
    fields: Vec<Rc<str>>,
    place: Rc<str>,
}

struct Bound {
    fixed: Row,
    local: Vec<V>,
}

impl Row {
    fn set(&mut self, name: &str, value: &V, scratch: &mut Declared) -> Result<(), String> {
        match name {
            "cx" | "cy" => {
                let center = match &mut self.shape {
                    Shape::Circle { center, .. } | Shape::Ellipse { center, .. } => center,
                    _ => unreachable!(),
                };
                center[usize::from(name == "cy")] = svg::length(value)?;
            }
            "r" => {
                let Shape::Circle { r, .. } = &mut self.shape else {
                    unreachable!()
                };
                *r = svg::length(value)?;
            }
            "rx" | "ry" => {
                let Shape::Ellipse { r, .. } = &mut self.shape else {
                    unreachable!()
                };
                r[usize::from(name == "ry")] = match value {
                    V::Str(s) if s.trim() == "auto" => None,
                    _ => Some(svg::length(value)?),
                };
            }
            _ => {
                let field = Field::by_name(name).expect("eligible SVG attribute");
                set(scratch, field, &presented(value, true))?;
                match name {
                    "fill" => self.fill.clone_from(&scratch.svg_fill),
                    "fillOpacity" => self.fill_opacity = scratch.fill_opacity,
                    "fillRule" => self.even_odd = scratch.even_odd,
                    "color" => self.color = scratch.color,
                    "opacity" => self.opacity = scratch.opacity,
                    _ => unreachable!(),
                }
            }
        }
        Ok(())
    }
}

impl Table {
    pub fn new(table: &Instances, tag: Tag, place: Rc<str>) -> Result<Self, String> {
        let formula = Formula::new(table).ok_or("an SVG table needs a shared formula")?;
        let Element::Shape(shape) = svg::computed(tag, &[], None)?.element else {
            return Err("an SVG table needs a shape".into());
        };
        let mut rows = Vec::with_capacity(table.rows.len());
        let mut scratch = Declared::default();
        for data in &table.rows {
            let local = formula
                .prepare(data)
                .map_err(|e| format!("{place}: TypeError: {e}"))?;
            let mut fixed = Row {
                shape: shape.clone(),
                fill: None,
                fill_opacity: None,
                even_odd: None,
                color: None,
                opacity: 1.0,
            };
            for (name, prop) in table.fields.iter().zip(&formula.shared.outputs) {
                let value = match prop {
                    Prop::Const(v) => v,
                    Prop::Expr(e) if !formula.dynamic[*e as usize] => &local[*e as usize],
                    _ => continue,
                };
                fixed
                    .set(name, value, &mut scratch)
                    .map_err(|e| format!("{place}: attribute {name}: {e}"))?;
            }
            rows.push(Bound { fixed, local });
        }
        Ok(Self {
            formula,
            rows,
            fields: table.fields.clone(),
            place,
        })
    }

    pub fn reads(&self, f: impl FnMut(u32)) {
        self.formula.shared.imports.iter().copied().for_each(f);
    }

    pub fn compute(&self, values: &[V]) -> Result<Rc<Vec<Row>>, String> {
        let mut rows = Vec::with_capacity(self.rows.len());
        let mut local = Vec::new();
        let mut scratch = Declared::default();
        for row in &self.rows {
            self.formula
                .evaluate(&row.local, values, &mut local)
                .map_err(|e| format!("{}: {e}", self.place))?;
            let mut computed = row.fixed.clone();
            for (name, prop) in self.fields.iter().zip(&self.formula.shared.outputs) {
                if let Prop::Expr(e) = prop
                    && self.formula.dynamic[*e as usize]
                {
                    computed
                        .set(name, &local[*e as usize], &mut scratch)
                        .map_err(|e| format!("{}: attribute {name}: {e}", self.place))?;
                }
            }
            rows.push(computed);
        }
        Ok(Rc::new(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{front::Inputs, scene::Scene, time::Tick};

    fn run(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    fn load(source: &str, ordinary: bool) -> Scene {
        let source = format!(
            "export const composition = {{width:320,height:180,fps:30,duration:3}};{}",
            if ordinary {
                source.replace("map(i=>", "map((i,index,array)=>")
            } else {
                source.into()
            },
        );
        Scene::load(
            "s.motion.tsx",
            &|p| (p == "s.motion.tsx").then(|| source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap()
    }

    fn dots(attributes: &str, parent: &str, wrapper: &str) -> String {
        format!(
            r#"
            function Dots() {{
                const {{seconds:t}}=useCurrentTime();
                return <svg key="dots" width={{320}} height={{180}} viewBox="0 0 320 180">
                    <g {parent}>{{[0,1,2,3].map(i=><circle key={{`dot-${{i}}`}}
                        cx={{60+i*17+t*3}} cy={{60+Math.sin(t+i)*12}} r={{14+t*2}}
                        {attributes}/>)}}</g>
                </svg>;
            }}
            export default function Scene() {{ return <View>{wrapper}</View>; }}
        "#
        )
    }

    #[test]
    fn ellipse_rows_match_independent_shapes_and_reuse_layout() {
        run(|| {
            for tag in ["circle", "ellipse"] {
                for parent in [
                    "",
                    "fill=\"orange\" fillOpacity={0.4} color=\"blue\" opacity={0.7} transform=\"translate(7 3) rotate(12)\"",
                    "fill=\"none\" stroke=\"red\" strokeWidth={3} strokeDasharray=\"3 2\"",
                    "visibility=\"hidden\"",
                ] {
                    let source = dots("fill={i%2?'currentColor':'#39aacc99'} fillOpacity={0.5+0.2*Math.sin(t+i)} opacity={0.8}", parent, "<Dots/>")
                        .replace("<circle", &format!("<{tag}"));
                    let source = if tag == "ellipse" {
                        source.replace("r={14+t*2}", "rx={14+t*2} ry=\"8%\"")
                    } else {
                        source
                    };
                    let mut table = load(&source, false);
                    let mut plain = load(&source, true);
                    let first = table.frame(Tick::ZERO).unwrap();
                    assert_eq!(first.template.instances.len(), 1);
                    assert_eq!(first.template.instances[0].1.rows.len(), 4);
                    assert_eq!(first.template.nodes.len(), 4);
                    for t in ["0", "1.5", "0.25", "2.9"] {
                        let at = Tick::parse_seconds(t).unwrap();
                        let a = table.frame(at).unwrap();
                        let b = plain.frame(at).unwrap();
                        assert_eq!(a.items, b.items, "{tag} {parent} at {t}");
                        assert!(Rc::ptr_eq(&first.layout, &a.layout));
                    }
                }
            }
        });
    }

    #[test]
    fn references_bounds_and_temporal_samples_keep_their_semantics() {
        run(|| {
            for wrapper in [
                "<Dots/><View style={{opacity:bounds('dots').width/100}}/>",
                "<TimeScope offset={0.2} rate={2}><Dots/></TimeScope>",
                "<Echo count={2} delay={0.1} decay={0.5}><Dots/></Echo>",
                "<Shutter samples={4} angle={180}><Dots/></Shutter>",
                "<View style={{perspective:500}}><Dots/></View>",
            ] {
                let source = dots("fill=\"orange\" opacity={0.8}", "", wrapper);
                let mut a = load(&source, false);
                let mut b = load(&source, true);
                for t in ["0", "1.5", "0.2"] {
                    let at = Tick::parse_seconds(t).unwrap();
                    assert_eq!(
                        a.frame(at).unwrap().items,
                        b.frame(at).unwrap().items,
                        "{wrapper}"
                    );
                }
            }
            let source = indoc::indoc! {r##"
                export default function Scene() {
                    const {seconds:t}=useCurrentTime();
                    return <svg width={320} height={180}>
                        <defs><g id="dots">{[0,1,2].map(i=><circle
                            cx={20+i*25+noise2d(7,t,i)} cy={20}
                            r={interpolate(t,[i,i+1],[2,8])}/>)}</g></defs>
                        <use href="#dots" fill="red"/>
                        <use href="#dots" y={40} fill="blue"/>
                    </svg>;
                }
            "##};
            let mut a = load(source, false);
            let mut b = load(source, true);
            let at = Tick::parse_seconds("1").unwrap();
            let a = a.frame(at).unwrap();
            assert!(a.template.instances.is_empty());
            assert_eq!(a.items, b.frame(at).unwrap().items);

            let mixed = source.replace("return <svg", "return <View>{[0,1,2].map(i=><View className=\"absolute\" style={{left:i*20,top:0,width:10,height:10,backgroundColor:'red'}}/>)}<svg")
                .replace("</svg>;", "</svg></View>;");
            let mut scene = load(&mixed, false);
            let frame = scene.frame(at).unwrap();
            assert_eq!(
                frame.template.instances.len(),
                1,
                "SVG references keep box instancing"
            );
            assert!(matches!(
                frame.template.nodes[frame.template.instances[0].0 as usize].kind,
                crate::template::Kind::View
            ));
        });
    }

    #[test]
    fn invalid_later_rows_still_fail() {
        run(|| {
            for attr in [
                "cx={t<i ? 20 : 'bad'}",
                "fill={t<i ? 'red' : 'bad'}",
                "opacity={1/(i-t)}",
            ] {
                let source = format!(
                    r#"export default function Scene() {{const {{seconds:t}}=useCurrentTime(); return <svg>{{[1,2,3].map(i=><circle r={{10}} {attr}/>)}}</svg>;}}"#
                );
                for ordinary in [false, true] {
                    let mut checker = load(&source, ordinary).into_checker();
                    checker.check(Tick::ZERO).unwrap();
                    assert!(
                        checker.check(Tick::parse_seconds("1").unwrap()).is_err(),
                        "{attr}"
                    );
                }
            }
        });
    }

    #[test]
    fn batched_ellipses_match_independent_pixels() {
        if !std::env::var("VALLE_TEST_LAYERS")
            .unwrap_or_default()
            .split(',')
            .any(|x| x == "gpu")
        {
            return;
        }
        run(|| {
            let gpu = valle_native::gpu::adapter(None)
                .and_then(|a| crate::gpu::Gpu::with(a.adapter))
                .unwrap();
            let mut renderer = crate::gpu::Renderer::new(&gpu, 320, 180);
            for attr in [
                "fill={i%2?'currentColor':'#39aacc99'} fillOpacity={0.5+0.2*Math.sin(t+i)}",
                "fill={i%2?'none':'#ef348b'}",
                "fill=\"orange\" opacity={0.5}",
            ] {
                for wrapper in [
                    "<Dots/>",
                    "<View style={{opacity:0.6,filter:'blur(1px)',rotate:'8deg'}}><Dots/></View>",
                    "<Echo count={2} delay={0.1} decay={0.5}><Dots/></Echo>",
                    "<Shutter samples={4} angle={180}><Dots/></Shutter>",
                ] {
                    let source = dots(attr, "color=\"red\"", wrapper);
                    for tag in ["circle", "ellipse"] {
                        let source = if tag == "ellipse" {
                            source
                                .replace("<circle", "<ellipse")
                                .replace("r={14+t*2}", "rx={14+t*2} ry=\"12%\"")
                        } else {
                            source.clone()
                        };
                        let mut a = load(&source, false);
                        let mut b = load(&source, true);
                        for t in ["0", "1.5", "0.2", "2.9"] {
                            let at = Tick::parse_seconds(t).unwrap();
                            let x = a.frame(at).unwrap();
                            let y = b.frame(at).unwrap();
                            let x = renderer
                                .render(&x.items, &x.pictures, a.faces(), a.images())
                                .unwrap();
                            let y = renderer
                                .render(&y.items, &y.pictures, b.faces(), b.images())
                                .unwrap();
                            assert_eq!(x, y, "{tag} {attr} {wrapper} at {t}");
                        }
                    }
                }
            }
        });
    }
}
