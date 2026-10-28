//! Actual reads of each validated drawing subtree. This is derived once from the immutable
//! template, never inferred from equal paint values or from the frames visited by a cache.

use std::{collections::BTreeSet, sync::Arc};

use crate::requirements::{ExternalTexture, FontKey, RuntimeShaderKey, Scene3dKey};

use super::{
    BlendMode, DrawProgram, InstanceShape, Node, NodeId, PaintParameter, ShaderTextureBinding,
    ShaderTextureSource, TransitionKernel,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubtreeResource {
    Texture(ExternalTexture),
    Font(FontKey),
    Shader(RuntimeShaderKey),
    Scene(Scene3dKey),
}

/// Sorted, deduplicated parameter and resource reads, including mask and shader inputs.
/// Resource declarations identify reads, not the host's resolved pixels. A raster cache must
/// also bind their resolved identities, or reject resource-dependent subtrees.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubtreeDependencies {
    slots: Vec<usize>,
    resources: Vec<SubtreeResource>,
    reads_destination: bool,
}

impl SubtreeDependencies {
    pub fn slots(&self) -> &[usize] {
        &self.slots
    }

    pub fn resources(&self) -> &[SubtreeResource] {
        &self.resources
    }

    pub const fn reads_destination(&self) -> bool {
        self.reads_destination
    }
}

pub(super) fn derive(
    program: &DrawProgram,
    bindings: &[PaintParameter],
) -> Vec<Arc<SubtreeDependencies>> {
    let mut paints = vec![None; program.paints().len()];
    for binding in bindings {
        paints[binding.paint.raw() as usize] = Some(binding.slot);
    }
    let mut nodes = vec![None; program.nodes().len()];
    // Share the overwhelmingly common empty dependency set among unrelated static nodes.
    let empty = Arc::new(SubtreeDependencies::default());
    for index in 0..nodes.len() {
        visit(
            program,
            NodeId::from_raw(index as u32),
            &paints,
            &mut nodes,
            &empty,
        );
    }
    nodes.into_iter().map(Option::unwrap).collect()
}

fn visit(
    program: &DrawProgram,
    id: NodeId,
    paints: &[Option<usize>],
    nodes: &mut [Option<Arc<SubtreeDependencies>>],
    empty: &Arc<SubtreeDependencies>,
) -> Arc<SubtreeDependencies> {
    if let Some(cached) = &nodes[id.raw() as usize] {
        return cached.clone();
    }
    let mut slots = BTreeSet::new();
    let mut resources = BTreeSet::new();
    let mut reads_destination = false;
    let mut paint = |id: super::PaintId| {
        slots.extend(paints[id.raw() as usize]);
    };
    match &program.nodes()[id.raw() as usize] {
        Node::Path(path) => {
            path.fill.into_iter().for_each(&mut paint);
            path.stroke.iter().for_each(|stroke| paint(stroke.paint));
        }
        Node::GlyphRun(run) => {
            paint(run.paint);
            run.stroke.iter().for_each(|stroke| paint(stroke.paint));
            resources.insert(SubtreeResource::Font(run.font.clone()));
        }
        Node::Group(group) => {
            reads_destination =
                group.backdrop.is_some() || group.internal_blend != BlendMode::Normal;
            for child in group
                .children
                .iter()
                .copied()
                .chain(group.mask.iter().map(|mask| mask.source))
            {
                let child = visit(program, child, paints, nodes, empty);
                slots.extend(child.slots.iter().copied());
                resources.extend(child.resources.iter().cloned());
                reads_destination |= child.reads_destination;
            }
            if let Some(shader) = &group.shader {
                resources.insert(SubtreeResource::Shader(shader.shader.clone()));
                texture_reads(&shader.textures, &mut resources, &mut reads_destination);
            }
            if let Some(transition) = &group.transition
                && let TransitionKernel::Shader {
                    shader, textures, ..
                } = &transition.kernel
            {
                resources.insert(SubtreeResource::Shader(shader.clone()));
                texture_reads(textures, &mut resources, &mut reads_destination);
            }
        }
        Node::Image(image) => {
            resources.insert(SubtreeResource::Texture(image.texture.clone()));
        }
        Node::InstanceBatch(batch) => {
            if let InstanceShape::Image(region) = &batch.shape {
                resources.insert(SubtreeResource::Texture(region.texture.clone()));
            }
        }
        Node::RuntimeShader(shader) => {
            resources.insert(SubtreeResource::Shader(shader.shader.clone()));
            texture_reads(&shader.textures, &mut resources, &mut reads_destination);
        }
        Node::Scene3d(scene) => {
            resources.insert(SubtreeResource::Scene(scene.scene.clone()));
        }
        Node::Shadow(_) => {}
    }
    let dependencies = if slots.is_empty() && resources.is_empty() && !reads_destination {
        empty.clone()
    } else {
        Arc::new(SubtreeDependencies {
            slots: slots.into_iter().collect(),
            resources: resources.into_iter().collect(),
            reads_destination,
        })
    };
    nodes[id.raw() as usize] = Some(dependencies.clone());
    dependencies
}

fn texture_reads(
    bindings: &[ShaderTextureBinding],
    resources: &mut BTreeSet<SubtreeResource>,
    destination: &mut bool,
) {
    for binding in bindings {
        match &binding.source {
            Some(ShaderTextureSource::External { texture }) => {
                resources.insert(SubtreeResource::Texture(texture.clone()));
            }
            Some(ShaderTextureSource::Backdrop { .. }) => *destination = true,
            None => {}
        }
    }
}
