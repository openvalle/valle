use super::*;
use crate::layout::Rect;
use crate::paint::work;
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
        (path == "s.tsx").then(|| source.to_string())
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
                radius: 0.0,
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
