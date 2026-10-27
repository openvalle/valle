use super::*;
use valle_engine::{
    compositor::{
        bloom::f32_to_half,
        delivery::{DeliveryStorage, transform_working_pixel},
    },
    resource::{
        OutputAlphaMode, OutputBackground, OutputBitDepth, OutputColorEncoding, SignalLuminance,
        ToneMap,
    },
};

fn spec(gamut: GamutMap, dither: Dither) -> OutputSpec {
    OutputSpec::new(
        OutputColorEncoding::SRGB,
        OutputAlphaMode::Opaque,
        OutputBackground::opaque_srgb([0, 0, 0]),
        ToneMap::None,
        gamut,
        dither,
        OutputBitDepth::Eight,
        SignalLuminance::SDR_100,
    )
    .unwrap()
}

fn backends() -> Vec<Backend> {
    let mut result = vec![Backend::Scalar];
    let detected = Backend::detect();
    if detected != Backend::Scalar {
        result.push(detected);
    }
    result
}

fn bytes(pixels: &[[u16; 4]]) -> Vec<u8> {
    pixels
        .iter()
        .flatten()
        .flat_map(|h| h.to_ne_bytes())
        .collect()
}

fn reference(pixel: [u16; 4], spec: OutputSpec, x: usize, y: usize) -> [u8; 4] {
    match transform_working_pixel(pixel.map(half_to_f32), spec, [x as u32, y as u32])
        .unwrap()
        .storage
    {
        DeliveryStorage::Integer {
            bit_depth: 8,
            codes,
        } => codes.map(|v| v as u8),
        _ => panic!("expected sRGB8"),
    }
}

fn sample(index: u32) -> [u16; 4] {
    let mut state = index.wrapping_mul(747796405).wrapping_add(2891336453);
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        // Include subnormals, negative/extended-range values and the largest finite half.
        let h = state as u16;
        if h & 0x7c00 == 0x7c00 { h & !0x0400 } else { h }
    };
    [next(), next(), next(), 0x3c00]
}

#[test]
fn all_finite_half_patterns_stay_within_one_reference_code() {
    let samples: Vec<_> = (0..=u16::MAX)
        .filter(|h| h & 0x7c00 != 0x7c00)
        .enumerate()
        .map(|(i, h)| {
            let mut pixel = sample(i as u32);
            pixel[i % 3] = h;
            pixel
        })
        .collect();
    let input = bytes(&samples);
    for gamut in [GamutMap::Clip, GamutMap::ChromaCompress] {
        for dither in [Dither::None, Dither::Triangular { seed: u64::MAX }] {
            let spec = spec(gamut, dither);
            let expected: Vec<_> = samples
                .iter()
                .enumerate()
                .flat_map(|(x, p)| reference(*p, spec, x, 129))
                .collect();
            for backend in backends() {
                let mut converter = Converter::new(spec);
                converter.backend = backend;
                let mut output = vec![0; samples.len() * 4];
                converter
                    .row(&input, 129, false, false, &mut output)
                    .unwrap();
                for (i, (actual, expected)) in output.iter().zip(&expected).enumerate() {
                    assert!(
                        actual.abs_diff(*expected) <= 1,
                        "{backend:?} {gamut:?} {dither:?}, sample {:?} channel {}: {actual} vs {expected}",
                        samples[i / 4],
                        i % 4
                    );
                    if i % 4 == 3 {
                        assert_eq!(*actual, 255);
                    }
                }
            }
        }
    }
}

#[test]
fn simd_matches_portable_bytes_for_mixed_batches_and_cache_modes() {
    let mut samples: Vec<_> = (0..16387).map(sample).collect();
    samples.extend([
        [0, 0, 0, 0x3c00],
        [0x8000, 1, 0x8001, 0x3c00],
        [0x7bff, 0xfbff, 0x03ff, 0x3c00],
        [0x3c00; 4],
    ]);
    let input = bytes(&samples);
    for gamut in [GamutMap::Clip, GamutMap::ChromaCompress] {
        for dither in [Dither::None, Dither::Triangular { seed: 42 }] {
            let spec = spec(gamut, dither);
            let mut scalar = Converter::new(spec);
            scalar.backend = Backend::Scalar;
            let mut expected = vec![0; samples.len() * 4];
            scalar
                .row(&input, 127, false, false, &mut expected)
                .unwrap();
            for backend in backends() {
                let mut converter = Converter::new(spec);
                converter.backend = backend;
                for cached in [true, false, true] {
                    let mut actual = vec![0; expected.len()];
                    converter
                        .row(&input, 127, false, cached, &mut actual)
                        .unwrap();
                    assert_eq!(
                        actual, expected,
                        "{backend:?} {gamut:?} {dither:?}, cache={cached}"
                    );
                }
            }
        }
    }
}

unsafe fn pack(backend: Backend, codes: Codes, noise: Option<&Codes>, swap: bool) -> [u8; 16] {
    // SAFETY: tests only enumerate backends supported by this CPU.
    unsafe {
        match backend {
            Backend::Scalar => Scalar::pack(codes, noise, swap),
            #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
            Backend::Neon => neon::Neon::pack(codes, noise, swap),
            #[cfg(all(target_arch = "x86_64", target_endian = "little"))]
            Backend::Avx2 => x86::Avx2::pack(codes, noise, swap),
        }
    }
}

#[test]
fn quantization_handles_halfway_neighbors_saturation_and_channel_order() {
    for integer in 0..255 {
        let half = integer as f32 + 0.5;
        let codes = [
            [half.next_down(), half, half.next_up(), 0.0],
            [-1.0, 256.0, 254.5, 255.0],
            [0.49, 0.5, 1.5, 2.5],
        ];
        for noise in [None, Some([[0.25, -0.25, 0.5, -0.5]; 3])] {
            for swap in [false, true] {
                let expected: [u8; 16] = std::array::from_fn(|i| {
                    let c = i % 4;
                    if c == 3 {
                        255
                    } else {
                        let c = if swap { 2 - c } else { c };
                        (codes[c][i / 4] + noise.map_or(0.0, |n| n[c][i / 4]))
                            .clamp(0.0, 255.0)
                            .round() as u8
                    }
                });
                for backend in backends() {
                    // SAFETY: available backend, bounded test samples/noise.
                    assert_eq!(
                        unsafe { pack(backend, codes, noise.as_ref(), swap) },
                        expected,
                        "{backend:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn cache_keeps_dither_at_original_coordinates_across_rows_and_tail_widths() {
    let color = [0.18, 0.31, 0.43, 1.0].map(f32_to_half);
    for width in [1, 2, 3, 4, 5, 127, 128, 129, 131, 257] {
        let input = bytes(&vec![color; width]);
        for backend in backends() {
            for seed in [0, 93] {
                let spec = spec(GamutMap::ChromaCompress, Dither::Triangular { seed });
                let mut converter = Converter::new(spec);
                converter.backend = backend;
                let mut first = None;
                for y in [0, 1, 127, 128, 129, 4095, 0] {
                    for swap in [false, true] {
                        let mut result = vec![0; width * 4];
                        converter.row(&input, y, swap, true, &mut result).unwrap();
                        let mut uncached = vec![0; result.len()];
                        converter
                            .row(&input, y, swap, false, &mut uncached)
                            .unwrap();
                        assert_eq!(result, uncached);
                        for (x, pixel) in result.as_chunks::<4>().0.iter().enumerate() {
                            let mut exact = reference(color, spec, x, y);
                            if swap {
                                exact.swap(0, 2);
                            }
                            assert!(pixel.iter().zip(exact).all(|(a, b)| a.abs_diff(b) <= 1));
                        }
                        if y == 0 && !swap {
                            if let Some(first) = &first {
                                assert_eq!(&result, first);
                            } else {
                                first = Some(result);
                            }
                        }
                    }
                }
                if width >= 128 {
                    let first = first.unwrap();
                    assert!(
                        first.as_chunks::<4>().0.iter().any(|p| p != &first[..4]),
                        "dither must survive color-cache hits"
                    );
                }
            }
        }
    }
}

#[test]
fn cache_collisions_compare_every_input_bit() {
    let mut slots = [None; CACHE_SIZE];
    let (a, b) = (0..=CACHE_SIZE)
        .find_map(|i| {
            let pixel = sample(i as u32);
            let key = pixel
                .iter()
                .enumerate()
                .fold(0, |k, (c, h)| k | (u64::from(*h) << (c * 16)));
            let keys = [key; 4];
            let slot = cache_slot(keys);
            if let Some(old) = slots[slot] {
                Some((old, pixel))
            } else {
                slots[slot] = Some(pixel);
                None
            }
        })
        .unwrap();
    for backend in backends() {
        let mut converter = Converter::new(spec(GamutMap::ChromaCompress, Dither::None));
        converter.backend = backend;
        for pixel in [a, b, a, b] {
            let input = bytes(&[pixel; 4]);
            let (mut cached, mut uncached) = ([0; 16], [0; 16]);
            converter.row(&input, 0, false, true, &mut cached).unwrap();
            converter
                .row(&input, 0, false, false, &mut uncached)
                .unwrap();
            assert_eq!(cached, uncached);
        }
    }
}

#[test]
fn invalid_channels_are_rejected_even_after_a_cache_hit() {
    for backend in backends() {
        let mut converter = Converter::new(spec(GamutMap::Clip, Dither::None));
        converter.backend = backend;
        for channel in 0..4 {
            for bad in [0x7c00, 0xfc00, 0x7c01, 0x7fff] {
                for lane in 0..4 {
                    let mut pixels = [[0x3400, 0x3000, 0x3800, 0x3c00]; 4];
                    let mut output = [0; 16];
                    converter
                        .row(&bytes(&pixels), 0, false, true, &mut output)
                        .unwrap();
                    pixels[lane][channel] = bad;
                    assert!(
                        converter
                            .row(&bytes(&pixels), 0, false, true, &mut output)
                            .is_err()
                    );
                }
            }
        }
        for alpha in [0, 0x3bff, 0x3c01] {
            assert!(
                converter
                    .row(&bytes(&[[0, 0, 0, alpha]]), 0, false, true, &mut [0; 4])
                    .is_err()
            );
        }
    }
}

#[test]
fn unaligned_rows_and_tails_do_not_touch_padding() {
    for backend in backends() {
        let mut converter = Converter::new(spec(GamutMap::Clip, Dither::None));
        converter.backend = backend;
        for width in 0..18 {
            let pixels: Vec<_> = (0..width).map(sample).collect();
            let mut input = vec![99];
            input.extend(bytes(&pixels));
            input.push(99);
            let mut output = vec![77; width as usize * 4 + 2];
            let end = output.len() - 1;
            converter
                .row(
                    &input[1..input.len() - 1],
                    0,
                    false,
                    false,
                    &mut output[1..end],
                )
                .unwrap();
            assert_eq!(output[0], 77);
            assert_eq!(output[end], 77);
        }
        assert!(
            converter
                .row(&[0; 9], 0, false, false, &mut [0; 4])
                .is_err()
        );
        assert!(
            converter
                .row(&[0; 8], 0, false, false, &mut [0; 3])
                .is_err()
        );
    }
}

use super::super::rgba8_target_info;
use super::super::{OutputStaging, stage_cpu_sdr_output_into, working_color_space};
use skia_safe::{AlphaType, ColorType, Data, Image, ImageInfo, images};
use valle_engine::resource::{Extent2d, LuminanceNits};

fn raster(
    width: usize,
    height: usize,
    padding: usize,
    sample: impl Fn(usize) -> [u16; 4],
) -> Image {
    let row_bytes = width * 8 + padding;
    // Invalid F16 bytes in the padding catch accidental processing of stride as real pixels.
    let mut data = vec![0xff; row_bytes * height];
    for i in 0..width * height {
        let offset = i / width * row_bytes + i % width * 8;
        data[offset..offset + 8].copy_from_slice(&bytes(&[sample(i)]));
    }
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBAF16,
        AlphaType::Premul,
        Some(working_color_space().unwrap()),
    );
    images::raster_from_data(&info, Data::new_copy(&data), row_bytes).unwrap()
}

#[test]
fn staging_reuses_storage_but_invalidates_colors_and_noise_on_spec_changes() {
    let mut staging = OutputStaging::default();
    for (width, height) in [(131, 129), (3, 5), (132, 130), (1, 1)] {
        let image = raster(width, height, 24, |i| sample(i as u32));
        for gamut in [GamutMap::Clip, GamutMap::ChromaCompress] {
            for dither in [
                Dither::Triangular { seed: 0 },
                Dither::Triangular { seed: 51 },
                Dither::None,
            ] {
                let spec = spec(gamut, dither);
                let target =
                    rgba8_target_info(spec, Extent2d::new(width as u32, height as u32).unwrap())
                        .unwrap();
                for color in [ColorType::RGBA8888, ColorType::BGRA8888] {
                    let info = target.with_color_type(color);
                    let mut fresh = OutputStaging::default();
                    assert_eq!(
                        stage_cpu_sdr_output_into(&image, spec, &info, &mut staging).unwrap(),
                        Some(width * 4)
                    );
                    stage_cpu_sdr_output_into(&image, spec, &info, &mut fresh).unwrap();
                    assert_eq!(staging.pixels(), fresh.pixels());
                    assert!(
                        staging.working.is_empty(),
                        "direct F16 must not allocate an F32 readback"
                    );
                    for (i, pixel) in staging.pixels().as_chunks::<4>().0.iter().enumerate() {
                        let mut exact = reference(sample(i as u32), spec, i % width, i / width);
                        if color == ColorType::BGRA8888 {
                            exact.swap(0, 2);
                        }
                        assert!(pixel.iter().zip(exact).all(|(a, b)| a.abs_diff(b) <= 1));
                    }
                }
                let ptr = staging.pixels.as_ptr();
                let cache_ptr = staging.sdr.as_ref().unwrap().converter.cache.as_ptr();
                stage_cpu_sdr_output_into(&image, spec, &target, &mut staging).unwrap();
                assert_eq!(staging.pixels.as_ptr(), ptr);
                assert_eq!(
                    staging.sdr.as_ref().unwrap().converter.cache.as_ptr(),
                    cache_ptr
                );
            }
        }
    }
}

#[test]
fn unsupported_output_and_input_contracts_fall_back() {
    let image = raster(4, 2, 0, |_| [0x3000, 0x3400, 0x3800, 0x3c00]);
    let base = spec(GamutMap::ChromaCompress, Dither::Triangular { seed: 0 });
    let extent = Extent2d::new(4, 2).unwrap();
    let target = rgba8_target_info(base, extent).unwrap();
    let mut staging = OutputStaging::default();
    let mut cases = vec![OutputSpec::srgb_preview(OutputBackground::Transparent).unwrap()];
    for (encoding, tone, depth, white) in [
        (
            OutputColorEncoding::DISPLAY_P3,
            ToneMap::None,
            OutputBitDepth::Eight,
            100,
        ),
        (
            OutputColorEncoding::REC709,
            ToneMap::None,
            OutputBitDepth::Eight,
            100,
        ),
        (
            OutputColorEncoding::SRGB,
            ToneMap::ReinhardLuminance,
            OutputBitDepth::Eight,
            100,
        ),
        (
            OutputColorEncoding::SRGB,
            ToneMap::None,
            OutputBitDepth::Eight,
            203,
        ),
        (
            OutputColorEncoding::SRGB,
            ToneMap::None,
            OutputBitDepth::Ten,
            100,
        ),
        (
            OutputColorEncoding::REC2020_PQ,
            ToneMap::None,
            OutputBitDepth::Ten,
            203,
        ),
    ] {
        let nits = LuminanceNits::new(white).unwrap();
        cases.push(
            OutputSpec::new(
                encoding,
                OutputAlphaMode::Opaque,
                OutputBackground::opaque_srgb([0, 0, 0]),
                tone,
                GamutMap::Clip,
                Dither::None,
                depth,
                SignalLuminance::new(nits, nits).unwrap(),
            )
            .unwrap(),
        );
    }
    for spec in cases {
        assert!(
            stage_cpu_sdr_output_into(&image, spec, &target, &mut staging)
                .unwrap()
                .is_none()
        );
        assert!(staging.sdr.is_none());
    }
    for info in [
        image.image_info().with_color_type(ColorType::RGBAF32),
        image.image_info().with_alpha_type(AlphaType::Unpremul),
        image
            .image_info()
            .with_color_space(skia_safe::ColorSpace::new_srgb()),
    ] {
        let row_bytes = info.min_row_bytes();
        let data = vec![0; row_bytes * 2];
        let input = images::raster_from_data(&info, Data::new_copy(&data), row_bytes).unwrap();
        assert!(
            stage_cpu_sdr_output_into(&input, base, &target, &mut staging)
                .unwrap()
                .is_none()
        );
    }
    assert!(
        stage_cpu_sdr_output_into(&image, base, &target.with_dimensions((5, 2)), &mut staging)
            .is_err()
    );
    assert!(
        stage_cpu_sdr_output_into(
            &image,
            base,
            &target.with_color_type(ColorType::RGBAF16),
            &mut staging
        )
        .is_err()
    );
}

#[test]
fn retained_storage_is_bounded_released_and_optional_under_memory_pressure() {
    use valle_engine::motion::{
        cpu::CpuBudget,
        memory::{MemoryBudget, MemoryKind},
    };
    let spec = spec(GamutMap::Clip, Dither::Triangular { seed: 7 });
    let retained = Converter::retained_bytes(spec.dither()) as u64;
    let construction = Converter::construction_bytes(spec.dither()) as u64;
    let image = raster(4, 2, 0, |_| [0x3000, 0x3400, 0x3800, 0x3c00]);
    let info = rgba8_target_info(spec, Extent2d::new(4, 2).unwrap()).unwrap();
    for limit in [1, retained, retained + construction] {
        let budget = MemoryBudget::new(Some(limit));
        let cpu = CpuBudget::new_with_memory(1, Some(budget.clone())).unwrap();
        let mut staging = OutputStaging::default();
        cpu.run(|| {
            let result = stage_cpu_sdr_output_into(&image, spec, &info, &mut staging).unwrap();
            if limit < retained + construction {
                assert!(result.is_none());
                assert_eq!(budget.report().used_bytes, 0);
            } else {
                assert!(result.is_some());
                assert_eq!(budget.report().used_bytes, retained);
                assert_eq!(
                    budget.report().by_kind[MemoryKind::Delivery as usize],
                    retained
                );
                let state = staging.sdr.as_ref().unwrap();
                assert_eq!(
                    state.converter.cache.capacity() * size_of::<Entry>()
                        + state.converter.noise.capacity() * size_of::<Codes>(),
                    retained as usize
                );
                stage_cpu_sdr_output_into(&image, spec, &info, &mut staging).unwrap();
                assert_eq!(budget.report().used_bytes, retained);
                staging.clear_cache();
                assert_eq!(budget.report().used_bytes, 0);
            }
        });
        drop(staging);
        assert_eq!(budget.report().used_bytes, 0);
        assert!(budget.report().peak_bytes <= limit);
    }
}

/// Run manually with --ignored --nocapture; timing assertions would make CI hardware-dependent.
#[test]
#[ignore = "1080p CPU output microbenchmark; run explicitly on an idle machine"]
fn benchmark_cpu_sdr_output() {
    use crate::executor::skia::{SkiaTarget, effect::EffectRuntime, surface::raster_surface};
    use std::{hint::black_box, time::Instant};
    let (width, height) = (1920, 1080);
    let spec = spec(GamutMap::ChromaCompress, Dither::Triangular { seed: 0 });
    let info =
        rgba8_target_info(spec, Extent2d::new(width as u32, height as u32).unwrap()).unwrap();
    for kind in ["flat", "gradient", "noise"] {
        let image = raster(width, height, 0, |i| match kind {
            "flat" => [0.18, 0.31, 0.43, 1.0].map(f32_to_half),
            "gradient" => [((i % width) as f32 / width as f32), 0.31, 0.43, 1.0].map(f32_to_half),
            _ => {
                let p = sample(i as u32);
                [
                    f32_to_half((p[0] as f32) / 65535.0),
                    f32_to_half((p[1] as f32) / 65535.0),
                    f32_to_half((p[2] as f32) / 65535.0),
                    0x3c00,
                ]
            }
        });
        let mut staging = OutputStaging::default();
        let mut surface = raster_surface(&info).unwrap();
        let mut effects = EffectRuntime::output_for_test(spec);
        for mode in ["exact", "shader", "scalar", "auto"] {
            let mut times = Vec::new();
            for repeat in 0..4 {
                let started = Instant::now();
                if mode == "shader" {
                    let mut target = SkiaTarget::new(&mut surface, spec).unwrap();
                    assert!(
                        target.commit_shader(effects.output_shader(&image, spec, false).unwrap())
                    );
                } else {
                    let row_bytes = if mode == "exact" {
                        super::super::stage_output_into(&image, spec, &info, &mut staging).unwrap()
                    } else {
                        if let Some(state) = &mut staging.sdr {
                            state.converter.backend = if mode == "scalar" {
                                Backend::Scalar
                            } else {
                                Backend::detect()
                            };
                        }
                        stage_cpu_sdr_output_into(&image, spec, &info, &mut staging)
                            .unwrap()
                            .unwrap()
                    };
                    assert!(
                        SkiaTarget::new(&mut surface, spec)
                            .unwrap()
                            .commit_pixels(staging.pixels(), row_bytes)
                    );
                }
                black_box(surface.peek_pixels().unwrap().bytes().unwrap()[0]);
                if repeat > 0 {
                    times.push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "cpu-sdr {kind} {mode}: median={:.3}ms samples={times:?}",
                times[1]
            );
        }
    }
}
