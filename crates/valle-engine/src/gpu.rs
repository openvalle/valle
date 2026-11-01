//! The GPU: display lists drawn with wgpu into a frame of f16 working colors (premultiplied
//! sRGB-encoded values, composited as browsers composite them), then out as 8-bit sRGB with
//! straight alpha.
//!
//! A frame is planned first: its rectangles, glyphs and layers become instance data, uploaded
//! once, and steps, each a render pass that draws runs of instances into the frame or a layer,
//! or composites a layer onto what is under it: a group's at its opacity, a clip's inside its
//! rounded rectangle. A layer takes a texture from a pool when a step
//! first draws into it and gives it back once composited. Glyph outlines are encoded once and
//! kept in two buffers, the atlas, for the renderer's life.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};

use crate::layout::Rect;
use crate::paint::{Item, SQUARE, inner};
use crate::slug;
use crate::template::Error;
use crate::text::Face;

const WORK: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUTPUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const COMMON: &str = include_str!("../shaders/common.wgsl");

/// u32 words per rectangle, glyph and layer instance.
const RECT: usize = 40;
const GLYPH: usize = 20;
const LAYER: usize = 16;

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
    glyphs: wgpu::RenderPipeline,
    layers: wgpu::RenderPipeline,
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
        let glyph = include_str!("../shaders/slug.wgsl");
        let layer = include_str!("../shaders/layer.wgsl");
        let output = include_str!("../shaders/output.wgsl");
        Ok(Gpu {
            info: adapter.get_info(),
            rects: pipeline(&drawing, "rect", rect, WORK, over),
            glyphs: pipeline(&drawing, "slug", glyph, WORK, over),
            layers: pipeline(&imaging, "layer", layer, WORK, over),
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
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// A texture to draw into, and its bind group for reading it as an image.
struct Surface {
    view: wgpu::TextureView,
    image: wgpu::BindGroup,
}

impl Surface {
    fn new(gpu: &Gpu, texture: &wgpu::Texture) -> Surface {
        let view = texture.create_view(&Default::default());
        let image = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &gpu.image,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Surface { view, image }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Run {
    Rects(Range<u32>),
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
}

#[derive(Default)]
struct Plan {
    rects: Vec<u32>,
    glyphs: Vec<u32>,
    layers: Vec<u32>,
    steps: Vec<Step>,
    targets: usize,
}

/// Draws display lists of one size, and keeps the outlines of the glyphs they show.
pub struct Renderer<'a> {
    gpu: &'a Gpu,
    size: [u32; 2],
    frame: Surface,
    output: wgpu::Texture,
    readback: wgpu::Buffer,
    screen: wgpu::Buffer,
    rects: Store,
    glyphs: Store,
    layers: Store,
    curves: Store,
    lists: Store,
    /// Words in `curves` (4 a texel) and `lists` so far.
    atlas_len: [u64; 2],
    /// By face and glyph; `None` for a glyph without curves.
    outlines: HashMap<(u32, u32), Option<Placed>>,
    draw: Option<wgpu::BindGroup>,
    /// Layer textures: all of them, and the free ones.
    pool: Vec<Surface>,
    free: Vec<usize>,
}

impl<'a> Renderer<'a> {
    pub fn new(gpu: &'a Gpu, width: u32, height: u32) -> Renderer<'a> {
        let device = &gpu.device;
        let size = [width, height];
        let frame = Surface::new(gpu, &texture(device, size, WORK));
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
            rects: Store::new(device, "rects"),
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
        let mut atlas = (Vec::new(), Vec::new());
        self.plan(items, 0, faces, &mut plan, &mut atlas)?;
        let gpu = self.gpu;
        let device = &gpu.device;
        let mut encoder = device.create_command_encoder(&Default::default());

        let [curves_len, lists_len] = self.atlas_len;
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
        self.atlas_len = [
            curves_len + atlas.0.len() as u64,
            lists_len + atlas.1.len() as u64,
        ];
        for (store, words) in [
            (&mut self.rects, &plan.rects),
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

    fn plan(
        &mut self,
        items: &[Item],
        target: usize,
        faces: &[Face],
        plan: &mut Plan,
        atlas: &mut (Vec<u32>, Vec<u32>),
    ) -> Result<(), Error> {
        let mut runs: Vec<Run> = Vec::new();
        for item in items {
            match item {
                Item::Rect { .. } | Item::Border { .. } => {
                    // A fill is a ring whose inside has no size.
                    let (rect, radii, inside, inner_radii, colors) = match item {
                        Item::Rect { rect, radii, color } => {
                            (rect, radii, Rect::default(), SQUARE, [*color; 4])
                        }
                        Item::Border {
                            rect,
                            radii,
                            widths,
                            colors,
                        } => {
                            let (inside, inner_radii) = inner(*rect, *radii, *widths);
                            (rect, radii, inside, inner_radii, *colors)
                        }
                        _ => unreachable!("a rectangle or a border"),
                    };
                    let i = (plan.rects.len() / RECT) as u32;
                    let words = (xywh(*rect).into_iter())
                        .chain(radii.iter().flatten().copied())
                        .chain(xywh(inside))
                        .chain(inner_radii.iter().flatten().copied())
                        .chain(colors.into_iter().flatten());
                    plan.rects.extend(words.map(f32::to_bits));
                    extend(&mut runs, Run::Rects(i..i + 1));
                }
                Item::Glyphs {
                    face,
                    size,
                    color,
                    glyphs,
                } => {
                    for &(id, [x, y]) in glyphs {
                        let Some(o) = self.outline(*face, id, faces, atlas)? else {
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
                        plan.glyphs
                            .extend([o.at[0], o.at[1], o.bands[0], o.bands[1]]);
                        plan.glyphs.extend(color.map(f32::to_bits));
                        extend(&mut runs, Run::Glyphs(i..i + 1));
                    }
                }
                Item::Group { .. } | Item::Clip { .. } => {
                    if !runs.is_empty() {
                        let runs = std::mem::take(&mut runs);
                        plan.steps.push(Step::Draw { target, runs });
                    }
                    // A group shows all of its layer, a clip the part in its rectangle.
                    let [width, height] = self.size.map(|s| s as f32);
                    let (items, opacity, clip, radii) = match item {
                        Item::Group { opacity, items } => (
                            items,
                            *opacity,
                            [-1.0, -1.0, width + 2.0, height + 2.0],
                            SQUARE,
                        ),
                        Item::Clip { rect, radii, items } => (items, 1.0, xywh(*rect), *radii),
                        _ => unreachable!("a group or a clip"),
                    };
                    let layer = plan.targets;
                    plan.targets += 1;
                    self.plan(items, layer, faces, plan, atlas)?;
                    let at = (plan.layers.len() / LAYER) as u32;
                    let words = [opacity, 0.0, 0.0, 0.0]
                        .into_iter()
                        .chain(clip)
                        .chain(radii.into_iter().flatten());
                    plan.layers.extend(words.map(f32::to_bits));
                    plan.steps.push(Step::Composite { target, layer, at });
                }
            }
        }
        if !runs.is_empty() {
            plan.steps.push(Step::Draw { target, runs });
        }
        Ok(())
    }

    /// Where glyph `id` of face `face` is in the atlas, encoding it into `atlas` (curve words,
    /// list words) if it is new.
    fn outline(
        &mut self,
        face: u32,
        id: u32,
        faces: &[Face],
        atlas: &mut (Vec<u32>, Vec<u32>),
    ) -> Result<Option<Placed>, Error> {
        if let Some(placed) = self.outlines.get(&(face, id)) {
            return Ok(*placed);
        }
        let f = faces
            .get(face as usize)
            .ok_or("engine bug: a glyph names a face that is not there")?;
        let o = slug::outline(f, id)?;
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
        self.outlines.insert((face, id), placed);
        Ok(placed)
    }

    fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, plan: &Plan) {
        let gpu = self.gpu;
        let Some(draw) = &self.draw else {
            return;
        };
        // Each layer's pool texture once it has one, and whether a pass drew into each target.
        let mut surfaces: Vec<Option<usize>> = vec![None; plan.targets];
        let mut started = vec![false; plan.targets];
        for step in &plan.steps {
            let (target, layer) = match step {
                Step::Draw { target, .. } => (*target, None),
                Step::Composite { target, layer, .. } => match surfaces[*layer] {
                    Some(s) => (*target, Some(s)),
                    // Nothing was drawn into the layer.
                    None => continue,
                },
            };
            let view = match target {
                0 => &self.frame.view,
                t => {
                    let s = *surfaces[t].get_or_insert_with(|| match self.free.pop() {
                        Some(s) => s,
                        None => {
                            let t = texture(&gpu.device, self.size, WORK);
                            self.pool.push(Surface::new(gpu, &t));
                            self.pool.len() - 1
                        }
                    });
                    &self.pool[s].view
                }
            };
            let mut pass = begin(encoder, view, std::mem::replace(&mut started[target], true));
            pass.set_bind_group(0, draw, &[]);
            match step {
                Step::Draw { runs, .. } => {
                    for run in runs {
                        let (pipeline, range) = match run {
                            Run::Rects(r) => (&gpu.rects, r),
                            Run::Glyphs(r) => (&gpu.glyphs, r),
                        };
                        pass.set_pipeline(pipeline);
                        pass.draw(0..4, range.clone());
                    }
                }
                Step::Composite { at, .. } => {
                    let s = layer.expect("a composited layer has a surface");
                    pass.set_pipeline(&gpu.layers);
                    pass.set_bind_group(1, &self.pool[s].image, &[]);
                    pass.draw(0..3, *at..*at + 1);
                }
            }
            drop(pass);
            if let Some(s) = layer {
                self.free.push(s);
            }
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

/// A rectangle as shaders take it: x, y, width and height.
fn xywh(r: Rect) -> [f32; 4] {
    [r.x, r.y, r.width, r.height]
}

/// Appends instance `next` to the last run when it is of the same kind and follows it.
fn extend(runs: &mut Vec<Run>, next: Run) {
    match (runs.last_mut(), &next) {
        (Some(Run::Rects(r)), Run::Rects(n)) | (Some(Run::Glyphs(r)), Run::Glyphs(n))
            if r.end == n.start =>
        {
            r.end = n.end;
        }
        _ => runs.push(next),
    }
}
