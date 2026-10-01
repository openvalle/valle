//! Closed glTF linear-blend skins. Joint matrices are sampled from the node forest per frame.

use std::collections::BTreeSet;

use serde::Deserialize;

use super::{ContractErrors, FLOAT, Root, geometry::accessor_bytes, nodes::Affine, one_error};
use crate::scene3d::MAX_SKIN_JOINTS;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct GltfSkin {
    pub joints: Vec<usize>,
    #[serde(default)]
    pub inverse_bind_matrices: Option<usize>,
    #[serde(default)]
    pub skeleton: Option<usize>,
    #[serde(default)]
    _name: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelSkin {
    pub(crate) joints: Vec<usize>,
    pub(crate) inverse_bind: Vec<Affine>,
    pub(crate) skeleton: Option<usize>,
}

pub(super) fn admit_skins(root: &Root, bin: &[u8]) -> Result<Vec<ModelSkin>, ContractErrors> {
    let mut skins = Vec::with_capacity(root.skins.len());
    for (skin_index, source) in root.skins.iter().enumerate() {
        let path = format!("/glb/skins/{skin_index}");
        if source.joints.is_empty()
            || source.joints.len() > MAX_SKIN_JOINTS
            || source.joints.iter().any(|&joint| joint >= root.nodes.len())
            || source.joints.iter().copied().collect::<BTreeSet<_>>().len() != source.joints.len()
        {
            return Err(one_error(
                format!("{path}/joints"),
                "skin joints must be distinct node indices within the joint budget",
            ));
        }
        if source
            .skeleton
            .is_some_and(|index| index >= root.nodes.len())
        {
            return Err(one_error(
                format!("{path}/skeleton"),
                "skeleton node index is out of range",
            ));
        }
        let inverse_bind = if let Some(index) = source.inverse_bind_matrices {
            let Some(accessor) = root.accessors.get(index) else {
                return Err(one_error(
                    format!("{path}/inverseBindMatrices"),
                    "inverse bind accessor index is out of range",
                ));
            };
            if accessor.kind != "MAT4"
                || accessor.component_type != FLOAT
                || accessor.normalized
                || accessor.count < source.joints.len()
            {
                return Err(one_error(
                    format!("{path}/inverseBindMatrices"),
                    "inverse bind matrices require enough non-normalized FLOAT MAT4 elements",
                ));
            }
            let bytes = accessor_bytes(root, bin, index, 64, 4, true)?;
            bytes
                .elements()
                .take(source.joints.len())
                .map(|element| {
                    let matrix = std::array::from_fn(|i| {
                        f32::from_le_bytes(element[i * 4..i * 4 + 4].try_into().unwrap())
                    });
                    Affine::new(matrix)
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![Affine::identity(); source.joints.len()]
        };
        skins.push(ModelSkin {
            joints: source.joints.clone(),
            inverse_bind,
            skeleton: source.skeleton,
        });
    }
    Ok(skins)
}
