//! Software codecs need FFmpeg, but no GPU or external media. The compatibility CI runs this
//! same suite in a separate process for each supported FFmpeg major.

use std::path::{Path, PathBuf};

use valle_native::media::{self, Codec, Decoding, Encoding, Options, RATE, TICKS_PER_SECOND};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("valle-media-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn options(codec: Codec) -> Options {
    Options {
        size: [64, 32],
        fps: [8, 1],
        codec,
        encoding: Encoding::Software,
        bitrate: None,
        sound: false,
        surfaces: false,
    }
}

fn picture(index: u8, alpha: bool) -> Vec<u8> {
    (0..32)
        .flat_map(|_| {
            (0..64).flat_map(move |x| {
                if x < 32 {
                    let gray = 32 + index * 8;
                    [gray, gray, gray, if alpha { index * 10 } else { 255 }]
                } else {
                    [200, 60, 100, 255]
                }
            })
        })
        .collect()
}

#[test]
fn loaded_version_matches_the_ci_job() {
    let version = media::version().expect("FFmpeg is installed");
    if let Ok(expected) = std::env::var("VALLE_TEST_FFMPEG_MAJOR") {
        assert_eq!(format!("{version:?}"), format!("V{expected}"));
    }
}

#[test]
fn software_codecs_keep_frames_duration_colors_and_alpha() {
    let dir = Folder::new("video");
    for codec in [Codec::H264, Codec::Hevc, Codec::ProRes4444] {
        let alpha = codec == Codec::ProRes4444;
        let path = dir.0.join(if alpha { "video.mov" } else { "video.mp4" });
        let mut writer = media::writer(&path, &options(codec)).unwrap();
        assert!(writer.surface().unwrap().is_none());
        for index in 0..24 {
            writer.push(&picture(index, alpha)).unwrap();
        }
        writer.finish().unwrap();
        writer.finish().unwrap();
        drop(writer);
        assert_eq!(
            media::length(&path).unwrap(),
            3 * TICKS_PER_SECOND,
            "{codec:?}"
        );
        assert!(media::sound(&path).unwrap().is_none());

        let mut reader = media::reader(&path, Some(Decoding::Software)).unwrap();
        assert_eq!(reader.size(), [64, 32]);
        // Sequential decoding, repeated reads, backward seeking and a jump past AHEAD.
        for index in (0..24).chain([0, 23, 23, 12, 3, 22, 1]) {
            let at = i64::from(index) * TICKS_PER_SECOND / 8;
            let actual = reader.frame(at).unwrap();
            let expected = picture(index, alpha);
            assert_eq!(actual.len(), expected.len());
            for x in [8, 48] {
                let i = (16 * 64 + x) * 4;
                for c in 0..4 {
                    // Flat interiors avoid chroma subsampling edges; lossy RGB conversion may
                    // round a few levels, while ProRes alpha should be preserved to one level.
                    let tolerance = if c == 3 { 1 } else { 5 };
                    assert!(
                        actual[i + c].abs_diff(expected[i + c]) <= tolerance,
                        "{codec:?} frame {index}, x={x}, channel {c}: {} != {}",
                        actual[i + c],
                        expected[i + c]
                    );
                }
            }
            assert_eq!(reader.position(), Some(at));
            assert_eq!(reader.frame(at).unwrap(), actual);
        }
        let last = reader.frame(23 * TICKS_PER_SECOND / 8).unwrap();
        assert_eq!(reader.frame(10 * TICKS_PER_SECOND).unwrap(), last);
        assert_eq!(reader.frame(-1).unwrap(), reader.frame(0).unwrap());
    }
}

#[test]
fn fractional_fps_keeps_the_final_frame_duration() {
    let dir = Folder::new("fps");
    let path = dir.0.join("video.mp4");
    let o = Options {
        fps: [30_000, 1001],
        ..options(Codec::H264)
    };
    let mut writer = media::writer(&path, &o).unwrap();
    for index in 0..3 {
        writer.push(&picture(index, false)).unwrap();
    }
    writer.finish().unwrap();
    drop(writer);
    let step = TICKS_PER_SECOND * 1001 / 30_000;
    assert_eq!(media::length(&path).unwrap(), 3 * step);
    let mut reader = media::reader(&path, Some(Decoding::Software)).unwrap();
    let last = reader.frame(2 * step).unwrap();
    assert_eq!(reader.position(), Some(2 * step));
    assert_eq!(reader.frame(3 * step - 1).unwrap(), last);
}

/// A PCM WAV with no encoder delay, built independently of the media layer under test.
fn wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) {
    let bytes = samples.len() as u32 * 2;
    let mut out = Vec::from(*b"RIFF");
    out.extend((36 + bytes).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16_u32.to_le_bytes());
    out.extend(1_u16.to_le_bytes());
    out.extend(channels.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * u32::from(channels) * 2).to_le_bytes());
    out.extend((channels * 2).to_le_bytes());
    out.extend(16_u16.to_le_bytes());
    out.extend(b"data");
    out.extend(bytes.to_le_bytes());
    out.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    std::fs::write(path, out).unwrap();
}

#[test]
fn audio_resamples_mono_and_preserves_stereo_and_the_tail() {
    let dir = Folder::new("audio");
    let path = dir.0.join("sound.wav");
    // 0.1 seconds at 44.1 kHz includes a partial decoder block and a resampler tail.
    let mono: Vec<i16> = (0..4410)
        .map(|i| ((i as f64 * std::f64::consts::TAU * 440.0 / 44_100.0).sin() * 8192.0) as i16)
        .collect();
    wav(&path, 44_100, 1, &mono);
    let actual = media::sound(&path).unwrap().unwrap();
    assert_eq!(actual.len(), 4800 * 2);
    for (i, pair) in actual
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .skip(40)
        .take(4720)
    {
        assert_eq!(pair[0], pair[1]);
        // FFmpeg's default mono-to-stereo matrix sends -3 dB to each channel.
        let expected = (i as f64 * std::f64::consts::TAU * 440.0 / f64::from(RATE)).sin()
            * 0.25
            * std::f64::consts::FRAC_1_SQRT_2;
        assert!((f64::from(pair[0]) - expected).abs() < 0.001, "sample {i}");
    }

    let stereo: Vec<i16> = (0..2051).flat_map(|i| [i as i16, -(i as i16)]).collect();
    wav(&path, RATE, 2, &stereo);
    let actual = media::sound(&path).unwrap().unwrap();
    assert_eq!(actual.len(), stereo.len());
    for (a, b) in actual.iter().zip(&stereo) {
        assert_eq!(*a, f32::from(*b) / 32768.0);
    }
}

#[test]
fn muxed_pcm_keeps_channels_and_a_partial_final_block() {
    let dir = Folder::new("mux");
    let path = dir.0.join("video.mov");
    let o = Options {
        sound: true,
        ..options(Codec::ProRes4444)
    };
    let mut writer = media::writer(&path, &o).unwrap();
    writer.push(&picture(0, true)).unwrap();
    let samples: Vec<f32> = (0..2051).flat_map(|i| [i as f32 / 8192.0, -0.25]).collect();
    // Two irregular chunks must produce the same PCM, including the final 3 stereo pairs.
    writer.sound(&samples[..398]).unwrap();
    writer.sound(&samples[398..]).unwrap();
    writer.finish().unwrap();
    drop(writer);
    let actual = media::sound(&path).unwrap().unwrap();
    assert_eq!(actual, samples);
}

#[test]
fn invalid_files_and_missing_streams_return_errors() {
    let dir = Folder::new("invalid");
    let path = dir.0.join("input.mp4");
    for data in [
        None,
        Some(&b"not a video"[..]),
        Some(&b"\0\0\0\x18ftyp"[..]),
    ] {
        if let Some(data) = data {
            std::fs::write(&path, data).unwrap();
        }
        assert!(media::reader(&path, Some(Decoding::Software)).is_err());
        assert!(media::length(&path).is_err());
        assert!(media::sound(&path).is_err());
    }
    let path = dir.0.join("only-audio.wav");
    wav(&path, RATE, 1, &[0; 64]);
    assert!(media::reader(&path, Some(Decoding::Software)).is_err());
    assert!(media::length(&path).is_err());
}

#[test]
fn invalid_encoder_inputs_do_not_panic_or_stall() {
    let dir = Folder::new("inputs");
    let path = dir.0.join("video.mp4");
    let base = options(Codec::H264);
    for o in [
        Options {
            size: [0, 32],
            ..base.clone()
        },
        Options {
            size: [63, 32],
            ..base.clone()
        },
        Options {
            fps: [0, 1],
            ..base.clone()
        },
        Options {
            fps: [30, 0],
            ..base.clone()
        },
        Options {
            fps: [i64::MAX, 1],
            ..base.clone()
        },
        Options {
            codec: Codec::ProRes4444,
            ..base.clone()
        },
    ] {
        assert!(media::writer(&path, &o).is_err(), "{o:?}");
    }
    let o = Options {
        sound: true,
        ..base
    };
    let mut writer = media::writer(&path, &o).unwrap();
    assert!(writer.push(&[0; 4]).is_err());
    assert!(writer.sound(&[0.25]).is_err());
    writer.push(&picture(0, false)).unwrap();
    writer.sound(&[0.0; 2048]).unwrap();
    writer.finish().unwrap();
}
