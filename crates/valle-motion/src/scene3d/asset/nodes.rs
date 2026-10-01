//! Source-index identities and immutable affine transforms for a closed static glTF forest.
use super::{
    ContractErrors, ModelMesh, ModelVertex, Node, PrimitiveRange, Root, SkinVertex,
    animation::ModelTrs,
    geometry::{cross, normalize, valid_weight},
    one_error,
    skin::ModelSkin,
};
use crate::scene3d::{MAX_ABS_POSITION, MAX_NODE_DEPTH, MAX_SCALE, MAX_TRIANGLES, MAX_VERTICES};

#[derive(Clone, Debug, PartialEq)]
pub struct ModelNode {
    /// Original glTF node index, stable across scene traversal and duplicate display names.
    pub id: u32,
    pub name: Option<String>,
    pub parent: Option<u32>,
    pub mesh: Option<u32>,
    /// Local column-major affine transform, before the parent transform.
    pub transform: [f32; 16],
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelInstance {
    pub node: u32,
    pub mesh: u32,
    pub skin: Option<u32>,
    pub(crate) transform: Affine,
    pub(crate) weights: Vec<f32>,
    pub(crate) joint_palette: Vec<Affine>,
}
impl ModelInstance {
    pub fn world_transform(&self) -> &[f32; 16] {
        &self.transform.matrix
    }
    pub fn morph_weights(&self) -> &[f32] {
        &self.weights
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Affine {
    matrix: [f32; 16],
    normal: [[f32; 3]; 3],
    pub mirrored: bool,
}
impl Affine {
    pub(crate) fn identity() -> Self {
        Self {
            matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            normal: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            mirrored: false,
        }
    }
    pub(crate) fn new(matrix: [f32; 16]) -> Result<Self, ContractErrors> {
        if matrix.iter().any(|v| !v.is_finite() || v.abs() > 1.0e12)
            || [matrix[3], matrix[7], matrix[11], matrix[15]] != [0.0, 0.0, 0.0, 1.0]
        {
            return Err(one_error(
                "/glb/nodes/transform",
                "matrix must be finite, bounded and affine",
            ));
        }
        let columns: [[f64; 3]; 3] =
            std::array::from_fn(|c| std::array::from_fn(|r| matrix[c * 4 + r] as f64));
        let cofactor: [[f64; 3]; 3] = std::array::from_fn(|c| {
            let a = columns[(c + 1) % 3];
            let b = columns[(c + 2) % 3];
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        });
        let determinant: f64 = (0..3).map(|i| columns[0][i] * cofactor[0][i]).sum();
        if !determinant.is_finite() || determinant == 0.0 {
            return Err(one_error(
                "/glb/nodes/transform",
                "singular node transforms are not supported",
            ));
        }
        let normal = cofactor.map(|c| c.map(|v| (v / determinant) as f32));
        if normal
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1.0e12)
        {
            return Err(one_error(
                "/glb/nodes/transform",
                "inverse transform exceeds numeric bounds",
            ));
        }
        Ok(Self {
            matrix,
            normal,
            mirrored: determinant < 0.0,
        })
    }
    pub(crate) fn compose(self, local: Self) -> Result<Self, ContractErrors> {
        Self::new(std::array::from_fn(|i| {
            let row = i % 4;
            let column = i / 4;
            (0..4)
                .map(|k| self.matrix[k * 4 + row] * local.matrix[column * 4 + k])
                .sum()
        }))
    }
    pub fn point(self, p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|r| {
            self.matrix[r] * p[0]
                + self.matrix[4 + r] * p[1]
                + self.matrix[8 + r] * p[2]
                + self.matrix[12 + r]
        })
    }
    pub fn normal(self, n: [f32; 3]) -> [f32; 3] {
        normalize(self.normal_raw(n))
    }
    pub(crate) fn normal_raw(self, n: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|r| {
            self.normal[0][r] * n[0] + self.normal[1][r] * n[1] + self.normal[2][r] * n[2]
        })
    }

    pub(crate) fn from_model_trs(trs: ModelTrs, index: usize) -> Result<Self, ContractErrors> {
        local_transform(
            &Node {
                mesh: None,
                skin: None,
                weights: None,
                name: None,
                children: Vec::new(),
                matrix: None,
                rotation: Some(trs.rotation),
                translation: Some(trs.translation),
                scale: Some(trs.scale),
                _extras: None,
            },
            index,
        )
    }
}

fn local_transform(node: &Node, index: usize) -> Result<Affine, ContractErrors> {
    let path = format!("/glb/nodes/{index}");
    let matrix = if let Some(matrix) = node.matrix {
        if node.translation.is_some() || node.rotation.is_some() || node.scale.is_some() {
            return Err(one_error(
                &path,
                "matrix and TRS properties are mutually exclusive",
            ));
        }
        matrix
    } else {
        let rotation = node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let length: f32 = rotation.iter().map(|v| v * v).sum();
        if !length.is_finite() || (length - 1.0).abs() > 0.001 {
            return Err(one_error(
                format!("{path}/rotation"),
                "rotation must be a finite unit quaternion",
            ));
        }
        let q = rotation.map(|v| v / libm::sqrtf(length));
        let scale = node.scale.unwrap_or([1.0; 3]);
        let translation = node.translation.unwrap_or([0.0; 3]);
        let rotate = |v: [f32; 3]| {
            let xyz = [q[0], q[1], q[2]];
            let c = cross(xyz, v);
            let c = cross(xyz, std::array::from_fn(|i| c[i] + q[3] * v[i]));
            std::array::from_fn::<_, 3, _>(|i| v[i] + 2.0 * c[i])
        };
        let columns: [[f32; 3]; 3] = std::array::from_fn(|c| {
            rotate(std::array::from_fn(|r| if r == c { scale[c] } else { 0.0 }))
        });
        std::array::from_fn(|i| {
            let c = i / 4;
            let r = i % 4;
            if c == 3 {
                if r == 3 { 1.0 } else { translation[r] }
            } else if r == 3 {
                0.0
            } else {
                columns[c][r]
            }
        })
    };
    let affine = Affine::new(matrix)?;
    let lengths: [f32; 3] = std::array::from_fn(|c| {
        libm::sqrtf((0..3).map(|r| matrix[c * 4 + r] * matrix[c * 4 + r]).sum())
    });
    if lengths.iter().any(|s| !(0.000001..=MAX_SCALE).contains(s))
        || matrix[12..15].iter().any(|v| v.abs() > MAX_ABS_POSITION)
    {
        return Err(one_error(
            &path,
            "node translation and non-zero scale magnitudes must be bounded",
        ));
    }
    // A glTF local matrix must decompose as TRS. Composition may legitimately introduce shear,
    // which remains supported in world matrices and their inverse-transpose normal transform.
    for a in 0..3 {
        for b in a + 1..3 {
            let dot: f32 = (0..3).map(|r| matrix[a * 4 + r] * matrix[b * 4 + r]).sum();
            if dot.abs() > lengths[a] * lengths[b] * 0.0001 {
                return Err(one_error(
                    &path,
                    "local matrix must be decomposable into TRS (no shear)",
                ));
            }
        }
    }
    Ok(affine)
}

fn resolve(
    index: usize,
    parents: &[Option<usize>],
    locals: &[Affine],
    world: &mut [Option<(Affine, usize)>],
    visiting: &mut [bool],
    stack: usize,
) -> Result<(Affine, usize), ContractErrors> {
    if let Some(result) = world[index] {
        return Ok(result);
    }
    if visiting[index] {
        return Err(one_error(
            "/glb/nodes/children",
            "node hierarchy contains a cycle",
        ));
    }
    if stack >= MAX_NODE_DEPTH {
        return Err(one_error(
            "/glb/nodes/children",
            "node hierarchy exceeds the depth budget",
        ));
    }
    visiting[index] = true;
    let (transform, depth) = if let Some(parent) = parents[index] {
        let (transform, depth) = resolve(parent, parents, locals, world, visiting, stack + 1)?;
        (transform.compose(locals[index])?, depth + 1)
    } else {
        (locals[index], 1)
    };
    if depth > MAX_NODE_DEPTH {
        return Err(one_error(
            "/glb/nodes/children",
            "node hierarchy exceeds the depth budget",
        ));
    }
    visiting[index] = false;
    world[index] = Some((transform, depth));
    Ok((transform, depth))
}
pub(super) fn admit_nodes(
    root: &Root,
    meshes: &[ModelMesh],
    mesh_weights: &[Vec<f32>],
    primitives: &[PrimitiveRange],
    indices: &[u32],
    vertices: &[ModelVertex],
    skin_vertices: &[Option<Vec<SkinVertex>>],
    skins: &[ModelSkin],
) -> Result<(Vec<ModelNode>, Vec<ModelInstance>), ContractErrors> {
    let mut parents = vec![None; root.nodes.len()];
    let mut locals = Vec::with_capacity(root.nodes.len());
    for (index, node) in root.nodes.iter().enumerate() {
        if node.mesh.is_some_and(|m| m >= meshes.len()) {
            return Err(one_error(
                format!("/glb/nodes/{index}/mesh"),
                "mesh index is out of range",
            ));
        }
        if node.skin.is_some_and(|s| s >= skins.len()) || node.skin.is_some() && node.mesh.is_none()
        {
            return Err(one_error(
                format!("/glb/nodes/{index}/skin"),
                "skin must reference an admitted skin and a mesh",
            ));
        }
        if let Some(weights) = &node.weights {
            if node
                .mesh
                .is_none_or(|m| meshes[m].morph_target_count as usize != weights.len())
                || weights.iter().any(|w| !valid_weight(*w))
            {
                return Err(one_error(
                    format!("/glb/nodes/{index}/weights"),
                    "node weights require a morph mesh and matching finite bounded values",
                ));
            }
        }
        for &child in &node.children {
            let Some(parent) = parents.get_mut(child) else {
                return Err(one_error(
                    format!("/glb/nodes/{index}/children"),
                    "child node index is out of range",
                ));
            };
            if parent.replace(index).is_some() {
                return Err(one_error(
                    "/glb/nodes/children",
                    "a node must have exactly one parent reference",
                ));
            }
        }
        locals.push(local_transform(node, index)?);
    }
    let mut world = vec![None; locals.len()];
    let mut visiting = vec![false; locals.len()];
    for index in 0..locals.len() {
        resolve(index, &parents, &locals, &mut world, &mut visiting, 0)?;
    }
    for scene in &root.scenes {
        let mut roots = std::collections::BTreeSet::new();
        for &index in &scene.nodes {
            if index >= locals.len() || parents[index].is_some() || !roots.insert(index) {
                return Err(one_error(
                    "/glb/scenes/nodes",
                    "scene roots must be distinct, in range and have no parent",
                ));
            }
        }
    }
    let mut active = vec![false; locals.len()];
    let mut pending = root.scenes[root.scene].nodes.clone();
    while let Some(index) = pending.pop() {
        active[index] = true;
        pending.extend_from_slice(&root.nodes[index].children);
    }
    let mut nodes = Vec::with_capacity(locals.len());
    let mut instances = Vec::new();
    let mut rendered_vertices = 0u32;
    let mut rendered_triangles = 0u32;
    for (index, node) in root.nodes.iter().enumerate() {
        nodes.push(ModelNode {
            id: index as u32,
            name: node.name.clone(),
            parent: parents[index].map(|i| i as u32),
            mesh: node.mesh.map(|i| i as u32),
            transform: locals[index].matrix,
            active: active[index],
        });
        if !active[index] {
            continue;
        }
        let Some(mesh_index) = node.mesh else {
            continue;
        };
        let mesh = meshes[mesh_index];
        if let Some(skin_index) = node.skin {
            let skin = &skins[skin_index];
            if skin.joints.iter().any(|&joint| !active[joint])
                || skin.skeleton.is_some_and(|skeleton| !active[skeleton])
            {
                return Err(one_error(
                    format!("/glb/nodes/{index}/skin"),
                    "skin joints and skeleton must belong to the selected scene",
                ));
            }
            let common = skin.skeleton.map_or_else(
                || {
                    let mut candidate = Some(skin.joints[0]);
                    while let Some(ancestor) = candidate {
                        if skin
                            .joints
                            .iter()
                            .all(|&joint| is_ancestor(ancestor, joint, &parents))
                        {
                            return Some(ancestor);
                        }
                        candidate = parents[ancestor];
                    }
                    None
                },
                Some,
            );
            if common.is_none_or(|ancestor| {
                skin.joints
                    .iter()
                    .any(|&joint| !is_ancestor(ancestor, joint, &parents))
            }) {
                return Err(one_error(
                    format!("/glb/nodes/{index}/skin"),
                    "skin joints must share the declared or inferred skeleton root",
                ));
            }
            for primitive_index in mesh.first_primitive as usize
                ..(mesh.first_primitive + mesh.primitive_count) as usize
            {
                let Some(attributes) = &skin_vertices[primitive_index] else {
                    return Err(one_error(
                        format!("/glb/nodes/{index}/skin"),
                        "skinned mesh primitives require JOINTS_0 and WEIGHTS_0",
                    ));
                };
                if attributes
                    .iter()
                    .flat_map(|vertex| vertex.joints)
                    .any(|joint| joint as usize >= skin.joints.len())
                {
                    return Err(one_error(
                        format!("/glb/nodes/{index}/skin"),
                        "JOINTS_0 values must index this skin's joint list",
                    ));
                }
            }
        }
        rendered_vertices = rendered_vertices.saturating_add(mesh.vertex_count);
        rendered_triangles = rendered_triangles.saturating_add(mesh.triangle_count);
        if rendered_vertices > MAX_VERTICES || rendered_triangles > MAX_TRIANGLES {
            return Err(one_error(
                "/glb/budget/instances",
                "instanced geometry exceeds the rendered vertex or triangle budget",
            ));
        }
        let transform = world[index].unwrap().0;
        for primitive in &primitives
            [mesh.first_primitive as usize..(mesh.first_primitive + mesh.primitive_count) as usize]
        {
            if node.skin.is_some() {
                continue;
            }
            for &vertex in &indices[primitive.first_index as usize
                ..(primitive.first_index + primitive.index_count) as usize]
            {
                if transform
                    .point(vertices[vertex as usize].position)
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > MAX_ABS_POSITION)
                {
                    return Err(one_error(
                        "/glb/nodes/transform",
                        "transformed positions must remain finite and bounded",
                    ));
                }
            }
        }
        instances.push(ModelInstance {
            node: index as u32,
            mesh: mesh_index as u32,
            skin: node.skin.map(|i| i as u32),
            transform,
            weights: node
                .weights
                .clone()
                .unwrap_or_else(|| mesh_weights[mesh_index].clone()),
            joint_palette: Vec::new(),
        });
    }
    if instances.is_empty() {
        return Err(one_error(
            "/glb/scenes",
            "default scene must contain a mesh instance",
        ));
    }
    Ok((nodes, instances))
}

fn is_ancestor(ancestor: usize, mut node: usize, parents: &[Option<usize>]) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        let Some(parent) = parents[node] else {
            return false;
        };
        node = parent;
    }
}

impl super::AdmittedModel {
    /// Resolve only from immutable bind transforms plus this frame's explicit replacements.
    /// Parent indices may follow children; the admitted forest resolver handles either order.
    pub(crate) fn frame_instances(
        &self,
        states: &[crate::scene3d::NodeFrameState],
        animation: Option<(u32, f32)>,
    ) -> Result<Vec<ModelInstance>, ContractErrors> {
        let mut locals = self
            .nodes
            .iter()
            .map(|n| Affine::new(n.transform))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some((clip, time)) = animation {
            let Some(animation) = self.animations.get(clip as usize) else {
                return Err(one_error(
                    "/frame/animation/clip",
                    "animation clip is out of range",
                ));
            };
            let mut trs = self.source_trs.clone();
            animation.apply(time, &mut trs)?;
            let mut touched = vec![false; self.nodes.len()];
            for channel in &animation.channels {
                if channel.is_trs() {
                    touched[channel.node as usize] = true;
                }
            }
            for (index, touched) in touched.into_iter().enumerate() {
                if touched {
                    locals[index] = Affine::from_model_trs(trs[index].unwrap(), index)?;
                }
            }
        }
        for state in states {
            let Some(node) = self.nodes.get(state.id as usize).filter(|n| n.active) else {
                return Err(one_error(
                    "/frame/nodes/id",
                    "node binding must reference an active node in the selected model scene",
                ));
            };
            locals[node.id as usize] = Affine::from_transform(state.transform)?;
        }
        let parents = self
            .nodes
            .iter()
            .map(|n| n.parent.map(|p| p as usize))
            .collect::<Vec<_>>();
        let mut world = vec![None; locals.len()];
        let mut visiting = vec![false; locals.len()];
        let mut instances = Vec::with_capacity(self.instances.len());
        let weight_overrides = animation
            .map(|(clip, time)| {
                self.animations[clip as usize].sample_weights(time, self.nodes.len())
            })
            .transpose()?;
        for source in &self.instances {
            let transform = resolve(
                source.node as usize,
                &parents,
                &locals,
                &mut world,
                &mut visiting,
                0,
            )?
            .0;
            let mesh = self.meshes[source.mesh as usize];
            for primitive in &self.primitives[mesh.first_primitive as usize
                ..(mesh.first_primitive + mesh.primitive_count) as usize]
            {
                if source.skin.is_some() {
                    continue;
                }
                for &index in &self.indices[primitive.first_index as usize
                    ..(primitive.first_index + primitive.index_count) as usize]
                {
                    if transform
                        .point(self.vertices[index as usize].position)
                        .iter()
                        .any(|v| !v.is_finite() || v.abs() > MAX_ABS_POSITION)
                    {
                        return Err(one_error(
                            "/frame/nodes/transform",
                            "transformed model positions must remain finite and bounded",
                        ));
                    }
                }
            }
            let joint_palette = if let Some(skin_index) = source.skin {
                let skin = &self.skins[skin_index as usize];
                skin.joints
                    .iter()
                    .zip(&skin.inverse_bind)
                    .map(|(&joint, &inverse)| {
                        let world_transform =
                            resolve(joint, &parents, &locals, &mut world, &mut visiting, 0)?.0;
                        world_transform.compose(inverse)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            };
            instances.push(ModelInstance {
                node: source.node,
                mesh: source.mesh,
                skin: source.skin,
                transform,
                weights: weight_overrides
                    .as_ref()
                    .and_then(|overrides| overrides[source.node as usize].clone())
                    .unwrap_or_else(|| source.weights.clone()),
                joint_palette,
            });
        }
        Ok(instances)
    }
}
