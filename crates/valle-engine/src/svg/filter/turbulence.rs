//! SVG 1.1's seeded Perlin tables, shared by all pixels of a turbulence pass.
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct Noise {
    pub frequency: [f64; 2],
    pub octaves: u32,
    pub fractal: bool,
    pub stitch: bool,
    /// 256 permutation entries, followed by four channels of 256 unit gradients.
    pub table: Rc<[f32]>,
}

impl Noise {
    pub fn stitch(&self, tile: [f64; 4]) -> ([f64; 2], [f32; 4]) {
        let mut frequency = self.frequency;
        let mut stitch = [0.; 4];
        if self.stitch {
            for axis in 0..2 {
                let size = tile[axis + 2];
                let f = frequency[axis];
                if size <= 0. || f == 0. {
                    continue;
                }
                let low = (size * f).floor() / size;
                let high = (size * f).ceil() / size;
                frequency[axis] = if f / low < high / f { low } else { high };
                let width = (size * frequency[axis] + 0.5).floor();
                stitch[axis] = width as f32;
                stitch[axis + 2] = (tile[axis] * frequency[axis] + 4096. + width).trunc() as f32;
            }
        }
        (frequency, stitch)
    }
}

fn random(seed: &mut i64) -> i64 {
    *seed = (*seed * 16807) % 2147483647;
    *seed
}

pub fn table(seed: f64) -> Rc<[f32]> {
    let seed = seed.trunc();
    let mut seed = if seed <= 0. {
        -(seed % 2147483646.) + 1.
    } else {
        seed.min(2147483646.)
    } as i64;
    let mut table = vec![0.; 256 + 4 * 256 * 2];
    for k in 0..4 {
        for i in 0..256 {
            table[i] = i as f32;
            let x = (random(&mut seed) % 512 - 256) as f64;
            let y = (random(&mut seed) % 512 - 256) as f64;
            let length = x.hypot(y);
            let at = 256 + (k * 256 + i) * 2;
            if length != 0. {
                table[at] = (x / length) as f32;
                table[at + 1] = (y / length) as f32;
            }
        }
    }
    for i in (1..256).rev() {
        let j = (random(&mut seed) % 256) as usize;
        table.swap(i, j);
    }
    table.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_seed_normalization_and_generator_are_stable() {
        let mut seed = 1;
        for _ in 0..10000 {
            random(&mut seed);
        }
        assert_eq!(seed, 1043618065); // SVG's reference sequence.
        assert_eq!(table(0.), table(1.));
        assert_eq!(table(-3.9), table(4.));
        assert_eq!(table(1e30), table(2147483646.));
        assert_ne!(table(7.), table(8.));
        let t = table(7.);
        let mut permutation = t[..256].iter().map(|n| *n as usize).collect::<Vec<_>>();
        permutation.sort_unstable();
        assert_eq!(permutation, (0..256).collect::<Vec<_>>());
        for &[x, y] in t[256..].as_chunks::<2>().0 {
            assert!((x.hypot(y) - 1.).abs() < 1e-6);
        }
    }

    #[test]
    fn stitching_uses_relative_frequency_distance_and_the_tile_origin() {
        let n = Noise {
            frequency: [0.043, 0.057],
            octaves: 2,
            fractal: true,
            stitch: true,
            table: table(1.),
        };
        let (f, s) = n.stitch([10., 20., 100., 100.]);
        assert_eq!(f, [0.04, 0.06]);
        assert_eq!(s, [4., 6., 4100., 4103.]);
    }
}
