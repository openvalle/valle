//! Bounded accessor expansion shared by geometry, morphs, animation and skin palettes.
use super::{ARRAY_BUFFER, ContractErrors, ELEMENT_ARRAY_BUFFER, Root, accessor_error};
use crate::scene3d::{MAX_TRIANGLES, MAX_VERTICES};
use std::borrow::Cow;

pub(super) struct AccessorBytes<'a> {
    bytes: Cow<'a, [u8]>,
    stride: usize,
    size: usize,
    count: usize,
}
impl AccessorBytes<'_> {
    pub(super) fn elements(&self) -> impl Iterator<Item = &[u8]> {
        (0..self.count).map(|i| &self.bytes[i * self.stride..i * self.stride + self.size])
    }
}

pub(super) fn accessor_bytes<'a>(
    root: &Root,
    bin: &'a [u8],
    index: usize,
    size: usize,
    component: usize,
    vertex: bool,
) -> Result<AccessorBytes<'a>, ContractErrors> {
    let accessor = &root.accessors[index];
    let count = accessor.count;
    let limit = if vertex {
        MAX_VERTICES as usize
    } else {
        MAX_TRIANGLES as usize * 3
    };
    if count == 0 || count > limit {
        return accessor_error(
            index,
            "accessor count must be positive and within the geometry budget",
        );
    }
    let mut result = if let Some(view_index) = accessor.buffer_view {
        let Some(view) = root.buffer_views.get(view_index) else {
            return accessor_error(index, "bufferView index is out of range");
        };
        if view.target.is_some_and(|t| {
            t != if vertex {
                ARRAY_BUFFER
            } else {
                ELEMENT_ARRAY_BUFFER
            }
        }) || (!vertex && view.byte_stride.is_some())
        {
            return accessor_error(index, "buffer view target or stride does not match its use");
        }
        let stride = view.byte_stride.unwrap_or(size);
        if stride < size
            || stride % component != 0
            || (vertex && (!accessor.byte_offset.is_multiple_of(4) || !stride.is_multiple_of(4)))
        {
            return accessor_error(index, "accessor is misaligned or exceeds its buffer view");
        }
        let bytes = view_bytes(
            root,
            bin,
            index,
            view_index,
            accessor.byte_offset,
            count,
            stride,
            size,
            component,
            false,
        )?;
        AccessorBytes {
            bytes: Cow::Borrowed(bytes),
            stride,
            size,
            count,
        }
    } else {
        if accessor.byte_offset != 0 {
            return accessor_error(
                index,
                "an accessor without a bufferView must have zero byteOffset",
            );
        }
        AccessorBytes {
            bytes: Cow::Owned(vec![0; count * size]),
            stride: size,
            size,
            count,
        }
    };
    if let Some(sparse) = &accessor.sparse {
        if sparse.count == 0 || sparse.count > count {
            return accessor_error(
                index,
                "sparse count must be positive and no larger than accessor count",
            );
        }
        let index_size = match sparse.indices.component_type {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            _ => {
                return accessor_error(index, "sparse indices require unsigned byte, short or int");
            }
        };
        let indices = view_bytes(
            root,
            bin,
            index,
            sparse.indices.buffer_view,
            sparse.indices.byte_offset,
            sparse.count,
            index_size,
            index_size,
            index_size,
            true,
        )?;
        let value_stride = if vertex { size.div_ceil(4) * 4 } else { size };
        if vertex && !sparse.values.byte_offset.is_multiple_of(4) {
            return accessor_error(index, "sparse vertex values must be four-byte aligned");
        }
        let values = view_bytes(
            root,
            bin,
            index,
            sparse.values.buffer_view,
            sparse.values.byte_offset,
            sparse.count,
            value_stride,
            size,
            component,
            true,
        )?;
        let mut dense = Vec::with_capacity(count * size);
        for element in result.elements() {
            dense.extend_from_slice(element);
        }
        let mut previous = None;
        for (n, bytes) in indices.chunks_exact(index_size).enumerate() {
            let target = match index_size {
                1 => bytes[0] as usize,
                2 => u16::from_le_bytes(bytes.try_into().unwrap()) as usize,
                _ => u32::from_le_bytes(bytes.try_into().unwrap()) as usize,
            };
            if target >= count || previous.is_some_and(|p| target <= p) {
                return accessor_error(
                    index,
                    "sparse indices must strictly increase and stay within accessor count",
                );
            }
            dense[target * size..(target + 1) * size]
                .copy_from_slice(&values[n * value_stride..n * value_stride + size]);
            previous = Some(target);
        }
        result.bytes = Cow::Owned(dense);
        result.stride = size;
    }
    Ok(result)
}

#[expect(
    clippy::too_many_arguments,
    reason = "One checked byte range for dense and sparse accessor layouts"
)]
fn view_bytes<'a>(
    root: &Root,
    bin: &'a [u8],
    index: usize,
    view_index: usize,
    offset: usize,
    count: usize,
    stride: usize,
    size: usize,
    component: usize,
    sparse: bool,
) -> Result<&'a [u8], ContractErrors> {
    let Some(view) = root.buffer_views.get(view_index) else {
        return accessor_error(index, "bufferView index is out of range");
    };
    if sparse && (view.target.is_some() || view.byte_stride.is_some()) {
        return accessor_error(
            index,
            "sparse buffer views cannot have target or byteStride",
        );
    }
    let start = view.byte_offset.checked_add(offset);
    let end = (count - 1)
        .checked_mul(stride)
        .and_then(|v| v.checked_add(size))
        .and_then(|v| start?.checked_add(v));
    let view_end = view.byte_offset.checked_add(view.byte_length);
    if !offset.is_multiple_of(component)
        || start.is_none_or(|s| !s.is_multiple_of(component))
        || end.is_none_or(|e| e > view_end.unwrap_or(0) || e > bin.len())
    {
        return accessor_error(index, "accessor is misaligned or exceeds its buffer view");
    }
    Ok(&bin[start.unwrap()..end.unwrap()])
}
