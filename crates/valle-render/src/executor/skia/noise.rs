//! Noise displacement on Raster: `draw_noise_displacement` fills a rect with the shared runtime
//! kernel over Skia's Perlin noise; on aarch64, `draw` computes the kernel's bytes pixel by pixel
//! instead of through its raster pipeline program, and `NoiseFields` keeps the noise it computes
//! for the draws of later frames.

use std::sync::Mutex;

use skia_safe::{IRect, Image, Surface};

use super::draw::DrawError;

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
pub(crate) use fused::NoiseFields;

/// The Perlin noise the kernel reads, as Skia's noise shader is made from it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Noise {
    pub(super) frequency: [f32; 2],
    pub(super) octaves: u8,
    pub(super) seed: u32,
    pub(super) turbulence: bool,
}

/// The kernel with `uniforms` over `rect`, reading the `source` subset of `input` placed at canvas
/// `origin`, as its draw under the Src blend writes it. The noise of its pixels comes from
/// `fields`, or goes there for later draws, where they keep it. Returns `false`, drawing nothing,
/// where the kernel's own draw stays.
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    surface: &mut Surface,
    input: &Image,
    source: IRect,
    origin: [i32; 2],
    rect: IRect,
    uniforms: &[f32; 15],
    noise: Noise,
    fields: Option<&Mutex<NoiseFields>>,
) -> Result<bool, DrawError> {
    fused::draw(
        surface, input, source, origin, rect, uniforms, noise, fields,
    )
}

/// The kernel repeats the instructions of Skia's raster pipeline on aarch64; elsewhere the
/// kernel's own draw stays.
#[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    _surface: &mut Surface,
    _input: &Image,
    _source: IRect,
    _origin: [i32; 2],
    _rect: IRect,
    _uniforms: &[f32; 15],
    _noise: Noise,
    _fields: Option<&Mutex<NoiseFields>>,
) -> Result<bool, DrawError> {
    Ok(false)
}

/// Elsewhere the kernel's own draw computes the noise, and nothing keeps it.
#[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
#[derive(Debug, Default)]
pub(crate) struct NoiseFields;

#[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
impl NoiseFields {
    pub(crate) fn set_byte_limit(&mut self, _bytes: usize) {}

    pub(crate) fn clear(&mut self) {}

    #[cfg(test)]
    pub(crate) fn rects(&self) -> Vec<IRect> {
        Vec::new()
    }
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod fused {
    use std::{
        arch::aarch64::*,
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use skia_safe::{
        AlphaType, ColorType, IRect, Image, ImageInfo, Surface, surface::ContentChangeMode,
    };
    use valle_engine::{
        compositor::bloom::fill_image_rows,
        motion::memory::{MemoryKind, MemoryLease, reserve_retained},
    };

    use super::{
        super::draw::{DrawError, rows_clip},
        Noise,
    };

    /// Pixels per vector.
    const LANES: usize = 4;

    /// The noise channels the kernel reads: red and green displace, alpha unpremultiplies them.
    const CHANNELS: [usize; 3] = [0, 1, 3];

    /// The bytes of the fields one executor keeps, unless its budget sets them.
    const FIELD_BYTES: usize = 64 * 1024 * 1024;

    /// The keys of draws that kept no field, remembered so that a second draw keeps one.
    const SEEN_KEYS: usize = 64;

    /// The noise of earlier draws, kept for later draws that place the same noise on the same
    /// pixels: per pixel, the unpremultiplied red and green of the noise, clamped to [0, 1], from
    /// which the kernel computes the displacement. A key keeps its field from its second draw on,
    /// so that noise moving every frame never takes the bytes. Fields hold exactly what a draw
    /// computes, so clearing them changes no pixel.
    pub(crate) struct NoiseFields {
        /// The least recently used first.
        fields: VecDeque<Arc<Field>>,
        /// The oldest first.
        seen: VecDeque<FieldKey>,
        bytes: usize,
        byte_limit: usize,
    }

    impl Default for NoiseFields {
        fn default() -> Self {
            Self {
                fields: VecDeque::new(),
                seen: VecDeque::new(),
                bytes: 0,
                byte_limit: FIELD_BYTES,
            }
        }
    }

    impl std::fmt::Debug for NoiseFields {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("NoiseFields")
                .field("entries", &self.fields.len())
                .field("bytes", &self.bytes)
                .finish()
        }
    }

    impl NoiseFields {
        pub(crate) fn set_byte_limit(&mut self, bytes: usize) {
            self.clear();
            self.byte_limit = bytes;
        }

        pub(crate) fn clear(&mut self) {
            self.fields.clear();
            self.seen.clear();
            self.bytes = 0;
        }

        /// The rects of the kept fields, the least recently used first.
        #[cfg(test)]
        pub(crate) fn rects(&self) -> Vec<IRect> {
            self.fields.iter().map(|field| field.rect).collect()
        }

        /// The field of `key` that covers `rect`, now the most recently used.
        fn find(&mut self, key: &FieldKey, rect: IRect) -> Option<Arc<Field>> {
            let index = self
                .fields
                .iter()
                .position(|field| field.key == *key && covers(field.rect, rect))?;
            let field = self.fields.remove(index)?;
            self.fields.push_back(Arc::clone(&field));
            Some(field)
        }

        /// The rect of the field of `key`, where one is kept.
        fn rect_of(&self, key: &FieldKey) -> Option<IRect> {
            self.fields
                .iter()
                .find(|field| field.key == *key)
                .map(|field| field.rect)
        }

        /// Whether `key` was drawn before, remembering it.
        fn repeated(&mut self, key: FieldKey) -> bool {
            if self.seen.contains(&key) {
                return true;
            }
            if self.seen.len() == SEEN_KEYS {
                self.seen.pop_front();
            }
            self.seen.push_back(key);
            false
        }

        /// Keeps `field` in place of any other of its key, retiring the least recently used
        /// fields beyond the byte limit.
        fn insert(&mut self, field: Arc<Field>) {
            let bytes = &mut self.bytes;
            self.fields.retain(|kept| {
                let other = kept.key != field.key;
                if !other {
                    *bytes -= kept.bytes();
                }
                other
            });
            self.bytes += field.bytes();
            self.fields.push_back(field);
            while self.bytes > self.byte_limit
                && let Some(retired) = self.fields.pop_front()
            {
                self.bytes -= retired.bytes();
            }
        }
    }

    /// What the noise at a pixel depends on: the map of the target's pixels into local units, as
    /// the bits of `to_local` and `local_origin`, and the noise.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct FieldKey {
        placement: [u32; 6],
        frequency: [u32; 2],
        octaves: u8,
        seed: u32,
        turbulence: bool,
    }

    impl FieldKey {
        /// The key of the noise a draw with `uniforms` reads over `rect`, and the offset from the
        /// target's pixels to the pixels of the fields of that key. Where `to_local` is an
        /// identity and `local_origin` whole, the local point of a pixel is its column or row
        /// plus one half, plus the origin, all exact, whatever the target: the noise of a pixel
        /// is that of its column and row offset by the origin. Those offset pixels hold the
        /// fields of such noise, under an identity and no origin.
        fn new(uniforms: &[f32; 15], noise: Noise, rect: IRect) -> (Self, [i32; 2]) {
            // The pixels and the origin stay within 2^20, and so every sum with a half is exact.
            const LIMIT: i32 = 1 << 20;
            let whole = |value: f32| value.fract() == 0.0 && value.abs() <= LIMIT as f32;
            let [to_local @ .., x, y] = <[f32; 6]>::try_from(&uniforms[..6]).unwrap();
            let (placement, shift) = if to_local == [1.0, 0.0, 0.0, 1.0]
                && whole(x)
                && whole(y)
                && rect.right <= LIMIT
                && rect.bottom <= LIMIT
            {
                ([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [x as i32, y as i32])
            } else {
                (
                    [to_local[0], to_local[1], to_local[2], to_local[3], x, y],
                    [0, 0],
                )
            };
            let key = Self {
                placement: placement.map(f32::to_bits),
                frequency: noise.frequency.map(f32::to_bits),
                octaves: noise.octaves,
                seed: noise.seed,
                turbulence: noise.turbulence,
            };
            (key, shift)
        }
    }

    /// The noise of the pixels of `rect`, row by row: each pixel's red, then its green. Room for
    /// three more pixels follows, so that the last pixel loads as the first of a vector.
    struct Field {
        key: FieldKey,
        rect: IRect,
        units: Vec<f32>,
        _memory: Option<MemoryLease>,
    }

    impl Field {
        fn bytes(&self) -> usize {
            self.units.len() * 4
        }

        /// The noise from pixel (`x`, `y`) of the field to its end.
        fn from(&self, x: i32, y: i32) -> &[f32] {
            let width = self.rect.width() as usize;
            let pixel = (y - self.rect.top) as usize * width + (x - self.rect.left) as usize;
            &self.units[pixel * 2..]
        }
    }

    /// Whether `outer` holds every pixel of `inner`.
    fn covers(outer: IRect, inner: IRect) -> bool {
        outer.left <= inner.left
            && outer.top <= inner.top
            && outer.right >= inner.right
            && outer.bottom >= inner.bottom
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        surface: &mut Surface,
        input: &Image,
        source: IRect,
        origin: [i32; 2],
        rect: IRect,
        uniforms: &[f32; 15],
        noise: Noise,
        fields: Option<&Mutex<NoiseFields>>,
    ) -> Result<bool, DrawError> {
        // Skia makes no noise shader for these frequencies, and a constant one for no octaves.
        if !(noise.frequency[0] >= 0.0 && noise.frequency[1] >= 0.0) || noise.octaves == 0 {
            return Ok(false);
        }
        let (Some(clip), Some(pixels)) = (rows_clip(surface), input.peek_pixels()) else {
            return Ok(false);
        };
        let target = surface.image_info();
        let half = |info: &ImageInfo| {
            info.color_type() == ColorType::RGBAF16 && info.alpha_type() == AlphaType::Premul
        };
        // In the target's color space the image reads its pixels as they are.
        if !half(&target)
            || !half(pixels.info())
            || pixels.info().color_space() != target.color_space()
        {
            return Ok(false);
        }
        let Some(rect) = IRect::intersect(&rect, &clip) else {
            return Ok(true);
        };
        let [to_local, origin_local, to_canvas, scale, bounds] =
            [0..4, 4..6, 6..10, 10..11, 11..15].map(|range| &uniforms[range]);
        let kernel = Kernel {
            to_local: to_local.try_into().unwrap(),
            local_origin: origin_local.try_into().unwrap(),
            to_canvas: to_canvas.try_into().unwrap(),
            scale: scale[0],
            bounds: bounds.try_into().unwrap(),
            // The image shader maps canvas points into the image by the inverse of its placement.
            image_offset: [
                -((origin[0] - source.left) as f32),
                -((origin[1] - source.top) as f32),
            ],
            image: Input {
                bytes: pixels
                    .bytes()
                    .ok_or_else(|| DrawError::Surface("Raster image has no pixels".into()))?,
                stride: pixels.row_bytes() / 8,
                size: [pixels.width() as f32, pixels.height() as f32],
            },
            lattice: Lattice::new(noise.seed as f32),
            frequency: noise.frequency,
            octaves: noise.octaves,
            turbulence: noise.turbulence,
        };
        let (key, shift) = FieldKey::new(uniforms, noise, rect);
        let field = fields.and_then(|fields| {
            field(
                fields,
                key,
                rect.with_offset((shift[0], shift[1])),
                shift,
                &kernel,
            )
        });
        // As for a Src draw: pixels a snapshot still shares are replaced before they are written,
        // by a copy unless the rect is the whole surface.
        let whole = rect == IRect::from_wh(surface.width(), surface.height());
        surface.notify_content_will_change(if whole {
            ContentChangeMode::Discard
        } else {
            ContentChangeMode::Retain
        });
        let mut out = surface
            .peek_pixels()
            .ok_or_else(|| DrawError::Surface("Raster surface has no pixels".into()))?;
        let row_bytes = out.row_bytes();
        let bytes = out
            .bytes_mut()
            .ok_or_else(|| DrawError::Surface("Raster surface pixels are not writable".into()))?;
        let span =
            rect.top as usize * row_bytes..(rect.bottom as usize * row_bytes).min(bytes.len());
        let columns = rect.left as usize * 8..rect.right as usize * 8;
        fill_image_rows(&mut bytes[span], row_bytes, |first, rows| {
            for (index, row) in rows.chunks_mut(row_bytes).enumerate() {
                let y = rect.top + (first + index) as i32;
                let out = &mut row[columns.clone()];
                match &field {
                    Some(field) => {
                        let units = field.from(rect.left + shift[0], y + shift[1]);
                        kernel.row_over(units, y, rect.left, out);
                    }
                    None => kernel.row(y, rect.left, out),
                }
            }
        });
        Ok(true)
    }

    /// The field of `key` that holds `wanted`, in the pixels of the fields of `key`, which are
    /// the target's offset by `shift`: one `fields` keeps, or else one computed now and kept
    /// there, where `key` was drawn before and the field fits. A kept field of `key` that misses
    /// some of those pixels grows to hold them.
    fn field(
        fields: &Mutex<NoiseFields>,
        key: FieldKey,
        wanted: IRect,
        shift: [i32; 2],
        kernel: &Kernel,
    ) -> Option<Arc<Field>> {
        let floats = |rect: IRect| (rect.width() as usize * rect.height() as usize + LANES - 1) * 2;
        let rect = {
            let mut fields = fields.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(field) = fields.find(&key, wanted) {
                return Some(field);
            }
            let rect = match fields.rect_of(&key) {
                Some(kept) => IRect::join(&kept, &wanted),
                None if fields.repeated(key) => wanted,
                None => return None,
            };
            [rect, wanted]
                .into_iter()
                .find(|rect| floats(*rect) * 4 <= fields.byte_limit)?
        };
        let memory = reserve_retained(MemoryKind::Resource, (floats(rect) * 4) as u64).ok()?;
        let width = rect.width() as usize * 2;
        let mut units = vec![0.0; floats(rect)];
        let rows = rect.height() as usize * width;
        fill_image_rows(&mut units[..rows], width, |first, rows| {
            for (index, row) in rows.chunks_mut(width).enumerate() {
                let y = rect.top + (first + index) as i32;
                kernel.units_row(y - shift[1], rect.left - shift[0], row);
            }
        });
        let field = Arc::new(Field {
            key,
            rect,
            units,
            _memory: memory,
        });
        fields
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(Arc::clone(&field));
        Some(field)
    }

    /// The input's half-float pixels.
    struct Input<'a> {
        bytes: &'a [u8],
        /// Pixels per row.
        stride: usize,
        size: [f32; 2],
    }

    /// The kernel's uniforms and children.
    struct Kernel<'a> {
        to_local: [f32; 4],
        local_origin: [f32; 2],
        to_canvas: [f32; 4],
        scale: f32,
        bounds: [f32; 4],
        image_offset: [f32; 2],
        image: Input<'a>,
        lattice: Lattice,
        frequency: [f32; 2],
        octaves: u8,
        turbulence: bool,
    }

    impl Kernel<'_> {
        /// Writes the kernel's pixels of canvas row `y` from column `left` into `out`.
        fn row(&self, y: i32, left: i32, out: &mut [u8]) {
            let py = center(y);
            for (block, out) in out.chunks_mut(LANES * 8).enumerate() {
                let px = centers(left + (block * LANES) as i32);
                store(self.displaced(px, py, self.units(px, py)), out);
            }
        }

        /// Writes the noise of canvas row `y` from column `left` into `out`: each pixel's red,
        /// then its green.
        fn units_row(&self, y: i32, left: i32, out: &mut [f32]) {
            let py = center(y);
            for (block, out) in out.chunks_mut(LANES * 2).enumerate() {
                let [red, green] = self.units(centers(left + (block * LANES) as i32), py);
                let mut units = [0.0; LANES * 2];
                // SAFETY: NEON is part of the aarch64 baseline; the store writes the eight floats.
                unsafe { vst2q_f32(units.as_mut_ptr(), float32x4x2_t(red, green)) };
                out.copy_from_slice(&units[..out.len()]);
            }
        }

        /// Writes the kernel's pixels of canvas row `y` from column `left` into `out`, with the
        /// noise of those pixels in `units`, as `units_row` writes it, followed by the noise of at
        /// least three more pixels.
        fn row_over(&self, units: &[f32], y: i32, left: i32, out: &mut [u8]) {
            let py = center(y);
            for (block, out) in out.chunks_mut(LANES * 8).enumerate() {
                let px = centers(left + (block * LANES) as i32);
                let units = &units[block * LANES * 2..][..LANES * 2];
                // SAFETY: NEON is part of the aarch64 baseline; the load reads the eight floats.
                let float32x4x2_t(red, green) = unsafe { vld2q_f32(units.as_ptr()) };
                store(self.displaced(px, py, [red, green]), out);
            }
        }

        /// The red and green of the noise at the four points (`px`, `py`) as the kernel reads
        /// them, `saturate(unpremul(noise))`, as its raster pipeline program computes them:
        /// fused multiply-adds for each `dot`, and every other operation as written.
        #[inline(always)]
        fn units(&self, px: float32x4_t, py: float32x4_t) -> [float32x4_t; 2] {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                let splat = |value: f32| vdupq_n_f32(value);
                let local_x = vaddq_f32(
                    dot(&self.to_local[0..2], px, py),
                    splat(self.local_origin[0]),
                );
                let local_y = vaddq_f32(
                    dot(&self.to_local[2..4], px, py),
                    splat(self.local_origin[1]),
                );
                let [red, green, alpha] = self.noise(local_x, local_y);
                // Color over `max(alpha, 0.0001)`, clamped to [0, 1].
                let divisor = vmaxq_f32(alpha, splat(0.0001));
                let unit = |channel: float32x4_t| {
                    vminq_f32(
                        vmaxq_f32(vdivq_f32(channel, divisor), splat(0.0)),
                        splat(1.0),
                    )
                };
                [unit(red), unit(green)]
            }
        }

        /// The rest of the kernel's `main` at the four points (`px`, `py`), from the red and
        /// green of the noise there, `units`. Returns the four half-float pixels.
        #[inline(always)]
        fn displaced(
            &self,
            px: float32x4_t,
            py: float32x4_t,
            [red, green]: [float32x4_t; 2],
        ) -> [u64; LANES] {
            // SAFETY: NEON is part of the aarch64 baseline; the stores write the four lanes of
            // each array, and each pixel read stays inside the input, whose indices are clamped
            // to its pixels.
            unsafe {
                let splat = |value: f32| vdupq_n_f32(value);
                let zero = splat(0.0);
                let half = splat(0.5);
                let scale = splat(self.scale);
                let vector_x = vmulq_f32(scale, vsubq_f32(red, half));
                let vector_y = vmulq_f32(scale, vsubq_f32(green, half));
                let offset_x = dot(&self.to_canvas[0..2], vector_x, vector_y);
                let offset_y = dot(&self.to_canvas[2..4], vector_x, vector_y);
                let sample_x = vaddq_f32(vrndmq_f32(vaddq_f32(px, offset_x)), half);
                let sample_y = vaddq_f32(vrndmq_f32(vaddq_f32(py, offset_y)), half);
                let [left, top, right, bottom] = self.bounds;
                let outside = vorrq_u32(
                    vorrq_u32(
                        vcltq_f32(sample_x, splat(left)),
                        vcltq_f32(sample_y, splat(top)),
                    ),
                    vorrq_u32(
                        vcgtq_f32(sample_x, splat(right)),
                        vcgtq_f32(sample_y, splat(bottom)),
                    ),
                );
                // `image.eval(sample)`: the image shader translates the point into the image,
                // decals it to (0, size] on each axis and reads the nearest pixel, a point on a
                // pixel edge reading the pixel before it.
                let x = vaddq_f32(sample_x, splat(self.image_offset[0]));
                let y = vaddq_f32(sample_y, splat(self.image_offset[1]));
                let [width, height] = [splat(self.image.size[0]), splat(self.image.size[1])];
                let decal = |at: float32x4_t, limit: float32x4_t| {
                    vorrq_u32(
                        vandq_u32(vcltq_f32(zero, at), vcltq_f32(at, limit)),
                        vceqq_f32(at, limit),
                    )
                };
                let inside = vbicq_u32(vandq_u32(decal(x, width), decal(y, height)), outside);
                let texel = |at: float32x4_t, limit: float32x4_t| {
                    let below = vreinterpretq_f32_u32(vsubq_u32(
                        vreinterpretq_u32_f32(limit),
                        vdupq_n_u32(1),
                    ));
                    let clamped = vminq_f32(vmaxq_f32(splat(f32::MIN_POSITIVE), at), below);
                    let down = vsubq_u32(vreinterpretq_u32_f32(clamped), vdupq_n_u32(1));
                    vreinterpretq_u32_s32(vcvtq_s32_f32(vreinterpretq_f32_u32(down)))
                };
                let at = vmlaq_u32(
                    texel(x, width),
                    texel(y, height),
                    vdupq_n_u32(self.image.stride as u32),
                );
                let (mut indices, mut kept) = ([0_u32; LANES], [0_u32; LANES]);
                vst1q_u32(indices.as_mut_ptr(), at);
                vst1q_u32(kept.as_mut_ptr(), inside);
                let mut pixels = [0; LANES];
                for ((pixel, &index), &kept) in pixels.iter_mut().zip(&indices).zip(&kept) {
                    if kept != 0 {
                        let bytes = &self.image.bytes[index as usize * 8..][..8];
                        // The pixel widens to floats and the store narrows it back.
                        let halves = vcreate_u16(u64::from_le_bytes(bytes.try_into().unwrap()));
                        let stored = vcvt_f16_f32(vcvt_f32_f16(vreinterpret_f16_u16(halves)));
                        *pixel =
                            vget_lane_u64::<0>(vreinterpret_u64_u16(vreinterpret_u16_f16(stored)));
                    }
                }
                pixels
            }
        }

        /// Skia's Perlin noise stage at the four points (`x`, `y`): the premultiplied red, green
        /// and alpha of the noise. Blue does not reach the kernel's result.
        #[inline(always)]
        fn noise(&self, x: float32x4_t, y: float32x4_t) -> [float32x4_t; 3] {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                let splat = |value: f32| vdupq_n_f32(value);
                let zero = splat(0.0);
                let one = splat(1.0);
                let two = splat(2.0);
                let three = splat(3.0);
                let half = splat(0.5);
                let mut noise_x = vmulq_f32(vaddq_f32(x, half), splat(self.frequency[0]));
                let mut noise_y = vmulq_f32(vaddq_f32(y, half), splat(self.frequency[1]));
                let mut sums = [zero; 3];
                let mut ratio = 1.0_f32;
                let lerp = |from: float32x4_t, to: float32x4_t, t: float32x4_t| {
                    vfmaq_f32(from, vsubq_f32(to, from), t)
                };
                let smooth = |t: float32x4_t| {
                    vmulq_f32(vmulq_f32(t, t), vsubq_f32(three, vmulq_f32(two, t)))
                };
                for _ in 0..self.octaves {
                    let floor_x = vrndmq_f32(noise_x);
                    let floor_y = vrndmq_f32(noise_y);
                    let fract_x = vsubq_f32(noise_x, floor_x);
                    let fract_y = vsubq_f32(noise_y, floor_y);
                    let corners = self.lattice.corners(floor_x, floor_y);
                    let smooth_x = smooth(fract_x);
                    let smooth_y = smooth(fract_y);
                    let fract_x1 = vsubq_f32(fract_x, one);
                    let fract_y1 = vsubq_f32(fract_y, one);
                    let ratio_v = splat(ratio);
                    for (sum, &channel) in sums.iter_mut().zip(&CHANNELS) {
                        let dot = |corner: usize, x: float32x4_t, y: float32x4_t| {
                            let [gradient_x, gradient_y] = &corners[corner];
                            vfmaq_f32(vmulq_f32(gradient_y[channel], y), gradient_x[channel], x)
                        };
                        let a = lerp(
                            dot(0, fract_x, fract_y),
                            dot(1, fract_x1, fract_y),
                            smooth_x,
                        );
                        let b = lerp(
                            dot(2, fract_x, fract_y1),
                            dot(3, fract_x1, fract_y1),
                            smooth_x,
                        );
                        let mut color = lerp(a, b, smooth_y);
                        if self.turbulence {
                            color = vabsq_f32(color);
                        }
                        *sum = vfmaq_f32(*sum, color, ratio_v);
                    }
                    noise_x = vmulq_f32(noise_x, two);
                    noise_y = vmulq_f32(noise_y, two);
                    ratio *= 0.5;
                }
                if !self.turbulence {
                    for sum in &mut sums {
                        *sum = vfmaq_f32(half, *sum, half);
                    }
                }
                let clamp = |value: float32x4_t| vminq_f32(vmaxq_f32(zero, value), one);
                let [red, green, alpha] = sums;
                [
                    vmulq_f32(clamp(red), alpha),
                    vmulq_f32(clamp(green), alpha),
                    clamp(alpha),
                ]
            }
        }
    }

    /// Skia's Perlin noise tables for a seed, with each gradient as the stage reads it.
    struct Lattice {
        selector: [u8; 256],
        /// For each lattice point, the x components of its gradients in the four channels, then
        /// their y components: the stage's `mad(sample, 2 / 65535, -1)` of each stored sample.
        gradients: [[[f32; 4]; 2]; 256],
    }

    impl Lattice {
        fn new(seed: f32) -> Self {
            let tables = valle_engine::compositor::perlin::PerlinTables::new(seed);
            let mut gradients = [[[0.0; 4]; 2]; 256];
            for (gradient, samples) in gradients.iter_mut().zip(&tables.samples) {
                for (channel, sample) in samples.iter().enumerate() {
                    for (axis, stored) in sample.iter().enumerate() {
                        gradient[axis][channel] = f32::from(*stored).mul_add(2.0 / 65_535.0, -1.0);
                    }
                }
            }
            Self {
                selector: tables.selector,
                gradients,
            }
        }

        /// The gradients at the corners of the lattice cells whose first corners are (`floor_x`,
        /// `floor_y`), in the stage's order: per corner, per axis, a vector of the four points
        /// for each channel.
        #[inline(always)]
        fn corners(
            &self,
            floor_x: float32x4_t,
            floor_y: float32x4_t,
        ) -> [[[float32x4_t; 4]; 2]; 4] {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                let one = vdupq_n_f32(1.0);
                let ceil_x = vaddq_f32(floor_x, one);
                let ceil_y = vaddq_f32(floor_y, one);
                let shared = vandq_u32(
                    vceqq_f32(floor_x, vdupq_laneq_f32::<0>(floor_x)),
                    vceqq_f32(floor_y, vdupq_laneq_f32::<0>(floor_y)),
                );
                if vminvq_u32(shared) != 0 {
                    // The four points are in one cell, so every lookup takes the same entries:
                    // the first point's are read once and spread to all four.
                    let first = |value: float32x4_t| vgetq_lane_u32::<0>(lookup(value)) as usize;
                    let lattice_x = vdupq_n_f32(f32::from(self.selector[first(floor_x)]));
                    let lattice_y = vdupq_n_f32(f32::from(self.selector[first(ceil_x)]));
                    return [
                        self.spread(first(vaddq_f32(lattice_x, floor_y))),
                        self.spread(first(vaddq_f32(lattice_y, floor_y))),
                        self.spread(first(vaddq_f32(lattice_x, ceil_y))),
                        self.spread(first(vaddq_f32(lattice_y, ceil_y))),
                    ];
                }
                let lattice_x = self.select(lookup(floor_x));
                let lattice_y = self.select(lookup(ceil_x));
                [
                    self.gather(lookup(vaddq_f32(lattice_x, floor_y))),
                    self.gather(lookup(vaddq_f32(lattice_y, floor_y))),
                    self.gather(lookup(vaddq_f32(lattice_x, ceil_y))),
                    self.gather(lookup(vaddq_f32(lattice_y, ceil_y))),
                ]
            }
        }

        /// The gradients of the lattice point at `index`, each spread to all four lanes: per
        /// axis, a vector for each channel.
        #[inline(always)]
        fn spread(&self, index: usize) -> [[float32x4_t; 4]; 2] {
            // SAFETY: NEON is part of the aarch64 baseline; each load reads the four floats of
            // one axis of the point.
            unsafe {
                let point = &self.gradients[index];
                let axis = |axis: usize| {
                    let channels = vld1q_f32(point[axis].as_ptr());
                    [
                        vdupq_laneq_f32::<0>(channels),
                        vdupq_laneq_f32::<1>(channels),
                        vdupq_laneq_f32::<2>(channels),
                        vdupq_laneq_f32::<3>(channels),
                    ]
                };
                [axis(0), axis(1)]
            }
        }

        /// The selector entries at four indices below 256, as floats.
        #[inline(always)]
        fn select(&self, index: uint32x4_t) -> float32x4_t {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                let mut indices = [0_u32; LANES];
                vst1q_u32(indices.as_mut_ptr(), index);
                let entry = |lane: usize| u32::from(self.selector[indices[lane] as usize & 0xff]);
                let entries = [entry(0), entry(1), entry(2), entry(3)];
                vcvtq_f32_s32(vreinterpretq_s32_u32(vld1q_u32(entries.as_ptr())))
            }
        }

        /// The gradients at four indices below 256: per axis, a vector of the four points for
        /// each channel.
        #[inline(always)]
        fn gather(&self, index: uint32x4_t) -> [[float32x4_t; 4]; 2] {
            // SAFETY: NEON is part of the aarch64 baseline; each load reads the four floats of
            // one axis of one lattice point.
            unsafe {
                let mut indices = [0_u32; LANES];
                vst1q_u32(indices.as_mut_ptr(), index);
                let point = |lane: usize| &self.gradients[indices[lane] as usize & 0xff];
                let points = [point(0), point(1), point(2), point(3)];
                let axis = |axis: usize| {
                    transpose([
                        vld1q_f32(points[0][axis].as_ptr()),
                        vld1q_f32(points[1][axis].as_ptr()),
                        vld1q_f32(points[2][axis].as_ptr()),
                        vld1q_f32(points[3][axis].as_ptr()),
                    ])
                };
                [axis(0), axis(1)]
            }
        }
    }

    /// The center of each pixel of row `y`, as the pipeline seeds every lane with it.
    #[inline(always)]
    fn center(y: i32) -> float32x4_t {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe { vdupq_n_f32(y as f32 + 0.5) }
    }

    /// The centers of the four pixels from `column`, as the pipeline seeds its lanes with them.
    #[inline(always)]
    fn centers(column: i32) -> float32x4_t {
        // SAFETY: NEON is part of the aarch64 baseline; the load reads the four floats.
        unsafe {
            vaddq_f32(
                vdupq_n_f32(column as f32),
                vld1q_f32([0.5, 1.5, 2.5, 3.5].as_ptr()),
            )
        }
    }

    /// The kernel's `dot` of a row of a matrix with (`x`, `y`): a product, then a fused
    /// multiply-add.
    #[inline(always)]
    fn dot(row: &[f32], x: float32x4_t, y: float32x4_t) -> float32x4_t {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe { vfmaq_f32(vmulq_f32(vdupq_n_f32(row[1]), y), vdupq_n_f32(row[0]), x) }
    }

    /// Writes the pixels of `pixels` that `out` has room for.
    #[inline(always)]
    fn store(pixels: [u64; LANES], out: &mut [u8]) {
        for (bytes, pixel) in out.as_chunks_mut::<8>().0.iter_mut().zip(&pixels) {
            *bytes = pixel.to_le_bytes();
        }
    }

    /// The stage's lattice index of each lane: the nearest integer, ties to even, wrapped to 256.
    #[inline(always)]
    fn lookup(value: float32x4_t) -> uint32x4_t {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            vandq_u32(
                vreinterpretq_u32_s32(vcvtnq_s32_f32(value)),
                vdupq_n_u32(0xff),
            )
        }
    }

    /// Rows of four lanes into columns.
    #[inline(always)]
    fn transpose(rows: [float32x4_t; 4]) -> [float32x4_t; 4] {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let low = vtrnq_f32(rows[0], rows[1]);
            let high = vtrnq_f32(rows[2], rows[3]);
            [
                vcombine_f32(vget_low_f32(low.0), vget_low_f32(high.0)),
                vcombine_f32(vget_low_f32(low.1), vget_low_f32(high.1)),
                vcombine_f32(vget_high_f32(low.0), vget_high_f32(high.0)),
                vcombine_f32(vget_high_f32(low.1), vget_high_f32(high.1)),
            ]
        }
    }
}
