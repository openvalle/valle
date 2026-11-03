//! A decoded video frame's planes into straight sRGB RGBA, as the native engine's FFmpeg converts
//! them: by the matrix and range the file tags, its values taken as sRGB's, chroma interpolated
//! bilinearly from the middle of the pixels it covers. The browser's own conversion to RGBA loses
//! precision in blue.

/// How a frame's planes hold its samples, 8 bits each: those WebCodecs decodes 8-bit 4:2:0 into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    /// Y, U and V planes, U and V of half the width and height.
    I420,
    /// I420 with an alpha plane of full size.
    I420a,
    /// A Y plane, and U and V interleaved in one of half the width and height.
    Nv12,
}

/// A frame's planes in `bytes`: each one's offset and bytes a row.
pub struct Planes<'a> {
    pub format: Format,
    pub size: [u32; 2],
    pub planes: &'a [(usize, usize)],
    pub bytes: &'a [u8],
    /// BT.709's matrix, else BT.601's.
    pub bt709: bool,
    /// Full range, else limited (16–235, 16–240).
    pub full: bool,
}

/// The frame as straight RGBA, 4 bytes a pixel, rows top down.
pub fn rgba(p: &Planes) -> Result<Vec<u8>, String> {
    let [w, h] = p.size.map(|n| n as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let wanted = match p.format {
        Format::Nv12 => 2,
        Format::I420 => 3,
        Format::I420a => 4,
    };
    if p.planes.len() != wanted {
        return Err(format!(
            "{:?} has {wanted} planes, not {}",
            p.format,
            p.planes.len()
        ));
    }
    // Each plane's rows and bytes a sample, which must lie in `bytes`.
    let shape = |i: usize| -> (usize, usize, usize) {
        match (p.format, i) {
            (_, 0) | (Format::I420a, 3) => (w, h, 1),
            (Format::Nv12, _) => (cw, ch, 2),
            _ => (cw, ch, 1),
        }
    };
    for (i, &(offset, stride)) in p.planes.iter().enumerate() {
        let (width, rows, step) = shape(i);
        let end = offset + stride * (rows - 1) + width * step;
        if stride < width * step || end > p.bytes.len() {
            return Err(format!(
                "plane {i} does not fit its {} bytes",
                p.bytes.len()
            ));
        }
    }
    let sample = |i: usize, x: usize, y: usize, at: usize| {
        let (offset, stride) = p.planes[i];
        p.bytes[offset + y * stride + x * shape(i).2 + at]
    };
    // Y' = (Y − y0) × ys, C' = (C − 128) × cs.
    let (y0, ys, cs) = match p.full {
        true => (0.0, 1.0, 1.0),
        false => (16.0, 255.0 / 219.0, 255.0 / 224.0),
    };
    let (kr, kb) = if p.bt709 {
        (0.2126, 0.0722)
    } else {
        (0.299, 0.114)
    };
    let kg = 1.0 - kr - kb;
    let r_v = 2.0 * (1.0 - kr);
    let b_u = 2.0 * (1.0 - kb);
    let g_u = -b_u * kb / kg;
    let g_v = -r_v * kr / kg;
    let to = |x: f32| (x.round()).clamp(0.0, 255.0) as u8;
    // Cb' and Cr' of each chroma sample.
    let chroma = |which: usize| -> Vec<f32> {
        let (i, at) = match p.format {
            Format::Nv12 => (1, which),
            _ => (1 + which, 0),
        };
        (0..cw * ch)
            .map(|k| (f32::from(sample(i, k % cw, k / cw, at)) - 128.0) * cs)
            .collect()
    };
    let (cb, cr) = (chroma(0), chroma(1));

    // Where each column's and row's chroma lies between two samples, and how near the second.
    let between = |i: usize, n: usize| {
        let at = ((i as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (n - 1) as f32);
        let first = at.floor() as usize;
        (first, (first + 1).min(n - 1), at - at.floor())
    };
    let columns: Vec<(usize, usize, f32)> = (0..w).map(|x| between(x, cw)).collect();
    let mut out = vec![0; w * h * 4];
    for y in 0..h {
        let (r0, r1, ty) = between(y, ch);
        let row = &mut out[y * w * 4..(y + 1) * w * 4];
        for (x, &(x0, x1, tx)) in columns.iter().enumerate() {
            let mix = |c: &[f32]| {
                let top = c[r0 * cw + x0] * (1.0 - tx) + c[r0 * cw + x1] * tx;
                let bottom = c[r1 * cw + x0] * (1.0 - tx) + c[r1 * cw + x1] * tx;
                top * (1.0 - ty) + bottom * ty
            };
            let (cb, cr) = (mix(&cb), mix(&cr));
            let l = (f32::from(sample(0, x, y, 0)) - y0) * ys;
            let px = &mut row[x * 4..x * 4 + 4];
            px[0] = to(l + r_v * cr);
            px[1] = to(l + g_u * cb + g_v * cr);
            px[2] = to(l + b_u * cb);
            px[3] = if p.format == Format::I420a {
                sample(3, x, y, 0)
            } else {
                255
            };
        }
    }
    Ok(out)
}

/// Straight RGBA of `size` as an export encodes it, as the native engine converts it: I420, BT.709's
/// matrix, limited range, each chroma sample the mean of the pixels it covers; alpha dropped.
pub fn i420(rgba: &[u8], size: [u32; 2]) -> Vec<u8> {
    let [w, h] = size.map(|n| n as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let (kr, kb) = (0.2126f32, 0.0722f32);
    let kg = 1.0 - kr - kb;
    let to = |x: f32| x.round().clamp(0.0, 255.0) as u8;
    let mut out = vec![0; w * h + 2 * cw * ch];
    let (luma, chroma) = out.split_at_mut(w * h);
    let (u, v) = chroma.split_at_mut(cw * ch);
    let pixel = |x: usize, y: usize| {
        let p = &rgba[(y * w + x) * 4..];
        [p[0], p[1], p[2]].map(f32::from)
    };
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = pixel(x, y);
            luma[y * w + x] = to(16.0 + 219.0 / 255.0 * (kr * r + kg * g + kb * b));
        }
    }
    for j in 0..ch {
        for i in 0..cw {
            let (mut cb, mut cr, mut n) = (0.0, 0.0, 0.0);
            for y in 2 * j..(2 * j + 2).min(h) {
                for x in 2 * i..(2 * i + 2).min(w) {
                    let [r, g, b] = pixel(x, y);
                    let l = kr * r + kg * g + kb * b;
                    cb += (b - l) / (2.0 * (1.0 - kb));
                    cr += (r - l) / (2.0 * (1.0 - kr));
                    n += 1.0;
                }
            }
            u[j * cw + i] = to(128.0 + 224.0 / 255.0 * cb / n);
            v[j * cw + i] = to(128.0 + 224.0 / 255.0 * cr / n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of one color, as I420 planes back to back.
    fn flat(size: [u32; 2], [y, u, v]: [u8; 3]) -> (Vec<u8>, Vec<(usize, usize)>) {
        let [w, h] = size.map(|n| n as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let mut bytes = vec![y; w * h];
        bytes.extend(vec![u; cw * ch]);
        bytes.extend(vec![v; cw * ch]);
        (bytes, vec![(0, w), (w * h, cw), (w * h + cw * ch, cw)])
    }

    /// The first pixel of a 3×3 frame of one color, BT.709 limited.
    fn bt709(yuv: [u8; 3]) -> [u8; 4] {
        let (bytes, planes) = flat([3, 3], yuv);
        let out = rgba(&Planes {
            format: Format::I420,
            size: [3, 3],
            planes: &planes,
            bytes: &bytes,
            bt709: true,
            full: false,
        })
        .expect("converted");
        assert_eq!(out.len(), 3 * 3 * 4);
        out[..4].try_into().expect("a pixel")
    }

    #[test]
    fn limited_range_spans_the_whole_range() {
        assert_eq!(bt709([235, 128, 128]), [255, 255, 255, 255]);
        assert_eq!(bt709([16, 128, 128]), [0, 0, 0, 255]);
        // Blue, Y 32 Cb 240 Cr 118, is all blue: the browser's own conversion gives 243.
        let [r, g, b, _] = bt709([32, 240, 118]);
        assert!(r <= 1 && g == 0 && b == 255, "{r} {g} {b}");
    }

    #[test]
    fn colors_go_to_yuv_and_back() {
        let colors = [
            [255, 255, 255],
            [0, 0, 0],
            [0, 0, 255],
            [200, 120, 40],
            [12, 200, 90],
        ];
        for c in colors {
            let rgba: Vec<u8> = (0..9).flat_map(|_| [c[0], c[1], c[2], 255]).collect();
            let yuv = i420(&rgba, [3, 3]);
            let planes = [(0, 3), (9, 2), (13, 2)];
            let back = rgba_of(&yuv, &planes, [3, 3]);
            for k in 0..3 {
                let d = (i16::from(back[k]) - i16::from(c[k])).abs();
                assert!(d <= 1, "{c:?} came back as {:?}", &back[..3]);
            }
        }
    }

    fn rgba_of(bytes: &[u8], planes: &[(usize, usize)], size: [u32; 2]) -> Vec<u8> {
        rgba(&Planes {
            format: Format::I420,
            size,
            planes,
            bytes,
            bt709: true,
            full: false,
        })
        .expect("converted")
    }

    #[test]
    fn nv12_matches_i420() {
        let (bytes, planes) = flat([4, 2], [120, 90, 200]);
        let i420 = rgba(&Planes {
            format: Format::I420,
            size: [4, 2],
            planes: &planes,
            bytes: &bytes,
            bt709: false,
            full: false,
        })
        .expect("converted");
        let mut nv12 = vec![120; 8];
        nv12.extend([90, 200, 90, 200]);
        let nv = rgba(&Planes {
            format: Format::Nv12,
            size: [4, 2],
            planes: &[(0, 4), (8, 4)],
            bytes: &nv12,
            bt709: false,
            full: false,
        })
        .expect("converted");
        assert_eq!(i420, nv);
    }

    #[test]
    fn planes_out_of_their_bytes_are_refused() {
        let (bytes, planes) = flat([4, 4], [16, 128, 128]);
        let short = rgba(&Planes {
            format: Format::I420,
            size: [4, 4],
            planes: &planes,
            bytes: &bytes[..bytes.len() - 1],
            bt709: true,
            full: false,
        });
        assert!(short.is_err());
    }
}
