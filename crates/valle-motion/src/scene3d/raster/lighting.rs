//! Generic BRDF lookup and ACES fit. MIT attribution is retained in assets/THREE-LICENSE.txt.
const DFG_BYTES: &[u8; 1024] = include_bytes!("assets/dfg.rg16f");
const DFG: [[f32; 2]; 256] = {
    let mut table = [[0.0; 2]; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = 0;
        while c < 2 {
            let o = i * 4 + c * 2;
            table[i][c] = half(u16::from_le_bytes([DFG_BYTES[o], DFG_BYTES[o + 1]]));
            c += 1;
        }
        i += 1;
    }
    table
};
pub(super) fn dfg(roughness: f32, nv: f32) -> [f32; 2] {
    let x = roughness.clamp(0.0, 1.0) * 16.0 - 0.5;
    let y = nv.clamp(0.0, 1.0) * 16.0 - 0.5;
    let ix = libm::floorf(x) as i32;
    let iy = libm::floorf(y) as i32;
    let tx = x - ix as f32;
    let ty = y - iy as f32;
    let pixel = |x: i32, y: i32| DFG[y.clamp(0, 15) as usize * 16 + x.clamp(0, 15) as usize];
    let a = pixel(ix, iy);
    let b = pixel(ix + 1, iy);
    let c = pixel(ix, iy + 1);
    let d = pixel(ix + 1, iy + 1);
    std::array::from_fn(|i| {
        (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty
    })
}

const fn half(bits: u16) -> f32 {
    let sign = ((bits as u32) & 0x8000) << 16;
    let exp = (bits >> 10) & 31;
    let mant = (bits & 1023) as u32;
    let word = if exp == 0 {
        if mant == 0 {
            sign
        } else {
            let mut m = mant;
            let mut e = 113u32;
            while m & 1024 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 1023) << 13)
        }
    } else if exp == 31 {
        sign | 0x7f800000 | (mant << 13)
    } else {
        sign | (((exp as u32) + 112) << 23) | (mant << 13)
    };
    f32::from_bits(word)
}

pub(super) fn aces(color: [f32; 3], exposure: f32) -> [f32; 3] {
    let matrix = |m: [[f32; 3]; 3], v: [f32; 3]| m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2]);
    let v = matrix(
        [
            [0.59719, 0.35458, 0.04823],
            [0.07600, 0.90834, 0.01566],
            [0.02840, 0.13383, 0.83777],
        ],
        color.map(|v| v * exposure / 0.6),
    );
    let fitted =
        v.map(|v| (v * (v + 0.0245786) - 0.000090537) / (v * (0.983729 * v + 0.432951) + 0.238081));
    matrix(
        [
            [1.60475, -0.53108, -0.07367],
            [-0.10208, 1.10813, -0.00605],
            [-0.00327, -0.07276, 1.07602],
        ],
        fitted,
    )
    .map(|v| v.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brdf_table_and_half_decoding_are_valid() {
        assert_eq!(half(0x3c00), 1.0);
        assert_eq!(half(0xc000), -2.0);
        assert_eq!(half(1), 2.0_f32.powi(-24));
        assert!(
            DFG.iter()
                .flatten()
                .all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0)
        );
    }
}
