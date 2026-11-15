//! Shared basic shape formulas produce compact rows, without a computed CSS style per seed.

use std::rc::Rc;

use super::Formula;
use crate::style::Paint;
use crate::style::color::Specified;
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
    pub color: Option<Specified>,
    pub opacity: f64,
    inherit_opacity: bool,
    pub stroke: Option<Paint>,
    pub stroke_opacity: Option<f64>,
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
        use crate::style::cascade::Keyword;
        if let Some(keyword) = Keyword::of(value) {
            let inherit = keyword != Keyword::Initial;
            match name {
                "fill" => {
                    self.fill = (!inherit).then_some(Paint::Color(crate::style::color::BLACK))
                }
                "stroke" => self.stroke = (!inherit).then_some(Paint::None),
                "color" => self.color = (!inherit).then(|| crate::style::color::BLACK.into()),
                "fillOpacity" => self.fill_opacity = (!inherit).then_some(1.0),
                "strokeOpacity" => self.stroke_opacity = (!inherit).then_some(1.0),
                "fillRule" => self.even_odd = (!inherit).then_some(false),
                "opacity" => {
                    self.opacity = 1.0;
                    self.inherit_opacity = keyword == Keyword::Inherit;
                }
                _ => return Err(format!("{name} is SVG geometry, not a CSS property")),
            }
            return Ok(());
        }
        match name {
            "x" | "y" | "width" | "height" => {
                let Shape::Rect { at, size, .. } = &mut self.shape else {
                    unreachable!()
                };
                let (values, axis) = match name {
                    "x" => (at, 0),
                    "y" => (at, 1),
                    "width" => (size, 0),
                    _ => (size, 1),
                };
                let length = svg::length(value)?;
                values[axis] = match name {
                    "width" | "height" => length.nonnegative(name)?,
                    _ => length,
                };
            }
            "x1" | "y1" | "x2" | "y2" => {
                let Shape::Line(points) = &mut self.shape else {
                    unreachable!()
                };
                let i = match name {
                    "x1" => 0,
                    "y1" => 1,
                    "x2" => 2,
                    _ => 3,
                };
                points[i] = svg::length(value)?;
            }
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
                *r = svg::length(value)?.nonnegative(name)?;
            }
            "rx" | "ry" => {
                let r = match &mut self.shape {
                    Shape::Ellipse { r, .. } | Shape::Rect { r, .. } => r,
                    _ => unreachable!(),
                };
                r[usize::from(name == "ry")] = match value {
                    V::Str(s) if s.trim() == "auto" => None,
                    _ => Some(svg::length(value)?.nonnegative(name)?),
                };
            }
            _ => {
                let field = Field::by_name(name).expect("eligible SVG attribute");
                set(scratch, field, &presented(value, true))?;
                match name {
                    "fill" => self.fill.clone_from(&scratch.svg_fill),
                    "fillOpacity" => self.fill_opacity = scratch.fill_opacity,
                    "fillRule" => self.even_odd = scratch.even_odd,
                    "color" => self.color.clone_from(&scratch.color),
                    "opacity" => {
                        self.opacity = scratch.opacity;
                        self.inherit_opacity = false;
                    }
                    "stroke" => self.stroke.clone_from(&scratch.svg_stroke),
                    "strokeOpacity" => self.stroke_opacity = scratch.stroke_opacity,
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
                inherit_opacity: false,
                stroke: None,
                stroke_opacity: None,
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

    pub fn compute(
        &self,
        values: &[V],
        parent: Option<&crate::style::Style>,
    ) -> Result<Rc<Vec<Row>>, String> {
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
            if computed.inherit_opacity {
                computed.opacity = parent.map_or(1.0, |p| p.opacity);
            }
            computed
                .shape
                .resolve_font(parent.map_or(16.0, |p| p.font.size));
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

    #[test]
    fn em_geometry_tracks_inherited_font_size_in_tables_and_ordinary_nodes() {
        run(|| {
            let source = r#"
                export default function Scene() {
                    const {seconds:t} = useCurrentTime();
                    return <svg width={320} height={180}>
                        <g fontSize={20 + t * 10}>
                            {[0,1,2].map(i=><rect x={`${i * 3}em`} y="1em" width="2em" height="1em"/>)}
                        </g>
                    </svg>;
                }
            "#;
            let mut table = load(source, false);
            let mut ordinary = load(source, true);
            for second in [0, 1, 2, 0] {
                let t = Tick::parse_seconds(&second.to_string()).unwrap();
                let a = table.frame(t).unwrap();
                let b = ordinary.frame(t).unwrap();
                assert_eq!(a.items, b.items);
            }
        });
    }

    fn dots(attributes: &str, parent: &str, wrapper: &str) -> String {
        shapes(
            "circle",
            "cx={60+i*17+t*3} cy={60+Math.sin(t+i)*12} r={14+t*2}",
            attributes,
            parent,
            wrapper,
        )
    }

    fn shapes(tag: &str, geometry: &str, attributes: &str, parent: &str, wrapper: &str) -> String {
        format!(
            r#"
            function Dots() {{
                const {{seconds:t}}=useCurrentTime();
                return <svg key="dots" width={{320}} height={{180}} viewBox="0 0 320 180">
                    <g {parent}>{{[0,1,2,3].map(i=><{tag} key={{`dot-${{i}}`}}
                        {geometry} {attributes}/>)}}</g>
                </svg>;
            }}
            export default function Scene() {{ return <View>{wrapper}</View>; }}
        "#
        )
    }

    const SHAPES: [(&str, &str); 2] = [
        (
            "rect",
            "x={40+i*25+t*3} y={35+Math.sin(t+i)*12} width={5+i*10+t*3} height=\"30%\" rx={2+t*0.2} ry=\"8%\"",
        ),
        (
            "line",
            "x1={40+i*25+t*3} y1=\"30%\" x2={i*30+60} y2={60+Math.sin(t+i)*12}",
        ),
    ];

    #[test]
    fn rectangle_and_line_rows_preserve_styles_geometry_and_layout() {
        run(|| {
            for (tag, geometry) in SHAPES {
                for attributes in [
                    "fill={i%2?'currentColor':'#39aacc99'} color={i%2?'red':'blue'} opacity={0.8}",
                    "fill=\"none\" stroke={i%2?'currentColor':'#39aacc99'} strokeOpacity={0.5+0.2*Math.sin(t+i)} strokeWidth={4} strokeLinecap=\"round\"",
                    "stroke={i%2?'inherit':'none'} fill={i%2?'none':'inherit'} strokeDasharray=\"3 2\" strokeDashoffset={1} strokeLinejoin=\"bevel\" strokeMiterlimit={2} pathLength={100} paintOrder=\"stroke fill\"",
                    "fill=\"orange\" stroke=\"red\" strokeWidth=\"0.2em\" transform=\"translate(7 3) rotate(12)\"",
                ] {
                    for parent in [
                        "",
                        "fill=\"orange\" fillOpacity={0.4} color=\"blue\" stroke=\"red\" strokeWidth={3} strokeOpacity={0.3} opacity={0.7} transform=\"translate(7 3) rotate(12)\"",
                        "visibility=\"hidden\"",
                    ] {
                        let source = shapes(tag, geometry, attributes, parent, "<Dots/>");
                        let mut a = load(&source, false);
                        let mut b = load(&source, true);
                        let first = a.frame(Tick::ZERO).unwrap();
                        assert_eq!(first.template.instances.len(), 1, "{tag} {attributes}");
                        assert_eq!(first.template.nodes.len(), 4);
                        for t in ["0", "1.5", "0.25", "2.9"] {
                            let at = Tick::parse_seconds(t).unwrap();
                            let frame = a.frame(at).unwrap();
                            assert_eq!(
                                frame.items,
                                b.frame(at).unwrap().items,
                                "{tag} {attributes} {parent} at {t}"
                            );
                            assert!(Rc::ptr_eq(&first.layout, &frame.layout));
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn expanded_shapes_keep_constant_attributes() {
        run(|| {
            for (tag, geometry) in SHAPES {
                for wrapper in [
                    "<Dots/><View style={{opacity:bounds('dots').width/100}}/>",
                    "<TimeScope offset={0.2} rate={2}><Dots/></TimeScope>",
                    "<Echo count={2} delay={0.1} decay={0.5}><Dots/></Echo>",
                    "<Shutter samples={4} angle={180}><Dots/></Shutter>",
                    "<View style={{perspective:500}}><Dots/></View>",
                ] {
                    let source = shapes(
                        tag,
                        geometry,
                        "fill=\"orange\" stroke={i%2?'red':'blue'} strokeWidth={3} strokeDasharray=\"2 3\" pathLength={80} transform=\"translate(4 6)\"",
                        "",
                        wrapper,
                    );
                    let mut a = load(&source, false);
                    let mut b = load(&source, true);
                    for t in ["0", "1.5", "0.2"] {
                        let at = Tick::parse_seconds(t).unwrap();
                        assert_eq!(
                            a.frame(at).unwrap().items,
                            b.frame(at).unwrap().items,
                            "{tag} {wrapper}"
                        );
                    }
                }
                let source = shapes(tag, geometry, "fill=\"none\" stroke=\"red\" strokeWidth={3} strokeLinecap=\"round\" transform=\"translate(4 6)\"", "id=\"shapes\"", "<Dots/>")
                    .replace("<g id=", "<defs><g id=")
                    .replace("</g>", "</g></defs><use href=\"#shapes\"/>");
                let mut a = load(&source, false);
                let mut b = load(&source, true);
                let at = Tick::parse_seconds("1").unwrap();
                let frame = a.frame(at).unwrap();
                assert!(frame.template.instances.is_empty());
                assert_eq!(frame.items, b.frame(at).unwrap().items);
            }
        });
    }

    #[test]
    fn unsupported_shape_fields_keep_independent_nodes() {
        run(|| {
            for attributes in [
                "strokeWidth={1+i+t}",
                "strokeDashoffset={i+t}",
                "transform={`translate(${i+t} 2)`}",
                "style={{fill:'red'}}",
                "className=\"opacity-50\"",
            ] {
                let source = shapes(
                    "rect",
                    "x={20+i*25} width={20} height={20}",
                    attributes,
                    "stroke=\"blue\"",
                    "<Dots/>",
                );
                let mut a = load(&source, false);
                let mut b = load(&source, true);
                for t in ["0", "1.5", "0.2"] {
                    let at = Tick::parse_seconds(t).unwrap();
                    let frame = a.frame(at).unwrap();
                    assert!(frame.template.instances.is_empty(), "{attributes}");
                    assert_eq!(frame.items, b.frame(at).unwrap().items, "{attributes}");
                }
            }
        });
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
    fn invalid_shape_rows_are_checked_after_time_changes() {
        run(|| {
            for (tag, geometry, attributes) in [
                ("rect", "width={t<i ? 20 : 'bad'} height={20}", ""),
                (
                    "rect",
                    "width={20} height={20} rx={t<i ? 'auto' : 'bad'}",
                    "",
                ),
                ("line", "x1={20} y1={10} x2={t<i ? 40 : 'bad'} y2={30}", ""),
                (
                    "line",
                    "x1={20} x2={40} y2={30}",
                    "stroke={t<i ? 'red' : 'bad'}",
                ),
            ] {
                let source = shapes(tag, geometry, attributes, "", "<Dots/>")
                    .replace("[0,1,2,3]", "[1,2,3]");
                for ordinary in [false, true] {
                    let mut checker = load(&source, ordinary).into_checker();
                    checker.check(Tick::ZERO).unwrap();
                    assert!(
                        checker.check(Tick::parse_seconds("1").unwrap()).is_err(),
                        "{tag} {geometry} {attributes}"
                    );
                }
            }
        });
    }

    #[test]
    fn instance_dimensions_reject_negative_lengths_like_ordinary_shapes() {
        run(|| {
            for (tag, geometry, name) in [
                ("circle", "cx={40+i*25} cy={40} r={6-t*(i+1)*3}", "r"),
                (
                    "ellipse",
                    "cx={40+i*25} cy={40} rx={6-t*(i+1)*3} ry=\"auto\"",
                    "rx",
                ),
                (
                    "rect",
                    "x={40+i*25} width={6-t*(i+1)*3} height={20}",
                    "width",
                ),
                (
                    "rect",
                    "x={40+i*25} width={20} height={6-t*(i+1)*3}",
                    "height",
                ),
                (
                    "rect",
                    "x={40+i*25} width={20} height={20} rx=\"auto\" ry={6-t*(i+1)*3}",
                    "ry",
                ),
            ] {
                let source = shapes(tag, geometry, "fill=\"red\" opacity={0.8}", "", "<Dots/>");
                let mut a = load(&source, false);
                let mut b = load(&source, true);
                let first = a.frame(Tick::ZERO).unwrap();
                assert_eq!(first.template.instances.len(), 1);
                assert_eq!(first.items, b.frame(Tick::ZERO).unwrap().items);
                let at = Tick::parse_seconds("1").unwrap();
                for mut checker in [a.into_checker(), b.into_checker()] {
                    let error = checker.check(at).unwrap_err();
                    assert!(error.contains(&format!("{name} is negative")), "{error}");
                }
            }
        });
    }

    #[test]
    fn rectangle_and_line_rows_match_independent_pixels() {
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
            for (tag, geometry) in SHAPES {
                for attributes in [
                    "fill={i%2?'currentColor':'#39aacc99'} fillOpacity={0.5+0.2*Math.sin(t+i)} stroke={i%2?'none':'red'} strokeWidth={3} opacity={0.7}",
                    "fill=\"none\" stroke={i%2?'currentColor':'#39aacc99'} strokeWidth={4} strokeLinecap=\"round\" transform=\"translate(7 3) rotate(12)\"",
                    "fill=\"orange\" stroke=\"red\" strokeWidth={4} pathLength={40} strokeDasharray=\"2 3\" paintOrder=\"stroke fill\"",
                ] {
                    for wrapper in [
                        "<Dots/>",
                        "<View style={{opacity:0.6,filter:'blur(1px)',rotate:'8deg'}}><Dots/></View>",
                        "<Echo count={2} delay={0.1} decay={0.5}><Dots/></Echo>",
                        "<Shutter samples={4} angle={180}><Dots/></Shutter>",
                    ] {
                        let source = shapes(tag, geometry, attributes, "color=\"blue\"", wrapper);
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
                            assert_eq!(x, y, "{tag} {attributes} {wrapper} at {t}");
                        }
                    }
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
                "fill=\"orange\" stroke={t<1?'none':'red'} strokeWidth={3} strokeOpacity={0.6}",
                "fill=\"orange\" stroke={i%2?'none':'blue'} strokeWidth={3} transform=\"translate(7 3) rotate(12)\"",
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
