//! Deterministic admission of project-owned panoramic lighting. Runtime sampling reads only
//! frozen linear-sRGB cube faces; no decoder, preprocessing, clock or random state is involved.
use std::io::Cursor;

use serde::{Deserialize, Serialize};

use super::ContractErrors;
use crate::ContentDigest;

const MAGIC: &[u8; 8] = b"VALLEENV";
const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_STORAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RADIANCE: f32 = 65_504.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentSettings {
    pub face_size: u32,
    pub diffuse_size: u32,
    pub samples: u32,
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self {
            face_size: 64,
            diffuse_size: 8,
            samples: 64,
        }
    }
}

impl EnvironmentSettings {
    fn validate(self) -> Result<(), ContractErrors> {
        if !self.face_size.is_power_of_two()
            || !(4..=256).contains(&self.face_size)
            || !self.diffuse_size.is_power_of_two()
            || self.diffuse_size > self.face_size.min(32)
            || !self.samples.is_power_of_two()
            || !(16..=256).contains(&self.samples)
        {
            return Err(error(
                "settings require power-of-two faceSize 4–256, diffuseSize 1–32 (<= faceSize), and samples 16–256",
            ));
        }
        Ok(())
    }

    fn levels(self) -> impl Iterator<Item = u32> {
        (0..=self.face_size.ilog2()).map(move |level| self.face_size >> level)
    }

    fn pixels(self) -> usize {
        6 * (self.diffuse_size * self.diffuse_size + self.levels().map(|n| n * n).sum::<u32>())
            as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Header {
    source_digest: ContentDigest,
    source_size: [u32; 2],
    settings: EnvironmentSettings,
}

#[derive(Clone, Debug, PartialEq)]
struct Cube {
    size: u32,
    // Face order +X, -X, +Y, -Y, +Z, -Z; each face is row-major, top row first.
    pixels: Vec<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentAsset {
    header: Header,
    content_digest: ContentDigest,
    diffuse: Cube,
    specular: Vec<Cube>,
}

impl EnvironmentAsset {
    pub fn descriptor(
        &self,
    ) -> valle_timeline::internal::wire::resource::EnvironmentResourceDescriptorWire {
        valle_timeline::internal::wire::resource::EnvironmentResourceDescriptorWire {
            byte_length: (12
                + crate::canonical_bytes(&self.header)
                    .expect("validated environment metadata")
                    .len()
                + self.storage_bytes() as usize) as u32,
            source_size: self.source_size(),
            face_size: self.settings().face_size,
            diffuse_size: self.settings().diffuse_size,
            samples: self.settings().samples,
        }
    }
    /// PNG/JPEG RGB is explicitly sRGB; HDR RGB is linear sRGB. Alpha is not a lighting channel.
    pub fn from_encoded(
        bytes: &[u8],
        settings: EnvironmentSettings,
    ) -> Result<Self, ContractErrors> {
        settings.validate()?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(error("encoded panorama exceeds 64 MiB"));
        }
        let format = image::guess_format(bytes).map_err(|e| error(e.to_string()))?;
        if !matches!(
            format,
            image::ImageFormat::Hdr | image::ImageFormat::Png | image::ImageFormat::Jpeg
        ) {
            return Err(error("panorama must be HDR, PNG or JPEG"));
        }
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(2048);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|e| error(e.to_string()))?
            .into_rgb32f();
        let source_size = [decoded.width(), decoded.height()];
        validate_source_size(source_size)?;
        let panorama = decoded
            .pixels()
            .map(|pixel| {
                if format == image::ImageFormat::Hdr {
                    pixel.0
                } else {
                    pixel.0.map(super::asset::srgb_to_linear)
                }
            })
            .collect::<Vec<_>>();
        if !valid_pixels(&panorama) {
            return Err(error("panorama RGB must be finite and within 0–65504"));
        }
        let sample = |direction| sample_panorama(&panorama, source_size, direction);
        let diffuse = Cube::generate(settings.diffuse_size, |n| {
            integrate(n, settings.samples, None, &sample)
        });
        let last = settings.face_size.ilog2() as f32;
        let specular = settings
            .levels()
            .enumerate()
            .map(|(level, size)| {
                Cube::generate(size, |n| {
                    if level == 0 {
                        sample(n)
                    } else {
                        integrate(n, settings.samples, Some(level as f32 / last), &sample)
                    }
                })
            })
            .collect();
        let mut asset = Self {
            header: Header {
                source_digest: ContentDigest::of_bytes(bytes),
                source_size,
                settings,
            },
            content_digest: ContentDigest::of_bytes(&[]),
            diffuse,
            specular,
        };
        asset.content_digest = ContentDigest::of_bytes(&asset.frozen_bytes()?);
        Ok(asset)
    }

    pub fn settings(&self) -> EnvironmentSettings {
        self.header.settings
    }
    pub fn source_size(&self) -> [u32; 2] {
        self.header.source_size
    }
    pub fn content_digest(&self) -> ContentDigest {
        self.content_digest
    }
    pub fn storage_bytes(&self) -> u64 {
        (self.header.settings.pixels() * 12) as u64
    }
    pub fn pixel_count(&self) -> u64 {
        self.header.settings.pixels() as u64
    }

    /// A short canonical header followed by diffuse faces and roughness mips, RGB f32-LE.
    /// The byte digest includes source identity, settings, dimensions and every generated texel.
    pub fn frozen_bytes(&self) -> Result<Vec<u8>, ContractErrors> {
        let header = crate::canonical_bytes(&self.header).map_err(|e| error(e.to_string()))?;
        let mut bytes = Vec::with_capacity(12 + header.len() + self.storage_bytes() as usize);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&header);
        for cube in std::iter::once(&self.diffuse).chain(&self.specular) {
            for pixel in &cube.pixels {
                for value in pixel {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(bytes)
    }

    pub fn from_frozen(bytes: &[u8]) -> Result<Self, ContractErrors> {
        if bytes.len() < 12
            || bytes.len() > MAX_STORAGE_BYTES + 4096
            || bytes.get(..8) != Some(MAGIC)
        {
            return Err(error("invalid frozen environment header or byte budget"));
        }
        let length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if length > 4096 || length > bytes.len() - 12 {
            return Err(error("invalid environment header length"));
        }
        let header_bytes = &bytes[12..12 + length];
        let header: Header =
            serde_json::from_slice(header_bytes).map_err(|e| error(e.to_string()))?;
        header.settings.validate()?;
        validate_source_size(header.source_size)?;
        if crate::canonical_bytes(&header).map_err(|e| error(e.to_string()))? != header_bytes {
            return Err(error("environment metadata must be canonical"));
        }
        let storage = header.settings.pixels() * 12;
        if storage > MAX_STORAGE_BYTES || bytes.len() - 12 - length != storage {
            return Err(error("environment mip dimensions do not match its bytes"));
        }
        let mut pixels = bytes[12 + length..].chunks_exact(12);
        let mut cube = |size: u32| -> Result<Cube, ContractErrors> {
            let values = pixels
                .by_ref()
                .take((6 * size * size) as usize)
                .map(|pixel| {
                    std::array::from_fn(|i| {
                        f32::from_le_bytes(pixel[i * 4..i * 4 + 4].try_into().unwrap())
                    })
                })
                .collect::<Vec<_>>();
            if !valid_pixels(&values) {
                return Err(error(
                    "environment texels must be finite and within 0–65504",
                ));
            }
            Ok(Cube {
                size,
                pixels: values,
            })
        };
        let diffuse = cube(header.settings.diffuse_size)?;
        let specular = header
            .settings
            .levels()
            .map(cube)
            .collect::<Result<_, _>>()?;
        Ok(Self {
            header,
            diffuse,
            specular,
            content_digest: ContentDigest::of_bytes(bytes),
        })
    }

    /// Diffuse radiance is cosine-integrated irradiance divided by pi, ready for albedo.
    pub fn diffuse(&self, direction: [f32; 3]) -> [f32; 3] {
        self.diffuse.sample(direction)
    }
    pub fn specular(&self, direction: [f32; 3], roughness: f32) -> [f32; 3] {
        let level = roughness.clamp(0.0, 1.0) * (self.specular.len() - 1) as f32;
        let lo = libm::floorf(level) as usize;
        mix(
            self.specular[lo].sample(direction),
            self.specular[(lo + 1).min(self.specular.len() - 1)].sample(direction),
            level - lo as f32,
        )
    }
}

impl Cube {
    fn generate(size: u32, mut sample: impl FnMut([f32; 3]) -> [f32; 3]) -> Self {
        let mut pixels = Vec::with_capacity((6 * size * size) as usize);
        for face in 0..6 {
            for y in 0..size {
                for x in 0..size {
                    pixels.push(sample(face_direction(
                        face,
                        (x as f32 + 0.5) / size as f32,
                        (y as f32 + 0.5) / size as f32,
                    )));
                }
            }
        }
        Self { size, pixels }
    }

    fn sample(&self, direction: [f32; 3]) -> [f32; 3] {
        let (face, u, v) = face_uv(direction);
        let x = u * self.size as f32 - 0.5;
        let y = v * self.size as f32 - 0.5;
        let ix = libm::floorf(x) as i32;
        let iy = libm::floorf(y) as i32;
        let pixel = |mut face, x: i32, y: i32| {
            let (x, y) = if x < 0 || y < 0 || x >= self.size as i32 || y >= self.size as i32 {
                // Reproject edge taps onto the adjacent face instead of clamping cube seams.
                let (next, u, v) = face_uv(face_direction(
                    face,
                    (x as f32 + 0.5) / self.size as f32,
                    (y as f32 + 0.5) / self.size as f32,
                ));
                face = next;
                ((u * self.size as f32) as i32, (v * self.size as f32) as i32)
            } else {
                (x, y)
            };
            self.pixels[(face * self.size * self.size
                + y.clamp(0, self.size as i32 - 1) as u32 * self.size
                + x.clamp(0, self.size as i32 - 1) as u32) as usize]
        };
        mix(
            mix(pixel(face, ix, iy), pixel(face, ix + 1, iy), x - ix as f32),
            mix(
                pixel(face, ix, iy + 1),
                pixel(face, ix + 1, iy + 1),
                x - ix as f32,
            ),
            y - iy as f32,
        )
    }
}

fn integrate(
    n: [f32; 3],
    count: u32,
    roughness: Option<f32>,
    sample: &impl Fn([f32; 3]) -> [f32; 3],
) -> [f32; 3] {
    let up = if n[1].abs() < 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let tangent = normalize(cross(up, n));
    let bitangent = cross(n, tangent);
    let mut sum = [0.0f64; 3];
    let mut weight = 0.0f64;
    for i in 0..count {
        // Hammersley sequence is a pure function of sample index and configured count.
        let phi = core::f32::consts::TAU * (i as f32 + 0.5) / count as f32;
        let xi = (i.reverse_bits() as f64 / 4_294_967_296.0) as f32;
        let cosine = if let Some(r) = roughness {
            let a = r * r;
            libm::sqrtf((1.0 - xi) / (1.0 + (a * a - 1.0) * xi))
        } else {
            libm::sqrtf(1.0 - xi)
        };
        let sine = libm::sqrtf((1.0 - cosine * cosine).max(0.0));
        let h = std::array::from_fn(|j| {
            tangent[j] * libm::cosf(phi) * sine
                + bitangent[j] * libm::sinf(phi) * sine
                + n[j] * cosine
        });
        let (direction, w) = if roughness.is_some() {
            let nh = dot(n, h);
            let l = std::array::from_fn(|j| 2.0 * nh * h[j] - n[j]);
            (l, dot(n, l).max(0.0))
        } else {
            (h, 1.0)
        };
        if w > 0.0 {
            let radiance = sample(direction);
            for j in 0..3 {
                sum[j] += f64::from(radiance[j]) * f64::from(w);
            }
            weight += f64::from(w);
        }
    }
    sum.map(|v| (v / weight.max(f64::MIN_POSITIVE)) as f32)
}

fn sample_panorama(
    pixels: &[[f32; 3]],
    [width, height]: [u32; 2],
    direction: [f32; 3],
) -> [f32; 3] {
    let d = normalize(direction);
    let x = (libm::atan2f(d[2], d[0]) / core::f32::consts::TAU + 0.5) * width as f32 - 0.5;
    let y = libm::acosf(d[1].clamp(-1.0, 1.0)) / core::f32::consts::PI * height as f32 - 0.5;
    let ix = libm::floorf(x) as i32;
    let iy = libm::floorf(y) as i32;
    let pixel = |x: i32, y: i32| {
        pixels[(y.clamp(0, height as i32 - 1) as u32 * width + x.rem_euclid(width as i32) as u32)
            as usize]
    };
    mix(
        mix(pixel(ix, iy), pixel(ix + 1, iy), x - ix as f32),
        mix(pixel(ix, iy + 1), pixel(ix + 1, iy + 1), x - ix as f32),
        y - iy as f32,
    )
}

fn face_direction(face: u32, u: f32, v: f32) -> [f32; 3] {
    let u = 2.0 * u - 1.0;
    let v = 2.0 * v - 1.0;
    normalize(match face {
        0 => [1.0, -v, -u],
        1 => [-1.0, -v, u],
        2 => [u, 1.0, v],
        3 => [u, -1.0, -v],
        4 => [u, -v, 1.0],
        _ => [-u, -v, -1.0],
    })
}

fn face_uv(d: [f32; 3]) -> (u32, f32, f32) {
    let [x, y, z] = d;
    let [ax, ay, az] = d.map(f32::abs);
    let (face, u, v) = if ax >= ay && ax >= az {
        if x >= 0.0 {
            (0, -z / ax, -y / ax)
        } else {
            (1, z / ax, -y / ax)
        }
    } else if ay >= az {
        if y >= 0.0 {
            (2, x / ay, z / ay)
        } else {
            (3, x / ay, -z / ay)
        }
    } else if z >= 0.0 {
        (4, x / az, -y / az)
    } else {
        (5, -x / az, -y / az)
    };
    (face, u * 0.5 + 0.5, v * 0.5 + 0.5)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = libm::sqrtf(dot(v, v));
    if length <= 0.0 || !length.is_finite() {
        [0.0, 1.0, 0.0]
    } else {
        v.map(|v| v / length)
    }
}
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
}
fn valid_pixels(values: &[[f32; 3]]) -> bool {
    values
        .iter()
        .flatten()
        .all(|v| v.is_finite() && (0.0..=MAX_RADIANCE).contains(v))
}
fn validate_source_size([width, height]: [u32; 2]) -> Result<(), ContractErrors> {
    if height == 0 || height > 2048 || width != height * 2 {
        Err(error(
            "panorama must be equirectangular 2:1, at most 4096×2048",
        ))
    } else {
        Ok(())
    }
}
fn error(message: impl Into<String>) -> ContractErrors {
    let mut e = ContractErrors::default();
    e.push("/environment", message);
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(width: usize, height: usize, pixel: impl Fn(usize, usize) -> [f32; 3]) -> Vec<u8> {
        let values = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| image::Rgb(pixel(x, y)))
            .collect::<Vec<_>>();
        let mut bytes = Vec::new();
        image::codecs::hdr::HdrEncoder::new(&mut bytes)
            .encode(&values, width, height)
            .unwrap();
        bytes
    }

    #[test]
    fn lighting_is_frozen_bounded_and_preserves_constant_hdr_energy() {
        let bytes = hdr(8, 4, |_, _| [4.0, 2.0, 1.0]);
        let settings = EnvironmentSettings {
            face_size: 8,
            diffuse_size: 2,
            samples: 16,
        };
        let asset = EnvironmentAsset::from_encoded(&bytes, settings).unwrap();
        let frozen = asset.frozen_bytes().unwrap();
        assert_eq!(asset, EnvironmentAsset::from_frozen(&frozen).unwrap());
        assert_eq!(asset.content_digest(), ContentDigest::of_bytes(&frozen));
        assert_eq!(
            frozen,
            EnvironmentAsset::from_encoded(&bytes, settings)
                .unwrap()
                .frozen_bytes()
                .unwrap()
        );
        assert_eq!(
            asset.storage_bytes(),
            (6 * (4 + 64 + 16 + 4 + 1) * 12) as u64
        );
        for n in [
            [1.0, 0.0, 0.0],
            [-1.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
        ] {
            for value in [
                asset.diffuse(n),
                asset.specular(n, 0.0),
                asset.specular(n, 0.7),
                asset.specular(n, 1.0),
            ] {
                for i in 0..3 {
                    assert!((value[i] - [4.0, 2.0, 1.0][i]).abs() < 0.00001);
                }
            }
        }
        assert_ne!(
            asset.content_digest(),
            EnvironmentAsset::from_encoded(
                &bytes,
                EnvironmentSettings {
                    samples: 32,
                    ..settings
                }
            )
            .unwrap()
            .content_digest()
        );
        assert!(
            EnvironmentAsset::from_encoded(
                &bytes,
                EnvironmentSettings {
                    face_size: 512,
                    ..settings
                }
            )
            .is_err()
        );
        assert!(EnvironmentAsset::from_encoded(&hdr(4, 4, |_, _| [1.0; 3]), settings).is_err());
        assert!(EnvironmentAsset::from_frozen(&frozen[..frozen.len() - 1]).is_err());
        let mut invalid = frozen.clone();
        let end = invalid.len();
        invalid[end - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(EnvironmentAsset::from_frozen(&invalid).is_err());
    }

    #[test]
    fn faces_seams_and_roughness_follow_directional_panorama() {
        let bytes = hdr(32, 16, |x, y| {
            if x < 16 && y < 8 {
                [8.0, 0.0, 0.0]
            } else {
                [0.0, 0.0, 1.0]
            }
        });
        let asset = EnvironmentAsset::from_encoded(
            &bytes,
            EnvironmentSettings {
                face_size: 16,
                diffuse_size: 4,
                samples: 32,
            },
        )
        .unwrap();
        let a = asset.specular([0.0, 1.0, -1.0], 0.0);
        let b = asset.specular([0.0, 1.0, 1.0], 0.0);
        assert!(
            a[0] > 7.0 && b[0] < 0.1,
            "panorama azimuth must map consistently: {a:?} {b:?}"
        );
        assert_ne!(a, asset.specular([0.0, 1.0, -1.0], 1.0));
        for face in 0..6 {
            for (u, v) in [(0.1, 0.3), (0.8, 0.7)] {
                let (actual, x, y) = face_uv(face_direction(face, u, v));
                assert_eq!(face, actual);
                assert!((u - x).abs() < 1e-6 && (v - y).abs() < 1e-6);
            }
        }
        let cube = Cube::generate(32, |n| n.map(|v| v * 0.5 + 0.5));
        for axis in 0..3 {
            let mut a = [1.0; 3];
            a[axis] += 0.00001;
            let mut b = [1.0; 3];
            b[axis] -= 0.00001;
            let a = cube.sample(a);
            let b = cube.sample(b);
            assert!((0..3).all(|i| (a[i] - b[i]).abs() < 0.01));
        }
    }
}
