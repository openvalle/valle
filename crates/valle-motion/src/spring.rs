//! Closed-form damped harmonic oscillator.

use valle_draw::math;

/// Compile-time physical parameters. Explicit parameters and named presets are mutually exclusive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringParams {
    pub mass: f64,
    pub stiffness: f64,
    pub damping: f64,
}

/// Sample the oscillator at time `t` in seconds, starting at zero with zero velocity and converging
/// to one. A closed-form solution keeps the curve independent of frame rate. Negative time returns
/// zero; convergence continues without a rest threshold.
pub fn spring_at(t: f64, params: SpringParams) -> f64 {
    if !t.is_finite() || t <= 0.0 {
        return 0.0;
    }
    let SpringParams {
        mass,
        stiffness,
        damping,
    } = params;
    // Natural frequency and damping ratio; admission requires finite, positive parameters.
    let omega0 = math::sqrt(stiffness / mass);
    let zeta = damping / (2.0 * math::sqrt(stiffness * mass));

    // Solve y'' + 2*zeta*omega0*y' + omega0^2*y = 0 with y(0)=1 and y'(0)=0, then return 1-y.
    let y = if zeta < 1.0 {
        // Underdamped: oscillatory convergence with overshoot.
        let omega_d = omega0 * math::sqrt(1.0 - zeta * zeta);
        let (sin, cos) = math::sin_cos(omega_d * t);
        math::exp(-zeta * omega0 * t) * (cos + (zeta * omega0 / omega_d) * sin)
    } else if zeta == 1.0 {
        // Critically damped: fastest convergence without overshoot.
        math::exp(-omega0 * t) * (1.0 + omega0 * t)
    } else {
        // Overdamped: two real roots and slower convergence.
        let root = omega0 * math::sqrt(zeta * zeta - 1.0);
        let r1 = -omega0 * zeta + root;
        let r2 = -omega0 * zeta - root;
        (r2 * math::exp(r1 * t) - r1 * math::exp(r2 * t)) / (r2 - r1)
    };
    1.0 - y
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
        };
        let over = SpringParams {
            mass: 1.0,
            stiffness: 100.0,
            damping: 40.0,
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
}
