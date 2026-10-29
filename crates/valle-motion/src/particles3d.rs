//! Deterministic particle motion in world units, independent of cameras and renderers.
//! Every sample is a pure function of immutable initial conditions and explicit seconds.

use crate::compute::noise::SeededRandom;
use serde::{Deserialize, Serialize};

pub const MAX_PARTICLES: u32 = 16_384;
const MAX_COORDINATE: f32 = 10_000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParticleSystem3D {
    pub seed: u64,
    pub count: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub velocity: [[f32; 2]; 3],
    pub gravity: [f32; 3],
    pub birth_interval: f64,
    pub lifetime: f64,
    pub drag: f64,
    pub looping: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle3D {
    pub id: u32,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub progress: f32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct ParticleError(pub &'static str);

#[derive(Clone, Debug)]
pub struct PreparedParticles3D {
    spec: ParticleSystem3D,
    initial: Vec<([f32; 3], [f32; 3])>,
}

impl ParticleSystem3D {
    pub fn validate(&self) -> Result<(), ParticleError> {
        if self.count == 0 || self.count > MAX_PARTICLES || self.seed > 9_007_199_254_740_991 {
            return Err(ParticleError(
                "particle count must be 1..=16384 and seed a safe integer",
            ));
        }
        if !self.birth_interval.is_finite()
            || !(0.0..=1.0e6).contains(&self.birth_interval)
            || !self.lifetime.is_finite()
            || !(1.0e-6..=1.0e6).contains(&self.lifetime)
            || !self.drag.is_finite()
            || !(0.0..=1.0e4).contains(&self.drag)
        {
            return Err(ParticleError(
                "particle birth interval, lifetime or drag is outside its finite range",
            ));
        }
        for axis in 0..3 {
            let values = [
                self.min[axis],
                self.max[axis],
                self.velocity[axis][0],
                self.velocity[axis][1],
                self.gravity[axis],
            ];
            if values
                .iter()
                .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
                || self.min[axis] > self.max[axis]
                || self.velocity[axis][0] > self.velocity[axis][1]
            {
                return Err(ParticleError(
                    "particle bounds, velocity ranges and gravity must be ordered and within +/-10000",
                ));
            }
            // Bound the whole lifetime before allocating or drawing. Drag can only reduce
            // each independent initial-velocity/constant-acceleration contribution.
            let (distance, acceleration) = displacement_terms(self.drag, self.lifetime);
            let bound = f64::from(self.min[axis].abs().max(self.max[axis].abs()))
                + f64::from(
                    self.velocity[axis][0]
                        .abs()
                        .max(self.velocity[axis][1].abs()),
                ) * distance
                + f64::from(self.gravity[axis].abs()) * acceleration;
            if !bound.is_finite() || bound > f64::from(MAX_COORDINATE) {
                return Err(ParticleError(
                    "particle trajectory exceeds +/-10000 world units over its lifetime",
                ));
            }
        }
        Ok(())
    }

    pub fn prepare(&self) -> Result<PreparedParticles3D, ParticleError> {
        self.validate()?;
        let initial = (0..self.count)
            .map(|id| {
                let mut rng = SeededRandom::new(
                    self.seed ^ u64::from(id).wrapping_mul(0x9e37_79b9_7f4a_7c15),
                );
                let position = std::array::from_fn(|axis| {
                    rng.next_range(f64::from(self.min[axis]), f64::from(self.max[axis])) as f32
                });
                let velocity = std::array::from_fn(|axis| {
                    rng.next_range(
                        f64::from(self.velocity[axis][0]),
                        f64::from(self.velocity[axis][1]),
                    ) as f32
                });
                (position, velocity)
            })
            .collect();
        Ok(PreparedParticles3D {
            spec: self.clone(),
            initial,
        })
    }
}

// Exact solution of dv/dt = gravity - drag * velocity. A short series avoids
// cancellation for tiny drag and converges continuously to ballistic motion.
fn displacement_terms(drag: f64, age: f64) -> (f64, f64) {
    let x = drag * age;
    if x < 1.0e-4 {
        let velocity = age * (1.0 - x / 2.0 + x * x / 6.0 - x * x * x / 24.0);
        let acceleration = age * age * (0.5 - x / 6.0 + x * x / 24.0 - x * x * x / 120.0);
        (velocity, acceleration)
    } else {
        let velocity = -libm::expm1(-x) / drag;
        (velocity, (age - velocity) / drag)
    }
}

impl PreparedParticles3D {
    pub fn count(&self) -> u32 {
        self.spec.count
    }
    pub fn storage_bytes(&self) -> u64 {
        self.initial.len() as u64 * 24
    }

    /// Writes live particles in stable ID order, reusing caller-owned storage.
    /// Negative time is an empty pre-roll; the lifetime interval is half-open.
    pub fn sample_into(
        &self,
        time: f64,
        output: &mut Vec<Particle3D>,
    ) -> Result<(), ParticleError> {
        if !time.is_finite() || time.abs() > 1.0e9 {
            return Err(ParticleError(
                "particle time must be finite within +/-1e9 seconds",
            ));
        }
        output.clear();
        if time < 0.0 {
            return Ok(());
        }
        let spec = &self.spec;
        let period = (f64::from(spec.count) * spec.birth_interval).max(spec.lifetime);
        for (id, (origin, initial_velocity)) in self.initial.iter().enumerate() {
            let since_birth = time - id as f64 * spec.birth_interval;
            if since_birth < 0.0 {
                continue;
            }
            let age = if spec.looping {
                since_birth.rem_euclid(period)
            } else {
                since_birth
            };
            if age >= spec.lifetime {
                continue;
            }
            let (distance, acceleration) = displacement_terms(spec.drag, age);
            let decay = libm::exp(-spec.drag * age);
            output.push(Particle3D {
                id: id as u32,
                position: std::array::from_fn(|axis| {
                    (f64::from(origin[axis])
                        + f64::from(initial_velocity[axis]) * distance
                        + f64::from(spec.gravity[axis]) * acceleration) as f32
                }),
                velocity: std::array::from_fn(|axis| {
                    (f64::from(initial_velocity[axis]) * decay
                        + f64::from(spec.gravity[axis]) * distance) as f32
                }),
                progress: (age / spec.lifetime) as f32,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> ParticleSystem3D {
        ParticleSystem3D {
            seed: 17,
            count: 8,
            min: [0.0; 3],
            max: [0.0; 3],
            velocity: [[2.0, 2.0]; 3],
            gravity: [0.0, -4.0, 0.0],
            birth_interval: 0.1,
            lifetime: 2.0,
            drag: 0.0,
            looping: false,
        }
    }
    #[test]
    fn ballistic_motion_and_random_seeks_are_exact() {
        let prepared = spec().prepare().unwrap();
        let mut out = Vec::new();
        prepared.sample_into(0.5, &mut out).unwrap();
        assert_eq!(out.len(), 6);
        assert_eq!(out[0].position, [1.0, 0.5, 1.0]);
        assert_eq!(out[0].velocity, [2.0, 0.0, 2.0]);
        let expected = out.clone();
        for t in [1.9, -1.0, 0.0, 20.0, 0.5] {
            prepared.sample_into(t, &mut out).unwrap();
        }
        assert_eq!(out, expected);
    }
    #[test]
    fn drag_matches_analytic_solution_and_zero_limit() {
        for drag in [0.0, 1.0e-12, 0.5, 1000.0] {
            let mut s = spec();
            s.drag = drag;
            let mut out = Vec::new();
            s.prepare().unwrap().sample_into(1.0, &mut out).unwrap();
            let expected = if drag < 1.0e-10 {
                2.0
            } else {
                2.0 * (1.0 - libm::exp(-drag)) / drag
            };
            assert!((f64::from(out[0].position[0]) - expected).abs() < 1.0e-6);
        }
    }
    #[test]
    fn burst_loop_endpoints_and_seed_are_stable() {
        let mut s = spec();
        s.birth_interval = 0.0;
        s.looping = true;
        s.max = [1.0; 3];
        let p = s.prepare().unwrap();
        let mut a = Vec::new();
        let mut b = Vec::new();
        p.sample_into(0.0, &mut a).unwrap();
        p.sample_into(2.0, &mut b).unwrap();
        assert_eq!(a.len(), 8);
        assert_eq!(a, b);
        s.looping = false;
        s.prepare().unwrap().sample_into(2.0, &mut b).unwrap();
        assert!(b.is_empty());
        s.seed += 1;
        s.prepare().unwrap().sample_into(0.0, &mut b).unwrap();
        assert_ne!(a, b);
    }
    #[test]
    fn invalid_state_fails_before_allocation() {
        let mut s = spec();
        s.count = u32::MAX;
        assert!(s.prepare().is_err());
        s = spec();
        s.lifetime = 1.0e6;
        assert!(s.prepare().is_err());
        s = spec();
        s.drag = f64::NAN;
        assert!(s.prepare().is_err());
        s = spec();
        s.velocity[2] = [2.0, 1.0];
        assert!(s.prepare().is_err());
        assert!(
            spec()
                .prepare()
                .unwrap()
                .sample_into(f64::NAN, &mut Vec::new())
                .is_err()
        );
    }
}
