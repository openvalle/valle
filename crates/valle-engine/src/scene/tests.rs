use indoc::indoc;

use super::*;
use crate::layout::Rect;
use crate::paint::{SQUARE, work};
use crate::style::color;
use crate::testing::fonts;

const SOURCE: &str = indoc! {"
    export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
    export default function Scene() {
      const { seconds, progress } = useCurrentTime();
      return (
        <View style={{ display: 'flex', padding: 10 }}>
          <View style={{ width: 100 * progress, height: 20, background: 'red' }} />
          {seconds < 1 ? <Text style={{ color: 'blue' }}>early</Text> : null}
        </View>
      );
    }
"};

/// Runs `f` on a thread with the stack scene code needs.
fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(32 << 20)
        .spawn(f)
        .expect("a thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn load(source: &'static str) -> Result<Scene, Error> {
    Scene::load(
        "s.tsx",
        &|path| (path == "s.tsx").then(|| source.as_bytes().to_vec()),
        &Inputs::default(),
        fonts(),
    )
}

#[test]
fn frames_paint_from_source() {
    big_stack(|| {
        let mut scene = load(SOURCE).unwrap();
        assert_eq!(scene.composition().width, 200);
        let at = |s: &str| Tick::parse_seconds(s).unwrap();
        let red = work(color::parse("red").unwrap());
        let items = scene.frame(at("0.5")).unwrap().items;
        let [first, Item::Glyphs { face: 0, color, .. }] = &items[..] else {
            panic!("{items:?}")
        };
        assert_eq!(*color, work(color::parse("blue").unwrap()));
        let rect = Rect {
            x: 10.0,
            y: 10.0,
            width: 25.0,
            height: 20.0,
        };
        assert_eq!(
            *first,
            Item::Rect {
                rect,
                radii: SQUARE,
                color: red
            }
        );
        assert_eq!(scene.faces().len(), 1);
        // A second variant, then the first again: its sheet is kept.
        let late = scene.frame(at("1.5")).unwrap().items;
        assert!(matches!(&late[..], [Item::Rect { rect, .. }] if rect.width == 75.0));
        scene.frame(at("0.25")).unwrap();
        assert_eq!(scene.sheets.len(), 2);
    });
}

#[test]
fn errors_point_at_the_source() {
    big_stack(|| {
        let mut scene = load(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                export default function Scene() {
                  return <View style={{ width: 'wide' }} />;
                }
            "})
        .unwrap();
        let e = scene.frame(Tick(0)).unwrap_err();
        assert!(e.starts_with("s.tsx:3:"), "{e}");
        assert!(e.contains("style width"), "{e}");
    });
}

/// A PNG of RGBA `pixels`, rows of `width`.
fn png(width: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
    let mut out = Vec::new();
    let height = pixels.len() as u32 / width;
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(pixels.as_flattened()).unwrap();
    writer.finish().unwrap();
    out
}

#[test]
fn images_come_from_the_resources_the_scene_declares() {
    big_stack(|| {
        let source = indoc! {"
            export const composition = { width: 40, height: 20, fps: 1, duration: 1 };
            export const controls = {
              resources: { dot: resource({ kind: 'image', default: '../img/dot.png' }) },
            };
            export default function Scene() {
              return <Image src='resource://dot' style={{ width: 10 }} />;
            }
        "};
        let image = png(2, &[[255, 0, 0, 255], [255, 255, 255, 128]]);
        let files = |source: String, image: Option<Vec<u8>>| {
            move |path: &str| match path {
                "scenes/s.tsx" => Some(source.clone().into_bytes()),
                "img/dot.png" => image.clone(),
                _ => None,
            }
        };
        let read = files(source.into(), Some(image.clone()));
        let mut scene = Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts()).unwrap();
        // The default from the scene's folder.
        assert_eq!(scene.resources()[0].default, ["img/dot.png"]);
        assert_eq!(scene.images()[0].size, [2, 1]);
        // Premultiplied in 8 bits, as Chrome keeps it.
        assert_eq!(scene.images()[0].pixels[4..], [128, 128, 128, 128]);
        let frame = scene.frame(Tick(0)).unwrap();
        // As tall as its width and the image's ratio make it.
        let border = frame.layout.boxes[0].border;
        assert_eq!([border.width, border.height], [10.0, 5.0]);
        assert!(matches!(&frame.items[..], [Item::Image { image: 0, .. }]));

        let read = files(source.replace("://dot", "://logo"), Some(image));
        let e = Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts())
            .and_then(|mut s| s.frame(Tick(0)).map(drop))
            .unwrap_err();
        assert!(e.starts_with("scenes/s.tsx:6:"), "{e}");
        assert!(e.contains("declares no resource logo"), "{e}");
        let read = files(source.into(), None);
        let e = Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts())
            .err()
            .unwrap();
        assert!(e.contains("resource dot: img/dot.png: not found"), "{e}");

        // A file bound in place of the default.
        let wide = png(3, &[[0, 0, 255, 255]; 3]);
        let read = move |path: &str| match path {
            "scenes/s.tsx" => Some(source.as_bytes().to_vec()),
            "other/wide.png" => Some(wide.clone()),
            _ => None,
        };
        let bind = |name: &str| Inputs {
            resources: [(name.to_string(), vec!["other/wide.png".to_string()])].into(),
            ..Inputs::default()
        };
        let scene = Scene::load("scenes/s.tsx", &read, &bind("dot"), fonts()).unwrap();
        assert_eq!(scene.images()[0].size, [3, 1]);
        let e = Scene::load("scenes/s.tsx", &read, &bind("logo"), fonts())
            .err()
            .unwrap();
        assert_eq!(
            e,
            "scenes/s.tsx: resources: the scene declares no resource logo in controls.resources"
        );

        // A file on the web, by its address, as the default or bound.
        let web = source.replace("../img/dot.png", "https://x.org/dot.png");
        let read = move |path: &str| match path {
            "scenes/s.tsx" => Some(web.clone().into_bytes()),
            "https://x.org/dot.png" => Some(png(2, &[[0, 0, 0, 255]; 2])),
            "https://x.org/wide.png" => Some(png(3, &[[0, 0, 0, 255]; 3])),
            _ => None,
        };
        let scene = Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts()).unwrap();
        assert_eq!(scene.resources()[0].default, ["https://x.org/dot.png"]);
        assert_eq!(scene.images()[0].size, [2, 1]);
        let bind = |location: &str| Inputs {
            resources: [("dot".to_string(), vec![location.to_string()])].into(),
            ..Inputs::default()
        };
        let scene = Scene::load(
            "scenes/s.tsx",
            &read,
            &bind("https://x.org/wide.png"),
            fonts(),
        )
        .unwrap();
        assert_eq!(scene.images()[0].size, [3, 1]);
        let e = Scene::load("scenes/s.tsx", &read, &bind("file:///wide.png"), fonts())
            .err()
            .unwrap();
        assert_eq!(
            e,
            "scenes/s.tsx: resource dot: file:///wide.png: write a path, or an http:// or \
             https:// address"
        );
    });
}

#[test]
fn background_and_mask_images_take_their_natural_size() {
    big_stack(|| {
        let source = indoc! {"
            export const composition = { width: 200, height: 40, fps: 1, duration: 1 };
            export const controls = {
              resources: { dot: resource({ kind: 'image', default: './dot.png' }) },
            };
            const image = 'url(resource://dot)';
            export default function Scene() {
              return (
                <View style={{ display: 'flex' }}>
                  <View style={{ width: 40, height: 30, backgroundImage: image }} />
                  <View style={{ width: 40, height: 30, backgroundImage: image, backgroundSize: 'auto 4px' }} />
                  <View style={{ width: 40, height: 30, backgroundImage: image, backgroundSize: 'contain' }} />
                  <View style={{ width: 40, height: 30, backgroundImage: image, backgroundSize: 'cover' }} />
                  <View style={{ width: 40, height: 30, backgroundColor: 'red', maskImage: image }} />
                </View>
              );
            }
        "};
        let image = png(2, &[[255, 0, 0, 255], [255, 255, 255, 128]]);
        let read = |path: &str| match path {
            "scenes/s.tsx" => Some(source.as_bytes().to_vec()),
            "scenes/dot.png" => Some(image.clone()),
            _ => None,
        };
        let frame = Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts())
            .unwrap()
            .frame(Tick(0))
            .unwrap();
        let size = |item: &Item| match item {
            Item::Image {
                tiles, image: 0, ..
            } => Some([tiles.first.width, tiles.first.height]),
            _ => None,
        };
        let sizes: Vec<_> = (frame.items.iter())
            .filter_map(|item| match item {
                Item::Mask { mask, .. } => mask.first().and_then(size),
                item => size(item),
            })
            .collect();
        // 2 × 1 as it is, 4 px tall, fitting the box and covering it, and as a mask.
        assert_eq!(
            sizes,
            [
                [2.0, 1.0],
                [8.0, 4.0],
                [40.0, 20.0],
                [60.0, 30.0],
                [2.0, 1.0]
            ]
        );
    });
}

#[test]
fn svg_files_show_in_images_only() {
    big_stack(|| {
        let source = indoc! {"
            export const composition = { width: 200, height: 40, fps: 1, duration: 1 };
            export const controls = {
              resources: {
                dot: resource({ kind: 'image', default: './dot.png' }),
                icon: resource({ kind: 'image', default: './icon.svg' }),
              },
            };
            export default function Scene() {
              return (
                <View style={{ display: 'flex', alignItems: 'flex-start' }}>
                  <Image src='resource://icon' style={{ width: 20 }} />
                  <Image src='resource://dot' />
                  <View style={{ width: 40, height: 30, backgroundImage: 'url(resource://dot)' }} />
                </View>
              );
            }
        "};
        let icon = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 2 1'><rect width='2' height='1'/></svg>";
        let image = png(2, &[[255, 0, 0, 255], [255, 255, 255, 128]]);
        let files = |source: String| {
            let image = image.clone();
            move |path: &str| match path {
                "scenes/s.tsx" => Some(source.clone().into_bytes()),
                "scenes/dot.png" => Some(image.clone()),
                "scenes/icon.svg" => Some(icon.as_bytes().to_vec()),
                _ => None,
            }
        };
        let mut scene = Scene::load(
            "scenes/s.tsx",
            &files(source.into()),
            &Inputs::default(),
            fonts(),
        )
        .unwrap();
        // The images display lists name are the files that are not SVG.
        assert_eq!(scene.images().len(), 1);
        assert!(matches!(
            scene.pictures()[0],
            Picture::Image { image: 0, .. }
        ));
        let frame = scene.frame(Tick(0)).unwrap();
        // As tall as its viewBox's ratio makes it.
        let border = frame.layout.boxes[1].border;
        assert_eq!([border.width, border.height], [20.0, 10.0]);
        assert!(matches!(
            &frame.items[..],
            [
                Item::Clip { .. },
                Item::Image { image: 0, .. },
                Item::Image { image: 0, .. }
            ]
        ));

        let source = source.replace("url(resource://dot)", "url(resource://icon)");
        let e = Scene::load(
            "scenes/s.tsx",
            &files(source.clone()),
            &Inputs::default(),
            fonts(),
        )
        .and_then(|mut s| s.frame(Tick(0)).map(drop))
        .unwrap_err();
        assert!(e.starts_with("scenes/s.tsx:13:"), "{e}");
        assert!(
            e.contains("backgroundImage: an SVG file shows in an <Image> only"),
            "{e}"
        );

        let broken = |path: &str| match path {
            "scenes/icon.svg" => {
                Some(b"<svg xmlns='http://www.w3.org/2000/svg'><text/></svg>".to_vec())
            }
            _ => files(source.clone())(path),
        };
        let e = Scene::load("scenes/s.tsx", &broken, &Inputs::default(), fonts())
            .err()
            .unwrap();
        assert_eq!(
            e,
            "scenes/s.tsx: resource icon: scenes/icon.svg: 1:41: <text> is not supported yet"
        );
    });
}

#[test]
fn auto_motion_blur_measures_the_trail_on_the_canvas() {
    big_stack(|| {
        // At 30 fps a 180° shutter is open a 60th of a second: 120 px a second moves 2 px. The
        // second box turns a quarter in a box turned back a quarter, and moves as fast twice
        // over: 4 px along its own y.
        let mut scene = load(indoc! {"
            export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
            const box = { position: 'absolute', width: 10, height: 10, motionBlur: 'auto' };
            function Turned() {
              const { seconds } = useCurrentTime();
              return <View style={{ ...box, rotate: '90deg', translate: `${120 * seconds}px` }} />;
            }
            export default function Scene() {
              const { seconds } = useCurrentTime();
              return (
                <View>
                  <View style={{ ...box, left: 120 * seconds }} />
                  <View style={{ position: 'absolute', rotate: '-90deg' }}>
                    <TimeScope rate={2}>
                      <Turned />
                    </TimeScope>
                  </View>
                  <View style={{ ...box, top: 50 }} />
                </View>
              );
            }
        "})
        .unwrap();
        let frame = scene.frame(Tick::parse_seconds("1").unwrap()).unwrap();
        let trail = |i: usize| match frame.styles[i].filter.last() {
            Some(filter::Function::MotionBlur(t)) => t.map(|v| (v * 1e3).round() / 1e3),
            other => panic!("{other:?}"),
        };
        assert_eq!(trail(1), [2.0, 0.0]);
        assert_eq!(trail(3), [0.0, -4.0]);
        assert_eq!(trail(4), [0.0, 0.0]);
        // Held at the start: only the half after the frame moves.
        let frame = scene.frame(Tick::ZERO).unwrap();
        let Some(filter::Function::MotionBlur(t)) = frame.styles[1].filter.last() else {
            panic!()
        };
        assert!((t[0] - 1.0).abs() < 1e-3, "{t:?}");
    });
}

#[test]
fn measure_text_gives_the_box_the_text_lays_out_in() {
    big_stack(|| {
        let mut scene = load(indoc! {"
            export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
            const one = measureText('Measure me', { fontSize: 20, fontWeight: 700 });
            const wrapped = measureText('one two three four', { fontSize: 16, maxWidth: 60 });
            export default function Scene() {
              return (
                <View>
                  <View style={{ width: one.width, height: one.height }} />
                  <Text style={{ position: 'absolute', fontSize: 20, fontWeight: 700 }}>
                    Measure me
                  </Text>
                  <View style={{ width: wrapped.width, height: wrapped.height }} />
                  <Text style={{ position: 'absolute', fontSize: 16, maxWidth: 60 }}>
                    one two three four
                  </Text>
                  <View style={{ width: wrapped.lines }} />
                </View>
              );
            }
        "})
        .unwrap();
        let frame = scene.frame(Tick::ZERO).unwrap();
        let size = |i: usize| {
            let b = frame.layout.boxes[i].border;
            [b.width, b.height]
        };
        assert_eq!(size(1), size(2));
        assert_eq!(size(3), size(4));
        assert!(size(3)[0] <= 60.0, "{:?}", size(3));
        let lines = frame.layout.lines[4].as_ref().unwrap().boxes.len();
        assert!(lines > 1);
        assert_eq!(size(5)[0], lines as f32);
        assert!(
            load(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                const m = measureText('x', { padding: { top: 1 } });
                export default function Scene() {
                  return <View />;
                }
            "})
            .and_then(|mut s| s.frame(Tick::ZERO).map(|_| ()))
            .is_err()
        );
    });
}

#[test]
fn text_outline_gives_the_glyphs_as_paths() {
    big_stack(|| {
        let mut scene = load(indoc! {"
            export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
            const style = { fontSize: 40, fontWeight: 700 };
            const m = measureText('Hi there', style);
            const left = textOutline('Hi there', { ...style, origin: { x: 100, y: 50 } });
            const center = textOutline('Hi there', { ...style, origin: { x: 100, y: 50 }, align: 'center' });
            const two = textOutline('a b\\nc', { fontSize: 20, whiteSpace: 'pre-line' });
            const g = left.glyphs;
            const b = left.bounds;
            export default function Scene() {
              return (
                <View>
                  <View style={{ width: g.length, height: g[1].word + 10 * g[2].word }} />
                  <View style={{ width: g[0].x, height: g[0].y }} />
                  <View style={{ width: center.glyphs[0].x + m.width / 2, height: center.bounds.width - b.width }} />
                  <View style={{ width: b.y, height: b.y + b.height }} />
                  <View style={{ width: two.glyphs.map((x) => x.line).join('') === '001' ? 1 : 0, height: two.glyphs[2].word }} />
                  <View style={{ width: left.path.startsWith('M') && left.path.includes(g[3].path) ? 1 : 0 }} />
                </View>
              );
            }
        "})
        .unwrap();
        let frame = scene.frame(Tick::ZERO).unwrap();
        let size = |i: usize| {
            let b = frame.layout.boxes[i].border;
            [b.width, b.height]
        };
        // "Hi there": 7 glyphs with ink, "i" of word 0, "t" of word 1.
        assert_eq!(size(1), [7.0, 10.0]);
        assert_eq!(size(2), [100.0, 50.0]);
        let [x, w] = size(3);
        assert!((x - 100.0).abs() < 0.01 && w.abs() < 0.01, "{:?}", size(3));
        // Ink above the baseline and below it.
        let [top, bottom] = size(4);
        assert!(top < 30.0 && bottom > 50.0, "{:?}", size(4));
        assert_eq!(size(5), [1.0, 2.0]);
        assert_eq!(size(6)[0], 1.0);
        for bad in [
            "textOutline('x', { align: 'middle' })",
            "textOutline('x', { origin: { x: '1' } })",
            "textOutline('x', { padding: { top: 1 } })",
        ] {
            let source = format!(
                "export const composition = {{ width: 200, height: 100, fps: 30, duration: 2 }};\n\
                 const o = {bad};\n\
                 export default function Scene() {{ return <View />; }}\n"
            );
            assert!(
                load(Box::leak(source.into_boxed_str()))
                    .and_then(|mut s| s.frame(Tick::ZERO).map(|_| ()))
                    .is_err(),
                "{bad}"
            );
        }
    });
}

#[test]
fn a_role_reads_the_pictures_the_timeline_gives_or_placeholders() {
    big_stack(|| {
        let source = indoc! {"
            export const composition = { width: 40, height: 20, fps: 30, duration: 1 };
            export const role = transition();
            export default function Scene() {
              return (
                <View style={{ display: 'flex' }}>
                  <Image src='input://from' style={{ width: 10, height: 10 }} />
                  <Image src='input://to' style={{ width: 10, height: 10 }} />
                </View>
              );
            }
        "};
        let image = png(2, &[[255, 0, 0, 255], [0, 0, 255, 255]]);
        let read = move |path: &str| match path {
            "s.tsx" => Some(source.as_bytes().to_vec()),
            "b.png" => Some(image.clone()),
            _ => None,
        };
        let mut inputs = Inputs::default();
        inputs.pictures.insert("to".into(), "b.png".into());
        let mut scene = Scene::load("s.tsx", &read, &inputs, fonts()).unwrap();
        // The placeholder is the composition's size; the bound file its own.
        let sizes: Vec<_> = (scene.pictures().iter())
            .map(|p| match p {
                Picture::Image { image, size } => (*image, *size),
                _ => panic!("{p:?}"),
            })
            .collect();
        assert_eq!(sizes, [(0, [40, 20]), (1, [2, 1])]);
        let frame = scene.frame(Tick(0)).unwrap();
        assert!(matches!(
            &frame.items[..],
            [Item::Image { image: 0, .. }, Item::Image { image: 1, .. }]
        ));

        inputs.pictures.insert("source".into(), "b.png".into());
        let e = Scene::load("s.tsx", &read, &inputs, fonts()).err().unwrap();
        assert!(
            e.starts_with("s.tsx: pictures: the scene reads no input://source"),
            "{e}"
        );
    });
}
