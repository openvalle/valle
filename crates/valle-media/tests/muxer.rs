#![cfg(feature = "libav")]
//! Audio/video stream topology, presentation timestamps and container duration.

use valle_media::frame::RgbaFrame;

#[test]
fn muxer_emits_video_and_aac_with_matching_duration() {
    use valle_media::codec::{Muxer, probe_av};
    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("muxer.mp4");
    let (w, h, fps, n) = (64u32, 48u32, 30u32, 45i64); // 1.5s @30
    {
        let mut mux = Muxer::open(&out, w, h, fps, None).expect("open muxer");
        for i in 0..n {
            let mut f = RgbaFrame::new(w, h);
            f.fill([20, (i % 256) as u8, 200, 255]);
            mux.encode_video(&f).expect("encode video");
        }
        mux.finish().expect("finish");
    }
    let size = std::fs::metadata(&out).expect("mp4 exists").len();
    assert!(size > 1000, "mp4 too small ({size})");

    let p = probe_av(&out).expect("probe");
    let (vw, vh, v_dur) = p.video.expect("expected a video stream");
    assert_eq!((vw, vh), (w, h));
    // The output must always contain an AAC track.
    let a_dur = p.audio.expect("expected an AAC audio track");

    let video_dur = n as f64 / fps as f64; // 1.5s
    assert!(
        (v_dur - video_dur).abs() < 0.2,
        "video dur {v_dur:.3}s != nominal {video_dur:.3}s"
    );
    // Audio and video durations must agree within AAC framing and priming tolerance.
    assert!(
        (a_dur - v_dur).abs() < 0.2,
        "audio dur {a_dur:.3}s must match video {v_dur:.3}s (M2)"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn media_muxer_preserves_video_only_topology() {
    use valle_media::codec::{Muxer, VideoFrameTransport, probe_av};

    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("video-only.mp4");
    {
        let mut mux = Muxer::open_media_rational_with_transport(
            &out,
            64,
            48,
            30,
            1,
            false,
            None,
            false,
            Some(1),
            VideoFrameTransport::Cpu,
        )
        .expect("open video-only media muxer");
        for shade in [32, 96, 160, 224] {
            let mut frame = RgbaFrame::new(64, 48);
            frame.fill([shade, 20, 200, 255]);
            mux.encode_video(&frame).expect("encode video-only frame");
        }
        mux.finish().expect("finish video-only MP4");
    }

    let probe = probe_av(&out).expect("probe video-only MP4");
    assert!(probe.video.is_some(), "video-only MP4 must contain video");
    assert!(
        probe.audio.is_none(),
        "media tools must not invent an AAC stream for video-only input"
    );
}

#[test]
fn muxer_keeps_all_frames_at_fractional_second_boundary() {
    use valle_media::codec::{Muxer, decode_rgba_frames};

    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("fractional-boundary.mp4");
    let (w, h, fps, frame_count) = (64u32, 48u32, 30u32, 160usize);
    {
        let mut mux = Muxer::open(&out, w, h, fps, None).expect("open muxer");
        for i in 0..frame_count {
            let mut frame = RgbaFrame::new(w, h);
            frame.fill([20, (i % 256) as u8, 200, 255]);
            mux.encode_video(&frame).expect("encode video");
        }
        mux.finish().expect("finish");
    }

    let decoded = decode_rgba_frames(&out, None).expect("decode every frame");
    assert_eq!(
        decoded.len(),
        frame_count,
        "5⅓s @30fps must not discard the 160th frame at the MP4 edit boundary"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn timestamped_muxer_preserves_vfr_presentation_spacing() {
    use valle_media::codec::{Muxer, VideoFrameTransport, probe_av};

    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("vfr.mp4");
    let source_pts = [0_i64, 40, 100, 140];
    {
        let mut mux = Muxer::open_timestamped_with_transport(
            &out,
            64,
            48,
            valle_media::codec::TimeBase(1, 1_000),
            25,
            1,
            None,
            false,
            Some(1),
            VideoFrameTransport::Cpu,
        )
        .expect("open timestamped muxer");
        let durations = [40_i64, 60, 40, 55];
        for ((index, pts), duration) in source_pts.into_iter().enumerate().zip(durations) {
            let mut frame = RgbaFrame::new(64, 48);
            frame.fill([20, index as u8 * 40, 200, 255]);
            mux.encode_video_at_pts_with_duration(&frame, pts, Some(duration))
                .expect("encode VFR frame");
        }
        assert!((mux.video_duration_seconds().unwrap() - 0.195).abs() < 1e-12);
        mux.finish().expect("finish VFR output");
    }

    let (decoded_pts, output_time_base) = decoded_video_pts(&out);
    assert_eq!(decoded_pts.len(), source_pts.len());
    let decoded_seconds = decoded_pts
        .iter()
        .map(|pts| *pts as f64 * f64::from(output_time_base))
        .collect::<Vec<_>>();
    for (actual, expected_ms) in decoded_seconds.iter().zip(source_pts) {
        assert!(
            (*actual - expected_ms as f64 / 1_000.0).abs() <= f64::from(output_time_base),
            "decoded VFR PTS {actual:.9}s did not preserve {expected_ms}ms"
        );
    }
    let probe = probe_av(&out).expect("probe VFR output");
    let (_, _, video_duration) = probe.video.expect("VFR video stream");
    assert!(
        (video_duration - 0.195).abs() <= 0.001,
        "final declared VFR duration drifted: {video_duration:.9}s"
    );
    assert!(probe.audio.is_some(), "default muxer still carries AAC");
}

#[test]
fn timestamped_muxer_preserves_cfr_final_frame_duration_without_audio() {
    use valle_media::codec::{Muxer, VideoFrameTransport, probe_av};

    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("cfr-no-audio.mp4");
    {
        let mut mux = Muxer::open_timestamped_media_with_transport(
            &out,
            64,
            48,
            valle_media::codec::TimeBase(1, 4),
            4,
            1,
            false,
            None,
            false,
            Some(1),
            VideoFrameTransport::Cpu,
        )
        .expect("open timestamped CFR muxer");
        for pts in 0_i64..8 {
            let mut frame = RgbaFrame::new(64, 48);
            frame.fill([20, pts as u8 * 20, 200, 255]);
            mux.encode_video_at_pts_with_duration(&frame, pts, Some(1))
                .expect("encode CFR frame with exact duration");
        }
        assert!((mux.video_duration_seconds().unwrap() - 2.0).abs() < 1e-12);
        mux.finish().expect("finish timestamped CFR output");
    }

    let probe = probe_av(&out).expect("probe timestamped CFR output");
    let (_, _, video_duration) = probe.video.expect("CFR output has video");
    assert!(
        (video_duration - 2.0).abs() <= 0.001,
        "final CFR duration drifted: {video_duration:.9}s"
    );
    assert!(
        probe.audio.is_none(),
        "video-only input must remain video-only"
    );
}

fn decoded_video_pts(path: &std::path::Path) -> (Vec<i64>, valle_media::codec::TimeBase) {
    // Decode with the same ABI selected by Valle; preserve the independent decoded-frame check.
    macro_rules! decode {
        ($ff:path) => {{
            use $ff as ff;
            fn receive_video_pts(decoder: &mut ff::decoder::Video, output: &mut Vec<i64>) {
                let mut frame = ff::frame::Video::empty();
                while decoder.receive_frame(&mut frame).is_ok() {
                    output.push(
                        frame
                            .timestamp()
                            .or_else(|| frame.pts())
                            .expect("decoded video frame PTS"),
                    );
                }
            }

            ff::init().expect("initialize libav");
            let mut input = ff::format::input(path).expect("open encoded VFR output");
            let stream = input
                .streams()
                .best(ff::media::Type::Video)
                .expect("video stream");
            let stream_index = stream.index();
            let time_base = stream.time_base();
            let parameters = stream.parameters();
            let mut decoder = ff::codec::context::Context::from_parameters(parameters)
                .expect("video codec context")
                .decoder()
                .video()
                .expect("video decoder");
            let mut timestamps = Vec::new();
            for (stream, packet) in input.packets() {
                if stream.index() != stream_index {
                    continue;
                }
                decoder.send_packet(&packet).expect("send video packet");
                receive_video_pts(&mut decoder, &mut timestamps);
            }
            decoder.send_eof().expect("flush video decoder");
            receive_video_pts(&mut decoder, &mut timestamps);
            (
                timestamps,
                valle_media::codec::TimeBase(time_base.0, time_base.1),
            )
        }};
    }
    let capabilities =
        valle_media::codec::ffi::ffmpeg_capabilities().expect("selected media runtime");
    match capabilities["ffmpegMajor"].as_u64().unwrap() {
        7 => decode!(valle_ffmpeg::backend::v7),
        8 => decode!(valle_ffmpeg::backend::v8),
        9 => decode!(valle_ffmpeg::backend::v9),
        other => panic!("unexpected FFmpeg adapter {other}"),
    }
}

#[test]
fn hw_muxer_roundtrip_if_available() {
    use valle_media::codec::{Muxer, probe_av};
    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("hardware.mp4");
    // Use hardware encoding when available; otherwise require an explicit failure without software fallback.
    let mut mux = match Muxer::open_ext(&out, 64, 48, 30, None, true, None) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("skip hw test: {e}");
            return;
        }
    };
    let mut f = valle_media::frame::RgbaFrame::new(64, 48);
    f.fill([200, 30, 30, 255]);
    for _ in 0..15 {
        mux.encode_video(&f).expect("hw encode video");
    }
    mux.finish().expect("hw finish");
    let size = std::fs::metadata(&out).expect("mp4 exists").len();
    assert!(
        size > 1000,
        "hardware MP4 contains too little encoded data ({size} bytes)"
    );
    let p = probe_av(&out).expect("probe");
    assert_eq!(p.video.map(|(w, h, _)| (w, h)), Some((64, 48)));
    assert!(
        p.audio.is_some(),
        "hw path must keep the always-AAC invariant"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn audio_muxer_emits_single_aac_stream() {
    // Audio-only output contains one AAC stream, no video stream, and the expected sample duration.
    use valle_media::codec::{AudioMuxer, probe_av};
    use valle_media::frame::AudioBuffer;
    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("audio.mp4");
    {
        let mut mux = AudioMuxer::open(&out).expect("open audio muxer");
        let (rate, ch) = mux.audio_format();
        let frames = (rate as usize * 3) / 2; // 1.5s
        let mut samples = Vec::with_capacity(frames * ch as usize);
        for i in 0..frames {
            let v = (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / rate as f32).sin() * 0.3;
            for _ in 0..ch {
                samples.push(v);
            }
        }
        let buf = AudioBuffer {
            samples,
            sample_rate: rate,
            channels: ch,
        };
        mux.encode_audio(&buf).expect("encode audio");
        mux.finish().expect("finish");
    }
    let p = probe_av(&out).expect("probe");
    assert!(
        p.video.is_none(),
        "audio-only output must have no video stream: {p:?}"
    );
    let a_dur = p.audio.expect("expected AAC stream");
    assert!((a_dur - 1.5).abs() < 0.1, "audio dur {a_dur:.3}s != 1.5s");
    let _ = std::fs::remove_file(&out);
}
