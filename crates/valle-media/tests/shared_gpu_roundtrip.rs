#![cfg(all(feature = "libav", target_os = "macos"))]
//! Explicit hardware ownership proof; run separately with each FFmpeg installation.
use std::ffi::c_void;
use valle_media::{
    LibavVideoSource, Muxer, SharedVideoFrameHandle, VideoFrameTransport, VideoSource,
};
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut c_void) -> usize;
}
#[test]
#[ignore = "requires a real VideoToolbox encoder/decoder and shared hardware frames"]
fn shared_frames_keep_their_abi_and_survive_decoder_drop() {
    let frame_backend = std::hint::black_box(
        valle_media::SharedVideoFrame::backend
            as fn(&valle_media::SharedVideoFrame) -> valle_media::SharedFrameBackend,
    );
    let frame_dimensions = std::hint::black_box(
        valle_media::SharedVideoFrame::dimensions
            as fn(&valle_media::SharedVideoFrame) -> (u32, u32),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.mp4");
    let mut muxer = Muxer::open_ext_with_transport(
        &path,
        64,
        48,
        30,
        None,
        true,
        None,
        VideoFrameTransport::SharedGpu,
    )
    .unwrap();
    let pool = muxer.shared_frame_pool().expect("shared GPU frame pool");
    assert_eq!(
        pool.backend(),
        valle_media::SharedFrameBackend::VideoToolbox
    );
    assert_eq!(pool.dimensions(), (64, 48));
    let clone_pool = std::hint::black_box(
        <valle_media::SharedVideoFramePool as Clone>::clone
            as fn(&valle_media::SharedVideoFramePool) -> valle_media::SharedVideoFramePool,
    );
    let retained_pool = clone_pool(&pool);
    assert_eq!(retained_pool.dimensions(), pool.dimensions());
    for _ in 0..6 {
        let frame = retained_pool.acquire().unwrap();
        assert_eq!(frame_backend(&frame), pool.backend());
        assert_eq!(frame_dimensions(&frame), pool.dimensions());
        let SharedVideoFrameHandle::VideoToolbox(buffer) = frame.handle() else {
            panic!("expected VideoToolbox surface")
        };
        // The borrowed CVPixelBuffer belongs to frame and remains alive through submission.
        unsafe {
            assert_eq!(CVPixelBufferLockBaseAddress(buffer, 0), 0);
            let data = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
            assert!(!data.is_null());
            let stride = CVPixelBufferGetBytesPerRow(buffer);
            for y in 0..48 {
                for x in 0..64 {
                    std::ptr::copy_nonoverlapping(
                        [0u8, 0, 255, 255].as_ptr(),
                        data.add(y * stride + x * 4),
                        4,
                    );
                }
            }
            assert_eq!(CVPixelBufferUnlockBaseAddress(buffer, 0), 0);
        }
        muxer.encode_shared(frame).unwrap();
    }
    muxer.finish().unwrap();
    drop(muxer);
    drop(pool);
    drop(retained_pool);
    let mut decoder =
        LibavVideoSource::open_with_transport(&path, VideoFrameTransport::SharedGpu).unwrap();
    let frame = decoder.frame_at_lazy(0.0).unwrap();
    assert!(
        frame.decoded_gpu().is_some(),
        "decode must retain a real hardware surface"
    );
    let gpu = frame.decoded_gpu().unwrap();
    assert_eq!(gpu.backend(), valle_media::SharedFrameBackend::VideoToolbox);
    assert_eq!(gpu.dimensions(), (64, 48));
    assert_eq!(gpu.coded_dimensions(), (64, 48));
    assert_eq!(gpu.rotation_deg(), 0);
    assert!(gpu.supports_direct_import());
    drop(decoder);
    let rgba = frame.rgba().unwrap();
    let pixel = rgba.pixel(32, 24).unwrap();
    assert!(
        pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60,
        "red frame was corrupted: {pixel:?}"
    );
}
