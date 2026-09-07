use std::path::Path;

use anyhow::{Context, Result, ensure};

use crate::models::inference::edgetam::{
    EMBEDDING_DIM, EMBEDDING_GRID, MEMORY_SLOTS, MEMORY_TOKENS_PER_FRAME,
};

/// Immutable host constants loaded once from the verified model artifact.
#[derive(Debug)]
pub struct EdgeTamConstants {
    pub(crate) current_positions: Vec<f32>,
    pub(crate) memory_positions: Vec<f32>,
    pub(crate) temporal_positions: Vec<f32>,
    pub(crate) no_memory_embedding: Vec<f32>,
    pub(crate) no_object_pointer: Vec<f32>,
}

impl EdgeTamConstants {
    pub fn load(artifact_root: &Path) -> Result<Self> {
        let current_positions = load_expected(
            &artifact_root.join("const_curr_pos.npy"),
            &[1, EMBEDDING_GRID * EMBEDDING_GRID, EMBEDDING_DIM],
        )?;
        let memory_positions = load_expected(
            &artifact_root.join("const_maskmem_pos.npy"),
            &[1, MEMORY_TOKENS_PER_FRAME, 64],
        )?;
        let temporal_positions = load_expected(
            &artifact_root.join("const_maskmem_tpos.npy"),
            &[MEMORY_SLOTS, 1, 1, 64],
        )?;
        let no_memory_embedding = load_expected(
            &artifact_root.join("const_no_mem_embed.npy"),
            &[1, 1, EMBEDDING_DIM],
        )?;
        let no_object_pointer = load_expected(
            &artifact_root.join("const_no_obj_ptr.npy"),
            &[1, EMBEDDING_DIM],
        )?;
        Ok(Self {
            current_positions,
            memory_positions,
            temporal_positions,
            no_memory_embedding,
            no_object_pointer,
        })
    }

    /// Positional input used by backends whose attention graph does not fold it as a constant.
    pub fn current_positions(&self) -> &[f32] {
        &self.current_positions
    }

    #[cfg(test)]
    pub(crate) fn zeros() -> Self {
        Self {
            current_positions: vec![0.0; EMBEDDING_GRID * EMBEDDING_GRID * EMBEDDING_DIM],
            memory_positions: vec![0.0; MEMORY_TOKENS_PER_FRAME * 64],
            temporal_positions: vec![0.0; MEMORY_SLOTS * 64],
            no_memory_embedding: vec![0.0; EMBEDDING_DIM],
            no_object_pointer: vec![0.0; EMBEDDING_DIM],
        }
    }
}

fn load_expected(path: &Path, expected_shape: &[usize]) -> Result<Vec<f32>> {
    let (shape, values) = load_npy_f32(path)?;
    ensure!(
        shape == expected_shape,
        "{} has shape {shape:?}, expected {expected_shape:?}",
        path.display()
    );
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "{} contains NaN or Inf",
        path.display()
    );
    Ok(values)
}

fn load_npy_f32(path: &Path) -> Result<(Vec<usize>, Vec<f32>)> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read EdgeTAM constant {}", path.display()))?;
    ensure!(
        bytes.len() >= 10 && &bytes[..6] == b"\x93NUMPY",
        "{} is not a NumPy file",
        path.display()
    );
    let (header_start, header_len): (usize, usize) = match bytes[6] {
        1 => (10, usize::from(u16::from_le_bytes([bytes[8], bytes[9]]))),
        2 | 3 => {
            ensure!(
                bytes.len() >= 12,
                "{} has a truncated NumPy header",
                path.display()
            );
            (
                12,
                u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize,
            )
        }
        version => anyhow::bail!(
            "{} uses unsupported NumPy version {version}",
            path.display()
        ),
    };
    let data_start = header_start
        .checked_add(header_len)
        .context("NumPy header length overflowed")?;
    ensure!(
        data_start <= bytes.len(),
        "{} has a truncated NumPy header",
        path.display()
    );
    let header = std::str::from_utf8(&bytes[header_start..data_start])?;
    ensure!(
        (header.contains("'<f4'") || header.contains("\"<f4\""))
            && !header.contains("'fortran_order': True")
            && !header.contains("\"fortran_order\": True"),
        "{} must contain C-order little-endian f32 data",
        path.display()
    );
    let shape_marker = header
        .find("'shape':")
        .or_else(|| header.find("\"shape\":"))
        .context("NumPy header is missing shape")?;
    let shape_header = &header[shape_marker..];
    let open = shape_header.find('(').context("NumPy shape is invalid")?;
    let close = shape_header[open + 1..]
        .find(')')
        .map(|offset| offset + open + 1)
        .context("NumPy shape is invalid")?;
    let shape = shape_header[open + 1..close]
        .split(',')
        .filter(|dimension| !dimension.trim().is_empty())
        .map(|dimension| dimension.trim().parse::<usize>())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let values = shape.iter().try_fold(1_usize, |total, dimension| {
        total
            .checked_mul(*dimension)
            .context("NumPy shape overflowed")
    })?;
    let byte_len = values
        .checked_mul(4)
        .context("NumPy byte length overflowed")?;
    ensure!(
        bytes.len() - data_start == byte_len,
        "{} data has {} bytes, shape {shape:?} requires {byte_len}",
        path.display(),
        bytes.len() - data_start
    );
    let values = bytes[data_start..]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    Ok((shape, values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npy_constants_validate_release_shapes_and_reject_corruption() {
        let root = tempfile::tempdir().unwrap();
        let arrays: &[(&str, &[usize])] = &[
            (
                "const_curr_pos.npy",
                &[1, EMBEDDING_GRID * EMBEDDING_GRID, EMBEDDING_DIM],
            ),
            ("const_maskmem_pos.npy", &[1, MEMORY_TOKENS_PER_FRAME, 64]),
            ("const_maskmem_tpos.npy", &[MEMORY_SLOTS, 1, 1, 64]),
            ("const_no_mem_embed.npy", &[1, 1, EMBEDDING_DIM]),
            ("const_no_obj_ptr.npy", &[1, EMBEDDING_DIM]),
        ];
        for (name, shape) in arrays {
            let axes = shape
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let header =
                format!("{{'descr': '<f4', 'fortran_order': False, 'shape': ({axes},), }}\n");
            let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
            bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
            bytes.extend_from_slice(header.as_bytes());
            bytes.resize(bytes.len() + shape.iter().product::<usize>() * 4, 0);
            std::fs::write(root.path().join(name), bytes).unwrap();
        }
        let constants = EdgeTamConstants::load(root.path()).unwrap();
        assert_eq!(
            constants.current_positions.len(),
            EMBEDDING_GRID * EMBEDDING_GRID * EMBEDDING_DIM
        );
        assert_eq!(
            constants.memory_positions.len(),
            MEMORY_TOKENS_PER_FRAME * 64
        );
        assert_eq!(constants.temporal_positions.len(), MEMORY_SLOTS * 64);
        assert_eq!(constants.no_memory_embedding.len(), EMBEDDING_DIM);
        assert_eq!(constants.no_object_pointer.len(), EMBEDDING_DIM);
        std::fs::write(root.path().join("const_no_obj_ptr.npy"), b"truncated").unwrap();
        assert!(EdgeTamConstants::load(root.path()).is_err());
    }
}
