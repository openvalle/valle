//! Deterministic GeometryBatch evaluation and fixed-step particle trajectories.

use crate::compute::noise::{SeededRandom, value_noise_2d};
use valle_draw::Point;
use valle_draw::program::recording::BatchInstance;
use valle_draw::program::{AuthorColor, GradientInterpolation, interpolate_author_colors};

use crate::{BatchPositions, BatchStagger, GeometryBatchSpec, ParticleForce, ParticleSpec};

const PARTICLE_BAKE_HZ: f64 = 120.0;
const MAX_PARTICLE_BAKE_SAMPLES: usize = 4_000_000;

/// One immutable trajectory table per authored particle batch. Rows are laid out contiguously,
/// so arbitrary frame seeks only read and interpolate two neighboring positions.
#[derive(Debug, Clone)]
pub(crate) struct BakedParticleTrajectories {
    steps: usize,
    lifetime: f64,
    positions: Vec<Point>,
}

impl BakedParticleTrajectories {
    fn at(&self, index: usize, age: f64) -> Point {
        let row = index * (self.steps + 1);
        let step = ((age * PARTICLE_BAKE_HZ).floor() as usize).min(self.steps);
        if step == self.steps {
            return self.positions[row + step];
        }
        let start = step as f64 / PARTICLE_BAKE_HZ;
        let end = ((step + 1) as f64 / PARTICLE_BAKE_HZ).min(self.lifetime);
        if end <= start {
            return self.positions[row + step];
        }
        let progress = ((age - start) / (end - start)).clamp(0.0, 1.0);
        let left = self.positions[row + step];
        let right = self.positions[row + step + 1];
        Point::new(
            lerp(left.x, right.x, progress),
            lerp(left.y, right.y, progress),
        )
    }
}

pub(crate) fn particle_bake_samples(spec: &ParticleSpec) -> Option<usize> {
    if spec.count == 0 {
        return None;
    }
    let steps = (spec.lifetime * PARTICLE_BAKE_HZ).ceil();
    if !steps.is_finite() || steps < 1.0 || steps > usize::MAX as f64 {
        return None;
    }
    (steps as usize)
        .checked_add(1)?
        .checked_mul(spec.count as usize)
        .filter(|samples| *samples <= MAX_PARTICLE_BAKE_SAMPLES)
}

fn particle_initial(spec: &ParticleSpec, index: usize) -> (Point, Point) {
    let mut random =
        SeededRandom::new(spec.seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let x0 = spec.emitter.x + random.next_f64() * spec.emitter.width;
    let y0 = spec.emitter.y + random.next_f64() * spec.emitter.height;
    let vx = random.next_range(spec.velocity_x[0], spec.velocity_x[1]);
    let vy = random.next_range(spec.velocity_y[0], spec.velocity_y[1]);
    (Point::new(x0, y0), Point::new(vx, vy))
}

fn curl_acceleration(seed: u64, scale: f64, strength: f64, position: Point) -> Point {
    let x = position.x * scale;
    let y = position.y * scale;
    let h = 0.25;
    let dx = (value_noise_2d(seed, x + h, y) - value_noise_2d(seed, x - h, y)) / (2.0 * h);
    let dy = (value_noise_2d(seed, x, y + h) - value_noise_2d(seed, x, y - h)) / (2.0 * h);
    Point::new(dy * strength, -dx * strength)
}

pub(crate) fn bake_particle_trajectories(
    spec: &ParticleSpec,
) -> Result<Option<BakedParticleTrajectories>, &'static str> {
    if spec.forces.is_empty() {
        return Ok(None);
    }
    let samples = particle_bake_samples(spec).ok_or("particle trajectory exceeds sample budget")?;
    let steps = samples / spec.count as usize - 1;
    let mut positions = Vec::with_capacity(samples);
    for index in 0..spec.count as usize {
        let (mut position, mut velocity) = particle_initial(spec, index);
        positions.push(position);
        for step in 0..steps {
            let time = step as f64 / PARTICLE_BAKE_HZ;
            let dt = (spec.lifetime - time).min(1.0 / PARTICLE_BAKE_HZ);
            if dt <= 0.0 {
                positions.push(position);
                continue;
            }
            let mut acceleration = spec.gravity;
            for force in &spec.forces {
                match *force {
                    ParticleForce::CurlNoise {
                        seed,
                        scale,
                        strength,
                    } if strength != 0.0 => {
                        let curl = curl_acceleration(seed, scale, strength, position);
                        acceleration.x += curl.x;
                        acceleration.y += curl.y;
                    }
                    ParticleForce::Drag { coefficient } if coefficient != 0.0 => {
                        acceleration.x -= coefficient * velocity.x;
                        acceleration.y -= coefficient * velocity.y;
                    }
                    _ => {}
                }
            }
            velocity.x += acceleration.x * dt;
            velocity.y += acceleration.y * dt;
            position.x += velocity.x * dt;
            position.y += velocity.y * dt;
            if !position.x.is_finite()
                || !position.y.is_finite()
                || !velocity.x.is_finite()
                || !velocity.y.is_finite()
            {
                return Err("particle trajectory contains non-finite state");
            }
            positions.push(position);
        }
    }
    Ok(Some(BakedParticleTrajectories {
        steps,
        lifetime: spec.lifetime,
        positions,
    }))
}

/// Frame-progress inputs for the compact GeometryBatch field programs. Values are clamped by the
/// resolver; callers provide zero for a field that is absent from the batch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BatchFieldProgress {
    pub position: f64,
    pub size: f64,
    pub fill: f64,
    pub opacity: f64,
    pub rotation: f64,
    pub skew_x: f64,
    pub stroke_width: f64,
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn point_at(values: &[Point], index: usize, progress: f64) -> Point {
    match values {
        [one] => *one,
        [from, to] => Point::new(lerp(from.x, to.x, progress), lerp(from.y, to.y, progress)),
        many => many[index],
    }
}

fn color_at(values: &[AuthorColor], index: usize, progress: f64) -> AuthorColor {
    match values {
        [one] => *one,
        [from, to] => interpolate_author_colors(*from, *to, progress, GradientInterpolation::Srgb),
        many => many[index],
    }
}

fn opacity_at(values: &[f64], index: usize, progress: f64) -> f64 {
    match values {
        [] => 1.0,
        [one] => *one,
        [from, to] => lerp(*from, *to, progress),
        many => many[index],
    }
}

fn rotation_at(values: &[f64], index: usize, progress: f64) -> f64 {
    match values {
        [] => 0.0,
        [one] => *one,
        [from, to] => lerp(*from, *to, progress),
        many => many[index],
    }
}

fn stroke_width_at(values: &[f64], index: usize, progress: f64) -> f64 {
    match values {
        [] => 0.0,
        [one] => *one,
        [from, to] => lerp(*from, *to, progress),
        many => many[index],
    }
}

fn staggered_progress(progress: f64, stagger: &BatchStagger, index: usize) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    let delay = match stagger {
        BatchStagger::Step(step) => step * index as f64,
        BatchStagger::PerInstance(delays) => delays[index],
    };
    if delay <= 0.0 {
        progress
    } else {
        ((progress - delay) / (1.0 - delay)).clamp(0.0, 1.0)
    }
}

fn point_field_at(
    from: &[Point],
    to: &[Point],
    index: usize,
    progress: f64,
    stagger: &BatchStagger,
) -> Point {
    let from = if from.len() == 1 {
        from[0]
    } else {
        from[index]
    };
    let to = if to.len() == 1 { to[0] } else { to[index] };
    let progress = staggered_progress(progress, stagger, index);
    Point::new(lerp(from.x, to.x, progress), lerp(from.y, to.y, progress))
}

fn color_field_at(
    from: &[AuthorColor],
    to: &[AuthorColor],
    index: usize,
    progress: f64,
    stagger: &BatchStagger,
) -> AuthorColor {
    let from = if from.len() == 1 {
        from[0]
    } else {
        from[index]
    };
    let to = if to.len() == 1 { to[0] } else { to[index] };
    let progress = staggered_progress(progress, stagger, index);
    interpolate_author_colors(from, to, progress, GradientInterpolation::Srgb)
}

fn number_field_at(
    from: &[f64],
    to: &[f64],
    index: usize,
    progress: f64,
    stagger: &BatchStagger,
    default: f64,
) -> f64 {
    let from = if from.is_empty() {
        default
    } else if from.len() == 1 {
        from[0]
    } else {
        from[index]
    };
    let to = if to.len() == 1 { to[0] } else { to[index] };
    lerp(from, to, staggered_progress(progress, stagger, index))
}

/// Evaluate directly at `frame / fps`; standalone callers bake force trajectories on demand.
pub fn resolve_geometry_batch(
    batch: &GeometryBatchSpec,
    frame: f64,
    fps: f64,
    field_progress: BatchFieldProgress,
) -> Vec<BatchInstance> {
    let baked = match &batch.positions {
        BatchPositions::Particles { spec, .. } => match bake_particle_trajectories(spec) {
            Ok(baked) => baked,
            Err(_) => return Vec::new(),
        },
        BatchPositions::Static { .. } => None,
    };
    resolve_geometry_batch_with_trajectories(batch, baked.as_ref(), frame, fps, field_progress)
}

/// Frame path: immutable trajectories were built once during scene preparation.
pub(crate) fn resolve_geometry_batch_with_trajectories(
    batch: &GeometryBatchSpec,
    baked: Option<&BakedParticleTrajectories>,
    frame: f64,
    fps: f64,
    field_progress: BatchFieldProgress,
) -> Vec<BatchInstance> {
    resolve_geometry_batch_with_identity(batch, baked, frame, fps, field_progress, &mut Vec::new())
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BatchRowIdentity {
    pub index: usize,
    pub cycle: i64,
}

pub(crate) fn resolve_geometry_batch_with_identity(
    batch: &GeometryBatchSpec,
    baked: Option<&BakedParticleTrajectories>,
    frame: f64,
    fps: f64,
    field_progress: BatchFieldProgress,
    identity: &mut Vec<BatchRowIdentity>,
) -> Vec<BatchInstance> {
    match &batch.positions {
        BatchPositions::Static { values } => values
            .iter()
            .enumerate()
            .map(|(index, position)| {
                (
                    index,
                    BatchInstance {
                        position: batch.position_field.as_ref().map_or(*position, |field| {
                            point_field_at(
                                values,
                                &field.to,
                                index,
                                field_progress.position,
                                &field.stagger,
                            )
                        }),
                        size: batch.size_field.as_ref().map_or_else(
                            || {
                                if batch.sizes.len() == 1 {
                                    batch.sizes[0]
                                } else {
                                    batch.sizes[index]
                                }
                            },
                            |field| {
                                point_field_at(
                                    &batch.sizes,
                                    &field.to,
                                    index,
                                    field_progress.size,
                                    &field.stagger,
                                )
                            },
                        ),
                        color: batch
                            .fill_field
                            .as_ref()
                            .map_or_else(
                                || {
                                    if batch.fills.len() == 1 {
                                        batch.fills[0]
                                    } else {
                                        batch.fills[index]
                                    }
                                },
                                |field| {
                                    color_field_at(
                                        &batch.fills,
                                        &field.to,
                                        index,
                                        field_progress.fill,
                                        &field.stagger,
                                    )
                                },
                            )
                            .with_opacity(if let Some(field) = &batch.opacity_field {
                                number_field_at(
                                    &batch.opacities,
                                    &field.to,
                                    index,
                                    field_progress.opacity,
                                    &field.stagger,
                                    1.0,
                                )
                            } else if batch.opacities.is_empty() || batch.opacities.len() == 1 {
                                batch.opacities.first().copied().unwrap_or(1.0)
                            } else {
                                batch.opacities[index]
                            })
                            .to_working(),
                        rotation: (if let Some(field) = &batch.rotation_field {
                            number_field_at(
                                &batch.rotations,
                                &field.to,
                                index,
                                field_progress.rotation,
                                &field.stagger,
                                0.0,
                            )
                        } else if batch.rotations.is_empty() || batch.rotations.len() == 1 {
                            batch.rotations.first().copied().unwrap_or(0.0)
                        } else {
                            batch.rotations[index]
                        }) as f32,
                        skew_x: (if let Some(field) = &batch.skew_x_field {
                            number_field_at(
                                &batch.skew_xs,
                                &field.to,
                                index,
                                field_progress.skew_x,
                                &field.stagger,
                                0.0,
                            )
                        } else if batch.skew_xs.is_empty() || batch.skew_xs.len() == 1 {
                            batch.skew_xs.first().copied().unwrap_or(0.0)
                        } else {
                            batch.skew_xs[index]
                        }) as f32,
                        stroke_width: (if let Some(field) = &batch.stroke_width_field {
                            number_field_at(
                                &batch.stroke_widths,
                                &field.to,
                                index,
                                field_progress.stroke_width,
                                &field.stagger,
                                0.0,
                            )
                        } else if batch.stroke_widths.is_empty() || batch.stroke_widths.len() == 1 {
                            batch.stroke_widths.first().copied().unwrap_or(0.0)
                        } else {
                            batch.stroke_widths[index]
                        }) as f32,
                        opacity: 1.0,
                    },
                )
            })
            .filter_map(|(index, instance)| {
                if instance.size.x == 0.0 || instance.size.y == 0.0 {
                    return None;
                }
                identity.push(BatchRowIdentity { index, cycle: 0 });
                Some(instance)
            })
            .collect(),
        BatchPositions::Particles { spec, .. } => {
            let seconds = frame / fps;
            let cycle = spec.count as f64 * spec.birth_interval;
            (0..spec.count as usize)
                .filter_map(|index| {
                    let birth = index as f64 * spec.birth_interval;
                    let age = if spec.looping {
                        (seconds - birth).rem_euclid(cycle)
                    } else {
                        seconds - birth
                    };
                    if !(0.0..spec.lifetime).contains(&age) {
                        return None;
                    }
                    let progress = (age / spec.lifetime).clamp(0.0, 1.0);
                    let position = if let Some(baked) = baked {
                        baked.at(index, age)
                    } else {
                        let (origin, velocity) = particle_initial(spec, index);
                        Point::new(
                            origin.x + velocity.x * age + 0.5 * spec.gravity.x * age * age,
                            origin.y + velocity.y * age + 0.5 * spec.gravity.y * age * age,
                        )
                    };
                    let size = point_at(&batch.sizes, index, progress);
                    if size.x == 0.0 || size.y == 0.0 {
                        return None;
                    }
                    identity.push(BatchRowIdentity {
                        index,
                        cycle: if spec.looping {
                            ((seconds - birth) / cycle).floor() as i64
                        } else {
                            0
                        },
                    });
                    Some(BatchInstance {
                        position,
                        size,
                        color: color_at(&batch.fills, index, progress)
                            .with_opacity(opacity_at(&batch.opacities, index, progress))
                            .to_working(),
                        rotation: rotation_at(&batch.rotations, index, progress) as f32,
                        skew_x: rotation_at(&batch.skew_xs, index, progress) as f32,
                        stroke_width: stroke_width_at(&batch.stroke_widths, index, progress) as f32,
                        opacity: 1.0,
                    })
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GeometryBatchGeometry, ParticleForce, ParticleSpec};
    use valle_draw::{Rect, Rgba};

    fn fixture() -> GeometryBatchSpec {
        GeometryBatchSpec {
            geometry: GeometryBatchGeometry::Circle,
            positions: BatchPositions::Particles {
                frame: crate::ExprId(0),
                spec: ParticleSpec {
                    seed: 42,
                    count: 4,
                    emitter: Rect::new(10.0, 20.0, 5.0, 5.0),
                    birth_interval: 0.1,
                    lifetime: 1.0,
                    velocity_x: [-2.0, 2.0],
                    velocity_y: [-4.0, -1.0],
                    gravity: Point::new(0.0, 10.0),
                    looping: false,
                    forces: Vec::new(),
                },
            },
            sizes: vec![Point::new(2.0, 2.0), Point::new(6.0, 6.0)],
            fills: vec![AuthorColor::from_srgb8(Rgba::rgb(0, 255, 255))],
            opacities: vec![1.0, 0.0],
            rotations: vec![],
            skew_xs: vec![],
            stroke_widths: vec![],
            semantic_keys: vec![],
            position_field: None,
            size_field: None,
            fill_field: None,
            opacity_field: None,
            rotation_field: None,
            skew_x_field: None,
            stroke_width_field: None,
        }
    }

    #[test]
    fn direct_seek_is_deterministic() {
        let batch = fixture();
        let later = resolve_geometry_batch(&batch, 30.0, 60.0, BatchFieldProgress::default());
        let _earlier = resolve_geometry_batch(&batch, 5.0, 60.0, BatchFieldProgress::default());
        assert_eq!(
            later,
            resolve_geometry_batch(&batch, 30.0, 60.0, BatchFieldProgress::default())
        );
    }

    #[test]
    fn different_seeds_produce_different_instances() {
        let first = fixture();
        let mut second = fixture();
        let BatchPositions::Particles { spec, .. } = &mut second.positions else {
            unreachable!()
        };
        spec.seed += 1;

        assert_ne!(
            resolve_geometry_batch(&first, 30.0, 60.0, BatchFieldProgress::default()),
            resolve_geometry_batch(&second, 30.0, 60.0, BatchFieldProgress::default())
        );
    }

    #[test]
    fn baked_forces_are_random_access_and_match_on_demand_resolution() {
        let mut batch = fixture();
        let BatchPositions::Particles { spec, .. } = &mut batch.positions else {
            unreachable!();
        };
        spec.forces = vec![
            ParticleForce::CurlNoise {
                seed: 3,
                scale: 0.012,
                strength: 220.0,
            },
            ParticleForce::Drag { coefficient: 0.8 },
        ];
        let baked = bake_particle_trajectories(spec).unwrap().unwrap();
        let at_45 = resolve_geometry_batch_with_trajectories(
            &batch,
            Some(&baked),
            45.0,
            30.0,
            BatchFieldProgress::default(),
        );
        let _at_3 = resolve_geometry_batch_with_trajectories(
            &batch,
            Some(&baked),
            3.0,
            30.0,
            BatchFieldProgress::default(),
        );
        assert_eq!(
            at_45,
            resolve_geometry_batch_with_trajectories(
                &batch,
                Some(&baked),
                45.0,
                30.0,
                BatchFieldProgress::default(),
            )
        );
        assert_eq!(
            at_45,
            resolve_geometry_batch(&batch, 45.0, 30.0, BatchFieldProgress::default())
        );
    }

    #[test]
    fn birth_is_inclusive_and_lifetime_end_is_exclusive() {
        let batch = fixture();

        assert_eq!(
            resolve_geometry_batch(&batch, 0.0, 60.0, BatchFieldProgress::default()).len(),
            1
        );
        assert_eq!(
            resolve_geometry_batch(&batch, 6.0, 60.0, BatchFieldProgress::default()).len(),
            2
        );
        assert_eq!(
            resolve_geometry_batch(&batch, 60.0, 60.0, BatchFieldProgress::default()).len(),
            3
        );
    }

    #[test]
    fn fifty_thousand_fixed_instances_resolve_in_one_bounded_side_table() {
        let count = 50_000usize;
        let from = (0..count)
            .map(|index| Point::new((index % 500) as f64, (index / 500) as f64))
            .collect::<Vec<_>>();
        let to = from
            .iter()
            .map(|point| Point::new(point.x + 100.0, point.y + 50.0))
            .collect::<Vec<_>>();
        let batch = GeometryBatchSpec {
            geometry: GeometryBatchGeometry::Rect,
            positions: BatchPositions::Static {
                values: from.clone(),
            },
            sizes: vec![Point::new(2.0, 2.0)],
            fills: vec![AuthorColor::from_srgb8(Rgba::rgb(34, 211, 238))],
            opacities: vec![0.5],
            rotations: vec![],
            skew_xs: vec![],
            stroke_widths: vec![],
            semantic_keys: vec![],
            position_field: Some(crate::BatchPointField {
                to,
                progress: crate::ExprId(0),
                stagger: BatchStagger::default(),
            }),
            size_field: Some(crate::BatchPointField {
                to: vec![Point::new(4.0, 4.0)],
                progress: crate::ExprId(0),
                stagger: BatchStagger::default(),
            }),
            fill_field: None,
            opacity_field: None,
            rotation_field: None,
            skew_x_field: None,
            stroke_width_field: None,
        };
        let progress = BatchFieldProgress {
            position: 0.5,
            size: 0.5,
            ..BatchFieldProgress::default()
        };
        let first = resolve_geometry_batch(&batch, 0.0, 30.0, progress);
        let second = resolve_geometry_batch(&batch, 999.0, 30.0, progress);
        assert_eq!(
            first, second,
            "fixed fields do not depend on render history"
        );
        assert_eq!(first.len(), count);
        assert_eq!(first[0].position, Point::new(50.0, 25.0));
        assert_eq!(first[count - 1].size, Point::new(3.0, 3.0));
        assert!(
            std::mem::size_of_val(first.as_slice()) <= 3_200_000,
            "50k resolved instances must stay a compact contiguous side table"
        );
    }
}
