//! Deterministic, closed-form GeometryBatch evaluation.

use crate::compute::noise::SeededRandom;
use valle_draw::program::recording::BatchInstance;
use valle_draw::{Point, Rgba};

use crate::{BatchPositions, GeometryBatchSpec};

/// Frame-progress inputs for the compact GeometryBatch field programs. Values are clamped by the
/// resolver; callers provide zero for a field that is absent from the batch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BatchFieldProgress {
    pub position: f64,
    pub size: f64,
    pub fill: f64,
    pub opacity: f64,
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

fn color_at(values: &[Rgba], index: usize, progress: f64) -> Rgba {
    match values {
        [one] => *one,
        [from, to] => {
            let alpha = lerp(from.a as f64, to.a as f64, progress).round() as u8;
            Rgba {
                a: alpha,
                ..from.mix(*to, progress)
            }
        }
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

fn staggered_progress(progress: f64, stagger: f64, index: usize) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    let delay = stagger * index as f64;
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
    stagger: f64,
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

fn color_field_at(from: &[Rgba], to: &[Rgba], index: usize, progress: f64, stagger: f64) -> Rgba {
    let from = if from.len() == 1 {
        from[0]
    } else {
        from[index]
    };
    let to = if to.len() == 1 { to[0] } else { to[index] };
    let progress = staggered_progress(progress, stagger, index);
    let alpha = lerp(from.a as f64, to.a as f64, progress).round() as u8;
    Rgba {
        a: alpha,
        ..from.mix(to, progress)
    }
}

fn number_field_at(from: &[f64], to: &[f64], index: usize, progress: f64, stagger: f64) -> f64 {
    let from = if from.is_empty() {
        1.0
    } else if from.len() == 1 {
        from[0]
    } else {
        from[index]
    };
    let to = if to.len() == 1 { to[0] } else { to[index] };
    lerp(from, to, staggered_progress(progress, stagger, index))
}

/// Evaluate directly at `frame / fps`; no prior frame or mutable simulation state is observed.
pub fn resolve_geometry_batch(
    batch: &GeometryBatchSpec,
    frame: f64,
    fps: f64,
    field_progress: BatchFieldProgress,
) -> Vec<BatchInstance> {
    match &batch.positions {
        BatchPositions::Static { values } => values
            .iter()
            .enumerate()
            .map(|(index, position)| BatchInstance {
                position: batch.position_field.as_ref().map_or(*position, |field| {
                    point_field_at(
                        values,
                        &field.to,
                        index,
                        field_progress.position,
                        field.stagger,
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
                            field.stagger,
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
                                field.stagger,
                            )
                        },
                    )
                    .with_opacity(if let Some(field) = &batch.opacity_field {
                        number_field_at(
                            &batch.opacities,
                            &field.to,
                            index,
                            field_progress.opacity,
                            field.stagger,
                        )
                    } else if batch.opacities.is_empty() || batch.opacities.len() == 1 {
                        batch.opacities.first().copied().unwrap_or(1.0)
                    } else {
                        batch.opacities[index]
                    }),
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
                    let mut random = SeededRandom::new(
                        spec.seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                    );
                    let x0 = spec.emitter.x + random.next_f64() * spec.emitter.width;
                    let y0 = spec.emitter.y + random.next_f64() * spec.emitter.height;
                    let vx = random.next_range(spec.velocity_x[0], spec.velocity_x[1]);
                    let vy = random.next_range(spec.velocity_y[0], spec.velocity_y[1]);
                    let position = Point::new(
                        x0 + vx * age + 0.5 * spec.gravity.x * age * age,
                        y0 + vy * age + 0.5 * spec.gravity.y * age * age,
                    );
                    Some(BatchInstance {
                        position,
                        size: point_at(&batch.sizes, index, progress),
                        color: color_at(&batch.fills, index, progress).with_opacity(opacity_at(
                            &batch.opacities,
                            index,
                            progress,
                        )),
                    })
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GeometryBatchGeometry, ParticleSpec};
    use valle_draw::Rect;

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
                },
            },
            sizes: vec![Point::new(2.0, 2.0), Point::new(6.0, 6.0)],
            fills: vec![Rgba::rgb(0, 255, 255)],
            opacities: vec![1.0, 0.0],
            semantic_keys: vec![],
            position_field: None,
            size_field: None,
            fill_field: None,
            opacity_field: None,
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
            fills: vec![Rgba::rgb(34, 211, 238)],
            opacities: vec![0.5],
            semantic_keys: vec![],
            position_field: Some(crate::BatchPointField {
                to,
                progress: crate::ExprId(0),
                stagger: 0.0,
            }),
            size_field: Some(crate::BatchPointField {
                to: vec![Point::new(4.0, 4.0)],
                progress: crate::ExprId(0),
                stagger: 0.0,
            }),
            fill_field: None,
            opacity_field: None,
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
            std::mem::size_of_val(first.as_slice()) <= 2_500_000,
            "50k resolved instances must stay a compact contiguous side table"
        );
    }
}
