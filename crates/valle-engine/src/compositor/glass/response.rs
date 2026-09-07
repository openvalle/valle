//! Compact-support causal kernels. Support is `settle` seconds; K=0 outside [0, τ].

use valle_draw::program::glass::{
    KERNEL_RESPONSE_MANIFEST, PackedGlassCharacter, PackedGlassMotion,
};

use super::kinematics::GlassKinematics;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterKernel {
    Responsive,
    Fluid,
    Viscous,
    Elastic,
}

/// Fixed quadrature count used to resolve the causal response during prepare. The resolved draw
/// program carries only the current response; delayed samples are an implementation detail and
/// must not bloat the frame ABI or be recomputed by an executor.
pub const RESPONSE_INTEGRATION_SAMPLES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResponseManifest {
    pub character: CharacterKernel,
    pub a: i32,
    pub b: i32,
    pub elastic: bool,
}

impl CharacterKernel {
    /// Beta-density coefficients from the shared kernel manifest (valle-draw); the same table
    /// feeds the kernel digest that Native/Web bundles must match.
    pub fn manifest(self) -> ResponseManifest {
        let packed = match self {
            Self::Responsive => PackedGlassCharacter::Responsive,
            Self::Fluid => PackedGlassCharacter::Fluid,
            Self::Viscous => PackedGlassCharacter::Viscous,
            Self::Elastic => PackedGlassCharacter::Elastic,
        };
        let mut coefficients = (3, 6, false);
        let mut index = 0usize;
        while index < KERNEL_RESPONSE_MANIFEST.len() {
            let (character, a, b, elastic) = KERNEL_RESPONSE_MANIFEST[index];
            if character == packed {
                coefficients = (a, b, elastic);
                break;
            }
            index += 1;
        }
        ResponseManifest {
            character: self,
            a: coefficients.0,
            b: coefficients.1,
            elastic: coefficients.2,
        }
    }
}

fn beta(u: f64, a: i32, b: i32) -> f64 {
    if u <= 0.0 || u >= 1.0 {
        0.0
    } else {
        u.powi(a) * (1.0 - u).powi(b)
    }
}

/// Normalized kernel density at delay `s` in seconds, support `[0, settle]`.
pub fn kernel_value(character: CharacterKernel, s: f64, settle: f64) -> f64 {
    let normalization = kernel_normalization(character, settle);
    kernel_value_normalized(character, s, settle, normalization)
}

fn kernel_normalization(character: CharacterKernel, settle: f64) -> f64 {
    if settle <= 0.0 || !settle.is_finite() {
        return 0.0;
    }
    let spec = character.manifest();
    let mut integral = 0.0;
    let samples = 64;
    for i in 0..=samples {
        let t = i as f64 / samples as f64;
        let mut value = beta(t, spec.a, spec.b);
        if spec.elastic {
            value *= valle_draw::math::cos(std::f64::consts::PI * t);
        }
        let weight = if i == 0 || i == samples { 0.5 } else { 1.0 };
        integral += weight * if spec.elastic { value.abs() } else { value };
    }
    integral * settle / samples as f64
}

fn kernel_value_normalized(
    character: CharacterKernel,
    s: f64,
    settle: f64,
    normalization: f64,
) -> f64 {
    if s < 0.0 || s > settle || settle <= 0.0 {
        return 0.0;
    }
    let spec = character.manifest();
    let u = s / settle;
    let mut value = beta(u, spec.a, spec.b);
    if spec.elastic {
        value *= valle_draw::math::cos(std::f64::consts::PI * u);
    }
    // Elastic response is intentionally signed. Its ordinary integral is zero for the symmetric
    // manifest, so `kernel_normalization` uses L1 support instead of erasing the kernel.
    if normalization.abs() < 1e-18 {
        0.0
    } else {
        value / normalization
    }
}

/// Causal convolution of a scalar force history. `force[i]` is F(t - i*dt)` with i=0 current.
pub fn causal_response(character: CharacterKernel, settle: f64, dt: f64, force: &[f64]) -> f64 {
    if dt <= 0.0 || settle <= 0.0 || force.is_empty() {
        return 0.0;
    }
    let normalization = kernel_normalization(character, settle);
    let mut acc = 0.0;
    for (i, f) in force.iter().enumerate() {
        let s = i as f64 * dt;
        if s > settle {
            break;
        }
        acc += kernel_value_normalized(character, s, settle, normalization) * f * dt;
    }
    acc
}

/// Signed, orthogonal response channels before causal convolution.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GlassResponse {
    pub translation: [f64; 2],
    pub acceleration: [f64; 2],
    pub angular: f64,
    pub scale: [f64; 2],
    pub shear: f64,
    pub area: f64,
    pub pressure: f64,
    pub twist: f64,
}

impl GlassResponse {
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

    fn add_scaled(&mut self, other: Self, scale: f64) {
        self.translation[0] += other.translation[0] * scale;
        self.translation[1] += other.translation[1] * scale;
        self.acceleration[0] += other.acceleration[0] * scale;
        self.acceleration[1] += other.acceleration[1] * scale;
        self.angular += other.angular * scale;
        self.scale[0] += other.scale[0] * scale;
        self.scale[1] += other.scale[1] * scale;
        self.shear += other.shear * scale;
        self.area += other.area * scale;
        self.pressure += other.pressure * scale;
        self.twist += other.twist * scale;
    }

    fn lerp(self, other: Self, t: f64) -> Self {
        let mut output = self;
        output.add_scaled(self, -t);
        output.add_scaled(other, t);
        output
    }

    fn packed(self) -> PackedGlassMotion {
        PackedGlassMotion {
            translation: self.translation.map(|value| value as f32),
            acceleration: self.acceleration.map(|value| value as f32),
            angular: self.angular as f32,
            scale: self.scale.map(|value| value as f32),
            shear: self.shear as f32,
            area: self.area as f32,
            pressure: self.pressure as f32,
            twist: self.twist as f32,
        }
    }

    fn is_finite(self) -> bool {
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
        .all(f64::is_finite)
    }
}

/// Automatic kinematics and authored drive add channel-by-channel, preserving direction and
/// acceleration. Non-finite or negative intensity fails closed to a zero vector.
pub fn response_force(
    kinematics: GlassKinematics,
    drive: [f64; 4],
    intensity: f64,
) -> GlassResponse {
    if !intensity.is_finite() || intensity < 0.0 {
        return GlassResponse::ZERO;
    }
    let response = GlassResponse {
        translation: [
            (kinematics.linear_velocity[0] + drive[0]) * intensity,
            (kinematics.linear_velocity[1] + drive[1]) * intensity,
        ],
        acceleration: kinematics
            .linear_acceleration
            .map(|value| value * intensity),
        angular: kinematics.angular_velocity * intensity,
        scale: kinematics.scale_rate.map(|value| value * intensity),
        shear: kinematics.shear_rate * intensity,
        area: kinematics.area_rate * intensity,
        pressure: drive[2] * intensity,
        twist: drive[3] * intensity,
    };
    if response.is_finite() {
        response
    } else {
        GlassResponse::ZERO
    }
}

/// Resolves the current backend-neutral response
/// `Q(t) = ∫₀^settle K(u)·F(t−u)·du` on a fixed quadrature grid with linear interpolation of the
/// force history.
///
/// `history` is `(seconds, force)` sorted ascending by time, relative to the current sample
/// (times ≤ 0, newest last). Before the first sample the force is rest (0), so motion that
/// just started produces no pre-response. Crossing an epoch (`epoch_reset`) forbids all
/// pre-response: the envelope is zero and the kernel restarts from the next sample.
///
/// Executors never re-derive the response themselves; the frame ABI carries this one value.
pub fn packed_current_response(
    character: CharacterKernel,
    settle: f64,
    history: &[(f64, GlassResponse)],
    epoch_reset: bool,
) -> PackedGlassMotion {
    if epoch_reset || settle <= 0.0 || history.is_empty() {
        return PackedGlassMotion::ZERO;
    }
    let dt = settle / RESPONSE_INTEGRATION_SAMPLES as f64;
    let normalization = kernel_normalization(character, settle);
    let mut response = GlassResponse::ZERO;
    for sample in 0..RESPONSE_INTEGRATION_SAMPLES {
        let delay = sample as f64 * dt;
        let force = sample_force(history, -delay);
        response.add_scaled(
            force,
            kernel_value_normalized(character, delay, settle, normalization) * dt,
        );
    }
    response.packed()
}

/// Force at `at` (seconds, ≤ 0): linear interpolation between bracketing history samples;
/// before the first sample the surface was at rest (0).
fn sample_force(history: &[(f64, GlassResponse)], at: f64) -> GlassResponse {
    if at > 0.0 {
        return history
            .last()
            .map_or(GlassResponse::ZERO, |sample| sample.1);
    }
    if at <= history[0].0 {
        return if at == history[0].0 {
            history[0].1
        } else {
            GlassResponse::ZERO
        };
    }
    for pair in history.windows(2) {
        let (t0, f0) = pair[0];
        let (t1, f1) = pair[1];
        if at >= t0 && at <= t1 {
            let span = t1 - t0;
            if span <= 0.0 {
                return f1;
            }
            let t = (at - t0) / span;
            return f0.lerp(f1, t);
        }
    }
    history
        .last()
        .map_or(GlassResponse::ZERO, |sample| sample.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn force(x: f64) -> GlassResponse {
        GlassResponse {
            translation: [x, 0.0],
            ..GlassResponse::ZERO
        }
    }

    #[test]
    fn kernel_is_causal_and_compact() {
        let settle = 0.32;
        assert_eq!(kernel_value(CharacterKernel::Fluid, -0.01, settle), 0.0);
        assert_eq!(
            kernel_value(CharacterKernel::Fluid, settle + 1e-9, settle),
            0.0
        );
        assert!(kernel_value(CharacterKernel::Fluid, 0.1, settle).abs() > 0.0);
    }

    #[test]
    fn rest_force_yields_zero_response() {
        let force = [0.0; 16];
        let q = causal_response(CharacterKernel::Responsive, 0.32, 1.0 / 240.0, &force);
        assert!(q.abs() < 1e-12);
    }

    #[test]
    fn no_pre_response_before_motion_starts() {
        // Rest up to t=0: force history all zero.
        let history = [(-0.08, force(0.0)), (-0.04, force(0.0)), (0.0, force(0.0))];
        let response = packed_current_response(CharacterKernel::Fluid, 0.32, &history, false);
        assert_eq!(response, PackedGlassMotion::ZERO);
    }

    #[test]
    fn step_force_rises_then_stops_after_support() {
        let settle = 0.32;
        let dt = settle / 8.0;
        // Step 1.0 started one tap before now; force 1.0 from -dt onward.
        let history = [
            (-2.0 * dt, force(0.0)),
            (-dt, force(1.0)),
            (0.0, force(1.0)),
        ];
        let response = packed_current_response(CharacterKernel::Fluid, settle, &history, false);
        assert!(
            response.translation[0] > 0.0,
            "step must produce current response"
        );
        // Force stops a full support before now: the response has decayed to rest.
        let stopped = (0..24)
            .map(|i| {
                let t = -(23 - i) as f64 * dt;
                (t, force(if t <= -settle { 1.0 } else { 0.0 }))
            })
            .collect::<Vec<_>>();
        let response = packed_current_response(CharacterKernel::Fluid, settle, &stopped, false);
        assert!(
            response.translation[0].abs() < 1e-3,
            "stopped force must be at rest within support, got {:?}",
            response
        );
    }

    #[test]
    fn epoch_reset_forbids_pre_response() {
        let history = [
            (-0.08, force(10.0)),
            (-0.04, force(10.0)),
            (0.0, force(10.0)),
        ];
        let response = packed_current_response(CharacterKernel::Fluid, 0.32, &history, true);
        assert_eq!(response, PackedGlassMotion::ZERO);
    }

    #[test]
    fn steady_force_reaches_normalized_steady_state() {
        // Normalized kernel integrates to 1, so a long constant force converges to F itself.
        let settle = 0.32;
        let dt = settle / 8.0;
        let history = (0..24)
            .map(|i| (-(23 - i) as f64 * dt, force(100.0)))
            .collect::<Vec<_>>();
        let response = packed_current_response(CharacterKernel::Fluid, settle, &history, false);
        assert!(
            (f64::from(response.translation[0]) - 100.0).abs() < 8.0,
            "steady-state response should approach force, got {:?}",
            response
        );
    }

    #[test]
    fn drive_and_kinematics_add_at_one_force_point() {
        let kinematics = GlassKinematics {
            linear_velocity: [10.0, 0.0],
            ..GlassKinematics::ZERO
        };
        let drive = [3.0, 4.0, 5.0, 6.0];
        let force = response_force(kinematics, drive, 2.0);
        assert_eq!(force.translation, [26.0, 8.0]);
        assert_eq!(force.pressure, 10.0);
        assert_eq!(force.twist, 12.0);
    }

    #[test]
    fn degenerate_intensity_fails_closed() {
        let force = response_force(GlassKinematics::ZERO, [1.0, 0.0, 0.0, 0.0], f64::NAN);
        assert_eq!(force, GlassResponse::ZERO);
    }

    #[test]
    fn response_preserves_direction_acceleration_and_elastic_sign() {
        let negative = response_force(
            GlassKinematics {
                linear_velocity: [-20.0, 7.0],
                linear_acceleration: [-3.0, 4.0],
                ..GlassKinematics::ZERO
            },
            [0.0; 4],
            1.0,
        );
        assert_eq!(negative.translation, [-20.0, 7.0]);
        assert_eq!(negative.acceleration, [-3.0, 4.0]);
        assert!(kernel_value(CharacterKernel::Elastic, 0.08, 0.32) > 0.0);
        assert!(kernel_value(CharacterKernel::Elastic, 0.24, 0.32) < 0.0);

        let history = (0..24)
            .map(|index| (-(23 - index) as f64 * 0.04, force(-10.0)))
            .collect::<Vec<_>>();
        let response = packed_current_response(CharacterKernel::Fluid, 0.32, &history, false);
        assert!(response.translation[0] < 0.0);
    }

    #[test]
    fn manifest_matches_shared_kernel_table() {
        for character in [
            CharacterKernel::Responsive,
            CharacterKernel::Fluid,
            CharacterKernel::Viscous,
            CharacterKernel::Elastic,
        ] {
            let manifest = character.manifest();
            let packed = match character {
                CharacterKernel::Responsive => PackedGlassCharacter::Responsive,
                CharacterKernel::Fluid => PackedGlassCharacter::Fluid,
                CharacterKernel::Viscous => PackedGlassCharacter::Viscous,
                CharacterKernel::Elastic => PackedGlassCharacter::Elastic,
            };
            let (_, a, b, elastic) = KERNEL_RESPONSE_MANIFEST
                .iter()
                .find(|(c, _, _, _)| *c == packed)
                .expect("character in table");
            assert_eq!(manifest.a, *a);
            assert_eq!(manifest.b, *b);
            assert_eq!(manifest.elastic, *elastic);
        }
    }
}
