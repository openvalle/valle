//! The engine in the SDK's worker: a scene, or a timeline lowered to one, drawn with WebGPU into
//! an `OffscreenCanvas`. Files are read by path from what the worker has given; a load or a frame
//! that needs one it has not lists it, the worker fetches it and tries again.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use valle_engine::audio::{Mixer, PagedPcm, RATE};
use valle_engine::front::{self, Front, Holds, Inputs, controls};
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::image::Image;
use valle_engine::mix::{self, Source};
use valle_engine::scene::{Scene, Video};
use valle_engine::text::{BUILT_IN, Blob, BuiltIn, Fonts};
use valle_engine::time::Tick;
use valle_engine::timeline::{self, Host};
use wasm_bindgen::prelude::*;

use crate::output::VideoOutput;
use crate::yuv;
use web_sys::OffscreenCanvas;

/// The error a page shows its fallback for: no WebGPU in this browser.
const NO_WEBGPU: &str = "no-webgpu";

fn js(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// The first error the GPU reported, which the next request fails with: WebGPU reports them
/// after the calls that make them.
static FAILED: Mutex<Option<String>> = Mutex::new(None);

fn failed(message: String) {
    web_sys::console::error_1(&message.clone().into());
    FAILED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert(message);
}

/// Fails with the error the GPU reported last, if any.
fn check() -> Result<(), JsValue> {
    match FAILED.lock().unwrap_or_else(|e| e.into_inner()).take() {
        Some(message) => Err(js(format!("GPU: {message}"))),
        None => Ok(()),
    }
}

/// QuickJS asks the host for the time: scene code never reads it, so it is always 0.
#[unsafe(no_mangle)]
pub extern "C" fn __rquickjs_host_now_us() -> f64 {
    0.0
}

/// The files given so far by path, those the worker found not to exist, and those asked for that
/// were neither.
#[derive(Clone, Default)]
struct Files {
    given: Rc<RefCell<HashMap<String, Rc<[u8]>>>>,
    absent: Rc<RefCell<HashSet<String>>>,
    wanted: Rc<RefCell<BTreeSet<String>>>,
}

impl Files {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let given = self.given.borrow().get(path).cloned();
        if given.is_none() && !self.absent.borrow().contains(path) {
            self.wanted.borrow_mut().insert(path.to_string());
        }
        given.map(|b| b.to_vec())
    }

    /// The built-in fonts, by their addresses.
    fn fonts(&self) -> Fonts {
        let files = self.clone();
        Rc::new(move |font: &BuiltIn| {
            let bytes = files.read(font.url).ok_or("not loaded yet")?;
            Ok(Blob::from(bytes))
        })
    }

    /// A video file's frame size and length, which the worker probes it for.
    fn video(&self, file: &str) -> Result<Probed, String> {
        let path = format!("{VIDEO_INFO}{file}");
        if self.absent.borrow().contains(&path) {
            return Err(format!("{file}: no video in it"));
        }
        let bytes = self.read(&path).ok_or("not probed yet")?;
        let probed: Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let size = |k: &str| probed[k].as_u64().and_then(|n| u32::try_from(n).ok());
        match (size("width"), size("height"), probed["duration"].as_f64()) {
            (Some(width), Some(height), Some(duration)) => Ok(Probed {
                width,
                height,
                duration,
            }),
            _ => Err(format!("engine bug: {file}: probed as {probed}")),
        }
    }

    /// Ask for all declared resources before decoding the first one. A known missing file
    /// is left to the scene loader so its error still names the resource and its owner.
    fn scene(
        &self,
        entry: &str,
        read: &dyn Fn(&str) -> Option<Vec<u8>>,
        inputs: &Inputs,
    ) -> Result<Scene, String> {
        let text = |p: &str| read(p).and_then(|b| String::from_utf8(b).ok());
        let front = Front::load(entry, &text, inputs)?;
        let mut waiting = false;
        for slot in &front.resources {
            for path in inputs.resources.get(&slot.name).unwrap_or(&slot.default) {
                waiting |= match slot.kind {
                    Holds::Video => self.request(&format!("{VIDEO_INFO}{path}")),
                    _ => self.request(path),
                };
            }
        }
        if waiting {
            return Err("resources not loaded yet".into());
        }
        let mut scene = Scene::from_front(front, entry, read, inputs, self.fonts())?;
        self.videos(&mut scene, inputs)?;
        Ok(scene)
    }

    /// Queues a path whose existence the worker has not resolved yet.
    fn request(&self, path: &str) -> bool {
        if self.given.borrow().contains_key(path) || self.absent.borrow().contains(path) {
            return false;
        }
        self.wanted.borrow_mut().insert(path.to_string());
        true
    }

    /// Gives the scene's video resources their decoders: the worker's.
    fn videos(&self, scene: &mut Scene, inputs: &Inputs) -> Result<(), String> {
        for (name, file) in scene.video_files(inputs) {
            let probed = self
                .video(&file)
                .map_err(|e| format!("video resource {name}: {e}"))?;
            let video = Frames {
                resource: name.clone(),
                file,
                size: [probed.width, probed.height],
                files: self.clone(),
            };
            scene.set_video(&name, Box::new(video))?;
        }
        Ok(())
    }

    /// What a failure asks for: the files it wanted, if any, which the worker fetches before it
    /// tries again.
    fn missing(&self) -> Option<Value> {
        let wanted = std::mem::take(&mut *self.wanted.borrow_mut());
        (!wanted.is_empty()).then(|| json!({ "status": "missing", "files": wanted }))
    }
}

/// Where the worker gives what it probes a video file for, as JSON, and the frames it decodes of
/// it for a resource: `<resource>@<seconds>/<file>`, a format header followed by decoded planes.
/// Each resource has a decoder of its own, as natively.
const VIDEO_INFO: &str = "valle:video-info/";
const VIDEO: &str = "valle:video/";

struct Probed {
    width: u32,
    height: u32,
    duration: f64,
}

/// A video file's frames as the worker decodes them, each when the scene first shows it.
struct Frames {
    resource: String,
    file: String,
    size: [u32; 2],
    files: Files,
}

impl Video for Frames {
    fn size(&self) -> [u32; 2] {
        self.size
    }

    fn frame(&mut self, at: Tick) -> Result<Image, String> {
        let path = format!("{VIDEO}{}@{}/{}", self.resource, at.seconds(), self.file);
        // Shown once: the scene keeps the image while it shows the frame.
        let Some(pixels) = self.files.given.borrow_mut().remove(&path) else {
            self.files.wanted.borrow_mut().insert(path);
            return Err("not decoded yet".into());
        };
        planes(&pixels, self.size)
    }
}

/// A frame as the worker gives it: little-endian u32s of its format (0 for straight RGBA, 1
/// I420, 2 NV12, 3 I420A), BT.709 or not, full range or not, its width and height, its count of
/// planes, clockwise quarter turns, horizontal reflection, and each plane's offset and bytes a
/// row, then the planes. Keep YUV for GPU conversion and orientation.
fn planes(given: &[u8], size: [u32; 2]) -> Result<Image, String> {
    let word = |i: usize| -> Result<u32, String> {
        let bytes = given
            .get(4 * i..4 * i + 4)
            .ok_or("engine bug: a frame's header is short")?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
    };
    let [kind, bt709, full, w, h, count] = [0, 1, 2, 3, 4, 5].map(word);
    let [w, h] = [w?, h?];
    let orientation = valle_engine::image::orientation::Orientation {
        turns: word(6)?,
        flip: word(7)? == 1,
    };
    if orientation.turns > 3 {
        return Err("engine bug: invalid video rotation".into());
    }
    if orientation.size([w, h]) != size {
        return Err(format!(
            "a frame of {w}×{h} px, not {}×{} px",
            size[0], size[1]
        ));
    }
    let count = count? as usize;
    if count > 4 {
        return Err("engine bug: a frame has too many planes".into());
    }
    let planes: Vec<(usize, usize)> = (0..count)
        .map(|i| Ok((word(8 + 2 * i)? as usize, word(9 + 2 * i)? as usize)))
        .collect::<Result<_, String>>()?;
    let data = given
        .get((8 + 2 * count) * 4..)
        .ok_or("engine bug: a frame's header is short")?;
    let format = match kind? {
        0 if data.len() == (w * h * 4) as usize => {
            let pixels = if orientation == Default::default() {
                data.into()
            } else {
                orientation.rgba(data.to_vec(), [w, h]).into()
            };
            return Ok(Image::new(size, pixels));
        }
        1 => yuv::Format::I420,
        2 => yuv::Format::Nv12,
        3 => yuv::Format::I420a,
        k => return Err(format!("engine bug: a frame of kind {k}")),
    };
    Image::from_yuv(
        [w, h],
        data,
        yuv::Layout {
            format,
            planes,
            bt709: bt709? == 1,
            full: full? == 1,
            orientation,
        },
    )
}

enum Target<'a> {
    Canvas,
    Pixels,
    Video(&'a VideoOutput),
}

/// A scene open, its renderer and the frame size.
struct Open {
    scene: Scene,
    renderer: Renderer<'static>,
    size: [u32; 2],
}

// A same-size edit reuses the renderer. A resized edit keeps the old renderer until presentation succeeds.
enum Previous {
    Scene(Box<Scene>),
    Open(Box<Open>),
}

#[wasm_bindgen]
pub struct Engine {
    gpu: &'static Gpu,
    surface: wgpu::Surface<'static>,
    canvas: OffscreenCanvas,
    files: Files,
    open: Option<Open>,
    pending: Option<Scene>,
    previous: Option<Previous>,
    /// The sound of the timeline open, mixed, until it is taken.
    mixed: RefCell<Option<Sound>>,
}

/// The sound of a timeline, which the page's audio worklet plays and its export encodes.
#[wasm_bindgen]
pub struct Sound {
    mixer: Mixer,
    sources: Vec<Source>,
}

#[wasm_bindgen]
impl Sound {
    /// Its length in samples, at 48 000 a second.
    pub fn length(&self) -> f64 {
        self.mixer.length() as f64
    }

    /// The pages of source PCM needed before rendering, including seek pre-roll.
    pub fn pages(&mut self, sample: Option<f64>, samples: usize) -> String {
        let requested = self.mixer.needed_pages(sample.map(|s| s as i64), samples);
        json!(
            self.mixer
                .missing_pages(&requested)
                .iter()
                .map(|&(source, page)| match &self.sources[source] {
                    Source::File(path) => json!([source, page, path, false]),
                    Source::Timeline(path) => json!([source, page, path, true]),
                })
                .collect::<Vec<_>>()
        )
        .to_string()
    }

    pub fn give_page(&mut self, source: usize, page: f64, pcm: Vec<f32>) {
        self.mixer.give_page(source, page as i64, pcm);
    }

    pub fn clear_pages(&mut self) {
        self.mixer.clear_pages();
    }

    /// Ready to give the samples from `sample` on.
    pub fn seek(&mut self, sample: f64) {
        self.mixer.seek(sample as i64);
    }

    /// The next `samples`, interleaved stereo.
    pub fn render(&mut self, samples: usize) -> Vec<f32> {
        let mut out = vec![0.0; 2 * samples];
        self.mixer.render(&mut out);
        out
    }

    /// Another of it, from the start, its sources shared.
    pub fn fresh(&self) -> Sound {
        Sound {
            mixer: self.mixer.fresh(),
            sources: self.sources.clone(),
        }
    }
}

/// The path the worker gives a file's audio track metadata at; absent when it has none.
const SOUND: &str = "valle:sound/";

/// The built-in fonts' files: name, file, address, SHA-256 and bytes, as JSON. A page that hosts
/// them itself maps these addresses to its own in its `fetch`.
#[wasm_bindgen(js_name = builtInFonts)]
pub fn built_in_fonts() -> String {
    let fonts: Vec<Value> = (BUILT_IN.iter())
        .map(|f| json!({ "name": f.name, "file": f.file, "url": f.url, "sha256": f.sha256, "size": f.size }))
        .collect();
    Value::from(fonts).to_string()
}

/// Converts straight RGBA to limited-range BT.709 I420.
#[wasm_bindgen]
pub fn i420(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, JsValue> {
    if rgba.len() != (width * height * 4) as usize {
        return Err(js(format!("{} bytes for {width}×{height} px", rgba.len())));
    }
    Ok(yuv::i420(rgba, [width, height]))
}

#[wasm_bindgen]
impl Engine {
    /// The engine on `canvas`'s GPU. Fails with `no-webgpu` where the browser has none.
    pub async fn start(canvas: OffscreenCanvas) -> Result<Engine, JsValue> {
        std::panic::set_hook(Box::new(|p| {
            web_sys::console::error_1(&p.to_string().into())
        }));
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::BROWSER_WEBGPU;
        let instance = wgpu::Instance::new(desc);
        let surface = (instance
            .create_surface(wgpu::SurfaceTarget::OffscreenCanvas(canvas.clone())))
        .map_err(|e| js(format!("{NO_WEBGPU}: {e}")))?;
        let adapter = (instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .await
        .map_err(|e| js(format!("{NO_WEBGPU}: {e}")))?;
        let gpu = Gpu::of(adapter)
            .await
            .map_err(|e| js(format!("{NO_WEBGPU}: {e}")))?;
        let device = gpu.device();
        device.on_uncaptured_error(Arc::new(|e| failed(e.to_string())));
        device.set_device_lost_callback(|why, message| {
            if why != wgpu::DeviceLostReason::Destroyed {
                failed(format!("the device was lost: {message}"));
            }
        });
        Ok(Engine {
            gpu: Box::leak(Box::new(gpu)),
            surface,
            canvas,
            files: Files::default(),
            open: None,
            pending: None,
            previous: None,
            mixed: RefCell::new(None),
        })
    }

    /// Gives the file at `path`, or what a page put in place of a resource's.
    pub fn give(&self, path: String, bytes: Vec<u8>) {
        self.files.absent.borrow_mut().remove(&path);
        self.files.given.borrow_mut().insert(path, bytes.into());
    }

    /// Forgets a supplied file and the metadata of its media tracks.
    pub fn invalidate(&self, path: String) {
        let mut given = self.files.given.borrow_mut();
        let mut absent = self.files.absent.borrow_mut();
        for key in [
            format!("{VIDEO_INFO}{path}"),
            format!("{SOUND}{path}"),
            path.clone(),
        ] {
            absent.remove(&key);
            given.remove(&key);
        }
        given.retain(|key, _| {
            let frame_file = key
                .strip_prefix(VIDEO)
                .and_then(|key| key.split_once('@'))
                .and_then(|(_, key)| key.split_once('/'))
                .map(|(_, file)| file);
            frame_file != Some(path.as_str())
        });
        self.files.wanted.borrow_mut().clear();
    }

    /// Says there is no file at `path`: an import tries the extensions in turn.
    pub fn absent(&self, path: String) {
        self.files.absent.borrow_mut().insert(path);
    }

    /// Opens the scene or timeline at `entry`, its controls bound to `inputs`: JSON of `props`,
    /// `data`, and `resources`, each a list of paths given. Gives JSON: `status` `ok` with the
    /// composition and the controls, or `missing` with the files to give first.
    pub fn open(&mut self, entry: String, inputs: String) -> Result<String, JsValue> {
        self.load_scene(entry, inputs, None)
    }

    /// Prepares a new timeline without replacing the scene on screen.
    pub fn prepare(
        &mut self,
        entry: String,
        inputs: String,
        timeline: String,
    ) -> Result<String, JsValue> {
        self.pending = None;
        self.load_scene(entry, inputs, Some(timeline))
    }

    fn load_scene(
        &mut self,
        entry: String,
        inputs: String,
        timeline: Option<String>,
    ) -> Result<String, JsValue> {
        let read = |p: &str| {
            if p == entry
                && let Some(timeline) = &timeline
            {
                return Some(timeline.as_bytes().to_vec());
            }
            self.files.read(p)
        };
        let text = |p: &str| read(p).and_then(|b| String::from_utf8(b).ok());
        let given: Value = serde_json::from_str(&inputs).map_err(js)?;
        let object = |k: &str| given[k].as_object().cloned().unwrap_or_default();
        let mut bound = Inputs {
            props: object("props"),
            data: object("data"),
            ..Inputs::default()
        };
        for (name, files) in object("resources") {
            let files = files.as_array().into_iter().flatten();
            let files = files
                .filter_map(|f| f.as_str().map(str::to_string))
                .collect();
            bound.resources.insert(name, files);
        }
        let kind = valle_engine::entry::Kind::read(&entry).map_err(js)?;
        let attempt = || -> Result<(Scene, Value), String> {
            if kind == valle_engine::entry::Kind::Timeline {
                let doc = valle_timeline::parse(&text(&entry).ok_or("not loaded yet")?)
                    .map_err(|e| format!("{entry}: {e}"))?;
                let length = |p: &str| {
                    let seconds = self.files.video(p).ok()?.duration;
                    Tick::from_seconds(seconds).ok()
                };
                let host = Host {
                    read: &text,
                    length: &length,
                };
                let lowered = timeline::lower(&entry, &doc, &host)?;
                let read = |p: &str| match lowered.files.get(p) {
                    Some(t) => Some(t.clone().into_bytes()),
                    None => read(p),
                };
                let scene = self.files.scene(&lowered.entry, &read, &lowered.inputs)?;
                return Ok((scene, json!({ "props": {}, "data": {}, "resources": {} })));
            }
            let scene = self.files.scene(&entry, &read, &bound)?;
            let declared = front::declared(&entry, &text)?;
            let described =
                controls::describe(&declared.props, &declared.data, &declared.resources);
            Ok((scene, described))
        };
        let (scene, controls) = match attempt() {
            Ok(done) => done,
            Err(e) => {
                return self
                    .files
                    .missing()
                    .map(|m| m.to_string())
                    .ok_or_else(|| js(e));
            }
        };
        self.files.wanted.borrow_mut().clear();
        let c = scene.composition();
        let info = json!({
            "status": "ok",
            "width": c.width,
            "height": c.height,
            "fps": c.fps.to_string(),
            "frameRate": c.fps.as_f64(),
            "frames": c.fps.frames_in(c.duration),
            "duration": c.duration.seconds(),
            "loop": c.looped,
            "controls": controls,
        })
        .to_string();
        if timeline.is_some() {
            self.pending = Some(scene);
        } else {
            // The canvas's size if the page set one, else the composition's.
            let size = match [self.canvas.width(), self.canvas.height()] {
                [0, _] | [_, 0] | [300, 150] => [c.width, c.height],
                given => given,
            };
            if size != [c.width, c.height] {
                return Err(js(format!(
                    "a canvas of {}×{} px for a {}×{} composition: drawing at another size is not \
                     supported yet",
                    size[0], size[1], c.width, c.height
                )));
            }
            self.install(scene)?;
        }
        Ok(info)
    }

    /// Keeps the current scene available until the candidate's first frame has been presented.
    pub fn begin_update(&mut self) -> Result<(), JsValue> {
        let scene = self
            .pending
            .take()
            .ok_or_else(|| js("no timeline prepared"))?;
        self.previous = self.install(scene)?;
        Ok(())
    }

    pub fn commit(&mut self) {
        self.previous = None;
    }

    pub fn discard(&mut self) {
        self.pending = None;
        self.mixed.take();
        self.files.wanted.borrow_mut().clear();
        match self.previous.take() {
            Some(Previous::Scene(scene)) => {
                let open = self.open.as_mut().expect("a replacement is installed");
                Self::replace_scene(open, *scene);
            }
            Some(Previous::Open(open)) => {
                self.configure(open.size);
                self.open = Some(*open);
            }
            None => {}
        }
    }

    fn replace_scene(open: &mut Open, scene: Scene) -> Scene {
        if open.scene.faces() != scene.faces() {
            open.renderer.reset_glyphs();
        }
        std::mem::replace(&mut open.scene, scene)
    }

    fn install(&mut self, scene: Scene) -> Result<Option<Previous>, JsValue> {
        let c = scene.composition();
        let size = [c.width, c.height];
        if let Some(open) = self.open.as_mut()
            && open.size == size
            && open.scene.composition().width == c.width
            && open.scene.composition().height == c.height
        {
            return Ok(Some(Previous::Scene(Box::new(Self::replace_scene(
                open, scene,
            )))));
        }
        let renderer = Renderer::at_size(self.gpu, size, size).map_err(js)?;
        self.configure(size);
        let previous = self.open.replace(Open {
            scene,
            renderer,
            size,
        });
        Ok(previous.map(|open| Previous::Open(Box::new(open))))
    }

    fn configure(&self, size: [u32; 2]) {
        self.canvas.set_width(size[0]);
        self.canvas.set_height(size[1]);
        self.surface.configure(
            self.gpu.device(),
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
                format: Gpu::FRAME,
                color_space: wgpu::SurfaceColorSpace::Srgb,
                width: size[0],
                height: size[1],
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: Vec::new(),
            },
        );
    }

    /// Draws frame `index` into the canvas. Gives JSON: `status` `ok`, or `missing` with the
    /// files to give first.
    pub fn frame(&mut self, index: f64) -> Result<String, JsValue> {
        self.draw(index, Target::Canvas)
    }

    /// Prepares the encoder's planes at delivery size without changing the layout canvas.
    pub fn video_output(&mut self, width: u32, height: u32) -> Result<VideoOutput, JsValue> {
        let open = self.open.as_mut().ok_or_else(|| js("nothing is open"))?;
        let size = [width, height];
        if size != open.size {
            let c = open.scene.composition();
            open.renderer = Renderer::at_size(self.gpu, [c.width, c.height], size).map_err(js)?;
            open.size = size;
        }
        Ok(VideoOutput::new(self.gpu.device(), size))
    }

    /// Draws a frame directly into the reusable WebCodecs input.
    pub fn video_frame(&mut self, index: f64, output: &VideoOutput) -> Result<String, JsValue> {
        self.draw(index, Target::Video(output))
    }

    /// Draws a still for `pixels` to read without presenting it.
    pub fn render(&mut self, index: f64) -> Result<String, JsValue> {
        self.draw(index, Target::Pixels)
    }

    fn draw(&mut self, index: f64, target: Target<'_>) -> Result<String, JsValue> {
        check()?;
        let open = self.open.as_mut().ok_or_else(|| js("nothing is open"))?;
        let c = open.scene.composition();
        let last = (c.fps.frames_in(c.duration) - 1).max(0);
        let at = c
            .fps
            .frame_start((index as i64).clamp(0, last))
            .map_err(js)?;
        let frame = match open.scene.frame(at) {
            Ok(frame) => frame,
            Err(e) => {
                return self
                    .files
                    .missing()
                    .map(|m| m.to_string())
                    .ok_or_else(|| js(e));
            }
        };
        let scene = &open.scene;
        let (items, pictures) = (&frame.items, &frame.pictures);
        match target {
            Target::Pixels => open
                .renderer
                .submit(items, pictures, scene.faces(), scene.images())
                .map_err(js)?,
            Target::Video(output) => {
                let mut encoder = open
                    .renderer
                    .encode_planes(
                        items,
                        pictures,
                        scene.faces(),
                        scene.images(),
                        output.views(),
                    )
                    .map_err(js)?;
                output.copy(&mut encoder);
                self.gpu.queue().submit([encoder.finish()]);
            }
            Target::Canvas => {
                let texture = match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                    _ => return Err(js("the canvas gave no texture to draw into")),
                };
                open.renderer
                    .present(
                        items,
                        pictures,
                        scene.faces(),
                        scene.images(),
                        &texture.texture,
                    )
                    .map_err(js)?;
                self.gpu.queue().present(texture);
            }
        }
        Ok(json!({ "status": "ok", "size": open.size }).to_string())
    }

    /// Resolves once the GPU has drawn the frames submitted so far.
    pub async fn drawn(&self) -> Result<(), JsValue> {
        let (promise, resolve) = promise();
        self.gpu.queue().on_submitted_work_done(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        wasm_bindgen_futures::JsFuture::from(promise).await?;
        check()
    }

    /// The pixels of the frame drawn last, as the canvas shows them but with their alpha: straight
    /// sRGB, 4 bytes a pixel, rows top down.
    pub async fn pixels(&mut self) -> Result<Vec<u8>, JsValue> {
        let open = self.open.as_mut().ok_or_else(|| js("nothing is open"))?;
        let (promise, resolve) = promise();
        open.renderer.read_back(move |r| {
            let _ = resolve.call1(&JsValue::NULL, &r.is_ok().into());
        });
        let mapped = wasm_bindgen_futures::JsFuture::from(promise).await?;
        check()?;
        if mapped.as_bool() != Some(true) {
            return Err(js("the GPU could not copy the frame back"));
        }
        let open = self.open.as_mut().ok_or_else(|| js("nothing is open"))?;
        open.renderer.pixels().map_err(js)
    }

    /// Lets the GPU go: the engine draws nothing after.
    pub fn destroy(&mut self) {
        self.open = None;
        self.gpu.device().destroy();
    }

    pub fn close(&mut self) {
        self.open = None;
        self.mixed.take();
    }

    /// Prepares the timeline's audio graph from track metadata. The worker decodes PCM on demand.
    pub fn sound(&self, entry: String, timeline: Option<String>) -> Result<String, JsValue> {
        if valle_engine::entry::Kind::read(&entry).map_err(js)? == valle_engine::entry::Kind::Motion
        {
            return Ok(json!({ "status": "none" }).to_string());
        }
        let text = |p: &str| {
            if p == entry
                && let Some(timeline) = &timeline
            {
                return Some(timeline.clone());
            }
            self.files.read(p).and_then(|b| String::from_utf8(b).ok())
        };
        let audio = |file: &str| -> Result<Option<PagedPcm>, String> {
            let path = format!("{SOUND}{file}");
            if self.files.absent.borrow().contains(&path) {
                return Ok(None);
            }
            let bytes = self
                .files
                .read(&path)
                .ok_or("audio metadata not loaded yet")?;
            let (frames, rate): (i64, u32) =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            Ok(Some(PagedPcm { frames, rate }))
        };
        let sound = match paged_sound(&entry, &text, &audio, &[], &mut HashMap::new()) {
            Ok(sound) => (!sound.sources.is_empty()).then_some(sound),
            Err(e) => {
                return self
                    .files
                    .missing()
                    .map(|m| m.to_string())
                    .ok_or_else(|| js(e));
            }
        };
        self.files.wanted.borrow_mut().clear();
        let status = if sound.is_some() { "ok" } else { "none" };
        *self.mixed.borrow_mut() = sound;
        Ok(json!({ "status": status }).to_string())
    }

    pub fn take_sound(&self) -> Option<Sound> {
        self.mixed.take()
    }
}

/// Validate nested mixes with the same sample bounds as the parent, leaving all PCM on demand.
fn paged_sound(
    entry: &str,
    text: &dyn Fn(&str) -> Option<String>,
    audio: &dyn Fn(&str) -> Result<Option<PagedPcm>, String>,
    ancestors: &[&str],
    durations: &mut HashMap<String, i64>,
) -> Result<Sound, String> {
    let doc = valle_timeline::parse(&text(entry).ok_or("not loaded yet")?)
        .map_err(|e| format!("{entry}: {e}"))?;
    let (mix, sources) = mix::mix(entry, &doc)?;
    let mut nesting = ancestors.to_vec();
    nesting.push(entry);
    // Collect every missing metadata request before the worker fetches the batch.
    let metadata: Vec<Result<Option<PagedPcm>, String>> = sources
        .iter()
        .map(|source| match source {
            Source::File(file) => audio(file),
            Source::Timeline(file) => {
                if nesting.contains(&file.as_str()) {
                    return Err(format!("{file} is nested in itself"));
                }
                let frames = if let Some(&frames) = durations.get(file) {
                    frames
                } else {
                    let child = paged_sound(file, text, audio, &nesting, durations)?;
                    let frames = child.mixer.length();
                    durations.insert(file.clone(), frames);
                    frames
                };
                Ok(Some(PagedPcm { frames, rate: RATE }))
            }
        })
        .collect();
    let mut pcm = Vec::with_capacity(metadata.len());
    for (index, result) in metadata.into_iter().enumerate() {
        let info = result?.unwrap_or(PagedPcm {
            frames: 0,
            rate: RATE,
        });
        if info.frames == 0
            && let Some(clip) = mix
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .find(|c| c.source == index && !c.optional)
        {
            return Err(format!("{}: its file has no sound", clip.what));
        }
        pcm.push(info);
    }
    Ok(Sound {
        mixer: Mixer::paged(mix, pcm)?,
        sources,
    })
}

/// A promise, and the function that resolves it.
fn promise() -> (js_sys::Promise, js_sys::Function) {
    let mut resolve = None;
    let promise = js_sys::Promise::new(&mut |r, _| resolve = Some(r));
    (promise, resolve.expect("a promise's function runs at once"))
}
