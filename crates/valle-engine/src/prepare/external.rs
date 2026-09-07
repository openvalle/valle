use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_draw::{Rect, requirements::LocalBounds};

use crate::{
    compositor::reference::{ColorMathError, PremulRgba32, decode_author_srgb},
    frame::{
        CanvasMapping, LayerBackdrop as Backdrop, RasterFit as Fit, ResolvedTransform,
        SourceCrop as Crop,
    },
    resource::{
        AuthorSrgbStraight, PixelOrientation, SampleAspectRatio, SemanticAsset, SemanticAssetKind,
        UnitFraction, VisualInterpretation,
    },
};

use super::{
    DynamicBindingId, DynamicBindingKind, DynamicValue, RequestError, bounds::LocalGeometry,
    request::DynamicAllocator,
};

const UNIT_RECT: Rect = Rect::new(0.0, 0.0, 1.0, 1.0);
const BACKDROP_BLUR_SIGMA_PER_RADIUS: f64 = 0.5;

/// Sampling behavior at an authored or metadata crop edge. Clamp is relative to the declared
/// sample bounds, not the whole texture, so linear filtering cannot bleed excluded texels back in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalSampling {
    LinearClampToSampleBounds,
}

/// One immutable mapping from a unit content quad to the raw coded texture.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ExternalSample {
    Empty,
    Texture {
        /// Row-major affine 3×3 matrix mapping content `(u, v, 1)` to raw texture UV. It includes
        /// descriptor crop/orientation plus the current-frame authored crop and cover crop.
        texture_from_content: [f64; 9],
        /// Exact axis-aligned raw texture UV domain allowed to participate in linear filtering.
        input_sample_bounds: Rect,
        sampling: ExternalSampling,
    },
}

impl ExternalSample {
    /// Maps a positive normalized rect in interpreted display-content coordinates to raw texture
    /// UV. DrawProgram Image nodes use this same metadata crop/orientation contract as Import.
    pub fn from_display_rect(
        display_rect: Rect,
        interpretation: VisualInterpretation,
    ) -> Result<Self, ExternalPlacementError> {
        external_sample(Some(display_rect), interpretation)
    }

    pub const fn is_empty(self) -> bool {
        matches!(self, Self::Empty)
    }

    pub const fn texture_from_content(self) -> Option<[f64; 9]> {
        match self {
            Self::Empty => None,
            Self::Texture {
                texture_from_content,
                ..
            } => Some(texture_from_content),
        }
    }

    pub const fn input_sample_bounds(self) -> Option<Rect> {
        match self {
            Self::Empty => None,
            Self::Texture {
                input_sample_bounds,
                ..
            } => Some(input_sample_bounds),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PreparedExternalBackdrop {
    Color {
        working_linear_rec2020_premul: [f32; 4],
    },
    Blur {
        sigma_device_px: DynamicBindingId,
        sample: ExternalSample,
    },
}

/// Complete source-placement contract consumed by ImportPass on every backend.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ExternalPlacementWire", rename_all = "camelCase")]
pub struct ExternalPlacement {
    pub fit: Fit,
    /// Destination of the primary source quad in normalized clip-box coordinates.
    pub content_rect: Rect,
    /// Overflow boundary before layer scale/rotation. Currently the normalized clip box.
    pub clip_rect: Rect,
    pub sample: ExternalSample,
    pub backdrop: Option<PreparedExternalBackdrop>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExternalPlacementWire {
    fit: Fit,
    content_rect: Rect,
    clip_rect: Rect,
    sample: ExternalSample,
    backdrop: Option<PreparedExternalBackdrop>,
}

impl TryFrom<ExternalPlacementWire> for ExternalPlacement {
    type Error = ExternalPlacementError;

    fn try_from(value: ExternalPlacementWire) -> Result<Self, Self::Error> {
        let placement = Self {
            fit: value.fit,
            content_rect: value.content_rect,
            clip_rect: value.clip_rect,
            sample: value.sample,
            backdrop: value.backdrop,
        };
        placement.validate()?;
        Ok(placement)
    }
}

impl ExternalPlacement {
    pub(crate) fn validate(self) -> Result<(), ExternalPlacementError> {
        if self.clip_rect != UNIT_RECT {
            return Err(ExternalPlacementError::InvalidClipRect);
        }
        validate_rect(self.content_rect, true)?;
        if !rect_contains(self.clip_rect, self.content_rect) {
            return Err(ExternalPlacementError::InvalidContentRect);
        }
        let content_is_empty = self.content_rect.is_empty();
        if content_is_empty && (self.content_rect.width != 0.0 || self.content_rect.height != 0.0) {
            return Err(ExternalPlacementError::NonCanonicalEmptyRect);
        }
        validate_sample(self.sample)?;
        if self.sample.is_empty() != content_is_empty {
            return Err(ExternalPlacementError::SampleContentMismatch);
        }
        match self.backdrop {
            Some(PreparedExternalBackdrop::Color {
                working_linear_rec2020_premul,
            }) => {
                PremulRgba32::from_premultiplied(working_linear_rec2020_premul)
                    .map_err(|_| ExternalPlacementError::InvalidBackdrop)?;
            }
            Some(PreparedExternalBackdrop::Blur { sample, .. }) => {
                validate_sample(sample)?;
                if sample.is_empty() != self.sample.is_empty() {
                    return Err(ExternalPlacementError::InvalidBackdrop);
                }
            }
            None => {}
        }
        Ok(())
    }

    pub(crate) fn local_geometry(self) -> LocalGeometry {
        let content = if self.sample.is_empty() {
            LocalBounds::Empty
        } else {
            LocalBounds::from_rect(self.content_rect)
        };
        let backdrop_is_visible = match self.backdrop {
            Some(PreparedExternalBackdrop::Color { .. }) => true,
            Some(PreparedExternalBackdrop::Blur { sample, .. }) => !sample.is_empty(),
            None => false,
        };
        let output = if backdrop_is_visible {
            LocalBounds::from_rect(self.clip_rect)
        } else {
            content
        };
        LocalGeometry {
            viewport: self.clip_rect,
            content,
            output,
            scale_domain: output,
            max_intermediate_pixels: 0,
        }
    }
}

pub(crate) fn prepare_external_placement(
    asset: &SemanticAsset,
    source_crop: Crop,
    transform: &ResolvedTransform,
    mapping: CanvasMapping,
    device_scale: f64,
    semantic_path: &str,
    dynamic: &mut DynamicAllocator,
) -> Result<ExternalPlacement, ExternalPlacementError> {
    let extent = asset
        .descriptor
        .extent()
        .ok_or(ExternalPlacementError::MissingVisualDescriptor)?;
    let interpretation = asset
        .descriptor
        .visual_interpretation()
        .ok_or(ExternalPlacementError::MissingVisualDescriptor)?;
    validate_source_crop(source_crop)?;
    let box_width = transform.width * mapping.viewport_width_px;
    let box_height = transform.height * mapping.viewport_height_px;
    if ![box_width, box_height]
        .iter()
        .all(|value| value.is_finite())
        || box_width <= 0.0
        || box_height <= 0.0
    {
        return Err(ExternalPlacementError::InvalidBox);
    }
    let content_area = inset_rect(transform)?;
    let fit = transform.fit.unwrap_or_else(|| default_fit(asset.kind));
    let display_aspect = interpreted_display_aspect(extent, interpretation)?;
    let (content_rect, display_sample_rect) = fitted_rects(
        source_crop,
        display_aspect,
        content_area,
        [box_width, box_height],
        fit,
    )?;
    let sample = external_sample(display_sample_rect, interpretation)?;
    let backdrop = match transform.backdrop.as_ref() {
        None => None,
        Some(Backdrop::Color { color }) => {
            let rgba = color
                .parse_rgba()
                .ok_or(ExternalPlacementError::InvalidBackdropColor)?;
            Some(PreparedExternalBackdrop::Color {
                working_linear_rec2020_premul: decode_author_srgb(AuthorSrgbStraight(rgba))?
                    .channels(),
            })
        }
        Some(Backdrop::Blur { radius }) => {
            if *radius < 0 {
                return Err(ExternalPlacementError::InvalidBackdropRadius);
            }
            let (_, display_rect) = fitted_rects(
                source_crop,
                display_aspect,
                UNIT_RECT,
                [box_width, box_height],
                Fit::Cover,
            )?;
            let sample = external_sample(display_rect, interpretation)?;
            let sigma_device_px = dynamic.push(
                format!("{semantic_path}.backdrop.sigmaDevicePx"),
                DynamicBindingKind::DeviceLength,
                DynamicValue::Scalar(
                    *radius as f64 * device_scale * BACKDROP_BLUR_SIGMA_PER_RADIUS,
                ),
            )?;
            Some(PreparedExternalBackdrop::Blur {
                sigma_device_px,
                sample,
            })
        }
    };
    let placement = ExternalPlacement {
        fit,
        content_rect,
        clip_rect: UNIT_RECT,
        sample,
        backdrop,
    };
    placement.validate()?;
    Ok(placement)
}

fn validate_rect(rect: Rect, empty_allowed: bool) -> Result<(), ExternalPlacementError> {
    if ![rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
        || rect.width < 0.0
        || rect.height < 0.0
        || !rect.right().is_finite()
        || !rect.bottom().is_finite()
        || (!empty_allowed && rect.is_empty())
    {
        Err(ExternalPlacementError::InvalidContentRect)
    } else {
        Ok(())
    }
}

fn rect_contains(outer: Rect, inner: Rect) -> bool {
    inner.left() >= outer.left()
        && inner.top() >= outer.top()
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

fn validate_sample(sample: ExternalSample) -> Result<(), ExternalPlacementError> {
    let ExternalSample::Texture {
        texture_from_content,
        input_sample_bounds,
        sampling: ExternalSampling::LinearClampToSampleBounds,
    } = sample
    else {
        return Ok(());
    };
    if !texture_from_content.iter().all(|value| value.is_finite())
        || texture_from_content[6..] != [0.0, 0.0, 1.0]
    {
        return Err(ExternalPlacementError::InvalidSampleTransform);
    }
    let determinant = texture_from_content[0] * texture_from_content[4]
        - texture_from_content[1] * texture_from_content[3];
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(ExternalPlacementError::InvalidSampleTransform);
    }
    validate_rect(input_sample_bounds, false)
        .map_err(|_| ExternalPlacementError::InvalidSampleBounds)?;
    if !rect_contains(UNIT_RECT, input_sample_bounds) {
        return Err(ExternalPlacementError::InvalidSampleBounds);
    }
    let corners = sample_corners(texture_from_content);
    let left = corners
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let top = corners
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let right = corners
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = corners
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    if Rect::from_edges(left, top, right, bottom) != input_sample_bounds {
        return Err(ExternalPlacementError::InvalidSampleBounds);
    }
    Ok(())
}

fn sample_corners(matrix: [f64; 9]) -> [[f64; 2]; 4] {
    let map = |u: f64, v: f64| {
        [
            matrix[0] * u + matrix[1] * v + matrix[2],
            matrix[3] * u + matrix[4] * v + matrix[5],
        ]
    };
    [map(0.0, 0.0), map(1.0, 0.0), map(0.0, 1.0), map(1.0, 1.0)]
}

fn default_fit(kind: SemanticAssetKind) -> Fit {
    match kind {
        SemanticAssetKind::Video | SemanticAssetKind::Image => Fit::Cover,
        SemanticAssetKind::Lottie => Fit::Contain,
        SemanticAssetKind::Audio => Fit::Contain,
    }
}

fn inset_rect(transform: &ResolvedTransform) -> Result<Rect, ExternalPlacementError> {
    let inset = transform.inset;
    let values = [inset.top, inset.right, inset.bottom, inset.left];
    if !values
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    {
        return Err(ExternalPlacementError::InvalidInset);
    }
    Ok(UNIT_RECT.inset(inset.left, inset.top, inset.right, inset.bottom))
}

fn validate_source_crop(crop: Crop) -> Result<(), ExternalPlacementError> {
    if ![crop.x, crop.y, crop.width, crop.height]
        .iter()
        .all(|value| value.is_finite())
        || crop.x < 0.0
        || crop.y < 0.0
        || crop.width < 0.0
        || crop.height < 0.0
        || crop.x + crop.width > 1.0
        || crop.y + crop.height > 1.0
    {
        Err(ExternalPlacementError::InvalidSourceCrop)
    } else {
        Ok(())
    }
}

pub(crate) fn interpreted_display_extent(
    extent: crate::resource::Extent2d,
    interpretation: VisualInterpretation,
) -> Result<[f64; 2], ExternalPlacementError> {
    let intrinsic = interpretation.crop;
    let crop_width = fraction(intrinsic.right()) - fraction(intrinsic.left());
    let crop_height = fraction(intrinsic.bottom()) - fraction(intrinsic.top());
    let sar = ratio(interpretation.sample_aspect_ratio);
    let width = f64::from(extent.width()) * crop_width * sar;
    let height = f64::from(extent.height()) * crop_height;
    let display = if orientation_swaps_axes(interpretation.orientation) {
        [height, width]
    } else {
        [width, height]
    };
    if display
        .iter()
        .all(|value| value.is_finite() && *value > 0.0)
    {
        Ok(display)
    } else {
        Err(ExternalPlacementError::InvalidDisplayAspect)
    }
}

fn interpreted_display_aspect(
    extent: crate::resource::Extent2d,
    interpretation: VisualInterpretation,
) -> Result<f64, ExternalPlacementError> {
    let [width, height] = interpreted_display_extent(extent, interpretation)?;
    Ok(width / height)
}

fn fitted_rects(
    source_crop: Crop,
    display_aspect: f64,
    content_area: Rect,
    box_size: [f64; 2],
    fit: Fit,
) -> Result<(Rect, Option<Rect>), ExternalPlacementError> {
    if source_crop.width == 0.0 || source_crop.height == 0.0 || content_area.is_empty() {
        return Ok((Rect::new(content_area.x, content_area.y, 0.0, 0.0), None));
    }
    let source_aspect = display_aspect * source_crop.width / source_crop.height;
    let area_width_px = content_area.width * box_size[0];
    let area_height_px = content_area.height * box_size[1];
    let area_aspect = area_width_px / area_height_px;
    if !source_aspect.is_finite() || source_aspect <= 0.0 || !area_aspect.is_finite() {
        return Err(ExternalPlacementError::InvalidDisplayAspect);
    }
    match fit {
        Fit::Fill => Ok((content_area, Some(crop_rect(source_crop)))),
        Fit::Contain => {
            let (width_px, height_px) = if source_aspect >= area_aspect {
                (area_width_px, area_width_px / source_aspect)
            } else {
                (area_height_px * source_aspect, area_height_px)
            };
            let width = width_px / box_size[0];
            let height = height_px / box_size[1];
            Ok((
                Rect::new(
                    content_area.x + (content_area.width - width) * 0.5,
                    content_area.y + (content_area.height - height) * 0.5,
                    width,
                    height,
                ),
                Some(crop_rect(source_crop)),
            ))
        }
        Fit::Cover => Ok((
            content_area,
            Some(cover_source_rect(source_crop, source_aspect, area_aspect)),
        )),
    }
}

fn cover_source_rect(source: Crop, source_aspect: f64, target_aspect: f64) -> Rect {
    if source_aspect > target_aspect {
        let width = source.width * target_aspect / source_aspect;
        Rect::new(
            source.x + (source.width - width) * 0.5,
            source.y,
            width,
            source.height,
        )
    } else {
        let height = source.height * source_aspect / target_aspect;
        Rect::new(
            source.x,
            source.y + (source.height - height) * 0.5,
            source.width,
            height,
        )
    }
}

fn crop_rect(crop: Crop) -> Rect {
    Rect::new(crop.x, crop.y, crop.width, crop.height)
}

fn external_sample(
    display_rect: Option<Rect>,
    interpretation: VisualInterpretation,
) -> Result<ExternalSample, ExternalPlacementError> {
    let Some(display_rect) = display_rect else {
        return Ok(ExternalSample::Empty);
    };
    let corners = [
        display_to_texture([display_rect.left(), display_rect.top()], interpretation),
        display_to_texture([display_rect.right(), display_rect.top()], interpretation),
        display_to_texture([display_rect.left(), display_rect.bottom()], interpretation),
        display_to_texture(
            [display_rect.right(), display_rect.bottom()],
            interpretation,
        ),
    ];
    if !corners.iter().flatten().all(|value| value.is_finite()) {
        return Err(ExternalPlacementError::InvalidSampleTransform);
    }
    let origin = corners[0];
    let texture_from_content = [
        corners[1][0] - origin[0],
        corners[2][0] - origin[0],
        origin[0],
        corners[1][1] - origin[1],
        corners[2][1] - origin[1],
        origin[1],
        0.0,
        0.0,
        1.0,
    ];
    // The matrix is the executable source of truth. Derive its clamp domain from that exact
    // payload so subtraction/addition rounding cannot create two almost-equal sampling answers.
    let corners = sample_corners(texture_from_content);
    let left = corners
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let top = corners
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let right = corners
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = corners
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    Ok(ExternalSample::Texture {
        texture_from_content,
        input_sample_bounds: Rect::from_edges(left, top, right, bottom),
        sampling: ExternalSampling::LinearClampToSampleBounds,
    })
}

fn display_to_texture(point: [f64; 2], interpretation: VisualInterpretation) -> [f64; 2] {
    let [u, v] = point;
    let oriented = match interpretation.orientation {
        PixelOrientation::Identity => [u, v],
        PixelOrientation::MirrorHorizontal => [1.0 - u, v],
        PixelOrientation::Rotate180 => [1.0 - u, 1.0 - v],
        PixelOrientation::MirrorVertical => [u, 1.0 - v],
        PixelOrientation::MirrorHorizontalRotate270 => [v, u],
        PixelOrientation::Rotate90 => [v, 1.0 - u],
        PixelOrientation::MirrorHorizontalRotate90 => [1.0 - v, 1.0 - u],
        PixelOrientation::Rotate270 => [1.0 - v, u],
    };
    let crop = interpretation.crop;
    let left = fraction(crop.left());
    let top = fraction(crop.top());
    let width = fraction(crop.right()) - left;
    let height = fraction(crop.bottom()) - top;
    [left + oriented[0] * width, top + oriented[1] * height]
}

fn orientation_swaps_axes(orientation: PixelOrientation) -> bool {
    matches!(
        orientation,
        PixelOrientation::MirrorHorizontalRotate270
            | PixelOrientation::Rotate90
            | PixelOrientation::MirrorHorizontalRotate90
            | PixelOrientation::Rotate270
    )
}

fn fraction(value: UnitFraction) -> f64 {
    f64::from(value.numerator()) / f64::from(value.denominator())
}

fn ratio(value: SampleAspectRatio) -> f64 {
    f64::from(value.numerator()) / f64::from(value.denominator())
}

#[derive(Debug, Error)]
pub enum ExternalPlacementError {
    #[error("external source is missing a complete visual descriptor")]
    MissingVisualDescriptor,
    #[error("external source crop must be a finite canonical subset of [0, 1]²")]
    InvalidSourceCrop,
    #[error("external clip box must be finite and positive")]
    InvalidBox,
    #[error("external inset must contain finite values in [0, 1]")]
    InvalidInset,
    #[error("external interpreted display aspect is invalid")]
    InvalidDisplayAspect,
    #[error("external sample transform is not finite")]
    InvalidSampleTransform,
    #[error("external placement clip rect is not the canonical unit clip")]
    InvalidClipRect,
    #[error("external placement content rect is invalid or outside its clip")]
    InvalidContentRect,
    #[error("external placement uses a non-canonical empty rectangle")]
    NonCanonicalEmptyRect,
    #[error("external sample bounds are invalid or disagree with its UV transform")]
    InvalidSampleBounds,
    #[error("external sample emptiness disagrees with its content geometry")]
    SampleContentMismatch,
    #[error("external backdrop payload is invalid")]
    InvalidBackdrop,
    #[error("external backdrop color is invalid")]
    InvalidBackdropColor,
    #[error("external backdrop blur radius must be non-negative")]
    InvalidBackdropRadius,
    #[error(transparent)]
    Color(#[from] ColorMathError),
    #[error(transparent)]
    Dynamic(#[from] RequestError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{
        ColorDescription, Extent2d, InputAlphaMode, NormalizedCrop, SignalLuminance,
    };

    fn interpretation() -> VisualInterpretation {
        VisualInterpretation::new(
            ColorDescription::SRGB,
            SignalLuminance::SDR_100,
            InputAlphaMode::Opaque,
        )
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1.0e-12,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn cover_crops_the_interpreted_display_source() {
        let (destination, sample) = fitted_rects(
            Crop::default(),
            16.0 / 9.0,
            UNIT_RECT,
            [100.0, 100.0],
            Fit::Cover,
        )
        .unwrap();
        assert_eq!(destination, UNIT_RECT);
        assert_eq!(sample, Some(Rect::new(0.21875, 0.0, 0.5625, 1.0)));
    }

    #[test]
    fn contain_respects_inset_and_physical_box_aspect() {
        let area = UNIT_RECT.inset(0.1, 0.0, 0.1, 0.0);
        let (destination, sample) =
            fitted_rects(Crop::default(), 1.0, area, [200.0, 100.0], Fit::Contain).unwrap();
        assert_eq!(destination, Rect::new(0.25, 0.0, 0.5, 1.0));
        assert_eq!(sample, Some(UNIT_RECT));
    }

    #[test]
    fn descriptor_crop_and_rotation_map_to_raw_texture_uv() {
        let crop = NormalizedCrop::new(
            UnitFraction::new(1, 10).unwrap(),
            UnitFraction::new(1, 5).unwrap(),
            UnitFraction::new(9, 10).unwrap(),
            UnitFraction::new(4, 5).unwrap(),
        )
        .unwrap();
        let interpreted = interpretation()
            .with_orientation(PixelOrientation::Rotate90)
            .with_crop(crop);
        let sample = external_sample(Some(Rect::new(0.25, 0.0, 0.5, 1.0)), interpreted).unwrap();
        let ExternalSample::Texture {
            texture_from_content,
            input_sample_bounds,
            ..
        } = sample
        else {
            panic!("expected a texture sample")
        };
        for (actual, expected) in [
            (input_sample_bounds.x, 0.1),
            (input_sample_bounds.y, 0.35),
            (input_sample_bounds.width, 0.8),
            (input_sample_bounds.height, 0.3),
        ] {
            assert_close(actual, expected);
        }
        for (actual, expected) in texture_from_content
            .into_iter()
            .zip([0.0, 0.8, 0.1, -0.3, 0.0, 0.65, 0.0, 0.0, 1.0])
        {
            assert_close(actual, expected);
        }
        ExternalPlacement {
            fit: Fit::Fill,
            content_rect: UNIT_RECT,
            clip_rect: UNIT_RECT,
            sample,
            backdrop: None,
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn sar_is_applied_before_a_quarter_turn_orientation() {
        let interpreted = interpretation()
            .with_sample_aspect_ratio(SampleAspectRatio::new(2, 1).unwrap())
            .with_orientation(PixelOrientation::Rotate90);
        assert_eq!(
            interpreted_display_extent(Extent2d::new(100, 50).unwrap(), interpreted).unwrap(),
            [50.0, 200.0]
        );
        assert_eq!(
            interpreted_display_aspect(Extent2d::new(100, 50).unwrap(), interpreted).unwrap(),
            0.25
        );
    }

    #[test]
    fn program_intrinsic_extent_includes_descriptor_crop_sar_and_orientation() {
        let crop = NormalizedCrop::new(
            UnitFraction::new(1, 4).unwrap(),
            UnitFraction::new(1, 5).unwrap(),
            UnitFraction::new(3, 4).unwrap(),
            UnitFraction::new(4, 5).unwrap(),
        )
        .unwrap();
        let interpreted = interpretation()
            .with_crop(crop)
            .with_sample_aspect_ratio(SampleAspectRatio::new(3, 2).unwrap())
            .with_orientation(PixelOrientation::Rotate270);
        // Raw crop = 400×360, SAR makes it 600×360, quarter turn produces 360×600.
        let [width, height] =
            interpreted_display_extent(Extent2d::new(800, 600).unwrap(), interpreted).unwrap();
        assert_close(width, 360.0);
        assert_close(height, 600.0);
    }

    #[test]
    fn empty_source_crop_is_explicit_and_never_invents_a_sample() {
        let (destination, display) = fitted_rects(
            Crop {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            1.0,
            UNIT_RECT,
            [100.0, 100.0],
            Fit::Fill,
        )
        .unwrap();
        assert!(destination.is_empty());
        assert!(display.is_none());
        assert_eq!(
            external_sample(display, interpretation()).unwrap(),
            ExternalSample::Empty
        );
    }

    #[test]
    fn placement_wire_rejects_forged_uv_bounds_and_backdrop_pixels() {
        let placement = ExternalPlacement {
            fit: Fit::Fill,
            content_rect: UNIT_RECT,
            clip_rect: UNIT_RECT,
            sample: ExternalSample::Texture {
                texture_from_content: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                input_sample_bounds: UNIT_RECT,
                sampling: ExternalSampling::LinearClampToSampleBounds,
            },
            backdrop: None,
        };
        let wire = serde_json::to_value(placement).unwrap();
        assert_eq!(
            serde_json::from_value::<ExternalPlacement>(wire.clone()).unwrap(),
            placement
        );

        let mut forged_bounds = wire.clone();
        forged_bounds["sample"]["inputSampleBounds"]["width"] = serde_json::json!(0.5);
        assert!(serde_json::from_value::<ExternalPlacement>(forged_bounds).is_err());

        let mut projective_uv = wire.clone();
        projective_uv["sample"]["textureFromContent"][8] = serde_json::json!(0.0);
        assert!(serde_json::from_value::<ExternalPlacement>(projective_uv).is_err());

        let mut invalid_backdrop = wire;
        invalid_backdrop["backdrop"] = serde_json::json!({
            "kind": "color",
            "workingLinearRec2020Premul": [1.0, 0.0, 0.0, 0.0]
        });
        assert!(serde_json::from_value::<ExternalPlacement>(invalid_backdrop).is_err());
    }
}
