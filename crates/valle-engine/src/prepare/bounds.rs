use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::frame::{
    CanvasMapping, MaskShape, ResolvedCamera, ResolvedClipAnimation, ResolvedMask,
    ResolvedTransform,
};

use valle_draw::{
    Rect,
    requirements::{Insets, LocalBounds},
};

pub const MAX_DEVICE_INTERMEDIATE_PIXELS: u64 = 256 * 1024 * 1024;
pub(crate) const MAX_DEVICE_COORDINATE: f64 = 1_073_741_824.0;
const PERSPECTIVE_EPSILON: f64 = 1.0e-12;

/// Half-open device-space rectangle. Empty rectangles are explicit and remain valid for content
/// that is completely outside the output viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DeviceRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn full(width: u32, height: u32) -> Self {
        Self::new(0, 0, width, height)
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub const fn pixels(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    pub fn intersect(self, other: Self) -> Self {
        let left = i64::from(self.x).max(i64::from(other.x));
        let top = i64::from(self.y).max(i64::from(other.y));
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        if right <= left || bottom <= top {
            return Self::new(
                i32::try_from(left).unwrap_or(if left < 0 { i32::MIN } else { i32::MAX }),
                i32::try_from(top).unwrap_or(if top < 0 { i32::MIN } else { i32::MAX }),
                0,
                0,
            );
        }
        Self::new(
            i32::try_from(left).expect("intersection endpoint is inside an i32-backed rect"),
            i32::try_from(top).expect("intersection endpoint is inside an i32-backed rect"),
            u32::try_from(right - left).expect("positive intersection width fits u32"),
            u32::try_from(bottom - top).expect("positive intersection height fits u32"),
        )
    }

    pub fn outset_clamped(self, pixels: [f64; 4], root: Self) -> Result<Self, BoundsError> {
        if !pixels
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
        {
            return Err(BoundsError::InvalidOutset);
        }
        if self.is_empty() {
            return Ok(self);
        }
        let left = f64::from(self.x) - pixels[0];
        let top = f64::from(self.y) - pixels[1];
        let right = f64::from(self.x) + f64::from(self.width) + pixels[2];
        let bottom = f64::from(self.y) + f64::from(self.height) + pixels[3];
        rect_from_edges(left, top, right, bottom, root)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DeviceFloatRect {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl DeviceFloatRect {
    fn from_points(points: &[[f64; 2]; 4]) -> Result<Option<Self>, BoundsError> {
        if !points.iter().flatten().all(|value| value.is_finite()) {
            return Err(BoundsError::NonFiniteTransform);
        }
        let bounds = Self {
            left: points
                .iter()
                .map(|point| point[0])
                .fold(f64::INFINITY, f64::min),
            top: points
                .iter()
                .map(|point| point[1])
                .fold(f64::INFINITY, f64::min),
            right: points
                .iter()
                .map(|point| point[0])
                .fold(f64::NEG_INFINITY, f64::max),
            bottom: points
                .iter()
                .map(|point| point[1])
                .fold(f64::NEG_INFINITY, f64::max),
        };
        validate_device_edges(bounds.left, bounds.top, bounds.right, bounds.bottom)?;
        Ok((bounds.right > bounds.left && bounds.bottom > bounds.top).then_some(bounds))
    }

    fn outset(self, pixels: [f64; 4]) -> Result<Self, BoundsError> {
        if !pixels
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
        {
            return Err(BoundsError::InvalidOutset);
        }
        let result = Self {
            left: self.left - pixels[0],
            top: self.top - pixels[1],
            right: self.right + pixels[2],
            bottom: self.bottom + pixels[3],
        };
        validate_device_edges(result.left, result.top, result.right, result.bottom)?;
        Ok(result)
    }

    fn intersect(self, other: Self) -> Option<Self> {
        let result = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (result.right > result.left && result.bottom > result.top).then_some(result)
    }

    fn to_device(self, root: DeviceRect) -> Result<DeviceRect, BoundsError> {
        rect_from_edges(self.left, self.top, self.right, self.bottom, root)
    }
}

fn outset_projected(
    bounds: Option<DeviceFloatRect>,
    pixels: [f64; 4],
) -> Result<Option<DeviceFloatRect>, BoundsError> {
    bounds.map(|bounds| bounds.outset(pixels)).transpose()
}

fn projected_to_device(
    bounds: Option<DeviceFloatRect>,
    root: DeviceRect,
) -> Result<DeviceRect, BoundsError> {
    bounds.map_or(Ok(DeviceRect::new(0, 0, 0, 0)), |bounds| {
        bounds.to_device(root)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "DeviceTransformWire", rename_all = "camelCase")]
pub struct DeviceTransform {
    /// Row-major homography mapping normalized layer coordinates to device pixels.
    matrix: [f64; 9],
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeviceTransformWire {
    matrix: [f64; 9],
}

impl TryFrom<DeviceTransformWire> for DeviceTransform {
    type Error = BoundsError;

    fn try_from(value: DeviceTransformWire) -> Result<Self, Self::Error> {
        let transform = Self::from_projective(value.matrix)?;
        transform.validate_wire()?;
        Ok(transform)
    }
}

impl DeviceTransform {
    pub const fn from_affine(affine: [f64; 6]) -> Self {
        Self {
            matrix: [
                affine[0], affine[1], affine[2], affine[3], affine[4], affine[5], 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn from_projective(matrix: [f64; 9]) -> Result<Self, BoundsError> {
        if matrix.iter().all(|value| value.is_finite()) {
            Ok(Self { matrix })
        } else {
            Err(BoundsError::NonFiniteTransform)
        }
    }

    pub const fn matrix(self) -> [f64; 9] {
        self.matrix
    }

    /// Rejects a serialized homography before it can enter planning. Prepared transforms always
    /// map the normalized layer box, so a horizon or an over-budget coordinate anywhere on that
    /// box is invalid even when a later drawable happens not to touch the bad corner.
    pub(crate) fn validate_wire(self) -> Result<(), BoundsError> {
        let corners = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        validate_perspective_domain(self, &corners)?;
        let projected = [
            self.project(corners[0])?,
            self.project(corners[1])?,
            self.project(corners[2])?,
            self.project(corners[3])?,
        ];
        DeviceFloatRect::from_points(&projected)?;
        Ok(())
    }

    fn project(self, point: [f64; 2]) -> Result<[f64; 2], BoundsError> {
        let [x, y] = point;
        let w = self.matrix[6] * x + self.matrix[7] * y + self.matrix[8];
        if !w.is_finite() || w == 0.0 {
            return Err(BoundsError::PerspectiveHorizon);
        }
        let projected = [
            (self.matrix[0] * x + self.matrix[1] * y + self.matrix[2]) / w,
            (self.matrix[3] * x + self.matrix[4] * y + self.matrix[5]) / w,
        ];
        if projected.iter().all(|value| value.is_finite()) {
            Ok(projected)
        } else {
            Err(BoundsError::NonFiniteTransform)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundsReason {
    Exact,
    ConservativeCameraTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PreparedBoundsWire", rename_all = "camelCase")]
pub struct PreparedBounds {
    pub content: DeviceRect,
    pub output: DeviceRect,
    pub sample: DeviceRect,
    pub reason: BoundsReason,
    pub max_intermediate_pixels: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedBoundsWire {
    content: DeviceRect,
    output: DeviceRect,
    sample: DeviceRect,
    reason: BoundsReason,
    max_intermediate_pixels: u64,
}

impl TryFrom<PreparedBoundsWire> for PreparedBounds {
    type Error = BoundsError;

    fn try_from(value: PreparedBoundsWire) -> Result<Self, Self::Error> {
        let bounds = Self {
            content: value.content,
            output: value.output,
            sample: value.sample,
            reason: value.reason,
            max_intermediate_pixels: value.max_intermediate_pixels,
        };
        bounds.validate_wire()?;
        Ok(bounds)
    }
}

impl PreparedBounds {
    pub(crate) fn validate_wire(self) -> Result<(), BoundsError> {
        for rect in [self.content, self.output, self.sample] {
            validate_prepared_rect(rect)?;
            if rect.pixels() > self.max_intermediate_pixels {
                return Err(BoundsError::InvalidPreparedBounds);
            }
        }
        enforce_intermediate_budget(self.max_intermediate_pixels)?;
        if self.sample != self.output {
            return Err(BoundsError::InvalidPreparedBounds);
        }
        if self.reason == BoundsReason::ConservativeCameraTarget
            && (self.content != self.output || self.max_intermediate_pixels != self.output.pixels())
        {
            return Err(BoundsError::InvalidPreparedBounds);
        }
        Ok(())
    }
}

fn validate_prepared_rect(rect: DeviceRect) -> Result<(), BoundsError> {
    if rect.is_empty() {
        return if rect == DeviceRect::new(0, 0, 0, 0) {
            Ok(())
        } else {
            Err(BoundsError::InvalidPreparedBounds)
        };
    }
    let right = i64::from(rect.x) + i64::from(rect.width);
    let bottom = i64::from(rect.y) + i64::from(rect.height);
    if rect.x < 0
        || rect.y < 0
        || right > MAX_DEVICE_COORDINATE as i64
        || bottom > MAX_DEVICE_COORDINATE as i64
    {
        Err(BoundsError::InvalidPreparedBounds)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LocalGeometry {
    pub viewport: Rect,
    pub content: LocalBounds,
    pub output: LocalBounds,
    pub scale_domain: LocalBounds,
    pub max_intermediate_pixels: u64,
}

pub(crate) struct LayerGeometryInput<'a> {
    pub transform: &'a ResolvedTransform,
    pub animation: &'a ResolvedClipAnimation,
    pub camera: Option<&'a ResolvedCamera>,
    pub mask: Option<&'a super::PreparedMask>,
    pub mapping: CanvasMapping,
    pub output: [u32; 2],
    pub geometry: Option<LocalGeometry>,
}

fn validate_layer_inputs(
    transform: &ResolvedTransform,
    animation: &ResolvedClipAnimation,
    camera: Option<&ResolvedCamera>,
    mask: Option<&super::PreparedMask>,
    mapping: CanvasMapping,
) -> Result<(), BoundsError> {
    let transform_values = [
        transform.x,
        transform.y,
        transform.width,
        transform.height,
        transform.scale,
        transform.rotation,
        transform.opacity,
        transform.inset.top,
        transform.inset.right,
        transform.inset.bottom,
        transform.inset.left,
    ];
    if !transform_values.iter().all(|value| value.is_finite())
        || transform.width < 0.0
        || transform.height < 0.0
        || transform.scale < 0.0
        || !(0.0..=1.0).contains(&transform.opacity)
        || [
            transform.inset.top,
            transform.inset.right,
            transform.inset.bottom,
            transform.inset.left,
        ]
        .iter()
        .any(|value| !(0.0..=1.0).contains(value))
    {
        return Err(BoundsError::InvalidGeometryInput);
    }
    if let Some(crop) = transform.crop
        && (![crop.x, crop.y, crop.width, crop.height]
            .iter()
            .all(|value| value.is_finite())
            || crop.width <= 0.0
            || crop.height <= 0.0)
    {
        return Err(BoundsError::InvalidGeometryInput);
    }
    let animation_values = [
        animation.scale,
        animation.opacity,
        animation.rotation_deg,
        animation.translate_x_px,
        animation.translate_y_px,
        animation.blur_sigma_px,
    ];
    if !animation_values.iter().all(|value| value.is_finite())
        || animation.scale < 0.0
        || !(0.0..=1.0).contains(&animation.opacity)
        || animation.blur_sigma_px < 0.0
    {
        return Err(BoundsError::InvalidGeometryInput);
    }
    if let Some(inset) = animation.clip_inset
        && !inset
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    {
        return Err(BoundsError::InvalidClipInset);
    }
    let mapping_values = [
        mapping.scale,
        mapping.offset_x_px,
        mapping.offset_y_px,
        mapping.viewport_width_px,
        mapping.viewport_height_px,
    ];
    if !mapping_values.iter().all(|value| value.is_finite())
        || mapping.scale <= 0.0
        || mapping.viewport_width_px <= 0.0
        || mapping.viewport_height_px <= 0.0
    {
        return Err(BoundsError::InvalidGeometryInput);
    }
    if let Some(camera) = camera {
        if ![
            camera.center_x,
            camera.center_y,
            camera.zoom,
            camera.rotation,
        ]
        .iter()
        .all(|value| value.is_finite())
            || camera.zoom <= 0.0
            || camera
                .target
                .as_ref()
                .and_then(|target| target.margin)
                .is_some_and(|margin| !margin.is_finite() || !(0.0..0.5).contains(&margin))
        {
            return Err(BoundsError::InvalidGeometryInput);
        }
    }
    if let Some(mask) = mask {
        mask.validate_wire()?;
    }
    Ok(())
}

/// Exact normalized-layer → device transform, independent of the layer's content geometry.
/// Temporal material qualification must run before a DrawProgram exists, so it shares this
/// function with [`layer_geometry`] instead of maintaining a second placement formula.
pub(crate) fn layer_device_transform(
    transform: &ResolvedTransform,
    animation: &ResolvedClipAnimation,
    camera: Option<&ResolvedCamera>,
    mapping: CanvasMapping,
    output: [u32; 2],
) -> Result<DeviceTransform, BoundsError> {
    validate_layer_inputs(transform, animation, camera, None, mapping)?;
    if camera.is_some_and(|value| value.target.is_some()) {
        return Ok(DeviceTransform::from_affine([
            f64::from(output[0]),
            0.0,
            0.0,
            0.0,
            f64::from(output[1]),
            0.0,
        ]));
    }
    let base_width = transform.width * mapping.viewport_width_px;
    let base_height = transform.height * mapping.viewport_height_px;
    let (anchor_x, anchor_y) = crate::frame::anchor_fraction(transform.anchor);
    let scale = transform.scale * f64::from(animation.scale);
    let sx = if transform.flip_x { -scale } else { scale };
    let sy = if transform.flip_y { -scale } else { scale };
    let (sin, cos) = sin_cos_degrees(transform.rotation + f64::from(animation.rotation_deg));
    let a = cos * base_width * sx;
    let b = sin * base_width * sx;
    let c = -sin * base_height * sy;
    let d = cos * base_height * sy;
    let rotated_width = a.abs() + c.abs();
    let rotated_height = b.abs() + d.abs();
    let anchor_point = [
        mapping.offset_x_px + transform.x * mapping.viewport_width_px,
        mapping.offset_y_px + transform.y * mapping.viewport_height_px,
    ];
    let center = [
        anchor_point[0] + (0.5 - anchor_x) * rotated_width,
        anchor_point[1] + (0.5 - anchor_y) * rotated_height,
    ];
    let translated_center = [
        center[0] + f64::from(animation.translate_x_px) * mapping.scale,
        center[1] + f64::from(animation.translate_y_px) * mapping.scale,
    ];
    let mut affine = [
        a,
        c,
        translated_center[0] - (a + c) / 2.0,
        b,
        d,
        translated_center[1] - (b + d) / 2.0,
    ];
    if let Some(camera) = camera {
        affine = apply_camera(affine, camera, mapping);
    }
    DeviceTransform::from_projective([
        affine[0], affine[1], affine[2], affine[3], affine[4], affine[5], 0.0, 0.0, 1.0,
    ])
}

pub(crate) fn layer_geometry(
    input: LayerGeometryInput<'_>,
) -> Result<(DeviceTransform, PreparedBounds), BoundsError> {
    let LayerGeometryInput {
        transform,
        animation,
        camera,
        mask,
        mapping,
        output,
        geometry,
    } = input;
    let root = DeviceRect::full(output[0], output[1]);
    validate_layer_inputs(transform, animation, camera, mask, mapping)?;
    enforce_intermediate_budget(root.pixels())?;
    if camera.is_some_and(|value| value.target.is_some()) {
        return Ok((
            DeviceTransform::from_affine([
                f64::from(output[0]),
                0.0,
                0.0,
                0.0,
                f64::from(output[1]),
                0.0,
            ]),
            PreparedBounds {
                content: root,
                output: root,
                sample: root,
                reason: BoundsReason::ConservativeCameraTarget,
                max_intermediate_pixels: root.pixels(),
            },
        ));
    }

    let device_transform = layer_device_transform(transform, animation, camera, mapping, output)?;
    let (viewport, mut local_content, mut local_output) = match geometry {
        Some(geometry) => (geometry.viewport, geometry.content, geometry.output),
        None => {
            let viewport = Rect::new(0.0, 0.0, 1.0, 1.0);
            let bounds = LocalBounds::from_rect(viewport);
            (viewport, bounds, bounds)
        }
    };
    if let Some(inset) = animation.clip_inset {
        let clip = LocalBounds::from_rect(clip_inset_rect(viewport, inset)?);
        local_content = intersect_local_bounds(local_content, clip);
        local_output = intersect_local_bounds(local_output, clip);
    }
    let content_projected = project_local_bounds(local_content, viewport, device_transform)?;
    let content = projected_to_device(content_projected, root)?;
    let mut output_projected = project_local_bounds(local_output, viewport, device_transform)?;
    let mut max_intermediate_pixels = projected_to_device(output_projected, root)?.pixels();
    if let Some(geometry) = geometry {
        let scale = geometry
            .scale_domain
            .rect()
            .map(|domain| max_local_scale(device_transform, domain, viewport))
            .transpose()?
            .unwrap_or(0.0);
        max_intermediate_pixels = max_intermediate_pixels.max(scale_intermediate_pixels(
            geometry.max_intermediate_pixels,
            scale,
        )?);
    }
    if animation.blur_sigma_px > 0.0 {
        let radius = f64::from(animation.blur_sigma_px) * mapping.scale * 3.0;
        output_projected = outset_projected(output_projected, [radius; 4])?;
        max_intermediate_pixels =
            max_intermediate_pixels.max(projected_to_device(output_projected, root)?.pixels());
    }
    if let Some(mask) = mask
        && !mask.invert()
    {
        output_projected = match (output_projected, mask_bounds(mask)?) {
            (Some(output), Some(mask)) => output.intersect(mask),
            _ => None,
        };
    }
    let output_bounds = projected_to_device(output_projected, root)?;
    enforce_intermediate_budget(max_intermediate_pixels)?;
    Ok((
        device_transform,
        PreparedBounds {
            content,
            output: output_bounds,
            sample: output_bounds,
            reason: BoundsReason::Exact,
            max_intermediate_pixels,
        },
    ))
}

pub(crate) fn program_rect_to_device(
    rect: Rect,
    footprint: Insets,
    viewport: Rect,
    transform: DeviceTransform,
    root: DeviceRect,
) -> Result<(DeviceRect, DeviceRect), BoundsError> {
    let projected = project_local_rect_unclamped(rect, viewport, transform)?;
    let output = projected_to_device(projected, root)?;
    let scale = max_local_scale(transform, rect, viewport)?;
    let sample = projected_to_device(
        outset_projected(
            projected,
            [
                f64::from(footprint.left) * scale,
                f64::from(footprint.top) * scale,
                f64::from(footprint.right) * scale,
                f64::from(footprint.bottom) * scale,
            ],
        )?,
        root,
    )?;
    Ok((sample, output))
}

pub(crate) fn program_bounds_to_device(
    bounds: LocalBounds,
    viewport: Rect,
    transform: DeviceTransform,
    root: DeviceRect,
) -> Result<DeviceRect, BoundsError> {
    projected_to_device(project_local_bounds(bounds, viewport, transform)?, root)
}

fn project_local_bounds(
    bounds: LocalBounds,
    viewport: Rect,
    transform: DeviceTransform,
) -> Result<Option<DeviceFloatRect>, BoundsError> {
    bounds.rect().map_or(Ok(None), |rect| {
        project_local_rect_unclamped(rect, viewport, transform)
    })
}

#[cfg(test)]
fn project_local_rect(
    rect: Rect,
    viewport: Rect,
    transform: DeviceTransform,
    root: DeviceRect,
) -> Result<DeviceRect, BoundsError> {
    projected_to_device(
        project_local_rect_unclamped(rect, viewport, transform)?,
        root,
    )
}

fn project_local_rect_unclamped(
    rect: Rect,
    viewport: Rect,
    transform: DeviceTransform,
) -> Result<Option<DeviceFloatRect>, BoundsError> {
    validate_viewport(viewport)?;
    if rect.is_empty() {
        return Ok(None);
    }
    let corners = normalized_corners(rect, viewport);
    validate_perspective_domain(transform, &corners)?;
    let projected = [
        transform.project(corners[0])?,
        transform.project(corners[1])?,
        transform.project(corners[2])?,
        transform.project(corners[3])?,
    ];
    DeviceFloatRect::from_points(&projected)
}

fn max_local_scale(
    transform: DeviceTransform,
    rect: Rect,
    viewport: Rect,
) -> Result<f64, BoundsError> {
    validate_viewport(viewport)?;
    if rect.is_empty() {
        return Ok(0.0);
    }
    let corners = normalized_corners(rect, viewport);
    validate_perspective_domain(transform, &corners)?;
    let minimum_w = corners
        .iter()
        .map(|[x, y]| {
            (transform.matrix[6] * x + transform.matrix[7] * y + transform.matrix[8]).abs()
        })
        .fold(f64::INFINITY, f64::min);
    let denominator = minimum_w * minimum_w;
    let [a, b, c, d, e, f, g, h, i] = transform.matrix;
    let [u0, v0] = corners[0];
    let [u1, v1] = corners[3];
    let max_linear = |constant: f64, coefficient: f64, low: f64, high: f64| {
        (constant + coefficient * low)
            .abs()
            .max((constant + coefficient * high).abs())
    };
    let dx_du = max_linear(a * i - g * c, a * h - g * b, v0, v1) / denominator;
    let dx_dv = max_linear(b * i - h * c, b * g - h * a, u0, u1) / denominator;
    let dy_du = max_linear(d * i - g * f, d * h - g * e, v0, v1) / denominator;
    let dy_dv = max_linear(e * i - h * f, e * g - h * d, u0, u1) / denominator;
    let inverse_width = viewport.width.recip();
    let inverse_height = viewport.height.recip();
    let dx_du = dx_du * inverse_width;
    let dy_du = dy_du * inverse_width;
    let dx_dv = dx_dv * inverse_height;
    let dy_dv = dy_dv * inverse_height;
    let scale = if g == 0.0 && h == 0.0 {
        operator_norm_2x2(
            (a / i) * inverse_width,
            (b / i) * inverse_height,
            (d / i) * inverse_width,
            (e / i) * inverse_height,
        )
    } else {
        // Each component above is an independent absolute upper bound over the domain. The
        // induced 1/∞ norm inequality remains conservative without inventing correlated signs.
        let one_norm = (dx_du + dy_du).max(dx_dv + dy_dv);
        let infinity_norm = (dx_du + dx_dv).max(dy_du + dy_dv);
        valle_draw::math::sqrt(one_norm * infinity_norm)
    };
    if scale.is_finite() {
        Ok(scale)
    } else {
        Err(BoundsError::NonFiniteTransform)
    }
}

fn operator_norm_2x2(a: f64, b: f64, c: f64, d: f64) -> f64 {
    let squared_sum = a * a + b * b + c * c + d * d;
    let determinant = a * d - b * c;
    let discriminant = (squared_sum * squared_sum - 4.0 * determinant * determinant).max(0.0);
    valle_draw::math::sqrt((squared_sum + valle_draw::math::sqrt(discriminant)) * 0.5)
}

fn normalized_corners(rect: Rect, viewport: Rect) -> [[f64; 2]; 4] {
    let normalize = |x: f64, y: f64| {
        [
            (x - viewport.x) / viewport.width,
            (y - viewport.y) / viewport.height,
        ]
    };
    [
        normalize(rect.left(), rect.top()),
        normalize(rect.right(), rect.top()),
        normalize(rect.left(), rect.bottom()),
        normalize(rect.right(), rect.bottom()),
    ]
}

fn validate_perspective_domain(
    transform: DeviceTransform,
    corners: &[[f64; 2]; 4],
) -> Result<(), BoundsError> {
    let denominators = corners
        .map(|[x, y]| transform.matrix[6] * x + transform.matrix[7] * y + transform.matrix[8]);
    if !denominators.iter().all(|value| value.is_finite()) {
        return Err(BoundsError::PerspectiveHorizon);
    }
    let maximum = denominators
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f64, f64::max);
    if maximum == 0.0 {
        return Err(BoundsError::PerspectiveHorizon);
    }
    let tolerance = maximum * PERSPECTIVE_EPSILON;
    let mut sign = 0.0_f64;
    for w in denominators {
        if w.abs() <= tolerance {
            return Err(BoundsError::PerspectiveHorizon);
        }
        let current = w.signum();
        if sign == 0.0 {
            sign = current;
        } else if current != sign {
            return Err(BoundsError::PerspectiveHorizon);
        }
    }
    Ok(())
}

fn validate_viewport(viewport: Rect) -> Result<(), BoundsError> {
    if [viewport.x, viewport.y, viewport.width, viewport.height]
        .iter()
        .all(|value| value.is_finite())
        && viewport.width > 0.0
        && viewport.height > 0.0
    {
        Ok(())
    } else {
        Err(BoundsError::InvalidViewport)
    }
}

fn intersect_local_bounds(left: LocalBounds, right: LocalBounds) -> LocalBounds {
    let (Some(left), Some(right)) = (left.rect(), right.rect()) else {
        return LocalBounds::Empty;
    };
    LocalBounds::from_rect(Rect::from_edges(
        left.left().max(right.left()),
        left.top().max(right.top()),
        left.right().min(right.right()),
        left.bottom().min(right.bottom()),
    ))
}

fn scale_intermediate_pixels(local: u64, scale: f64) -> Result<u64, BoundsError> {
    let pixels = (local as f64) * scale * scale;
    if !pixels.is_finite() || pixels > u64::MAX as f64 {
        return Err(BoundsError::BudgetExceeded);
    }
    Ok(pixels.ceil() as u64)
}

fn enforce_intermediate_budget(pixels: u64) -> Result<(), BoundsError> {
    if pixels > MAX_DEVICE_INTERMEDIATE_PIXELS {
        Err(BoundsError::IntermediateBudgetExceeded {
            actual: pixels,
            limit: MAX_DEVICE_INTERMEDIATE_PIXELS,
        })
    } else {
        Ok(())
    }
}

pub(crate) fn prepare_mask(
    mask: &ResolvedMask,
    camera: Option<&ResolvedCamera>,
    mapping: CanvasMapping,
) -> Result<super::PreparedMask, BoundsError> {
    if ![
        mask.x,
        mask.y,
        mask.width,
        mask.height,
        mask.feather,
        mask.rotation,
    ]
    .iter()
    .all(|value| value.is_finite())
        || mask.width <= 0.0
        || mask.height <= 0.0
        || mask.feather < 0.0
    {
        return Err(BoundsError::InvalidGeometryInput);
    }
    let width = mask.width * mapping.viewport_width_px;
    let height = mask.height * mapping.viewport_height_px;
    let center = [
        mapping.offset_x_px + mask.x * mapping.viewport_width_px,
        mapping.offset_y_px + mask.y * mapping.viewport_height_px,
    ];
    let (sin, cos) = sin_cos_degrees(mask.rotation);
    let mut affine = [
        cos * width,
        -sin * height,
        center[0] - (cos * width - sin * height) / 2.0,
        sin * width,
        cos * height,
        center[1] - (sin * width + cos * height) / 2.0,
    ];
    if let Some(camera) = camera {
        affine = apply_camera(affine, camera, mapping);
    }
    let camera_zoom = camera.map_or(1.0, |camera| camera.zoom);
    let authored_radius_device_px =
        mask.feather * mapping.viewport_width_px.min(mapping.viewport_height_px) * camera_zoom;
    super::PreparedMask::new(
        match mask.kind {
            MaskShape::Rect => super::PreparedMaskShape::Rect,
            MaskShape::Ellipse => super::PreparedMaskShape::Ellipse,
        },
        DeviceTransform::from_affine(affine),
        authored_radius_device_px * super::MASK_FEATHER_SIGMA_PER_RADIUS,
        mask.invert,
    )
}

fn mask_bounds(mask: &super::PreparedMask) -> Result<Option<DeviceFloatRect>, BoundsError> {
    let transform = mask.device_from_mask();
    let unit = Rect::new(0.0, 0.0, 1.0, 1.0);
    let bounds = project_local_rect_unclamped(unit, unit, transform)?;
    outset_projected(bounds, [mask.support_radius_device_px(); 4])
}

fn apply_camera(affine: [f64; 6], camera: &ResolvedCamera, mapping: CanvasMapping) -> [f64; 6] {
    let camera_center = [
        mapping.offset_x_px + camera.center_x * mapping.viewport_width_px,
        mapping.offset_y_px + camera.center_y * mapping.viewport_height_px,
    ];
    let canvas_center = [
        mapping.offset_x_px + mapping.viewport_width_px * 0.5,
        mapping.offset_y_px + mapping.viewport_height_px * 0.5,
    ];
    let (sin, cos) = sin_cos_degrees(camera.rotation);
    let a = cos * camera.zoom;
    let b = sin * camera.zoom;
    let c = -sin * camera.zoom;
    let d = cos * camera.zoom;
    let tx = canvas_center[0] - a * camera_center[0] - c * camera_center[1];
    let ty = canvas_center[1] - b * camera_center[0] - d * camera_center[1];
    [
        a * affine[0] + c * affine[3],
        a * affine[1] + c * affine[4],
        a * affine[2] + c * affine[5] + tx,
        b * affine[0] + d * affine[3],
        b * affine[1] + d * affine[4],
        b * affine[2] + d * affine[5] + ty,
    ]
}

fn sin_cos_degrees(degrees: f64) -> (f64, f64) {
    match degrees.rem_euclid(360.0) {
        0.0 => (0.0, 1.0),
        90.0 => (1.0, 0.0),
        180.0 => (0.0, -1.0),
        270.0 => (-1.0, 0.0),
        value => valle_draw::math::sin_cos(value.to_radians()),
    }
}

fn clip_inset_rect(bounds: Rect, inset: [f32; 4]) -> Result<Rect, BoundsError> {
    if !inset
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    {
        return Err(BoundsError::InvalidClipInset);
    }
    let left = bounds.x + bounds.width * f64::from(inset[3]);
    let top = bounds.y + bounds.height * f64::from(inset[0]);
    let right = bounds.right() - bounds.width * f64::from(inset[1]);
    let bottom = bounds.bottom() - bounds.height * f64::from(inset[2]);
    if right <= left || bottom <= top {
        return Ok(Rect::new(bounds.x, bounds.y, 0.0, 0.0));
    }
    Ok(Rect::from_edges(left, top, right, bottom))
}

fn rect_from_edges(
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    root: DeviceRect,
) -> Result<DeviceRect, BoundsError> {
    validate_device_edges(left, top, right, bottom)?;
    let root_right = f64::from(root.x) + f64::from(root.width);
    let root_bottom = f64::from(root.y) + f64::from(root.height);
    let left = left.floor().max(f64::from(root.x)).min(root_right);
    let top = top.floor().max(f64::from(root.y)).min(root_bottom);
    let right = right.ceil().max(f64::from(root.x)).min(root_right);
    let bottom = bottom.ceil().max(f64::from(root.y)).min(root_bottom);
    if right <= left || bottom <= top {
        return Ok(DeviceRect::new(0, 0, 0, 0));
    }
    if left < f64::from(i32::MIN)
        || top < f64::from(i32::MIN)
        || right > f64::from(i32::MAX)
        || bottom > f64::from(i32::MAX)
    {
        return Err(BoundsError::BudgetExceeded);
    }
    Ok(DeviceRect::new(
        left as i32,
        top as i32,
        (right - left) as u32,
        (bottom - top) as u32,
    ))
}

fn validate_device_edges(left: f64, top: f64, right: f64, bottom: f64) -> Result<(), BoundsError> {
    if ![left, top, right, bottom]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(BoundsError::NonFiniteTransform);
    }
    if [left, top, right, bottom]
        .iter()
        .any(|value| value.abs() > MAX_DEVICE_COORDINATE)
    {
        return Err(BoundsError::BudgetExceeded);
    }
    Ok(())
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum BoundsError {
    #[error("device transform or projected bounds is not finite")]
    NonFiniteTransform,
    #[error("projective transform crosses or touches the perspective horizon")]
    PerspectiveHorizon,
    #[error("program viewport must be finite with positive axes")]
    InvalidViewport,
    #[error("layer geometry input is non-finite or outside its closed semantic range")]
    InvalidGeometryInput,
    #[error("spatial footprint is unbounded")]
    Unbounded,
    #[error("spatial outset is negative or non-finite")]
    InvalidOutset,
    #[error("effect parameters cannot be frozen into a bounded device-space kernel")]
    InvalidEffect,
    #[error("animation clip inset is outside the normalized range")]
    InvalidClipInset,
    #[error("projected bounds exceeds the supported device coordinate budget")]
    BudgetExceeded,
    #[error("device intermediate pixel budget exceeded: {actual} > {limit}")]
    IntermediateBudgetExceeded { actual: u64, limit: u64 },
    #[error("prepared bounds are non-canonical or outside their declared budget")]
    InvalidPreparedBounds,
    #[error("prepared mask is non-canonical or outside the device budget")]
    InvalidPreparedMask,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Anchor, LayerInset, MaskShape};

    fn resolved_transform() -> ResolvedTransform {
        ResolvedTransform {
            x: 0.5,
            y: 0.5,
            width: 1.0,
            height: 1.0,
            anchor: Anchor::Center,
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
            fit: None,
            crop: None,
            inset: LayerInset::default(),
            flip_x: false,
            flip_y: false,
            backdrop: None,
        }
    }

    fn mapping(width: f64, height: f64) -> CanvasMapping {
        CanvasMapping {
            scale: 1.0,
            offset_x_px: 0.0,
            offset_y_px: 0.0,
            viewport_width_px: width,
            viewport_height_px: height,
        }
    }

    fn affine_point(affine: [f64; 6], point: [f64; 2]) -> [f64; 2] {
        [
            affine[0] * point[0] + affine[1] * point[1] + affine[2],
            affine[3] * point[0] + affine[4] * point[1] + affine[5],
        ]
    }

    #[test]
    fn camera_maps_its_authored_center_to_canvas_center() {
        let camera = ResolvedCamera {
            center_x: 0.25,
            center_y: 0.75,
            zoom: 2.0,
            rotation: 0.0,
            target: None,
        };
        let affine = apply_camera(
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            &camera,
            mapping(200.0, 100.0),
        );
        assert_eq!(affine_point(affine, [50.0, 75.0]), [100.0, 50.0]);
        assert_eq!(affine_point(affine, [75.0, 75.0]), [150.0, 50.0]);
    }

    #[test]
    fn inverse_clockwise_camera_rotation_moves_world_right_up() {
        let camera = ResolvedCamera {
            center_x: 0.5,
            center_y: 0.5,
            zoom: 1.0,
            // The compiled-render adapter negates an authored clockwise +90° camera.
            rotation: -90.0,
            target: None,
        };
        let affine = apply_camera(
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            &camera,
            mapping(200.0, 100.0),
        );
        assert_eq!(affine_point(affine, [150.0, 50.0]), [100.0, 0.0]);
    }

    #[test]
    fn mask_geometry_and_feather_are_frozen_in_device_space() {
        let mask = ResolvedMask {
            kind: MaskShape::Ellipse,
            x: 0.5,
            y: 0.5,
            width: 0.5,
            height: 0.4,
            feather: 0.1,
            rotation: 0.0,
            invert: false,
        };
        let camera = ResolvedCamera {
            center_x: 0.5,
            center_y: 0.5,
            zoom: 2.0,
            rotation: 0.0,
            target: None,
        };
        let prepared = prepare_mask(&mask, Some(&camera), mapping(200.0, 100.0)).unwrap();

        assert_eq!(prepared.shape(), super::super::PreparedMaskShape::Ellipse);
        assert_eq!(
            prepared.device_from_mask().matrix(),
            [200.0, 0.0, 0.0, 0.0, 80.0, 10.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(prepared.feather_sigma_device_px(), 10.0);
        assert_eq!(prepared.support_radius_device_px(), 40.0);
        assert!(!prepared.invert());
    }

    #[test]
    fn prepared_mask_remains_an_orthogonal_device_shape_after_two_rotations() {
        let mask = ResolvedMask {
            kind: MaskShape::Rect,
            x: 0.4,
            y: 0.6,
            width: 0.7,
            height: 0.3,
            feather: 0.02,
            rotation: 37.0,
            invert: false,
        };
        let camera = ResolvedCamera {
            center_x: 0.5,
            center_y: 0.5,
            zoom: 1.7,
            rotation: 23.0,
            target: None,
        };

        let prepared = prepare_mask(&mask, Some(&camera), mapping(3840.0, 2160.0)).unwrap();
        let geometry = prepared.geometry().unwrap();
        let dot = geometry.axis_x[0] * geometry.axis_y[0] + geometry.axis_x[1] * geometry.axis_y[1];
        assert!(dot.abs() < 1.0e-12);
        assert!(
            geometry
                .half_extent_device_px
                .iter()
                .all(|value| *value > 0.0)
        );
    }

    #[test]
    fn half_open_intersection_and_outset_are_clamped() {
        let root = DeviceRect::full(100, 80);
        let rect = DeviceRect::new(10, 20, 30, 40);
        assert_eq!(
            rect.outset_clamped([20.0; 4], root).unwrap(),
            DeviceRect::new(0, 0, 60, 80)
        );
        assert_eq!(
            rect.intersect(DeviceRect::new(30, 0, 30, 30)),
            DeviceRect::new(30, 20, 10, 10)
        );
    }

    #[test]
    fn affine_identity_preserves_local_pixels_and_backdrop_footprint() {
        let viewport = Rect::new(0.0, 0.0, 64.0, 48.0);
        let transform = DeviceTransform::from_affine([64.0, 0.0, 0.0, 0.0, 48.0, 0.0]);
        assert_eq!(max_local_scale(transform, viewport, viewport).unwrap(), 1.0);
        assert_eq!(
            program_rect_to_device(
                Rect::new(8.0, 6.0, 24.0, 12.0),
                Insets::uniform(2.0),
                viewport,
                transform,
                DeviceRect::full(64, 48),
            )
            .unwrap(),
            (DeviceRect::new(6, 4, 28, 16), DeviceRect::new(8, 6, 24, 12),)
        );
    }

    #[test]
    fn non_center_anchor_pins_the_post_rotation_aabb() {
        let mut transform = resolved_transform();
        transform.x = 0.0;
        transform.y = 0.0;
        transform.width = 0.5;
        transform.height = 0.5;
        transform.anchor = Anchor::TopLeft;
        transform.rotation = 90.0;
        let (_, bounds) = layer_geometry(LayerGeometryInput {
            transform: &transform,
            animation: &ResolvedClipAnimation::RASTER_IDENTITY,
            camera: None,
            mask: None,
            mapping: mapping(200.0, 100.0),
            output: [200, 100],
            geometry: None,
        })
        .unwrap();
        assert_eq!(bounds.content, DeviceRect::new(0, 0, 50, 100));
    }

    #[test]
    fn projective_bounds_are_device_space_and_scale_invariant() {
        let unit = Rect::new(0.0, 0.0, 1.0, 1.0);
        let perspective =
            DeviceTransform::from_projective([100.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.5, 0.0, 1.0])
                .unwrap();
        assert_eq!(
            project_local_rect(unit, unit, perspective, DeviceRect::full(200, 200)).unwrap(),
            DeviceRect::new(0, 0, 67, 100)
        );

        let tiny_identity = DeviceTransform::from_projective([
            1.0e-15, 0.0, 0.0, 0.0, 1.0e-15, 0.0, 0.0, 0.0, 1.0e-15,
        ])
        .unwrap();
        assert_eq!(
            project_local_rect(unit, unit, tiny_identity, DeviceRect::full(10, 10)).unwrap(),
            DeviceRect::new(0, 0, 1, 1)
        );
    }

    #[test]
    fn projective_horizon_and_coordinate_budget_fail_before_execution() {
        let unit = Rect::new(0.0, 0.0, 1.0, 1.0);
        let horizon =
            DeviceTransform::from_projective([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -2.0, 0.0, 1.0])
                .unwrap();
        assert_eq!(
            project_local_rect(unit, unit, horizon, DeviceRect::full(100, 100)),
            Err(BoundsError::PerspectiveHorizon)
        );

        let too_far =
            DeviceTransform::from_affine([1.0, 0.0, MAX_DEVICE_COORDINATE + 1.0, 0.0, 1.0, 0.0]);
        assert_eq!(
            project_local_rect(unit, unit, too_far, DeviceRect::full(100, 100)),
            Err(BoundsError::BudgetExceeded)
        );
    }

    #[test]
    fn device_transform_wire_rejects_horizons_and_over_budget_coordinates() {
        let transform = DeviceTransform::from_affine([100.0, 0.0, 0.0, 0.0, 100.0, 0.0]);
        let mut horizon = serde_json::to_value(transform).unwrap();
        horizon["matrix"][8] = serde_json::json!(0.0);
        assert!(serde_json::from_value::<DeviceTransform>(horizon).is_err());

        let mut too_far = serde_json::to_value(transform).unwrap();
        too_far["matrix"][2] = serde_json::json!(MAX_DEVICE_COORDINATE + 1.0);
        assert!(serde_json::from_value::<DeviceTransform>(too_far).is_err());
    }

    #[test]
    fn prepared_bounds_wire_rejects_noncanonical_empty_and_forged_budget() {
        let bounds = PreparedBounds {
            content: DeviceRect::new(10, 10, 20, 20),
            output: DeviceRect::new(8, 8, 24, 24),
            sample: DeviceRect::new(8, 8, 24, 24),
            reason: BoundsReason::Exact,
            max_intermediate_pixels: 24 * 24,
        };
        let wire = serde_json::to_value(bounds).unwrap();
        assert_eq!(
            serde_json::from_value::<PreparedBounds>(wire.clone()).unwrap(),
            bounds
        );

        let mut forged_budget = wire.clone();
        forged_budget["maxIntermediatePixels"] = serde_json::json!(1);
        assert!(serde_json::from_value::<PreparedBounds>(forged_budget).is_err());

        let mut noncanonical_empty = wire;
        noncanonical_empty["content"] =
            serde_json::json!({"x": 12, "y": 10, "width": 0, "height": 0});
        assert!(serde_json::from_value::<PreparedBounds>(noncanonical_empty).is_err());
    }

    #[test]
    fn preset_animation_is_applied_in_local_space_before_rotation() {
        let unit = Rect::new(0.0, 0.0, 1.0, 1.0);
        let quarter_turn = DeviceTransform::from_affine([0.0, -100.0, 100.0, 100.0, 0.0, 0.0]);
        let local_clip = clip_inset_rect(unit, [0.0, 0.5, 0.0, 0.0]).unwrap();
        assert_eq!(
            project_local_rect(local_clip, unit, quarter_turn, DeviceRect::full(100, 100),)
                .unwrap(),
            DeviceRect::new(0, 0, 100, 50)
        );
    }

    #[test]
    fn spatial_outset_is_applied_before_root_clamping() {
        let unit = Rect::new(0.0, 0.0, 1.0, 1.0);
        let outside = DeviceTransform::from_affine([10.0, 0.0, 101.0, 0.0, 10.0, 20.0]);
        let projected = project_local_rect_unclamped(unit, unit, outside).unwrap();
        assert!(
            projected_to_device(projected, DeviceRect::full(100, 100))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            projected_to_device(
                outset_projected(projected, [5.0; 4]).unwrap(),
                DeviceRect::full(100, 100),
            )
            .unwrap(),
            DeviceRect::new(96, 15, 4, 20)
        );
    }

    #[test]
    fn device_intermediates_have_a_hard_admission_budget() {
        assert_eq!(
            enforce_intermediate_budget(MAX_DEVICE_INTERMEDIATE_PIXELS + 1),
            Err(BoundsError::IntermediateBudgetExceeded {
                actual: MAX_DEVICE_INTERMEDIATE_PIXELS + 1,
                limit: MAX_DEVICE_INTERMEDIATE_PIXELS,
            })
        );
    }
}
