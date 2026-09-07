#![cfg(feature = "libav")]
//! Bounded audio reads and exact forward catch-up.

use valle_media::frame::RgbaFrame;

#[test]
fn forward_audio_stream_reads_bounded_chunks_until_the_actual_end() {
    use valle_media::codec::{AudioMuxer, LibavAudioStream};
    use valle_media::frame::AudioBuffer;

    let temp = tempfile::tempdir().expect("temp dir");
    let output = temp.path().join("stream-source.mp4");
    {
        let mut muxer = AudioMuxer::open(&output).expect("open audio muxer");
        let (sample_rate, channels) = muxer.audio_format();
        let frames = sample_rate as usize / 2;
        muxer
            .encode_audio(&AudioBuffer {
                samples: vec![0.125; frames * usize::from(channels)],
                sample_rate,
                channels,
            })
            .expect("encode audio");
        muxer.finish().expect("finish audio");
    }

    let mut stream = LibavAudioStream::open(&output, 16_000, 1).expect("open bounded stream");
    assert_eq!(stream.total_frames(), None);
    let mut read = 0_u64;
    loop {
        let chunk = stream.read(997).expect("read bounded chunk");
        assert!(chunk.frames() <= 997);
        if chunk.frames() == 0 {
            break;
        }
        read += chunk.frames() as u64;
    }
    assert_eq!(stream.total_frames(), Some(read));
    assert_eq!(stream.position(), read);
    assert_eq!(stream.read(997).unwrap().frames(), 0);
    assert!(stream.read(0).is_err());
}

#[test]
fn audio_stream_catchup_bounded_and_exact() {
    // A cold read deep into the source must match forward playback with bounded buffering.
    use valle_media::codec::{AudioSource, LibavAudioSource, Muxer};
    use valle_media::frame::AudioBuffer;
    let temp = tempfile::tempdir().expect("temp dir");
    let out = temp.path().join("audio-catchup.mp4");
    {
        let (w, h, fps, rate) = (64u32, 48u32, 30u32, 48000u32);
        let mut mux = Muxer::open_ext(&out, w, h, fps, None, false, Some(2)).expect("open muxer");
        mux.expect_external_audio();
        let spf = rate / fps; // 1600 samples per frame.
        let f = RgbaFrame::new(w, h);
        for i in 0..(fps as i64 * 12) {
            mux.encode_video(&f).expect("video");
            let mut s = Vec::with_capacity(spf as usize * 2);
            for k in 0..spf {
                let n = (i * spf as i64 + k as i64) as f64;
                s.push(((n * 0.03).sin() * 0.5) as f32);
                s.push(((n * 0.011).sin() * 0.4) as f32);
            }
            mux.encode_audio(&AudioBuffer {
                samples: s,
                sample_rate: rate,
                channels: 2,
            })
            .expect("audio");
        }
        mux.finish().expect("finish");
    }
    // Reference: consume each video-frame interval up to 10s, then read [10, 10.5).
    let mut lin = LibavAudioSource::open(&out, 48000, 2).expect("open lin");
    for i in 0..300 {
        let _ = lin.samples(i as f64 / 30.0, 1.0 / 30.0).expect("advance");
    }
    let want = lin.samples(10.0, 0.5).expect("lin chunk");
    // Cold catch-up: read [10, 10.5) from a fresh source.
    let mut cold = LibavAudioSource::open(&out, 48000, 2).expect("open cold");
    let got = cold.samples(10.0, 0.5).expect("cold chunk");
    assert_eq!(
        got.samples, want.samples,
        "cold catch-up must match sequential consumption bit for bit"
    );
    assert!(
        cold.buf_high_water() < 48000 * 2,
        "catch-up buffering must be O(chunk); observed high-water mark: {} samples",
        cold.buf_high_water()
    );
    let _ = std::fs::remove_file(&out);
}
