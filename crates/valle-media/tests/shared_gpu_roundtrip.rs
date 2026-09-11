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
    for _ in 0..6 {
        let frame = pool.acquire().unwrap();
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
    let mut decoder =
        LibavVideoSource::open_with_transport(&path, VideoFrameTransport::SharedGpu).unwrap();
    let frame = decoder.frame_at_lazy(0.0).unwrap();
    assert!(
        frame.decoded_gpu().is_some(),
        "decode must retain a real hardware surface"
    );
    drop(decoder);
    let rgba = frame.rgba().unwrap();
    let pixel = rgba.pixel(32, 24).unwrap();
    assert!(
        pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60,
        "red frame was corrupted: {pixel:?}"
    );
}
