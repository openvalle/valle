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
    let source =
        format!("export const composition = {{width:200,height:100,fps:30,duration:3}}; {body}");
    Scene::load(
        "s.tsx",
        &|p| (p == "s.tsx").then(|| source.as_bytes().to_vec()),
        &Inputs::default(),
        fonts(),
    )
    .unwrap()
}

fn at(s: &str) -> Tick {
    Tick::parse_seconds(s).unwrap()
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
            Ok(Image {
                size,
                pixels: vec![255; (size[0] * size[1] * 4) as usize].into(),
            })
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
