//! Immutable glTF TRS animation tracks. Sampling reads a clip and an explicit time only.
use std::collections::BTreeSet;

use serde::Deserialize;

use super::{
    ContractErrors, FLOAT, ModelMesh, Root,
    geometry::{accessor_bytes, valid_weight},
    one_error,
};
use crate::scene3d::{
    MAX_ANIMATION_CHANNELS, MAX_ANIMATION_KEYS, MAX_ANIMATIONS, MAX_MORPH_TARGETS,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ModelTrs {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelAnimation {
    pub name: Option<String>,
    pub duration: f32,
    pub(crate) samplers: Vec<Option<ModelSampler>>,
    pub(crate) channels: Vec<ModelChannel>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelSampler {
    times: Vec<f32>,
    values: Vec<f32>,
    components: usize,
    scalar_output: bool,
    interpolation: Interpolation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interpolation {
    Linear,
    Step,
    CubicSpline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ChannelPath {
    Translation,
    Rotation,
    Scale,
    Weights,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelChannel {
    pub node: u32,
    sampler: usize,
    path: ChannelPath,
}
impl ModelChannel {
    pub(crate) fn is_trs(&self) -> bool {
        self.path != ChannelPath::Weights
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GltfAnimation {
    channels: Vec<GltfChannel>,
    samplers: Vec<GltfSampler>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GltfChannel {
    sampler: usize,
    target: GltfTarget,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GltfTarget {
    node: usize,
    path: String,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GltfSampler {
    input: usize,
    output: usize,
    #[serde(default = "linear")]
    interpolation: String,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

fn linear() -> String {
    "LINEAR".into()
}

pub(super) fn admit_animations(
    root: &Root,
    bin: &[u8],
    meshes: &[ModelMesh],
) -> Result<(Vec<ModelAnimation>, Vec<Option<ModelTrs>>), ContractErrors> {
    if root.animations.len() > MAX_ANIMATIONS {
        return Err(one_error(
            "/glb/animations",
            "animation count exceeds the clip budget",
        ));
    }
    let source_trs = root
        .nodes
        .iter()
        .map(|node| {
            node.matrix.is_none().then_some(ModelTrs {
                translation: node.translation.unwrap_or([0.0; 3]),
                rotation: node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                scale: node.scale.unwrap_or([1.0; 3]),
            })
        })
        .collect::<Vec<_>>();
    let mut clips = Vec::with_capacity(root.animations.len());
    let mut total_keys = 0usize;
    for (clip_index, clip) in root.animations.iter().enumerate() {
        let path = format!("/glb/animations/{clip_index}");
        if clip.channels.is_empty()
            || clip.channels.len() > MAX_ANIMATION_CHANNELS
            || clip.samplers.is_empty()
            || clip.samplers.len() > MAX_ANIMATION_CHANNELS
        {
            return Err(one_error(
                &path,
                "clip channel and sampler counts must fit the budget",
            ));
        }
        let mut channels = Vec::with_capacity(clip.channels.len());
        let mut samplers: Vec<Option<ModelSampler>> = vec![None; clip.samplers.len()];
        let mut targets = BTreeSet::new();
        let mut duration = 0.0f32;
        for (channel_index, channel) in clip.channels.iter().enumerate() {
            let channel_path = format!("{path}/channels/{channel_index}");
            let target = &channel.target;
            if target.node >= root.nodes.len() {
                return Err(one_error(
                    format!("{channel_path}/target/node"),
                    "animated node must exist",
                ));
            }
            let kind = match target.path.as_str() {
                "translation" => ChannelPath::Translation,
                "rotation" => ChannelPath::Rotation,
                "scale" => ChannelPath::Scale,
                "weights" => ChannelPath::Weights,
                _ => {
                    return Err(one_error(
                        format!("{channel_path}/target/path"),
                        "only node translation, rotation, scale and weights animation is admitted",
                    ));
                }
            };
            if kind == ChannelPath::Weights {
                if root.nodes[target.node]
                    .mesh
                    .and_then(|index| meshes.get(index))
                    .is_none_or(|mesh| mesh.morph_target_count == 0)
                {
                    return Err(one_error(
                        format!("{channel_path}/target/node"),
                        "weight animation requires a node with morph targets",
                    ));
                }
            } else if source_trs[target.node].is_none() {
                return Err(one_error(
                    format!("{channel_path}/target/node"),
                    "TRS animation requires a node without a matrix",
                ));
            }
            if !targets.insert((target.node, kind)) {
                return Err(one_error(
                    &channel_path,
                    "clip channels must not repeat a node path",
                ));
            }
            let Some(source) = clip.samplers.get(channel.sampler) else {
                return Err(one_error(
                    format!("{channel_path}/sampler"),
                    "sampler index is out of range",
                ));
            };
            let components = match kind {
                ChannelPath::Rotation => 4,
                ChannelPath::Weights => {
                    meshes[root.nodes[target.node].mesh.unwrap()].morph_target_count as usize
                }
                _ => 3,
            };
            let scalar_output = kind == ChannelPath::Weights;
            if let Some(sampler) = &samplers[channel.sampler] {
                if sampler.components != components || sampler.scalar_output != scalar_output {
                    return Err(one_error(
                        format!("{channel_path}/sampler"),
                        "one sampler cannot target paths with different element widths",
                    ));
                }
            } else {
                let sampler_path = format!("{path}/samplers/{}", channel.sampler);
                let interpolation = match source.interpolation.as_str() {
                    "LINEAR" => Interpolation::Linear,
                    "STEP" => Interpolation::Step,
                    "CUBICSPLINE" => Interpolation::CubicSpline,
                    _ => {
                        return Err(one_error(
                            format!("{sampler_path}/interpolation"),
                            "interpolation must be LINEAR, STEP or CUBICSPLINE",
                        ));
                    }
                };
                let times = read_floats(root, bin, source.input, 1, true)?;
                if times
                    .iter()
                    .any(|t| !t.is_finite() || !(0.0..=1.0e9).contains(t))
                    || times.windows(2).any(|pair| pair[0] >= pair[1])
                {
                    return Err(one_error(
                        format!("{sampler_path}/input"),
                        "key times must be finite, non-negative and strictly increasing",
                    ));
                }
                total_keys = total_keys.saturating_add(times.len());
                if total_keys > MAX_ANIMATION_KEYS {
                    return Err(one_error(
                        "/glb/animations",
                        "animation key budget exceeded",
                    ));
                }
                let values = read_floats(root, bin, source.output, components, scalar_output)?;
                let key_width = components
                    * if interpolation == Interpolation::CubicSpline {
                        3
                    } else {
                        1
                    };
                if values.len() != times.len() * key_width
                    || values.iter().any(|v| !v.is_finite() || v.abs() > 1.0e9)
                {
                    return Err(one_error(
                        format!("{sampler_path}/output"),
                        "output count and finite values must match key times and interpolation",
                    ));
                }
                if kind == ChannelPath::Rotation {
                    for key in 0..times.len() {
                        let group = if interpolation == Interpolation::CubicSpline {
                            key * 3 + 1
                        } else {
                            key
                        };
                        let quaternion = &values[group * 4..group * 4 + 4];
                        let length_squared: f32 = quaternion.iter().map(|v| v * v).sum();
                        if (length_squared - 1.0).abs() > 0.002 {
                            return Err(one_error(
                                format!("{sampler_path}/output"),
                                "rotation key values must be unit quaternions",
                            ));
                        }
                    }
                }
                if kind == ChannelPath::Weights {
                    let group_width = if interpolation == Interpolation::CubicSpline {
                        3
                    } else {
                        1
                    };
                    for key in 0..times.len() {
                        let start =
                            (key * group_width + if group_width == 3 { 1 } else { 0 }) * components;
                        if values[start..start + components]
                            .iter()
                            .any(|w| !valid_weight(*w))
                        {
                            return Err(one_error(
                                format!("{sampler_path}/output"),
                                "weight key values must be finite and bounded",
                            ));
                        }
                    }
                }
                let sampler = ModelSampler {
                    times,
                    values,
                    components,
                    scalar_output,
                    interpolation,
                };
                duration = duration.max(*sampler.times.last().unwrap());
                samplers[channel.sampler] = Some(sampler);
            }
            channels.push(ModelChannel {
                node: target.node as u32,
                sampler: channel.sampler,
                path: kind,
            });
        }
        clips.push(ModelAnimation {
            name: clip.name.clone(),
            duration,
            samplers,
            channels,
        });
    }
    Ok((clips, source_trs))
}

fn read_floats(
    root: &Root,
    bin: &[u8],
    index: usize,
    components: usize,
    scalar_output: bool,
) -> Result<Vec<f32>, ContractErrors> {
    let Some(accessor) = root.accessors.get(index) else {
        return Err(one_error(
            format!("/glb/accessors/{index}"),
            "animation accessor is missing",
        ));
    };
    let expected = if scalar_output {
        "SCALAR"
    } else if components == 4 {
        "VEC4"
    } else {
        "VEC3"
    };
    if accessor.kind != expected || accessor.component_type != FLOAT || accessor.normalized {
        return Err(one_error(
            format!("/glb/accessors/{index}"),
            "animation sampler requires a non-normalized FLOAT accessor of the target type",
        ));
    }
    let bytes = accessor_bytes(
        root,
        bin,
        index,
        if scalar_output { 4 } else { components * 4 },
        4,
        true,
    )?;
    Ok(bytes
        .elements()
        .flat_map(|element| {
            element
                .chunks_exact(4)
                .map(|component| f32::from_le_bytes(component.try_into().unwrap()))
        })
        .collect())
}

impl ModelAnimation {
    pub(crate) fn apply(
        &self,
        time: f32,
        nodes: &mut [Option<ModelTrs>],
    ) -> Result<(), ContractErrors> {
        for channel in &self.channels {
            if channel.path == ChannelPath::Weights {
                continue;
            }
            let sampler = self.samplers[channel.sampler]
                .as_ref()
                .expect("admission resolves every channel sampler");
            let values = sampler.sample(time, channel.path == ChannelPath::Rotation);
            let node = nodes[channel.node as usize]
                .as_mut()
                .expect("admission requires animated nodes to use TRS");
            match channel.path {
                ChannelPath::Translation => node.translation = values[..3].try_into().unwrap(),
                ChannelPath::Scale => node.scale = values[..3].try_into().unwrap(),
                ChannelPath::Rotation => {
                    let length = libm::sqrtf(values[..4].iter().map(|value| value * value).sum());
                    if !length.is_finite() || length <= 1.0e-12 {
                        return Err(one_error(
                            "/frame/animation",
                            "sampled rotation quaternion is degenerate",
                        ));
                    }
                    node.rotation = std::array::from_fn(|i| values[i] / length);
                }
                ChannelPath::Weights => unreachable!(),
            }
        }
        Ok(())
    }

    pub(crate) fn sample_weights(
        &self,
        time: f32,
        node_count: usize,
    ) -> Result<Vec<Option<Vec<f32>>>, ContractErrors> {
        let mut weights = vec![None; node_count];
        for channel in &self.channels {
            if channel.path != ChannelPath::Weights {
                continue;
            }
            let sampler = self.samplers[channel.sampler].as_ref().unwrap();
            let sampled = sampler.sample(time, false);
            if sampled[..sampler.components]
                .iter()
                .any(|w| !valid_weight(*w))
            {
                return Err(one_error(
                    "/frame/animation/weights",
                    "sampled morph weights must be finite and bounded",
                ));
            }
            weights[channel.node as usize] = Some(sampled[..sampler.components].to_vec());
        }
        Ok(weights)
    }
}

impl ModelSampler {
    fn key(&self, index: usize) -> [f32; MAX_MORPH_TARGETS] {
        let group = if self.interpolation == Interpolation::CubicSpline {
            index * 3 + 1
        } else {
            index
        };
        self.group(group)
    }

    fn group(&self, group: usize) -> [f32; MAX_MORPH_TARGETS] {
        let start = group * self.components;
        let mut value = [0.0; MAX_MORPH_TARGETS];
        value[..self.components].copy_from_slice(&self.values[start..start + self.components]);
        value
    }

    fn sample(&self, time: f32, rotation: bool) -> [f32; MAX_MORPH_TARGETS] {
        let next = self.times.partition_point(|key| *key <= time);
        if next == 0 {
            return self.key(0);
        }
        if next == self.times.len() || self.interpolation == Interpolation::Step {
            return self.key(next - 1);
        }
        let before = next - 1;
        let interval = self.times[next] - self.times[before];
        let t = (time - self.times[before]) / interval;
        let a = self.key(before);
        let b = self.key(next);
        match self.interpolation {
            Interpolation::Linear if rotation => {
                let q = slerp(a[..4].try_into().unwrap(), b[..4].try_into().unwrap(), t);
                let mut sampled = [0.0; MAX_MORPH_TARGETS];
                sampled[..4].copy_from_slice(&q);
                sampled
            }
            Interpolation::Linear => std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t),
            Interpolation::CubicSpline => {
                let out_tangent = self.group(before * 3 + 2);
                let in_tangent = self.group(next * 3);
                let t2 = t * t;
                let t3 = t2 * t;
                std::array::from_fn(|i| {
                    (2.0 * t3 - 3.0 * t2 + 1.0) * a[i]
                        + (t3 - 2.0 * t2 + t) * interval * out_tangent[i]
                        + (-2.0 * t3 + 3.0 * t2) * b[i]
                        + (t3 - t2) * interval * in_tangent[i]
                })
            }
            Interpolation::Step => unreachable!(),
        }
    }
}

fn slerp(a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    if dot < 0.0 {
        b = b.map(|value| -value);
        dot = -dot;
    }
    let value = if dot > 0.9995 {
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
    } else {
        let angle = libm::acosf(dot.clamp(-1.0, 1.0));
        let inverse = 1.0 / libm::sinf(angle);
        let wa = libm::sinf((1.0 - t) * angle) * inverse;
        let wb = libm::sinf(t * angle) * inverse;
        std::array::from_fn(|i| a[i] * wa + b[i] * wb)
    };
    let length = libm::sqrtf(value.iter().map(|v| v * v).sum());
    value.map(|v| v / length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_linear_cubic_and_quaternion_sampling_use_one_explicit_time() {
        let linear = ModelSampler {
            times: vec![0.0, 2.0],
            values: vec![0.0, 0.0, 0.0, 4.0, 0.0, 0.0],
            components: 3,
            scalar_output: false,
            interpolation: Interpolation::Linear,
        };
        assert_eq!(linear.sample(-1.0, false)[0], 0.0);
        assert_eq!(linear.sample(1.0, false)[0], 2.0);
        assert_eq!(linear.sample(3.0, false)[0], 4.0);

        let step = ModelSampler {
            interpolation: Interpolation::Step,
            ..linear.clone()
        };
        assert_eq!(step.sample(1.999, false)[0], 0.0);
        assert_eq!(step.sample(2.0, false)[0], 4.0);

        let cubic = ModelSampler {
            times: vec![0.0, 2.0],
            // Each key has an in tangent, a value and an out tangent.
            values: vec![
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0,
                0.0, 0.0,
            ],
            components: 3,
            scalar_output: false,
            interpolation: Interpolation::CubicSpline,
        };
        assert_eq!(cubic.sample(1.0, false)[0], 2.5);

        let rotation = ModelSampler {
            times: vec![0.0, 2.0],
            values: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0],
            components: 4,
            scalar_output: false,
            interpolation: Interpolation::Linear,
        };
        let halfway = rotation.sample(1.0, true);
        assert!((halfway[2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1.0e-6);
        assert!((halfway[3] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1.0e-6);

        let weights = ModelSampler {
            times: vec![0.0, 2.0],
            values: vec![0.0; 5].into_iter().chain([1.0; 5]).collect(),
            components: 5,
            scalar_output: true,
            interpolation: Interpolation::Linear,
        };
        assert_eq!(&weights.sample(1.0, false)[..5], &[0.5; 5]);
    }
}
