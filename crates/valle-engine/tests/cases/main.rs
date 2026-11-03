//! The case runner: every scene under `tests/cases` is a case, and so is each scene the support
//! table makes (see `properties.rs`). Cases run in parallel.
//!
//! - A case in `errors/` names the error it must fail with on its first line, as `// error: …`;
//!   a `…` in it stands for any text.
//! - A case in `css/`, `svg/` or `tailwind/` is standard CSS, SVG or Tailwind: each frame is
//!   printed as HTML, and its boxes and lines must be where Chrome puts them, each box edge within
//!   1/32 px. The Chrome baselines are drawn here when missing or out of date (see `chrome.rs`),
//!   never on CI.
//! - The case `tailwind/classes` compares the CSS of every Tailwind class the engine expands with
//!   the Tailwind compiler's (see `tailwind.rs`), and `d3/checks` the built-in charts' results
//!   with d3's (see `d3.rs`).
//! - Any other case is drawn on the GPU, every frame, and each frame compared with its Valle
//!   baseline, `tests/baselines/valle/<case>.<frame>.png`, within one level a channel. Frames that
//!   differ or have no baseline go to `target/review` with a page to compare them, and
//!   `cargo xtask accept <case>` takes what was drawn as the new baselines.
//!
//! `cargo test -p valle-engine --test cases [-- <case>…]` runs the cases, or those under the
//! given paths (`errors`, `errors/style.tsx`). The GPU cases run only when `VALLE_TEST_LAYERS`
//! names `gpu`, as `cargo xtask test` does unless told to skip them.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use valle_engine::front::Inputs;
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::scene::Scene;
use valle_engine::text::{BUILT_IN, Blob, BuiltIn, Fonts};
use valle_engine::time::Tick;
use valle_native::urls::UrlCache;

mod chrome;
mod d3;
mod html;
mod properties;
mod tailwind;

/// The largest canvas, px a side, and the most frames a case may have.
const MAX_SIZE: u32 = 512;
const MAX_FRAMES: i64 = 10;
/// Scene code needs a stack of more than 12 MiB.
const STACK: usize = 32 << 20;

struct Runner {
    repo: PathBuf,
    root: PathBuf,
    /// The cases made from the support table, by path, and their source.
    made: BTreeMap<String, String>,
    baselines: PathBuf,
    chrome_baselines: PathBuf,
    /// The Chrome version the Chrome baselines are drawn by.
    chrome: Result<String, String>,
    /// The Tailwind version `package.json` pins.
    tailwind: Result<String, String>,
    /// On CI, which never draws baselines.
    ci: bool,
    review: PathBuf,
    gpu_layer: bool,
    gpu: OnceLock<Result<Gpu, String>>,
    /// Frames that went to review.
    reviewed: Mutex<Vec<Reviewed>>,
    /// The most any channel of a frame differed from its baseline.
    largest: AtomicU8,
}

/// A frame that went to review: its name, what was wrong, and when there was something to
/// compare it with, the captions of that and of the image marking where they differ.
type Reviewed = (String, String, Option<(&'static str, &'static str)>);

/// What a frame in review was compared with, and an image marking where they differ.
struct Against<'a> {
    image: &'a Path,
    marked: &'a [u8],
    captions: (&'static str, &'static str),
}

enum Outcome {
    Passed,
    Failed(String),
    Skipped,
    /// Chrome baselines to draw before the case can run.
    Stale(Vec<chrome::Job>),
}

/// Why a case's frames stopped: its scene failed, or the case breaks the runner's rules.
enum Failure {
    Scene(String),
    Case(String),
}

fn main() -> ExitCode {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest
        .ancestors()
        .nth(2)
        .expect("the crate is in crates/");
    let layers = std::env::var("VALLE_TEST_LAYERS").unwrap_or_default();
    let version = repo.join("tests/baselines/chrome/version");
    let mut runner = Runner {
        repo: repo.to_path_buf(),
        root: repo.join("tests/cases"),
        made: BTreeMap::new(),
        baselines: repo.join("tests/baselines/valle"),
        chrome_baselines: repo.join("tests/baselines/chrome"),
        chrome: std::fs::read_to_string(&version)
            .map(|v| v.trim().to_string())
            .map_err(|e| format!("{}: {e}", version.display())),
        tailwind: tailwind::version(repo),
        ci: std::env::var_os("CI").is_some(),
        review: repo.join("target/review"),
        gpu_layer: layers.split(',').any(|l| l == "gpu"),
        gpu: OnceLock::new(),
        reviewed: Mutex::new(Vec::new()),
        largest: AtomicU8::new(0),
    };
    // The built-in fonts, which Chrome's pages name, downloaded before the cases run.
    for font in &BUILT_IN {
        if let Err(e) =
            UrlCache::global().and_then(|c| c.fetch_pinned(font.url, font.sha256, font.file))
        {
            eprintln!("cases: the font {}: {e}", font.name);
            return ExitCode::FAILURE;
        }
    }
    let made = (runner.chrome_properties()).and_then(|known| properties::cases(&known));
    runner.made = match made {
        Ok(made) => made,
        Err(e) => {
            eprintln!("cases: {e}");
            return ExitCode::FAILURE;
        }
    };
    let filters: Vec<String> = std::env::args().skip(1).collect();
    let checks = [tailwind::CASE.to_string(), d3::CASE.to_string()];
    let cases = match select(&runner.root, runner.made.keys().chain(&checks), &filters) {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("cases: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Review holds only the last run's frames, which are what `accept` takes.
    let _ = std::fs::remove_dir_all(&runner.review);
    let code = runner.run_all(&cases);
    if let Err(e) = runner.write_review() {
        eprintln!("cases: review: {e}");
        return ExitCode::FAILURE;
    }
    code
}

/// The cases under `root` and those made, as `/`-separated paths from it, sorted; only those the
/// filters name when there are any.
fn select<'a>(
    root: &Path,
    made: impl Iterator<Item = &'a String>,
    filters: &[String],
) -> Result<Vec<String>, String> {
    if let Some(flag) = filters.iter().find(|f| f.starts_with('-')) {
        return Err(format!("unknown flag {flag}"));
    }
    let mut cases = Vec::new();
    walk(root, "", &mut cases)?;
    cases.extend(made.cloned());
    cases.sort();
    let named = |case: &str, filter: &str| {
        let f = filter.trim_end_matches('/');
        case == f || case.strip_suffix(".tsx") == Some(f) || case.starts_with(&format!("{f}/"))
    };
    for f in filters {
        if !cases.iter().any(|c| named(c, f)) {
            return Err(format!("no case is at {f}"));
        }
    }
    if !filters.is_empty() {
        cases.retain(|c| filters.iter().any(|f| named(c, f)));
    }
    Ok(cases)
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if path.is_dir() {
            walk(&path, &format!("{prefix}{name}/"), out)?;
        } else if name.ends_with(".tsx") {
            out.push(format!("{prefix}{name}"));
        }
    }
    Ok(())
}

impl Runner {
    fn run_all(&self, cases: &[String]) -> ExitCode {
        println!("\nrunning {} cases", cases.len());
        let start = Instant::now();
        let mut done = self.each(cases, &(0..cases.len()).collect::<Vec<_>>());
        // Cases whose Chrome baselines are out of date run again once they are drawn.
        let stale: Vec<usize> = (done.iter())
            .filter(|(.., o)| matches!(o, Outcome::Stale(_)))
            .map(|&(i, ..)| i)
            .collect();
        if !stale.is_empty() {
            let mut jobs = Vec::new();
            for (.., outcome) in &mut done {
                if let Outcome::Stale(j) = outcome {
                    jobs.append(j);
                }
            }
            done.retain(|(i, ..)| !stale.contains(i));
            match self.draw_chrome(&jobs) {
                Ok(()) => {
                    for (i, took, outcome) in self.each(cases, &stale) {
                        let outcome = match outcome {
                            Outcome::Stale(_) => {
                                println!("case {} ... FAILED", cases[i]);
                                Outcome::Failed("its Chrome baseline is out of date again".into())
                            }
                            outcome => outcome,
                        };
                        done.push((i, took, outcome));
                    }
                }
                Err(e) => {
                    for &i in &stale {
                        println!("case {} ... FAILED", cases[i]);
                        done.push((i, Duration::ZERO, Outcome::Failed(e.clone())));
                    }
                }
            }
        }
        done.sort_by_key(|&(i, ..)| i);
        let failed = (done.iter())
            .filter(|(.., o)| matches!(o, Outcome::Failed(_)))
            .count();
        let skipped = (done.iter())
            .filter(|(.., o)| matches!(o, Outcome::Skipped))
            .count();
        if failed > 0 {
            println!("\nfailures:");
            for (i, _, outcome) in &done {
                if let Outcome::Failed(why) = outcome {
                    println!("\n---- {}\n{why}", cases[*i]);
                }
            }
            if !self.reviewed().is_empty() {
                let page = self.review.join("index.html");
                println!("\ncompare the frames in {}", page.display());
            }
        }
        done.sort_by_key(|&(_, took, _)| std::cmp::Reverse(took));
        println!("\nslowest:");
        for (i, took, _) in done.iter().take(10) {
            println!("  {:>8.3} s  {}", took.as_secs_f64(), cases[*i]);
        }
        // Which GPU drew and how close it came: whether baselines hold on other GPUs.
        if let Some(Ok(gpu)) = self.gpu.get() {
            let largest = self.largest.load(Ordering::Relaxed);
            println!(
                "\ndrawn on {}, at most {largest} levels from the baselines",
                gpu.name()
            );
        }
        let skips = match skipped {
            0 => String::new(),
            n => format!("; {n} skipped, the gpu layer being off"),
        };
        println!(
            "\ncase result: {}. {} passed; {failed} failed{skips}; finished in {:.2}s\n",
            if failed == 0 { "ok" } else { "FAILED" },
            cases.len() - failed - skipped,
            start.elapsed().as_secs_f64()
        );
        if failed == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }

    /// Runs the cases at `todo`, in parallel, and reports each but those with baselines to draw.
    fn each(&self, cases: &[String], todo: &[usize]) -> Vec<(usize, Duration, Outcome)> {
        let next = AtomicUsize::new(0);
        let done: Mutex<Vec<(usize, Duration, Outcome)>> = Mutex::new(Vec::new());
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
        std::thread::scope(|s| {
            for _ in 0..workers.min(todo.len()) {
                std::thread::Builder::new()
                    .stack_size(STACK)
                    .spawn_scoped(s, || {
                        while let Some(&i) = todo.get(next.fetch_add(1, Ordering::Relaxed)) {
                            let case = &cases[i];
                            let began = Instant::now();
                            let outcome = self.run(case);
                            let took = began.elapsed();
                            let mut done = done.lock().unwrap_or_else(|e| e.into_inner());
                            match outcome {
                                Outcome::Passed => println!("case {case} ... ok"),
                                Outcome::Failed(_) => println!("case {case} ... FAILED"),
                                Outcome::Skipped | Outcome::Stale(_) => {}
                            }
                            done.push((i, took, outcome));
                        }
                    })
                    .expect("a worker thread");
            }
        });
        done.into_inner().unwrap_or_else(|e| e.into_inner())
    }

    fn run(&self, case: &str) -> Outcome {
        let checked = match case {
            tailwind::CASE => Some(tailwind::case(&self.repo, self.ci)),
            d3::CASE => Some(d3::case(&self.repo, self.ci)),
            _ => None,
        };
        if let Some(checked) = checked {
            return match checked {
                Ok(()) => Outcome::Passed,
                Err(e) => Outcome::Failed(e),
            };
        }
        let standard = ["css/", "svg/", "tailwind/"]
            .iter()
            .any(|d| case.starts_with(d));
        let drawn = !standard && !case.starts_with("errors/");
        if drawn && !self.gpu_layer {
            return Outcome::Skipped;
        }
        let run = std::panic::AssertUnwindSafe(|| {
            if standard {
                self.chrome(case)
            } else if drawn {
                self.pixels(case).map(|()| Vec::new())
            } else {
                self.error(case).map(|()| Vec::new())
            }
        });
        match std::panic::catch_unwind(run) {
            Ok(Ok(jobs)) if jobs.is_empty() => Outcome::Passed,
            Ok(Ok(jobs)) => Outcome::Stale(jobs),
            Ok(Err(why)) => Outcome::Failed(why),
            Err(panic) => {
                let why = (panic.downcast_ref::<String>().cloned())
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                Outcome::Failed(format!("panicked: {why}"))
            }
        }
    }

    /// A case's source, or that of a file it reads.
    fn source(&self, path: &str) -> Option<String> {
        match self.made.get(path) {
            Some(source) => Some(source.clone()),
            None => std::fs::read_to_string(self.root.join(path)).ok(),
        }
    }

    /// The bytes of a case or of a file it reads: from the web, only a built-in font's, from
    /// the cache.
    fn bytes(&self, path: &str) -> Option<Vec<u8>> {
        if let Some(source) = self.made.get(path) {
            return Some(source.clone().into_bytes());
        }
        if path.starts_with("https://") {
            let font = BUILT_IN.iter().find(|f| f.url == path)?;
            return std::fs::read(cached(font).ok()?).ok();
        }
        std::fs::read(self.root.join(path)).ok()
    }

    fn error(&self, case: &str) -> Result<(), String> {
        let source = self.source(case).ok_or("cannot read the case")?;
        let expected = (source.lines().next())
            .and_then(|l| l.strip_prefix("// error: "))
            .ok_or("an errors/ case names its error on its first line: // error: …")?;
        let failed = match self.frames(case, &mut |scene, _, at| scene.frame(at).map(drop)) {
            Ok(()) => None,
            Err(Failure::Scene(e)) => Some(e),
            Err(Failure::Case(e)) => return Err(e),
        };
        match failed {
            None => Err(format!("no error; expected: {expected}")),
            Some(e) if fits(&e, expected) => Ok(()),
            Some(e) => Err(format!("error:    {e}\nexpected: {expected}")),
        }
    }

    fn gpu(&self) -> Result<&Gpu, String> {
        self.gpu.get_or_init(Gpu::new).as_ref().map_err(|e| {
            format!("{e}; the gpu layer needs a GPU (`cargo xtask test --skip gpu` skips it)")
        })
    }

    fn pixels(&self, case: &str) -> Result<(), String> {
        let gpu = self.gpu()?;
        let stem = case.strip_suffix(".tsx").unwrap_or(case);
        let mut renderer = None;
        let mut problems = Vec::new();
        let mut review = Ok(());
        let drawn = self.frames(case, &mut |scene, i, at| {
            let c = scene.composition();
            let renderer = renderer.get_or_insert_with(|| Renderer::new(gpu, c.width, c.height));
            let items = scene.frame(at)?.items;
            let pixels = renderer.render(&items, scene.faces(), scene.images())?;
            match self.compare(&format!("{stem}.{i}.png"), [c.width, c.height], &pixels) {
                Ok(Some(problem)) => problems.push(format!("frame {i}: {problem}")),
                Ok(None) => {}
                Err(e) => review = Err(e),
            }
            Ok(())
        });
        match drawn {
            Ok(()) => review?,
            Err(Failure::Scene(e) | Failure::Case(e)) => return Err(e),
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("\n"))
        }
    }

    /// Compares the case's frames with their Chrome baselines, boxes and lines and on the GPU
    /// pixels, or gives the baselines to draw when any is missing or was drawn from other HTML or
    /// by another Chrome.
    fn chrome(&self, case: &str) -> Result<Vec<chrome::Job>, String> {
        let version = self.chrome.as_ref().map_err(String::clone)?;
        let gpu = if self.gpu_layer {
            Some(self.gpu()?)
        } else {
            None
        };
        let stem = case.strip_suffix(".tsx").unwrap_or(case);
        let mut renderer = None;
        let mut jobs = Vec::new();
        let mut problems = Vec::new();
        let drawn = self.frames(case, &mut |scene, i, at| {
            let frame = scene.frame(at)?;
            let c = scene.composition();
            let size = [c.width, c.height];
            let tailwind = self.tailwind.as_deref().map_err(|e| e.to_string())?;
            let fonts = scene.font_files();
            let html = html::print(&frame, c, scene.resources(), fonts, tailwind)?;
            let out = self.chrome_baselines.join(format!("{stem}.{i}"));
            let png = self.chrome_baselines.join(format!("{stem}.{i}.png"));
            let json = self.chrome_baselines.join(format!("{stem}.{i}.json"));
            match chrome::read::<chrome::Baseline>(&json)? {
                Some(b) if b.chrome == *version && b.html == html && png.exists() => {
                    let off = chrome::compare(&frame, &b);
                    problems.extend(off.into_iter().map(|p| format!("frame {i}: {p}")));
                    if let Some(gpu) = gpu {
                        let renderer =
                            renderer.get_or_insert_with(|| Renderer::new(gpu, size[0], size[1]));
                        let items = chrome::glyphless(&frame, scene.pictures());
                        let drawn = renderer.render(&items, scene.faces(), scene.images())?;
                        let name = format!("{stem}.{i}.png");
                        if let Some(p) = self.against_chrome(&name, size, &drawn, &png)? {
                            problems.push(format!("frame {i}: {p}"));
                        }
                    }
                }
                _ => jobs.push(chrome::Job {
                    page: page(&html)?,
                    html,
                    size,
                    out: out.to_string_lossy().into_owned(),
                }),
            }
            Ok(())
        });
        if let Err(Failure::Scene(e) | Failure::Case(e)) = drawn {
            return Err(e);
        }
        if !jobs.is_empty() {
            Ok(jobs)
        } else if problems.is_empty() {
            Ok(Vec::new())
        } else {
            Err(problems.join("\n"))
        }
    }

    /// Draws Chrome baselines with `web/tests/chrome.ts`, which CI never does.
    fn draw_chrome(&self, jobs: &[chrome::Job]) -> Result<(), String> {
        let version = self.chrome.as_ref().map_err(String::clone)?;
        if self.ci {
            return Err(format!(
                "the Chrome {version} baselines are missing or out of date, and CI never draws \
                 them: `cargo xtask test` draws them on a machine that can run Chrome"
            ));
        }
        let dir = self.repo.join("target/chrome");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let list = dir.join("jobs.json");
        let text = serde_json::to_string(jobs).map_err(|e| e.to_string())?;
        std::fs::write(&list, text).map_err(|e| format!("{}: {e}", list.display()))?;
        match jobs.len() {
            0 => println!("\nlisting the CSS properties Chrome knows"),
            n => println!("\ndrawing {n} Chrome baselines"),
        }
        bun(
            &self.repo,
            &["web/tests/chrome.ts".as_ref(), list.as_os_str()],
        )
    }

    /// The CSS properties the pinned Chrome knows, which `web/tests/chrome.ts` lists whenever it
    /// draws.
    fn chrome_properties(&self) -> Result<Vec<String>, String> {
        let version = self.chrome.as_ref().map_err(String::clone)?;
        let path = self.chrome_baselines.join("properties.json");
        let listed = || {
            let list = chrome::read::<chrome::Properties>(&path)?;
            let drawn = list.filter(|l| {
                l.chrome == *version
                    && !l.easings.is_empty()
                    && !l.formats.is_empty()
                    && !l.paths.is_empty()
            });
            Ok::<_, String>(drawn.map(|l| l.properties))
        };
        if let Some(known) = listed()? {
            return Ok(known);
        }
        self.draw_chrome(&[])?;
        listed()?.ok_or_else(|| format!("{}: Chrome {version} listed nothing", path.display()))
    }

    /// Runs `each` on the case's frames in order, with the scene, the frame and its time.
    fn frames(
        &self,
        case: &str,
        each: &mut dyn FnMut(&mut Scene, i64, Tick) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let read = |path: &str| self.bytes(path);
        let mut scene =
            Scene::load(case, &read, &Inputs::default(), fonts()).map_err(Failure::Scene)?;
        let c = scene.composition();
        if c.width > MAX_SIZE || c.height > MAX_SIZE {
            let e = format!("cases are at most {MAX_SIZE} × {MAX_SIZE} px");
            return Err(Failure::Case(e));
        }
        let n = c.fps.frames_in(c.duration);
        if n > MAX_FRAMES {
            let e = format!("cases have at most {MAX_FRAMES} frames, not {n}");
            return Err(Failure::Case(e));
        }
        for i in 0..n {
            let at = c.fps.frame_start(i).map_err(Failure::Case)?;
            each(&mut scene, i, at).map_err(Failure::Scene)?;
        }
        Ok(())
    }

    /// What is wrong with a drawn frame against its baseline, if anything; a frame that is wrong
    /// goes to review.
    fn compare(&self, name: &str, size: [u32; 2], drawn: &[u8]) -> Result<Option<String>, String> {
        let baseline = self.baselines.join(name);
        let (problem, diff) = if baseline.exists() {
            let (got, expected) = read_png(&baseline)?;
            if got != size {
                let [w, h] = got;
                let problem = format!("the baseline is {w} × {h} px, the frame is not");
                (Some(problem), None)
            } else {
                let mut diff = Vec::with_capacity(drawn.len());
                let (mut off, mut most) = (0, 0);
                for (a, b) in drawn.chunks(4).zip(expected.chunks(4)) {
                    let d = (a.iter().zip(b)).map(|(x, y)| x.abs_diff(*y)).max();
                    let d = d.unwrap_or(0);
                    most = most.max(d);
                    off += usize::from(d > 1);
                    diff.extend(if d > 1 { [255, 0, 0, 255] } else { [255; 4] });
                }
                self.largest.fetch_max(most, Ordering::Relaxed);
                let problem = (off > 0)
                    .then(|| format!("{off} pixels are off by more than 1 level, by up to {most}"));
                (problem, Some(diff))
            }
        } else {
            (Some(format!("no baseline {name}")), None)
        };
        let Some(problem) = problem else {
            return Ok(None);
        };
        let against = diff.as_ref().map(|marked| Against {
            image: &baseline,
            marked,
            captions: ("baseline", "off by more than 1 level"),
        });
        self.to_review(name, size, drawn, against, &problem)?;
        Ok(Some(problem))
    }

    /// What is wrong with a frame drawn without its glyphs against Chrome's, if anything; a frame
    /// that is wrong goes to review.
    fn against_chrome(
        &self,
        name: &str,
        size: [u32; 2],
        drawn: &[u8],
        png: &Path,
    ) -> Result<Option<String>, String> {
        let (got, theirs) = read_png(png)?;
        if got != size {
            let [w, h] = got;
            let problem = format!("Chrome's frame is {w} × {h} px, the frame is not");
            self.to_review(name, size, drawn, None, &problem)?;
            return Ok(Some(problem));
        }
        let (far, marked) = chrome::far(drawn, &theirs, size);
        // A thousandth of the pixels.
        let most = (size[0] * size[1]) as usize / 1000;
        if far <= most {
            return Ok(None);
        }
        let problem = format!(
            "{far} pixels are more than 8 levels outside what Chrome has around them; at most \
             {most} may be"
        );
        let against = Against {
            image: png,
            marked: &marked,
            captions: ("Chrome, without text", "far from Chrome"),
        };
        self.to_review(name, size, drawn, Some(against), &problem)?;
        Ok(Some(problem))
    }

    /// Puts a frame in review: what was drawn, and what it was compared with.
    fn to_review(
        &self,
        name: &str,
        size: [u32; 2],
        drawn: &[u8],
        against: Option<Against>,
        problem: &str,
    ) -> Result<(), String> {
        let stem = name.trim_end_matches(".png");
        write_png(&self.review.join(name), size, drawn)?;
        if let Some(a) = &against {
            write_png(
                &self.review.join(format!("{stem}.diff.png")),
                size,
                a.marked,
            )?;
            let to = self.review.join(format!("{stem}.expected.png"));
            std::fs::copy(a.image, &to).map_err(|e| format!("{}: {e}", a.image.display()))?;
        }
        let captions = against.map(|a| a.captions);
        let mut reviewed = self.reviewed.lock().unwrap_or_else(|e| e.into_inner());
        reviewed.push((name.to_string(), problem.to_string(), captions));
        Ok(())
    }

    fn reviewed(&self) -> Vec<Reviewed> {
        let mut list = (self.reviewed.lock().unwrap_or_else(|e| e.into_inner())).clone();
        list.sort();
        list
    }

    /// The page comparing the frames that went to review.
    fn write_review(&self) -> Result<(), String> {
        let reviewed = self.reviewed();
        if reviewed.is_empty() {
            return Ok(());
        }
        let mut html = String::from(
            "<!doctype html>\n<meta charset=\"utf-8\">\n<title>Valle review</title>\n<style>\n\
             body { font: 14px system-ui, sans-serif; margin: 24px; }\n\
             figure { display: inline-block; margin: 0 16px 16px 0; }\n\
             img { image-rendering: pixelated; background: repeating-conic-gradient(#ccc 0 25%, \
             #fff 0 50%) 0 0 / 16px 16px; }\n</style>\n<h1>Frames to review</h1>\n\
             <p>Accept the cases that look right with <code>cargo xtask accept &lt;case&gt;</code>; \
             a css or svg case that differs from Chrome is the engine's to fix.\n",
        );
        for (name, problem, against) in reviewed {
            let stem = name.trim_end_matches(".png");
            let figure = |file: String, caption: &str| {
                format!("<figure><img src=\"{file}\"><figcaption>{caption}</figcaption></figure>\n")
            };
            let _ = write!(html, "<h2>{name}</h2>\n<p>{problem}\n");
            if let Some((expected, _)) = against {
                html += &figure(format!("{stem}.expected.png"), expected);
            }
            html += &figure(name.clone(), "drawn");
            if let Some((_, marks)) = against {
                html += &figure(format!("{stem}.diff.png"), marks);
            }
        }
        let page = self.review.join("index.html");
        std::fs::write(&page, html).map_err(|e| format!("{}: {e}", page.display()))
    }
}

/// Runs a script with bun in the repository, after installing the locked packages.
fn bun(repo: &Path, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let script = args.first().map_or("".into(), |s| s.to_string_lossy());
    for args in [
        &["install", "--frozen-lockfile", "--silent"].map(std::ffi::OsStr::new)[..],
        args,
    ] {
        let status = Command::new("bun")
            .args(args)
            .current_dir(repo)
            .status()
            .map_err(|e| format!("cannot run bun, which {script} needs: {e}"))?;
        if !status.success() {
            let shown: Vec<_> = args.iter().map(|a| a.to_string_lossy()).collect();
            return Err(format!("`bun {}` failed ({status})", shown.join(" ")));
        }
    }
    Ok(())
}

/// Whether `error` fits `expected`, where each `…` stands for any text.
fn fits(error: &str, expected: &str) -> bool {
    let mut parts = expected.split('…');
    let Some(mut rest) = error.strip_prefix(parts.next().unwrap_or_default()) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

fn read_png(path: &Path) -> Result<([u32; 2], Vec<u8>), String> {
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| bad(&e))?;
    let mut reader = (png::Decoder::new(BufReader::new(file)).read_info()).map_err(|e| bad(&e))?;
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| bad(&"too large"))?
    ];
    let info = reader.next_frame(&mut pixels).map_err(|e| bad(&e))?;
    pixels.truncate(info.buffer_size());
    let size = [info.width, info.height];
    match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => Ok((size, pixels)),
        // Chrome leaves out the alpha of an opaque frame.
        (png::ColorType::Rgb, png::BitDepth::Eight) => {
            let rgba = pixels
                .chunks(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect();
            Ok((size, rgba))
        }
        _ => Err(bad(&"frames are 8-bit RGBA or RGB")),
    }
}

fn write_png(path: &Path, size: [u32; 2], pixels: &[u8]) -> Result<(), String> {
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| bad(&e))?;
    }
    let file = std::fs::File::create(path).map_err(|e| bad(&e))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| bad(&e))?;
    writer.write_image_data(pixels).map_err(|e| bad(&e))?;
    writer.finish().map_err(|e| bad(&e))
}

/// The file of built-in font `font` in the global cache.
fn cached(font: &BuiltIn) -> Result<PathBuf, String> {
    UrlCache::global()?.fetch_pinned(font.url, font.sha256, font.file)
}

/// The page Chrome loads for `html`: each built-in font it names by its file's name the file in
/// the cache, which the server `web/tests/chrome.ts` runs gives under `/file`.
fn page(html: &str) -> Result<String, String> {
    let mut page = html.to_string();
    for font in &BUILT_IN {
        let name = format!("url(\"{}\")", font.file);
        if page.contains(&name) {
            let path = cached(font)?;
            page = page.replace(&name, &format!("url(\"/file{}\")", path.display()));
        }
    }
    Ok(page)
}

/// The built-in fonts, from the global cache, each read once for all the cases.
fn fonts() -> Fonts {
    static GIVEN: Mutex<BTreeMap<&str, Blob<u8>>> = Mutex::new(BTreeMap::new());
    std::rc::Rc::new(|font: &BuiltIn| {
        let mut given = GIVEN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(blob) = given.get(font.sha256) {
            return Ok(blob.clone());
        }
        let path = cached(font)?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let blob = Blob::from(bytes);
        given.insert(font.sha256, blob.clone());
        Ok(blob)
    })
}
