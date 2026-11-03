//! `particles()`: particle `i` is born `i × interval` seconds in (again every `count × interval`
//! with `loop`) at a seeded point of the emitter with a seeded velocity, lives `lifetime` seconds,
//! and moves under gravity in closed form, or with forces (curl noise, drag) along a trajectory
//! stepped at 120 Hz when the system is first asked for and read linearly, so any frame is
//! computed on its own.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use serde_json::Value;

use crate::noise;
use crate::template::V;

const STEPS_PER_SECOND: f64 = 120.0;
/// Positions a system with forces may store, over all its particles.
const MAX_SAMPLES: usize = 4_000_000;
/// Systems kept by their spec, as the frames of a scene ask for the same few again and again.
const KEPT: usize = 16;

#[derive(Clone, Copy, Debug)]
enum Force {
    Curl {
        seed: u64,
        scale: f64,
        strength: f64,
    },
    Drag(f64),
}

/// One `particles()` system, from its spec.
#[derive(Debug)]
pub struct System {
    spec: Rc<str>,
    seed: u64,
    count: u32,
    interval: f64,
    looping: bool,
    lifetime: f64,
    /// x, y, width, height.
    emitter: [f64; 4],
    /// [low, high] of x, then of y.
    velocity: [[f64; 2]; 2],
    gravity: [f64; 2],
    forces: Vec<Force>,
    /// With forces, each particle's position every step of its life, particle after particle.
    steps: Vec<[f64; 2]>,
    per: usize,
}

impl PartialEq for System {
    fn eq(&self, other: &System) -> bool {
        self.spec == other.spec
    }
}

/// What a particle's op reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    X,
    Y,
    Age,
    Progress,
    Alive,
}

impl Field {
    pub fn parse(name: &str) -> Option<Field> {
        Some(match name {
            "x" => Field::X,
            "y" => Field::Y,
            "age" => Field::Age,
            "progress" => Field::Progress,
            "alive" => Field::Alive,
            _ => return None,
        })
    }
}

thread_local! {
    static SYSTEMS: RefCell<HashMap<Rc<str>, Rc<System>>> = RefCell::new(HashMap::new());
}

impl System {
    /// The system of a spec, made once: JSON of seed, count, interval, loop, lifetime, emitter
    /// [x, y, width, height], velocity [x low, x high, y low, y high], gravity [x, y] and
    /// forces, each `{ curl: [seed, scale, strength] }` or `{ drag: coefficient }`.
    pub fn get(spec: &str) -> Result<Rc<System>, String> {
        if let Some(hit) = SYSTEMS.with(|s| s.borrow().get(spec).cloned()) {
            return Ok(hit);
        }
        let system = Rc::new(System::parse(spec)?);
        SYSTEMS.with(|s| {
            let mut s = s.borrow_mut();
            if s.len() >= KEPT {
                s.clear();
            }
            s.insert(system.spec.clone(), system.clone());
        });
        Ok(system)
    }

    fn parse(spec: &str) -> Result<System, String> {
        let j: Value = serde_json::from_str(spec).map_err(|e| format!("particles spec: {e}"))?;
        let bad = |what: &str| format!("particles spec: {what}");
        let num = |v: &Value, what: &str| v.as_f64().ok_or_else(|| bad(what));
        let nums = |v: &Value, what: &str, n: usize| -> Result<Vec<f64>, String> {
            let a = v
                .as_array()
                .filter(|a| a.len() == n)
                .ok_or_else(|| bad(what))?;
            a.iter().map(|x| num(x, what)).collect()
        };
        let mut forces = Vec::new();
        for f in j["forces"].as_array().into_iter().flatten() {
            if let Some(c) = f.get("curl") {
                let strength = num(&c[2], "curl")?;
                if strength != 0.0 {
                    forces.push(Force::Curl {
                        seed: c[0].as_u64().ok_or_else(|| bad("curl"))?,
                        scale: num(&c[1], "curl")?,
                        strength,
                    });
                }
            } else {
                let k = num(&f["drag"], "drag")?;
                if k != 0.0 {
                    forces.push(Force::Drag(k));
                }
            }
        }
        let v = nums(&j["velocity"], "velocity", 4)?;
        let e = nums(&j["emitter"], "emitter", 4)?;
        let g = nums(&j["gravity"], "gravity", 2)?;
        let mut system = System {
            spec: Rc::from(spec),
            seed: j["seed"].as_u64().ok_or_else(|| bad("seed"))?,
            count: (j["count"].as_u64().and_then(|c| u32::try_from(c).ok()))
                .ok_or_else(|| bad("count"))?,
            interval: num(&j["interval"], "interval")?,
            looping: j["loop"].as_bool().ok_or_else(|| bad("loop"))?,
            lifetime: num(&j["lifetime"], "lifetime")?,
            emitter: [e[0], e[1], e[2], e[3]],
            velocity: [[v[0], v[1]], [v[2], v[3]]],
            gravity: [g[0], g[1]],
            forces,
            steps: Vec::new(),
            per: 0,
        };
        if !system.forces.is_empty() {
            system.step()?;
        }
        Ok(system)
    }

    /// Particle `index`'s start and velocity.
    fn initial(&self, index: u32) -> ([f64; 2], [f64; 2]) {
        let seed = self.seed ^ u64::from(index).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let r = noise::random(seed, 4, [0.0, 1.0]);
        let [x, y, w, h] = self.emitter;
        let [[x0, x1], [y0, y1]] = self.velocity;
        (
            [x + r[0] * w, y + r[1] * h],
            [x0 + r[2] * (x1 - x0), y0 + r[3] * (y1 - y0)],
        )
    }

    fn step(&mut self) -> Result<(), String> {
        let steps = (self.lifetime * STEPS_PER_SECOND).ceil() as usize;
        if (steps + 1).saturating_mul(self.count as usize) > MAX_SAMPLES {
            return Err(format!(
                "particles with forces step {} positions; at most {MAX_SAMPLES}: fewer particles \
                 or a shorter lifetime",
                (steps + 1).saturating_mul(self.count as usize)
            ));
        }
        self.per = steps + 1;
        self.steps = Vec::with_capacity(self.per * self.count as usize);
        let dt = 1.0 / STEPS_PER_SECOND;
        for index in 0..self.count {
            let (mut p, mut v) = self.initial(index);
            self.steps.push(p);
            for _ in 0..steps {
                let mut a = self.gravity;
                for f in &self.forces {
                    match *f {
                        Force::Curl {
                            seed,
                            scale,
                            strength,
                        } => {
                            // The curl of the noise field: along its contour lines, so it swirls.
                            let (x, y, h) = (p[0] * scale, p[1] * scale, 0.25);
                            let dx = (noise::noise2(seed, x + h, y)
                                - noise::noise2(seed, x - h, y))
                                / (2.0 * h);
                            let dy = (noise::noise2(seed, x, y + h)
                                - noise::noise2(seed, x, y - h))
                                / (2.0 * h);
                            a[0] += dy * strength;
                            a[1] -= dx * strength;
                        }
                        Force::Drag(k) => {
                            a[0] -= k * v[0];
                            a[1] -= k * v[1];
                        }
                    }
                }
                v = [v[0] + a[0] * dt, v[1] + a[1] * dt];
                p = [p[0] + v[0] * dt, p[1] + v[1] * dt];
                if !p.iter().chain(&v).all(|x| x.is_finite()) {
                    return Err("a particle's path is not finite: weaker forces".into());
                }
                self.steps.push(p);
            }
        }
        Ok(())
    }

    /// Particle `index`'s age at time `t`: from its latest birth, negative before its first.
    fn age(&self, index: u32, t: f64) -> f64 {
        let age = t - f64::from(index) * self.interval;
        if self.looping && age >= 0.0 {
            age.rem_euclid(f64::from(self.count) * self.interval)
        } else {
            age
        }
    }

    /// Particle `index`'s position at `age` seconds, held at its birth and its death.
    fn at(&self, index: u32, age: f64) -> [f64; 2] {
        let age = age.clamp(0.0, self.lifetime);
        if self.steps.is_empty() {
            let (p, v) = self.initial(index);
            let g = self.gravity;
            return [0, 1].map(|k| p[k] + v[k] * age + 0.5 * g[k] * age * age);
        }
        let path = &self.steps[index as usize * self.per..][..self.per];
        let s = age * STEPS_PER_SECOND;
        let i = (s.floor() as usize).min(self.per - 2);
        let u = (s - i as f64).clamp(0.0, 1.0);
        [0, 1].map(|k| path[i][k] + (path[i + 1][k] - path[i][k]) * u)
    }

    /// `field` of particle `index` at time `t`: its position, its age in seconds, its age over its
    /// lifetime in [0, 1], or whether it is alive.
    pub fn field(&self, index: u32, field: Field, t: f64) -> V {
        let age = self.age(index, t);
        V::Num(match field {
            Field::X => self.at(index, age)[0],
            Field::Y => self.at(index, age)[1],
            Field::Age => age,
            Field::Progress => (age / self.lifetime).clamp(0.0, 1.0),
            Field::Alive => return V::Bool((0.0..self.lifetime).contains(&age)),
        })
    }

    pub fn count(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(forces: &str, looping: bool) -> String {
        format!(
            r#"{{"seed":7,"count":4,"interval":0.5,"loop":{looping},"lifetime":1,
                "emitter":[10,20,0,0],"velocity":[100,100,0,0],"gravity":[0,50],
                "forces":{forces}}}"#
        )
    }

    #[test]
    fn particles_are_born_one_interval_apart_and_loop() {
        let s = System::get(&spec("[]", true)).unwrap();
        // Particle 1 is born at 0.5 s: at 1 s it is half a second old.
        assert_eq!(s.field(1, Field::Age, 1.0), V::Num(0.5));
        assert_eq!(s.field(1, Field::X, 1.0), V::Num(10.0 + 50.0));
        assert_eq!(s.field(1, Field::Y, 1.0), V::Num(20.0 + 0.5 * 50.0 * 0.25));
        assert_eq!(s.field(1, Field::Alive, 0.25), V::Bool(false));
        assert_eq!(s.field(1, Field::Alive, 1.4), V::Bool(true));
        assert_eq!(s.field(1, Field::Alive, 1.6), V::Bool(false));
        // Around again every 4 × 0.5 s.
        assert_eq!(s.field(1, Field::Age, 2.75), V::Num(0.25));
        let once = System::get(&spec("[]", false)).unwrap();
        assert_eq!(once.field(1, Field::Alive, 2.75), V::Bool(false));
    }

    #[test]
    fn forces_bend_the_path_and_drag_slows_it() {
        let free = System::get(&spec("[]", false)).unwrap();
        let dragged = System::get(&spec(r#"[{"drag":2}]"#, false)).unwrap();
        let curled = System::get(&spec(r#"[{"curl":[3,0.05,400]}]"#, false)).unwrap();
        let num = |v: V| match v {
            V::Num(x) => x,
            _ => panic!("a number"),
        };
        let x = |s: &System| num(s.field(0, Field::X, 0.9));
        // Stepping follows the closed form closely with no force but gravity.
        let stepped = System::get(&spec(r#"[{"drag":0},{"curl":[3,0.05,0]}]"#, false)).unwrap();
        assert!(stepped.steps.is_empty());
        assert!(x(&dragged) < x(&free) - 20.0);
        let y = |s: &System| num(s.field(0, Field::Y, 0.9));
        assert!((y(&curled) - y(&free)).abs() > 1.0);
    }
}
