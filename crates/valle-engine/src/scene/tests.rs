use super::*;
use crate::layout::Rect;
use crate::paint::{SQUARE, work};
use crate::style::color;

const SOURCE: &str = "export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
export default function Scene() {
  const { seconds, progress } = useCurrentTime();
  return (
    <View style={{ display: 'flex', padding: 10 }}>
      <View style={{ width: 100 * progress, height: 20, background: 'red' }} />
      {seconds < 1 ? <Text style={{ color: 'blue' }}>early</Text> : null}
    </View>
  );
}
";

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
    Scene::load("s.tsx", &|path| {
        (path == "s.tsx").then(|| source.as_bytes().to_vec())
    })
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
        let mut scene = load(
            "export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
export default function Scene() {
  return <View style={{ width: 'wide' }} />;
}
",
        )
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
        let source = "export const composition = { width: 40, height: 20, fps: 1, duration: 1 };
export const controls = {
  resources: { dot: resource({ kind: 'image', default: '../img/dot.png' }) },
};
export default function Scene() {
  return <Image src='resource://dot' style={{ width: 10 }} />;
}
";
        let image = png(2, &[[255, 0, 0, 255], [255, 255, 255, 128]]);
        let files = |source: String, image: Option<Vec<u8>>| {
            move |path: &str| match path {
                "scenes/s.tsx" => Some(source.clone().into_bytes()),
                "img/dot.png" => image.clone(),
                _ => None,
            }
        };
        let read = files(source.into(), Some(image.clone()));
        let mut scene = Scene::load("scenes/s.tsx", &read).unwrap();
        // The default from the scene's folder.
        assert_eq!(scene.resources()[0].default.as_deref(), Some("img/dot.png"));
        assert_eq!(scene.images()[0].size, [2, 1]);
        // Premultiplied in 8 bits, as Chrome keeps it.
        assert_eq!(scene.images()[0].pixels[4..], [128, 128, 128, 128]);
        let frame = scene.frame(Tick(0)).unwrap();
        // As tall as its width and the image's ratio make it.
        let border = frame.layout.boxes[0].border;
        assert_eq!([border.width, border.height], [10.0, 5.0]);
        assert!(matches!(&frame.items[..], [Item::Image { image: 0, .. }]));

        let read = files(source.replace("://dot", "://logo"), Some(image));
        let e = Scene::load("scenes/s.tsx", &read)
            .and_then(|mut s| s.frame(Tick(0)).map(drop))
            .unwrap_err();
        assert!(e.starts_with("scenes/s.tsx:6:"), "{e}");
        assert!(e.contains("declares no resource logo"), "{e}");
        let read = files(source.into(), None);
        let e = Scene::load("scenes/s.tsx", &read).err().unwrap();
        assert!(e.contains("resource dot: img/dot.png: not found"), "{e}");
    });
}
