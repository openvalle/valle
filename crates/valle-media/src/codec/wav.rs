//! Streaming IEEE-float WAV output for offline audio tools.

use std::{
    fs::File,
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

use anyhow::{Context, Result, anyhow, ensure};

use crate::frame::AudioBuffer;

const HEADER_BYTES: u64 = 44;

pub struct FloatWavWriter {
    writer: Option<BufWriter<File>>,
    sample_rate: u32,
    channels: u16,
    frames_written: u64,
}

impl FloatWavWriter {
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self> {
        ensure!(sample_rate > 0, "WAV sample rate must be greater than zero");
        ensure!(channels > 0, "WAV channel count must be greater than zero");
        let bytes_per_frame = u32::from(channels)
            .checked_mul(4)
            .ok_or_else(|| anyhow!("WAV channel count overflows block alignment"))?;
        ensure!(
            bytes_per_frame <= u32::from(u16::MAX),
            "WAV channel count overflows block alignment"
        );
        let byte_rate = sample_rate
            .checked_mul(bytes_per_frame)
            .ok_or_else(|| anyhow!("WAV sample rate overflows byte rate"))?;
        let file = File::create(path).with_context(|| format!("create WAV {}", path.display()))?;
        let mut writer = BufWriter::new(file);
        writer.write_all(b"RIFF")?;
        writer.write_all(&36_u32.to_le_bytes())?;
        writer.write_all(b"WAVEfmt ")?;
        writer.write_all(&16_u32.to_le_bytes())?;
        writer.write_all(&3_u16.to_le_bytes())?; // WAVE_FORMAT_IEEE_FLOAT
        writer.write_all(&channels.to_le_bytes())?;
        writer.write_all(&sample_rate.to_le_bytes())?;
        writer.write_all(&byte_rate.to_le_bytes())?;
        writer.write_all(&(bytes_per_frame as u16).to_le_bytes())?;
        writer.write_all(&32_u16.to_le_bytes())?;
        writer.write_all(b"data")?;
        writer.write_all(&0_u32.to_le_bytes())?;
        ensure!(
            writer.stream_position()? == HEADER_BYTES,
            "internal WAV header size mismatch"
        );
        Ok(Self {
            writer: Some(writer),
            sample_rate,
            channels,
            frames_written: 0,
        })
    }

    pub fn write(&mut self, buffer: &AudioBuffer) -> Result<()> {
        ensure!(
            buffer.sample_rate == self.sample_rate && buffer.channels == self.channels,
            "audio buffer {}Hz/{}ch does not match WAV {}Hz/{}ch",
            buffer.sample_rate,
            buffer.channels,
            self.sample_rate,
            self.channels
        );
        ensure!(
            buffer
                .samples
                .len()
                .is_multiple_of(usize::from(self.channels)),
            "audio buffer is not an integral number of interleaved frames"
        );
        let new_frames = self
            .frames_written
            .checked_add(buffer.frames() as u64)
            .ok_or_else(|| anyhow!("WAV frame count overflow"))?;
        let data_bytes = new_frames
            .checked_mul(u64::from(self.channels))
            .and_then(|samples| samples.checked_mul(4))
            .ok_or_else(|| anyhow!("WAV data size overflow"))?;
        ensure!(
            data_bytes <= u64::from(u32::MAX) - 36,
            "WAV exceeds the classic RIFF 4 GiB limit"
        );
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| anyhow!("WAV writer is already finished"))?;
        for sample in &buffer.samples {
            ensure!(sample.is_finite(), "cannot write non-finite WAV sample");
            writer.write_all(&sample.clamp(-1.0, 1.0).to_le_bytes())?;
        }
        self.frames_written = new_frames;
        Ok(())
    }

    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }

    pub fn finish(mut self) -> Result<u64> {
        let mut writer = self
            .writer
            .take()
            .ok_or_else(|| anyhow!("WAV writer is already finished"))?;
        let data_bytes = self
            .frames_written
            .checked_mul(u64::from(self.channels))
            .and_then(|samples| samples.checked_mul(4))
            .ok_or_else(|| anyhow!("WAV data size overflow"))? as u32;
        writer.flush().context("flush WAV samples")?;
        writer.seek(SeekFrom::Start(4))?;
        writer.write_all(&(36_u32 + data_bytes).to_le_bytes())?;
        writer.seek(SeekFrom::Start(40))?;
        writer.write_all(&data_bytes.to_le_bytes())?;
        writer.flush().context("flush WAV header")?;
        writer
            .get_ref()
            .sync_all()
            .context("sync completed WAV file")?;
        Ok(self.frames_written)
    }
}

/// Reopen and strictly validate the canonical float-WAV written by [`FloatWavWriter`].
///
/// The check intentionally uses the staged file on disk, rather than the writer's in-memory
/// counters. Besides the audio format and frame count, both RIFF length fields must describe the
/// exact filesystem length, so truncated data and unaccounted trailing bytes are rejected before
/// publication.
pub fn validate_float_wav(
    path: &Path,
    expected_sample_rate: u32,
    expected_channels: u16,
    expected_frames: u64,
) -> Result<()> {
    ensure!(
        expected_sample_rate > 0,
        "expected WAV sample rate must be greater than zero"
    );
    ensure!(
        expected_channels > 0,
        "expected WAV channel count must be greater than zero"
    );

    let mut file = File::open(path).with_context(|| format!("reopen WAV {}", path.display()))?;
    let file_len = file
        .metadata()
        .with_context(|| format!("read WAV metadata {}", path.display()))?
        .len();
    ensure!(
        file_len >= HEADER_BYTES,
        "WAV {} is truncated: {file_len} bytes",
        path.display()
    );
    let mut header = [0_u8; HEADER_BYTES as usize];
    file.read_exact(&mut header)
        .with_context(|| format!("read WAV header {}", path.display()))?;

    ensure!(&header[0..4] == b"RIFF", "WAV RIFF signature is missing");
    ensure!(&header[8..12] == b"WAVE", "WAV WAVE signature is missing");
    ensure!(&header[12..16] == b"fmt ", "WAV fmt chunk is missing");
    ensure!(
        u32_at(&header, 16) == 16,
        "WAV fmt chunk must contain the canonical 16-byte payload"
    );
    ensure!(
        u16_at(&header, 20) == 3,
        "WAV format must be IEEE float PCM"
    );

    let channels = u16_at(&header, 22);
    let sample_rate = u32_at(&header, 24);
    let byte_rate = u32_at(&header, 28);
    let block_align = u16_at(&header, 32);
    let bits_per_sample = u16_at(&header, 34);
    ensure!(
        channels == expected_channels,
        "WAV channel mismatch: found {channels}, expected {expected_channels}"
    );
    ensure!(
        sample_rate == expected_sample_rate,
        "WAV sample-rate mismatch: found {sample_rate}, expected {expected_sample_rate}"
    );
    ensure!(bits_per_sample == 32, "float WAV samples must be 32-bit");
    let expected_block_align = u32::from(expected_channels)
        .checked_mul(4)
        .ok_or_else(|| anyhow!("expected WAV block alignment overflow"))?;
    ensure!(
        u32::from(block_align) == expected_block_align,
        "WAV block alignment mismatch: found {block_align}, expected {expected_block_align}"
    );
    let expected_byte_rate = expected_sample_rate
        .checked_mul(expected_block_align)
        .ok_or_else(|| anyhow!("expected WAV byte rate overflow"))?;
    ensure!(
        byte_rate == expected_byte_rate,
        "WAV byte-rate mismatch: found {byte_rate}, expected {expected_byte_rate}"
    );
    ensure!(&header[36..40] == b"data", "WAV data chunk is missing");

    let riff_len = u64::from(u32_at(&header, 4)) + 8;
    ensure!(
        riff_len == file_len,
        "WAV RIFF length mismatch: header={riff_len}, file={file_len}"
    );
    let data_bytes = u64::from(u32_at(&header, 40));
    let described_file_len = HEADER_BYTES
        .checked_add(data_bytes)
        .ok_or_else(|| anyhow!("WAV file length overflow"))?;
    ensure!(
        described_file_len == file_len,
        "WAV data length mismatch: header describes {described_file_len} bytes, file has {file_len}"
    );
    ensure!(
        data_bytes.is_multiple_of(u64::from(block_align)),
        "WAV data is not an integral number of frames"
    );
    let frames = data_bytes / u64::from(block_align);
    ensure!(
        frames == expected_frames,
        "WAV frame mismatch: found {frames}, expected {expected_frames}"
    );
    let expected_data_bytes = expected_frames
        .checked_mul(expected_block_align.into())
        .ok_or_else(|| anyhow!("expected WAV data length overflow"))?;
    let expected_file_len = HEADER_BYTES
        .checked_add(expected_data_bytes)
        .ok_or_else(|| anyhow!("expected WAV file length overflow"))?;
    ensure!(
        file_len == expected_file_len,
        "WAV file-length mismatch: found {file_len}, expected {expected_file_len}"
    );
    Ok(())
}

fn u16_at(header: &[u8; HEADER_BYTES as usize], offset: usize) -> u16 {
    u16::from_le_bytes([header[offset], header[offset + 1]])
}

fn u32_at(header: &[u8; HEADER_BYTES as usize], offset: usize) -> u32 {
    u32::from_le_bytes([
        header[offset],
        header[offset + 1],
        header[offset + 2],
        header[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_float_wav_preserves_interleaved_samples() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("audio.wav");
        let mut writer = FloatWavWriter::create(&path, 16_000, 2).unwrap();
        writer
            .write(&AudioBuffer {
                samples: vec![0.25, -0.25, 0.5, -0.5],
                sample_rate: 16_000,
                channels: 2,
            })
            .unwrap();
        assert_eq!(writer.finish().unwrap(), 2);

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            16_000
        );
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 16);
        let samples = bytes[HEADER_BYTES as usize..]
            .chunks_exact(4)
            .map(|sample| f32::from_le_bytes(sample.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(samples, vec![0.25, -0.25, 0.5, -0.5]);
        validate_float_wav(&path, 16_000, 2, 2).unwrap();
    }

    #[test]
    fn staged_wav_validation_rejects_format_and_file_length_mismatches() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("audio.wav");
        let mut writer = FloatWavWriter::create(&path, 48_000, 1).unwrap();
        writer
            .write(&AudioBuffer {
                samples: vec![0.25; 8],
                sample_rate: 48_000,
                channels: 1,
            })
            .unwrap();
        writer.finish().unwrap();

        assert!(validate_float_wav(&path, 44_100, 1, 8).is_err());
        assert!(validate_float_wav(&path, 48_000, 2, 4).is_err());
        assert!(validate_float_wav(&path, 48_000, 1, 7).is_err());

        let file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        (&file).write_all(&[0]).unwrap();
        file.sync_all().unwrap();
        assert!(validate_float_wav(&path, 48_000, 1, 8).is_err());
    }
}
