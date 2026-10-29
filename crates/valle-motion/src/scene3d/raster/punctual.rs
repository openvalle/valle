//! Deterministic inverse-square punctual illumination shared with the GPU equations.
use super::V3;

pub(super) struct Punctual {
    pub position: V3,
    pub color: [f32; 3],
    pub range: f32,
    /// Emission direction, outer cosine, inner cosine. None emits in all directions.
    pub cone: Option<(V3, f32, f32)>,
}
impl Punctual {
    pub fn sample(&self, position: V3) -> (V3, [f32; 3]) {
        let delta = self.position - position;
        let squared = delta.dot(delta);
        let direction = delta.normalized();
        // Bound the ideal point-source singularity within a radius of 0.1 world units.
        let mut attenuation = 1.0 / squared.max(0.01);
        if self.range > 0.0 {
            let ratio = squared / (self.range * self.range);
            let fade = (1.0 - ratio * ratio).clamp(0.0, 1.0);
            attenuation *= fade * fade;
        }
        if let Some((axis, outer, inner)) = self.cone {
            let cosine = -axis.dot(direction);
            attenuation *= if inner == outer {
                f32::from(cosine >= outer)
            } else {
                let t = ((cosine - outer) / (inner - outer)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
        }
        (direction, self.color.map(|c| c * attenuation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn point_attenuation_is_inverse_square_and_range_fades_to_zero() {
        let mut light = Punctual {
            position: V3::new(0.0, 0.0, 2.0),
            color: [1.0; 3],
            range: 0.0,
            cone: None,
        };
        assert_eq!(light.sample(V3::new(0.0, 0.0, 0.0)).1, [0.25; 3]);
        assert_eq!(light.sample(V3::new(0.0, 0.0, -2.0)).1, [0.0625; 3]);
        light.range = 2.0;
        assert_eq!(light.sample(V3::new(0.0, 0.0, 0.0)).1, [0.0; 3]);
        let (direction, color) = light.sample(light.position);
        assert!(color.into_iter().all(f32::is_finite));
        assert!(direction.dot(direction).is_finite());
    }
    #[test]
    fn spot_cone_keeps_a_smooth_penumbra_and_never_lights_behind() {
        let mut light = Punctual {
            position: V3::new(0.0, 0.0, 2.0),
            color: [1.0; 3],
            range: 0.0,
            cone: Some((V3::new(0.0, 0.0, -1.0), 0.5, 0.9)),
        };
        assert_eq!(light.sample(V3::new(0.0, 0.0, 0.0)).1, [0.25; 3]);
        assert_eq!(light.sample(V3::new(0.0, 0.0, 3.0)).1, [0.0; 3]);
        assert_eq!(light.sample(V3::new(3.0, 0.0, 2.0)).1, [0.0; 3]);
        let sample = V3::new(2.0, 0.0, 0.0);
        let soft = light.sample(sample).1[0];
        light.cone = None;
        assert!(soft > 0.0 && soft < light.sample(sample).1[0]);
    }
}
