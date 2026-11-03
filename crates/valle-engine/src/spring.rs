//! Springs: a damped oscillator from 0 to 1, in closed form, so any time can be asked for.

use std::f64::consts::TAU;

/// A spring's mass, stiffness and damping, and its velocity at time zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    mass: f64,
    stiffness: f64,
    damping: f64,
    velocity: f64,
}

/// Stiffness and damping of a unit mass, as react-spring's presets have them.
const PRESETS: [(&str, f64, f64); 5] = [
    ("gentle", 120.0, 14.0),
    ("wobbly", 180.0, 12.0),
    ("stiff", 210.0, 20.0),
    ("slow", 280.0, 60.0),
    ("bouncy", 140.0, 8.0),
];

/// The options `spring()` takes, in the order the runtime passes them.
pub const OPTIONS: [&str; 7] = [
    "preset",
    "duration",
    "bounce",
    "mass",
    "stiffness",
    "damping",
    "initialVelocity",
];

impl Spring {
    /// A spring from its options, each written or not: a preset, SwiftUI's duration and bounce, or
    /// mass, stiffness and damping, with 1, 100 and 10 for those left out.
    pub fn new(options: [Option<&str>; 7]) -> Result<Spring, String> {
        let [preset, duration, bounce, mass, stiffness, damping, velocity] = options;
        let number = |name: &str, v: Option<&str>| match v.map(str::parse::<f64>) {
            None => Ok(None),
            Some(Ok(x)) if x.is_finite() => Ok(Some(x)),
            _ => Err(format!("spring {name} must be a number")),
        };
        let physics = [
            ("mass", mass),
            ("stiffness", stiffness),
            ("damping", damping),
        ];
        let swift = [("duration", duration), ("bounce", bounce)];
        let (mass, stiffness, damping) = if let Some(name) = preset {
            if let Some(other) = first_written(&swift).or(first_written(&physics)) {
                return Err(format!("spring takes a preset or {other}, not both"));
            }
            let Some(&(_, stiffness, damping)) = PRESETS.iter().find(|p| p.0 == name) else {
                let names: Vec<&str> = PRESETS.iter().map(|p| p.0).collect();
                return Err(format!(
                    "spring preset {name} is not one of {}",
                    names.join(", ")
                ));
            };
            (1.0, stiffness, damping)
        } else if first_written(&swift).is_some() {
            if let Some(other) = first_written(&physics) {
                return Err(format!(
                    "spring takes duration and bounce or {other}, not both"
                ));
            }
            let duration = number("duration", duration)?.unwrap_or(0.5);
            let bounce = number("bounce", bounce)?.unwrap_or(0.0);
            if duration <= 0.0 || bounce <= -1.0 || bounce >= 1.0 {
                return Err("spring duration must be over 0 and bounce between -1 and 1".into());
            }
            // SwiftUI's: the undamped spring swings once a duration; a bounce under 0 overdamps.
            let ratio = if bounce < 0.0 {
                1.0 / (1.0 + bounce)
            } else {
                1.0 - bounce
            };
            let w = TAU / duration;
            (1.0, w * w, 2.0 * ratio * w)
        } else {
            (
                number("mass", mass)?.unwrap_or(1.0),
                number("stiffness", stiffness)?.unwrap_or(100.0),
                number("damping", damping)?.unwrap_or(10.0),
            )
        };
        if mass <= 0.0 || stiffness <= 0.0 || damping < 0.0 {
            return Err("spring mass and stiffness must be over 0, damping at least 0".into());
        }
        Ok(Spring {
            mass,
            stiffness,
            damping,
            velocity: number("initialVelocity", velocity)?.unwrap_or(0.0),
        })
    }

    /// Where the spring is at `t` seconds, from 0 to 1 and past it, and how fast it moves: at 0
    /// before time zero, its initial velocity at zero.
    pub fn at(&self, t: f64) -> (f64, f64) {
        if t.is_nan() {
            return (f64::NAN, f64::NAN);
        }
        if t <= 0.0 {
            return (0.0, if t == 0.0 { self.velocity } else { 0.0 });
        }
        let Spring {
            mass,
            stiffness,
            damping,
            velocity: v0,
        } = *self;
        let omega = (stiffness / mass).sqrt();
        let zeta = damping / (2.0 * (stiffness * mass).sqrt());
        let decay = zeta * omega;
        // y = 1 - x, with y(0) = 1 and y'(0) = -v0.
        let (y, velocity) = if zeta < 1.0 {
            let w = omega * (1.0 - zeta * zeta).sqrt();
            let (sin, cos) = (w * t).sin_cos();
            let envelope = (-decay * t).exp();
            (
                envelope * (cos + (decay - v0) / w * sin),
                envelope * (v0 * cos + (omega * omega - decay * v0) / w * sin),
            )
        } else if zeta == 1.0 {
            let envelope = (-omega * t).exp();
            (
                envelope * (1.0 + (omega - v0) * t),
                envelope * (v0 + omega * (omega - v0) * t),
            )
        } else {
            // The slow root rationalized, so two almost equal values are not subtracted.
            let sum = zeta + (zeta * zeta - 1.0).sqrt();
            let (r1, r2) = (-omega / sum, -omega * sum);
            let slow = (r2 + v0) * (r1 * t).exp();
            let fast = (r1 + v0) * (r2 * t).exp();
            (
                (slow - fast) / (r2 - r1),
                (r2 * fast - r1 * slow) / (r2 - r1),
            )
        };
        (1.0 - y, velocity)
    }
}

/// The name of the first option of `list` that is written.
fn first_written<'a>(list: &[(&'a str, Option<&str>)]) -> Option<&'a str> {
    list.iter().find(|(_, v)| v.is_some()).map(|p| p.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spring(options: &[(&str, &str)]) -> Result<Spring, String> {
        Spring::new(OPTIONS.map(|k| options.iter().find(|o| o.0 == k).map(|o| o.1)))
    }

    /// The equation of motion stepped with RK4: m x'' = -k (x - 1) - c x'.
    fn stepped(s: Spring, t: f64) -> (f64, f64) {
        let n = (t * 20000.0).ceil() as usize;
        let h = t / n as f64;
        let f = |x: f64, v: f64| (v, (-s.stiffness * (x - 1.0) - s.damping * v) / s.mass);
        let (mut x, mut v) = (0.0, s.velocity);
        for _ in 0..n {
            let k1 = f(x, v);
            let k2 = f(x + h / 2.0 * k1.0, v + h / 2.0 * k1.1);
            let k3 = f(x + h / 2.0 * k2.0, v + h / 2.0 * k2.1);
            let k4 = f(x + h * k3.0, v + h * k3.1);
            x += h / 6.0 * (k1.0 + 2.0 * k2.0 + 2.0 * k3.0 + k4.0);
            v += h / 6.0 * (k1.1 + 2.0 * k2.1 + 2.0 * k3.1 + k4.1);
        }
        (x, v)
    }

    #[test]
    fn springs_move_as_their_equation_says() {
        for options in [
            &[][..],
            &[("preset", "wobbly")],
            &[("preset", "slow")],
            &[("mass", "2"), ("stiffness", "50"), ("damping", "20")],
            &[("stiffness", "100"), ("damping", "20")],
            &[("damping", "0")],
            &[("duration", "0.8"), ("bounce", "0.4")],
            &[("duration", "0.4"), ("bounce", "-0.5")],
            &[("preset", "bouncy"), ("initialVelocity", "-6")],
            &[("damping", "20"), ("initialVelocity", "8")],
        ] {
            let s = spring(options).unwrap();
            for t in [0.05, 0.3, 0.7, 1.5] {
                let (ours, exact) = (s.at(t), stepped(s, t));
                let close = (ours.0 - exact.0).abs() < 1e-9 && (ours.1 - exact.1).abs() < 1e-7;
                assert!(close, "{options:?} at {t}: {ours:?} {exact:?}");
            }
        }
    }

    #[test]
    fn springs_rest_before_they_start() {
        let s = spring(&[("initialVelocity", "3")]).unwrap();
        assert_eq!(s.at(-1.0), (0.0, 0.0));
        assert_eq!(s.at(0.0), (0.0, 3.0));
        assert!((s.at(30.0).0 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn options_that_disagree_are_errors() {
        for bad in [
            &[("preset", "wobbly"), ("mass", "2")][..],
            &[("preset", "springy")],
            &[("duration", "1"), ("damping", "4")],
            &[("duration", "0")],
            &[("bounce", "1")],
            &[("mass", "0")],
            &[("damping", "-1")],
            &[("stiffness", "x")],
        ] {
            assert!(spring(bad).is_err(), "{bad:?}");
        }
    }
}
