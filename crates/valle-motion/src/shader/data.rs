use std::io::Cursor;

/// Eight byte planes, carried in alpha so both backends leave values untouched.
pub const DATA_TEXTURE_BYTES_PER_PIXEL: u64 = 8;
pub const MAX_DATA_TEXTURE_BYTES: u64 = 64 * 1024 * 1024;

pub fn data_texture_storage_bytes(width: u32, height: u32) -> Result<u64, String> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("data texture dimensions must be in 1..=4096".into());
    }
    let bytes = u64::from(width) * u64::from(height) * DATA_TEXTURE_BYTES_PER_PIXEL;
    if bytes > MAX_DATA_TEXTURE_BYTES {
        return Err("decoded data texture exceeds 64 MiB".into());
    }
    Ok(bytes)
}

/// Decode normalized PNG/JPEG channels without color, alpha or orientation processing.
/// Storage is a 2w × 4h Alpha8 image. Each row of quadrants holds one source
/// channel's high and low bytes. Reconstructing the normalized 16-bit value in
/// float arithmetic avoids losing precision through RuntimeEffect's half4 children.
pub fn decode_data_texture(encoded: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let bytes = data_texture_storage_bytes(width, height)?;
    if encoded.len() as u64 > MAX_DATA_TEXTURE_BYTES {
        return Err("encoded data texture exceeds 64 MiB".into());
    }
    let format = image::guess_format(encoded).map_err(|e| e.to_string())?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err("data textures require PNG or JPEG".into());
    }
    let reader = || {
        let mut reader = image::ImageReader::with_format(Cursor::new(encoded), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(MAX_DATA_TEXTURE_BYTES);
        reader.limits(limits);
        reader
    };
    if reader().into_dimensions().map_err(|e| e.to_string())? != (width, height) {
        return Err("data texture dimensions differ from the resource descriptor".into());
    }
    let rgba = reader().decode().map_err(|e| e.to_string())?.into_rgba16();
    let mut packed = vec![0; bytes as usize];
    let (w, h) = (width as usize, height as usize);
    for (index, pixel) in rgba.pixels().enumerate() {
        let (x, y) = (index % w, index / w);
        for channel in 0..4 {
            let offset = (y + channel * h) * (2 * w) + x;
            let [hi, lo] = pixel.0[channel].to_be_bytes();
            packed[offset] = hi;
            packed[offset + w] = lo;
        }
    }
    Ok(packed)
}
