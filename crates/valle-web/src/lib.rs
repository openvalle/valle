//! The browser's engine: a scene, or a timeline lowered to one, drawn with WebGPU into an
//! `OffscreenCanvas`, in the SDK's worker. Files are read by path from what the worker has given;
//! a load or a frame that needs one it has not lists it, the worker fetches it and tries again.
//! Built in only for wasm32; elsewhere the crate is empty.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use valle_engine::audio::Mixer;
use valle_engine::front::{self, Inputs, controls};
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::mix;
use valle_engine::scene::Scene;
use valle_engine::text::{BUILT_IN, Blob, BuiltIn, Fonts};
use valle_engine::timeline::{self, Host};
use wasm_bindgen::prelude::*;
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

    /// What a failure asks for: the files it wanted, if any, which the worker fetches before it
    /// tries again.
    fn missing(&self) -> Option<Value> {
        let wanted = std::mem::take(&mut *self.wanted.borrow_mut());
        (!wanted.is_empty()).then(|| json!({ "status": "missing", "files": wanted }))
    }
}

/// A scene open, its renderer and the frame size.
struct Open {
    scene: Scene,
    renderer: Renderer<'static>,
    size: [u32; 2],
}

#[wasm_bindgen]
pub struct Engine {
    gpu: &'static Gpu,
    surface: wgpu::Surface<'static>,
    canvas: OffscreenCanvas,
    files: Files,
    open: Option<Open>,
    /// The sound of the timeline open, mixed, until it is taken.
    mixed: RefCell<Option<Mixer>>,
}

/// The sound of a timeline, which the page's audio worklet plays and its export encodes.
#[wasm_bindgen]
pub struct Sound {
    mixer: Mixer,
}

#[wasm_bindgen]
impl Sound {
    /// Its length in samples, at 48 000 a second.
    pub fn length(&self) -> f64 {
        self.mixer.length() as f64
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
        }
    }
}

/// The path the worker gives a file's sound at: interleaved stereo at 48 000 a second, 32-bit
/// floats, little-endian; absent when it has none.
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
            mixed: RefCell::new(None),
        })
    }

    /// Gives the file at `path`, or what a page put in place of a resource's.
    pub fn give(&self, path: String, bytes: Vec<u8>) {
        self.files.absent.borrow_mut().remove(&path);
        self.files.given.borrow_mut().insert(path, bytes.into());
    }

    /// Says there is no file at `path`: an import tries the extensions in turn.
    pub fn absent(&self, path: String) {
        self.files.absent.borrow_mut().insert(path);
    }

    /// Opens the scene or timeline at `entry`, its controls bound to `inputs`: JSON of `props`,
    /// `data`, and `resources`, each a list of paths given. Gives JSON: `status` `ok` with the
    /// composition and the controls, or `missing` with the files to give first.
    pub fn open(&mut self, entry: String, inputs: String) -> Result<String, JsValue> {
        let read = |p: &str| self.files.read(p);
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
        let attempt = || -> Result<(Scene, Value), String> {
            if entry.ends_with(".json") {
                let doc = valle_timeline::parse(&text(&entry).ok_or("not loaded yet")?)
                    .map_err(|e| format!("{entry}: {e}"))?;
                let length = |_: &str| None;
                let host = Host {
                    read: &text,
                    length: &length,
                };
                let lowered = timeline::lower(&entry, &doc, &host)?;
                let read = |p: &str| match lowered.files.get(p) {
                    Some(t) => Some(t.clone().into_bytes()),
                    None => read(p),
                };
                let scene =
                    Scene::load(&lowered.entry, &read, &lowered.inputs, self.files.fonts())?;
                return Ok((scene, json!({ "props": {}, "data": {}, "resources": {} })));
            }
            let scene = Scene::load(&entry, &read, &bound, self.files.fonts())?;
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
        let renderer = Renderer::new(self.gpu, size[0], size[1]);
        self.open = Some(Open {
            scene,
            renderer,
            size,
        });
        Ok(json!({
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
        .to_string())
    }

    /// Draws frame `index` into the canvas. Gives JSON: `status` `ok`, or `missing` with the
    /// files to give first.
    pub fn frame(&mut self, index: f64) -> Result<String, JsValue> {
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
        let target = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return Err(js("the canvas gave no texture to draw into")),
        };
        let scene = &open.scene;
        (open.renderer)
            .present(
                &frame.items,
                &frame.pictures,
                scene.faces(),
                scene.images(),
                &target.texture,
            )
            .map_err(js)?;
        self.gpu.queue().present(target);
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

    /// Mixes the sound of the timeline at `entry`. Gives JSON: `status` `ok`, then `take_sound`
    /// gives it; `none` where it has no sound, as a Motion scene; or `missing` with the files to
    /// give first.
    pub fn sound(&self, entry: String) -> Result<String, JsValue> {
        if !entry.ends_with(".json") {
            return Ok(json!({ "status": "none" }).to_string());
        }
        let text = |p: &str| (self.files.read(p)).and_then(|b| String::from_utf8(b).ok());
        let attempt = || -> Result<Option<Mixer>, String> {
            let doc = valle_timeline::parse(&text(&entry).ok_or("not loaded yet")?)
                .map_err(|e| format!("{entry}: {e}"))?;
            let (_, sources) = mix::mix(&entry, &doc)?;
            if sources.is_empty() {
                return Ok(None);
            }
            let files = self.files.clone();
            let sound = move |p: &str| -> Result<Option<Vec<f32>>, String> {
                let path = format!("{SOUND}{p}");
                if files.absent.borrow().contains(&path) {
                    return Ok(None);
                }
                let bytes = files.read(&path).ok_or("not decoded yet")?;
                let samples = bytes.as_chunks::<4>().0.iter();
                Ok(Some(samples.map(|b| f32::from_le_bytes(*b)).collect()))
            };
            let host = mix::Host {
                read: &text,
                sound: &sound,
            };
            mix::mixer(&entry, &doc, &host).map(Some)
        };
        let mixer = match attempt() {
            Ok(mixer) => mixer,
            Err(e) => {
                let missing = self.files.missing();
                return missing.map(|m| m.to_string()).ok_or_else(|| js(e));
            }
        };
        self.files.wanted.borrow_mut().clear();
        let status = if mixer.is_some() { "ok" } else { "none" };
        *self.mixed.borrow_mut() = mixer;
        Ok(json!({ "status": status }).to_string())
    }

    /// The sound `sound` mixed, once.
    pub fn take_sound(&self) -> Option<Sound> {
        self.mixed.take().map(|mixer| Sound { mixer })
    }
}

/// A promise, and the function that resolves it.
fn promise() -> (js_sys::Promise, js_sys::Function) {
    let mut resolve = None;
    let promise = js_sys::Promise::new(&mut |r, _| resolve = Some(r));
    (promise, resolve.expect("a promise's function runs at once"))
}
