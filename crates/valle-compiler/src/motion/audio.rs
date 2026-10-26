//! Prepare-time audio analysis. The host supplies frozen, decoded PCM; the compiler writes only
//! bounded numeric signals into the artifact, so frame evaluation never reads a media file.

use std::{collections::BTreeMap, sync::Arc};

use oxc::ast::ast::Argument;
use oxc::span::{GetSpan, Span};
use rustfft::{FftPlanner, num_complex::Complex};

use super::*;

const FFT_SIZE: usize = 2048;
const MAX_BANDS: u8 = 32;
const MAX_RATE: u32 = 120;
const MIN_BEAT_SUPPORT: f64 = 0.35;

#[derive(Debug, Clone)]
pub struct AudioPcm {
    /// Digest of the exact encoded asset bytes admitted to the resource closure.
    pub content_hash: ContentDigest,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

type AudioResolver = dyn Fn(&str) -> Result<AudioPcm, String> + Send + Sync;

#[derive(Clone, Default)]
pub struct AudioAnalysisEnv {
    pub sources: BTreeMap<String, Arc<AudioPcm>>,
    resolver: Option<Arc<AudioResolver>>,
}

impl core::fmt::Debug for AudioAnalysisEnv {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AudioAnalysisEnv")
            .field("sources", &self.sources.keys().collect::<Vec<_>>())
            .field("has_resolver", &self.resolver.is_some())
            .finish()
    }
}

impl AudioAnalysisEnv {
    pub fn with_resolver(
        resolver: impl Fn(&str) -> Result<AudioPcm, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            sources: BTreeMap::new(),
            resolver: Some(Arc::new(resolver)),
        }
    }

    fn source(&mut self, control: &str) -> Result<Option<Arc<AudioPcm>>, String> {
        if let Some(source) = self.sources.get(control) {
            return Ok(Some(Arc::clone(source)));
        }
        let Some(resolver) = &self.resolver else {
            return Ok(None);
        };
        let source = Arc::new(resolver(control)?);
        self.sources.insert(control.to_owned(), Arc::clone(&source));
        Ok(Some(source))
    }
}

#[derive(Debug, Clone)]
pub(super) struct AudioTable {
    /// Bins per second.
    rate: u32,
    /// The length of the asset in seconds.
    end: f64,
    /// The value of each bin, at its start.
    level: Vec<f64>,
    bands: Vec<Vec<f64>>,
    /// The start of each bin with an onset, in seconds.
    onsets: Vec<f64>,
    /// The tracked beats in seconds and one more after the last interval.
    beats: Vec<f64>,
    beat_confidence: f64,
    stable_beats: bool,
}

impl AudioTable {
    /// The signal a method reads; `length` is how long each onset lasts.
    fn signal(
        &self,
        method: &str,
        band: Option<usize>,
        length: Option<f64>,
    ) -> Option<valle_motion::AudioSignal> {
        use valle_motion::AudioSignal;
        let series = |values: &Vec<f64>| AudioSignal::Series {
            rate: self.rate,
            end: self.end,
            values: values.clone(),
        };
        match method {
            "level" => Some(series(&self.level)),
            "band" => self.bands.get(band?).map(series),
            "onset" => Some(AudioSignal::Onsets {
                times: self.onsets.clone(),
                length: length?,
            }),
            "beatPhase" if self.stable_beats => Some(AudioSignal::Beats {
                times: self.beats.clone(),
            }),
            "beatPhase" => Some(AudioSignal::Constant { value: 0.0 }),
            "beatConfidence" => Some(AudioSignal::Constant {
                value: self.beat_confidence,
            }),
            _ => None,
        }
    }
}

fn quantize(value: f64) -> f64 {
    (value.max(0.0) * 1_000_000.0).round() / 1_000_000.0
}

// Host-owned immutable PCM is shared across editor recompiles. Cache by its Arc
// identity, not an encoded hash supplied alongside potentially different PCM.
// Weak ownership avoids retaining decoded songs after the host evicts them.
type CachedAudioTable = (std::sync::Weak<AudioPcm>, u8, u32, Arc<AudioTable>);

thread_local! {
    static TABLE_CACHE: std::cell::RefCell<std::collections::VecDeque<CachedAudioTable>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

fn analyze_cached(pcm: &Arc<AudioPcm>, bands: u8, rate: u32) -> Result<Arc<AudioTable>, String> {
    TABLE_CACHE.with_borrow_mut(|cache| {
        cache.retain(|(source, _, _, _)| source.strong_count() > 0);
        if let Some(at) = cache.iter().position(|(source, b, r, _)| {
            *b == bands && *r == rate && source.ptr_eq(&Arc::downgrade(pcm))
        }) {
            let entry = cache.remove(at).unwrap();
            let table = Arc::clone(&entry.3);
            cache.push_back(entry);
            return Ok(table);
        }
        let table = Arc::new(analyze(pcm, bands, rate)?);
        if cache.len() == 16 {
            cache.pop_front();
        }
        cache.push_back((Arc::downgrade(pcm), bands, rate, Arc::clone(&table)));
        Ok(table)
    })
}

pub(super) fn analyze(pcm: &AudioPcm, bands: u8, rate: u32) -> Result<AudioTable, String> {
    if pcm.sample_rate == 0 || pcm.samples.is_empty() || pcm.samples.iter().any(|s| !s.is_finite())
    {
        return Err("audioAnalysis needs non-empty finite mono PCM".into());
    }
    if !(1..=MAX_BANDS).contains(&bands) || !(1..=MAX_RATE).contains(&rate) {
        return Err("audioAnalysis bands must be 1..=32 and rate 1..=120".into());
    }
    let frames =
        (pcm.samples.len() as u128 * u128::from(rate)).div_ceil(u128::from(pcm.sample_rate));
    if frames == 0 || frames > valle_motion::expr::MAX_SIMULATION_STEPS as u128 {
        return Err(format!(
            "audioAnalysis exceeds {} samples; shorten the asset or lower rate",
            valle_motion::expr::MAX_SIMULATION_STEPS
        ));
    }
    let frames = frames as usize;
    let mut level = vec![0.0; frames];
    let mut band_series = vec![vec![0.0; frames]; bands as usize];
    let mut flux = vec![0.0; frames];
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let window = (0..FFT_SIZE)
        .map(|index| {
            0.5 - 0.5 * libm::cos(2.0 * core::f64::consts::PI * index as f64 / FFT_SIZE as f64)
        })
        .collect::<Vec<_>>();
    let mut spectrum = vec![Complex::new(0.0, 0.0); FFT_SIZE];
    let mut previous = vec![0.0; FFT_SIZE / 2];
    let nyquist = (pcm.sample_rate as f64 / 2.0).min(20_000.0);
    if nyquist <= 20.0 {
        return Err("audioAnalysis sample rate must exceed 40 Hz".into());
    }
    let log_range = libm::log(nyquist / 20.0);
    // Frequency and band membership depend only on this analysis configuration, not its frame.
    // Keep ascending bins so accumulation and the quantized signal remain byte-identical.
    let bins: Vec<_> = (1..FFT_SIZE / 2)
        .filter_map(|bin| {
            let frequency = bin as f64 * pcm.sample_rate as f64 / FFT_SIZE as f64;
            if frequency < 20.0 || frequency > nyquist {
                return None;
            }
            let relative = libm::log(frequency / 20.0) / log_range;
            let band = ((relative * f64::from(bands)).floor() as usize).min(bands as usize - 1);
            Some((bin, band))
        })
        .collect();
    let mut band_energy = vec![0.0; bands as usize];
    for frame in 0..frames {
        let from = (frame as u128 * u128::from(pcm.sample_rate) / u128::from(rate)) as usize;
        let until = (((frame + 1) as u128 * u128::from(pcm.sample_rate) / u128::from(rate))
            as usize)
            .min(pcm.samples.len());
        if from < until {
            let energy = pcm.samples[from..until]
                .iter()
                .map(|sample| f64::from(*sample) * f64::from(*sample))
                .sum::<f64>()
                / (until - from) as f64;
            level[frame] = quantize(libm::sqrt(energy));
        }
        let center = (from + until) / 2;
        for (index, slot) in spectrum.iter_mut().enumerate() {
            let sample_index = center as isize + index as isize - FFT_SIZE as isize / 2;
            let sample = if sample_index >= 0 {
                pcm.samples
                    .get(sample_index as usize)
                    .copied()
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            *slot = Complex::new(f64::from(sample) * window[index], 0.0);
        }
        fft.process(&mut spectrum);
        band_energy.fill(0.0);
        for &(bin, band) in &bins {
            let magnitude = spectrum[bin].norm();
            band_energy[band] += magnitude * magnitude;
            flux[frame] += (magnitude - previous[bin]).max(0.0);
            previous[bin] = magnitude;
        }
        for (band, &energy) in band_energy.iter().enumerate() {
            band_series[band][frame] = quantize(libm::sqrt(energy) * 4.0 / FFT_SIZE as f64);
        }
    }
    let peak = flux.iter().copied().fold(0.0, f64::max);
    let mean = flux.iter().sum::<f64>() / frames as f64;
    let gate = (peak * 0.15).max(mean * 1.5);
    // An onset is timed at the start of its bin.
    let mut onsets = Vec::new();
    let mut last = None;
    for frame in 0..frames {
        if flux[frame] >= gate
            && flux[frame] > 0.0
            && (frame == 0 || flux[frame] >= flux[frame - 1])
            && (frame + 1 == frames || flux[frame] > flux[frame + 1])
            && last.is_none_or(|last| frame >= last + 2)
        {
            onsets.push(frame as f64 / f64::from(rate));
            last = Some(frame);
        }
    }
    // Track at audio resolution instead of rounding a tempo to an integer number of video
    // frames. The phase runs between tracked beats, so timing error cannot accumulate over the
    // duration of the asset.
    let tracked = valle_media::analysis::beat::detect(&pcm.samples, pcm.sample_rate, 40.0, 240.0);
    let stable_beats = tracked.beats.len() >= 3 && tracked.support >= MIN_BEAT_SUPPORT;
    let mut beats = Vec::new();
    if stable_beats {
        beats = tracked.beats.iter().map(|beat| quantize(*beat)).collect();
        // Continue the final tracked interval only to the next expected beat. Long silence after
        // the last event does not fabricate an endless beat grid.
        let n = beats.len();
        beats.push(quantize(beats[n - 1] + beats[n - 1] - beats[n - 2]));
        beats.dedup();
    }
    Ok(AudioTable {
        rate,
        end: pcm.samples.len() as f64 / f64::from(pcm.sample_rate),
        level,
        bands: band_series,
        onsets,
        beats,
        beat_confidence: quantize(tracked.support.clamp(0.0, 1.0)),
        stable_beats,
    })
}

impl<'s> Compiler<'s> {
    /// Warn of an onset shorter than a frame of the composition, through the slope of its `time`,
    /// which includes the speed of the TimeScopes around the call: frames can miss it.
    fn check_onset_length(&mut self, length: f64, time: ExprId, span: Span) {
        let Some(rate) = self
            .composition
            .as_ref()
            .and_then(|composition| composition.frame_rate().ok().flatten())
        else {
            return;
        };
        let fps = rate.numerator() as f64 / f64::from(rate.denominator());
        // A time that is not a straight line of the clock runs at the speed of the scopes.
        let speed =
            valle_motion::time_function::TimeFunction::compile(&self.expr_arena.values, time)
                .filter(|time| time.input() == ContextInput::CompositionSeconds)
                .map_or_else(
                    || {
                        self.time_scopes
                            .iter()
                            .map(|transform| transform.speed.abs())
                            .product::<f64>()
                    },
                    |time| time.derivative().abs(),
                );
        if speed == 0.0 || length / speed * fps >= 1.0 {
            return;
        }
        self.warn(
            DiagCode::UnderOneFrame,
            span,
            format!(
                "each onset lasts {} seconds, {} seconds of output, shorter than a frame at {} \
                 fps: frames can miss an onset; make it at least `{}`",
                super::clock::seconds_text(length),
                super::clock::seconds_text(length / speed),
                super::clock::seconds_text(fps),
                super::clock::seconds_text(speed / fps),
            ),
        );
    }

    pub(super) fn lower_audio_analysis_call(
        &mut self,
        member: &oxc::ast::ast::StaticMemberExpression<'_>,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<Option<ExprId>> {
        let value = self.eval_static(&member.object)?;
        if value.get("__valleType").and_then(serde_json::Value::as_str) != Some("audioAnalysis") {
            return None;
        }
        let method = member.property.name.as_str();
        let expected = if matches!(method, "band" | "onset") {
            2
        } else {
            1
        };
        if method == "onset" && arguments.len() == 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "onset takes how long each onset lasts, in seconds, after the time: \
                 `onset(t, length)`, such as `BEAT.onset(seconds, 0.2)`",
            );
            return Some(None);
        }
        if !["level", "band", "onset", "beatPhase", "beatConfidence"].contains(&method)
            || arguments.len() != expected
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "audioAnalysis supports level(t), band(index, t), onset(t, length), beatPhase(t), and beatConfidence(t)",
            );
            return Some(None);
        }
        if value.get("fps").is_some_and(|fps| !fps.is_null()) {
            self.illegal(
                DiagCode::BuiltinRejected,
                member.object.span(),
                "audioAnalysis names its samples per second `rate`, such as \
                 `audioAnalysis(\"asset://music\", { bands: 8, rate: 30 })`",
            );
            return Some(None);
        }
        let source = value.get("source").and_then(serde_json::Value::as_str);
        let bands = value.get("bands").and_then(serde_json::Value::as_u64);
        let rate = value
            .get("rate")
            .and_then(serde_json::Value::as_u64)
            .or_else(|| {
                self.composition
                    .as_ref()
                    .and_then(|composition| composition.frame_rate().ok().flatten())
                    .filter(|rate| rate.denominator() == 1)
                    .and_then(|rate| u64::try_from(rate.numerator()).ok())
            });
        let (Some(control), Some(bands), Some(rate)) = (
            source.and_then(|source| source.strip_prefix("asset://")),
            bands,
            rate,
        ) else {
            self.illegal(DiagCode::BuiltinRejected, member.object.span(),
                "audioAnalysis requires asset://<audio-control>, a literal band count, and an integer rate option or an integer composition.fps");
            return Some(None);
        };
        if !(1..=u64::from(MAX_BANDS)).contains(&bands)
            || !(1..=u64::from(MAX_RATE)).contains(&rate)
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                member.object.span(),
                "audioAnalysis bands must be 1..=32 and rate 1..=120",
            );
            return Some(None);
        }
        if self
            .controls
            .assets
            .get(control)
            .is_none_or(|schema| schema.kind != AssetKind::Audio)
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                member.object.span(),
                format!("audioAnalysis `{control}` must be declared as an audio asset control"),
            );
            return Some(None);
        }
        let Some(bound) = self
            .resource_refs
            .iter()
            .find(|resource| resource.control == control)
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                member.object.span(),
                format!("audioAnalysis asset `{control}` has no bound resource"),
            );
            return Some(None);
        };
        let pcm = match self.audio.as_mut().map(|audio| audio.source(control)) {
            Some(Ok(Some(pcm))) => pcm,
            Some(Err(message)) => {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    member.object.span(),
                    format!("audioAnalysis asset `{control}` cannot be decoded: {message}"),
                );
                return Some(None);
            }
            _ => {
                self.illegal(DiagCode::BuiltinRejected, member.object.span(),
                    format!("audioAnalysis asset `{control}` has no decoded PCM; bind a decodable audio file"));
                return Some(None);
            }
        };
        if pcm.content_hash != bound.content_hash {
            self.illegal(
                DiagCode::BuiltinRejected,
                member.object.span(),
                format!("audioAnalysis asset `{control}` bytes differ from the frozen resource"),
            );
            return Some(None);
        }
        let key = (control.to_owned(), bands as u8, rate as u32);
        if !self.audio_tables.contains_key(&key) {
            let analyzed = analyze_cached(&pcm, bands as u8, rate as u32);
            let table = match analyzed {
                Ok(table) => table,
                Err(message) => {
                    self.illegal(DiagCode::BuiltinRejected, member.object.span(), message);
                    return Some(None);
                }
            };
            self.audio_tables.insert(key.clone(), table);
        }
        if method == "beatPhase" && !self.audio_tables[&key].stable_beats {
            let confidence = self.audio_tables[&key].beat_confidence;
            self.warn(DiagCode::AudioBeatUncertain, span, format!(
                "audioAnalysis asset `{control}` has no reliable beat grid (confidence {confidence:.3}); beatPhase returns 0. Use onset(t, length) for individual events or beatConfidence(t) to inspect periodic support"
            ));
        }
        let mut instance_band = None;
        let band = if method == "band" {
            let index = arguments[0]
                .as_expression()
                .and_then(|expr| self.eval_static(expr))
                .and_then(|value| value.as_u64());
            if let Some(index) = index.filter(|index| *index < bands) {
                Some(index as usize)
            } else if let Some(expression) = arguments[0].as_expression()
                && let Some(count) = self.instance_audio_band_count(expression, bands)
            {
                instance_band = Some((self.lower_expr(expression)?, count));
                None
            } else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    arguments[0].span(),
                    "audioAnalysis band index must be a static integer within the band count",
                );
                return Some(None);
            }
        } else {
            None
        };
        let length = if method == "onset" {
            let length = arguments[1]
                .as_expression()
                .and_then(|expr| self.eval_static(expr))
                .and_then(|value| value.as_f64())
                .filter(|length| length.is_finite() && *length > 0.0);
            let Some(length) = length else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    arguments[1].span(),
                    "onset length must be a static positive number of seconds, such as `0.2`",
                );
                return Some(None);
            };
            Some(length)
        } else {
            None
        };
        let time_argument = if method == "band" {
            &arguments[1]
        } else {
            &arguments[0]
        };
        let Some(time_expr) = time_argument.as_expression() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                time_argument.span(),
                "audioAnalysis time must be a numeric expression",
            );
            return Some(None);
        };
        let Some(time) = self.lower_expr(time_expr) else {
            return Some(None);
        };
        if let Some(length) = length {
            self.check_onset_length(length, time, arguments[1].span());
        }
        if let Some((index, count)) = instance_band {
            // The prepared numeric column proves the index is an integer in range.
            // Reuse ordinary AudioSample/Select expressions: uniform times are sampled
            // once per band and frame by the existing instance evaluation plan.
            let mut samples = (0..count)
                .map(|band| {
                    let signal = self.audio_tables[&key]
                        .signal(method, Some(band as usize), None)
                        .expect("proved band range");
                    self.push(Expr::AudioSample { time, signal }, span)
                })
                .collect::<Vec<_>>();
            let mut result = samples.pop().expect("nonempty band range");
            for (band, sample) in samples.into_iter().enumerate().rev() {
                let band = self.push(
                    Expr::Const {
                        value: MotionValue::Number(band as f64),
                    },
                    arguments[0].span(),
                );
                let condition = self.compare(valle_motion::CompareOp::Eq, index, band, span);
                result = self.push(
                    Expr::Select {
                        condition,
                        when_true: sample,
                        when_false: result,
                    },
                    span,
                );
            }
            return Some(Some(result));
        }
        let signal = self.audio_tables[&key]
            .signal(method, band, length)
            .expect("validated audio field");
        Some(Some(self.push(Expr::AudioSample { time, signal }, span)))
    }

    fn instance_audio_band_count(
        &mut self,
        expression: &Expression<'_>,
        bands: u64,
    ) -> Option<u64> {
        let scope = self.instance_scope.as_ref()?;
        let maximum = scope.scalar_max?;
        let parameter = scope.parameter.clone();
        let binding = self.bindings.scalars.get(&parameter)?;
        if !matches!(self.expr_arena.values.get(binding.0 as usize), Some(Expr::InstanceField { column, .. })
            if scope.fields.get("_value").is_some_and(|(expected, _)| column == expected))
        {
            return None;
        }
        match peel_expr(expression) {
            Expression::Identifier(id) if id.name.as_str() == parameter && maximum < bands => {
                Some(maximum + 1)
            }
            Expression::BinaryExpression(binary)
                if binary.operator == oxc::syntax::operator::BinaryOperator::Remainder
                    && matches!(peel_expr(&binary.left), Expression::Identifier(id) if id.name.as_str() == parameter) =>
            {
                let modulus = self.eval_static(&binary.right)?.as_u64()?;
                (modulus > 0 && modulus <= bands).then_some(modulus.min(maximum + 1))
            }
            _ => None,
        }
    }
}

/// The beat phase of a table analysed at 30 bins a second, at the start of bin `frame`.
#[cfg(test)]
fn phase_at(table: &AudioTable, frame: usize) -> f64 {
    table
        .signal("beatPhase", None, None)
        .unwrap()
        .at(frame as f64 / 30.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(samples: Vec<f32>) -> AudioPcm {
        AudioPcm {
            content_hash: ContentDigest::of_bytes(b"synthetic"),
            sample_rate: 48_000,
            samples,
        }
    }

    #[test]
    fn dual_tone_energy_lands_in_the_expected_log_bands() {
        let samples = (0..48_000)
            .map(|index| {
                let time = index as f64 / 48_000.0;
                (0.4 * libm::sin(core::f64::consts::TAU * 220.0 * time)
                    + 0.4 * libm::sin(core::f64::consts::TAU * 1760.0 * time))
                    as f32
            })
            .collect();
        let table = analyze(&pcm(samples), 8, 30).unwrap();
        assert!(table.bands[2][15] > table.bands[0][15] * 5.0);
        assert!(table.bands[5][15] > table.bands[7][15] * 5.0);
        assert!(table.bands[2][15] > 0.25);
        assert!(table.bands[5][15] > 0.25);
    }

    #[test]
    fn regular_pulses_have_frame_accurate_onsets_and_repeatable_phase_table() {
        let mut samples = vec![0.0; 48_000 * 2];
        for pulse in [12_000, 36_000, 60_000, 84_000] {
            samples[pulse..pulse + 512].fill(0.8);
        }
        let first = analyze(&pcm(samples.clone()), 8, 30).unwrap();
        let second = analyze(&pcm(samples), 8, 30).unwrap();
        let onsets = first
            .onsets
            .iter()
            .map(|time| (time * 30.0).round() as usize)
            .collect::<Vec<_>>();
        for expected in [8, 23, 38, 53] {
            assert!(
                onsets.iter().any(|frame| frame.abs_diff(expected) <= 1),
                "{onsets:?}"
            );
        }
        assert_eq!(first.level, second.level);
        assert_eq!(first.bands, second.bands);
        assert_eq!(first.onsets, second.onsets);
        assert_eq!(first.beats, second.beats);
        assert!(phase_at(&first, 16) > 0.35 && phase_at(&first, 16) < 0.65);
        let bytes = valle_motion::canonical_bytes(&(
            &first.level,
            &first.bands,
            &first.onsets,
            &first.beats,
        ))
        .unwrap();
        assert_eq!(
            ContentDigest::of_bytes(&bytes).as_hex(),
            "2845c6126bc2fafa8259cc041b82245125a37b990986f1e0be7aa009408531bd"
        );
    }
}

#[cfg(test)]
mod offbeat_regressions {
    use super::*;

    fn clicks(times: &[f64], seconds: usize) -> AudioPcm {
        let rate = 12_000;
        let mut samples = vec![0.0; rate * seconds];
        for &time in times {
            let at = (time * rate as f64).round() as usize;
            if at + 128 <= samples.len() {
                samples[at..at + 128].fill(0.8);
            }
        }
        AudioPcm {
            content_hash: ContentDigest::of_bytes(b"clicks"),
            sample_rate: rate as u32,
            samples,
        }
    }

    #[test]
    fn fractional_beat_period_does_not_drift_over_three_minutes() {
        let times: Vec<_> = (0..380).map(|i| 0.25 + i as f64 * 60.0 / 128.0).collect();
        let table = analyze(&clicks(&times, 180), 4, 30).unwrap();
        assert!(table.stable_beats, "confidence={}", table.beat_confidence);
        for &beat in times.iter().skip(4).take(372) {
            let frame = (beat * 30.0).round() as usize;
            let phase = phase_at(&table, frame);
            assert!(
                phase.min(1.0 - phase) < 0.12,
                "beat {beat}, frame {frame}, phase {phase}"
            );
        }
    }

    #[test]
    fn irregular_events_and_silence_do_not_invent_a_regular_phase() {
        let mut time = 0.25;
        let mut seed = 17_u32;
        let mut times = Vec::new();
        while time < 29.0 {
            times.push(time);
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            time += 0.15 + f64::from(seed) / f64::from(u32::MAX) * 0.75;
        }
        for pcm in [clicks(&times, 30), clicks(&[], 3)] {
            let table = analyze(&pcm, 4, 30).unwrap();
            assert!(!table.stable_beats, "confidence={}", table.beat_confidence);
            assert_eq!(
                table.signal("beatPhase", None, None),
                Some(valle_motion::AudioSignal::Constant { value: 0.0 })
            );
        }
    }

    #[test]
    fn one_offbeat_does_not_erase_regular_phase() {
        let mut samples = vec![0.0; 48_000 * 4];
        for pulse in [
            12_000, 36_000, 60_000, 84_000, 96_000, 108_000, 132_000, 156_000, 180_000,
        ] {
            samples[pulse..pulse + 512].fill(0.8);
        }
        let table = analyze(
            &AudioPcm {
                content_hash: ContentDigest::of_bytes(b"offbeat"),
                sample_rate: 48_000,
                samples,
            },
            8,
            30,
        )
        .unwrap();
        assert!(phase_at(&table, 16) > 0.35 && phase_at(&table, 16) < 0.65);
        assert!(phase_at(&table, 76) > 0.35 && phase_at(&table, 76) < 0.65);
    }
}
