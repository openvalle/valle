//! Closed-form damped harmonic oscillator.

use serde::{Deserialize, Serialize};
use valle_draw::math;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpringOutput {
    Position,
    Velocity,
}

/// Compile-time physical parameters. Explicit parameters and named presets are mutually exclusive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringParams {
    pub mass: f64,
    pub stiffness: f64,
    pub damping: f64,
    /// Normalized distance per second at time zero; independent of render frame rate.
    pub initial_velocity: f64,
}

/// Position and its analytic time derivative. Neither depends on a previously rendered frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringSample {
    pub position: f64,
    pub velocity: f64,
}

/// Sample the oscillator at time `t` in seconds, starting at zero and converging to one.
pub fn spring_at(t: f64, params: SpringParams) -> f64 {
    spring_sample_at(t, params).position
}

/// Closed-form damped motion, with velocity in normalized distance per second. Before time zero
/// both outputs are zero; at zero velocity equals the explicit initial condition. No numerical
/// integration, rest threshold, frame history or forced settling is involved.
pub fn spring_sample_at(t: f64, params: SpringParams) -> SpringSample {
    if !t.is_finite() || t < 0.0 {
        return SpringSample {
            position: 0.0,
            velocity: 0.0,
        };
    }
    if t == 0.0 {
        return SpringSample {
            position: 0.0,
            velocity: params.initial_velocity,
        };
    }
    let SpringParams {
        mass,
        stiffness,
        damping,
        initial_velocity: v0,
    } = params;
    // Natural frequency and damping ratio; admission requires finite, positive parameters.
    let omega0 = math::sqrt(stiffness / mass);
    let zeta = damping / (2.0 * math::sqrt(stiffness * mass));

    // Solve for y=1-x, with y(0)=1 and y'(0)=-v0.
    let decay = zeta * omega0;
    let (y, velocity) = if zeta < 1.0 {
        // Underdamped: oscillatory convergence with overshoot.
        let omega_d = omega0 * math::sqrt(1.0 - zeta * zeta);
        let (sin, cos) = math::sin_cos(omega_d * t);
        let envelope = math::exp(-decay * t);
        (
            envelope * (cos + ((decay - v0) / omega_d) * sin),
            envelope * (v0 * cos + ((omega0 * omega0 - decay * v0) / omega_d) * sin),
        )
    } else if zeta == 1.0 {
        // Critically damped: fastest convergence without overshoot.
        let envelope = math::exp(-omega0 * t);
        (
            envelope * (1.0 + (omega0 - v0) * t),
            envelope * (v0 + omega0 * (omega0 - v0) * t),
        )
    } else {
        // Overdamped: two real roots and slower convergence.
        // Rationalize the slow root to avoid subtracting two almost equal values.
        let sum = zeta + math::sqrt(zeta * zeta - 1.0);
        let r1 = -omega0 / sum;
        let r2 = -omega0 * sum;
        let slow = (r2 + v0) * math::exp(r1 * t);
        let fast = (r1 + v0) * math::exp(r2 * t);
        (
            (slow - fast) / (r2 - r1),
            (r2 * fast - r1 * slow) / (r2 - r1),
        )
    };
    SpringSample {
        position: 1.0 - y,
        velocity,
    }
}

/// Named physical presets shared by all evaluators. Compilation rejects mixing a preset with
/// explicit parameters.
pub fn preset(name: &str) -> Option<SpringParams> {
    let (stiffness, damping) = match name {
        "gentle" => (120.0, 14.0),
        "wobbly" => (180.0, 12.0),
        "stiff" => (210.0, 20.0),
        "slow" => (280.0, 60.0),
        "bouncy" => (140.0, 8.0),
        _ => return None,
    };
    Some(SpringParams {
        mass: 1.0,
        stiffness,
        damping,
        initial_velocity: 0.0,
    })
}

/// Supported preset names for diagnostics.
pub const PRESETS: &[&str] = &["gentle", "wobbly", "stiff", "slow", "bouncy"];

#[cfg(test)]
mod tests {
    use super::*;

    const GENTLE: SpringParams = SpringParams {
        mass: 1.0,
        stiffness: 120.0,
        damping: 14.0,
        initial_velocity: 0.0,
    };

    #[test]
    fn before_the_phase_starts_the_spring_sits_at_its_start_value() {
        // The oscillator has not started before time zero.
        assert_eq!(spring_at(-1.0, GENTLE), 0.0);
        assert_eq!(spring_at(-0.001, GENTLE), 0.0);
        assert_eq!(spring_at(0.0, GENTLE), 0.0);
    }

    #[test]
    fn the_spring_converges_towards_one_and_keeps_evolving_after_the_window() {
        // Continue convergence beyond the phase window.
        let a = spring_at(0.5, GENTLE);
        let b = spring_at(1.0, GENTLE);
        let c = spring_at(4.0, GENTLE);
        assert!(a > 0.0 && a < 1.2, "{a}");
        assert!((c - 1.0).abs() < 1e-6, "far from rest: {c}");
        assert!(b != a, "the spring must keep moving between samples");
    }

    #[test]
    fn critical_and_over_damping_do_not_overshoot() {
        // Critical and overdamped branches must not overshoot.
        let critical = SpringParams {
            mass: 1.0,
            stiffness: 100.0,
            damping: 20.0,
            initial_velocity: 0.0,
        };
        let over = SpringParams {
            mass: 1.0,
            stiffness: 100.0,
            damping: 40.0,
            initial_velocity: 0.0,
        };
        for params in [critical, over] {
            for step in 1..200 {
                let value = spring_at(f64::from(step) * 0.01, params);
                assert!(
                    (0.0..=1.0).contains(&value),
                    "non-oscillating spring overshot: {value}"
                );
            }
        }
    }

    #[test]
    fn an_underdamped_spring_overshoots_at_least_once() {
        // The underdamped branch must overshoot.
        let overshot = (1..400)
            .map(|step| spring_at(f64::from(step) * 0.01, GENTLE))
            .any(|value| value > 1.0);
        assert!(overshot, "an underdamped spring must overshoot");
    }

    #[test]
    fn analytic_velocity_matches_the_position_derivative_in_every_damping_regime() {
        for damping in [0.0, 8.0, 19.99999999, 20.0, 20.00000001, 40.0, 2000.0] {
            for initial_velocity in [-4.0, 0.0, 6.0] {
                let params = SpringParams {
                    mass: 1.0,
                    stiffness: 100.0,
                    damping,
                    initial_velocity,
                };
                for t in [0.001, 0.04, 0.2, 0.6, 2.0, 8.0] {
                    let h = 0.000001;
                    let before = spring_sample_at(t - h, params);
                    let sample = spring_sample_at(t, params);
                    let after = spring_sample_at(t + h, params);
                    let derivative = (after.position - before.position) / (2.0 * h);
                    assert!(
                        (sample.velocity - derivative).abs() < 0.00001,
                        "{params:?} at {t}: analytic={} numeric={derivative}",
                        sample.velocity
                    );
                    // Fourth-order difference keeps truncation error small even for the fast
                    // transient of a heavily overdamped spring.
                    let acceleration = (-spring_sample_at(t + 2.0 * h, params).velocity
                        + 8.0 * after.velocity
                        - 8.0 * before.velocity
                        + spring_sample_at(t - 2.0 * h, params).velocity)
                        / (12.0 * h);
                    let force =
                        params.stiffness * (1.0 - sample.position) - damping * sample.velocity;
                    assert!(
                        (acceleration - force).abs() < 0.001,
                        "oscillator equation: {params:?} at {t}: {acceleration} != {force}"
                    );
                }
            }
        }
    }

    #[test]
    fn initial_velocity_is_explicit_and_preserves_a_linear_handoff() {
        let params = SpringParams {
            initial_velocity: 0.5,
            ..GENTLE
        };
        assert_eq!(
            spring_sample_at(-0.01, params),
            SpringSample {
                position: 0.0,
                velocity: 0.0
            }
        );
        let start = spring_sample_at(0.0, params);
        // A preceding segment ends at x=100, moving at 100px/s. The spring travels 200px.
        assert_eq!(100.0 + 200.0 * start.position, 100.0);
        assert_eq!(200.0 * start.velocity, 100.0);
        let right_slope = 200.0 * spring_at(0.0000001, params) / 0.0000001;
        assert!((right_slope - 100.0).abs() < 0.002);
        let rest = spring_sample_at(10.0, params);
        assert!((rest.position - 1.0).abs() < 1e-12);
        assert!(rest.velocity.abs() < 1e-12);
    }

    #[test]
    fn an_undamped_spring_is_never_forced_to_rest() {
        let params = SpringParams {
            stiffness: 100.0,
            damping: 0.0,
            ..GENTLE
        };
        for t in [0.2, 6.0, 60.0] {
            let sample = spring_sample_at(t, params);
            assert_eq!(sample.position, 1.0 - math::cos(10.0 * t));
            assert_eq!(sample.velocity, 10.0 * math::sin(10.0 * t));
        }
    }
}
