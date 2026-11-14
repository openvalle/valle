//! Validation of a scene across time, without constructing a renderable frame. This shares
//! expression, style, resource and layout checks with rendering; it does not memoize JS calls.

use super::*;
use crate::template::Part;

mod interval;

/// Checks a loaded scene. Consuming the scene keeps resource bindings fixed for the lifetime
/// of its validation results. Rendering uses its own scene, not cached check results.
pub struct Checker {
    scene: Scene,
    plan: Option<Plan>,
}

/// Only the current variant is retained here; changing variants drops its successful values.
struct Plan {
    interval: bool,
    template: Rc<Template>,
    layout: bool,
    late: bool,
    inputs: Vec<u32>,
    checked: Option<Vec<V>>,
}

impl Plan {
    fn new(template: Rc<Template>) -> Self {
        let late = template
            .program
            .ops
            .iter()
            .any(|op| matches!(op, Op::Bounds(..)));
        // Layout's fallible work is text measurement, shaping and decoration. Boxes alone
        // need no layout for validation. A bounds() value does need their actual positions.
        let layout = late
            || template
                .nodes
                .iter()
                .any(|n| matches!(n.kind, Kind::Text { .. } | Kind::Svg { tag: Tag::Text, .. }));
        // Guard inputs have already been checked by Front. Only values consumed by nodes
        // can invalidate the remaining checks. Keep direct outputs, not all their operands:
        // changing seconds need not invalidate a width that still evaluates to 20px.
        let mut inputs = Vec::new();
        let mut prop = |p: &Prop| {
            if let Prop::Expr(e) = p {
                inputs.push(*e);
            }
        };
        for n in &template.nodes {
            for (_, p) in &n.style {
                prop(p);
            }
            match &n.kind {
                Kind::View | Kind::Mask { .. } => {}
                Kind::Text { content } | Kind::Span { content, .. } => {
                    for p in content {
                        if let Piece::Text(p) = p {
                            prop(p);
                        }
                    }
                }
                Kind::Image { video, .. } => {
                    if let Some(p) = video {
                        prop(p);
                    }
                }
                Kind::Svg {
                    attributes,
                    geometry,
                    ..
                } => {
                    for (_, p) in attributes {
                        prop(p);
                    }
                    for p in geometry.iter().flatten() {
                        if let Part::Expr(e) = p {
                            prop(&Prop::Expr(*e));
                        }
                    }
                }
                Kind::Shader { uniforms, .. } => {
                    for (_, parts) in uniforms {
                        for p in parts {
                            prop(p);
                        }
                    }
                }
            }
        }
        for (_, table) in &template.instances {
            table.rows.iter().flatten().for_each(&mut prop);
        }
        for (_, table) in &template.instances {
            if let Some(formula) = &table.formula {
                inputs.extend(&formula.imports);
            }
        }
        inputs.sort_unstable();
        inputs.dedup();
        Self {
            interval: interval::candidate(&template),
            template,
            layout,
            late,
            inputs,
            checked: None,
        }
    }
}

impl Checker {
    pub(super) fn new(scene: Scene) -> Self {
        Self { scene, plan: None }
    }

    /// The same tracing diagnostics as rendering, for checks of all or selected frames.
    pub fn traces(&self) -> (usize, Option<&str>) {
        self.scene.traces()
    }

    /// Check `first`, then prove as much of the inclusive output-frame interval as possible.
    /// Returns the last proven frame; unsupported work retains the exact sampled checker.
    pub fn check_frames(&mut self, first: i64, last: i64) -> Result<i64, Error> {
        let rate = self.scene.front.composition.fps;
        self.check(rate.frame_start(first)?)?;
        let plan = self.plan.as_mut().expect("checked frame has a plan");
        if first >= last || !plan.interval || plan.checked.is_none() {
            return Ok(first);
        }
        let _span = crate::perf::span("check.prove");
        let start = self.scene.front.clock(rate.frame_start(first)?);
        let proves = |end| -> Result<bool, Error> {
            let end = self.scene.front.clock(rate.frame_start(end)?);
            Ok(interval::proves(&plan.template, start, end))
        };
        if proves(last)? {
            return Ok(last);
        }
        if !proves(first + 1)? {
            // No useful interval even between neighbouring frames: avoid trying the same
            // unsupported proof at every following frame of this variant.
            plan.interval = false;
            return Ok(first);
        }
        let (mut low, mut high) = (first + 1, last);
        while high - low > 1 {
            let mid = low + (high - low) / 2;
            if proves(mid)? {
                low = mid;
            } else {
                high = mid;
            }
        }
        Ok(low)
    }

    /// Validate the frame at `at`, including sampled-time and video decode errors. No drawing
    /// commands, box pictures, final transforms or motion-blur velocities are produced.
    pub fn check(&mut self, at: Tick) -> Result<(), Error> {
        // Evaluate all operations, even unused fallible expressions. A successful first frame
        // is not evidence that a changing value or a later branch is valid.
        let frame = self.scene.front.frame(at)?;
        let template = frame.template;
        if self
            .plan
            .as_ref()
            .is_none_or(|p| !Rc::ptr_eq(&p.template, &template))
        {
            self.plan = Some(Plan::new(template.clone()));
        }
        let plan = self.plan.as_mut().expect("the current variant has a plan");
        if plan.checked.as_ref().is_some_and(|previous| {
            plan.inputs
                .iter()
                .zip(previous)
                .all(|(&e, v)| frame.values[e as usize] == *v)
        }) {
            return Ok(());
        }
        plan.checked = None;
        let needs_layout = plan.layout;
        let late = plan.late;
        let styles = self.scene.styled(&template, &frame.values)?;
        let sampled = styles.iter().any(|s| s.motion_auto);
        let (values, styles, layout, natural) = if needs_layout || sampled {
            let natural = self.scene.pictures.iter().map(Picture::natural).collect();
            let laid = self
                .scene
                .layout(at, template.clone(), frame.values, styles)?;
            if sampled {
                self.trails(at, &template, &laid.values, &laid.styles)?;
            }
            (laid.values, laid.styles, Some(laid.layout), natural)
        } else {
            // Do not leave a previous variant's layout alive when this one needs none.
            self.scene.retained = None;
            (frame.values, styles, None, Vec::new())
        };
        self.scene.text.sweep();
        self.scene.show_videos(&template, &values, &styles)?;
        backgrounds(&template, &styles, &self.scene.pictures)?;
        if let Some(layout) = layout {
            self.scene.retained = Some(RetainedLayout {
                text: text_values(&template, &values).cloned().collect(),
                template,
                styles: Rc::new(styles),
                natural,
                layout,
            });
        }
        // Equal values of the same variant have the same checks, with fixed resource bindings.
        // Subframe inputs can change while centre-frame values do not; never reuse those checks.
        // Late values have been settled above and are not comparable to the pre-layout values.
        if !sampled && !late {
            let plan = self.plan.as_mut().unwrap();
            plan.checked = Some(
                plan.inputs
                    .iter()
                    .map(|&e| values[e as usize].clone())
                    .collect(),
            );
        }
        Ok(())
    }

    /// Keep the sampled-style/layout errors that rendering's automatic motion blur can report,
    /// without calculating transforms, centres, inverse matrices or trail vectors.
    fn trails(
        &mut self,
        at: Tick,
        template: &Rc<Template>,
        values: &[V],
        styles: &[Style],
    ) -> Result<(), Error> {
        let c = self.scene.front.composition;
        let back = c.fps.shutter(360, 0, 2);
        let reach = self.scene.reach(template, styles);
        for offset in [back, -back] {
            let at = Sample {
                offset,
                duration: c.duration,
                looped: c.looped,
            }
            .at(at);
            if let Some(reach) = &reach {
                let then =
                    eval::run_only(&template.program, self.scene.front.clock(at), &reach.read);
                let Some(then) = then.ok().filter(|then| template.holds(then)) else {
                    continue;
                };
                if self
                    .scene
                    .restyled(template, &reach.nodes, &then, values, styles)?
                    .is_some()
                {
                    continue;
                }
            }
            let Some(then) = self.scene.front.values(template, at) else {
                continue;
            };
            self.scene.lay(at, template.clone(), then)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fonts;

    fn run(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    fn scene(body: &str) -> Scene {
        let source = format!(
            "export const composition = {{width:200,height:100,fps:30,duration:3}}; {body}"
        );
        Scene::load(
            "s.motion.tsx",
            &|p| (p == "s.motion.tsx").then(|| source.as_bytes().to_vec()),
            &Inputs::default(),
            fonts(),
        )
        .unwrap()
    }

    fn at(s: &str) -> Tick {
        Tick::parse_seconds(s).unwrap()
    }

    #[test]
    fn svg_map_rows_keep_unused_errors_and_later_invalid_geometry() {
        run(|| {
            // The invalid second row must still fail even though the interpolation is unused.
            let body = "export default function Scene(){return <svg>{[0,1].map(i=>{const unused=interpolate(0,[0,1-i],[0,1]);return <circle cx={i} cy={0} r={1}/>;})}</svg>;}";
            let error = scene(body).into_checker().check(at("0")).unwrap_err();
            assert!(error.contains("increase"), "{error}");
            let fallback = body.replace("map(i=>", "map((i,index,all)=>");
            assert!(error.starts_with("s.motion.tsx:1:"), "{error}");
            let ordinary = scene(&fallback).into_checker().check(at("0")).unwrap_err();
            assert_eq!(
                error.split("TypeError: ").nth(1).unwrap().lines().next(),
                ordinary.split("TypeError: ").nth(1).unwrap().lines().next()
            );

            let body = "export default function Scene(){const {seconds:t}=useCurrentTime();return <svg>{[0,1].map(i=><path d={`M0,0L${i},${1/(1-t*i)}`}/>)}</svg>;}";
            let mut checker = scene(body).into_checker();
            assert_eq!(checker.check_frames(0, 89).unwrap(), 0);
            checker.check(at("0.5")).unwrap();
            let error = checker.check(at("1")).unwrap_err();
            assert!(error.contains("Infinity"), "{error}");
            assert_eq!(error, scene(body).frame(at("1")).unwrap_err());
            checker.check(at("0")).unwrap();
        });
    }

    #[test]
    fn svg_templates_keep_standard_inputs_and_later_errors() {
        run(|| {
            for tag in ["polyline", "polygon"] {
                let body = format!(
                    "export default function Scene() {{ return <svg><{tag} points={{[1,2,3,4]}}/></svg>; }}"
                );
                let error = scene(&body).into_checker().check(at("0")).unwrap_err();
                assert!(
                    error.contains("points must be a number or a string"),
                    "{error}"
                );
            }
            for (tag, attribute, text) in [
                ("polyline", "points", "`0,0 10,${1/(1-t)}`"),
                ("path", "d", "`M0,0L10,${1/(1-t)}`"),
            ] {
                let body = format!(
                    "export default function Scene() {{ const {{seconds:t}}=useCurrentTime(); return <svg><{tag} {attribute}={{{text}}} stroke='red'/></svg>; }}"
                );
                let mut checker = scene(&body).into_checker();
                assert_eq!(checker.check_frames(0, 89).unwrap(), 0);
                checker.check(at("0.5")).unwrap();
                let error = checker.check(at("1")).unwrap_err();
                assert!(error.contains("Infinity"), "{error}");
                assert_eq!(error, scene(&body).frame(at("1")).unwrap_err());
                checker.check(at("0")).unwrap();
            }
        });
    }

    #[test]
    fn interval_checks_skip_safe_motion_and_stop_before_branch_changes() {
        run(|| {
            let mut smooth = scene("export default function Scene(){ const {seconds}=useCurrentTime(); return <View style={{width:10,height:10,opacity:0.5+0.5*Math.sin(seconds),translate:`${100*Math.sin(seconds)}px 0px`}}/>; }").into_checker();
            assert_eq!(smooth.check_frames(0, 89).unwrap(), 89);
            let mut branches = scene("export default function Scene(){ const {seconds}=useCurrentTime(); if(seconds<1) return <View style={{width:10}}/>; return <View style={{width:'broken'}}/>; }").into_checker();
            assert_eq!(branches.check_frames(0, 89).unwrap(), 29);
            assert!(branches.check_frames(30, 89).unwrap_err().contains("width"));
        });
    }

    #[test]
    fn shared_svg_maps_keep_interval_proofs_for_style_motion() {
        run(|| {
            let body = "export default function Scene(){const {seconds:t}=useCurrentTime();return <svg>{[0,20,40].map(i=><circle cx={i+10} cy={10} r={5} style={{opacity:0.5+0.5*Math.sin(t+i)}}/>)}</svg>}";
            for source in [
                body.to_string(),
                body.replace("map(i=>", "map((i,index,array)=>"),
            ] {
                let mut checker = scene(&source).into_checker();
                assert_eq!(checker.check_frames(0, 89).unwrap(), 89);
                assert_eq!(checker.traces().0, 1);
            }
        });
    }

    #[test]
    fn interval_and_sampled_checks_report_the_same_errors() {
        run(|| {
            for body in [
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <View style={{width:100/(1.5-t)}}/>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <View style={{opacity:t<1?'0.5':'broken'}}/>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <View style={{translate:`-${1-t}px 0px`}}/>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <View style={{translate:`${t}`}}/>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <View style={{transform:`translate(${t})`}}/>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();const unused=pathLength(t<1?'M0 0 L1 1':'bad');return <View/>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <Text style={{width:100-t*10}}>a changing line width</Text>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <View style={{width:Math.sqrt(1-t),height:10,motionBlur:'auto'}}/>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();if(Math.floor(t)%2)return <View style={{height:20}}/>;return <View style={{width:10}}/>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <View>{[1,2].map(p=><View style={{position:'absolute',left:p*10,top:0,width:8,height:8,opacity:1/(p-t)}}/>)}</View>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <View>{[1,2].map(p=><View style={{position:'absolute',left:p*10,top:0,width:8,height:8,translate:`-${p-t}px 0px`}}/>)}</View>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <svg>{[1,2].map(p=><circle cx={p*10} cy={10} r={5} style={{opacity:1/(p-t)}}/>)}</svg>}",
                "export default function Scene(){const {seconds:t}=useCurrentTime();return <svg>{[1,2].map(p=><circle cx={p*10} cy={10} r={5} style={{opacity:t<p?'0.5':'broken'}}/>)}</svg>}",
            ] {
                let mut plain = scene(body).into_checker();
                let mut ranged = scene(body).into_checker();
                let mut expected = Vec::new();
                let mut actual = Vec::new();
                let rate = plain.scene.front.composition.fps;
                for frame in 0..90 {
                    if let Err(e) = plain.check(rate.frame_start(frame).unwrap()) {
                        expected.push((frame, e));
                    }
                }
                let mut frame = 0;
                while frame < 90 {
                    match ranged.check_frames(frame, 89) {
                        Ok(end) => frame = end + 1,
                        Err(e) => {
                            actual.push((frame, e));
                            frame += 1;
                        }
                    }
                }
                assert_eq!(actual, expected, "{body}");
            }
        });
    }

    #[test]
    fn an_hour_of_provable_motion_needs_one_sample() {
        run(|| {
            let source = indoc::indoc! {r#"
                export const composition = {width:200,height:100,fps:30,duration:3600};
                export default function Scene() {
                    const {seconds}=useCurrentTime();
                    return <View style={{width:10,height:10,opacity:0.5+0.5*Math.sin(seconds)}}/>;
                }
            "#};
            let mut checker = Scene::load(
                "hour.motion.tsx",
                &|p| (p == "hour.motion.tsx").then(|| source.as_bytes().to_vec()),
                &Inputs::default(),
                fonts(),
            )
            .unwrap()
            .into_checker();
            assert_eq!(checker.check_frames(0, 107_999).unwrap(), 107_999);
            assert_eq!(checker.traces().0, 1);
        });
    }

    #[test]
    fn shared_map_keeps_interval_validation() {
        run(|| {
            let source = indoc::indoc! {r#"
                export const composition = {width:320,height:180,fps:30,duration:3600};
                const points = Array.from({length:100}, (_,i) => ({x:i*2,phase:i/10}));
                export default function Scene() {
                    const {seconds:t}=useCurrentTime();
                    return <View>{points.map(p=><View style={{position:'absolute',width:10,height:10,
                        left:p.x,top:0,translate:`${20*Math.sin(t+p.phase)}px 0px`}}/>)}</View>;
                }
            "#};
            let mut checker = Scene::load(
                "hour.motion.tsx",
                &|p| (p == "hour.motion.tsx").then(|| source.as_bytes().to_vec()),
                &Inputs::default(),
                fonts(),
            )
            .unwrap()
            .into_checker();
            assert_eq!(checker.check_frames(0, 107_999).unwrap(), 107_999);
            assert!(
                checker.plan.as_ref().unwrap().template.instances[0]
                    .1
                    .formula
                    .is_some()
            );
            assert_eq!(checker.traces().0, 1);
        });
    }

    #[test]
    fn errors_and_recovery_match_rendering_across_branches_and_seeks() {
        run(|| {
            for body in [
                "export default function Scene() { const {seconds}=useCurrentTime(); return <View style={{width: seconds < 1 ? '20px' : 'broken'}}/>; }",
                "export default function Scene() { const {seconds}=useCurrentTime(); return <View>{seconds < 1 ? null : <View style={{position:'absolute',width:'bad'}}/>}</View>; }",
                "export default function Scene() { const {seconds}=useCurrentTime(); return <Text style={{fontSize:12+seconds}}>text {seconds}</Text>; }",
                "export default function Scene() { const {seconds}=useCurrentTime(); const b=bounds('mark'); return <View><View key='mark' style={{width:10+seconds*10,height:10}}/><View style={{opacity:b.width/100}}/></View>; }",
                "export default function Scene() { const {seconds}=useCurrentTime(); return <Echo count={2} delay={0.1}><View style={{width:seconds < 1 ? 20 : 'bad'}}/></Echo>; }",
                "export default function Scene() { const {seconds}=useCurrentTime(); const unused=pathLength(seconds < 1 ? 'M0 0 L10 10' : 'bad'); return <View/>; }",
            ] {
                let mut drawn = scene(body);
                let mut checked = scene(body).into_checker();
                for t in ["0", "0.5", "1", "1.1", "2", "0.5", "2", "0"] {
                    assert_eq!(
                        checked.check(at(t)),
                        drawn.frame(at(t)).map(drop),
                        "{body} at {t}"
                    );
                }
            }
        });
    }

    #[test]
    fn boxes_skip_layout_but_text_and_bounds_keep_it() {
        run(|| {
            let mut fixed = scene("export default function Scene() { return <View>{Array.from({length:100}, (_,i)=><View key={i} style={{width:10,height:10,left:i*12}}/>)}</View>; }").into_checker();
            for t in ["0", "1", "2", "0"] {
                fixed.check(at(t)).unwrap();
                assert!(fixed.scene.retained.is_none());
                assert!(fixed.plan.as_ref().unwrap().checked.is_some());
            }
            for body in [
                "export default function Scene() { return <Text>Hello</Text>; }",
                "export default function Scene() { const b=bounds('mark'); return <View key='mark' style={{width:10,opacity:b.width/100}}/>; }",
            ] {
                let mut checker = scene(body).into_checker();
                checker.check(at("0")).unwrap();
                assert!(checker.scene.retained.is_some());
            }
        });
    }

    #[test]
    fn automatic_blur_checks_the_shutter_samples_without_drawing_them() {
        run(|| {
            let source = "export default function Scene() { const {seconds}=useCurrentTime(); return <View style={{width:10+Math.sqrt(0.00001-(seconds-1)*(seconds-1)), height:10, motionBlur:'auto'}}/>; }";
            let mut drawn = scene(source);
            let mut checker = scene(source).into_checker();
            let expected = drawn.frame(at("1")).map(drop);
            assert!(
                expected.is_err(),
                "the centre is valid, a shutter sample is not"
            );
            assert_eq!(checker.check(at("1")), expected);
        });
    }

    #[test]
    fn video_decode_errors_and_changing_dimensions_are_still_checked() {
        struct Clip {
            resize: bool,
        }
        impl Video for Clip {
            fn size(&self) -> [u32; 2] {
                [2, 2]
            }
            fn frame(&mut self, t: Tick) -> Result<Image, String> {
                if t >= at("1") && !self.resize {
                    return Err("broken video frame".into());
                }
                let size = if t >= at("1") { [3, 2] } else { [2, 2] };
                Ok(Image::new(
                    size,
                    vec![255; (size[0] * size[1] * 4) as usize].into(),
                ))
            }
        }
        run(|| {
            for resize in [false, true] {
                let load = || {
                    let mut s = scene(
                        "export const controls={resources:{clip:resource({kind:'video',default:'clip.mp4'})}}; export default function Scene() { return <Video src='resource://clip'/>; }",
                    );
                    s.set_video("clip", Box::new(Clip { resize })).unwrap();
                    s
                };
                let mut drawn = load();
                let mut checker = load().into_checker();
                for t in ["0", "0.5", "1", "0", "2"] {
                    let expected = drawn.frame(at(t)).map(drop);
                    assert_eq!(expected.is_err(), at(t) >= at("1"));
                    assert_eq!(checker.check(at(t)), expected);
                }
            }
        });
    }

    #[test]
    fn changing_guard_inputs_do_not_invalidate_fixed_node_checks() {
        run(|| {
            let mut checker = scene("export default function Scene() { const {seconds}=useCurrentTime(); if(seconds < 2) return <View style={{width:20}}/>; return <View style={{width:'bad'}}/>; }").into_checker();
            for t in ["0", "0.5", "1"] {
                checker.check(at(t)).unwrap();
                let plan = checker.plan.as_ref().unwrap();
                assert!(!plan.template.guards.is_empty());
                assert!(plan.inputs.is_empty());
                assert_eq!(plan.checked.as_deref(), Some([].as_slice()));
            }
            assert!(checker.check(at("2")).is_err());
            checker.check(at("0")).unwrap();
        });
    }
}
