//! Repeated absolute leaf boxes, kept as rows through style, layout and painting. Eligibility
//! is deliberately structural: unsupported declarations keep the ordinary node path.

use std::rc::Rc;

use super::computed::{Style, computed_transforms};
use super::declaration::{Declared, set};
use super::property::Field;
use super::units::Units;
use super::{color, position, transform};
use crate::layout::Rect;
use crate::template::{Instances, Prop, V};

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub rect: Rect,
    pub radii: [[super::math::Calc; 2]; 4],
    pub color: color::Rgba,
    pub opacity: f64,
    pub transform: Vec<transform::Function>,
    pub individual: usize,
    pub origin: [position::Offset; 2],
    pub origin_z: f64,
}

pub(super) struct Table {
    rows: Vec<Bound>,
    place: Rc<str>,
}

struct Bound {
    fixed: Declared,
    updates: Vec<(Field, Prop, Rc<str>)>,
}

impl Table {
    pub fn new(table: &Instances, place: Rc<str>) -> Result<Self, String> {
        let mut rows = Vec::with_capacity(table.rows.len());
        for row in &table.rows {
            let mut d = Declared::default();
            let mut rest = Vec::new();
            for (name, prop) in table.fields.iter().zip(row) {
                let field = Field::by_name(name).expect("eligible instance field");
                // These eligible fields are independent; no shorthand overrides another
                // column. Parse every constant once even when a dynamic column precedes it.
                if let Prop::Const(v) = prop {
                    set(&mut d, field, v).map_err(|e| format!("{place}: style {name}: {e}"))?;
                } else {
                    rest.push((field, prop.clone(), name.clone()));
                }
            }
            rows.push(Bound {
                fixed: d,
                updates: rest,
            });
        }
        Ok(Self { rows, place })
    }

    pub fn reads(&self, mut f: impl FnMut(u32)) {
        for row in &self.rows {
            for (_, p, _) in &row.updates {
                if let Prop::Expr(e) = p {
                    f(*e);
                }
            }
        }
    }

    pub fn compute(
        &self,
        values: &[V],
        parent: &Style,
        canvas: [f64; 2],
    ) -> Result<Rc<Vec<Row>>, String> {
        let u = Units {
            em: parent.font.size,
            canvas,
        };
        let mut rows = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let mut d = row.fixed.clone();
            for (field, prop, name) in &row.updates {
                let v = match prop {
                    Prop::Const(v) => v,
                    Prop::Expr(e) => &values[*e as usize],
                };
                set(&mut d, *field, v).map_err(|e| format!("{}: style {name}: {e}", self.place))?;
            }
            rows.push(Row {
                rect: Rect {
                    x: d.inset[3].px(u, 0.0) as f32,
                    y: d.inset[0].px(u, 0.0) as f32,
                    width: d.size[0].px(u, 0.0) as f32,
                    height: d.size[1].px(u, 0.0) as f32,
                },
                radii: d.radii.each_ref().map(|r| r.each_ref().map(|l| l.calc(u))),
                color: d.background.unwrap_or(parent.color),
                opacity: d.opacity,
                transform: computed_transforms(&d, u),
                individual: [&d.translate, &d.rotate, &d.scale]
                    .into_iter()
                    .flatten()
                    .count(),
                origin: d.transform_origin.each_ref().map(|p| p.computed(u)),
                origin_z: d.transform_origin_z.px(u, 0.0),
            });
        }
        Ok(Rc::new(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{front::Inputs, scene::Scene, time::Tick};

    fn scene(style: &str, parent: &str, extra: &str) -> Scene {
        let additions = [style, extra]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        let source = format!(
            r#"
            export const composition = {{width:320,height:180,fps:30,duration:3}};
            export default function Scene() {{
                const {{seconds:t}} = useCurrentTime();
                return <View style={{{{width:300,height:160,position:'relative',backgroundColor:'#172130',{parent}}}}}>
                    {{Array.from({{length:12}},(_,i)=><View key={{`dot-${{i}}`}} className="absolute" style={{{{
                        left:10+i*15+t*7,top:20+(i%3)*25,width:29.25,height:19.5,
                        borderRadius:7,backgroundColor:i%2?'#e97635':'#49c5f2',{additions}
                    }}}}/> )}}
                </View>;
            }}
        "#
        );
        Scene::load(
            "s.motion.tsx",
            &|p| (p == "s.motion.tsx").then(|| source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap()
    }

    fn run(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn repeated_leaves_keep_rows_without_rebuilding_layout_nodes() {
        run(|| {
            let mut scene = scene("translate:`${t*3}px 0px`", "", "");
            let first = scene.frame(Tick::ZERO).unwrap();
            assert_eq!(first.template.nodes.len(), 2);
            assert_eq!(first.template.instances[0].1.rows.len(), 12);
            let next = scene.frame(Tick::parse_seconds("1").unwrap()).unwrap();
            assert!(Rc::ptr_eq(&first.layout, &next.layout));
            assert_ne!(first.items, next.items);
        });
    }

    #[test]
    fn transparent_rows_do_not_expand_the_drawn_area() {
        run(|| {
            let mut table = scene("backgroundColor:'transparent'", "", "");
            let mut plain = scene("backgroundColor:'transparent'", "", "margin:0");
            let table = table.frame(Tick::ZERO).unwrap();
            let plain = plain.frame(Tick::ZERO).unwrap();
            assert!(!table.template.instances.is_empty());
            assert_eq!(table.items, plain.items);
        });
    }

    #[test]
    fn invalid_later_values_are_still_checked() {
        run(|| {
            for extra in ["", "margin:0"] {
                let mut check = scene("opacity: t<1 ? 1 : 'broken'", "", extra).into_checker();
                check.check(Tick::ZERO).unwrap();
                assert!(
                    check
                        .check(Tick::parse_seconds("2").unwrap())
                        .unwrap_err()
                        .contains("opacity")
                );
            }
            let mut ordinary = scene("boxShadow:'0px 2px 3px black'", "", "");
            assert!(
                ordinary
                    .frame(Tick::ZERO)
                    .unwrap()
                    .template
                    .instances
                    .is_empty()
            );
        });
    }

    #[test]
    fn bounds_and_3d_scenes_keep_individual_nodes() {
        run(|| {
            for (style, parent) in [
                ("opacity:bounds('dot-0').width/100", ""),
                ("", "transformStyle:'preserve-3d'"),
                ("", "perspective:500"),
            ] {
                let mut scene = scene(style, parent, "");
                assert!(
                    scene
                        .frame(Tick::ZERO)
                        .unwrap()
                        .template
                        .instances
                        .is_empty()
                );
            }
            let source = indoc::indoc! {r#"
                export const composition = {width:320,height:180,fps:30,duration:1};
                export default function Scene() {
                    return <View className="transform-3d">
                        {Array.from({length:3}, (_,i) => <View className="absolute" style={{
                            width:10,height:10,left:i*10,top:0,rotate:'10deg'
                        }}/>) }
                    </View>;
                }
            "#};
            let mut scene = Scene::load(
                "s.motion.tsx",
                &|p| (p == "s.motion.tsx").then(|| source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap();
            assert!(
                scene
                    .frame(Tick::ZERO)
                    .unwrap()
                    .template
                    .instances
                    .is_empty()
            );
        });
    }

    #[test]
    fn instance_pixels_match_independent_boxes() {
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
            for parent in [
                "",
                "overflow:'hidden',padding:9,border:'3px solid white'",
                "translate:'13.25px 7.5px',rotate:'7deg',opacity:0.7",
                "visibility:'hidden'",
                "display:'none'",
                "position:'static',margin:17",
                "display:'grid',padding:11,border:'2px solid white'",
                "filter:'blur(2px)',borderRadius:13,overflow:'hidden'",
            ] {
                for style in [
                    "",
                    "opacity:0.5",
                    "transform:`rotate(${t*20}deg) skewX(9deg)`",
                    "translate:`${t*3}px 2px`,scale:0.7,rotate:'17deg'",
                    "borderRadius:'50%',backgroundColor:'rgb(200 30 100 / 0.4)',opacity:0.5,transformOrigin:'0px 100%'",
                ] {
                    let mut table = scene(style, parent, "");
                    let mut boxes = scene(style, parent, "margin:0");
                    for at in ["0", "0.5", "1.25", "2.9"] {
                        let at = Tick::parse_seconds(at).unwrap();
                        let a = table.frame(at).unwrap();
                        let b = boxes.frame(at).unwrap();
                        let a = renderer
                            .render(&a.items, &a.pictures, table.faces(), table.images())
                            .unwrap();
                        let b = renderer
                            .render(&b.items, &b.pictures, boxes.faces(), boxes.images())
                            .unwrap();
                        let worst = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                        assert!(
                            worst <= 1,
                            "parent={parent} style={style} at={at:?} difference={worst}"
                        );
                    }
                }
            }
        });
    }
}
