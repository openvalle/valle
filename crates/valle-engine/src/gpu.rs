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
//!
//! A shader authors write (`shader.rs`) draws its box's quad straight onto its target, through a
//! pipeline of its own, made once; the images it reads are taken out of their layers first, each
//! into a texture of the box's size.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, mpsc};
use std::task::{Context, Poll, Waker};

use wgpu::util::DeviceExt;

use crate::image::Image;
use crate::layout::Rect;
use crate::paint::{Geometry, Item, Matrix, Radii, SQUARE, Spread, Tiles};
use crate::path::{Curve, Path};
use crate::shader::Program;
use crate::slug;
use crate::style::Blend;
use crate::style::gradient::Space;
use crate::template::Error;
use crate::text::{Face, color};
use crate::transition;

mod plan;
use plan::{Plan, Reach, Seen, Under};

const WORK: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

const OUTPUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const LUMA: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

const CHROMA: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;

const COMMON: &str = include_str!("../shaders/common.wgsl");

/// u32 words per rectangle, gradient, glyph and layer instance, per gradient segment and per
/// transform; blur passes take a layer's words too.
const RECT: usize = 44;

const GRADIENT: usize = 32;

const GLYPH: usize = 24;

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

const DISPLACEMENT: f32 = 4.0;

const LIGHT: f32 = 5.0;

const MOTION: f32 = 6.0;

/// Most taps a motion blur averages; past as many px its taps spread.
const MOTION_TAPS: f64 = 128.0;

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
    images: wgpu::RenderPipeline,
    glyphs: wgpu::RenderPipeline,
    layers: wgpu::RenderPipeline,
    adds: wgpu::RenderPipeline,
    blends: wgpu::RenderPipeline,
    masks: wgpu::RenderPipeline,
    blurs: wgpu::RenderPipeline,
    colors: wgpu::RenderPipeline,
    effects: wgpu::RenderPipeline,
    output: wgpu::RenderPipeline,
    /// The output as an encoder's luma and chroma planes.
    luma: wgpu::RenderPipeline,
    chroma: wgpu::RenderPipeline,
    /// Shaders authors write: the block of what the engine tells them, the quad of their box,
    /// the pass taking their images out of layers, and how they sample those.
    host: wgpu::BindGroupLayout,
    quad: wgpu::ShaderModule,
    extract: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
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
        now(Gpu::of(adapter))?
    }

    /// A device of `adapter` with what every WebGPU device has, so what runs natively runs in a
    /// browser too: in a browser, where the request completes later.
    pub async fn of(adapter: wgpu::Adapter) -> Result<Gpu, Error> {
        let (device, queue) = (adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("valle"),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .await
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
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
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
        // Colors added, and the most of the two alphas.
        let add = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Max,
            },
        });
        let stage = |layout: &wgpu::PipelineLayout, name: &str, source: &str, fs, format, blend| {
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
                    entry_point: Some(fs),
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
        let pipeline = |layout: &wgpu::PipelineLayout, name: &str, source: &str, format, blend| {
            stage(layout, name, source, "fs", format, blend)
        };
        let rect = include_str!("../shaders/rect.wgsl");
        let gradient = include_str!("../shaders/gradient.wgsl");
        let picture = include_str!("../shaders/image.wgsl");
        let glyph = include_str!("../shaders/slug.wgsl");
        let layer = include_str!("../shaders/layer.wgsl");
        let blend = include_str!("../shaders/blend.wgsl");
        let mask = include_str!("../shaders/mask.wgsl");
        let blur = include_str!("../shaders/blur.wgsl");
        let colors = include_str!("../shaders/colors.wgsl");
        let effects = include_str!("../shaders/effects.wgsl");
        let output = include_str!("../shaders/output.wgsl");
        let yuv = include_str!("../shaders/yuv.wgsl");
        let host = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("host"),
            entries: &[entry(0, buffer(wgpu::BufferBindingType::Uniform))],
        });
        let quad = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader"),
            source: wgpu::ShaderSource::Wgsl(
                [COMMON, include_str!("../shaders/shader.wgsl")]
                    .concat()
                    .into(),
            ),
        });
        let extract = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("extract"),
            layout: Some(&layout(&[Some(&draw), Some(&host), Some(&image)])),
            vertex: wgpu::VertexState {
                module: &quad,
                entry_point: Some("vx"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &quad,
                entry_point: Some("fx"),
                compilation_options: Default::default(),
                targets: &[Some(WORK.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shader"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Gpu {
            info: adapter.get_info(),
            rects: pipeline(&drawing, "rect", rect, WORK, over),
            gradients: pipeline(&drawing, "gradient", gradient, WORK, over),
            images: pipeline(&imaging, "image", picture, WORK, over),
            glyphs: pipeline(&drawing, "slug", glyph, WORK, over),
            layers: pipeline(&imaging, "layer", layer, WORK, over),
            adds: pipeline(&imaging, "layer", layer, WORK, add),
            blends: pipeline(&blending, "blend", blend, WORK, None),
            masks: pipeline(&blending, "mask", mask, WORK, over),
            blurs: pipeline(&imaging, "blur", blur, WORK, None),
            colors: pipeline(&imaging, "colors", colors, WORK, None),
            effects: pipeline(&imaging, "effects", effects, WORK, None),
            output: pipeline(&imaging, "output", output, OUTPUT, None),
            luma: stage(&imaging, "luma", yuv, "luma", LUMA, None),
            chroma: stage(&imaging, "chroma", yuv, "chroma", CHROMA, None),
            host,
            quad,
            extract,
            sampler,
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

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Waits until the GPU has done submission `done`.
    pub fn wait(&self, done: wgpu::SubmissionIndex) -> Result<(), Error> {
        (self.device)
            .poll(wgpu::PollType::Wait {
                submission_index: Some(done),
                timeout: None,
            })
            .map(|_| ())
            .map_err(|e| format!("GPU: {e}"))
    }

    /// The format of the frames renderers make: sRGB-encoded RGBA8, which a target `present`
    /// copies them into has.
    pub const FRAME: wgpu::TextureFormat = OUTPUT;

    /// The formats of the planes `Renderer::planes` draws into: luma, and chroma at half size.
    pub const PLANES: [wgpu::TextureFormat; 2] = [LUMA, CHROMA];
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

/// A texture of `image` with its mipmap, from the first level the GPU's textures fit.
fn mipmapped(gpu: &Gpu, image: &Image) -> wgpu::Texture {
    let most = gpu.device.limits().max_texture_dimension_2d;
    let mipmap = image.mipmap();
    let levels: Vec<&Image> = (std::iter::once(image).chain(&mipmap))
        .filter(|i| i.size.iter().all(|&n| n <= most))
        .collect();
    let size = |i: &Image| wgpu::Extent3d {
        width: i.size[0],
        height: i.size[1],
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: size(levels[0]),
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, i) in levels.into_iter().enumerate() {
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &i.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * i.size[0]),
                rows_per_image: Some(i.size[1]),
            },
            size(i),
        );
    }
    texture
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
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&gpu.sampler),
                },
            ],
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
    /// Instances of one of the scene's images, which are in the gradients' store.
    Images(Range<u32>, u32),
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
    /// Adds `layer`'s colors to `target`'s, the alpha the most of the two.
    Add {
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
    /// Draws `layer` blurred along one axis into `target`, a new layer, only within `clip` (min
    /// x, min y, max x, max y px), out of which the blur has nothing to spread.
    Blur {
        target: usize,
        layer: usize,
        at: u32,
        clip: [f32; 4],
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
    /// Draws one of the frame's shaders into `target`, reading `layers` as the images its
    /// shading names: content and backdrop, or a transition's two children.
    Shader {
        target: usize,
        layers: [Option<usize>; 2],
        shading: usize,
    },
}

impl Step {
    /// The layers the step reads.
    fn reads(&self) -> [Option<usize>; 2] {
        match *self {
            Step::Draw { .. } => [None; 2],
            Step::Mask { layer, mask, .. } => [Some(layer), Some(mask)],
            Step::Shader { layers, .. } => layers,
            Step::Composite { layer, .. }
            | Step::Add { layer, .. }
            | Step::Blend { layer, .. }
            | Step::Blur { layer, .. }
            | Step::Colors { layer, .. }
            | Step::Effect { layer, .. }
            | Step::Copy { layer, .. } => [Some(layer), None],
        }
    }
}

/// Delivery sizes up to UHD 8K, including portrait and other aspect ratios.
/// This is independent of layout dimensions and of the device's own limits.
pub fn output_size([width, height]: [u32; 2]) -> Result<(), Error> {
    if width == 0
        || height == 0
        || width.max(height) > 7680
        || u64::from(width) * u64::from(height) > 7680 * 4320
    {
        return Err(
            "output size must be nonzero, at most 7680 px per side and 33177600 pixels (8K)".into(),
        );
    }
    Ok(())
}

/// Where instances are: a transform's index and the transform.
type At = (u32, Matrix);

/// Draws display lists of one size, and keeps the outlines of the glyphs they show.
pub struct Renderer<'a> {
    gpu: &'a Gpu,
    size: [u32; 2],
    canvas: [u32; 2],
    mapping: Matrix,
    frame: Surface,
    output: Surface,
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
    /// Paths' outlines, by what they are made of; those the last frame drew.
    paths: HashMap<Vec<u32>, Kept>,
    /// Whether each glyph, by face and id, has colors of its own.
    colored: HashMap<(u32, u32), bool>,
    /// Color glyphs rasterized, by face, id, px an em and the text's color: their pixels, and
    /// their top left from their origin in px an em; those the last frame used.
    colors: HashMap<ColorKey, Option<(Image, [f32; 2])>>,
    used: HashSet<ColorKey>,
    draw: Option<wgpu::BindGroup>,
    /// The textures of the images the last frames showed, and of each of this frame's images, the
    /// index of its texture.
    images: Vec<(Arc<[u8]>, Surface)>,
    shown: Vec<usize>,
    /// Layer textures: all of them, and the free ones.
    pool: Vec<Surface>,
    free: Vec<usize>,
    /// The pipelines of the shaders authors wrote, and the layouts of their own bindings, by
    /// program.
    programs: HashMap<u64, (wgpu::BindGroupLayout, wgpu::RenderPipeline)>,
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
            canvas: size,
            mapping: Matrix::IDENTITY,
            frame,
            output: Surface::new(gpu, texture(device, size, OUTPUT)),
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
            paths: HashMap::new(),
            colored: HashMap::new(),
            colors: HashMap::new(),
            used: HashSet::new(),
            draw: None,
            images: Vec::new(),
            shown: Vec::new(),
            pool: Vec::new(),
            free: Vec::new(),
            programs: HashMap::new(),
        }
    }

    /// Draws a logical `canvas` directly at `size`, contained and centred without relayout.
    /// The same-size path keeps the original transforms and render passes.
    pub fn at_size(gpu: &'a Gpu, canvas: [u32; 2], size: [u32; 2]) -> Result<Self, Error> {
        output_size(size)?;
        if canvas.contains(&0) {
            return Err("the design canvas must have nonzero dimensions".into());
        }
        let limits = gpu.device.limits();
        if size.iter().any(|&v| v > limits.max_texture_dimension_2d)
            || u64::from(row(size[0])) * u64::from(size[1]) > limits.max_buffer_size
        {
            return Err(format!(
                "output {}×{} exceeds this GPU's texture or readback limits",
                size[0], size[1]
            ));
        }
        let scale = (f64::from(size[0]) / f64::from(canvas[0]))
            .min(f64::from(size[1]) / f64::from(canvas[1]));
        let offset = [0, 1].map(|i| (f64::from(size[i]) - f64::from(canvas[i]) * scale) / 2.0);
        let mut renderer = Self::new(gpu, size[0], size[1]);
        renderer.canvas = canvas;
        renderer.mapping = Matrix::translate(offset[0], offset[1]).times(Matrix::scale(scale));
        Ok(renderer)
    }

    /// The pixels of a picture drawn by the scene, without the output's letterbox.
    fn picture_size(&self, size: [u32; 2]) -> [u32; 2] {
        let scale = self.mapping.radius_scale();
        size.map(|v| (f64::from(v) * scale).ceil().max(1.0) as u32)
    }

    /// Makes the pipeline of `program` if there is none yet: the engine's quad, then its code,
    /// over the target. Its own bindings: its uniform block, then each image and its sampler.
    fn pipeline(&mut self, program: &Program) {
        let gpu = self.gpu;
        let device = &gpu.device;
        self.programs.entry(program.key).or_insert_with(|| {
            let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty,
                count: None,
            };
            let mut entries = Vec::new();
            if !program.uniforms.is_empty() {
                entries.push(entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ));
            }
            for k in 0..program.samplers.len() as u32 {
                let texture = wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                };
                entries.push(entry(1 + 2 * k, texture));
                let sampler = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
                entries.push(entry(2 + 2 * k, sampler));
            }
            let own = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shader"),
                entries: &entries,
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&gpu.draw), Some(&gpu.host), Some(&own)],
                immediate_size: 0,
            });
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("authored"),
                source: wgpu::ShaderSource::Wgsl(program.wgsl.as_str().into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("authored"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &gpu.quad,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: WORK,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
            });
            (own, pipeline)
        });
    }

    /// The frame `items` paint: straight sRGB, 4 bytes a pixel, rows top down. `faces` are the
    /// faces their glyphs name, and `images` the images they show. First the `pictures` boxes of
    /// it draw, which it reads as images: each in turn, which may read those before it, of the
    /// canvas's size.
    pub fn render(
        &mut self,
        items: &[Item],
        pictures: &[(u32, Vec<Item>)],
        faces: &[Face],
        images: &[Image],
    ) -> Result<Vec<u8>, Error> {
        let encoder = self.encode_frame(items, pictures, faces, images)?;
        let (sent, mapped) = mpsc::channel();
        self.copy_back(encoder, move |r| {
            let _ = sent.send(r);
        });
        (self.gpu.device)
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("GPU: {e}"))?;
        mapped
            .recv()
            .map_err(|e| format!("engine bug: readback: {e}"))?
            .map_err(|e| format!("GPU readback: {e}"))?;
        self.pixels()
    }

    /// Copies the frame drawn last into memory, where `pixels` reads it once `mapped` is called:
    /// what `render` gives, for a host that cannot wait for the GPU, as a browser's.
    pub fn read_back(
        &mut self,
        mapped: impl FnOnce(Result<(), wgpu::BufferAsyncError>) + wgpu::WasmNotSend + 'static,
    ) {
        let encoder = (self.gpu.device).create_command_encoder(&Default::default());
        self.copy_back(encoder, mapped);
    }

    fn copy_back(
        &mut self,
        mut encoder: wgpu::CommandEncoder,
        mapped: impl FnOnce(Result<(), wgpu::BufferAsyncError>) + wgpu::WasmNotSend + 'static,
    ) {
        let [width, height] = self.size;
        encoder.copy_texture_to_buffer(
            self.output.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row(width)),
                    rows_per_image: Some(height),
                },
            },
            self.output.texture.size(),
        );
        self.gpu.queue.submit([encoder.finish()]);
        self.readback.map_async(wgpu::MapMode::Read, .., mapped);
    }

    /// The frame copied back: straight sRGB, 4 bytes a pixel, rows top down.
    pub fn pixels(&mut self) -> Result<Vec<u8>, Error> {
        let [width, height] = self.size;
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

    /// Draws the frame `items` paint, as `render` does, for `read_back` to copy: submitted to the
    /// GPU, not waited for.
    pub fn submit(
        &mut self,
        items: &[Item],
        pictures: &[(u32, Vec<Item>)],
        faces: &[Face],
        images: &[Image],
    ) -> Result<(), Error> {
        let encoder = self.encode_frame(items, pictures, faces, images)?;
        self.gpu.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Draws the frame `items` paint, as `render` does, into `target`, a texture of the
    /// renderer's size in `Gpu::FRAME` that it can copy into, such as a canvas's: submitted to the
    /// GPU, not waited for.
    pub fn present(
        &mut self,
        items: &[Item],
        pictures: &[(u32, Vec<Item>)],
        faces: &[Face],
        images: &[Image],
        target: &wgpu::Texture,
    ) -> Result<(), Error> {
        let mut encoder = self.encode_frame(items, pictures, faces, images)?;
        encoder.copy_texture_to_texture(
            self.output.texture.as_image_copy(),
            target.as_image_copy(),
            self.output.texture.size(),
        );
        self.gpu.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Draws the frame `items` paint, as `render` does, into an encoder's planes, BT.709 at
    /// limited range: `luma` of the renderer's size and `chroma`, Cb and Cr, at half that rounded
    /// up, in `Gpu::PLANES`. Submitted to the GPU, not waited for: the submission to wait for
    /// before the encoder reads them.
    pub fn planes(
        &mut self,
        items: &[Item],
        pictures: &[(u32, Vec<Item>)],
        faces: &[Face],
        images: &[Image],
        [luma, chroma]: [&wgpu::TextureView; 2],
    ) -> Result<wgpu::SubmissionIndex, Error> {
        let mut encoder = self.encode_frame(items, pictures, faces, images)?;
        if let Some(draw) = &self.draw {
            for (view, pipeline) in [(luma, &self.gpu.luma), (chroma, &self.gpu.chroma)] {
                let mut pass = begin(&mut encoder, view, false);
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, draw, &[]);
                pass.set_bind_group(1, &self.output.image, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        Ok(self.gpu.queue.submit([encoder.finish()]))
    }

    /// Submits the pictures boxes draw, and encodes drawing the frame into the output.
    fn encode_frame(
        &mut self,
        items: &[Item],
        pictures: &[(u32, Vec<Item>)],
        faces: &[Face],
        images: &[Image],
    ) -> Result<wgpu::CommandEncoder, Error> {
        // Color glyphs' images, kept while the last frame showed them.
        let used = std::mem::take(&mut self.used);
        self.colors.retain(|k, _| used.contains(k));
        self.paths.retain(|_, k| std::mem::take(&mut k.used));
        for (image, items) in pictures {
            let size = self.picture_size(images[*image as usize].size);
            let logical = images[*image as usize].size;
            let mapping = Matrix::affine([
                f64::from(size[0]) / f64::from(logical[0]),
                0.0,
                0.0,
                f64::from(size[1]) / f64::from(logical[1]),
                0.0,
                0.0,
            ]);
            let mut encoder = self.prepare(items, faces, images, mapping, false)?;
            let target = &self.images[self.shown[*image as usize]].1.texture;
            // In working colors, as images are read.
            encoder.copy_texture_to_texture(
                self.frame.texture.as_image_copy(),
                target.as_image_copy(),
                target.size(),
            );
            self.gpu.queue.submit([encoder.finish()]);
        }
        let letterbox = self.mapping.0[2][0] > 0.0 || self.mapping.0[2][1] > 0.0;
        self.prepare(items, faces, images, self.mapping, letterbox)
    }

    /// Encodes drawing the frame `items` paint into the output, its buffers written.
    fn prepare(
        &mut self,
        items: &[Item],
        faces: &[Face],
        images: &[Image],
        mapping: Matrix,
        letterbox: bool,
    ) -> Result<wgpu::CommandEncoder, Error> {
        // The images' textures, kept while frames show them, and color glyphs'.
        let rasters = || self.colors.values().flatten().map(|(i, _)| &i.pixels);
        self.images.retain(|(p, _)| {
            images.iter().any(|i| Arc::ptr_eq(&i.pixels, p)) || rasters().any(|r| Arc::ptr_eq(r, p))
        });
        self.shown = (images.iter())
            .map(|i| {
                match self
                    .images
                    .iter()
                    .position(|(p, _)| Arc::ptr_eq(&i.pixels, p))
                {
                    Some(k) => k,
                    None => {
                        // One without pixels is a picture a box draws.
                        let t = match i.pixels.is_empty() {
                            true => texture(&self.gpu.device, self.picture_size(i.size), WORK),
                            false => mipmapped(self.gpu, i),
                        };
                        self.images
                            .push((i.pixels.clone(), Surface::new(self.gpu, t)));
                        self.images.len() - 1
                    }
                }
            })
            .collect();
        let mut plan = Plan {
            targets: 1,
            ..Plan::default()
        };
        let identity = plan.transform(Matrix::IDENTITY);
        let identity = identity.ok_or("engine bug: no identity")?;
        // Index 0 stays the identity: layer composites are in target pixels.
        let transform = if mapping == Matrix::IDENTITY {
            identity
        } else {
            plan.transform(mapping).ok_or("invalid output transform")?
        };
        let at = (transform, mapping);
        let mut atlas = (Vec::new(), Vec::new());
        if letterbox {
            let rect = Rect {
                x: 0.0,
                y: 0.0,
                width: self.canvas[0] as f32,
                height: self.canvas[1] as f32,
            };
            let clip = Item::Clip {
                rect,
                radii: SQUARE,
                items: items.to_vec(),
            };
            self.plan(&[clip], 0, at, faces, &mut plan, &mut atlas)?;
        } else {
            self.plan(items, 0, at, faces, &mut plan, &mut atlas)?;
        }
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
        Ok(encoder)
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

    /// Plans `items` into `layer`, a new one, as `plan` does: where they reach on it, which
    /// becomes its bounds.
    fn reach(
        &mut self,
        items: &[Item],
        layer: usize,
        at: At,
        faces: &[Face],
        plan: &mut Plan,
        atlas: &mut (Vec<u32>, Vec<u32>),
    ) -> Result<Reach, Error> {
        let outer = std::mem::take(&mut plan.reach);
        self.plan(items, layer, at, faces, plan, atlas)?;
        let reach = std::mem::replace(&mut plan.reach, outer);
        plan.bounds[layer] = reach;
        Ok(reach)
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
                Item::Rect { rect, .. }
                | Item::Ring { rect, .. }
                | Item::Gradient { rect, .. }
                | Item::Image { rect, .. }
                | Item::Backdrop { rect, .. } => {
                    plan.reach = plan.reach.union(Reach::of(matrix, *rect));
                }
                Item::Shader { rect, outset, .. } => {
                    let r = Rect {
                        x: rect.x - outset,
                        y: rect.y - outset,
                        width: rect.width + 2.0 * outset,
                        height: rect.height + 2.0 * outset,
                    };
                    plan.reach = plan.reach.union(Reach::of(matrix, r));
                }
                _ => {}
            }
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
                    let (shape, [[a, b], [c, d]], more) = match ramp.geometry {
                        Geometry::Line(ends) => (0, ends, [0.0; 2]),
                        Geometry::Ellipse(center, radii) => (1, [center, radii], [0.0; 2]),
                        Geometry::Turn(center, from) => (2, [center, [from, 0.0]], [0.0; 2]),
                        Geometry::Focal {
                            focus,
                            focus_radius,
                            center,
                            radius,
                        } => (3, [focus, center], [focus_radius, radius]),
                    };
                    let words = (xywh(*rect).into_iter())
                        .chain(radii.iter().flatten().copied())
                        .chain(xywh(tiles.first))
                        .chain(tiles.step.map(|s| s.unwrap_or(0.0)))
                        .chain(more)
                        .chain([a, b, c, d]);
                    plan.gradients.extend(words.map(f32::to_bits));
                    let space = match ramp.space {
                        _ if !ramp.premultiplied => 4,
                        Space::Srgb => 0,
                        Space::SrgbLinear => 1,
                        Space::Oklab => 2,
                        Space::Oklch(_) => 3,
                        Space::Lab => 5,
                        Space::Lch(_) => 6,
                    };
                    let count = ramp.segments.len() as u32;
                    let spread = match ramp.spread {
                        Spread::Pad => 0,
                        Spread::Repeat => 1,
                        Spread::Reflect => 2,
                    };
                    plan.gradients
                        .extend([spread, space, shape, transform, first, count, 0, 0]);
                    extend(runs, Run::Gradients(i..i + 1));
                }
                Item::Image {
                    rect,
                    radii,
                    tiles,
                    image,
                    pixelated,
                } => {
                    let i = (plan.gradients.len() / GRADIENT) as u32;
                    let words = (xywh(*rect).into_iter())
                        .chain(radii.iter().flatten().copied())
                        .chain(xywh(tiles.first))
                        .chain(tiles.step.map(|s| s.unwrap_or(0.0)))
                        .chain([0.0; 6]);
                    plan.gradients.extend(words.map(f32::to_bits));
                    plan.gradients
                        .extend([*pixelated as u32, 0, 0, transform, 0, 0, 0, 0]);
                    extend(runs, Run::Images(i..i + 1, *image));
                }
                Item::Glyphs {
                    face,
                    size,
                    color,
                    stroke,
                    glyphs,
                } => {
                    for &(id, [x, y]) in glyphs {
                        // A color glyph as an image of it at the size it shows at, as much as the
                        // text's color is opaque; its stroke is of its outline.
                        if *stroke == 0.0 && self.is_colored(*face, id, faces)? {
                            let ppem = (size * matrix.radius_scale() as f32)
                                .ceil()
                                .clamp(1.0, 2048.0);
                            let Some((image, [left, top], [w, h])) =
                                self.color(*face, id, ppem, *color, faces)?
                            else {
                                continue;
                            };
                            let k = size / ppem;
                            let rect = Rect {
                                x: x + left * k,
                                y: y + top * k,
                                width: w * k,
                                height: h * k,
                            };
                            let tiles = Tiles {
                                first: rect,
                                step: [None, None],
                            };
                            let image = Item::Image {
                                rect,
                                radii: SQUARE,
                                tiles,
                                image,
                                pixelated: false,
                            };
                            let item = match color[3] {
                                1.0 => image,
                                opacity => Item::Group {
                                    opacity,
                                    blend: Blend::Normal,
                                    items: vec![image],
                                },
                            };
                            self.draw(&[item], target, at, faces, plan, atlas, runs)?;
                            continue;
                        }
                        let Some(o) = self.outline(*face, id, stroke / size, faces, atlas)? else {
                            continue;
                        };
                        let s = size / o.em;
                        let [x0, y0, x1, y1] = o.bounds;
                        let r = Rect {
                            x: x + x0 * s,
                            y: y - y1 * s,
                            width: (x1 - x0) * s,
                            height: (y1 - y0) * s,
                        };
                        plan.reach = plan.reach.union(Reach::of(matrix, r).grown(1.0));
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
                        plan.glyphs.extend(o.bounds.map(f32::to_bits));
                        extend(runs, Run::Glyphs(i..i + 1));
                    }
                }
                Item::Path {
                    path,
                    even_odd,
                    color,
                } => {
                    // An outline of 1 px a unit, y up, as glyph outlines are, kept while frames
                    // draw the same path.
                    let tolerance = TOLERANCE / (matrix.radius_scale() as f32).max(1e-6);
                    let key = path_key(path, tolerance);
                    let kept = match self.paths.get_mut(&key) {
                        Some(kept) => kept,
                        None => {
                            let contours: Vec<Vec<Curve>> = (path.quads(tolerance).into_iter())
                                .map(|c| c.into_iter().map(|q| q.map(|[x, y]| [x, -y])).collect())
                                .collect();
                            let kept = Kept {
                                outline: slug::encode(1.0, &contours),
                                runs: None,
                                used: false,
                            };
                            self.paths.entry(key).or_insert(kept)
                        }
                    };
                    kept.used = true;
                    let o = &kept.outline;
                    if o.curves.is_empty() {
                        continue;
                    }
                    let [x0, y0, x1, y1] = o.bounds;
                    let r = Rect {
                        x: x0,
                        y: -y1,
                        width: x1 - x0,
                        height: y1 - y0,
                    };
                    let on = Reach::of(matrix, r);
                    plan.reach = plan.reach.union(on.grown(1.0));
                    let (curves, lists) = &mut plan.paths;
                    let first = [(curves.len() / 4) as u32, lists.len() as u32];
                    curves.extend(o.curves.iter().flatten().map(|v| v.to_bits()));
                    lists.extend(&o.lists);
                    // A path large on the target draws in runs of cells about `CELL` px across,
                    // where the transform keeps its scale everywhere.
                    let cells = match (on.0, matrix.least_scale()) {
                        (Some([x0, y0, x1, y1]), Some(least)) if (x1 - x0).max(y1 - y0) > CELL => {
                            let least = least as f32;
                            let across =
                                |n: f32| ((n * least / CELL).ceil() as usize).clamp(1, 256);
                            Some(([across(r.width), across(r.height)], 1.5 / least))
                        }
                        _ => None,
                    };
                    let whole = [slug::Run {
                        rect: o.bounds,
                        solid: false,
                    }];
                    let parts: &[slug::Run] = match cells {
                        Some((cells, margin)) => {
                            let at = (cells, margin.to_bits(), *even_odd);
                            if kept.runs.as_ref().is_none_or(|(k, _)| *k != at) {
                                let runs = o.runs(cells, margin, *even_odd);
                                kept.runs = Some((at, runs));
                            }
                            kept.runs.as_ref().map_or(&[], |(_, r)| r)
                        }
                        None => &whole,
                    };
                    let i = (plan.glyphs.len() / GLYPH) as u32;
                    let [u, v] = o.band_scale();
                    let bands = o.bands[0] | o.bands[1] << 16;
                    for part in parts {
                        let flags = u8::from(*even_odd) | u8::from(part.solid) << 1;
                        let floats = [0.0, 0.0, 1.0, f32::from(flags)]
                            .into_iter()
                            .chain(o.bounds);
                        let floats = floats.chain([u, v, 0.0, 0.0]);
                        plan.glyphs.extend(floats.map(f32::to_bits));
                        plan.placed.push(plan.glyphs.len());
                        plan.glyphs.extend([first[0], first[1], bands, transform]);
                        plan.glyphs.extend(color.map(f32::to_bits));
                        plan.glyphs.extend(part.rect.map(f32::to_bits));
                    }
                    let n = parts.len() as u32;
                    extend(runs, Run::Glyphs(i..i + n));
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
                    let outer = std::mem::take(&mut plan.reach);
                    self.plan(items, layer, at, faces, plan, atlas)?;
                    plan.seen = around;
                    let mut reach = std::mem::replace(&mut plan.reach, outer);
                    plan.bounds[layer] = reach;
                    let layer = plan.filtered(layer, filters, matrix, None, &mut reach);
                    plan.reach = plan.reach.union(reach);
                    plan.composite(target, layer);
                }
                Item::Shader {
                    program,
                    block,
                    inputs,
                    rect,
                    outset,
                    layers,
                } => {
                    if rect.width <= 0.0 || rect.height <= 0.0 {
                        continue;
                    }
                    if !runs.is_empty() {
                        let runs = std::mem::take(runs);
                        plan.steps.push(Step::Draw { target, runs });
                    }
                    let names = match layers.first() {
                        Some(&(name, _)) if name != "content" => transition::IMAGES,
                        _ => ["content", "backdrop"],
                    };
                    // Each layer in one of its own, if the shader reads it.
                    let mut reads = [None; 2];
                    for &(name, ref items) in layers {
                        let Some(k) = names.iter().position(|n| *n == name) else {
                            continue;
                        };
                        if !program.reads(name) {
                            continue;
                        }
                        let layer = plan.layer();
                        let mut inside = plan.seen.clone();
                        inside.here = Under {
                            root: layer,
                            open: Vec::new(),
                        };
                        let around = std::mem::replace(&mut plan.seen, inside);
                        let reach = self.reach(items, layer, at, faces, plan, atlas)?;
                        plan.reach = plan.reach.union(reach);
                        plan.seen = around;
                        reads[k] = Some(layer);
                    }
                    // What is under the box's group, as a backdrop filter reads it.
                    if names[1] == "backdrop" && program.reads("backdrop") {
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
                        reads[1] = Some(copy);
                    }
                    // Its images have as many pixels as the box covers on the target.
                    let scale = matrix.radius_scale() as f32;
                    let [w, h] = [rect.width, rect.height].map(|v| (v * scale).ceil().max(1.0));
                    if reads.iter().any(Option::is_some)
                        && [w, h].iter().any(|&v| {
                            !v.is_finite()
                                || v > self.gpu.device.limits().max_texture_dimension_2d as f32
                        })
                    {
                        return Err("shader input exceeds this GPU's texture limit".into());
                    }
                    let mut host = [0.0; 16];
                    host[..8].copy_from_slice(&[
                        rect.width,
                        rect.height,
                        *outset,
                        transform as f32,
                        w,
                        h,
                        rect.x,
                        rect.y,
                    ]);
                    plan.shadings.push((
                        program.clone(),
                        host,
                        block.clone(),
                        inputs.clone(),
                        names,
                    ));
                    let shading = plan.shadings.len() - 1;
                    plan.steps.push(Step::Shader {
                        target,
                        layers: reads,
                        shading,
                    });
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
                    let mut all = Reach::ALL;
                    let layer = plan.filtered(copy, filters, matrix, Some(bounds), &mut all);
                    // Only what is in the box shows.
                    plan.bounds[layer] = Reach::of(matrix, *rect).grown(1.0);
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
                    let reach = self.reach(items, layer, at, faces, plan, atlas)?;
                    plan.reach = plan.reach.union(reach);
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
                        let reach = self.reach(items, into, at, faces, plan, atlas)?;
                        // What shows is in the items' layer.
                        if into == layer {
                            plan.reach = plan.reach.union(reach);
                        }
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
                    let outer = std::mem::take(&mut plan.reach);
                    self.plan(items, layer, at, faces, plan, atlas)?;
                    plan.seen = around;
                    let mut reach = std::mem::replace(&mut plan.reach, outer);
                    plan.bounds[layer] = reach;
                    let sigma = sigma * matrix.radius_scale() as f32;
                    let first = plan.targets;
                    let layer = plan.blurred(layer, sigma, None, &mut reach);
                    for t in first..plan.targets {
                        plan.bounds[t] = reach;
                    }
                    plan.reach = plan.reach.union(reach);
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
    /// Whether glyph `id` of face `face` has colors of its own.
    fn is_colored(&mut self, face: u32, id: u32, faces: &[Face]) -> Result<bool, Error> {
        if let Some(&colored) = self.colored.get(&(face, id)) {
            return Ok(colored);
        }
        let f = faces
            .get(face as usize)
            .ok_or("engine bug: a glyph names a face that is not there")?;
        let colored = color::colored(f, id)?;
        self.colored.insert((face, id), colored);
        Ok(colored)
    }

    /// Color glyph `id` of face `face` at `ppem` px an em, in text of `color`: the index this
    /// frame's images give it, its top left from its origin in px an em, and its size in pixels;
    /// nothing if it paints nothing.
    fn color(
        &mut self,
        face: u32,
        id: u32,
        ppem: f32,
        color: [f32; 4],
        faces: &[Face],
    ) -> Result<Option<Shown>, Error> {
        let f = faces
            .get(face as usize)
            .ok_or("engine bug: a glyph names a face that is not there")?;
        // Where it paints in the text's color, the color opaque, in 8 bits.
        let foreground = [0, 1, 2].map(|c| (color[c].clamp(0.0, 1.0) * 255.0).round() as u8);
        let key = (face, id, ppem.to_bits(), foreground);
        self.used.insert(key);
        let raster =
            (self.colors.entry(key)).or_insert_with(|| color::raster(f, id, ppem, foreground));
        let Some((image, at)) = raster else {
            return Ok(None);
        };
        let (pixels, size, at) = (image.pixels.clone(), image.size, *at);
        let k = match self
            .images
            .iter()
            .position(|(p, _)| Arc::ptr_eq(p, &pixels))
        {
            Some(k) => k,
            None => {
                let t = mipmapped(self.gpu, image);
                self.images.push((pixels, Surface::new(self.gpu, t)));
                self.images.len() - 1
            }
        };
        let shown = match self.shown.iter().position(|&s| s == k) {
            Some(i) => i,
            None => {
                self.shown.push(k);
                self.shown.len() - 1
            }
        };
        Ok(Some((shown as u32, at, size.map(|n| n as f32))))
    }

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
        for (program, ..) in &plan.shadings {
            self.pipeline(program);
        }
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
            if let Step::Shader {
                target,
                layers,
                shading,
            } = *step
            {
                let (program, host, block, inputs, names) = &plan.shadings[shading];
                let device = &gpu.device;
                let uniform = |label, contents: &[u8]| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents,
                        usage: wgpu::BufferUsages::UNIFORM,
                    })
                };
                let words: Vec<u32> = host.iter().map(|v| v.to_bits()).collect();
                let host = uniform("host", &bytes(&words));
                let hosted = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &gpu.host,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: host.as_entire_binding(),
                    }],
                });
                // Its images: each of its layers into a texture of the box's size, clear if
                // nothing was drawn into it; an input of its own size.
                let size = [f32::from_bits(words[4]), f32::from_bits(words[5])].map(|v| v as u32);
                let mut views = Vec::new();
                for name in &program.samplers {
                    let k = names.iter().position(|n| *n == &**name);
                    let (image, size, hosted) = match k {
                        Some(k) => {
                            let s = layers[k].and_then(|l| surfaces[l]);
                            (s.map(|s| &self.pool[s]), size, hosted.clone())
                        }
                        None => {
                            let i = (inputs.iter()).find_map(|(n, i)| (n == name).then_some(*i));
                            let shown = i.map(|i| &self.images[self.shown[i as usize]].1);
                            let size = shown.map_or([1, 1], |s| {
                                let e = s.texture.size();
                                [e.width, e.height]
                            });
                            let mut words = [0.0f32; 16];
                            words[4..6].copy_from_slice(&size.map(|v| v as f32));
                            words[8] = 1.0;
                            let words: Vec<u32> = words.iter().map(|v| v.to_bits()).collect();
                            let own = uniform("input", &bytes(&words));
                            let hosted = device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: None,
                                layout: &gpu.host,
                                entries: &[wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: own.as_entire_binding(),
                                }],
                            });
                            (shown, size, hosted)
                        }
                    };
                    let view = texture(device, size, WORK).create_view(&Default::default());
                    let mut pass = begin(encoder, &view, false);
                    if let Some(image) = image {
                        pass.set_pipeline(&gpu.extract);
                        pass.set_bind_group(0, draw, &[]);
                        pass.set_bind_group(1, &hosted, &[]);
                        pass.set_bind_group(2, &image.image, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    drop(pass);
                    views.push(view);
                }
                let block = uniform("uniforms", block);
                let mut entries = Vec::new();
                if !program.uniforms.is_empty() {
                    entries.push(wgpu::BindGroupEntry {
                        binding: 0,
                        resource: block.as_entire_binding(),
                    });
                }
                for (k, view) in views.iter().enumerate() {
                    let k = k as u32;
                    entries.push(wgpu::BindGroupEntry {
                        binding: 1 + 2 * k,
                        resource: wgpu::BindingResource::TextureView(view),
                    });
                    entries.push(wgpu::BindGroupEntry {
                        binding: 2 + 2 * k,
                        resource: wgpu::BindingResource::Sampler(&gpu.sampler),
                    });
                }
                let (own, pipeline) = &self.programs[&program.key];
                let own = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: own,
                    entries: &entries,
                });
                let surface = match target {
                    0 => None,
                    t => Some(*surfaces[t].get_or_insert_with(|| {
                        take(gpu, &mut self.pool, &mut self.free, self.size)
                    })),
                };
                let view = &surface.map_or(&self.frame, |s| &self.pool[s]).view;
                let mut pass = begin(encoder, view, std::mem::replace(&mut started[target], true));
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, draw, &[]);
                pass.set_bind_group(1, &hosted, &[]);
                pass.set_bind_group(2, &own, &[]);
                pass.draw(0..4, 0..1);
                drop(pass);
                for l in layers.into_iter().flatten() {
                    if last[l] == k {
                        self.free.extend(surfaces[l]);
                    }
                }
                continue;
            }
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
                | Step::Add { target, .. }
                | Step::Blend { target, .. }
                | Step::Mask { target, .. }
                | Step::Blur { target, .. }
                | Step::Colors { target, .. }
                | Step::Effect { target, .. } => target,
                Step::Copy { .. } | Step::Shader { .. } => unreachable!("done above"),
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
                Step::Copy { .. } | Step::Shader { .. } => {}
                Step::Mask { .. } => {
                    let [Some(s), Some(m)] = [layer, mask] else {
                        unreachable!("a mask step reads two layers");
                    };
                    pass.set_pipeline(&gpu.masks);
                    pass.set_bind_group(1, &self.pool[s].image, &[]);
                    pass.set_bind_group(2, &self.pool[m].image, &[]);
                    let [l, m] = step
                        .reads()
                        .map(|l| l.map_or(Reach::ALL, |l| plan.bound(l)));
                    if let Some([x, y, w, h]) = l.meet(m).pixels(self.size) {
                        pass.set_scissor_rect(x, y, w, h);
                        pass.draw(0..3, 0..1);
                    }
                }
                Step::Draw { runs, .. } => {
                    for run in runs {
                        let (pipeline, range) = match run {
                            Run::Rects(r) => (&gpu.rects, r),
                            Run::Gradients(r) => (&gpu.gradients, r),
                            Run::Images(r, image) => {
                                let shown = &self.images[self.shown[*image as usize]].1;
                                pass.set_bind_group(1, &shown.image, &[]);
                                (&gpu.images, r)
                            }
                            Run::Glyphs(r) => (&gpu.glyphs, r),
                        };
                        pass.set_pipeline(pipeline);
                        pass.draw(0..4, range.clone());
                    }
                }
                Step::Composite { at, .. }
                | Step::Add { at, .. }
                | Step::Blend { at, .. }
                | Step::Blur { at, .. }
                | Step::Colors { at, .. }
                | Step::Effect { at, .. } => {
                    let s = layer.expect("a composited layer has a surface");
                    pass.set_pipeline(match step {
                        Step::Add { .. } => &gpu.adds,
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
                    // Only where it may change anything: where the layer has anything, or what
                    // the step makes of it does.
                    let reach = match *step {
                        Step::Blur { clip, .. } => Reach(Some(clip)),
                        Step::Colors { target, .. } | Step::Effect { target, .. } => {
                            plan.bound(target)
                        }
                        _ => step.reads()[0].map_or(Reach::ALL, |l| plan.bound(l)),
                    };
                    if let Some([x, y, w, h]) = reach.pixels(self.size) {
                        pass.set_scissor_rect(x, y, w, h);
                        pass.draw(0..3, *at..*at + 1);
                    }
                }
            }
            drop(pass);
            self.free.extend(done.into_iter().flatten().chain(under));
        }
        if !started[0] {
            drop(begin(encoder, &self.frame.view, false));
        }
        let mut pass = begin(encoder, &self.output.view, false);
        pass.set_pipeline(&gpu.output);
        pass.set_bind_group(0, draw, &[]);
        pass.set_bind_group(1, &self.frame.image, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A color glyph rasterized: its face and id, its px an em, and the text's color.
type ColorKey = (u32, u32, u32, [u8; 3]);

/// A color glyph's image as a frame shows it: its index in the frame's images, its top left from
/// its origin in px an em, and its size in pixels.
type Shown = (u32, [f32; 2], [f32; 2]);

/// Whether color matrices, applied in turn as `Step::Colors` applies them (each the factors of
/// red, green, blue and alpha in each of them, then their offsets), keep transparent
/// transparent.
fn keeps_transparent(matrices: &[[f32; 20]]) -> bool {
    let mut u = [0.0f32; 4];
    for m in matrices {
        u = std::array::from_fn(|r| {
            let row = &m[4 * r..4 * r + 4];
            (row.iter().zip(u).map(|(a, b)| a * b).sum::<f32>() + m[16 + r]).clamp(0.0, 1.0)
        });
    }
    u[3] <= 0.0
}

/// About how many target px across a large path's cells are.
const CELL: f32 = 32.0;

/// A path's outline, kept while frames draw it, and its runs of cells for the last grid asked.
struct Kept {
    outline: slug::Outline,
    runs: Option<(Grid, Vec<slug::Run>)>,
    used: bool,
}

/// A grid of cells over a path: columns and rows, the bits of the margin, and whether the path
/// fills by even-odd.
type Grid = ([usize; 2], u32, bool);

/// What a path's outline is made of, exactly: its segments and the tolerance it is flattened to.
fn path_key(path: &Path, tolerance: f32) -> Vec<u32> {
    let mut key = path.key();
    key.push(tolerance.to_bits());
    key
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
        (Some(Run::Images(r, a)), Run::Images(n, b)) if a == b && r.end == n.start => {
            r.end = n.end;
        }
        _ => runs.push(next),
    }
}
