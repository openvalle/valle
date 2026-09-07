use std::{collections::BTreeMap, sync::Arc};

use thiserror::Error;

use super::{
    BlendError, PixelError, PremulRgba32, ReferenceBlendMode, ReferenceImage, blend_over,
    effective_blend_source, validate_unit,
};
use crate::resource::Extent2d;

#[derive(Debug, Clone, PartialEq)]
pub struct CoverageImage {
    extent: Extent2d,
    values: Vec<f32>,
}

impl CoverageImage {
    pub fn new(extent: Extent2d, values: Vec<f32>) -> Result<Self, CompositeError> {
        let expected = image_len(extent)?;
        if values.len() != expected {
            return Err(CompositeError::CoverageCount {
                expected,
                actual: values.len(),
            });
        }
        for value in &values {
            validate_unit(*value, "coverage")?;
        }
        Ok(Self { extent, values })
    }

    pub fn solid(extent: Extent2d, value: f32) -> Result<Self, CompositeError> {
        validate_unit(value, "coverage")?;
        Self::new(extent, vec![value; image_len(extent)?])
    }

    pub const fn extent(&self) -> Extent2d {
        self.extent
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceMaskMode {
    Alpha,
    Luminance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceMask {
    pub image: ReferenceImage,
    pub mode: ReferenceMaskMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceEdgeMode {
    Clamp,
    Transparent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReferenceFilter {
    ColorMatrix {
        matrix: Box<[f32; 20]>,
    },
    BoxBlur {
        radius: u32,
        edge: ReferenceEdgeMode,
    },
}

/// Fixed clip-level order: clip → filters → mask → opacity → blend/source-over.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPipeline {
    clip: Option<CoverageImage>,
    filters: Vec<ReferenceFilter>,
    mask: Option<ReferenceMask>,
    opacity: f32,
    blend: ReferenceBlendMode,
}

impl LayerPipeline {
    pub fn new(opacity: f32, blend: ReferenceBlendMode) -> Result<Self, CompositeError> {
        validate_unit(opacity, "layer opacity")?;
        Ok(Self {
            clip: None,
            filters: Vec::new(),
            mask: None,
            opacity,
            blend,
        })
    }

    pub fn with_clip(mut self, clip: CoverageImage) -> Self {
        self.clip = Some(clip);
        self
    }

    pub fn with_filter(mut self, filter: ReferenceFilter) -> Self {
        self.filters.push(filter);
        self
    }

    pub fn with_mask(mut self, mask: ReferenceMask) -> Self {
        self.mask = Some(mask);
        self
    }

    pub const fn opacity(&self) -> f32 {
        self.opacity
    }

    pub const fn blend(&self) -> ReferenceBlendMode {
        self.blend
    }

    /// Apply only local operations. The parent backdrop is neither copied nor filtered here.
    pub fn prepare_local_source(
        &self,
        source: &ReferenceImage,
    ) -> Result<ReferenceImage, CompositeError> {
        let mut result = source.clone();
        if let Some(clip) = &self.clip {
            result = apply_coverage(&result, clip)?;
        }
        for filter in &self.filters {
            result = apply_filter(&result, filter)?;
        }
        if let Some(mask) = &self.mask {
            result = apply_mask(&result, mask)?;
        }
        result.scale_coverage(self.opacity).map_err(Into::into)
    }
}

impl Default for LayerPipeline {
    fn default() -> Self {
        Self::new(1.0, ReferenceBlendMode::Normal).expect("constant pipeline is valid")
    }
}

pub fn composite_layer(
    local_source: &ReferenceImage,
    backdrop: &ReferenceImage,
    pipeline: &LayerPipeline,
) -> Result<ReferenceImage, CompositeError> {
    local_source.ensure_same_extent(backdrop)?;
    let prepared = pipeline.prepare_local_source(local_source)?;
    blend_images(&prepared, backdrop, pipeline.blend)
}

pub fn apply_coverage(
    image: &ReferenceImage,
    coverage: &CoverageImage,
) -> Result<ReferenceImage, CompositeError> {
    if image.extent() != coverage.extent {
        return Err(CompositeError::ExtentMismatch);
    }
    ReferenceImage::new(
        image.extent(),
        image
            .pixels()
            .iter()
            .zip(&coverage.values)
            .map(|(pixel, coverage)| pixel.scale_coverage(*coverage))
            .collect::<Result<_, _>>()?,
    )
    .map_err(Into::into)
}

pub fn apply_mask(
    image: &ReferenceImage,
    mask: &ReferenceMask,
) -> Result<ReferenceImage, CompositeError> {
    image.ensure_same_extent(&mask.image)?;
    let factors = mask
        .image
        .pixels()
        .iter()
        .map(|pixel| mask_coverage_pixel(*pixel, mask.mode))
        .collect::<Vec<_>>();
    apply_coverage(image, &CoverageImage::new(image.extent(), factors)?)
}

/// Returns mask coverage in the canonical working-linear premultiplied domain.
pub fn mask_coverage_pixel(pixel: PremulRgba32, mode: ReferenceMaskMode) -> f32 {
    match mode {
        ReferenceMaskMode::Alpha => pixel.alpha(),
        ReferenceMaskMode::Luminance => {
            let rgb = pixel.rgb();
            (0.2627 * f64::from(rgb[0]) + 0.6780 * f64::from(rgb[1]) + 0.0593 * f64::from(rgb[2]))
                .clamp(0.0, 1.0) as f32
        }
    }
}

pub fn apply_filter(
    image: &ReferenceImage,
    filter: &ReferenceFilter,
) -> Result<ReferenceImage, CompositeError> {
    match filter {
        ReferenceFilter::ColorMatrix { matrix } => color_matrix(image, matrix),
        ReferenceFilter::BoxBlur { radius, edge } => box_blur(image, *radius, *edge),
    }
}

fn color_matrix(
    image: &ReferenceImage,
    matrix: &[f32; 20],
) -> Result<ReferenceImage, CompositeError> {
    let pixels = image
        .pixels()
        .iter()
        .map(|pixel| color_matrix_pixel(*pixel, matrix))
        .collect::<Result<_, CompositeError>>()?;
    ReferenceImage::new(image.extent(), pixels).map_err(Into::into)
}

pub fn color_matrix_pixel(
    pixel: PremulRgba32,
    matrix: &[f32; 20],
) -> Result<PremulRgba32, CompositeError> {
    let rgb = pixel.straight_rgb()?;
    let input = [rgb[0], rgb[1], rgb[2], pixel.alpha(), 1.0];
    let output = [0, 1, 2, 3].map(|row| {
        let offset = row * 5;
        matrix[offset] * input[0]
            + matrix[offset + 1] * input[1]
            + matrix[offset + 2] * input[2]
            + matrix[offset + 3] * input[3]
            + matrix[offset + 4]
    });
    if !output.iter().all(|channel| channel.is_finite()) {
        return Err(CompositeError::NonFiniteFilter);
    }
    PremulRgba32::from_straight([output[0], output[1], output[2]], output[3].clamp(0.0, 1.0))
        .map_err(Into::into)
}

fn box_blur(
    image: &ReferenceImage,
    radius: u32,
    edge: ReferenceEdgeMode,
) -> Result<ReferenceImage, CompositeError> {
    if radius == 0 {
        return Ok(image.clone());
    }
    if radius > 4096 {
        return Err(CompositeError::FilterRadiusTooLarge { radius });
    }
    let width = i64::from(image.extent().width());
    let height = i64::from(image.extent().height());
    let radius = i64::from(radius);
    let diameter = radius * 2 + 1;
    let divisor = (diameter * diameter) as f64;
    let mut output = Vec::with_capacity(image.pixels().len());
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f64; 4];
            for sample_y in (y - radius)..=(y + radius) {
                for sample_x in (x - radius)..=(x + radius) {
                    let sample = match edge {
                        ReferenceEdgeMode::Clamp => image
                            .pixel(
                                sample_x.clamp(0, width - 1) as u32,
                                sample_y.clamp(0, height - 1) as u32,
                            )
                            .expect("clamped sample is in bounds"),
                        ReferenceEdgeMode::Transparent
                            if sample_x < 0
                                || sample_y < 0
                                || sample_x >= width
                                || sample_y >= height =>
                        {
                            PremulRgba32::TRANSPARENT
                        }
                        ReferenceEdgeMode::Transparent => image
                            .pixel(sample_x as u32, sample_y as u32)
                            .expect("checked sample is in bounds"),
                    };
                    for (sum, channel) in sum.iter_mut().zip(sample.channels()) {
                        *sum += f64::from(channel);
                    }
                }
            }
            output.push(PremulRgba32::from_premultiplied(
                sum.map(|value| (value / divisor) as f32),
            )?);
        }
    }
    ReferenceImage::new(image.extent(), output).map_err(Into::into)
}

fn blend_images(
    source: &ReferenceImage,
    destination: &ReferenceImage,
    mode: ReferenceBlendMode,
) -> Result<ReferenceImage, CompositeError> {
    source.ensure_same_extent(destination)?;
    ReferenceImage::new(
        source.extent(),
        source
            .pixels()
            .iter()
            .zip(destination.pixels())
            .map(|(source, destination)| blend_over(*source, *destination, mode))
            .collect::<Result<_, _>>()?,
    )
    .map_err(Into::into)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompositeVersion(u64);

impl CompositeVersion {
    pub const ROOT: Self = Self(0);

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeToken(u32);

impl ScopeToken {
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct BackdropSnapshot {
    version: CompositeVersion,
    image: Arc<ReferenceImage>,
}

impl BackdropSnapshot {
    pub fn root(image: ReferenceImage) -> Self {
        Self {
            version: CompositeVersion::ROOT,
            image: Arc::new(image),
        }
    }

    pub const fn version(&self) -> CompositeVersion {
        self.version
    }

    pub fn image(&self) -> &ReferenceImage {
        &self.image
    }

    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.image, &other.image)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackdropSelector {
    LayerEntry,
    ScopeEntry(ScopeToken),
    Current,
    Isolated,
}

/// Reference model for one transparent local accumulator `G` over an immutable layer entry `B`.
#[derive(Debug)]
pub struct ReferenceLayer {
    entry: BackdropSnapshot,
    isolated: BackdropSnapshot,
    local: ReferenceImage,
    current_version: CompositeVersion,
    next_version: u64,
    current_cache: Option<BackdropSnapshot>,
    scopes: BTreeMap<ScopeToken, BackdropSnapshot>,
    next_scope: u32,
}

impl ReferenceLayer {
    pub fn root(entry: ReferenceImage) -> Result<Self, CompositeError> {
        Self::from_entry(BackdropSnapshot::root(entry))
    }

    pub fn from_entry(entry: BackdropSnapshot) -> Result<Self, CompositeError> {
        let extent = entry.image.extent();
        let isolated = BackdropSnapshot {
            version: CompositeVersion::ROOT,
            image: Arc::new(ReferenceImage::transparent(extent)?),
        };
        let next_version = entry
            .version
            .0
            .checked_add(1)
            .ok_or(CompositeError::VersionOverflow)?;
        Ok(Self {
            local: ReferenceImage::transparent(extent)?,
            current_version: entry.version,
            next_version,
            entry,
            isolated,
            current_cache: None,
            scopes: BTreeMap::new(),
            next_scope: 1,
        })
    }

    pub fn local_source(&self) -> &ReferenceImage {
        &self.local
    }

    pub fn read_backdrop(
        &mut self,
        selector: BackdropSelector,
    ) -> Result<BackdropSnapshot, CompositeError> {
        match selector {
            BackdropSelector::LayerEntry => Ok(self.entry.clone()),
            BackdropSelector::Isolated => Ok(self.isolated.clone()),
            BackdropSelector::ScopeEntry(token) => self
                .scopes
                .get(&token)
                .cloned()
                .ok_or(CompositeError::UnknownScope { token }),
            BackdropSelector::Current => {
                if let Some(snapshot) = &self.current_cache {
                    return Ok(snapshot.clone());
                }
                let image = self.local.source_over(&self.entry.image)?;
                let snapshot = BackdropSnapshot {
                    version: self.current_version,
                    image: Arc::new(image),
                };
                self.current_cache = Some(snapshot.clone());
                Ok(snapshot)
            }
        }
    }

    pub fn capture_scope(&mut self) -> Result<ScopeToken, CompositeError> {
        let token = ScopeToken(self.next_scope);
        self.next_scope = self
            .next_scope
            .checked_add(1)
            .ok_or(CompositeError::ScopeOverflow)?;
        let snapshot = self.read_backdrop(BackdropSelector::Current)?;
        self.scopes.insert(token, snapshot);
        Ok(token)
    }

    /// Draw one local source. A non-normal blend reads `Current = G over B`, then writes only the
    /// effective source back to `G`; `B` never becomes local content.
    pub fn draw_source(
        &mut self,
        source: &ReferenceImage,
        blend: ReferenceBlendMode,
    ) -> Result<(), CompositeError> {
        source.ensure_same_extent(&self.local)?;
        let following_version = self
            .next_version
            .checked_add(1)
            .ok_or(CompositeError::VersionOverflow)?;
        let effective = if blend == ReferenceBlendMode::Normal {
            source.clone()
        } else {
            let current = self.read_backdrop(BackdropSelector::Current)?;
            ReferenceImage::new(
                source.extent(),
                source
                    .pixels()
                    .iter()
                    .zip(current.image.pixels())
                    .map(|(source, destination)| {
                        effective_blend_source(*source, *destination, blend)
                    })
                    .collect::<Result<_, _>>()?,
            )?
        };
        self.local = effective.source_over(&self.local)?;
        self.current_version = CompositeVersion(self.next_version);
        self.next_version = following_version;
        self.current_cache = None;
        Ok(())
    }

    pub fn begin_group(&mut self, isolated: bool) -> Result<Self, CompositeError> {
        let entry = if isolated {
            self.read_backdrop(BackdropSelector::Isolated)?
        } else {
            self.read_backdrop(BackdropSelector::Current)?
        };
        Self::from_entry(entry)
    }

    pub fn finish_group(
        &mut self,
        group: Self,
        pipeline: &LayerPipeline,
    ) -> Result<(), CompositeError> {
        let source = pipeline.prepare_local_source(&group.local)?;
        self.draw_source(&source, pipeline.blend)
    }

    pub fn finish_over_entry(
        self,
        pipeline: &LayerPipeline,
    ) -> Result<ReferenceImage, CompositeError> {
        composite_layer(&self.local, &self.entry.image, pipeline)
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CompositeError {
    #[error(transparent)]
    Pixel(#[from] PixelError),
    #[error(transparent)]
    Blend(#[from] BlendError),
    #[error("coverage image count mismatch: expected {expected}, got {actual}")]
    CoverageCount { expected: usize, actual: usize },
    #[error("reference image/coverage extents differ")]
    ExtentMismatch,
    #[error("filter produced a non-finite channel")]
    NonFiniteFilter,
    #[error("reference box blur radius {radius} exceeds the safety limit")]
    FilterRadiusTooLarge { radius: u32 },
    #[error("unknown backdrop scope token {token:?}")]
    UnknownScope { token: ScopeToken },
    #[error("backdrop scope token space exhausted")]
    ScopeOverflow,
    #[error("composite version space exhausted")]
    VersionOverflow,
    #[error("image extent exceeds addressable memory")]
    ImageTooLarge,
}

fn image_len(extent: Extent2d) -> Result<usize, CompositeError> {
    (extent.width() as usize)
        .checked_mul(extent.height() as usize)
        .ok_or(CompositeError::ImageTooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(width: u32, height: u32) -> Extent2d {
        Extent2d::new(width, height).unwrap()
    }

    fn pixel(rgb: [f32; 3], alpha: f32) -> PremulRgba32 {
        PremulRgba32::from_straight(rgb, alpha).unwrap()
    }

    #[test]
    fn group_opacity_scales_only_local_source_not_parent_backdrop() {
        let background = ReferenceImage::solid(extent(1, 1), pixel([1.0, 0.0, 0.0], 0.5)).unwrap();
        let source = ReferenceImage::solid(extent(1, 1), pixel([0.0, 0.0, 1.0], 0.5)).unwrap();
        let pipeline = LayerPipeline::new(0.5, ReferenceBlendMode::Normal).unwrap();
        let result = composite_layer(&source, &background, &pipeline).unwrap();
        assert!(result.pixel(0, 0).unwrap().approx_eq(
            PremulRgba32::from_premultiplied([0.375, 0.0, 0.25, 0.625]).unwrap(),
            1e-7
        ));
    }

    #[test]
    fn filters_and_masks_never_touch_parent_backdrop() {
        let background = ReferenceImage::new(
            extent(2, 1),
            vec![pixel([1.0, 0.0, 0.0], 1.0), pixel([0.0, 0.0, 1.0], 1.0)],
        )
        .unwrap();
        let source = ReferenceImage::transparent(extent(2, 1)).unwrap();
        let pipeline = LayerPipeline::default()
            .with_filter(ReferenceFilter::BoxBlur {
                radius: 1,
                edge: ReferenceEdgeMode::Clamp,
            })
            .with_mask(ReferenceMask {
                image: ReferenceImage::solid(extent(2, 1), pixel([1.0; 3], 0.0)).unwrap(),
                mode: ReferenceMaskMode::Alpha,
            });
        assert_eq!(
            composite_layer(&source, &background, &pipeline).unwrap(),
            background
        );
    }

    #[test]
    fn current_layer_entry_scope_entry_and_isolated_are_distinct() {
        let background = ReferenceImage::solid(extent(1, 1), pixel([1.0, 0.0, 0.0], 0.5)).unwrap();
        let green = ReferenceImage::solid(extent(1, 1), pixel([0.0, 1.0, 0.0], 0.5)).unwrap();
        let blue = ReferenceImage::solid(extent(1, 1), pixel([0.0, 0.0, 1.0], 0.5)).unwrap();
        let mut layer = ReferenceLayer::root(background.clone()).unwrap();

        let entry = layer.read_backdrop(BackdropSelector::LayerEntry).unwrap();
        assert_eq!(entry.image(), &background);
        layer
            .draw_source(&green, ReferenceBlendMode::Normal)
            .unwrap();
        let current_before = layer.read_backdrop(BackdropSelector::Current).unwrap();
        assert_ne!(current_before.image(), entry.image());

        let token = layer.capture_scope().unwrap();
        let consumer_one = layer
            .read_backdrop(BackdropSelector::ScopeEntry(token))
            .unwrap();
        let consumer_two = layer
            .read_backdrop(BackdropSelector::ScopeEntry(token))
            .unwrap();
        assert!(consumer_one.shares_storage(&consumer_two));
        assert_eq!(consumer_one.version(), consumer_two.version());

        layer
            .draw_source(&blue, ReferenceBlendMode::Normal)
            .unwrap();
        let current_after = layer.read_backdrop(BackdropSelector::Current).unwrap();
        assert_ne!(current_after.version(), consumer_one.version());
        assert_eq!(
            layer
                .read_backdrop(BackdropSelector::ScopeEntry(token))
                .unwrap()
                .image(),
            consumer_one.image()
        );
        assert_eq!(
            layer
                .read_backdrop(BackdropSelector::Isolated)
                .unwrap()
                .image(),
            &ReferenceImage::transparent(extent(1, 1)).unwrap()
        );
    }

    #[test]
    fn nested_nonisolated_group_reads_parent_current_without_copying_it() {
        let background = ReferenceImage::solid(extent(1, 1), pixel([1.0, 0.0, 0.0], 0.5)).unwrap();
        let green = ReferenceImage::solid(extent(1, 1), pixel([0.0, 1.0, 0.0], 0.5)).unwrap();
        let mut parent = ReferenceLayer::root(background).unwrap();
        parent
            .draw_source(&green, ReferenceBlendMode::Normal)
            .unwrap();
        let parent_current = parent.read_backdrop(BackdropSelector::Current).unwrap();
        let mut child = parent.begin_group(false).unwrap();
        assert_eq!(
            child
                .read_backdrop(BackdropSelector::LayerEntry)
                .unwrap()
                .image(),
            parent_current.image()
        );
        assert_eq!(
            child.local_source(),
            &ReferenceImage::transparent(extent(1, 1)).unwrap()
        );
    }

    #[test]
    fn clip_mask_and_opacity_all_scale_rgb_and_alpha() {
        let source = ReferenceImage::solid(extent(1, 1), pixel([2.0, 1.0, 0.5], 1.0)).unwrap();
        let mask = ReferenceImage::solid(extent(1, 1), pixel([1.0; 3], 0.5)).unwrap();
        let pipeline = LayerPipeline::new(0.5, ReferenceBlendMode::Normal)
            .unwrap()
            .with_clip(CoverageImage::solid(extent(1, 1), 0.5).unwrap())
            .with_mask(ReferenceMask {
                image: mask,
                mode: ReferenceMaskMode::Alpha,
            });
        let prepared = pipeline.prepare_local_source(&source).unwrap();
        assert!(prepared.pixel(0, 0).unwrap().approx_eq(
            PremulRgba32::from_premultiplied([0.25, 0.125, 0.0625, 0.125]).unwrap(),
            1e-7
        ));
    }
}
