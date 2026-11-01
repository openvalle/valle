//! The GPU: display lists drawn with wgpu into a frame of f16 working colors (premultiplied
//! sRGB-encoded values, composited as browsers composite them), then out as 8-bit sRGB with
//! straight alpha.
//!
//! A frame is planned first: its rectangles, gradients, glyphs, paths and layers become instance
//! data, uploaded once, and steps, each a render pass that draws runs of instances into the frame
//! or a layer, or composites a layer onto what is under it: a group's at its opacity, a clip's
//! inside its rounded rectangle, a mask's through the alpha of the mask's layer. A group that
//! blends reads what is under it from a copy, as a pass cannot read its own target, and replaces
//! it with the two blended. Each instance names the transform from its px to the target's, which
//! its shaders invert to find their pixels in its px; the first is the identity. A layer takes a
//! texture from a pool when a step first draws into it and gives it back once composited. Glyph
//! outlines are encoded once and kept in two buffers, the atlas, for the renderer's life; paths
//! are encoded as they are, after the atlas, for their frame only.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};

use crate::layout::Rect;
use crate::paint::{Filter, Geometry, Item, Matrix, Radii, SQUARE};
use crate::path::Curve;
use crate::slug;
use crate::style::Blend;
use crate::style::gradient::Space;
use crate::template::Error;
use crate::text::Face;

const WORK: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUTPUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const COMMON: &str = include_str!("../shaders/common.wgsl");

/// u32 words per rectangle, gradient, glyph and layer instance, per gradient segment and per
/// transform; blur passes take a layer's words too.
const RECT: usize = 44;
const GRADIENT: usize = 32;
const GLYPH: usize = 20;
const LAYER: usize = 20;
const SEGMENT: usize = 12;
const TRANSFORM: usize = 24;

/// How far a path's quadratic curves may be from its cubic ones and arcs, in target px.
const TOLERANCE: f32 = 0.02;

/// Valle's effects, as `effects.wgsl` tells them apart.
const ABERRATION: f32 = 0.0;
const LENS: f32 = 1.0;
const GRAIN: f32 = 2.0;
const RADIAL: f32 = 3.0;

/// Premultiplied colors to the luminance of their working values, Rec. 709's weights as Skia's,
/// times their alpha, as the alpha of black.
const LUMINANCE: [f32; 20] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2126, 0.7152, 0.0722, 0.0, 0.0,
    0.0, 0.0, 0.0,
];

/// The result of a request wgpu completes at once, as it does on native backends.
fn now<T>(request: impl Future<Output = T>) -> Result<T, Error> {
    let mut request = std::pin::pin!(request);
    match request
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(v) => Ok(v),
        Poll::Pending => Err("engine bug: a wgpu request did not complete at once".into()),
    }
}

/// A device and the pipelines every renderer draws with. Renderers on several threads may share
/// one.
pub struct Gpu {
    info: wgpu::AdapterInfo,
    device: wgpu::Device,
    queue: wgpu::Queue,
    draw: wgpu::BindGroupLayout,
    image: wgpu::BindGroupLayout,
    rects: wgpu::RenderPipeline,
    gradients: wgpu::RenderPipeline,
    glyphs: wgpu::RenderPipeline,
    layers: wgpu::RenderPipeline,
    blends: wgpu::RenderPipeline,
    masks: wgpu::RenderPipeline,
    blurs: wgpu::RenderPipeline,
    colors: wgpu::RenderPipeline,
    effects: wgpu::RenderPipeline,
    output: wgpu::RenderPipeline,
}

impl Gpu {
    /// The default adapter; `WGPU_BACKEND` and `WGPU_ADAPTER_NAME` choose another.
    pub fn new() -> Result<Gpu, Error> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = now(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))?
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let (device, queue) = now(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("valle"),
            // What every WebGPU device has, so what runs here runs in a browser too.
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))?
        .map_err(|e| format!("no GPU device: {e}"))?;

        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty,
            count: None,
        };
        let buffer = |ty| wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let storage = buffer(wgpu::BufferBindingType::Storage { read_only: true });
        let draw = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draw"),
            entries: &[
                entry(0, buffer(wgpu::BufferBindingType::Uniform)),
                entry(1, storage),
                entry(2, storage),
                entry(3, storage),
                entry(4, storage),
                entry(5, storage),
                entry(6, storage),
                entry(7, storage),
                entry(8, storage),
            ],
        });
        let image = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image"),
            entries: &[entry(
                0,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            )],
        });
        let layout = |groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        let drawing = layout(&[Some(&draw)]);
        let imaging = layout(&[Some(&draw), Some(&image)]);
        let blending = layout(&[Some(&draw), Some(&image), Some(&image)]);
        let over = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let pipeline = |layout: &wgpu::PipelineLayout, name: &str, source: &str, format, blend| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl([COMMON, source].concat().into()),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let rect = include_str!("../shaders/rect.wgsl");
        let gradient = include_str!("../shaders/gradient.wgsl");
        let glyph = include_str!("../shaders/slug.wgsl");
        let layer = include_str!("../shaders/layer.wgsl");
        let blend = include_str!("../shaders/blend.wgsl");
        let mask = include_str!("../shaders/mask.wgsl");
        let blur = include_str!("../shaders/blur.wgsl");
        let colors = include_str!("../shaders/colors.wgsl");
        let effects = include_str!("../shaders/effects.wgsl");
        let output = include_str!("../shaders/output.wgsl");
        Ok(Gpu {
            info: adapter.get_info(),
            rects: pipeline(&drawing, "rect", rect, WORK, over),
            gradients: pipeline(&drawing, "gradient", gradient, WORK, over),
            glyphs: pipeline(&drawing, "slug", glyph, WORK, over),
            layers: pipeline(&imaging, "layer", layer, WORK, over),
            blends: pipeline(&blending, "blend", blend, WORK, None),
            masks: pipeline(&blending, "mask", mask, WORK, over),
            blurs: pipeline(&imaging, "blur", blur, WORK, None),
            colors: pipeline(&imaging, "colors", colors, WORK, None),
            effects: pipeline(&imaging, "effects", effects, WORK, None),
            output: pipeline(&imaging, "output", output, OUTPUT, None),
            device,
            queue,
            draw,
            image,
        })
    }

    /// The adapter's name and backend.
    pub fn name(&self) -> String {
        format!("{} ({:?})", self.info.name, self.info.backend)
    }
}

/// Where an outline is in the atlas.
#[derive(Clone, Copy)]
struct Placed {
    em: f32,
    bounds: [f32; 4],
    band_scale: [f32; 2],
    bands: [u32; 2],
    /// The first curve texel and the first list word.
    at: [u32; 2],
}

/// A storage buffer that grows: to the next power of two, keeping what it holds when `keep`.
struct Store {
    buffer: wgpu::Buffer,
    label: &'static str,
}

impl Store {
    fn new(device: &wgpu::Device, label: &'static str) -> Store {
        Store {
            buffer: storage(device, label, 4096),
            label,
        }
    }

    /// Makes room for `bytes`; true when the buffer changed.
    fn reserve(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        bytes: u64,
        keep: u64,
    ) -> bool {
        if bytes <= self.buffer.size() {
            return false;
        }
        let bigger = storage(device, self.label, bytes.next_power_of_two());
        if keep > 0 {
            encoder.copy_buffer_to_buffer(&self.buffer, 0, &bigger, 0, keep);
        }
        self.buffer = bigger;
        true
    }
}

fn storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn texture(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// A texture to draw into, and its bind group for reading it as an image.
struct Surface {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    image: wgpu::BindGroup,
}

impl Surface {
    fn new(gpu: &Gpu, texture: wgpu::Texture) -> Surface {
        let view = texture.create_view(&Default::default());
        let image = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &gpu.image,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Surface {
            texture,
            view,
            image,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Run {
    Rects(Range<u32>),
    Gradients(Range<u32>),
    Glyphs(Range<u32>),
}

/// Targets: 0 is the frame, the others layers.
#[derive(Debug, PartialEq)]
enum Step {
    Draw {
        target: usize,
        runs: Vec<Run>,
    },
    Composite {
        target: usize,
        layer: usize,
        at: u32,
    },
    /// Blends `layer` with what is under it in `target`.
    Blend {
        target: usize,
        layer: usize,
        at: u32,
    },
    /// Composites `layer` onto `target` through the alpha of `mask`.
    Mask {
        target: usize,
        layer: usize,
        mask: usize,
    },
    /// Draws `layer` blurred along one axis into `target`, a new layer.
    Blur {
        target: usize,
        layer: usize,
        at: u32,
    },
    /// Draws `layer` with its colors through matrices into `target`, a new layer.
    Colors {
        target: usize,
        layer: usize,
        at: u32,
    },
    /// Draws `layer` through one of Valle's effects into `target`, a new layer.
    Effect {
        target: usize,
        layer: usize,
        at: u32,
    },
    /// Copies `layer`, or the frame, into `target`, a new layer.
    Copy {
        target: usize,
        layer: usize,
    },
}

impl Step {
    /// The layers the step reads.
    fn reads(&self) -> [Option<usize>; 2] {
        match *self {
            Step::Draw { .. } => [None; 2],
            Step::Mask { layer, mask, .. } => [Some(layer), Some(mask)],
            Step::Composite { layer, .. }
            | Step::Blend { layer, .. }
            | Step::Blur { layer, .. }
            | Step::Colors { layer, .. }
            | Step::Effect { layer, .. }
            | Step::Copy { layer, .. } => [Some(layer), None],
        }
    }
}

/// What is under what a plan is at, as a backdrop reads it.
#[derive(Clone, Default)]
struct Seen {
    here: Under,
    /// Around the innermost group, which a mask in it keeps: the backdrop of the box they are of.
    group: Under,
}

/// The nearest target that started empty for what is drawn in it, a group's or a filter's layer,
/// or the frame, and the layers begun since, each with the instance compositing it.
#[derive(Clone, Default)]
struct Under {
    root: usize,
    open: Vec<(usize, u32)>,
}

#[derive(Default)]
struct Plan {
    transforms: Vec<u32>,
    rects: Vec<u32>,
    gradients: Vec<u32>,
    segments: Vec<u32>,
    glyphs: Vec<u32>,
    layers: Vec<u32>,
    steps: Vec<Step>,
    targets: usize,
    seen: Seen,
    /// The frame's paths, encoded as glyph outlines are (curve words, list words), which go
    /// after the atlas for this frame only; and where in `glyphs` each path instance has its
    /// first curve texel and list word, which are from the start of these.
    paths: (Vec<u32>, Vec<u32>),
    placed: Vec<usize>,
}

impl Plan {
    /// A new layer.
    fn layer(&mut self) -> usize {
        self.targets += 1;
        self.targets - 1
    }

    /// Adds the words of a layer instance, or of a pass that takes their place: its index.
    fn words(&mut self, words: impl IntoIterator<Item = f32>) -> u32 {
        let at = (self.layers.len() / LAYER) as u32;
        self.layers.extend(words.into_iter().map(f32::to_bits));
        at
    }

    /// `layer` composited onto `target`, all of it.
    fn composite(&mut self, target: usize, layer: usize) {
        let words = [1.0, 0.0, 0.0, 0.0, -1.0e7, -1.0e7, 2.0e7, 2.0e7];
        let at = self.words(words.into_iter().chain([0.0; 12]));
        self.steps.push(Step::Composite { target, layer, at });
    }

    /// A new layer of `layer` blurred by a Gaussian of `sigma` px: along x, then along y. Within
    /// `bounds` (x, y, width and height, whole px) if they are given, which it reads as if
    /// mirrored at their edges.
    fn blurred(&mut self, mut layer: usize, sigma: f32, bounds: Option<[f32; 4]>) -> usize {
        if sigma > 0.0 {
            for [x, y] in [[1.0, 0.0], [0.0, 1.0]] {
                let words = [sigma, x, y, 0.0].into_iter();
                let words = words.chain(bounds.unwrap_or([0.0; 4])).chain([0.0; 12]);
                let at = self.words(words);
                let target = self.layer();
                self.steps.push(Step::Blur { target, layer, at });
                layer = target;
            }
        }
        layer
    }

    /// A new layer of `layer` through `filters`, in the px `matrix` takes to the target's, where
    /// a blur is as much larger as the transform makes it and an offset turns with it. Blurs are
    /// within `bounds` if they are given.
    fn filtered(
        &mut self,
        mut layer: usize,
        filters: &[Filter],
        matrix: Matrix,
        bounds: Option<[f32; 4]>,
    ) -> usize {
        let scale = matrix.radius_scale() as f32;
        let [x, y, _] = matrix.0;
        for filter in filters {
            layer = match filter {
                Filter::Blur(sigma) => self.blurred(layer, sigma * scale, bounds),
                Filter::Colors(matrices) => self.colored(layer, [0.0; 2], matrices, false),
                Filter::Luminance => self.colored(layer, [0.0; 2], &[LUMINANCE], true),
                Filter::Aberration(offset) => {
                    let d = f64::from(*offset);
                    let [dx, dy] = [0, 1].map(|k| (x[k] * d) as f32);
                    self.effect(layer, ABERRATION, &[dx, dy])
                }
                Filter::Lens { k, frame } => {
                    let Some(t) = self.transform(matrix) else {
                        continue;
                    };
                    let [x0, y0, w, h] = [frame.x, frame.y, frame.width, frame.height];
                    self.effect(layer, LENS, &[t as f32, k[0], k[1], x0, y0, w, h])
                }
                Filter::Grain {
                    seed,
                    amount,
                    size,
                    origin,
                } => {
                    let Some(t) = self.transform(matrix) else {
                        continue;
                    };
                    let seed = f32::from_bits(*seed);
                    self.effect(
                        layer,
                        GRAIN,
                        &[t as f32, seed, *amount, *size, origin[0], origin[1]],
                    )
                }
                Filter::Radial { center, amount } => {
                    let Some(t) = self.transform(matrix) else {
                        continue;
                    };
                    if *amount <= 0.0 {
                        continue;
                    }
                    // Passes of three taps, each twice as far apart as the last, 1, 2, 4… parts
                    // of 2ⁿ − 1 of the reach: together a triangle across it either way, its taps
                    // at most a px apart where the transform stretches it most.
                    let reach = f64::from(*amount) * x[0].hypot(x[1]).max(y[0].hypot(y[1]));
                    let mut passes = 1;
                    while f64::from((1u32 << passes) - 1) < reach && passes < 16 {
                        passes += 1;
                    }
                    let parts = ((1u32 << passes) - 1) as f32;
                    for k in 0..passes {
                        let part = (1u32 << k) as f32 / parts;
                        let words = [t as f32, center[0], center[1], *amount, part];
                        layer = self.effect(layer, RADIAL, &words);
                    }
                    layer
                }
                Filter::Shadow {
                    offset: [dx, dy],
                    sigma,
                    color,
                } => {
                    let a = color[3];
                    if a <= 0.0 {
                        continue;
                    }
                    // The alpha in the color, moved and blurred, under the layer.
                    let [r, g, b] = [0, 1, 2].map(|k| color[k] / a);
                    let mut tint = [0.0; 20];
                    tint[15] = a;
                    tint[16..19].copy_from_slice(&[r, g, b]);
                    let (dx, dy) = (f64::from(*dx), f64::from(*dy));
                    let offset = [0, 1].map(|k| (x[k] * dx + y[k] * dy) as f32);
                    let shadow = self.colored(layer, offset, &[tint], false);
                    let shadow = self.blurred(shadow, sigma * scale, bounds);
                    let both = self.layer();
                    self.composite(both, shadow);
                    self.composite(both, layer);
                    both
                }
            };
        }
        layer
    }

    /// A new layer of `layer` through one of Valle's effects, `kind`, with its parameters.
    fn effect(&mut self, layer: usize, kind: f32, parameters: &[f32]) -> usize {
        let words = std::iter::once(kind).chain(parameters.iter().copied());
        let at = self.words(words.chain(std::iter::repeat(0.0)).take(LAYER));
        let target = self.layer();
        self.steps.push(Step::Effect { target, layer, at });
        target
    }

    /// A new layer of `layer` moved by `offset` px, its colors through `matrices`, its
    /// `premultiplied` ones if asked.
    fn colored(
        &mut self,
        layer: usize,
        offset: [f32; 2],
        matrices: &[[f32; 20]],
        premultiplied: bool,
    ) -> usize {
        let count = matrices.len() as f32;
        let flag = f32::from(u8::from(premultiplied));
        let at = self.words(
            [count, offset[0], offset[1], flag]
                .into_iter()
                .chain([0.0; 16]),
        );
        for m in matrices {
            self.words(*m);
        }
        let target = self.layer();
        self.steps.push(Step::Colors { target, layer, at });
        target
    }

    /// Adds `m` and its inverse to the transforms: its index, or `None` if it has no inverse.
    fn transform(&mut self, m: Matrix) -> Option<u32> {
        let inverse = m.inverse()?;
        let i = (self.transforms.len() / TRANSFORM) as u32;
        for c in m.0.into_iter().chain(inverse.0) {
            let column = [c[0], c[1], c[2], 0.0];
            self.transforms.extend(column.map(|v| (v as f32).to_bits()));
        }
        Some(i)
    }
}

/// Where instances are: a transform's index and the transform.
type At = (u32, Matrix);

/// Draws display lists of one size, and keeps the outlines of the glyphs they show.
pub struct Renderer<'a> {
    gpu: &'a Gpu,
    size: [u32; 2],
    frame: Surface,
    output: wgpu::Texture,
    readback: wgpu::Buffer,
    screen: wgpu::Buffer,
    transforms: Store,
    rects: Store,
    gradients: Store,
    segments: Store,
    glyphs: Store,
    layers: Store,
    curves: Store,
    lists: Store,
    /// Words in `curves` (4 a texel) and `lists` so far.
    atlas_len: [u64; 2],
    /// By face, glyph and the bits of the width of its stroke in ems (0 for its fill); `None` for
    /// a glyph without curves.
    outlines: HashMap<(u32, u32, u32), Option<Placed>>,
    draw: Option<wgpu::BindGroup>,
    /// Layer textures: all of them, and the free ones.
    pool: Vec<Surface>,
    free: Vec<usize>,
}

impl<'a> Renderer<'a> {
    pub fn new(gpu: &'a Gpu, width: u32, height: u32) -> Renderer<'a> {
        let device = &gpu.device;
        let size = [width, height];
        let frame = Surface::new(gpu, texture(device, size, WORK));
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row(width) * height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let screen = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let words = [width as f32, height as f32, 0.0, 0.0].map(f32::to_bits);
        gpu.queue.write_buffer(&screen, 0, &bytes(&words));
        Renderer {
            gpu,
            size,
            frame,
            output: texture(device, size, OUTPUT),
            readback,
            screen,
            transforms: Store::new(device, "transforms"),
            rects: Store::new(device, "rects"),
            gradients: Store::new(device, "gradients"),
            segments: Store::new(device, "segments"),
            glyphs: Store::new(device, "glyphs"),
            layers: Store::new(device, "layers"),
            curves: Store::new(device, "curves"),
            lists: Store::new(device, "lists"),
            atlas_len: [0, 0],
            outlines: HashMap::new(),
            draw: None,
            pool: Vec::new(),
            free: Vec::new(),
        }
    }

    /// The frame `items` paint: straight sRGB, 4 bytes a pixel, rows top down. `faces` are the
    /// faces their glyphs name.
    pub fn render(&mut self, items: &[Item], faces: &[Face]) -> Result<Vec<u8>, Error> {
        let mut plan = Plan {
            targets: 1,
            ..Plan::default()
        };
        let identity = plan.transform(Matrix::IDENTITY);
        let at = (identity.ok_or("engine bug: no identity")?, Matrix::IDENTITY);
        let mut atlas = (Vec::new(), Vec::new());
        self.plan(items, 0, at, faces, &mut plan, &mut atlas)?;
        let gpu = self.gpu;
        let device = &gpu.device;
        let mut encoder = device.create_command_encoder(&Default::default());

        // The frame's paths go after the atlas, where the next frame's glyphs may go.
        let [curves_len, lists_len] = self.atlas_len;
        let kept = [
            curves_len + atlas.0.len() as u64,
            lists_len + atlas.1.len() as u64,
        ];
        for &i in &plan.placed {
            plan.glyphs[i] += (kept[0] / 4) as u32;
            plan.glyphs[i + 1] += kept[1] as u32;
        }
        atlas.0.extend(&plan.paths.0);
        atlas.1.extend(&plan.paths.1);
        let mut changed = self.curves.reserve(
            device,
            &mut encoder,
            (curves_len + atlas.0.len() as u64) * 4,
            curves_len * 4,
        );
        changed |= self.lists.reserve(
            device,
            &mut encoder,
            (lists_len + atlas.1.len() as u64) * 4,
            lists_len * 4,
        );
        let queue = &gpu.queue;
        queue.write_buffer(&self.curves.buffer, curves_len * 4, &bytes(&atlas.0));
        queue.write_buffer(&self.lists.buffer, lists_len * 4, &bytes(&atlas.1));
        self.atlas_len = kept;
        for (store, words) in [
            (&mut self.transforms, &plan.transforms),
            (&mut self.rects, &plan.rects),
            (&mut self.gradients, &plan.gradients),
            (&mut self.segments, &plan.segments),
            (&mut self.glyphs, &plan.glyphs),
            (&mut self.layers, &plan.layers),
        ] {
            changed |= store.reserve(device, &mut encoder, words.len() as u64 * 4, 0);
            queue.write_buffer(&store.buffer, 0, &bytes(words));
        }
        if changed || self.draw.is_none() {
            fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
                wgpu::BindGroupEntry {
                    binding,
                    resource: buffer.as_entire_binding(),
                }
            }
            self.draw = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("draw"),
                layout: &gpu.draw,
                entries: &[
                    entry(0, &self.screen),
                    entry(1, &self.rects.buffer),
                    entry(2, &self.glyphs.buffer),
                    entry(3, &self.curves.buffer),
                    entry(4, &self.lists.buffer),
                    entry(5, &self.layers.buffer),
                    entry(6, &self.gradients.buffer),
                    entry(7, &self.segments.buffer),
                    entry(8, &self.transforms.buffer),
                ],
            }));
        }
        self.encode(&mut encoder, &plan);

        let [width, height] = self.size;
        encoder.copy_texture_to_buffer(
            self.output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row(width)),
                    rows_per_image: Some(height),
                },
            },
            self.output.size(),
        );
        queue.submit([encoder.finish()]);
        let (sent, mapped) = mpsc::channel();
        self.readback.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = sent.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("GPU: {e}"))?;
        mapped
            .recv()
            .map_err(|e| format!("engine bug: readback: {e}"))?
            .map_err(|e| format!("GPU readback: {e}"))?;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        {
            let view = self
                .readback
                .get_mapped_range(..)
                .map_err(|e| format!("engine bug: readback: {e}"))?;
            for line in view.chunks(row(width) as usize) {
                pixels.extend_from_slice(&line[..(width * 4) as usize]);
            }
        }
        self.readback.unmap();
        Ok(pixels)
    }

    /// Plans drawing `items` into `target`, at `at`.
    fn plan(
        &mut self,
        items: &[Item],
        target: usize,
        at: At,
        faces: &[Face],
        plan: &mut Plan,
        atlas: &mut (Vec<u32>, Vec<u32>),
    ) -> Result<(), Error> {
        let mut runs = Vec::new();
        self.draw(items, target, at, faces, plan, atlas, &mut runs)?;
        if !runs.is_empty() {
            plan.steps.push(Step::Draw { target, runs });
        }
        Ok(())
    }

    /// Plans `items` into `target` at `at`, adding their instances to `runs`, which a layer's
    /// steps come after.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        items: &[Item],
        target: usize,
        at: At,
        faces: &[Face],
        plan: &mut Plan,
        atlas: &mut (Vec<u32>, Vec<u32>),
        runs: &mut Vec<Run>,
    ) -> Result<(), Error> {
        let (transform, matrix) = at;
        for item in items {
            match item {
                Item::Rect { .. } | Item::Ring { .. } => {
                    // A fill is a ring whose inside has no size.
                    let (rect, radii, inside, inner_radii, colors) = match item {
                        Item::Rect { rect, radii, color } => {
                            (rect, radii, &Rect::default(), &SQUARE, [*color; 4])
                        }
                        Item::Ring {
                            rect,
                            radii,
                            inner,
                            inner_radii,
                            colors,
                        } => (rect, radii, inner, inner_radii, *colors),
                        _ => unreachable!("a rectangle or a ring"),
                    };
                    let i = (plan.rects.len() / RECT) as u32;
                    let words = (xywh(*rect).into_iter())
                        .chain(radii.iter().flatten().copied())
                        .chain(xywh(*inside))
                        .chain(inner_radii.iter().flatten().copied())
                        .chain(colors.into_iter().flatten());
                    plan.rects.extend(words.map(f32::to_bits));
                    plan.rects.extend([transform, 0, 0, 0]);
                    extend(runs, Run::Rects(i..i + 1));
                }
                Item::Gradient {
                    rect,
                    radii,
                    tiles,
                    ramp,
                } => {
                    let i = (plan.gradients.len() / GRADIENT) as u32;
                    let first = (plan.segments.len() / SEGMENT) as u32;
                    for s in &ramp.segments {
                        let words = [s.at[0], s.at[1], 0.0, 0.0].into_iter();
                        let words = words.chain(s.colors.into_iter().flatten());
                        plan.segments.extend(words.map(f32::to_bits));
                    }
                    let (shape, [[a, b], [c, d]]) = match ramp.geometry {
                        Geometry::Line(ends) => (0, ends),
                        Geometry::Ellipse(center, radii) => (1, [center, radii]),
                        Geometry::Turn(center, from) => (2, [center, [from, 0.0]]),
                    };
                    let words = (xywh(*rect).into_iter())
                        .chain(radii.iter().flatten().copied())
                        .chain(xywh(tiles.first))
                        .chain(tiles.step.map(|s| s.unwrap_or(0.0)))
                        .chain([0.0, 0.0, a, b, c, d]);
                    plan.gradients.extend(words.map(f32::to_bits));
                    let space = match ramp.space {
                        Space::Srgb => 0,
                        Space::SrgbLinear => 1,
                        Space::Oklab => 2,
                        Space::Oklch(_) => 3,
                    };
                    let count = ramp.segments.len() as u32;
                    let repeating = u32::from(ramp.repeating);
                    plan.gradients
                        .extend([repeating, space, shape, transform, first, count, 0, 0]);
                    extend(runs, Run::Gradients(i..i + 1));
                }
                Item::Glyphs {
                    face,
                    size,
                    color,
                    stroke,
                    glyphs,
                } => {
                    for &(id, [x, y]) in glyphs {
                        let Some(o) = self.outline(*face, id, stroke / size, faces, atlas)? else {
                            continue;
                        };
                        let i = (plan.glyphs.len() / GLYPH) as u32;
                        let floats = [x, y, size / o.em, 0.0].into_iter().chain(o.bounds).chain([
                            o.band_scale[0],
                            o.band_scale[1],
                            0.0,
                            0.0,
                        ]);
                        plan.glyphs.extend(floats.map(f32::to_bits));
                        let bands = o.bands[0] | o.bands[1] << 16;
                        plan.glyphs.extend([o.at[0], o.at[1], bands, transform]);
                        plan.glyphs.extend(color.map(f32::to_bits));
                        extend(runs, Run::Glyphs(i..i + 1));
                    }
                }
                Item::Path {
                    path,
                    even_odd,
                    color,
                } => {
                    // An outline of 1 px a unit, y up, as glyph outlines are.
                    let tolerance = TOLERANCE / (matrix.radius_scale() as f32).max(1e-6);
                    let contours: Vec<Vec<Curve>> = (path.quads(tolerance).into_iter())
                        .map(|c| c.into_iter().map(|q| q.map(|[x, y]| [x, -y])).collect())
                        .collect();
                    let o = slug::encode(1.0, &contours);
                    if o.curves.is_empty() {
                        continue;
                    }
                    let (curves, lists) = &mut plan.paths;
                    let first = [(curves.len() / 4) as u32, lists.len() as u32];
                    curves.extend(o.curves.iter().flatten().map(|v| v.to_bits()));
                    lists.extend(&o.lists);
                    let i = (plan.glyphs.len() / GLYPH) as u32;
                    let [u, v] = o.band_scale();
                    let fill = f32::from(u8::from(*even_odd));
                    let floats = [0.0, 0.0, 1.0, fill].into_iter().chain(o.bounds);
                    let floats = floats.chain([u, v, 0.0, 0.0]);
                    plan.glyphs.extend(floats.map(f32::to_bits));
                    plan.placed.push(plan.glyphs.len());
                    let bands = o.bands[0] | o.bands[1] << 16;
                    plan.glyphs.extend([first[0], first[1], bands, transform]);
                    plan.glyphs.extend(color.map(f32::to_bits));
                    extend(runs, Run::Glyphs(i..i + 1));
                }
                Item::Transform { matrix: m, items } => {
                    let m = matrix.times(*m);
                    // A transform that flattens the plane shows nothing.
                    if let Some(i) = plan.transform(m) {
                        self.draw(items, target, (i, m), faces, plan, atlas, runs)?;
                    }
                }
                Item::Filter { filters, items } => {
                    if !runs.is_empty() {
                        let runs = std::mem::take(runs);
                        plan.steps.push(Step::Draw { target, runs });
                    }
                    let layer = plan.layer();
                    let mut inside = plan.seen.clone();
                    inside.here = Under {
                        root: layer,
                        open: Vec::new(),
                    };
                    let around = std::mem::replace(&mut plan.seen, inside);
                    self.plan(items, layer, at, faces, plan, atlas)?;
                    plan.seen = around;
                    let layer = plan.filtered(layer, filters, matrix, None);
                    plan.composite(target, layer);
                }
                Item::Backdrop {
                    filters,
                    rect,
                    radii,
                } => {
                    if !runs.is_empty() {
                        let runs = std::mem::take(runs);
                        plan.steps.push(Step::Draw { target, runs });
                    }
                    // What is under the box's group: where it began, and the layers begun since,
                    // as they are, in the box's border box, its edges mirrored.
                    let Under { root, open } = plan.seen.group.clone();
                    let copy = plan.layer();
                    plan.steps.push(Step::Copy {
                        target: copy,
                        layer: root,
                    });
                    for (layer, at) in open {
                        plan.steps.push(Step::Composite {
                            target: copy,
                            layer,
                            at,
                        });
                    }
                    let b = matrix.bounds(*rect);
                    let [x0, y0] = [b.x, b.y].map(f32::floor);
                    let [x1, y1] = [b.x + b.width, b.y + b.height].map(f32::ceil);
                    let bounds = [x0, y0, x1 - x0, y1 - y0];
                    let layer = plan.filtered(copy, filters, matrix, Some(bounds));
                    let at = plan.words([1.0, 0.0, 0.0, 0.0].into_iter().chain(xywh(*rect)));
                    plan.layers
                        .extend(radii.iter().flatten().map(|v| v.to_bits()));
                    plan.layers.extend([transform, 0, 0, 0]);
                    plan.steps.push(Step::Composite { target, layer, at });
                }
                Item::Group {
                    opacity,
                    blend,
                    items,
                } if *blend != Blend::Normal => {
                    if !runs.is_empty() {
                        let runs = std::mem::take(runs);
                        plan.steps.push(Step::Draw { target, runs });
                    }
                    let layer = plan.layer();
                    let inside = Seen {
                        here: Under {
                            root: layer,
                            open: Vec::new(),
                        },
                        group: plan.seen.here.clone(),
                    };
                    let around = std::mem::replace(&mut plan.seen, inside);
                    self.plan(items, layer, at, faces, plan, atlas)?;
                    plan.seen = around;
                    let words = [*opacity, *blend as u32 as f32, 0.0, 0.0];
                    let at = plan.words(words.into_iter().chain([0.0; 16]));
                    plan.steps.push(Step::Blend { target, layer, at });
                }
                Item::Mask { mask, items } if lone(mask).is_none() => {
                    if !runs.is_empty() {
                        let runs = std::mem::take(runs);
                        plan.steps.push(Step::Draw { target, runs });
                    }
                    // The items' layer and the mask's, each starting empty for what is in it, as
                    // a group's does. The mask is the box's, so what is in it of the box sees the
                    // backdrop the box's group does.
                    let layer = plan.layer();
                    let shape = plan.layer();
                    for (into, items) in [(layer, items), (shape, mask)] {
                        let inside = Seen {
                            here: Under {
                                root: into,
                                open: Vec::new(),
                            },
                            group: plan.seen.group.clone(),
                        };
                        let around = std::mem::replace(&mut plan.seen, inside);
                        self.plan(items, into, at, faces, plan, atlas)?;
                        plan.seen = around;
                    }
                    plan.steps.push(Step::Mask {
                        target,
                        layer,
                        mask: shape,
                    });
                }
                Item::Group { .. }
                | Item::Clip { .. }
                | Item::Mask { .. }
                | Item::Cut { .. }
                | Item::Shadow { .. } => {
                    if !runs.is_empty() {
                        let runs = std::mem::take(runs);
                        plan.steps.push(Step::Draw { target, runs });
                    }
                    // A group shows all of its layer, a clip the part in its rectangle, and so
                    // does a mask that is one opaque rectangle, a cut the part on one side of its
                    // line, and a shadow its layer blurred, inside its rectangle or outside it.
                    // The group's is on the target, and the others in their transform's px, where a
                    // blur is as much larger as the transform makes it.
                    let [width, height] = self.size.map(|s| s as f32);
                    let whole = [-1.0, -1.0, width + 2.0, height + 2.0];
                    let (items, opacity, sigma, clip, radii, outside) = match item {
                        Item::Group { opacity, items, .. } => {
                            (items, *opacity, 0.0, whole, SQUARE, false)
                        }
                        Item::Clip { rect, radii, items } => {
                            (items, 1.0, 0.0, xywh(*rect), *radii, false)
                        }
                        Item::Mask { mask, items } => {
                            let Some((rect, radii)) = lone(mask) else {
                                unreachable!("a mask of one opaque rectangle")
                            };
                            (items, 1.0, 0.0, xywh(rect), radii, false)
                        }
                        Item::Cut {
                            line: [a, b, c],
                            items,
                        } => (items, 1.0, 0.0, [*a, *b, *c, 0.0], SQUARE, false),
                        Item::Shadow {
                            sigma,
                            rect,
                            radii,
                            inside,
                            items,
                        } => (items, 1.0, *sigma, xywh(*rect), *radii, !inside),
                        _ => unreachable!("a group, a clip or a shadow"),
                    };
                    let index = (plan.layers.len() / LAYER) as u32;
                    let cut = f32::from(u8::from(matches!(item, Item::Cut { .. })));
                    let words = [opacity, f32::from(u8::from(outside)), cut, 0.0]
                        .into_iter()
                        .chain(clip)
                        .chain(radii.into_iter().flatten());
                    plan.layers.extend(words.map(f32::to_bits));
                    let on = match item {
                        Item::Group { .. } => 0,
                        _ => transform,
                    };
                    plan.layers.extend([on, 0, 0, 0]);
                    let layer = plan.layer();
                    // A group's layer, and a mask's, is under what is in it; a clip's or a cut's
                    // is drawn over what is under it. A mask is its box's, as its group is.
                    let inside = match item {
                        Item::Group { .. } => Seen {
                            here: Under {
                                root: layer,
                                open: Vec::new(),
                            },
                            group: plan.seen.here.clone(),
                        },
                        Item::Mask { .. } => Seen {
                            here: Under {
                                root: layer,
                                open: Vec::new(),
                            },
                            group: plan.seen.group.clone(),
                        },
                        Item::Shadow { .. } => plan.seen.clone(),
                        _ => {
                            let mut seen = plan.seen.clone();
                            seen.here.open.push((layer, index));
                            seen
                        }
                    };
                    let around = std::mem::replace(&mut plan.seen, inside);
                    self.plan(items, layer, at, faces, plan, atlas)?;
                    plan.seen = around;
                    let layer = plan.blurred(layer, sigma * matrix.radius_scale() as f32, None);
                    plan.steps.push(Step::Composite {
                        target,
                        layer,
                        at: index,
                    });
                }
            }
        }
        Ok(())
    }

    /// Where glyph `id` of face `face` is in the atlas, or its stroke `stroke` ems wide, encoding
    /// it into `atlas` (curve words, list words) if it is new.
    fn outline(
        &mut self,
        face: u32,
        id: u32,
        stroke: f32,
        faces: &[Face],
        atlas: &mut (Vec<u32>, Vec<u32>),
    ) -> Result<Option<Placed>, Error> {
        let key = (face, id, stroke.to_bits());
        if let Some(placed) = self.outlines.get(&key) {
            return Ok(*placed);
        }
        let f = faces
            .get(face as usize)
            .ok_or("engine bug: a glyph names a face that is not there")?;
        let o = slug::outline(f, id, stroke)?;
        let placed = (!o.curves.is_empty()).then(|| {
            let at = [
                ((self.atlas_len[0] + atlas.0.len() as u64) / 4) as u32,
                (self.atlas_len[1] + atlas.1.len() as u64) as u32,
            ];
            atlas
                .0
                .extend(o.curves.iter().flatten().map(|v| v.to_bits()));
            atlas.1.extend(&o.lists);
            Placed {
                em: o.em,
                bounds: o.bounds,
                band_scale: o.band_scale(),
                bands: o.bands,
                at,
            }
        });
        self.outlines.insert(key, placed);
        Ok(placed)
    }

    fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, plan: &Plan) {
        let gpu = self.gpu;
        let Some(draw) = &self.draw else {
            return;
        };
        // Each layer's pool texture once it has one, whether a pass drew into each target, and
        // the last step that reads each layer, after which its texture goes back to the pool.
        let mut surfaces: Vec<Option<usize>> = vec![None; plan.targets];
        let mut started = vec![false; plan.targets];
        let mut last = vec![0; plan.targets];
        for (k, step) in plan.steps.iter().enumerate() {
            for l in step.reads().into_iter().flatten() {
                last[l] = k;
            }
        }
        for (k, step) in plan.steps.iter().enumerate() {
            if let Step::Copy { target, layer } = *step {
                // Nothing to copy from a layer nothing was drawn into, and the frame is cleared
                // first if nothing was.
                if layer != 0 && surfaces[layer].is_none() {
                    continue;
                }
                let to = take(gpu, &mut self.pool, &mut self.free, self.size);
                surfaces[target] = Some(to);
                started[target] = true;
                let from = surfaces[layer].map_or(&self.frame, |s| &self.pool[s]);
                if !std::mem::replace(&mut started[layer], true) {
                    drop(begin(encoder, &from.view, false));
                }
                encoder.copy_texture_to_texture(
                    from.texture.as_image_copy(),
                    self.pool[to].texture.as_image_copy(),
                    from.texture.size(),
                );
                continue;
            }
            let target = match *step {
                Step::Draw { target, .. }
                | Step::Composite { target, .. }
                | Step::Blend { target, .. }
                | Step::Mask { target, .. }
                | Step::Blur { target, .. }
                | Step::Colors { target, .. }
                | Step::Effect { target, .. } => target,
                Step::Copy { .. } => unreachable!("copied above"),
            };
            // The layers the step reads go back to the pool once it is their last; a layer
            // nothing was drawn into shows nothing, nor does a mask let anything through.
            let reads = step.reads();
            let done = reads.map(|l| l.filter(|&l| last[l] == k).and_then(|l| surfaces[l]));
            if reads.into_iter().flatten().any(|l| surfaces[l].is_none()) {
                self.free.extend(done.into_iter().flatten());
                continue;
            }
            let [layer, mask] = reads.map(|l| l.and_then(|l| surfaces[l]));
            let surface =
                match target {
                    0 => None,
                    t => Some(*surfaces[t].get_or_insert_with(|| {
                        take(gpu, &mut self.pool, &mut self.free, self.size)
                    })),
                };
            // What a blend is under, copied: a target nothing was drawn into yet is cleared
            // first, as the pool's textures hold what they last did.
            let under = match step {
                Step::Blend { .. } => {
                    let copy = take(gpu, &mut self.pool, &mut self.free, self.size);
                    let from = surface.map_or(&self.frame, |s| &self.pool[s]);
                    if !std::mem::replace(&mut started[target], true) {
                        drop(begin(encoder, &from.view, false));
                    }
                    encoder.copy_texture_to_texture(
                        from.texture.as_image_copy(),
                        self.pool[copy].texture.as_image_copy(),
                        from.texture.size(),
                    );
                    Some(copy)
                }
                _ => None,
            };
            let view = &surface.map_or(&self.frame, |s| &self.pool[s]).view;
            let mut pass = begin(encoder, view, std::mem::replace(&mut started[target], true));
            pass.set_bind_group(0, draw, &[]);
            match step {
                Step::Copy { .. } => {}
                Step::Mask { .. } => {
                    let [Some(s), Some(m)] = [layer, mask] else {
                        unreachable!("a mask step reads two layers");
                    };
                    pass.set_pipeline(&gpu.masks);
                    pass.set_bind_group(1, &self.pool[s].image, &[]);
                    pass.set_bind_group(2, &self.pool[m].image, &[]);
                    pass.draw(0..3, 0..1);
                }
                Step::Draw { runs, .. } => {
                    for run in runs {
                        let (pipeline, range) = match run {
                            Run::Rects(r) => (&gpu.rects, r),
                            Run::Gradients(r) => (&gpu.gradients, r),
                            Run::Glyphs(r) => (&gpu.glyphs, r),
                        };
                        pass.set_pipeline(pipeline);
                        pass.draw(0..4, range.clone());
                    }
                }
                Step::Composite { at, .. }
                | Step::Blend { at, .. }
                | Step::Blur { at, .. }
                | Step::Colors { at, .. }
                | Step::Effect { at, .. } => {
                    let s = layer.expect("a composited layer has a surface");
                    pass.set_pipeline(match step {
                        Step::Blend { .. } => &gpu.blends,
                        Step::Blur { .. } => &gpu.blurs,
                        Step::Colors { .. } => &gpu.colors,
                        Step::Effect { .. } => &gpu.effects,
                        _ => &gpu.layers,
                    });
                    pass.set_bind_group(1, &self.pool[s].image, &[]);
                    if let Some(u) = under {
                        pass.set_bind_group(2, &self.pool[u].image, &[]);
                    }
                    pass.draw(0..3, *at..*at + 1);
                }
            }
            drop(pass);
            self.free.extend(done.into_iter().flatten().chain(under));
        }
        if !started[0] {
            drop(begin(encoder, &self.frame.view, false));
        }
        let view = self.output.create_view(&Default::default());
        let mut pass = begin(encoder, &view, false);
        pass.set_pipeline(&gpu.output);
        pass.set_bind_group(0, draw, &[]);
        pass.set_bind_group(1, &self.frame.image, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A free texture from the pool of layer textures, or a new one.
fn take(gpu: &Gpu, pool: &mut Vec<Surface>, free: &mut Vec<usize>, size: [u32; 2]) -> usize {
    free.pop().unwrap_or_else(|| {
        pool.push(Surface::new(gpu, texture(&gpu.device, size, WORK)));
        pool.len() - 1
    })
}

/// A pass drawing into `view`: over what it holds once `started`, else from transparent.
fn begin<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    started: bool,
) -> wgpu::RenderPass<'e> {
    let load = if started {
        wgpu::LoadOp::Load
    } else {
        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
    };
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// Bytes per row of a readback: padded as copies need.
fn row(width: u32) -> u32 {
    (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

/// The rectangle a mask is, if it is one opaque rectangle: what it shows is what a clip to it
/// shows.
fn lone(mask: &[Item]) -> Option<(Rect, Radii)> {
    match mask {
        [Item::Rect { rect, radii, color }] if color[3] >= 1.0 => Some((*rect, *radii)),
        _ => None,
    }
}

/// A rectangle as shaders take it: x, y, width and height.
fn xywh(r: Rect) -> [f32; 4] {
    [r.x, r.y, r.width, r.height]
}

/// Appends instance `next` to the last run when it is of the same kind and follows it.
fn extend(runs: &mut Vec<Run>, next: Run) {
    match (runs.last_mut(), &next) {
        (Some(Run::Rects(r)), Run::Rects(n))
        | (Some(Run::Gradients(r)), Run::Gradients(n))
        | (Some(Run::Glyphs(r)), Run::Glyphs(n))
            if r.end == n.start =>
        {
            r.end = n.end;
        }
        _ => runs.push(next),
    }
}
