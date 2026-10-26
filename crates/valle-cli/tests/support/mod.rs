use std::sync::OnceLock;
use valle_engine::compositor::reference::{OUTPUT_DITHER_SIZE, output_dither_pixels};

/// Expected terminal bytes for an exact sRGB solid, including delivery's fixed spatial noise.
pub fn srgb_delivery_pixel(mut rgba: [u8; 4], x: usize, y: usize) -> [u8; 4] {
    if rgba[3] == 0 {
        return [0; 4];
    }
    static NOISE: OnceLock<Vec<u8>> = OnceLock::new();
    let noise = NOISE.get_or_init(|| output_dither_pixels(0));
    let size = OUTPUT_DITHER_SIZE as usize;
    let at = ((y % size) * size + x % size) * 4;
    for channel in 0..3 {
        let offset = f32::from(noise[at + channel]) * (2.0 / 255.0) - 1.0;
        rgba[channel] = (f32::from(rgba[channel]) + offset)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    rgba
}
