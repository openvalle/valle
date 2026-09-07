#![cfg(feature = "libav")]
//! Libav video roundtrips, decoder progression and deterministic encoding.
use valle_media::codec::{Encoder, Mp4Encoder, decode_rgba_frames, probe_dimensions};
use valle_media::frame::RgbaFrame;

#[test]
fn transparent_mov_roundtrips_rgb_and_alpha_losslessly() {
    use valle_media::codec::{TransparentVideoMuxer, probe_av};

    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("transparent.mov");
    let source = RgbaFrame {
        width: 4,
        height: 2,
        data: vec![
            255, 0, 0, 0, 0, 255, 0, 32, 0, 0, 255, 128, 240, 120, 30, 255, 7, 11, 13, 1, 17, 19,
            23, 64, 29, 31, 37, 192, 41, 43, 47, 254,
        ],
    };
    {
        let mut mux =
            TransparentVideoMuxer::open(&out, 4, 2, 5, 2).expect("open transparent qtrle muxer");
        mux.encode_video(&source).expect("encode first frame");
        mux.encode_video(&source).expect("encode second frame");
        mux.finish().expect("finish transparent MOV");
    }

    let probe = probe_av(&out).expect("probe transparent MOV");
    assert!(
        probe.video.is_some(),
        "transparent MOV must be an ordinary video stream"
    );
    assert!(
        probe.audio.is_none(),
        "matte preprocessing must not invent an audio track"
    );
    let frames = decode_rgba_frames(&out, None).expect("decode transparent MOV");
    assert_eq!(frames, vec![source.clone(), source]);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn encode_then_decode_roundtrip() {
    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("roundtrip.mp4");
    let (w, h) = (64u32, 48u32);

    {
        let mut enc = Mp4Encoder::new(&out, w, h, 30, None).expect("create encoder");
        for _ in 0..30 {
            let mut f = RgbaFrame::new(w, h);
            f.fill([220, 20, 20, 255]); // Red.
            enc.encode_frame(&f).expect("encode frame");
        }
        enc.finish().expect("finish");
    }

    // Require actual encoded frames rather than a 262-byte empty MP4 container.
    let size = std::fs::metadata(&out).expect("mp4 exists").len();
    assert!(
        size > 1000,
        "mp4 suspiciously small ({size} bytes) — likely empty/broken"
    );

    assert_eq!(probe_dimensions(&out).expect("probe"), (w, h));

    let frames = decode_rgba_frames(&out, None).expect("decode");
    assert!(
        frames.len() >= 10,
        "expected ~30 decoded frames, got {}",
        frames.len()
    );
    assert_eq!(frames[0].width, w);
    assert_eq!(frames[0].height, h);

    // Lossy RGBA/YUV/H.264 conversion should retain a strong red channel and low green/blue channels.
    let px = frames[0].pixel(0, 0).expect("pixel");
    assert!(
        px[0] > 150 && px[1] < 100 && px[2] < 100,
        "first pixel not red: {px:?}"
    );

    let _ = std::fs::remove_file(&out);
}

#[test]
fn exhausted_source_does_not_reseek_on_continued_forward() {
    // After EOF, forward reads must reuse the same final-frame Arc without seeking or decoding again.
    use std::sync::Arc;
    use valle_media::codec::{LibavVideoSource, VideoSource};
    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("still-short.mp4");
    let (w, h) = (64u32, 48u32);
    {
        // Use a short multi-frame source and sample well beyond its end to allow encoder lookahead and timing variation.
        let mut enc = Mp4Encoder::new(&out, w, h, 30, None).expect("encoder");
        for _ in 0..6 {
            let mut f = RgbaFrame::new(w, h);
            f.fill([30, 200, 60, 255]);
            enc.encode_frame(&f).expect("encode frame");
        }
        enc.finish().expect("finish");
    }
    let mut src = LibavVideoSource::open(&out).expect("open source");
    let _ = src.frame_at(0.0).expect("frame at 0");
    // Advance naturally to EOF without crossing the forward-seek threshold.
    let b = src.frame_at(0.9).expect("frame at 0.9 (drives to EOF)");
    // Large forward jumps after EOF must still reuse the final-frame Arc.
    let c = src.frame_at(2.0).expect("frame at 2.0");
    let d = src.frame_at(3.0).expect("frame at 3.0");
    assert!(
        Arc::ptr_eq(&b, &c) && Arc::ptr_eq(&c, &d),
        "forward reads after EOF must reuse the final frame pointer"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn rejects_odd_dimensions() {
    // YUV420P requires even dimensions.
    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("odd.mp4");
    assert!(Mp4Encoder::new(&out, 63, 48, 30, None).is_err());
}

#[test]
fn pinned_encode_threads_bytes_reproducible() {
    // Fixed encoder threads make container bytes reproducible across repeated runs.
    use valle_media::codec::Muxer;
    let encode = |out: &std::path::Path| {
        let (w, h, fps, n) = (64u32, 48u32, 30u32, 40i64);
        let mut mux = Muxer::open_ext(out, w, h, fps, None, false, Some(2)).expect("open muxer");
        for i in 0..n {
            // Vary frames to exercise inter-frame prediction.
            let mut f = RgbaFrame::new(w, h);
            for (p, px) in f.data.chunks_exact_mut(4).enumerate() {
                let v = ((p as i64 * 7 + i * 13) % 251) as u8;
                px.copy_from_slice(&[v, v.wrapping_add(80), 200u8.wrapping_sub(v), 255]);
            }
            mux.encode_video(&f).expect("encode video");
        }
        mux.finish().expect("finish");
    };
    let temp = tempfile::tempdir().expect("temp dir");
    let a = temp.path().join("pin-threads-a.mp4");
    let b = temp.path().join("pin-threads-b.mp4");
    encode(&a);
    encode(&b);
    let (ba, bb) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    assert!(ba.len() > 1000, "mp4 too small ({})", ba.len());
    assert_eq!(ba, bb, "fixed-thread encoding must produce identical bytes");
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
}

#[test]
fn yuv_pool_recycle_bytes_identical() {
    // Recycled frames must preserve bytes and actually return to the software frame pool.
    use valle_media::codec::{Muxer, RgbaToYuv};
    let (w, h, fps, n) = (64u32, 48u32, 30u32, 40i64);
    let encode = |out: &std::path::Path, recycle: bool| -> usize {
        let mut mux = Muxer::open_ext(out, w, h, fps, None, false, Some(2)).expect("open muxer");
        let mut conv = RgbaToYuv::new(w, h).expect("converter");
        let mut pool = None;
        let mut reused = 0usize;
        for i in 0..n {
            let mut f = RgbaFrame::new(w, h);
            for (p, px) in f.data.chunks_exact_mut(4).enumerate() {
                let v = ((p as i64 * 7 + i * 13) % 251) as u8;
                px.copy_from_slice(&[v, v.wrapping_add(80), 200u8.wrapping_sub(v), 255]);
            }
            let recycled = if recycle { pool.take() } else { None };
            reused += recycled.is_some() as usize;
            let yuv = conv.convert(&f, recycled).expect("convert");
            pool = mux.encode_yuv(yuv).expect("encode yuv");
        }
        mux.finish().expect("finish");
        reused
    };
    let temp = tempfile::tempdir().expect("temp dir");
    let a = temp.path().join("yuv-pool-a.mp4");
    let b = temp.path().join("yuv-pool-b.mp4");
    encode(&a, false);
    let reused = encode(&b, true);
    assert!(
        reused >= (n as usize) - 2,
        "software buffers should be reused for nearly every frame ({reused}/{n})"
    );
    let (ba, bb) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    assert!(ba.len() > 1000, "mp4 too small ({})", ba.len());
    assert_eq!(
        ba, bb,
        "pooled and fresh allocation must produce identical bytes"
    );
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
}
