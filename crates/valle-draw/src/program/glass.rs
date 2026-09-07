//! Typed Motion Glass execution data shared by DrawProgram construction and both executors.
//!
//! `MotionGlassProgram` is not an independent packet. It is validated as part of the typed
//! DrawProgram arena and serialized only inside the outer DrawProgram packet.

#[cfg(test)]
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::program::{BackdropScope, NodeId};
use crate::requirements::{
    DestinationOperation, DestinationUse, DrawCapability, DrawRequirements, Insets, LocalBounds,
    SamplingMode,
};
use crate::{Rect, math};

pub const MOTION_GLASS_SCHEMA_ID: &str = "motion-glass-schema";
pub const MOTION_GLASS_KERNEL_ID: &str = "motion-glass-kernel";
pub const MOTION_GLASS_SCHEMA_MANIFEST: &str = "deny-unknown-fields+owner-kind-id+strict-owner-scope-identity+sorted-surfaces+shape-rect-local-to-owner-radius-presence+resolved-current-signed-response+simple-path+field-members-merge-1-128+physical-material-bevel-width-thickness-ior-roughness-dispersion-tint-fresnel-shadow+resolved-screen-light+conservative-backdrop-scope-bounds+surface-foreground-neighborhood-bounds-luma+foreground-exact-owner-geometry+foreground-owner-space-execution+kernel-schema-digests";
pub const MOTION_GLASS_KERNEL_MANIFEST: &str = "shared-sksl+materialization+distance-preserving-c2-potential+signed-current-response+spatial-owner-metric-field+critical-point-classification+continuous-density-shadow+presence-distance+analytic-projective-sdf+singularity-safe-projective-sampling+resolution-covariant-optical-lengths+independent-bevel-and-optical-depth+full-short-axis-height-field+motion-perturbed-surface-normals+snell-edge-calibrated-monotone-rgb-refraction+five-tap-roughness-diffusion+alpha-gated-tint+schlick-fresnel+dual-lobe-resolved-3d-light+subtle-contact-shadow+layout-bounds-inert-material+real-foreground-mask+rgba32f-linear-rec2020-premul";
pub const MOTION_GLASS_SCHEMA_DIGEST_HEX: &str =
    "d2dfd775199401be92df9f26e50719b8939ce3ecc4214d0df2f2d534bf0cfc07";
pub const MOTION_GLASS_SCHEMA_DIGEST: [u8; 32] = [
    0xd2, 0xdf, 0xd7, 0x75, 0x19, 0x94, 0x01, 0xbe, 0x92, 0xdf, 0x9f, 0x26, 0xe5, 0x07, 0x19, 0xb8,
    0x93, 0x9c, 0xe3, 0xec, 0xc4, 0x21, 0x4d, 0x0d, 0xf2, 0xf2, 0xd5, 0x34, 0xbf, 0x0c, 0xfc, 0x07,
];
pub const MAX_GLASS_SURFACES: usize = 8;
pub const MAX_GLASS_MEMBERS: usize = 8;
pub const MAX_GLASS_PATH_POINTS: usize = 24;
pub const MIN_GLASS_MERGE_DISTANCE: f32 = 1.0;
pub const MAX_GLASS_MERGE_DISTANCE: f32 = 128.0;
/// Byte-identical production kernel compiled by native Skia and Web CanvasKit.
pub const MOTION_GLASS_SKSL: &str = include_str!("../../assets/shaders/motionglass.sksl");
/// Independent Glass captures the destination at its painter position.
pub const BACKDROP_SCOPE_CURRENT: BackdropScope = BackdropScope::Current;

/// Resolved foreground tones (packed). Auto uses the foreground subtree's measured luminance.
pub const FOREGROUND_TONE_AUTO: u8 = 1;
pub const FOREGROUND_TONE_LIGHT: u8 = 2;
pub const FOREGROUND_TONE_DARK: u8 = 3;
pub const FOREGROUND_TONE_NONE: u8 = 4;

// ---- G1.6 shared kernel manifest (single source of truth for the kernel digest) ----
//
// These constants mirror `benchmarks/glass/reference.toml` and `response.toml`. They are the
// frozen semantic contract that Native and Web kernel bundles must satisfy: changing any value
// changes the kernel digest and therefore every capability/program that carries it.

pub const KERNEL_MATERIALIZATION: &str = "M(p)=p^3*(10-15p+6p^2)";
pub const KERNEL_POTENTIAL_ID: &str = "distance-c2-w";
pub const KERNEL_EPSILON: f64 = 1.0e-6;
pub const KERNEL_NARROW_BAND: f64 = 4.0;
pub const KERNEL_DERIVATIVE_STEP_SECONDS: f64 = 1.0 / 240.0;
pub const KERNEL_SETTLE_MIN_SECONDS: f64 = 0.08;
pub const KERNEL_SETTLE_MAX_SECONDS: f64 = 1.20;
pub const KERNEL_REFERENCE_PIXEL: &str = "rgba32f-linear-rec2020-premul";
/// `(character, a, b, elastic)` Beta-density coefficients per packed character.
pub const KERNEL_RESPONSE_MANIFEST: [(PackedGlassCharacter, i32, i32, bool); 4] = [
    (PackedGlassCharacter::Responsive, 3, 6, false),
    (PackedGlassCharacter::Fluid, 3, 4, false),
    (PackedGlassCharacter::Viscous, 5, 3, false),
    (PackedGlassCharacter::Elastic, 3, 3, true),
];

/// Frozen kernel digest hex; verified by `kernel_digest_is_frozen`.
pub const MOTION_GLASS_KERNEL_DIGEST_HEX: &str =
    "94188d0c70b1bdc386871012f4b1a316dfe52672942cebacec2701f4e391122a";
pub const MOTION_GLASS_KERNEL_DIGEST: [u8; 32] = [
    0x94, 0x18, 0x8d, 0x0c, 0x70, 0xb1, 0xbd, 0xc3, 0x86, 0x87, 0x10, 0x12, 0xf4, 0xb1, 0xa3, 0x16,
    0xdf, 0xe5, 0x26, 0x72, 0x94, 0x2c, 0xeb, 0xac, 0xec, 0x27, 0x01, 0xf4, 0xe3, 0x91, 0x12, 0x2a,
];

fn valid_glass_transform(matrix: [f32; 9]) -> bool {
    if !matrix.into_iter().all(f32::is_finite) {
        return false;
    }
    let [a, b, c, d, e, f, g, h, i] = matrix.map(f64::from);
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    determinant.is_finite() && determinant.abs() > KERNEL_EPSILON
}

fn transform_has_no_horizon(matrix: [f32; 9], rect: Rect) -> bool {
    let denominators = [
        [rect.left(), rect.top()],
        [rect.right(), rect.top()],
        [rect.right(), rect.bottom()],
        [rect.left(), rect.bottom()],
    ]
    .map(|point| {
        f64::from(matrix[6]) * point[0] + f64::from(matrix[7]) * point[1] + f64::from(matrix[8])
    });
    let Some(first) = denominators.first().copied() else {
        return false;
    };
    first.is_finite()
        && first.abs() > KERNEL_EPSILON
        && denominators.into_iter().all(|value| {
            value.is_finite()
                && value.abs() > KERNEL_EPSILON
                && value.is_sign_positive() == first.is_sign_positive()
        })
}

fn valid_optional_rect(rect: Option<Rect>) -> bool {
    rect.is_none_or(|rect| {
        !rect.is_empty()
            && [rect.left(), rect.top(), rect.right(), rect.bottom()]
                .into_iter()
                .all(f64::is_finite)
    })
}

fn valid_optional_unit(value: Option<f32>) -> bool {
    value.is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value))
}

fn valid_shape_geometry(
    shape: PackedGlassShapeKind,
    rect: Rect,
    radius: f32,
    path_points: &[[f32; 2]],
) -> bool {
    if !radius.is_finite()
        || ![rect.left(), rect.top(), rect.right(), rect.bottom()]
            .into_iter()
            .all(f64::is_finite)
        || rect.is_empty()
    {
        return false;
    }
    let maximum_radius = 0.5 * rect.width.min(rect.height);
    match shape {
        PackedGlassShapeKind::Circle | PackedGlassShapeKind::Capsule => {
            radius > 0.0
                && f64::from(radius) <= maximum_radius + KERNEL_EPSILON
                && path_points.is_empty()
        }
        PackedGlassShapeKind::ContinuousRect => {
            radius >= 0.0
                && f64::from(radius) <= maximum_radius + KERNEL_EPSILON
                && path_points.is_empty()
        }
        PackedGlassShapeKind::Path => {
            radius == 0.0
                && path_points.len() >= 3
                && path_points.len() <= MAX_GLASS_PATH_POINTS
                && simple_polygon_inside_rect(path_points, rect)
        }
    }
}

fn simple_polygon_inside_rect(points: &[[f32; 2]], rect: Rect) -> bool {
    let epsilon = KERNEL_EPSILON * rect.width.max(rect.height).max(1.0);
    if !points.iter().all(|point| {
        let [x, y] = point.map(f64::from);
        x.is_finite()
            && y.is_finite()
            && x >= rect.left() - epsilon
            && x <= rect.right() + epsilon
            && y >= rect.top() - epsilon
            && y <= rect.bottom() + epsilon
    }) {
        return false;
    }
    let mut twice_area = 0.0;
    for index in 0..points.len() {
        let a = points[index].map(f64::from);
        let b = points[(index + 1) % points.len()].map(f64::from);
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        if math::sqrt(dx * dx + dy * dy) <= epsilon {
            return false;
        }
        twice_area += a[0] * b[1] - b[0] * a[1];
    }
    if twice_area.abs() <= epsilon * epsilon {
        return false;
    }
    for left in 0..points.len() {
        let left_next = (left + 1) % points.len();
        for right in left + 1..points.len() {
            let right_next = (right + 1) % points.len();
            if left == right || left_next == right || right_next == left {
                continue;
            }
            if segments_intersect(
                points[left].map(f64::from),
                points[left_next].map(f64::from),
                points[right].map(f64::from),
                points[right_next].map(f64::from),
                epsilon,
            ) {
                return false;
            }
        }
    }
    true
}

fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], epsilon: f64) -> bool {
    let cross = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let on_segment = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        q[0] >= p[0].min(r[0]) - epsilon
            && q[0] <= p[0].max(r[0]) + epsilon
            && q[1] >= p[1].min(r[1]) - epsilon
            && q[1] <= p[1].max(r[1]) + epsilon
    };
    let values = [
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    ];
    if values[0] * values[1] < -epsilon && values[2] * values[3] < -epsilon {
        return true;
    }
    (values[0].abs() <= epsilon && on_segment(a, c, b))
        || (values[1].abs() <= epsilon && on_segment(a, d, b))
        || (values[2].abs() <= epsilon && on_segment(c, a, d))
        || (values[3].abs() <= epsilon && on_segment(c, b, d))
}

fn transformed_rect_bounds(matrix: [f32; 9], rect: Rect) -> Option<Rect> {
    if !transform_has_no_horizon(matrix, rect) {
        return None;
    }
    let matrix = matrix.map(f64::from);
    let project = |x: f64, y: f64| {
        let denominator = matrix[6] * x + matrix[7] * y + matrix[8];
        let point = [
            (matrix[0] * x + matrix[1] * y + matrix[2]) / denominator,
            (matrix[3] * x + matrix[4] * y + matrix[5]) / denominator,
        ];
        point.into_iter().all(f64::is_finite).then_some(point)
    };
    let points = [
        project(rect.left(), rect.top())?,
        project(rect.right(), rect.top())?,
        project(rect.right(), rect.bottom())?,
        project(rect.left(), rect.bottom())?,
    ];
    Some(Rect::from_edges(
        points
            .iter()
            .map(|point| point[0])
            .fold(f64::INFINITY, f64::min),
        points
            .iter()
            .map(|point| point[1])
            .fold(f64::INFINITY, f64::min),
        points
            .iter()
            .map(|point| point[0])
            .fold(f64::NEG_INFINITY, f64::max),
        points
            .iter()
            .map(|point| point[1])
            .fold(f64::NEG_INFINITY, f64::max),
    ))
}

fn rect_contains(outer: Rect, inner: Rect) -> bool {
    outer.left() <= inner.left() + KERNEL_EPSILON
        && outer.top() <= inner.top() + KERNEL_EPSILON
        && outer.right() + KERNEL_EPSILON >= inner.right()
        && outer.bottom() + KERNEL_EPSILON >= inner.bottom()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlassOwnerKind {
    Independent = 1,
    Field = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PackedGlassShapeKind {
    Capsule = 1,
    Circle = 2,
    ContinuousRect = 3,
    Path = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackedGlassCharacter {
    Responsive = 1,
    Fluid = 2,
    Viscous = 3,
    Elastic = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PackedGlassForegroundTone {
    Auto,
    Light,
    Dark,
    None,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "camelCase"))]
pub struct PackedGlassSurface {
    pub surface_id: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub shape: PackedGlassShapeKind,
    pub rect: Rect,
    /// Homography from `rect`/`path_points` coordinates into the material owner's local space.
    /// Kinematic response remains device-qualified; geometry is not baked to device pixels, so
    /// ordinary DrawProgram ancestor transforms apply exactly once.
    pub local_to_owner: [f32; 9],
    pub radius: f32,
    pub presence: f32,
    /// Current signed causal response, resolved during prepare. Field programs keep one response
    /// per member so the kernel can mix motion spatially instead of collapsing the complete field
    /// to a global max. Temporal integration samples never enter the executor ABI.
    pub response: PackedGlassMotion,
    /// G4.1 closed-polygon path points (device pixels); empty for analytic shapes. `Path`
    /// requires at least three points; analytic shapes must leave it empty.
    pub path_points: Vec<[f32; 2]>,
    /// Foreground protection metadata only. Pixels remain ordinary DrawProgram children; this
    /// owner-local neighborhood continuously modulates optics without becoming a fake primitive.
    pub foreground_tone: PackedGlassForegroundTone,
    pub foreground_protection: f32,
    pub foreground_bounds: Option<Rect>,
    /// Coverage-weighted child luminance for `Auto`; explicit tone hints leave this absent.
    pub foreground_luma: Option<f32>,
}

/// Compact geometry program for one real foreground subtree. Protection/tone stay authoritative
/// on the material surface; duplicating them here would inflate every marker and create a second
/// value that can drift. The subtree remains ordinary DrawProgram content and this program only
/// clips it after the shared material pass.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionGlassForegroundProgram {
    pub owner_id: String,
    pub surface_id: String,
    pub shape: PackedGlassShapeKind,
    pub rect: Rect,
    pub local_to_owner: [f32; 9],
    pub radius: f32,
    pub presence: f32,
    pub path_points: Vec<[f32; 2]>,
    pub kernel_digest: [u8; 32],
    pub schema_digest: [u8; 32],
}

impl MotionGlassForegroundProgram {
    pub fn from_owner(
        owner: &MotionGlassProgram,
        surface_id: &str,
    ) -> Result<Self, MotionGlassError> {
        owner.validate()?;
        let surface = owner
            .surfaces
            .iter()
            .find(|surface| surface.surface_id == surface_id)
            .ok_or(MotionGlassError::Invalid(
                "foreground surface is not owned by material program",
            ))?;
        let program = Self {
            owner_id: owner.owner_id.clone(),
            surface_id: surface.surface_id.clone(),
            shape: surface.shape,
            rect: surface.rect,
            local_to_owner: surface.local_to_owner,
            radius: surface.radius,
            presence: surface.presence,
            path_points: surface.path_points.clone(),
            kernel_digest: owner.kernel_digest,
            schema_digest: owner.schema_digest,
        };
        program.validate()?;
        Ok(program)
    }

    pub fn validate(&self) -> Result<(), MotionGlassError> {
        if self.owner_id.is_empty()
            || self.owner_id.len() > 4_096
            || self.surface_id.is_empty()
            || self.surface_id.len() > 4_096
            || !self.presence.is_finite()
            || !(0.0..=1.0).contains(&self.presence)
            || !valid_shape_geometry(self.shape, self.rect, self.radius, &self.path_points)
            || !valid_glass_transform(self.local_to_owner)
            || !transform_has_no_horizon(self.local_to_owner, self.rect)
        {
            return Err(MotionGlassError::Invalid("foreground fields"));
        }
        if self.kernel_digest != motion_glass_kernel_digest()
            || self.schema_digest != schema_digest()
        {
            return Err(MotionGlassError::Invalid("foreground digest mismatch"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "camelCase"))]
pub struct PackedGlassField {
    pub field_id: String,
    pub merge_distance: f32,
    pub member_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "camelCase"))]
pub struct PackedGlassMaterial {
    /// Device-pixel width of the rounded height-field boundary rolloff. This is deliberately separate
    /// from optical thickness: a thin UI bevel can still carry strong refraction.
    pub bevel_width: f32,
    /// Device-pixel optical path depth through the material.
    pub thickness: f32,
    /// Refractive index of the green channel. Red/blue use `± dispersion`.
    pub refractive_index: f32,
    /// Surface micro-roughness. The kernel derives both diffusion and highlight width from it.
    pub roughness: f32,
    pub dispersion: f32,
    /// Straight linear Rec.2020 tint. Alpha is the sole tint-energy authority.
    pub tint_linear: [f32; 4],
    pub specular_strength: f32,
    pub shadow_strength: f32,
    pub foreground_gain: f32,
    pub light: PackedGlassLight,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackedGlassLight {
    /// Unit screen-space direction toward the authored light.
    pub direction: [f32; 2],
    pub elevation: f32,
    pub intensity: f32,
}

impl PackedGlassLight {
    pub const DEFAULT: Self = Self {
        direction: [-0.349_231, -0.937_037],
        elevation: 0.55,
        intensity: 0.65,
    };
}

/// Signed current response channels. Keeping the channels orthogonal is essential:
/// opposite translations must produce opposite drag, acceleration must remain distinguishable
/// from velocity, and an elastic kernel must be allowed to cross zero.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackedGlassMotion {
    pub translation: [f32; 2],
    pub acceleration: [f32; 2],
    pub angular: f32,
    pub scale: [f32; 2],
    pub shear: f32,
    pub area: f32,
    pub pressure: f32,
    pub twist: f32,
}

impl PackedGlassMotion {
    pub const ZERO: Self = Self {
        translation: [0.0; 2],
        acceleration: [0.0; 2],
        angular: 0.0,
        scale: [0.0; 2],
        shear: 0.0,
        area: 0.0,
        pressure: 0.0,
        twist: 0.0,
    };

    pub fn is_finite(self) -> bool {
        [
            self.translation[0],
            self.translation[1],
            self.acceleration[0],
            self.acceleration[1],
            self.angular,
            self.scale[0],
            self.scale[1],
            self.shear,
            self.area,
            self.pressure,
            self.twist,
        ]
        .into_iter()
        .all(f32::is_finite)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "camelCase"))]
pub struct BackdropUse {
    pub scope: BackdropScope,
    pub sample_bounds: Rect,
    pub output_bounds: Rect,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "camelCase"))]
pub struct MotionGlassProgram {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub owner_kind: GlassOwnerKind,
    pub owner_id: String,
    pub surfaces: Vec<PackedGlassSurface>,
    pub field: Option<PackedGlassField>,
    pub material: PackedGlassMaterial,
    pub backdrop: BackdropUse,
    pub kernel_id: String,
    /// Exact digest of the admitted semantic kernel manifest. `kernel_id` alone is a label and
    /// cannot prove that two independently built executors implement the same math.
    pub kernel_digest: [u8; 32],
    pub schema_digest: [u8; 32],
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MotionGlassError {
    #[error("{0}")]
    Invalid(&'static str),
}

impl MotionGlassProgram {
    pub fn validate(&self) -> Result<(), MotionGlassError> {
        if self.owner_id.is_empty() || self.owner_id.len() > 4_096 {
            return Err(MotionGlassError::Invalid("owner id"));
        }
        if self.surfaces.is_empty() || self.surfaces.len() > MAX_GLASS_SURFACES {
            return Err(MotionGlassError::Invalid("surface count"));
        }
        for pair in self.surfaces.windows(2) {
            match pair[0].surface_id.cmp(&pair[1].surface_id) {
                core::cmp::Ordering::Greater => {
                    return Err(MotionGlassError::Invalid("surfaces must be sorted by id"));
                }
                core::cmp::Ordering::Equal => {
                    return Err(MotionGlassError::Invalid("surface ids must be unique"));
                }
                core::cmp::Ordering::Less => {}
            }
        }
        for surface in &self.surfaces {
            if surface.surface_id.is_empty()
                || surface.surface_id.len() > 4_096
                || !surface.presence.is_finite()
                || !(0.0..=1.0).contains(&surface.presence)
                || !valid_shape_geometry(
                    surface.shape,
                    surface.rect,
                    surface.radius,
                    &surface.path_points,
                )
                || !valid_glass_transform(surface.local_to_owner)
                || !transform_has_no_horizon(surface.local_to_owner, surface.rect)
                || !surface.foreground_protection.is_finite()
                || !(0.0..=1.0).contains(&surface.foreground_protection)
                || !valid_optional_rect(surface.foreground_bounds)
                || !valid_optional_unit(surface.foreground_luma)
            {
                return Err(MotionGlassError::Invalid("surface fields must be finite"));
            }
            if !surface.response.is_finite() {
                return Err(MotionGlassError::Invalid("surface response"));
            }
            if (surface.foreground_tone == PackedGlassForegroundTone::None
                && surface.foreground_protection != 0.0)
                || (surface.foreground_protection == 0.0
                    && (surface.foreground_bounds.is_some() || surface.foreground_luma.is_some()))
                || (surface.foreground_protection > 0.0
                    && (surface.foreground_bounds.is_none()
                        || (surface.foreground_tone == PackedGlassForegroundTone::Auto
                            && surface.foreground_luma.is_none())))
                || (surface.foreground_tone != PackedGlassForegroundTone::Auto
                    && surface.foreground_luma.is_some())
            {
                return Err(MotionGlassError::Invalid("foreground metadata"));
            }
        }
        if let Some(field) = &self.field {
            if self.owner_kind != GlassOwnerKind::Field {
                return Err(MotionGlassError::Invalid(
                    "field payload requires field owner",
                ));
            }
            if field.field_id != self.owner_id
                || field.field_id.is_empty()
                || field.member_ids.is_empty()
                || field.member_ids.len() > MAX_GLASS_MEMBERS
                || !field.merge_distance.is_finite()
                || !(MIN_GLASS_MERGE_DISTANCE..=MAX_GLASS_MERGE_DISTANCE)
                    .contains(&field.merge_distance)
            {
                return Err(MotionGlassError::Invalid("invalid field payload"));
            }
            if field.member_ids.len() != self.surfaces.len()
                || field
                    .member_ids
                    .iter()
                    .zip(&self.surfaces)
                    .any(|(member, surface)| member != &surface.surface_id)
            {
                return Err(MotionGlassError::Invalid(
                    "field members must exactly equal sorted packed surfaces",
                ));
            }
        } else if self.owner_kind == GlassOwnerKind::Field {
            return Err(MotionGlassError::Invalid(
                "field owner requires field payload",
            ));
        }
        if self.owner_kind == GlassOwnerKind::Independent
            && (self.surfaces.len() != 1 || self.field.is_some())
        {
            return Err(MotionGlassError::Invalid(
                "independent owner requires exactly one surface and no field",
            ));
        }
        if self.owner_kind == GlassOwnerKind::Independent
            && self.surfaces[0].surface_id != self.owner_id
        {
            return Err(MotionGlassError::Invalid(
                "independent owner id must equal its surface id",
            ));
        }
        let valid_scope = match (&self.owner_kind, &self.backdrop.scope) {
            (GlassOwnerKind::Independent, BackdropScope::Current) => true,
            (GlassOwnerKind::Field, BackdropScope::ScopeEntry(owner)) => owner == &self.owner_id,
            _ => false,
        };
        if !valid_scope {
            return Err(MotionGlassError::Invalid("invalid backdrop scope"));
        }
        let material = &self.material;
        let material_fields = [
            material.bevel_width,
            material.thickness,
            material.refractive_index,
            material.roughness,
            material.dispersion,
            material.specular_strength,
            material.shadow_strength,
            material.foreground_gain,
            material.light.direction[0],
            material.light.direction[1],
            material.light.elevation,
            material.light.intensity,
        ];
        let light_x = f64::from(material.light.direction[0]);
        let light_y = f64::from(material.light.direction[1]);
        let light_length = math::sqrt(light_x * light_x + light_y * light_y);
        if !material_fields.into_iter().all(f32::is_finite)
            || !material
                .tint_linear
                .into_iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            || material.thickness < 0.0
            || material.bevel_width < 0.0
            || !(1.0..=1.6).contains(&material.refractive_index)
            || !(0.0..=1.0).contains(&material.roughness)
            || !(0.0..=0.08).contains(&material.dispersion)
            || material.refractive_index - material.dispersion < 1.0
            || material.refractive_index + material.dispersion > 1.6
            || !(0.0..=1.0).contains(&material.specular_strength)
            || !(0.0..=0.5).contains(&material.shadow_strength)
            || !(0.0..=1.0).contains(&material.foreground_gain)
            || !(0.999..=1.001).contains(&light_length)
            || !(0.0..=1.0).contains(&material.light.elevation)
            || !(0.0..=1.0).contains(&material.light.intensity)
        {
            return Err(MotionGlassError::Invalid("material fields"));
        }
        let sample = self.backdrop.sample_bounds;
        let output = self.backdrop.output_bounds;
        if ![
            sample.left(),
            sample.top(),
            sample.right(),
            sample.bottom(),
            output.left(),
            output.top(),
            output.right(),
            output.bottom(),
        ]
        .into_iter()
        .all(f64::is_finite)
            || sample.is_empty()
            || output.is_empty()
            || sample.left() > output.left()
            || sample.top() > output.top()
            || sample.right() < output.right()
            || sample.bottom() < output.bottom()
        {
            return Err(MotionGlassError::Invalid("backdrop bounds"));
        }
        if self.surfaces.iter().any(|surface| {
            transformed_rect_bounds(surface.local_to_owner, surface.rect)
                .is_none_or(|bounds| !rect_contains(output, bounds))
        }) {
            return Err(MotionGlassError::Invalid(
                "output bounds do not contain surface geometry",
            ));
        }
        if self.kernel_id != MOTION_GLASS_KERNEL_ID {
            return Err(MotionGlassError::Invalid("unknown kernel id"));
        }
        if self.kernel_digest != motion_glass_kernel_digest() {
            return Err(MotionGlassError::Invalid("kernel digest mismatch"));
        }
        if self.schema_digest != schema_digest() {
            return Err(MotionGlassError::Invalid("schema digest mismatch"));
        }
        Ok(())
    }
}

pub const fn schema_digest() -> [u8; 32] {
    MOTION_GLASS_SCHEMA_DIGEST
}

#[cfg(test)]
fn computed_schema_digest() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"motion-glass-schema-manifest");
    hasher.update(MOTION_GLASS_SCHEMA_ID.as_bytes());
    hasher.update((MAX_GLASS_SURFACES as u64).to_le_bytes());
    hasher.update((MAX_GLASS_MEMBERS as u64).to_le_bytes());
    hasher.update((MAX_GLASS_PATH_POINTS as u64).to_le_bytes());
    hasher.update(MOTION_GLASS_SCHEMA_MANIFEST.as_bytes());
    hasher.finalize().into()
}

/// Frozen digest of the admitted semantic manifest and shared SkSL. Runtime validation compares
/// these bytes directly; the test-only recomputation below hashes every semantic input and catches
/// source drift without putting SHA-256 on the per-frame execution path.
pub const fn motion_glass_kernel_digest() -> [u8; 32] {
    MOTION_GLASS_KERNEL_DIGEST
}

#[cfg(test)]
fn computed_motion_glass_kernel_digest() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"motion-glass-kernel-manifest");
    hasher.update(MOTION_GLASS_KERNEL_ID.as_bytes());
    hasher.update(KERNEL_MATERIALIZATION.as_bytes());
    hasher.update(KERNEL_POTENTIAL_ID.as_bytes());
    for (character, a, b, elastic) in KERNEL_RESPONSE_MANIFEST {
        hasher.update([character as u8, a as u8, b as u8, u8::from(elastic)]);
    }
    hasher.update(KERNEL_EPSILON.to_le_bytes());
    hasher.update(KERNEL_NARROW_BAND.to_le_bytes());
    hasher.update(KERNEL_DERIVATIVE_STEP_SECONDS.to_le_bytes());
    hasher.update(KERNEL_SETTLE_MIN_SECONDS.to_le_bytes());
    hasher.update(KERNEL_SETTLE_MAX_SECONDS.to_le_bytes());
    hasher.update(KERNEL_REFERENCE_PIXEL.as_bytes());
    hasher.update(MOTION_GLASS_KERNEL_MANIFEST.as_bytes());
    hasher.update(MOTION_GLASS_SKSL.as_bytes());
    hasher.finalize().into()
}

/// Verifies that a program carries the admitted shared kernel id and digest.
pub fn verify_kernel(program: &MotionGlassProgram) -> Result<(), MotionGlassError> {
    if program.kernel_id != MOTION_GLASS_KERNEL_ID {
        return Err(MotionGlassError::Invalid("unknown kernel id"));
    }
    if program.schema_digest != schema_digest() {
        return Err(MotionGlassError::Invalid("schema digest mismatch"));
    }
    if program.kernel_digest != motion_glass_kernel_digest() {
        return Err(MotionGlassError::Invalid("kernel digest mismatch"));
    }
    Ok(())
}

/// G1.4: derived execution contract failures. Fail-closed before any graph build.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GlassRequirementsError {
    #[error("independent Glass program must own exactly one surface")]
    SurfaceCount,
    #[error("independent Glass program may not declare a field payload")]
    FieldPayload,
    #[error("Glass owner has an invalid backdrop scope")]
    InvalidScope,
    #[error("Glass program is invalid: {0}")]
    InvalidProgram(&'static str),
    #[error("Glass program bounds are not finite")]
    NonFiniteBounds,
    #[error("Glass program bounds are empty")]
    EmptyBounds,
}

impl MotionGlassProgram {
    /// Derives the `DrawRequirements` contract for an independent Glass program.
    ///
    /// - independent surfaces may only declare `Current`;
    /// - the current sample is the working-color ROI (`sample_bounds`);
    /// - no foreground/environment temporal branch exists yet, so no extra resources;
    /// - bounds that are not finite or are empty fail before graph build.
    pub fn derive_requirements(&self) -> Result<DrawRequirements, GlassRequirementsError> {
        match self.owner_kind {
            GlassOwnerKind::Independent if self.surfaces.len() != 1 || self.field.is_some() => {
                return Err(GlassRequirementsError::SurfaceCount);
            }
            GlassOwnerKind::Field if self.field.is_none() => {
                return Err(GlassRequirementsError::FieldPayload);
            }
            _ => {}
        }
        let valid_scope = match (&self.owner_kind, &self.backdrop.scope) {
            (GlassOwnerKind::Independent, BackdropScope::Current) => true,
            (GlassOwnerKind::Field, BackdropScope::ScopeEntry(owner)) => owner == &self.owner_id,
            _ => false,
        };
        if !valid_scope {
            return Err(GlassRequirementsError::InvalidScope);
        }
        let output = self.backdrop.output_bounds;
        let sample = self.backdrop.sample_bounds;
        if ![
            output.left(),
            output.top(),
            output.right(),
            output.bottom(),
            sample.left(),
            sample.top(),
            sample.right(),
            sample.bottom(),
        ]
        .into_iter()
        .all(|value| value.is_finite())
        {
            return Err(GlassRequirementsError::NonFiniteBounds);
        }
        if output.is_empty() || sample.is_empty() {
            return Err(GlassRequirementsError::EmptyBounds);
        }
        if let Err(MotionGlassError::Invalid(message)) = self.validate() {
            return Err(GlassRequirementsError::InvalidProgram(message));
        }
        let margin = Insets::new(
            (output.left() - sample.left()).max(0.0) as f32,
            (output.top() - sample.top()).max(0.0) as f32,
            (sample.right() - output.right()).max(0.0) as f32,
            (sample.bottom() - output.bottom()).max(0.0) as f32,
        );
        Ok(DrawRequirements {
            content_bounds: LocalBounds::from_rect(output),
            output_bounds: LocalBounds::from_rect(output),
            external_textures: Vec::new(),
            fonts: Vec::new(),
            runtime_shaders: Vec::new(),
            scene3d: Vec::new(),
            destination_uses: vec![DestinationUse {
                node: NodeId::from_raw(0),
                scope: self.backdrop.scope.clone(),
                output_bounds: output,
                sample_bounds: sample,
                operation: DestinationOperation::Backdrop {
                    footprint: margin,
                    sampling: SamplingMode::LinearClamp,
                },
            }],
            filter_footprint: margin,
            color_domains: vec![crate::requirements::ColorDomain::LinearRec2020],
            alpha_modes: vec![crate::requirements::AlphaMode::Premultiplied],
            capabilities: { vec![DrawCapability::BackdropRead, DrawCapability::MotionGlass] },
            max_intermediate_pixels: ((sample.right() - sample.left()).max(0.0)
                * (sample.bottom() - sample.top()).max(0.0))
            .ceil() as u64,
        })
    }
}

impl serde::Serialize for MotionGlassProgram {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("MotionGlassProgram", 9)?;
        state.serialize_field("ownerKind", &(self.owner_kind as u8))?;
        state.serialize_field("ownerId", &self.owner_id)?;
        state.serialize_field("surfaces", &self.surfaces)?;
        state.serialize_field("field", &self.field)?;
        state.serialize_field("material", &self.material)?;
        state.serialize_field("backdrop", &self.backdrop)?;
        state.serialize_field("kernelId", &self.kernel_id)?;
        state.serialize_field("kernelDigest", &self.kernel_digest)?;
        state.serialize_field("schemaDigest", &self.schema_digest)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for MotionGlassProgram {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            owner_kind: u8,
            owner_id: String,
            surfaces: Vec<PackedGlassSurface>,
            field: Option<PackedGlassField>,
            material: PackedGlassMaterial,
            backdrop: BackdropUse,
            kernel_id: String,
            kernel_digest: [u8; 32],
            schema_digest: [u8; 32],
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            owner_kind: match wire.owner_kind {
                1 => GlassOwnerKind::Independent,
                2 => GlassOwnerKind::Field,
                _ => return Err(serde::de::Error::custom("ownerKind")),
            },
            owner_id: wire.owner_id,
            surfaces: wire.surfaces,
            field: wire.field,
            material: wire.material,
            backdrop: wire.backdrop,
            kernel_id: wire.kernel_id,
            kernel_digest: wire.kernel_digest,
            schema_digest: wire.schema_digest,
        })
    }
}

impl serde::Serialize for PackedGlassSurface {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("PackedGlassSurface", 12)?;
        state.serialize_field("surfaceId", &self.surface_id)?;
        state.serialize_field("shape", &(self.shape as u8))?;
        state.serialize_field("rect", &self.rect)?;
        state.serialize_field("localToOwner", &self.local_to_owner)?;
        state.serialize_field("radius", &self.radius)?;
        state.serialize_field("presence", &self.presence)?;
        state.serialize_field("response", &self.response)?;
        state.serialize_field("pathPoints", &self.path_points)?;
        state.serialize_field("foregroundTone", &self.foreground_tone)?;
        state.serialize_field("foregroundProtection", &self.foreground_protection)?;
        state.serialize_field("foregroundBounds", &self.foreground_bounds)?;
        state.serialize_field("foregroundLuma", &self.foreground_luma)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for PackedGlassSurface {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            surface_id: String,
            shape: u8,
            rect: Rect,
            local_to_owner: [f32; 9],
            radius: f32,
            presence: f32,
            response: PackedGlassMotion,
            path_points: Vec<[f32; 2]>,
            foreground_tone: PackedGlassForegroundTone,
            foreground_protection: f32,
            foreground_bounds: Option<Rect>,
            foreground_luma: Option<f32>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            surface_id: wire.surface_id,
            shape: match wire.shape {
                1 => PackedGlassShapeKind::Capsule,
                2 => PackedGlassShapeKind::Circle,
                3 => PackedGlassShapeKind::ContinuousRect,
                4 => PackedGlassShapeKind::Path,
                _ => return Err(serde::de::Error::custom("shape")),
            },
            rect: wire.rect,
            local_to_owner: wire.local_to_owner,
            radius: wire.radius,
            presence: wire.presence,
            response: wire.response,
            path_points: wire.path_points,
            foreground_tone: wire.foreground_tone,
            foreground_protection: wire.foreground_protection,
            foreground_bounds: wire.foreground_bounds,
            foreground_luma: wire.foreground_luma,
        })
    }
}

impl serde::Serialize for PackedGlassField {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("PackedGlassField", 3)?;
        state.serialize_field("fieldId", &self.field_id)?;
        state.serialize_field("mergeDistance", &self.merge_distance)?;
        state.serialize_field("memberIds", &self.member_ids)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for PackedGlassField {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            field_id: String,
            merge_distance: f32,
            member_ids: Vec<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            field_id: wire.field_id,
            merge_distance: wire.merge_distance,
            member_ids: wire.member_ids,
        })
    }
}

impl serde::Serialize for PackedGlassMaterial {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde_json::json!({
            "bevelWidth": self.bevel_width,
            "thickness": self.thickness,
            "refractiveIndex": self.refractive_index,
            "roughness": self.roughness,
            "dispersion": self.dispersion,
            "tintLinear": self.tint_linear,
            "specularStrength": self.specular_strength,
            "shadowStrength": self.shadow_strength,
            "foregroundGain": self.foreground_gain,
            "light": self.light,
        })
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for PackedGlassMaterial {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            bevel_width: f32,
            thickness: f32,
            refractive_index: f32,
            roughness: f32,
            dispersion: f32,
            tint_linear: [f32; 4],
            specular_strength: f32,
            shadow_strength: f32,
            foreground_gain: f32,
            light: PackedGlassLight,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            bevel_width: wire.bevel_width,
            thickness: wire.thickness,
            refractive_index: wire.refractive_index,
            roughness: wire.roughness,
            dispersion: wire.dispersion,
            tint_linear: wire.tint_linear,
            specular_strength: wire.specular_strength,
            shadow_strength: wire.shadow_strength,
            foreground_gain: wire.foreground_gain,
            light: wire.light,
        })
    }
}

impl serde::Serialize for BackdropUse {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde_json::json!({
            "scope": self.scope,
            "sampleBounds": self.sample_bounds,
            "outputBounds": self.output_bounds,
        })
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for BackdropUse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            scope: BackdropScope,
            sample_bounds: Rect,
            output_bounds: Rect,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            scope: wire.scope,
            sample_bounds: wire.sample_bounds,
            output_bounds: wire.output_bounds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Rect,
        program::{
            DrawProgramBuilder, DrawProgramError, Group, Node, compile_recording,
            recording::{ProgramRecording, RecordCmd, RecordingError},
        },
    };

    fn sample_program() -> MotionGlassProgram {
        MotionGlassProgram {
            owner_kind: GlassOwnerKind::Independent,
            owner_id: "hero-lens".into(),
            surfaces: vec![PackedGlassSurface {
                surface_id: "hero-lens".into(),
                shape: PackedGlassShapeKind::Circle,
                rect: Rect::new(10.0, 20.0, 80.0, 80.0),
                local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                radius: 40.0,
                presence: 1.0,
                response: PackedGlassMotion::ZERO,
                path_points: vec![],
                foreground_tone: PackedGlassForegroundTone::Auto,
                foreground_protection: 0.65,
                foreground_bounds: Some(Rect::new(25.0, 35.0, 40.0, 20.0)),
                foreground_luma: Some(0.85),
            }],
            field: None,
            material: PackedGlassMaterial {
                bevel_width: 12.0,
                thickness: 20.0,
                refractive_index: 1.2,
                roughness: 0.08,
                dispersion: 0.012,
                tint_linear: [0.0; 4],
                specular_strength: 0.5,
                shadow_strength: 0.1,
                foreground_gain: 0.7,
                light: PackedGlassLight::DEFAULT,
            },
            backdrop: BackdropUse {
                scope: BackdropScope::Current,
                sample_bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                output_bounds: Rect::new(10.0, 20.0, 80.0, 80.0),
            },
            kernel_id: MOTION_GLASS_KERNEL_ID.into(),
            kernel_digest: motion_glass_kernel_digest(),
            schema_digest: schema_digest(),
        }
    }

    #[test]
    fn schema_digest_is_frozen() {
        assert_eq!(computed_schema_digest(), schema_digest());
        let hex: String = schema_digest()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hex, MOTION_GLASS_SCHEMA_DIGEST_HEX);
    }

    #[test]
    fn kernel_digest_is_frozen() {
        assert_eq!(
            computed_motion_glass_kernel_digest(),
            motion_glass_kernel_digest()
        );
        let hex: String = motion_glass_kernel_digest()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hex, MOTION_GLASS_KERNEL_DIGEST_HEX);
    }

    #[test]
    fn verify_kernel_accepts_admitted_program_and_rejects_foreign_kernel() {
        let program = sample_program();
        assert!(verify_kernel(&program).is_ok());
        let mut foreign = program.clone();
        foreign.kernel_id = "third-party-kernel".into();
        assert!(verify_kernel(&foreign).is_err());
    }

    #[test]
    fn recording_validates_and_compiles_the_typed_program_without_an_inner_packet() {
        let program = sample_program();
        let mut recording = ProgramRecording::new();
        recording.cmds = vec![
            RecordCmd::BeginMotionGlass {
                program: Box::new(program.clone()),
            },
            RecordCmd::End,
        ];

        recording.validate().unwrap();
        let draw = compile_recording(Rect::new(0.0, 0.0, 100.0, 100.0), &recording).unwrap();
        let Node::Group(group) = &draw.nodes()[draw.roots()[0].index()] else {
            panic!("Glass recording must compile to a group")
        };
        assert_eq!(group.glass.as_deref(), Some(&program));

        let RecordCmd::BeginMotionGlass { program } = &mut recording.cmds[0] else {
            unreachable!()
        };
        program.kernel_id = "foreign".into();
        assert_eq!(
            recording.validate(),
            Err(RecordingError::InvalidMotionGlass { at: 0 })
        );
    }

    #[test]
    fn foreground_validation_bounds_identity_lengths() {
        let owner = sample_program();
        let mut foreground = MotionGlassForegroundProgram::from_owner(&owner, "hero-lens").unwrap();
        foreground.owner_id = "o".repeat(4_097);
        assert!(foreground.validate().is_err());

        let mut foreground = MotionGlassForegroundProgram::from_owner(&owner, "hero-lens").unwrap();
        foreground.surface_id = "s".repeat(4_097);
        assert!(foreground.validate().is_err());
    }

    #[test]
    fn foreground_marker_rejects_valid_but_owner_mismatched_geometry() {
        let owner = sample_program();
        let mut foreground = MotionGlassForegroundProgram::from_owner(&owner, "hero-lens").unwrap();
        foreground.presence = 0.5;
        foreground
            .validate()
            .expect("the forged marker is locally valid");

        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        let mut foreground_group = Group::plain(Vec::new());
        foreground_group.glass_foreground = Some(Box::new(foreground));
        let foreground_node = builder.push_node(Node::Group(foreground_group));
        let mut owner_group = Group::plain(vec![foreground_node]);
        owner_group.glass = Some(Box::new(owner));
        let root = builder.push_node(Node::Group(owner_group));
        builder.add_root(root);

        let error = builder
            .finish()
            .expect_err("foreground geometry drift must fail closed");
        assert!(
            matches!(
                error,
                DrawProgramError::InvalidValue { ref reason, .. }
                    if reason.contains("exact geometry and metadata")
            ),
            "unexpected validation error: {error}"
        );
    }

    #[test]
    fn json_wire_rejects_unknown_fields() {
        let mut value = serde_json::to_value(sample_program()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("futureField".into(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<MotionGlassProgram>(value).is_err());
    }

    #[test]
    fn projective_horizon_crossing_fails_closed() {
        let mut program = sample_program();
        program.surfaces[0].local_to_owner = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -0.02, 0.0, 1.0];
        assert!(program.validate().is_err());
    }

    #[test]
    fn unsorted_surfaces_fail_closed() {
        let mut program = sample_program();
        program.surfaces.push(PackedGlassSurface {
            surface_id: "a-first".into(),
            ..program.surfaces[0].clone()
        });
        assert!(program.validate().is_err());
    }

    #[test]
    fn pointer_like_kernel_id_is_rejected() {
        let mut program = sample_program();
        program.kernel_id = "gpu-handle".into();
        assert!(program.validate().is_err());
    }

    #[test]
    fn owner_identity_and_backdrop_scope_are_part_of_validation() {
        let mut wrong_owner = sample_program();
        wrong_owner.owner_id = "different-owner".into();
        assert!(wrong_owner.validate().is_err());

        let mut wrong_scope = sample_program();
        wrong_scope.backdrop.scope = BackdropScope::LayerEntry("wrong".into());
        assert!(wrong_scope.validate().is_err());
    }

    #[test]
    fn roughness_is_unit_bounded() {
        let mut program = sample_program();
        program.material.roughness = 1.0;
        assert!(program.validate().is_ok());
        program.material.roughness = 1.000_1;
        assert!(program.validate().is_err());
    }

    #[test]
    fn path_program_validates_and_rejects_bad_points() {
        let mut program = sample_program();
        program.surfaces[0].shape = PackedGlassShapeKind::Path;
        program.surfaces[0].radius = 0.0;
        program.surfaces[0].path_points =
            vec![[10.0, 20.0], [90.0, 20.0], [90.0, 100.0], [10.0, 100.0]];
        program
            .validate()
            .expect("closed polygon with >= 3 points validates");
        program.surfaces[0].path_points =
            vec![[10.0, 20.0], [90.0, 100.0], [90.0, 20.0], [10.0, 100.0]];
        assert!(
            program.validate().is_err(),
            "self-intersecting paths are ambiguous and must fail closed"
        );
        program.surfaces[0].path_points = vec![[0.0, 0.0], [100.0, 0.0]];
        assert!(program.validate().is_err(), "Path requires >= 3 points");
        program.surfaces[0].shape = PackedGlassShapeKind::Circle;
        program.surfaces[0].radius = 40.0;
        program.surfaces[0].path_points = vec![[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]];
        assert!(
            program.validate().is_err(),
            "analytic shapes must not carry points"
        );
    }

    #[test]
    fn requirements_derive_for_independent_circle() {
        let program = sample_program();
        let requirements = program.derive_requirements().unwrap();
        assert_eq!(
            requirements.output_bounds.rect(),
            Some(Rect::new(10.0, 20.0, 80.0, 80.0))
        );
        assert_eq!(requirements.destination_uses.len(), 1);
        assert_eq!(
            requirements.destination_uses[0].scope,
            BackdropScope::Current
        );
        assert_eq!(
            requirements.destination_uses[0].sample_bounds,
            Rect::new(0.0, 0.0, 100.0, 100.0)
        );
        assert!(
            requirements
                .capabilities
                .contains(&DrawCapability::BackdropRead)
        );
        assert!(requirements.external_textures.is_empty());
        assert!(requirements.fonts.is_empty());
        assert!(requirements.runtime_shaders.is_empty());
        assert!(requirements.scene3d.is_empty());
        assert!(requirements.max_intermediate_pixels >= 80 * 80);
    }

    #[test]
    fn requirements_accept_field_payload() {
        let mut program = sample_program();
        program.owner_kind = GlassOwnerKind::Field;
        program.owner_id = "orbit".into();
        program.field = Some(PackedGlassField {
            field_id: "orbit".into(),
            merge_distance: 24.0,
            member_ids: vec!["hero-lens".into()],
        });
        program.backdrop.scope = BackdropScope::ScopeEntry("orbit".into());
        let requirements = program.derive_requirements().expect("field is executable");
        assert!(
            requirements
                .capabilities
                .contains(&DrawCapability::MotionGlass)
        );
    }

    #[test]
    fn requirements_reject_non_current_scope() {
        let mut program = sample_program();
        program.backdrop.scope = BackdropScope::LayerEntry("wrong".into());
        assert!(matches!(
            program.derive_requirements(),
            Err(GlassRequirementsError::InvalidScope)
        ));
    }

    #[test]
    fn requirements_reject_empty_bounds() {
        let mut program = sample_program();
        program.backdrop.output_bounds = Rect::new(0.0, 0.0, 0.0, 0.0);
        assert!(matches!(
            program.derive_requirements(),
            Err(GlassRequirementsError::EmptyBounds)
        ));
    }
}
