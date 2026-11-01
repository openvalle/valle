use std::collections::BTreeSet;

use super::*;
use crate::text::Blob;

fn face(coords: Vec<i16>) -> Face {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fonts/NotoSans-Variable.ttf"
    );
    Face {
        data: Blob::from(std::fs::read(path).unwrap()),
        index: 0,
        coords,
    }
}

fn glyph(face: &Face, c: char) -> u32 {
    let font = FontRef::from_index(face.data.data(), face.index).unwrap();
    font.charmap().map(c).unwrap().to_u32()
}

fn of(c: char) -> Outline {
    let f = face(Vec::new());
    outline(&f, glyph(&f, c)).unwrap()
}

/// The curve at a texel, relative to `p`.
fn curve(o: &Outline, texel: u32, p: [f32; 2]) -> [[f32; 2]; 3] {
    let [a, b] = [o.curves[texel as usize], o.curves[texel as usize + 1]];
    [
        [a[0] - p[0], a[1] - p[1]],
        [a[2] - p[0], a[3] - p[1]],
        [b[0] - p[0], b[1] - p[1]],
    ]
}

fn list(o: &Outline, band: usize) -> &[u32] {
    let [count, start] = [o.lists[2 * band], o.lists[2 * band + 1]];
    &o.lists[start as usize..(start + count) as usize]
}

/// Which crossings of the line `v = 0` count, from the signs of the three points: bit 0 for the
/// crossing where `v` falls, bit 8 for the one where it rises.
fn crossings(v: [f32; 3]) -> u32 {
    let i = u32::from(v[0] < 0.0) | u32::from(v[1] < 0.0) << 1 | u32::from(v[2] < 0.0) << 2;
    (0x2E74 >> i) & 0x0101
}

/// Where the curve crosses its axis `cut = 0`: the other coordinate at the falling and the
/// rising crossing.
fn roots(c: [[f32; 2]; 3], cut: usize) -> [f32; 2] {
    let o = 1 - cut;
    let a = [0, 1].map(|k| c[0][k] - 2.0 * c[1][k] + c[2][k]);
    let b = [0, 1].map(|k| c[0][k] - c[1][k]);
    let [ay, by, cy] = [a[cut], b[cut], c[0][cut]];
    let d = (by * by - ay * cy).max(0.0).sqrt();
    let fall = if by > 0.0 {
        cy / (by + d)
    } else {
        (by - d) / ay
    };
    let rise = if by < 0.0 {
        cy / (by - d)
    } else {
        (by + d) / ay
    };
    [fall, rise].map(|t| (a[o] * t - 2.0 * b[o]) * t + c[0][o])
}

/// A pixel's coverage at `p`, `ppu` pixels per font unit: the shader's arithmetic.
fn coverage(o: &Outline, p: [f32; 2], ppu: f32) -> f32 {
    let mut cov = [0.0f32; 2];
    let mut weight = [0.0f32; 2];
    // The horizontal ray, then the vertical one.
    for (ray, cut, band) in [
        (0, 1, o.band(1, p[1])),
        (1, 0, o.bands[0] as usize + o.band(0, p[0])),
    ] {
        // Against the horizontal ray a falling curve winds one way; against the vertical one a
        // rising curve does.
        let sign = if ray == 0 { 1.0 } else { -1.0 };
        for &texel in list(o, band) {
            let c = curve(o, texel, p);
            if c[0][ray].max(c[1][ray]).max(c[2][ray]) * ppu < -0.5 {
                break;
            }
            let code = crossings([c[0][cut], c[1][cut], c[2][cut]]);
            let r = roots(c, cut).map(|r| r * ppu);
            if code & 1 != 0 {
                cov[ray] += sign * (r[0] + 0.5).clamp(0.0, 1.0);
                weight[ray] = weight[ray].max((1.0 - r[0].abs() * 2.0).clamp(0.0, 1.0));
            }
            if code > 1 {
                cov[ray] -= sign * (r[1] + 0.5).clamp(0.0, 1.0);
                weight[ray] = weight[ray].max((1.0 - r[1].abs() * 2.0).clamp(0.0, 1.0));
            }
        }
    }
    let mixed = (cov[0] * weight[0] + cov[1] * weight[1]).abs()
        / (weight[0] + weight[1]).max(1.0 / 65536.0);
    mixed.max(cov[0].abs().min(cov[1].abs())).clamp(0.0, 1.0)
}

/// The area inside the outline, by Green's theorem: ∮ x dy over its curves, exact by Simpson's
/// rule since the integrand is cubic.
fn area(o: &Outline) -> f32 {
    let texels: BTreeSet<u32> = (0..o.bands[0] as usize)
        .flat_map(|b| list(o, b).iter().copied())
        .collect();
    let mut sum = 0.0;
    for texel in texels {
        let [p0, p1, p2] = curve(o, texel, [0.0, 0.0]);
        let at = |t: f32| {
            let x = (1.0 - t) * (1.0 - t) * p0[0] + 2.0 * t * (1.0 - t) * p1[0] + t * t * p2[0];
            let dy = 2.0 * (1.0 - t) * (p1[1] - p0[1]) + 2.0 * t * (p2[1] - p1[1]);
            x * dy
        };
        sum += (at(0.0) + 4.0 * at(0.5) + at(1.0)) / 6.0;
    }
    sum.abs()
}

#[test]
fn coverage_adds_up_to_the_area() {
    for c in ['o', 'g', '@', 'M', 'e', '&'] {
        let o = of(c);
        // Pixels of 4 font units: the glyph is about 180 px tall.
        let s = 4.0;
        let [x0, y0, x1, y1] = o.bounds;
        let mut sum = 0.0;
        let mut y = y0 - s;
        while y < y1 + s {
            let mut x = x0 - s;
            while x < x1 + s {
                sum += coverage(&o, [x, y], 1.0 / s) * s * s;
                x += s;
            }
            y += s;
        }
        let a = area(&o);
        assert!((sum - a).abs() < a * 0.005, "{c}: {sum} vs {a}");
    }
}

#[test]
fn edges_cover_what_lies_inside() {
    let o = of('l');
    let [x0, y0, x1, y1] = o.bounds;
    let mid = (y0 + y1) / 2.0;
    // A pixel of 10 font units centered on the stem's left edge, then inside, then outside.
    let ppu = 0.1;
    assert_eq!(coverage(&o, [x0, mid], ppu), 0.5);
    assert_eq!(coverage(&o, [x0 + 2.5, mid], ppu), 0.75);
    assert_eq!(coverage(&o, [(x0 + x1) / 2.0, mid], ppu), 1.0);
    assert_eq!(coverage(&o, [x0 - 5.0, mid], ppu), 0.0);
    assert_eq!(coverage(&o, [x1 + 20.0, mid], ppu), 0.0);
    // The hole of an o is empty.
    let o = of('o');
    let [x0, y0, x1, y1] = o.bounds;
    assert_eq!(coverage(&o, [(x0 + x1) / 2.0, (y0 + y1) / 2.0], 0.1), 0.0);
}

#[test]
fn lines_cross_where_they_are_from_any_pixel() {
    // Pixel centers land anywhere, and moving a line's points to one rounds them: its a is then
    // a stray ulp rather than 0.
    let o = of('V');
    let texels: BTreeSet<u32> = (0..(o.bands[0] + o.bands[1]) as usize)
        .flat_map(|b| list(&o, b).iter().copied())
        .collect();
    let mut lines = 0;
    for texel in texels {
        let c = curve(&o, texel, [0.0, 0.0]);
        if (0..2).any(|k| c[0][k] - 2.0 * c[1][k] + c[2][k] != 0.0) {
            continue;
        }
        lines += 1;
        // The rays along the line never cross it.
        for cut in [0, 1].into_iter().filter(|&k| c[0][k] != c[2][k]) {
            let other = 1 - cut;
            for i in 0..1000 {
                let mut p = [-3.7, -3.7];
                p[cut] = c[0][cut] + (c[2][cut] - c[0][cut]) * (i as f32 + 0.5) / 1000.0;
                let moved = curve(&o, texel, p);
                let code = crossings([moved[0][cut], moved[1][cut], moved[2][cut]]);
                assert_ne!(code, 0, "curve {texel} at {p:?}");
                let s = (p[cut] - c[0][cut]) / (c[2][cut] - c[0][cut]);
                let want = c[0][other] + (c[2][other] - c[0][other]) * s - p[other];
                let r = roots(moved, cut);
                let got = if code & 1 != 0 { r[0] } else { r[1] };
                assert!((got - want).abs() < 0.01, "curve {texel}: {got} vs {want}");
            }
        }
    }
    // Four slanted, three level.
    assert_eq!(lines, 7);
}

#[test]
fn bands_list_every_curve_reaching_them_in_order() {
    let o = of('g');
    assert_eq!(o.bands, [16, 16]);
    // Every curve is in some list, and lists name only curves, never a contour's last texel.
    let all: BTreeSet<u32> = (0..32).flat_map(|b| list(&o, b).iter().copied()).collect();
    assert_eq!(*all.last().unwrap() as usize, o.curves.len() - 2);
    let [x0, y0, x1, y1] = o.bounds;
    for (cut, base, lo, hi) in [(1, 0, y0, y1), (0, 16, x0, x1)] {
        let sort = 1 - cut;
        let step = (hi - lo) / 16.0;
        for band in 0..16 {
            let [from, to] = [lo + step * band as f32, lo + step * (band + 1) as f32];
            let listed = list(&o, base + band);
            let reach = |t: u32| {
                let c = curve(&o, t, [0.0, 0.0]);
                c[0][sort].max(c[1][sort]).max(c[2][sort])
            };
            assert!(listed.windows(2).all(|w| reach(w[0]) >= reach(w[1])));
            for &t in &all {
                let c = curve(&o, t, [0.0, 0.0]);
                let v = [c[0][cut], c[1][cut], c[2][cut]];
                // A curve the ray runs along never crosses it.
                let across = !(v[0] == v[1] && v[1] == v[2]);
                let (min, max) = (v[0].min(v[1]).min(v[2]), v[0].max(v[1]).max(v[2]));
                let reaches = across && max >= from && min <= to;
                let near = across && max >= from - 0.01 && min <= to + 0.01;
                assert!(!reaches || listed.contains(&t), "band {band}: curve {t}");
                assert!(near || !listed.contains(&t), "band {band}: curve {t}");
            }
        }
    }
}

#[test]
fn spaces_have_no_curves_and_weights_change_the_outline() {
    let o = of(' ');
    assert!(o.curves.is_empty() && o.lists.is_empty());
    assert_eq!(o.em, 1000.0);
    let regular = face(Vec::new());
    let font = FontRef::from_index(regular.data.data(), 0).unwrap();
    let black = font.axes().location([("wght", 900.0)]);
    let f = face(black.coords().iter().map(|c| c.to_bits()).collect());
    let bold = outline(&f, glyph(&f, 'l')).unwrap();
    let regular = of('l');
    assert!(bold.bounds[2] - bold.bounds[0] > regular.bounds[2] - regular.bounds[0]);
}
