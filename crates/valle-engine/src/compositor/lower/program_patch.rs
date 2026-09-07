//! Canonical random-access byte patches for exact per-frame DrawPrograms.
//!
//! A RenderPlan template owns one admitted baseline program. Bindings carry only a deterministic
//! copy/literal patch against that baseline. The patch is deliberately simple and independently
//! bounded: reconstructing it never consults previous frames, so random seek, cache eviction and
//! worker count cannot alter output.

use std::collections::HashMap;

use thiserror::Error;

const MAGIC: &[u8; 8] = b"VLDPAT\0\0";
const HEADER_BYTES: usize = 16;
const MATCH_KEY_BYTES: usize = 16;
const MIN_COPY_BYTES: usize = 16;
const MAX_CANDIDATES: usize = 4;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const ALIGNED_FAST_PATH_BYTES: usize = 64 * 1024;
const COPY: u8 = 0;
const INSERT: u8 = 1;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProgramPatchError {
    #[error("program patch input exceeds the 64 MiB budget")]
    ByteBudget,
    #[error("program patch is truncated")]
    Truncated,
    #[error("program patch has the wrong magic")]
    BadMagic,
    #[error("program patch instruction is invalid")]
    Instruction,
    #[error("program patch output length mismatch")]
    OutputLength,
}

#[derive(Debug)]
enum Instruction {
    Copy { offset: u32, length: u32 },
    Insert(Vec<u8>),
}

pub fn encode(base: &[u8], target: &[u8]) -> Result<Vec<u8>, ProgramPatchError> {
    check_budget(base.len())?;
    check_budget(target.len())?;
    if base == target && !target.is_empty() {
        return encode_instructions(
            target.len(),
            vec![Instruction::Copy {
                offset: 0,
                length: u32::try_from(target.len()).map_err(|_| ProgramPatchError::ByteBudget)?,
            }],
        );
    }
    // Dense typed tables keep their physical length and offsets stable while most scalar values
    // change. Building a HashMap with tens of thousands of keys for that shape costs more than
    // the patch itself, especially in WASM. An aligned scan is exact and allocation-light.
    if base.len() == target.len() && target.len() >= ALIGNED_FAST_PATH_BYTES {
        return encode_aligned(base, target);
    }
    let mut positions = HashMap::<u128, Vec<u32>>::new();
    if base.len() >= MATCH_KEY_BYTES {
        // Index canonical blocks, not every overlapping byte. The target still probes every byte,
        // so inserted dynamic JSON can shift alignment without losing the next long static run;
        // the smaller table avoids hundreds of thousands of WASM HashMap allocations per frame.
        for offset in (0..=base.len() - MATCH_KEY_BYTES).step_by(MATCH_KEY_BYTES) {
            let key = key_at(base, offset);
            let candidates = positions.entry(key).or_default();
            if candidates.len() < MAX_CANDIDATES {
                candidates.push(u32::try_from(offset).map_err(|_| ProgramPatchError::ByteBudget)?);
            } else {
                // Keep both early and recent occurrences. Repeated JSON field names otherwise
                // crowd out the nearby structural match that tends to extend the farthest.
                candidates.rotate_left(1);
                *candidates.last_mut().expect("non-empty candidate list") =
                    u32::try_from(offset).map_err(|_| ProgramPatchError::ByteBudget)?;
            }
        }
    }

    let mut instructions = Vec::new();
    let mut literal = Vec::new();
    let mut target_offset = 0usize;
    while target_offset < target.len() {
        let mut best = None::<(usize, usize)>;
        if target_offset + MATCH_KEY_BYTES <= target.len()
            && let Some(candidates) = positions.get(&key_at(target, target_offset))
        {
            for candidate in candidates {
                let base_offset = *candidate as usize;
                let mut length = MATCH_KEY_BYTES;
                while base_offset + length < base.len()
                    && target_offset + length < target.len()
                    && base[base_offset + length] == target[target_offset + length]
                {
                    length += 1;
                }
                if length >= MIN_COPY_BYTES
                    && best.is_none_or(|(_, best_length)| length > best_length)
                {
                    best = Some((base_offset, length));
                }
            }
        }
        if let Some((base_offset, length)) = best {
            flush_literal(&mut literal, &mut instructions)?;
            push_copy(base_offset, length, &mut instructions)?;
            target_offset += length;
        } else {
            literal.push(target[target_offset]);
            target_offset += 1;
        }
    }
    flush_literal(&mut literal, &mut instructions)?;

    encode_instructions(target.len(), instructions)
}

fn encode_aligned(base: &[u8], target: &[u8]) -> Result<Vec<u8>, ProgramPatchError> {
    debug_assert_eq!(base.len(), target.len());
    // Dense DrawProgram tables can contain thousands of alternating scalar runs. Building one
    // `Instruction::Insert(Vec<_>)` allocation per run made patch construction dominate Native
    // frame lowering even though the wire format itself is already streaming. Write that format
    // directly and backfill only the instruction count.
    let mut patch = Vec::with_capacity(target.len().min(MAX_BYTES));
    patch.extend_from_slice(MAGIC);
    patch.extend_from_slice(
        &u32::try_from(target.len())
            .map_err(|_| ProgramPatchError::ByteBudget)?
            .to_le_bytes(),
    );
    patch.extend_from_slice(&0_u32.to_le_bytes());
    let mut instruction_count = 0_u32;
    let mut literal_start = None::<usize>;
    let mut offset = 0usize;
    while offset < target.len() {
        if base[offset] != target[offset] {
            literal_start.get_or_insert(offset);
            offset += 1;
            continue;
        }
        let start = offset;
        while offset < target.len() && base[offset] == target[offset] {
            offset += 1;
        }
        let length = offset - start;
        if length >= MIN_COPY_BYTES {
            if let Some(literal_start) = literal_start.take() {
                push_encoded_insert(
                    &mut patch,
                    &target[literal_start..start],
                    &mut instruction_count,
                )?;
            }
            push_encoded_copy(&mut patch, start, length, &mut instruction_count)?;
        } else if literal_start.is_none() {
            literal_start = Some(start);
        }
    }
    if let Some(literal_start) = literal_start {
        push_encoded_insert(&mut patch, &target[literal_start..], &mut instruction_count)?;
    }
    patch[12..16].copy_from_slice(&instruction_count.to_le_bytes());
    Ok(patch)
}

fn push_encoded_copy(
    patch: &mut Vec<u8>,
    offset: usize,
    length: usize,
    instruction_count: &mut u32,
) -> Result<(), ProgramPatchError> {
    patch.push(COPY);
    patch.extend_from_slice(
        &u32::try_from(offset)
            .map_err(|_| ProgramPatchError::ByteBudget)?
            .to_le_bytes(),
    );
    patch.extend_from_slice(
        &u32::try_from(length)
            .map_err(|_| ProgramPatchError::ByteBudget)?
            .to_le_bytes(),
    );
    *instruction_count = instruction_count
        .checked_add(1)
        .ok_or(ProgramPatchError::ByteBudget)?;
    Ok(())
}

fn push_encoded_insert(
    patch: &mut Vec<u8>,
    bytes: &[u8],
    instruction_count: &mut u32,
) -> Result<(), ProgramPatchError> {
    if bytes.is_empty() {
        return Ok(());
    }
    patch.push(INSERT);
    patch.extend_from_slice(
        &u32::try_from(bytes.len())
            .map_err(|_| ProgramPatchError::ByteBudget)?
            .to_le_bytes(),
    );
    patch.extend_from_slice(bytes);
    *instruction_count = instruction_count
        .checked_add(1)
        .ok_or(ProgramPatchError::ByteBudget)?;
    Ok(())
}

fn encode_instructions(
    target_len: usize,
    instructions: Vec<Instruction>,
) -> Result<Vec<u8>, ProgramPatchError> {
    let mut patch = Vec::new();
    patch.extend_from_slice(MAGIC);
    patch.extend_from_slice(
        &u32::try_from(target_len)
            .map_err(|_| ProgramPatchError::ByteBudget)?
            .to_le_bytes(),
    );
    patch.extend_from_slice(
        &u32::try_from(instructions.len())
            .map_err(|_| ProgramPatchError::ByteBudget)?
            .to_le_bytes(),
    );
    for instruction in instructions {
        match instruction {
            Instruction::Copy { offset, length } => {
                patch.push(COPY);
                patch.extend_from_slice(&offset.to_le_bytes());
                patch.extend_from_slice(&length.to_le_bytes());
            }
            Instruction::Insert(bytes) => {
                patch.push(INSERT);
                patch.extend_from_slice(
                    &u32::try_from(bytes.len())
                        .map_err(|_| ProgramPatchError::ByteBudget)?
                        .to_le_bytes(),
                );
                patch.extend_from_slice(&bytes);
            }
        }
    }
    check_budget(patch.len())?;
    Ok(patch)
}

pub fn apply(base: &[u8], patch: &[u8]) -> Result<Vec<u8>, ProgramPatchError> {
    check_budget(base.len())?;
    check_budget(patch.len())?;
    if patch.len() < HEADER_BYTES {
        return Err(ProgramPatchError::Truncated);
    }
    if &patch[..8] != MAGIC {
        return Err(ProgramPatchError::BadMagic);
    }
    let target_len = u32_at(patch, 8) as usize;
    let instruction_count = u32_at(patch, 12) as usize;
    check_budget(target_len)?;
    let mut output = Vec::with_capacity(target_len);
    let mut cursor = HEADER_BYTES;
    for _ in 0..instruction_count {
        let tag = *patch.get(cursor).ok_or(ProgramPatchError::Truncated)?;
        cursor += 1;
        match tag {
            COPY => {
                let offset = take_u32(patch, &mut cursor)? as usize;
                let length = take_u32(patch, &mut cursor)? as usize;
                let end = offset
                    .checked_add(length)
                    .filter(|end| *end <= base.len())
                    .ok_or(ProgramPatchError::Instruction)?;
                if length == 0 {
                    return Err(ProgramPatchError::Instruction);
                }
                output.extend_from_slice(&base[offset..end]);
            }
            INSERT => {
                let length = take_u32(patch, &mut cursor)? as usize;
                let end = cursor
                    .checked_add(length)
                    .filter(|end| *end <= patch.len())
                    .ok_or(ProgramPatchError::Truncated)?;
                if length == 0 {
                    return Err(ProgramPatchError::Instruction);
                }
                output.extend_from_slice(&patch[cursor..end]);
                cursor = end;
            }
            _ => return Err(ProgramPatchError::Instruction),
        }
        if output.len() > target_len {
            return Err(ProgramPatchError::OutputLength);
        }
    }
    if cursor != patch.len() || output.len() != target_len {
        return Err(ProgramPatchError::OutputLength);
    }
    Ok(output)
}

pub fn validate(patch: &[u8]) -> Result<(), ProgramPatchError> {
    if patch.len() < HEADER_BYTES {
        return Err(ProgramPatchError::Truncated);
    }
    if &patch[..8] != MAGIC {
        return Err(ProgramPatchError::BadMagic);
    }
    check_budget(patch.len())?;
    let target_len = u32_at(patch, 8) as usize;
    let instruction_count = u32_at(patch, 12) as usize;
    check_budget(target_len)?;
    if instruction_count > patch.len().saturating_sub(HEADER_BYTES) / 5 {
        return Err(ProgramPatchError::Truncated);
    }
    let mut cursor = HEADER_BYTES;
    let mut produced = 0usize;
    for _ in 0..instruction_count {
        let tag = *patch.get(cursor).ok_or(ProgramPatchError::Truncated)?;
        cursor += 1;
        let length = match tag {
            COPY => {
                let _offset = take_u32(patch, &mut cursor)?;
                take_u32(patch, &mut cursor)? as usize
            }
            INSERT => {
                let length = take_u32(patch, &mut cursor)? as usize;
                let end = cursor
                    .checked_add(length)
                    .filter(|end| *end <= patch.len())
                    .ok_or(ProgramPatchError::Truncated)?;
                cursor = end;
                length
            }
            _ => return Err(ProgramPatchError::Instruction),
        };
        if length == 0 {
            return Err(ProgramPatchError::Instruction);
        }
        produced = produced
            .checked_add(length)
            .filter(|produced| *produced <= target_len)
            .ok_or(ProgramPatchError::OutputLength)?;
    }
    if cursor != patch.len() || produced != target_len {
        return Err(ProgramPatchError::OutputLength);
    }
    Ok(())
}

fn push_copy(
    offset: usize,
    length: usize,
    instructions: &mut Vec<Instruction>,
) -> Result<(), ProgramPatchError> {
    if let Some(Instruction::Copy {
        offset: previous_offset,
        length: previous_length,
    }) = instructions.last_mut()
        && usize::try_from(*previous_offset).expect("u32 fits usize")
            + usize::try_from(*previous_length).expect("u32 fits usize")
            == offset
    {
        *previous_length = previous_length
            .checked_add(u32::try_from(length).map_err(|_| ProgramPatchError::ByteBudget)?)
            .ok_or(ProgramPatchError::ByteBudget)?;
        return Ok(());
    }
    instructions.push(Instruction::Copy {
        offset: u32::try_from(offset).map_err(|_| ProgramPatchError::ByteBudget)?,
        length: u32::try_from(length).map_err(|_| ProgramPatchError::ByteBudget)?,
    });
    Ok(())
}

fn flush_literal(
    literal: &mut Vec<u8>,
    instructions: &mut Vec<Instruction>,
) -> Result<(), ProgramPatchError> {
    if literal.is_empty() {
        return Ok(());
    }
    check_budget(literal.len())?;
    instructions.push(Instruction::Insert(std::mem::take(literal)));
    Ok(())
}

fn key_at(bytes: &[u8], offset: usize) -> u128 {
    u128::from_le_bytes(
        bytes[offset..offset + MATCH_KEY_BYTES]
            .try_into()
            .expect("caller checked the match key range"),
    )
}

fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, ProgramPatchError> {
    let end = cursor.checked_add(4).ok_or(ProgramPatchError::Truncated)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(ProgramPatchError::Truncated)?
        .try_into()
        .expect("four-byte slice");
    *cursor = end;
    Ok(u32::from_le_bytes(value))
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated header"),
    )
}

fn check_budget(bytes: usize) -> Result<(), ProgramPatchError> {
    (bytes <= MAX_BYTES)
        .then_some(())
        .ok_or(ProgramPatchError::ByteBudget)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_header_contains_only_magic_and_binding_local_lengths() {
        let patch = encode(b"", b"frame").unwrap();
        assert_eq!(&patch[..8], MAGIC);
        assert_eq!(u32_at(&patch, 8), 5);
        assert_eq!(u32_at(&patch, 12), 1);
        assert_eq!(apply(b"", &patch).unwrap(), b"frame");
    }

    #[test]
    fn exact_random_access_patch_roundtrips_shifted_content() {
        let base =
            b"prefix:[{\"x\":1234,\"name\":\"alpha\"},{\"x\":5678,\"name\":\"beta\"}]:suffix";
        let target = b"prefix:[{\"x\":9,\"name\":\"alpha\"},{\"x\":5678,\"name\":\"beta\"},{\"x\":42}]:suffix";
        let patch = encode(base, target).unwrap();
        validate(&patch).unwrap();
        assert_eq!(apply(base, &patch).unwrap(), target);

        // A patch is self-contained against the admitted baseline; reconstructing a later frame
        // never depends on applying the earlier frame first. Small payloads may be shorter than
        // the fixed patch header, so compactness belongs to the product benchmark, not this
        // correctness contract.
        let later = b"prefix:[{\"x\":11,\"name\":\"alpha\"}]:suffix";
        let later_patch = encode(base, later).unwrap();
        assert_eq!(apply(base, &later_patch).unwrap(), later);
    }

    #[test]
    fn malformed_copy_cannot_escape_the_baseline() {
        let mut patch = encode(b"abcdefghijklmnop", b"abcdefghijklmnop").unwrap();
        let copy_offset = HEADER_BYTES + 1;
        patch[copy_offset..copy_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            apply(b"abcdefghijklmnop", &patch),
            Err(ProgramPatchError::Instruction)
        );
    }

    #[test]
    fn aligned_dense_payload_keeps_static_runs_without_a_hash_index() {
        let mut base = vec![0x11; ALIGNED_FAST_PATH_BYTES + 4096];
        let mut target = base.clone();
        for (index, byte) in target[1024..ALIGNED_FAST_PATH_BYTES].iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(31);
        }
        // Model the mutable packed checksum as well as the dense instance table.
        base[28..60].fill(0x22);
        target[28..60].fill(0x33);
        let patch = encode(&base, &target).unwrap();
        assert_eq!(apply(&base, &patch).unwrap(), target);
        assert!(patch.len() < target.len());
    }
}
