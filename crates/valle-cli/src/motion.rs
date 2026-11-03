//! `valle motion check` and `valle motion render`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use serde_json::{Value, json};
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::image::Image;
use valle_engine::scene::{Scene, Video};
use valle_engine::text::{Blob, BuiltIn, Fonts};
use valle_engine::time::Tick;
use valle_native::media::{self, Decoding};
use valle_native::urls::UrlCache;

use crate::args::{Args, Frames};
use crate::picture::{storyboard, write_png};
use crate::{Failure, video};

/// A scene loaded with what it read: the web addresses, each with the file it gave.
pub struct Loaded {
    pub scene: Scene,
    pub urls: Vec<(String, PathBuf)>,
}

pub fn load(args: &Args) -> Result<Loaded, Failure> {
    let lowered = match args.file.ends_with(".timeline.json") {
        true => Some(crate::timeline::lower(args)?),
        false => None,
    };
    let urls = RefCell::new(Vec::new());
    let cache = RefCell::new(None);
    let read = |path: &str| -> Option<Vec<u8>> {
        if let Some(text) = lowered.as_ref().and_then(|l| l.files.get(path)) {
            return Some(text.clone().into_bytes());
        }
        if path.starts_with("https://") || path.starts_with("http://") {
            let mut cache = cache.borrow_mut();
            if cache.is_none() {
                *cache = Some(UrlCache::global().ok()?);
            }
            let file = cache.as_ref()?.fetch(path).ok()?;
            let bytes = std::fs::read(&file).ok()?;
            urls.borrow_mut().push((path.to_string(), file));
            return Some(bytes);
        }
        std::fs::read(path).ok()
    };
    let (entry, inputs) = match &lowered {
        Some(l) => (l.entry.as_str(), &l.inputs),
        None => (args.file.as_str(), &args.inputs),
    };
    let mut scene =
        Scene::load(entry, &read, inputs, fonts()).map_err(|e| Failure::new("scene", e))?;
    let decoding = match args.option("decoder") {
        None => None,
        Some("vt" | "hardware") => Some(Decoding::Hardware),
        Some("ffmpeg" | "software") => Some(Decoding::Software),
        Some(other) => {
            return Err(Failure::new("usage", format!("no decoder {other}"))
                .hint("--decoder is vt (the hardware's) or ffmpeg (FFmpeg's own, on the CPU)"));
        }
    };
    // The resources of one file share its decoders.
    let mut pools: BTreeMap<PathBuf, Rc<RefCell<Pool>>> = BTreeMap::new();
    for (name, file) in scene.video_files(inputs) {
        let failed = |e: String| Failure::new("video", format!("video resource {name}: {e}"));
        let path = match file.starts_with("https://") || file.starts_with("http://") {
            true => (UrlCache::global().and_then(|c| c.fetch(&file))).map_err(failed)?,
            false => PathBuf::from(&file),
        };
        let pool = match pools.get(&path) {
            Some(p) => p.clone(),
            None => {
                let first = media::reader(&path, decoding).map_err(failed)?;
                let pool = Rc::new(RefCell::new(Pool {
                    path: path.clone(),
                    decoding,
                    size: first.size(),
                    readers: vec![(first, 0)],
                    uses: 0,
                }));
                pools.insert(path, pool.clone());
                pool
            }
        };
        scene
            .set_video(&name, Box::new(Pooled(pool)))
            .map_err(|e| Failure::new("scene", e))?;
    }
    Ok(Loaded {
        scene,
        urls: urls.into_inner(),
    })
}

/// The decoders of one video file, which every resource of the file shares: a frame goes to
/// the decoder already at or just before its time, else a new one, up to `DECODERS`, past which
/// the one used longest ago starts over there.
struct Pool {
    path: PathBuf,
    decoding: Option<Decoding>,
    size: [u32; 2],
    /// Each decoder, and when it was last used.
    readers: Vec<(Box<dyn media::Read>, u64)>,
    uses: u64,
}

/// Decoders a video file may have at once: clips of it a frame shows together, as a transition
/// between two cuts of it does, and one more.
const DECODERS: usize = 3;

/// A video resource the host decodes, as the scene reads it.
struct Pooled(Rc<RefCell<Pool>>);

impl Video for Pooled {
    fn size(&self) -> [u32; 2] {
        self.0.borrow().size
    }

    fn frame(&mut self, at: Tick) -> Result<Image, String> {
        let mut pool = self.0.borrow_mut();
        pool.uses += 1;
        let uses = pool.uses;
        let near = (pool.readers.iter()).position(|(r, _)| {
            r.position()
                .is_none_or(|p| p <= at.0 && at.0 - p <= media::AHEAD)
        });
        let k = match near {
            Some(k) => k,
            None if pool.readers.len() < DECODERS => {
                let reader = media::reader(&pool.path, pool.decoding)?;
                pool.readers.push((reader, 0));
                pool.readers.len() - 1
            }
            None => (0..pool.readers.len())
                .min_by_key(|&k| pool.readers[k].1)
                .expect("a decoder or more"),
        };
        pool.readers[k].1 = uses;
        let size = pool.size;
        let pixels = pool.readers[k].0.frame(at.0)?;
        Ok(Image {
            size,
            pixels: pixels.into(),
        })
    }
}

/// The built-in fonts, from the global cache, each read once.
pub fn fonts() -> Fonts {
    static GIVEN: Mutex<BTreeMap<&str, Blob<u8>>> = Mutex::new(BTreeMap::new());
    Rc::new(|font: &BuiltIn| {
        let mut given = GIVEN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(blob) = given.get(font.sha256) {
            return Ok(blob.clone());
        }
        let path = UrlCache::global()?.fetch_pinned(font.url, font.sha256, font.file)?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let blob = Blob::from(bytes);
        given.insert(font.sha256, blob.clone());
        Ok(blob)
    })
}

/// The frames `args` ask for, of a scene of `count` frames.
pub fn chosen(args: &Args, count: i64) -> Result<Vec<i64>, Failure> {
    let list = match &args.frames {
        None => (0..count).collect(),
        Some(Frames::Range(a, b)) => (*a..=*b).collect(),
        Some(Frames::List(list)) => list.clone(),
    };
    if let Some(f) = list.iter().find(|f| **f >= count) {
        return Err(Failure::new(
            "usage",
            format!("frame {f} is past the last, {}", count - 1),
        ));
    }
    Ok(list)
}

/// Every frame computed as it is drawn, but not drawn: each error once, with the frames it is
/// in.
pub fn check(args: &Args) -> Result<Value, Failure> {
    let Loaded { mut scene, urls } = load(args)?;
    let c = scene.composition();
    let count = c.fps.frames_in(c.duration);
    let mut problems: Vec<(String, Vec<i64>)> = Vec::new();
    for i in chosen(args, count)? {
        let at = c.fps.frame_start(i).map_err(|e| Failure::new("scene", e))?;
        if let Err(e) = scene.frame(at) {
            match problems.iter_mut().find(|(m, _)| *m == e) {
                Some((_, frames)) => frames.push(i),
                None => problems.push((e, vec![i])),
            }
        }
    }
    let urls: Vec<Value> = (urls.iter())
        .map(|(url, file)| json!({ "url": url, "sha256": sha256_of(file) }))
        .collect();
    let checked = json!({
        "frames": count,
        "width": c.width,
        "height": c.height,
        "fps": c.fps.to_string(),
        "duration": c.duration.seconds(),
        "urls": urls,
    });
    if problems.is_empty() {
        let mut out = json!({ "status": "ok", "summary": format!("{}, no problems", counted(count as usize, "frame")) });
        merge(&mut out, checked);
        return Ok(out);
    }
    let listed: Vec<Value> = (problems.iter())
        .map(|(m, frames)| json!({ "message": m, "first": frames[0], "count": frames.len() }))
        .collect();
    let mut f = Failure::new(
        "check",
        format!(
            "{} in {}",
            counted(problems.len(), "problem"),
            counted(count as usize, "frame")
        ),
    );
    f.problems = listed;
    Err(f.hint("fix the first frame of each, then check again"))
}

fn merge(out: &mut Value, more: Value) {
    if let (Value::Object(o), Value::Object(m)) = (out, more) {
        o.extend(m);
    }
}

/// The SHA-256 of a cached file, which names the folder it is in.
fn sha256_of(file: &Path) -> Option<String> {
    let dir = file.parent()?.file_name()?.to_str()?;
    Some(dir.to_string())
}

/// Frames drawn into a PNG, a folder of them, a storyboard, or a video.
pub fn render(args: &Args) -> Result<Value, Failure> {
    let out =
        (args.out.as_deref()).ok_or_else(|| Failure::new("usage", "render needs -o <out>"))?;
    let Loaded { mut scene, .. } = load(args)?;
    let c = scene.composition();
    let count = c.fps.frames_in(c.duration);
    let lower = out.to_ascii_lowercase();
    if lower.ends_with(".mp4") || lower.ends_with(".mov") {
        return video::render(args, &mut scene, Path::new(out));
    }
    let png = lower.ends_with(".png");
    let frames = match (png, args.storyboard, &args.frames) {
        (true, false, None) => vec![0],
        (true, false, Some(_)) => {
            let list = chosen(args, count)?;
            if list.len() != 1 {
                return Err(Failure::new("usage", "a .png shows one frame")
                    .hint("draw several into a folder, or add --storyboard"));
            }
            list
        }
        _ => chosen(args, count)?,
    };
    if args.storyboard && !png {
        return Err(Failure::new("usage", "a storyboard is a .png"));
    }
    let gpu = Gpu::new().map_err(|e| Failure::new("gpu", e))?;
    let mut renderer = Renderer::new(&gpu, c.width, c.height);
    let size = [c.width, c.height];
    let mut drawn = Vec::new();
    let mut written = Vec::new();
    for &i in &frames {
        let pixels = draw(&mut scene, &mut renderer, c.fps.frame_start(i))?;
        if args.storyboard {
            drawn.push(pixels);
        } else {
            let path = match png {
                true => PathBuf::from(out),
                false => Path::new(out).join(format!("frame-{i:05}.png")),
            };
            write_png(&path, size, &pixels)?;
            written.push(path.display().to_string());
        }
    }
    if args.storyboard {
        let (size, pixels) = storyboard(size, &drawn);
        write_png(Path::new(out), size, &pixels)?;
        written.push(out.to_string());
    }
    Ok(json!({
        "status": "ok",
        "summary": format!("{} drawn into {out}", counted(frames.len(), "frame")),
        "frames": frames,
        "files": written,
    }))
}

/// Frame `at` drawn: straight sRGB RGBA, rows top down.
pub fn draw(
    scene: &mut Scene,
    renderer: &mut Renderer,
    at: Result<Tick, String>,
) -> Result<Vec<u8>, Failure> {
    let at = at.map_err(|e| Failure::new("scene", e))?;
    let frame = scene.frame(at).map_err(|e| Failure::new("scene", e))?;
    renderer
        .render(&frame.items, &frame.pictures, scene.faces(), scene.images())
        .map_err(|e| Failure::new("gpu", e))
}

/// `n` things: `1 frame`, `2 frames`.
pub fn counted(n: usize, thing: &str) -> String {
    match n {
        1 => format!("1 {thing}"),
        n => format!("{n} {thing}s"),
    }
}
