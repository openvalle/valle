//! Decode-once PCM workspace for multi-stage audio tools.
//!
//! Short inputs stay in memory. Inputs above the job memory budget are written to a private raw
//! PCM spool and then read back in bounded windows. Dropping the workspace removes the spool on
//! success, failure, or cancellation.

use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::frame::AudioBuffer;

use super::{CancellationToken, ResourcePolicy, ToolError, ToolErrorCode};

const DECODE_WINDOW_FRAMES: usize = 16_384;
const SPOOL_WRITE_SAMPLES: usize = 16_384;
const SAMPLE_BYTES: u64 = size_of::<f32>() as u64;
static SPOOL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

trait DecodedAudioSource {
    fn read(&mut self, max_frames: usize) -> anyhow::Result<AudioBuffer>;
    fn total_frames(&self) -> Option<u64>;
}

impl DecodedAudioSource for crate::codec::LibavAudioStream {
    fn read(&mut self, max_frames: usize) -> anyhow::Result<AudioBuffer> {
        self.read(max_frames)
    }

    fn total_frames(&self) -> Option<u64> {
        self.total_frames()
    }
}

pub(super) struct AudioWorkspace {
    sample_rate: u32,
    channels: u16,
    total_frames: u64,
    storage: AudioStorage,
}

enum AudioStorage {
    Memory(Vec<f32>),
    Spool(SpoolFile),
}

struct SpoolFile {
    path: PathBuf,
    file: File,
}

impl AudioWorkspace {
    #[cfg(any(all(feature = "tool-transcribe", not(target_os = "windows")), test))]
    pub fn decode(
        input: &Path,
        sample_rate: u32,
        channels: u16,
        policy: &ResourcePolicy,
        cancellation: &CancellationToken,
    ) -> Result<Self, ToolError> {
        Self::decode_with_policy(input, sample_rate, channels, policy, cancellation, false)
    }

    /// Force duration-independent RAM usage for two-pass algorithms such as Demucs.
    #[cfg(feature = "tool-separate")]
    pub fn decode_spooled(
        input: &Path,
        sample_rate: u32,
        channels: u16,
        policy: &ResourcePolicy,
        cancellation: &CancellationToken,
    ) -> Result<Self, ToolError> {
        Self::decode_with_policy(input, sample_rate, channels, policy, cancellation, true)
    }

    fn decode_with_policy(
        input: &Path,
        sample_rate: u32,
        channels: u16,
        policy: &ResourcePolicy,
        cancellation: &CancellationToken,
        force_spool: bool,
    ) -> Result<Self, ToolError> {
        let mut decoder = crate::codec::LibavAudioStream::open(input, sample_rate, channels)
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::InvalidInput,
                    format!("decode {}: {error:#}", input.display()),
                )
            })?;
        Self::decode_source(
            &mut decoder,
            input,
            sample_rate,
            channels,
            policy,
            cancellation,
            force_spool,
        )
    }

    fn decode_source(
        decoder: &mut impl DecodedAudioSource,
        input: &Path,
        sample_rate: u32,
        channels: u16,
        policy: &ResourcePolicy,
        cancellation: &CancellationToken,
        force_spool: bool,
    ) -> Result<Self, ToolError> {
        // The source duration hint is intentionally not used for resource decisions: an overlong
        // hint must not force a short input onto an insufficient disk budget, and an underlong hint
        // must not let PCM grow past the memory ceiling. Non-forced workspaces spill dynamically.
        let storage = if force_spool {
            AudioStorage::Spool(SpoolFile::create(&policy.temporary_directory)?)
        } else {
            AudioStorage::Memory(Vec::new())
        };

        let mut workspace = Self {
            sample_rate,
            channels,
            total_frames: 0,
            storage,
        };
        loop {
            if cancellation.is_cancelled() {
                return Err(ToolError::cancelled());
            }
            let buffer = decoder.read(DECODE_WINDOW_FRAMES).map_err(|error| {
                ToolError::new(
                    ToolErrorCode::InvalidInput,
                    format!("decode {}: {error:#}", input.display()),
                )
            })?;
            if buffer.frames() == 0 {
                break;
            }
            workspace.append(&buffer, policy)?;
        }
        let decoded_frames = decoder.total_frames().ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Internal,
                "audio decoder returned EOF without recording its frame count",
            )
        })?;
        if decoded_frames != workspace.total_frames {
            return Err(ToolError::new(
                ToolErrorCode::Internal,
                format!(
                    "audio decoder length mismatch: stream={decoded_frames}, workspace={}",
                    workspace.total_frames
                ),
            ));
        }
        workspace.finish_decode()?;
        Ok(workspace)
    }

    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    #[cfg(feature = "tool-separate")]
    pub const fn channels(&self) -> u16 {
        self.channels
    }

    pub const fn total_frames(&self) -> u64 {
        self.total_frames
    }

    #[cfg(test)]
    pub const fn is_spooled(&self) -> bool {
        matches!(self.storage, AudioStorage::Spool(_))
    }

    #[cfg(test)]
    fn resident_sample_capacity(&self) -> usize {
        match &self.storage {
            AudioStorage::Memory(samples) => samples.capacity(),
            AudioStorage::Spool(_) => 0,
        }
    }

    /// Read `[start_frame, end_frame)` without retaining any other part of a spooled track.
    pub fn read_frames(
        &mut self,
        start_frame: u64,
        end_frame: u64,
    ) -> Result<AudioBuffer, ToolError> {
        if start_frame > end_frame || end_frame > self.total_frames {
            return Err(ToolError::invalid_input(format!(
                "audio workspace range [{start_frame}, {end_frame}) is outside [0, {})",
                self.total_frames
            )));
        }
        let frames = end_frame - start_frame;
        let sample_count_u64 = frames
            .checked_mul(u64::from(self.channels))
            .ok_or_else(|| size_error("audio workspace range overflow"))?;
        let sample_count = usize::try_from(sample_count_u64)
            .map_err(|_| size_error("audio workspace range exceeds addressable memory"))?;
        let samples = match &mut self.storage {
            AudioStorage::Memory(samples) => {
                let start = usize::try_from(
                    start_frame
                        .checked_mul(u64::from(self.channels))
                        .ok_or_else(|| size_error("audio workspace offset overflow"))?,
                )
                .map_err(|_| size_error("audio workspace offset exceeds addressable memory"))?;
                let end = start
                    .checked_add(sample_count)
                    .ok_or_else(|| size_error("audio workspace range overflow"))?;
                samples
                    .get(start..end)
                    .ok_or_else(|| size_error("audio workspace memory is truncated"))?
                    .to_vec()
            }
            AudioStorage::Spool(spool) => {
                spool.read_samples(start_frame, self.channels, sample_count)?
            }
        };
        Ok(AudioBuffer {
            samples,
            sample_rate: self.sample_rate,
            channels: self.channels,
        })
    }

    fn append(&mut self, buffer: &AudioBuffer, policy: &ResourcePolicy) -> Result<(), ToolError> {
        if buffer.sample_rate != self.sample_rate || buffer.channels != self.channels {
            return Err(ToolError::new(
                ToolErrorCode::Internal,
                "audio decoder changed the workspace output format",
            ));
        }
        if !buffer
            .samples
            .len()
            .is_multiple_of(usize::from(self.channels))
        {
            return Err(ToolError::new(
                ToolErrorCode::Internal,
                "audio decoder returned a partial interleaved frame",
            ));
        }
        let next_frames = self
            .total_frames
            .checked_add(buffer.frames() as u64)
            .ok_or_else(|| size_error("decoded audio frame count overflow"))?;
        let next_bytes = pcm_bytes(next_frames, self.channels)?;
        let spill_memory = matches!(&self.storage, AudioStorage::Memory(_))
            && next_bytes > policy.audio_memory_budget_bytes;

        if spill_memory {
            if next_bytes > policy.temporary_disk_budget_bytes {
                return Err(disk_budget_error(
                    next_bytes,
                    policy.temporary_disk_budget_bytes,
                ));
            }
            let mut spool = SpoolFile::create(&policy.temporary_directory)?;
            if let AudioStorage::Memory(samples) = &self.storage {
                spool.write_samples(samples)?;
            }
            spool.write_samples(&buffer.samples)?;
            self.storage = AudioStorage::Spool(spool);
        } else {
            match &mut self.storage {
                AudioStorage::Memory(samples) => samples.extend_from_slice(&buffer.samples),
                AudioStorage::Spool(spool) => {
                    if next_bytes > policy.temporary_disk_budget_bytes {
                        return Err(disk_budget_error(
                            next_bytes,
                            policy.temporary_disk_budget_bytes,
                        ));
                    }
                    spool.write_samples(&buffer.samples)?;
                }
            }
        }
        self.total_frames = next_frames;
        Ok(())
    }

    fn finish_decode(&mut self) -> Result<(), ToolError> {
        let expected_samples = self
            .total_frames
            .checked_mul(u64::from(self.channels))
            .ok_or_else(|| size_error("decoded audio sample count overflow"))?;
        match &mut self.storage {
            AudioStorage::Memory(samples) => {
                if samples.len() as u64 != expected_samples {
                    return Err(size_error(format!(
                        "decoded PCM has {} samples, expected {expected_samples}",
                        samples.len()
                    )));
                }
            }
            AudioStorage::Spool(spool) => {
                spool
                    .file
                    .flush()
                    .map_err(|error| spool_error(&spool.path, error))?;
                let bytes = spool
                    .file
                    .metadata()
                    .map_err(|error| spool_error(&spool.path, error))?
                    .len();
                let expected_bytes = expected_samples
                    .checked_mul(SAMPLE_BYTES)
                    .ok_or_else(|| size_error("decoded PCM byte count overflow"))?;
                if bytes != expected_bytes {
                    return Err(size_error(format!(
                        "decoded PCM spool has {bytes} bytes, expected {expected_bytes}"
                    )));
                }
            }
        }
        Ok(())
    }
}

impl SpoolFile {
    fn create(directory: &Path) -> Result<Self, ToolError> {
        for _ in 0..64 {
            let sequence = SPOOL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!(
                ".valle-audio-spool-{}-{sequence}.pcmf32le",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.create_new(true).read(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => return Ok(Self { path, file }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(spool_error(&path, error)),
            }
        }
        Err(ToolError::new(
            ToolErrorCode::ResourceBusy,
            format!(
                "could not allocate a unique audio spool in {}",
                directory.display()
            ),
        ))
    }

    fn write_samples(&mut self, samples: &[f32]) -> Result<(), ToolError> {
        // Keep serialization scratch independent of the decoded-track length. In particular,
        // spilling an in-memory workspace must not retain the full `Vec<f32>` while allocating an
        // equally large `Vec<u8>`.
        let mut bytes = [0_u8; SPOOL_WRITE_SAMPLES * size_of::<f32>()];
        for samples in samples.chunks(SPOOL_WRITE_SAMPLES) {
            for (target, sample) in bytes.chunks_exact_mut(size_of::<f32>()).zip(samples) {
                target.copy_from_slice(&sample.to_le_bytes());
            }
            let byte_len = samples
                .len()
                .checked_mul(size_of::<f32>())
                .ok_or_else(|| size_error("PCM spool write size overflow"))?;
            self.file
                .write_all(&bytes[..byte_len])
                .map_err(|error| spool_error(&self.path, error))?;
        }
        Ok(())
    }

    fn read_samples(
        &mut self,
        start_frame: u64,
        channels: u16,
        sample_count: usize,
    ) -> Result<Vec<f32>, ToolError> {
        let start_sample = start_frame
            .checked_mul(u64::from(channels))
            .ok_or_else(|| size_error("PCM spool read offset overflow"))?;
        let byte_offset = start_sample
            .checked_mul(SAMPLE_BYTES)
            .ok_or_else(|| size_error("PCM spool read offset overflow"))?;
        self.file
            .seek(SeekFrom::Start(byte_offset))
            .map_err(|error| spool_error(&self.path, error))?;
        let byte_len = sample_count
            .checked_mul(size_of::<f32>())
            .ok_or_else(|| size_error("PCM spool read size overflow"))?;
        let mut bytes = vec![0_u8; byte_len];
        self.file
            .read_exact(&mut bytes)
            .map_err(|error| spool_error(&self.path, error))?;
        Ok(bytes
            .chunks_exact(size_of::<f32>())
            .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .collect())
    }
}

impl Drop for SpoolFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn pcm_bytes(frames: u64, channels: u16) -> Result<u64, ToolError> {
    frames
        .checked_mul(u64::from(channels))
        .and_then(|samples| samples.checked_mul(SAMPLE_BYTES))
        .ok_or_else(|| size_error("decoded PCM byte count overflow"))
}

fn size_error(message: impl Into<String>) -> ToolError {
    ToolError::new(ToolErrorCode::ResourceBusy, message)
}

fn disk_budget_error(required: u64, budget: u64) -> ToolError {
    ToolError::new(
        ToolErrorCode::ResourceBusy,
        format!(
            "decoded PCM needs {required} bytes, exceeding the temporary-disk budget of {budget} bytes"
        ),
    )
}

fn spool_error(path: &Path, error: std::io::Error) -> ToolError {
    ToolError::new(
        ToolErrorCode::ResourceBusy,
        format!("audio spool {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SyntheticSource {
        remaining_frames: usize,
        decoded_frames: u64,
        chunk_frames: usize,
        cancellation_after_first_read: Option<CancellationToken>,
    }

    impl DecodedAudioSource for SyntheticSource {
        fn read(&mut self, max_frames: usize) -> anyhow::Result<AudioBuffer> {
            if self.remaining_frames == 0 {
                return Ok(AudioBuffer::silence(16_000, 1, 0));
            }
            let frames = self.remaining_frames.min(self.chunk_frames).min(max_frames);
            self.remaining_frames -= frames;
            self.decoded_frames += frames as u64;
            if let Some(cancellation) = self.cancellation_after_first_read.take() {
                cancellation.cancel();
            }
            Ok(AudioBuffer {
                samples: (0..frames)
                    .map(|index| (index as f32 / frames as f32).mul_add(2.0, -1.0))
                    .collect(),
                sample_rate: 16_000,
                channels: 1,
            })
        }

        fn total_frames(&self) -> Option<u64> {
            (self.remaining_frames == 0).then_some(self.decoded_frames)
        }
    }

    fn workspace_from_samples(
        samples: &[f32],
        channels: u16,
        spool_directory: Option<&Path>,
    ) -> AudioWorkspace {
        let total_frames = samples.len() as u64 / u64::from(channels);
        let storage = match spool_directory {
            None => AudioStorage::Memory(samples.to_vec()),
            Some(directory) => {
                let mut spool = SpoolFile::create(directory).unwrap();
                spool.write_samples(samples).unwrap();
                AudioStorage::Spool(spool)
            }
        };
        let mut workspace = AudioWorkspace {
            sample_rate: 16_000,
            channels,
            total_frames,
            storage,
        };
        workspace.finish_decode().unwrap();
        workspace
    }

    #[test]
    fn memory_and_spool_ranges_have_identical_samples() {
        let root = tempfile::tempdir().unwrap();
        let samples: Vec<f32> = (0..40).map(|index| index as f32 / 40.0).collect();
        let mut memory = workspace_from_samples(&samples, 2, None);
        let mut spool = workspace_from_samples(&samples, 2, Some(root.path()));
        let expected = memory.read_frames(3, 11).unwrap();
        assert_eq!(spool.read_frames(3, 11).unwrap(), expected);
        assert!(!memory.is_spooled());
        assert!(spool.is_spooled());
    }

    #[test]
    fn spool_is_removed_when_workspace_is_dropped() {
        let root = tempfile::tempdir().unwrap();
        let workspace = workspace_from_samples(&[0.0, 0.25, -0.5], 1, Some(root.path()));
        let path = match &workspace.storage {
            AudioStorage::Spool(spool) => spool.path.clone(),
            AudioStorage::Memory(_) => unreachable!(),
        };
        assert!(path.is_file());
        drop(workspace);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn pcm_spool_is_private_to_the_current_user() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let spool = SpoolFile::create(root.path()).unwrap();
        let mode = std::fs::metadata(&spool.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[test]
    fn pcm_spool_roundtrips_more_than_one_fixed_write_chunk() {
        let root = tempfile::tempdir().unwrap();
        let samples = (0..SPOOL_WRITE_SAMPLES * 2 + 7)
            .map(|index| index as f32 / 10_000.0)
            .collect::<Vec<_>>();
        let mut spool = workspace_from_samples(&samples, 1, Some(root.path()));
        assert_eq!(
            spool.read_frames(0, samples.len() as u64).unwrap().samples,
            samples
        );
    }

    #[test]
    fn invalid_or_oversized_ranges_are_rejected_before_allocation() {
        let mut workspace = workspace_from_samples(&[0.0, 1.0], 1, None);
        assert_eq!(
            workspace.read_frames(0, 3).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
        assert_eq!(
            workspace.read_frames(2, 1).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
    }

    #[test]
    fn memory_workspace_spills_when_actual_pcm_outgrows_the_hint_budget() {
        let root = tempfile::tempdir().unwrap();
        let policy = ResourcePolicy {
            audio_memory_budget_bytes: 16,
            temporary_directory: root.path().to_owned(),
            temporary_disk_budget_bytes: 128,
            ..ResourcePolicy::default()
        };
        let mut workspace = AudioWorkspace {
            sample_rate: 16_000,
            channels: 1,
            total_frames: 0,
            storage: AudioStorage::Memory(Vec::new()),
        };
        workspace
            .append(
                &AudioBuffer {
                    samples: vec![0.0, 0.25, 0.5, 0.75],
                    sample_rate: 16_000,
                    channels: 1,
                },
                &policy,
            )
            .unwrap();
        assert!(!workspace.is_spooled());
        workspace
            .append(
                &AudioBuffer {
                    samples: vec![-0.25, -0.5],
                    sample_rate: 16_000,
                    channels: 1,
                },
                &policy,
            )
            .unwrap();
        assert!(workspace.is_spooled());
        assert_eq!(workspace.total_frames(), 6);
        assert_eq!(
            workspace.read_frames(0, 6).unwrap().samples,
            vec![0.0, 0.25, 0.5, 0.75, -0.25, -0.5]
        );
    }

    #[test]
    fn decode_spills_once_and_serves_exact_bounded_windows() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        let source = AudioBuffer {
            samples: (0..4_000)
                .map(|index| (index as f32 / 2_000.0) - 1.0)
                .collect(),
            sample_rate: 16_000,
            channels: 1,
        };
        let mut writer = crate::codec::FloatWavWriter::create(&input, 16_000, 1).unwrap();
        writer.write(&source).unwrap();
        writer.finish().unwrap();

        let policy = ResourcePolicy {
            audio_memory_budget_bytes: 1,
            temporary_directory: root.path().to_owned(),
            temporary_disk_budget_bytes: 32_000,
            ..ResourcePolicy::default()
        };
        let mut workspace =
            AudioWorkspace::decode(&input, 16_000, 1, &policy, &CancellationToken::new()).unwrap();
        assert!(workspace.is_spooled());
        assert_eq!(workspace.total_frames(), 4_000);
        assert_eq!(
            workspace.read_frames(123, 321).unwrap().samples,
            source.samples[123..321]
        );
    }

    #[test]
    fn decode_rejects_a_spool_larger_than_the_job_disk_budget() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        let mut writer = crate::codec::FloatWavWriter::create(&input, 16_000, 1).unwrap();
        writer
            .write(&AudioBuffer {
                samples: vec![0.0; 1_000],
                sample_rate: 16_000,
                channels: 1,
            })
            .unwrap();
        writer.finish().unwrap();
        let policy = ResourcePolicy {
            audio_memory_budget_bytes: 1,
            temporary_directory: root.path().to_owned(),
            temporary_disk_budget_bytes: 100,
            ..ResourcePolicy::default()
        };
        let error = AudioWorkspace::decode(&input, 16_000, 1, &policy, &CancellationToken::new())
            .err()
            .expect("disk budget must be checked before decoding");
        assert_eq!(error.code, ToolErrorCode::ResourceBusy);
        assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("spool")
        }));
    }

    #[test]
    fn asr_workspace_spills_after_the_actual_pcm_crosses_its_memory_threshold() {
        let root = tempfile::tempdir().unwrap();
        let total_frames = 16_000 * 3 + 17;
        let mut source = SyntheticSource {
            remaining_frames: total_frames,
            decoded_frames: 0,
            chunk_frames: 997,
            cancellation_after_first_read: None,
        };
        let policy = ResourcePolicy {
            audio_memory_budget_bytes: 4_096,
            temporary_directory: root.path().to_owned(),
            temporary_disk_budget_bytes: (total_frames * size_of::<f32>()) as u64,
            ..ResourcePolicy::default()
        };
        let mut workspace = AudioWorkspace::decode_source(
            &mut source,
            Path::new("synthetic-asr-input"),
            16_000,
            1,
            &policy,
            &CancellationToken::new(),
            false,
        )
        .unwrap();

        assert!(workspace.is_spooled());
        assert_eq!(workspace.resident_sample_capacity(), 0);
        assert_eq!(workspace.total_frames(), total_frames as u64);
        assert_eq!(workspace.read_frames(15_777, 16_333).unwrap().frames(), 556);
    }

    #[test]
    fn cancellation_after_decode_started_removes_the_pcm_spool() {
        let root = tempfile::tempdir().unwrap();
        let cancellation = CancellationToken::new();
        let mut source = SyntheticSource {
            remaining_frames: 32_000,
            decoded_frames: 0,
            chunk_frames: 1_024,
            cancellation_after_first_read: Some(cancellation.clone()),
        };
        let policy = ResourcePolicy {
            audio_memory_budget_bytes: 1,
            temporary_directory: root.path().to_owned(),
            temporary_disk_budget_bytes: 256_000,
            ..ResourcePolicy::default()
        };
        let error = match AudioWorkspace::decode_source(
            &mut source,
            Path::new("synthetic-cancelled-input"),
            16_000,
            1,
            &policy,
            &cancellation,
            true,
        ) {
            Ok(_) => panic!("cancelled decoding must not return a workspace"),
            Err(error) => error,
        };

        assert_eq!(error.code, ToolErrorCode::Cancelled);
        assert_eq!(source.decoded_frames, 1_024);
        assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("spool")
        }));
    }
}
