//! Shared material images and embedded glTF metallic/roughness materials. Decode and mip once.
use crate::{
    ContentDigest,
    scene3d::{
        ContractErrors, MAX_MATERIALS, MAX_TEXTURE_PIXELS, MAX_TEXTURE_STORAGE_BYTES, MAX_TEXTURES,
        MaterialFrameState, MaterialTexture, MaterialTextureSlot, MipmapFilter, TextureFilter,
        TextureRole, TextureWrap,
    },
};
use serde::Deserialize;
use std::{collections::BTreeMap, io::Cursor, sync::Arc};

#[derive(Clone, Debug, PartialEq)]
pub struct ModelMaterial {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub emissive_intensity: f32,
    pub normal_scale: f32,
    pub occlusion_strength: f32,
    pub double_sided: bool,
    /// None is opaque; Some is alpha mask with this cutoff. Blending is not admitted.
    pub alpha_cutoff: Option<f32>,
    pub(crate) base_color_texture: Option<ModelTextureBinding>,
    pub(crate) metallic_roughness_texture: Option<ModelTextureBinding>,
    pub(crate) normal_texture: Option<ModelTextureBinding>,
    pub(crate) occlusion_texture: Option<ModelTextureBinding>,
    pub(crate) emissive_texture: Option<ModelTextureBinding>,
}
impl Default for ModelMaterial {
    fn default() -> Self {
        Self {
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            emissive: [0.0; 3],
            emissive_intensity: 1.0,
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            double_sided: false,
            alpha_cutoff: None,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
        }
    }
}

impl ModelMaterial {
    pub(crate) fn uses_uv(&self) -> bool {
        self.base_color_texture.is_some()
            || self.metallic_roughness_texture.is_some()
            || self.normal_texture.is_some()
            || self.occlusion_texture.is_some()
            || self.emissive_texture.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModelTextureBinding {
    pub image: usize,
    wrap_s: u32,
    wrap_t: u32,
    min_filter: u32,
    mag_filter: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct MipLevel {
    width: u32,
    height: u32,
    /// Straight linear RGBA. Data channels keep their normalized source values.
    rgba: Arc<[f32]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialImage {
    pub(crate) content_digest: ContentDigest,
    role: TextureRole,
    levels: Vec<MipLevel>,
}
impl MaterialImage {
    pub fn size(&self) -> [u32; 2] {
        [self.levels[0].width, self.levels[0].height]
    }
    pub fn content_digest(&self) -> ContentDigest {
        self.content_digest
    }
    pub fn pixel_count(&self) -> u64 {
        let [w, h] = self.size();
        u64::from(w) * u64::from(h)
    }
    pub fn storage_bytes(&self) -> u64 {
        self.levels.iter().map(|l| l.rgba.len() as u64 * 4).sum()
    }
    pub fn mip_count(&self) -> usize {
        self.levels.len()
    }
    pub(crate) fn storage_key(&self) -> (ContentDigest, TextureRole) {
        (self.content_digest, self.role)
    }
    pub(crate) fn sample(&self, binding: ModelTextureBinding, uv: [f32; 2], lod: f32) -> [f32; 4] {
        let filter = if lod <= 0.0 {
            binding.mag_filter
        } else {
            binding.min_filter
        };
        let linear = matches!(filter, 9729 | 9985 | 9987);
        let mip = matches!(filter, 9984..=9987);
        let level = if mip {
            lod.max(0.0).min((self.levels.len() - 1) as f32)
        } else {
            0.0
        };
        let a = if matches!(filter, 9984 | 9985) {
            libm::roundf(level) as usize
        } else {
            libm::floorf(level) as usize
        };
        let first = self.sample_level(a, binding, uv, linear);
        if !matches!(filter, 9986 | 9987) || a + 1 >= self.levels.len() {
            return first;
        }
        let second = self.sample_level(a + 1, binding, uv, linear);
        std::array::from_fn(|i| first[i] + (second[i] - first[i]) * (level - a as f32))
    }
    fn sample_level(
        &self,
        level: usize,
        binding: ModelTextureBinding,
        uv: [f32; 2],
        linear: bool,
    ) -> [f32; 4] {
        let image = &self.levels[level];
        let x = uv[0] * image.width as f32 - 0.5;
        let y = uv[1] * image.height as f32 - 0.5;
        let pixel = |x: i64, y: i64| {
            let x = wrap_index(x, image.width, binding.wrap_s);
            let y = wrap_index(y, image.height, binding.wrap_t);
            let offset = (y as usize * image.width as usize + x as usize) * 4;
            std::array::from_fn::<_, 4, _>(|i| image.rgba[offset + i])
        };
        if !linear {
            return pixel(libm::floorf(x + 0.5) as i64, libm::floorf(y + 0.5) as i64);
        }
        let ix = libm::floorf(x) as i64;
        let iy = libm::floorf(y) as i64;
        let tx = x - ix as f32;
        let ty = y - iy as f32;
        let a = pixel(ix, iy);
        let b = pixel(ix + 1, iy);
        let c = pixel(ix, iy + 1);
        let d = pixel(ix + 1, iy + 1);
        std::array::from_fn(|i| {
            (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty
        })
    }
}
fn wrap_index(i: i64, n: u32, mode: u32) -> u32 {
    let n = i64::from(n);
    match mode {
        33071 => i.clamp(0, n - 1) as u32,
        33648 => {
            let p = i.rem_euclid(n * 2);
            if p < n {
                p as u32
            } else {
                (2 * n - 1 - p) as u32
            }
        }
        _ => i.rem_euclid(n) as u32,
    }
}
pub(crate) fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        libm::powf((v + 0.055) / 1.055, 2.4)
    }
}

pub(crate) fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * libm::powf(v, 1.0 / 2.4) - 0.055
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct GltfImage {
    buffer_view: usize,
    mime_type: String,
    #[serde(default, rename = "name")]
    _name: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GltfTexture {
    source: usize,
    #[serde(default)]
    sampler: Option<usize>,
    #[serde(default, rename = "name")]
    _name: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub(super) struct GltfSampler {
    mag_filter: u32,
    min_filter: u32,
    wrap_s: u32,
    wrap_t: u32,
}
impl Default for GltfSampler {
    fn default() -> Self {
        Self {
            mag_filter: 9729,
            min_filter: 9987,
            wrap_s: 10497,
            wrap_t: 10497,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TextureInfo {
    index: usize,
    #[serde(default)]
    tex_coord: u32,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NormalInfo {
    index: usize,
    #[serde(default)]
    tex_coord: u32,
    #[serde(default = "one")]
    scale: f32,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OcclusionInfo {
    index: usize,
    #[serde(default)]
    tex_coord: u32,
    #[serde(default = "one")]
    strength: f32,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct Pbr {
    base_color_factor: [f32; 4],
    metallic_factor: f32,
    roughness_factor: f32,
    base_color_texture: Option<TextureInfo>,
    metallic_roughness_texture: Option<TextureInfo>,
}
impl Default for Pbr {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0; 4],
            metallic_factor: 1.0,
            roughness_factor: 1.0,
            base_color_texture: None,
            metallic_roughness_texture: None,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub(super) struct GltfMaterial {
    name: Option<String>,
    pbr_metallic_roughness: Pbr,
    normal_texture: Option<NormalInfo>,
    occlusion_texture: Option<OcclusionInfo>,
    emissive_texture: Option<TextureInfo>,
    emissive_factor: [f32; 3],
    alpha_mode: String,
    alpha_cutoff: f32,
    double_sided: bool,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}
impl Default for GltfMaterial {
    fn default() -> Self {
        Self {
            _extras: None,
            name: None,
            pbr_metallic_roughness: Pbr::default(),
            normal_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            emissive_factor: [0.0; 3],
            alpha_mode: "OPAQUE".into(),
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}
fn one() -> f32 {
    1.0
}
fn failure(path: impl Into<String>, message: impl Into<String>) -> ContractErrors {
    let mut e = ContractErrors::default();
    e.push(path, message);
    e
}

pub(super) fn admit_materials(
    root: &super::Root,
    bin: &[u8],
) -> Result<(Vec<ModelMaterial>, Vec<MaterialImage>), ContractErrors> {
    if root.materials.len() > MAX_MATERIALS
        || root.images.len() > MAX_TEXTURES as usize
        || root.textures.len() > MAX_TEXTURES as usize
        || root.samplers.len() > MAX_TEXTURES as usize
    {
        return Err(failure(
            "/glb/materials",
            "material/image/texture/sampler count exceeds the bounded PBR profile",
        ));
    }
    let mut image_slots = BTreeMap::<(usize, bool), usize>::new();
    let mut bind =
        |info: &TextureInfo, srgb: bool| -> Result<ModelTextureBinding, ContractErrors> {
            if info.tex_coord != 0 {
                return Err(failure(
                    "/glb/materials/texture/texCoord",
                    "only TEXCOORD_0 is admitted",
                ));
            }
            let texture = root.textures.get(info.index).ok_or_else(|| {
                failure(
                    "/glb/materials/texture/index",
                    "texture index is out of range",
                )
            })?;
            if texture.source >= root.images.len() {
                return Err(failure(
                    "/glb/textures/source",
                    "image index is out of range",
                ));
            }
            let next_slot = image_slots.len();
            let image = *image_slots
                .entry((texture.source, srgb))
                .or_insert(next_slot);
            let sampler = match texture.sampler {
                Some(index) => *root.samplers.get(index).ok_or_else(|| {
                    failure("/glb/textures/sampler", "sampler index is out of range")
                })?,
                None => GltfSampler::default(),
            };
            if !matches!(sampler.mag_filter, 9728 | 9729)
                || !matches!(sampler.min_filter, 9728 | 9729 | 9984..=9987)
                || ![sampler.wrap_s, sampler.wrap_t]
                    .iter()
                    .all(|v| matches!(v, 10497 | 33071 | 33648))
            {
                return Err(failure("/glb/samplers", "invalid filter or wrap mode"));
            }
            Ok(ModelTextureBinding {
                image,
                wrap_s: sampler.wrap_s,
                wrap_t: sampler.wrap_t,
                min_filter: sampler.min_filter,
                mag_filter: sampler.mag_filter,
            })
        };
    let mut materials = Vec::new();
    for (i, m) in root.materials.iter().enumerate() {
        let p = &m.pbr_metallic_roughness;
        let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        if !matches!(m.alpha_mode.as_str(), "OPAQUE" | "MASK")
            || !m.alpha_cutoff.is_finite()
            || m.alpha_cutoff < 0.0
            || !p.base_color_factor.into_iter().all(unit)
            || !m.emissive_factor.into_iter().all(unit)
            || !unit(p.metallic_factor)
            || !unit(p.roughness_factor)
            || m.normal_texture
                .as_ref()
                .is_some_and(|t| !t.scale.is_finite() || t.scale.abs() > 16.0)
            || m.occlusion_texture
                .as_ref()
                .is_some_and(|t| !unit(t.strength))
        {
            return Err(failure(
                format!("/glb/materials/{i}"),
                "PBR factors must be finite and bounded; only OPAQUE and MASK alpha modes are admitted",
            ));
        }
        let material = ModelMaterial {
            base_color: p.base_color_factor,
            metallic: p.metallic_factor,
            roughness: p.roughness_factor,
            emissive: m.emissive_factor,
            emissive_intensity: 1.0,
            normal_scale: m.normal_texture.as_ref().map_or(1.0, |t| t.scale),
            occlusion_strength: m.occlusion_texture.as_ref().map_or(1.0, |t| t.strength),
            double_sided: m.double_sided,
            alpha_cutoff: (m.alpha_mode == "MASK").then_some(m.alpha_cutoff),
            base_color_texture: p
                .base_color_texture
                .as_ref()
                .map(|t| bind(t, true))
                .transpose()?,
            metallic_roughness_texture: p
                .metallic_roughness_texture
                .as_ref()
                .map(|t| bind(t, false))
                .transpose()?,
            normal_texture: m
                .normal_texture
                .as_ref()
                .map(|t| {
                    bind(
                        &TextureInfo {
                            index: t.index,
                            tex_coord: t.tex_coord,
                        },
                        false,
                    )
                })
                .transpose()?,
            occlusion_texture: m
                .occlusion_texture
                .as_ref()
                .map(|t| {
                    bind(
                        &TextureInfo {
                            index: t.index,
                            tex_coord: t.tex_coord,
                        },
                        false,
                    )
                })
                .transpose()?,
            emissive_texture: m
                .emissive_texture
                .as_ref()
                .map(|t| bind(t, true))
                .transpose()?,
        };
        materials.push(material);
    }
    let mut images = Vec::new();
    let mut cache = BTreeMap::<(ContentDigest, bool), MaterialImage>::new();
    let mut pixels = 0u64;
    let mut storage = 0u64;
    let mut image_slots = image_slots.into_iter().collect::<Vec<_>>();
    image_slots.sort_by_key(|(_, slot)| *slot);
    for ((source, srgb), _) in image_slots {
        let image = &root.images[source];
        let view = root
            .buffer_views
            .get(image.buffer_view)
            .ok_or_else(|| failure("/glb/images/bufferView", "image view is out of range"))?;
        // validate_root checked view offsets before this function, including overflow and bounds.
        let bytes = &bin[view.byte_offset..view.byte_offset + view.byte_length];
        let digest = ContentDigest::of_bytes(bytes);
        let format = match image.mime_type.as_str() {
            "image/jpeg" => image::ImageFormat::Jpeg,
            "image/png" => image::ImageFormat::Png,
            _ => {
                return Err(failure(
                    "/glb/images/mimeType",
                    "only embedded JPEG and PNG are admitted",
                ));
            }
        };
        if image::guess_format(bytes).ok() != Some(format) {
            return Err(failure(
                "/glb/images/mimeType",
                "declared MIME does not match encoded bytes",
            ));
        }
        if let Some(previous) = cache.get(&(digest, srgb)) {
            images.push(previous.clone());
            continue;
        }
        if cache.len() >= MAX_TEXTURES as usize {
            return Err(failure(
                "/glb/budget/textures",
                "decoded image interpretations exceed the texture count budget",
            ));
        }
        let role = if srgb {
            TextureRole::Color
        } else {
            TextureRole::Data
        };
        let (width, height, mip_bytes) = image_dimensions(bytes)?;
        pixels = pixels.saturating_add(u64::from(width) * u64::from(height));
        storage = storage.saturating_add(mip_bytes);
        if pixels > MAX_TEXTURE_PIXELS || storage > MAX_TEXTURE_STORAGE_BYTES {
            return Err(failure(
                "/glb/budget/texturePixels",
                "decoded texture pixels or mip storage exceed the material budget",
            ));
        }
        let decoded = MaterialImage::from_encoded(bytes, role)?;
        cache.insert((digest, srgb), decoded.clone());
        images.push(decoded);
    }
    Ok((materials, images))
}

/// Area filtering includes the entire source footprint, including odd final rows and columns.
fn downsample(prev: &MipLevel) -> MipLevel {
    let width = (prev.width / 2).max(1);
    let height = (prev.height / 2).max(1);
    let mut rgba = vec![0.0f32; (width * height * 4) as usize];
    let sx = prev.width as f32 / width as f32;
    let sy = prev.height as f32 / height as f32;
    for y in 0..height {
        for x in 0..width {
            let left = x as f32 * sx;
            let right = (x + 1) as f32 * sx;
            let top = y as f32 * sy;
            let bottom = (y + 1) as f32 * sy;
            let mut sums = [0.0; 4];
            for iy in (libm::floorf(top) as u32)..(libm::ceilf(bottom) as u32).min(prev.height) {
                for ix in (libm::floorf(left) as u32)..(libm::ceilf(right) as u32).min(prev.width) {
                    let weight = (right.min((ix + 1) as f32) - left.max(ix as f32))
                        * (bottom.min((iy + 1) as f32) - top.max(iy as f32));
                    for (c, sum) in sums.iter_mut().enumerate() {
                        *sum += weight * prev.rgba[((iy * prev.width + ix) * 4) as usize + c];
                    }
                }
            }
            for c in 0..4 {
                rgba[((y * width + x) * 4) as usize + c] = sums[c] / (sx * sy);
            }
        }
    }
    MipLevel {
        width,
        height,
        rgba: rgba.into(),
    }
}

fn image_reader(bytes: &[u8]) -> Result<image::ImageReader<Cursor<&[u8]>>, ContractErrors> {
    if bytes.len() as u64 > MAX_TEXTURE_STORAGE_BYTES {
        return Err(failure(
            "/texture/bytes",
            "encoded texture exceeds the byte budget",
        ));
    }
    let format = image::guess_format(bytes).map_err(|e| failure("/texture", e.to_string()))?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err(failure("/texture", "material textures require PNG or JPEG"));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(MAX_TEXTURE_STORAGE_BYTES);
    reader.limits(limits);
    Ok(reader)
}
fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32, u64), ContractErrors> {
    let (width, height) = image_reader(bytes)?
        .into_dimensions()
        .map_err(|e| failure("/texture", e.to_string()))?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(failure(
            "/texture/size",
            "material texture dimensions must be in 1..=4096",
        ));
    }
    let (mut w, mut h) = (width, height);
    let mut storage = 0u64;
    loop {
        storage = storage.saturating_add(u64::from(w) * u64::from(h) * 16);
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    if u64::from(width) * u64::from(height) > MAX_TEXTURE_PIXELS
        || storage > MAX_TEXTURE_STORAGE_BYTES
    {
        return Err(failure(
            "/texture/budget",
            "material image pixels or float mip storage exceed the budget",
        ));
    }
    Ok((width, height, storage))
}
impl MaterialImage {
    pub fn role(&self) -> TextureRole {
        self.role
    }
    pub fn from_encoded(bytes: &[u8], role: TextureRole) -> Result<Self, ContractErrors> {
        let (width, height, _) = image_dimensions(bytes)?;
        let mut rgba = image_reader(bytes)?
            .decode()
            .map_err(|e| failure("/texture", e.to_string()))?
            .into_rgba32f()
            .into_raw();
        if role == TextureRole::Color {
            for pixel in rgba.chunks_exact_mut(4) {
                for c in &mut pixel[..3] {
                    *c = srgb_to_linear(*c);
                }
            }
        }
        let mut levels = vec![MipLevel {
            width,
            height,
            rgba: rgba.into(),
        }];
        while levels.last().is_some_and(|l| l.width > 1 || l.height > 1) {
            levels.push(downsample(levels.last().unwrap()));
        }
        Ok(Self {
            content_digest: ContentDigest::of_bytes(bytes),
            role,
            levels,
        })
    }
}
impl ModelMaterial {
    pub(crate) fn texture_mut(
        &mut self,
        slot: MaterialTextureSlot,
    ) -> &mut Option<ModelTextureBinding> {
        match slot {
            MaterialTextureSlot::BaseColor => &mut self.base_color_texture,
            MaterialTextureSlot::MetallicRoughness => &mut self.metallic_roughness_texture,
            MaterialTextureSlot::Normal => &mut self.normal_texture,
            MaterialTextureSlot::Occlusion => &mut self.occlusion_texture,
            MaterialTextureSlot::Emissive => &mut self.emissive_texture,
        }
    }
    pub(crate) fn apply_values(&mut self, value: &MaterialFrameState) {
        if let Some(c) = value.color {
            self.base_color = [
                srgb_to_linear(c.0[0]),
                srgb_to_linear(c.0[1]),
                srgb_to_linear(c.0[2]),
                c.0[3],
            ];
        }
        if let Some(c) = value.emissive {
            self.emissive = [
                srgb_to_linear(c.0[0]),
                srgb_to_linear(c.0[1]),
                srgb_to_linear(c.0[2]),
            ];
        }
        if let Some(v) = value.metallic {
            self.metallic = v;
        }
        if let Some(v) = value.roughness {
            self.roughness = v;
        }
        if let Some(v) = value.emissive_intensity {
            self.emissive_intensity = v;
        }
        if let Some(v) = value.normal_scale {
            self.normal_scale = v;
        }
        if let Some(v) = value.occlusion_strength {
            self.occlusion_strength = v;
        }
        if let Some(v) = value.alpha_cutoff {
            if self.alpha_cutoff.is_some() {
                self.alpha_cutoff = Some(v);
            }
        }
    }
}
impl ModelTextureBinding {
    pub(crate) fn external(image: usize, source: &MaterialTexture) -> Self {
        let wrap = |v| match v {
            TextureWrap::Clamp => 33071,
            TextureWrap::Repeat => 10497,
            TextureWrap::Mirror => 33648,
        };
        let mag_filter = match source.mag_filter {
            TextureFilter::Nearest => 9728,
            TextureFilter::Linear => 9729,
        };
        let min_filter = match (source.min_filter, source.mipmap) {
            (TextureFilter::Nearest, MipmapFilter::None) => 9728,
            (TextureFilter::Linear, MipmapFilter::None) => 9729,
            (TextureFilter::Nearest, MipmapFilter::Nearest) => 9984,
            (TextureFilter::Linear, MipmapFilter::Nearest) => 9985,
            (TextureFilter::Nearest, MipmapFilter::Linear) => 9986,
            (TextureFilter::Linear, MipmapFilter::Linear) => 9987,
        };
        Self {
            image,
            wrap_s: wrap(source.wrap_u),
            wrap_t: wrap(source.wrap_v),
            min_filter,
            mag_filter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn odd_mip_keeps_last_column_and_srgb_is_filtered_in_linear_light() {
        let image = MipLevel {
            width: 3,
            height: 1,
            rgba: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0].into(),
        };
        assert_eq!(
            &*downsample(&image).rgba,
            &[1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 1.0]
        );
    }
    #[test]
    fn encoded_sixteen_bit_channels_and_transparent_rgb_survive_shared_decode() {
        let pixels = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(
            2,
            1,
            vec![65534, 12345, 22222, 0, 65535, 12346, 22223, 65535],
        )
        .unwrap();
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba16(pixels)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let bytes = encoded.into_inner();
        let data = MaterialImage::from_encoded(&bytes, TextureRole::Data).unwrap();
        let color = MaterialImage::from_encoded(&bytes, TextureRole::Color).unwrap();
        let binding = ModelTextureBinding {
            image: 0,
            wrap_s: 33071,
            wrap_t: 33071,
            min_filter: 9728,
            mag_filter: 9728,
        };
        let a = data.sample(binding, [0.25, 0.5], 0.0);
        let b = data.sample(binding, [0.75, 0.5], 0.0);
        assert_eq!(
            a,
            [65534.0 / 65535.0, 12345.0 / 65535.0, 22222.0 / 65535.0, 0.0]
        );
        assert_eq!(b, [1.0, 12346.0 / 65535.0, 22223.0 / 65535.0, 1.0]);
        assert!(a[0] < b[0] && a[1] < b[1]);
        let c = color.sample(binding, [0.25, 0.5], 0.0);
        assert_eq!(c[0], srgb_to_linear(a[0]));
        assert_eq!(c[1], srgb_to_linear(a[1]));
        assert_eq!(c[3], 0.0);
        assert_eq!(data.storage_bytes(), 48);
        assert_ne!(data.storage_key(), color.storage_key());
    }

    #[test]
    fn sampler_respects_wrap_and_color_roles() {
        let image = MaterialImage {
            content_digest: ContentDigest::of_bytes(b"test"),
            role: TextureRole::Data,
            levels: vec![MipLevel {
                width: 2,
                height: 1,
                rgba: vec![128.0 / 255.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0].into(),
            }],
        };
        let mut binding = ModelTextureBinding {
            image: 0,
            wrap_s: 10497,
            wrap_t: 10497,
            min_filter: 9728,
            mag_filter: 9728,
        };
        assert_eq!(
            image.sample(binding, [1.25, 1.5], 0.0),
            [128.0 / 255.0, 0.0, 0.0, 1.0]
        );
        binding.wrap_s = 33071;
        assert_eq!(
            image.sample(binding, [1.25, 0.5], 0.0),
            [0.0, 1.0, 0.0, 1.0]
        );
        binding.wrap_s = 33648;
        assert_eq!(
            image.sample(binding, [-0.25, 0.5], 0.0),
            [128.0 / 255.0, 0.0, 0.0, 1.0]
        );
        let mut color = image.clone();
        color.role = TextureRole::Color;
        color.levels[0].rgba = image.levels[0]
            .rgba
            .chunks_exact(4)
            .flat_map(|p| {
                [
                    srgb_to_linear(p[0]),
                    srgb_to_linear(p[1]),
                    srgb_to_linear(p[2]),
                    p[3],
                ]
            })
            .collect::<Vec<_>>()
            .into();
        assert!((color.sample(binding, [0.25, 0.5], 0.0)[0] - 0.2158605).abs() < 1e-6);
        assert_ne!(image.storage_key(), color.storage_key());
    }
}
