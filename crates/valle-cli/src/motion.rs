//! `valle motion check` and `valle motion render`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use serde_json::{Value, json};
use valle_engine::gpu::Renderer;
use valle_engine::image::Image;
use valle_engine::scene::{Scene, Video};
use valle_engine::text::{Blob, BuiltIn, Fonts};
use valle_engine::time::Tick;
use valle_native::media::{self, Decoding};
use valle_native::urls::UrlCache;

use crate::args::{Args, Frames};
use crate::picture::{Labels, Storyboard, page_path, write_png};
use crate::{Failure, video};

/// A scene loaded with what it read: the web addresses, each with the file it gave.
pub struct Loaded {
    pub scene: Scene,
    pub urls: Vec<(String, PathBuf)>,
    /// Video sources by local path, once each, with their authored names for diagnostics.
    videos: BTreeMap<PathBuf, String>,
}

pub fn load(args: &Args) -> Result<Loaded, Failure> {
    let lowered = match args.timeline {
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
            let file = {
                let _span = valle_engine::perf::span("load.network");
                cache.as_ref()?.fetch(path).ok()?
            };
            let bytes = std::fs::read(&file).ok()?;
            urls.borrow_mut().push((path.to_string(), file));
            return Some(bytes);
        }
        std::fs::read(path).ok()
    };
    // The engine's paths are /-separated, on Windows too, as the timeline commands give them.
    let file = args.file.replace('\\', "/");
    let (entry, inputs) = match &lowered {
        Some(l) => (l.entry.as_str(), &l.inputs),
        None => (file.as_str(), &args.inputs),
    };
    let mut inputs = inputs.clone();
    inputs.fps = args.fps.or(inputs.fps);
    let mut scene = {
        let _span = valle_engine::perf::span("load.scene");
        Scene::load(entry, &read, &inputs, fonts()).map_err(|e| Failure::new("scene", e))?
    };
    let decoding = args.decoder;
    // The resources of one file share its decoders.
    let mut pools: BTreeMap<PathBuf, Rc<RefCell<Pool>>> = BTreeMap::new();
    let mut videos = BTreeMap::new();
    for (name, file) in scene.video_files(&inputs) {
        let origin = inputs
            .resource_origins
            .get(&name)
            .cloned()
            .unwrap_or_else(|| format!("{}: resource {name}", args.file));
        let failed = |e: String| Failure::new("video", format!("{origin}: {e}"));
        if !file.starts_with("https://")
            && !file.starts_with("http://")
            && !Path::new(&file).is_file()
        {
            return Err(failed(format!("{file}: not found")));
        }
        let path = match file.starts_with("https://") || file.starts_with("http://") {
            true => (UrlCache::global().and_then(|c| c.fetch(&file))).map_err(failed)?,
            false => PathBuf::from(&file),
        };
        let pool = match pools.get(&path) {
            Some(p) => p.clone(),
            None => {
                let first = media::reader(&path, decoding).map_err(failed)?;
                videos.insert(path.clone(), file);
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
        videos,
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
        Ok(Image::new(size, pixels.into()))
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
        None if args.storyboard => match args.step {
            Some(step) => (0..count).step_by(step as usize).collect(),
            None => {
                let n = i64::from(args.samples.unwrap_or(12)).min(count);
                if n <= 1 {
                    (0..n).collect()
                } else {
                    // Integer arithmetic keeps the endpoints exact even for long timelines.
                    (0..n)
                        .map(|i| {
                            ((i128::from(i) * i128::from(count - 1) + i128::from((n - 1) / 2))
                                / i128::from(n - 1)) as i64
                        })
                        .collect()
                }
            }
        },
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

/// The same warning in Motion, Timeline and Project checks.
pub fn hdr_warning(file: &str, transfer: &str) -> Value {
    json!({
        "code": "unsupported-hdr",
        "file": file,
        "message": format!("{file}: HDR ({transfer}) video is not supported; it is rendered as SDR without tone mapping, so colors and brightness may be incorrect. Convert the source to SDR first."),
    })
}

/// Check every frame without building drawing commands: each error once, with its frames.
pub fn check(args: &Args) -> Result<Value, Failure> {
    if args.timeline {
        crate::timeline::check_audio(args)?;
    }
    let Loaded {
        scene,
        urls,
        videos,
    } = load(args)?;
    let mut warnings = Vec::new();
    for (path, file) in videos {
        let info = media::probe(&path).map_err(|e| Failure::new("video", e))?;
        if let Some(transfer) = info.hdr {
            warnings.push(hdr_warning(&file, transfer));
        }
    }
    let c = scene.composition();
    let count = c.fps.frames_in(c.duration);
    let mut checker = scene.into_checker();
    let mut problems: Vec<(String, Vec<i64>)> = Vec::new();
    // The frames traced again, by why.
    let mut retraced: Vec<(String, Vec<i64>)> = Vec::new();
    let frames = chosen(args, count)?;
    for &i in &frames {
        let at = c.fps.frame_start(i).map_err(|e| Failure::new("scene", e))?;
        let before = checker.traces().0;
        if let Err(e) = checker.check(at) {
            match problems.iter_mut().find(|(m, _)| *m == e) {
                Some((_, frames)) => frames.push(i),
                None => problems.push((e, vec![i])),
            }
        }
        if let (after, Some(why)) = checker.traces()
            && after > before
            && before > 0
        {
            match retraced.iter_mut().find(|(w, _)| w == why) {
                Some((_, frames)) => frames.push(i),
                None => retraced.push((why.to_string(), vec![i])),
            }
        }
    }
    warnings.extend(slow(&retraced, frames.len()));
    let urls: Vec<Value> = (urls.iter())
        .map(|(url, file)| json!({ "url": url, "sha256": sha256_of(file) }))
        .collect();
    let checked = json!({
        "warnings": warnings,
        "frames": count,
        "width": c.width,
        "height": c.height,
        "fps": c.fps.to_string(),
        "duration": c.duration.seconds(),
        "urls": urls,
    });
    if problems.is_empty() {
        let mut summary = format!("{}, no problems", counted(count as usize, "frame"));
        for w in &warnings {
            summary.push_str(&format!(
                "\nwarning: {}",
                w["message"].as_str().unwrap_or("")
            ));
        }
        let mut out = json!({ "status": "ok", "summary": summary });
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

/// A warning where more than a tenth of the `checked` frames traced the scene again, which is
/// slow: how many did, and the decisions that changed, the most frequent first.
fn slow(retraced: &[(String, Vec<i64>)], checked: usize) -> Vec<Value> {
    let total: usize = retraced.iter().map(|(_, f)| f.len()).sum();
    if total * 10 <= checked {
        return Vec::new();
    }
    let mut sites: Vec<_> = retraced.iter().collect();
    sites.sort_by_key(|(_, frames)| std::cmp::Reverse(frames.len()));
    let listed: Vec<String> = (sites.iter().take(3))
        .map(|(why, frames)| format!("{why} ({} frames, from {})", frames.len(), frames[0]))
        .collect();
    let message = format!(
        "{total} of {} trace the scene again, which is slow: the structure follows values that \
         change, {}",
        counted(checked, "frame"),
        listed.join("; ")
    );
    let frames: Vec<i64> = sites.iter().map(|(_, f)| f[0]).collect();
    vec![json!({ "message": message, "count": total, "first": frames })]
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
    args.validate_output()?;
    let lower = out.to_ascii_lowercase();
    let document = args.timeline;
    if lower.ends_with(".wav") {
        return match document {
            true => video::sound(&mut crate::timeline::mixer(args)?, Path::new(out)),
            false => Err(Failure::new("usage", "a Motion scene has no sound").hint(
                "a timeline's sound goes into a .wav; a scene's frames into images or video",
            )),
        };
    }
    let Loaded { mut scene, .. } = load(args)?;
    let c = scene.composition();
    let count = c.fps.frames_in(c.duration);
    if lower.ends_with(".mp4") || lower.ends_with(".mov") {
        let mut mixer = match document {
            true => Some(crate::timeline::mixer(args)?),
            false => None,
        };
        return video::render(args, &mut scene, Path::new(out), mixer.as_mut());
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
    let size = args.output_size([c.width, c.height])?;
    let (gpu, adapter) = crate::gpu()?;
    let mut renderer =
        Renderer::at_size(&gpu, [c.width, c.height], size).map_err(|e| Failure::new("gpu", e))?;
    let mut written = Vec::new();
    let mut pages = Vec::new();
    if args.storyboard {
        let labels = Labels::new(&gpu, fonts())?;
        let per_page = args.per_page.unwrap_or(12) as usize;
        let count = frames.len().div_ceil(per_page);
        for (page, indices) in frames.chunks(per_page).enumerate() {
            let mut board = Storyboard::new(size, indices, c.fps)?;
            for (k, &i) in indices.iter().enumerate() {
                let pixels = draw(&mut scene, &mut renderer, c.fps.frame_start(i))?;
                board.paint(k, &pixels);
                crate::progress(page * per_page + k + 1, frames.len());
            }
            let path = page_path(Path::new(out), page, count);
            board.label(&labels);
            write_png(&path, board.size, &board.pixels)?;
            let file = path.display().to_string();
            board.layout["file"] = file.clone().into();
            pages.push(board.layout);
            written.push(file);
        }
    } else {
        for (k, &i) in frames.iter().enumerate() {
            let pixels = draw(&mut scene, &mut renderer, c.fps.frame_start(i))?;
            let path = match png {
                true => PathBuf::from(out),
                false => Path::new(out).join(format!("frame-{i:05}.png")),
            };
            write_png(&path, size, &pixels)?;
            written.push(path.display().to_string());
            crate::progress(k + 1, frames.len());
        }
    }
    let destination = if pages.len() > 1 {
        format!("{} PNG files", pages.len())
    } else {
        out.to_string()
    };
    let mut result = json!({
        "status": "ok",
        "summary": format!("{} drawn into {destination}", counted(frames.len(), "frame")),
        "frames": frames,
        "fps": c.fps.to_string(),
        "files": written,
        "adapter": adapter,
    });
    if args.storyboard {
        result["storyboard"] = json!({"pages":pages});
    }
    Ok(result)
}

/// Frame `at` drawn: straight sRGB RGBA, rows top down.
pub fn draw(
    scene: &mut Scene,
    renderer: &mut Renderer,
    at: Result<Tick, String>,
) -> Result<Vec<u8>, Failure> {
    let at = at.map_err(|e| Failure::new("scene", e))?;
    let frame = {
        let _span = valle_engine::perf::span("eval");
        scene.frame(at).map_err(|e| Failure::new("scene", e))?
    };
    let _span = valle_engine::perf::span("draw");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(args: &Args, count: i64) -> Vec<i64> {
        chosen(args, count).unwrap_or_else(|e| panic!("{}", e.message))
    }

    #[test]
    fn storyboards_sample_endpoints_without_changing_other_frame_defaults() {
        let mut args = Args {
            storyboard: true,
            ..Args::default()
        };
        for count in [1, 6, 12, 195, 1800] {
            let picked = frames(&args, count);
            assert_eq!(picked.len(), count.min(12) as usize);
            assert_eq!(picked.first(), Some(&0));
            assert_eq!(picked.last(), Some(&(count - 1)));
            let gaps: Vec<_> = picked.windows(2).map(|p| p[1] - p[0]).collect();
            if let (Some(lo), Some(hi)) = (gaps.iter().min(), gaps.iter().max()) {
                assert!(*lo > 0 && hi - lo <= 1);
            }
        }
        args.samples = Some(1);
        assert_eq!(frames(&args, 195), [0]);
        args.samples = Some(60);
        assert_eq!(frames(&args, 3), [0, 1, 2]);
        args.samples = None;
        args.step = Some(30);
        assert_eq!(frames(&args, 195), [0, 30, 60, 90, 120, 150, 180]);
        args.step = Some(u32::MAX);
        assert_eq!(frames(&args, 195), [0]);
        args.step = None;
        args.frames = Some(Frames::List(vec![20, 0, 20]));
        assert_eq!(frames(&args, 195), [20, 0, 20]);
        args.frames = Some(Frames::Range(0, 14));
        assert_eq!(frames(&args, 195), (0..15).collect::<Vec<_>>());
        assert_eq!(frames(&Args::default(), 195), (0..195).collect::<Vec<_>>());
    }
}
