#![cfg(feature = "motion")]

//! Review hints about the clock: a one-shot scene that ends as it starts, a loop that jumps at
//! its seam, content shown on a single frame, and what switches inside a Shutter or jumps under
//! motion blur.

use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, MotionReviewIssue, Viewport, prepare_scene, resolve_props,
};
use valle_timeline::{ClipClock, FrameRate, RationalTime};

const SCENE: &str = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Clock() { const { seconds } = useCurrentTime();
  return <Scene style={{width:640,height:360}}>
    <View key="dot" className="absolute" visible={VISIBLE}
      style={{left:LEFT,top:100,width:40,height:40,backgroundColor:"#ffffff"}} />
  </Scene>;
}
"##;

fn scene(left: &str, visible: &str) -> String {
    SCENE.replace("LEFT", left).replace("VISIBLE", visible)
}

/// The hints of `code` in a review of two seconds at 30 fps.
fn hints(source: &str, looping: bool, code: &str) -> Vec<MotionReviewIssue> {
    let compiled = compile_motion(source).unwrap();
    let prepared = prepare_scene(&compiled.artifact).unwrap();
    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).unwrap();
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).unwrap();
    let review = valle_motion::review_motion(
        &prepared,
        &props,
        &LayoutOptions {
            fonts: &fonts,
            viewport: Viewport::new((640, 360)),
            styles: None,
        },
        ClipClock::standalone(
            FrameRate::new(30, 1).unwrap(),
            compiled.artifact.role,
            RationalTime::new(2, 1).unwrap(),
            looping,
            None,
        )
        .unwrap(),
        60,
        false,
        &compiled.shared_source_elements(),
    )
    .unwrap();
    review
        .issues
        .into_iter()
        .filter(|issue| issue.code == code)
        .collect()
}

#[test]
fn a_one_shot_scene_that_ends_as_it_starts_may_be_a_loop() {
    let back = scene("interpolate(seconds, [0, 1, 2], [0, 100, 0])", "true");
    let found = hints(&back, false, "loop_candidate");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].start_frame, found[0].end_frame), (0, 59));
    // As a loop its last frame is a step before the first, and the seam is as long as the
    // steps around it.
    assert!(hints(&back, true, "loop_candidate").is_empty());
    assert!(hints(&back, true, "loop_seam").is_empty());
    // A scene that ends elsewhere, or that starts and ends blank, is not a loop.
    assert!(hints(&scene("seconds * 50", "true"), false, "loop_candidate").is_empty());
    let blank = scene("seconds * 50", "seconds > 0.5 && seconds < 1.5");
    assert!(hints(&blank, false, "loop_candidate").is_empty());
}

#[test]
fn a_loop_that_jumps_at_its_seam_is_flagged() {
    let found = hints(&scene("seconds * 50", "true"), true, "loop_seam");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].node, "dot");
    assert_eq!(found[0].start_frame, 59);
    // 50 px a second is 1.67 px a frame; the seam steps back over 59 of them.
    let seam = found[0].peak_displacement_px_per_frame.unwrap();
    assert!((seam - 59.0 * 50.0 / 30.0).abs() < 0.5, "{seam}");
}

#[test]
fn content_on_a_single_frame_is_flagged() {
    let flash = scene("100", "seconds >= 1 && seconds < 1.02");
    let found = hints(&flash, false, "single_frame");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (
            found[0].node.as_str(),
            found[0].start_frame,
            found[0].end_frame
        ),
        ("dot", 30, 30)
    );
    assert!(
        hints(
            &scene("100", "seconds >= 1 && seconds < 1.05"),
            false,
            "single_frame"
        )
        .is_empty()
    );
    // The first and the last frame each show it once; in a loop they are two frames in a row.
    let ends = scene("100", "seconds < 0.02 || seconds >= 1.95");
    let found = hints(&ends, false, "single_frame");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].start_frame, found[0].end_frame), (0, 59));
    assert!(hints(&ends, true, "single_frame").is_empty());
}

#[test]
fn a_switch_inside_a_shutter_and_a_jump_under_motion_blur_are_flagged() {
    let shuttered = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Count() { const { seconds } = useCurrentTime();
  return <Scene style={{width:640,height:360}}>
    <Shutter key="shutter" samples={8} angle={180}>
      <Text key="count" style={{fontSize:40,color:"#ffffff"}}>{`${Math.floor(seconds * 2)}`}</Text>
    </Shutter>
  </Scene>;
}
"##;
    let found = hints(shuttered, false, "shutter_switch");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (found[0].node.as_str(), found[0].start_frame),
        ("count", 15)
    );
    let outside = shuttered
        .replace("<Shutter key=\"shutter\" samples={8} angle={180}>", "")
        .replace("</Shutter>", "");
    assert!(hints(&outside, false, "shutter_switch").is_empty());

    let jump = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Cut() { const { seconds } = useCurrentTime();
  return <Scene style={{width:640,height:360}}>
    <View key="bar" className="absolute" style={{left:40,top:120,width:40,height:120,
      backgroundColor:"#ffffff",translate:point(seconds < 0.5 ? 0 : 100,0),motionBlur:"auto"}}/>
  </Scene>;
}
"##;
    let found = hints(jump, false, "blur_jump");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].node.as_str(), found[0].start_frame), ("bar", 15));
    assert!(
        hints(
            &jump.replace(",motionBlur:\"auto\"", ""),
            false,
            "blur_jump"
        )
        .is_empty()
    );
    let smooth = jump.replace("seconds < 0.5 ? 0 : 100", "seconds * 100");
    assert!(hints(&smooth, false, "blur_jump").is_empty());
}

const CARD: &str = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Card() { const { seconds } = useCurrentTime();
  return <Scene style={{width:640,height:360}}>
    <Shutter samples={8} angle={180}>
      <View key="card" className="absolute" visible={VISIBLE}
        style={{left:LEFT,top:120,width:120,height:80,backgroundColor:"#ffffff"}} />
    </Shutter>
  </Scene>;
}
"##;

#[test]
fn moving_into_or_out_of_the_frame_inside_a_shutter_is_not_a_switch() {
    let card = |left: &str, visible: &str| CARD.replace("LEFT", left).replace("VISIBLE", visible);
    // The card slides in across the left edge, or out across the right one.
    for left in ["-200 + 300 * seconds", "400 + 300 * seconds"] {
        let found = hints(&card(left, "true"), false, "shutter_switch");
        assert!(found.is_empty(), "{left}: {found:?}");
    }
    // It jumps into the frame or out of it, or appears in it: that frame shows both states.
    for (left, visible) in [
        ("seconds < 1 ? -300 : 200", "true"),
        ("seconds < 1 ? 200 : 700", "true"),
        ("200", "seconds >= 1"),
    ] {
        let found = hints(&card(left, visible), false, "shutter_switch");
        assert_eq!(found.len(), 1, "{left}, {visible}: {found:?}");
        assert_eq!(
            (found[0].node.as_str(), found[0].start_frame),
            ("card", 30),
            "{left}, {visible}"
        );
    }
}
