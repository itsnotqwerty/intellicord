use serde::Serialize;

use super::{
    Analyzer, ClockError, ClockInterval, Features, Format, FrameError, PcmEncoding, PcmPlane,
    RepetitionAnalyzer, RepetitionFeatures, SampleClockMapping, SpectralAnalyzer, SpectralFeatures,
    decode_pcm_chunk, decode_pcm_planes,
};

pub const MAX_CHUNK_FRAMES: usize = 4;

#[derive(Debug, Serialize)]
pub struct SampleGap {
    pub skipped_samples_per_channel: u64,
    pub discarded_partial_samples_per_channel: usize,
}

#[derive(Debug, Serialize)]
pub struct AssemblyBatch<Output = Features> {
    pub features: Vec<Output>,
    pub gap: Option<SampleGap>,
    pub buffered_samples_per_channel: usize,
}

pub struct FrameAssembler {
    format: Format,
    origin_ns: u64,
    next_sample: u64,
    pending_start: u64,
    pending: Vec<f32>,
    analyzer: Analyzer,
    discontinuity: bool,
}

trait FrameConsumer {
    type Output;
    fn reset(&mut self);
    fn process(&mut self, levels: Features, pcm: &[f32]) -> Self::Output;
}

struct LevelConsumer;

impl FrameConsumer for LevelConsumer {
    type Output = Features;

    fn reset(&mut self) {}

    fn process(&mut self, levels: Features, _pcm: &[f32]) -> Features {
        levels
    }
}

impl FrameConsumer for SpectralAnalyzer {
    type Output = SpectralFeatures;

    fn reset(&mut self) {
        SpectralAnalyzer::reset(self);
    }

    fn process(&mut self, levels: Features, pcm: &[f32]) -> SpectralFeatures {
        self.process_levels(levels, pcm)
    }
}

impl FrameConsumer for RepetitionAnalyzer {
    type Output = RepetitionFeatures;

    fn reset(&mut self) {
        RepetitionAnalyzer::reset(self);
    }

    fn process(&mut self, levels: Features, pcm: &[f32]) -> RepetitionFeatures {
        self.process_levels(levels, pcm)
    }
}

pub struct RepetitionFrameAssembler {
    frames: FrameAssembler,
    repetition: RepetitionAnalyzer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimedFrameError {
    Frame(FrameError),
    Clock(ClockError),
}

#[derive(Debug, Serialize)]
pub struct TimedRepetitionFeatures {
    pub schema_version: u16,
    pub start_sample_per_channel: u64,
    pub end_sample_per_channel: u64,
    pub start_time: ClockInterval,
    pub end_time: ClockInterval,
    pub nominal_sample_clock_features: RepetitionFeatures,
}

pub struct TimedRepetitionFrameAssembler {
    frames: RepetitionFrameAssembler,
    clock: SampleClockMapping,
}

impl TimedRepetitionFrameAssembler {
    pub fn new(clock: SampleClockMapping) -> Self {
        Self {
            frames: RepetitionFrameAssembler::new(clock.format(), 0),
            clock,
        }
    }

    pub fn push_bytes(
        &mut self,
        start_sample_per_channel: u64,
        encoding: PcmEncoding,
        bytes: &[u8],
    ) -> Result<AssemblyBatch<TimedRepetitionFeatures>, TimedFrameError> {
        let samples = decode_pcm_chunk(self.clock.format(), encoding, bytes)
            .map_err(TimedFrameError::Frame)?;
        self.push(start_sample_per_channel, &samples)
    }

    pub fn push_planes(
        &mut self,
        start_sample_per_channel: u64,
        encoding: PcmEncoding,
        samples_per_channel: usize,
        planes: &[PcmPlane<'_>],
    ) -> Result<AssemblyBatch<TimedRepetitionFeatures>, TimedFrameError> {
        let samples = decode_pcm_planes(self.clock.format(), encoding, samples_per_channel, planes)
            .map_err(TimedFrameError::Frame)?;
        self.push(start_sample_per_channel, &samples)
    }

    pub fn push(
        &mut self,
        start_sample_per_channel: u64,
        interleaved: &[f32],
    ) -> Result<AssemblyBatch<TimedRepetitionFeatures>, TimedFrameError> {
        let frames = &self.frames.frames;
        let end = frames
            .validate_chunk(start_sample_per_channel, interleaved)
            .map_err(TimedFrameError::Frame)?;
        self.clock.map(end).map_err(TimedFrameError::Clock)?;
        let contiguous = start_sample_per_channel == frames.next_sample;
        let buffered = if contiguous { frames.pending.len() } else { 0 };
        let first = if buffered > 0 {
            frames.pending_start
        } else {
            start_sample_per_channel
        };
        let frame_samples = (frames.format.sample_count() / frames.format.channels) as u64;
        let count = (buffered + interleaved.len()) / frames.format.sample_count();
        let mut timing = Vec::with_capacity(count);
        for index in 0..count {
            let start = first + index as u64 * frame_samples;
            let end = start + frame_samples;
            timing.push((
                start,
                end,
                self.clock.map(start).map_err(TimedFrameError::Clock)?,
                self.clock.map(end).map_err(TimedFrameError::Clock)?,
            ));
        }
        let batch = self
            .frames
            .push(start_sample_per_channel, interleaved)
            .map_err(TimedFrameError::Frame)?;
        let features = batch
            .features
            .into_iter()
            .zip(timing)
            .map(
                |(features, (start, end, start_time, end_time))| TimedRepetitionFeatures {
                    schema_version: 1,
                    start_sample_per_channel: start,
                    end_sample_per_channel: end,
                    start_time,
                    end_time,
                    nominal_sample_clock_features: features,
                },
            )
            .collect();
        Ok(AssemblyBatch {
            features,
            gap: batch.gap,
            buffered_samples_per_channel: batch.buffered_samples_per_channel,
        })
    }
}

impl RepetitionFrameAssembler {
    pub fn new(format: Format, origin_ns: u64) -> Self {
        Self {
            frames: FrameAssembler::new(format, origin_ns),
            repetition: RepetitionAnalyzer::default(),
        }
    }

    pub fn push(
        &mut self,
        start_sample_per_channel: u64,
        interleaved: &[f32],
    ) -> Result<AssemblyBatch<RepetitionFeatures>, FrameError> {
        self.frames
            .push_mapped(start_sample_per_channel, interleaved, &mut self.repetition)
    }

    pub fn reset(&mut self, origin_ns: u64) -> usize {
        self.repetition.reset();
        self.frames.reset(origin_ns)
    }
}

pub struct SpectralFrameAssembler {
    frames: FrameAssembler,
    spectral: SpectralAnalyzer,
}

impl SpectralFrameAssembler {
    pub fn new(format: Format, origin_ns: u64) -> Self {
        Self {
            frames: FrameAssembler::new(format, origin_ns),
            spectral: SpectralAnalyzer::default(),
        }
    }

    pub fn push(
        &mut self,
        start_sample_per_channel: u64,
        interleaved: &[f32],
    ) -> Result<AssemblyBatch<SpectralFeatures>, FrameError> {
        self.frames
            .push_mapped(start_sample_per_channel, interleaved, &mut self.spectral)
    }

    pub fn reset(&mut self, origin_ns: u64) -> usize {
        self.spectral.reset();
        self.frames.reset(origin_ns)
    }
}

impl FrameAssembler {
    pub fn new(format: Format, origin_ns: u64) -> Self {
        Self {
            format,
            origin_ns,
            next_sample: 0,
            pending_start: 0,
            pending: Vec::with_capacity(format.sample_count()),
            analyzer: Analyzer::default(),
            discontinuity: false,
        }
    }

    fn timestamp(&self, sample: u64) -> Result<u64, FrameError> {
        let elapsed = u128::from(sample) * 1_000_000_000 / u128::from(self.format.sample_rate);
        let timestamp = u128::from(self.origin_ns) + elapsed;
        u64::try_from(timestamp).map_err(|_| FrameError::InvalidTime)
    }

    pub fn push(
        &mut self,
        start_sample_per_channel: u64,
        interleaved: &[f32],
    ) -> Result<AssemblyBatch, FrameError> {
        self.push_mapped(start_sample_per_channel, interleaved, &mut LevelConsumer)
    }

    fn validate_chunk(
        &self,
        start_sample_per_channel: u64,
        interleaved: &[f32],
    ) -> Result<u64, FrameError> {
        if interleaved.is_empty()
            || interleaved.len() > self.format.sample_count() * MAX_CHUNK_FRAMES
            || !interleaved.len().is_multiple_of(self.format.channels)
        {
            return Err(FrameError::InvalidLength);
        }
        if interleaved
            .iter()
            .any(|sample| !sample.is_finite() || sample.abs() > 1.0)
        {
            return Err(FrameError::InvalidSample);
        }
        if start_sample_per_channel < self.next_sample {
            return Err(FrameError::InvalidTime);
        }
        let end_sample = start_sample_per_channel
            .checked_add((interleaved.len() / self.format.channels) as u64)
            .ok_or(FrameError::InvalidTime)?;
        self.timestamp(end_sample)?;
        Ok(end_sample)
    }

    fn push_mapped<Consumer: FrameConsumer>(
        &mut self,
        start_sample_per_channel: u64,
        interleaved: &[f32],
        consumer: &mut Consumer,
    ) -> Result<AssemblyBatch<Consumer::Output>, FrameError> {
        let end_sample = self.validate_chunk(start_sample_per_channel, interleaved)?;
        let gap = if start_sample_per_channel > self.next_sample {
            let gap = SampleGap {
                skipped_samples_per_channel: start_sample_per_channel - self.next_sample,
                discarded_partial_samples_per_channel: self.pending.len() / self.format.channels,
            };
            self.pending.fill(0.0);
            self.pending.clear();
            self.analyzer.reset();
            consumer.reset();
            self.discontinuity = true;
            Some(gap)
        } else {
            None
        };
        let mut features = Vec::with_capacity(MAX_CHUNK_FRAMES);
        let mut consumed = 0;
        while consumed < interleaved.len() {
            if self.pending.is_empty() {
                self.pending_start =
                    start_sample_per_channel + (consumed / self.format.channels) as u64;
            }
            let take =
                (self.format.sample_count() - self.pending.len()).min(interleaved.len() - consumed);
            self.pending
                .extend_from_slice(&interleaved[consumed..consumed + take]);
            consumed += take;
            if self.pending.len() == self.format.sample_count() {
                let timestamp = self
                    .timestamp(self.pending_start)
                    .expect("validated chunk end bounds every frame timestamp");
                let mut frame = self
                    .analyzer
                    .process(self.format, timestamp, &self.pending)
                    .expect(
                        "validated samples and monotonic sample positions form a complete frame",
                    );
                if self.discontinuity {
                    frame.discontinuity = true;
                    self.discontinuity = false;
                }
                features.push(consumer.process(frame, &self.pending));
                self.pending.fill(0.0);
                self.pending.clear();
            }
        }
        self.next_sample = end_sample;
        Ok(AssemblyBatch {
            features,
            gap,
            buffered_samples_per_channel: self.pending.len() / self.format.channels,
        })
    }

    pub fn reset(&mut self, origin_ns: u64) -> usize {
        let discarded = self.pending.len() / self.format.channels;
        self.pending.fill(0.0);
        self.pending.clear();
        self.origin_ns = origin_ns;
        self.next_sample = 0;
        self.pending_start = 0;
        self.analyzer.reset();
        self.discontinuity = true;
        discarded
    }
}

impl Drop for FrameAssembler {
    fn drop(&mut self) {
        self.pending.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_output_gap_resets_consumer_before_return_but_invalid_input_does_not() {
        struct Consumer {
            resets: usize,
        }
        impl FrameConsumer for Consumer {
            type Output = Features;

            fn reset(&mut self) {
                self.resets += 1;
            }

            fn process(&mut self, levels: Features, _pcm: &[f32]) -> Features {
                levels
            }
        }
        let mut consumer = Consumer { resets: 0 };
        let mut frames = FrameAssembler::new(Format::new(8_000, 1).unwrap(), 0);
        frames.push_mapped(0, &[0.5; 10], &mut consumer).unwrap();
        assert!(frames.push_mapped(20, &[f32::NAN], &mut consumer).is_err());
        assert_eq!(consumer.resets, 0);
        let gap = frames.push_mapped(20, &[0.5], &mut consumer).unwrap();
        assert!(gap.features.is_empty());
        assert_eq!(gap.gap.unwrap().discarded_partial_samples_per_channel, 10);
        assert_eq!(consumer.resets, 1);
        let next = frames.push_mapped(22, &[0.5], &mut consumer).unwrap();
        assert!(next.features.is_empty());
        assert_eq!(consumer.resets, 2);
    }

    #[test]
    fn arbitrary_chunks_match_complete_frame_analysis_without_clock_drift() {
        for sample_rate in [8_000, 44_100, 48_000, 96_000] {
            let format = Format::new(sample_rate, 2).unwrap();
            let mut assembler = FrameAssembler::new(format, 123);
            let samples: Vec<_> = (0..format.sample_count() * 7)
                .map(|index| (index % 17) as f32 / 20.0)
                .collect();
            let mut features = Vec::new();
            let mut consumed = 0;
            for chunk in samples.chunks(74) {
                let batch = assembler.push(consumed / 2, chunk).unwrap();
                assert!(batch.gap.is_none());
                assert!(batch.buffered_samples_per_channel < sample_rate as usize / 50);
                features.extend(batch.features);
                consumed += chunk.len() as u64;
            }
            assert_eq!(features.len(), 7);
            let mut analyzer = Analyzer::default();
            for (index, frame) in samples.chunks(format.sample_count()).enumerate() {
                let expected = analyzer
                    .process(
                        format,
                        123 + index as u64 * super::super::FRAME_DURATION_NS,
                        frame,
                    )
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&features[index]).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
            assert!(assembler.pending.is_empty());
        }
    }

    #[test]
    fn gap_discards_partial_pcm_and_survives_until_a_complete_recovery_frame() {
        let format = Format::new(8_000, 2).unwrap();
        let mut assembler = FrameAssembler::new(format, 700);
        let first = assembler.push(0, &vec![1.0; 80 * 2]).unwrap();
        assert!(first.features.is_empty());
        assert_eq!(first.buffered_samples_per_channel, 80);
        let gap = assembler.push(120, &[0.0, 0.0]).unwrap();
        assert!(gap.features.is_empty());
        let gap = gap.gap.unwrap();
        assert_eq!(gap.skipped_samples_per_channel, 40);
        assert_eq!(gap.discarded_partial_samples_per_channel, 80);
        let recovered = assembler.push(121, &vec![0.0; 319 * 2]).unwrap();
        assert!(recovered.gap.is_none());
        assert_eq!(recovered.features.len(), 2);
        for feature in &recovered.features {
            assert_eq!(
                feature.rms, 0.0,
                "pre-gap PCM must not appear in recovery output"
            );
        }
        assert_eq!(recovered.features[0].frame_start_ns, 700 + 15_000_000);
        assert!(recovered.features[0].discontinuity);
        assert_eq!(recovered.features[0].rms_change, None);
        assert!(!recovered.features[1].discontinuity);
        assert_eq!(recovered.features[1].rms_change, Some(0.0));
    }

    #[test]
    fn repeated_gaps_and_initial_missing_samples_are_not_hidden() {
        let format = Format::new(44_100, 1).unwrap();
        let mut assembler = FrameAssembler::new(format, 123);
        let first = assembler.push(1, &[0.5]).unwrap();
        assert_eq!(first.gap.unwrap().skipped_samples_per_channel, 1);
        let second = assembler.push(3, &[0.0]).unwrap();
        let gap = second.gap.unwrap();
        assert_eq!(gap.skipped_samples_per_channel, 1);
        assert_eq!(gap.discarded_partial_samples_per_channel, 1);
        let completed = assembler
            .push(4, &vec![0.0; format.sample_count() - 1])
            .unwrap();
        let feature = &completed.features[0];
        assert_eq!(feature.rms, 0.0);
        assert_eq!(feature.frame_start_ns, 123 + 3_000_000_000_u64 / 44_100);
        assert!(feature.discontinuity);
        assert_eq!(feature.rms_change, None);
    }

    #[test]
    fn rejected_chunks_do_not_mutate_partial_pcm_clock_or_comparison_state() {
        let format = Format::new(8_000, 2).unwrap();
        let mut assembler = FrameAssembler::new(format, 0);
        assembler
            .push(0, &vec![0.5; format.sample_count() + 2])
            .unwrap();
        let pending = assembler.pending.clone();
        let previous = assembler.analyzer.previous;
        let next = assembler.next_sample;
        let pending_start = assembler.pending_start;
        let mut invalid_tail = vec![0.0; format.sample_count() * MAX_CHUNK_FRAMES];
        *invalid_tail.last_mut().unwrap() = f32::NAN;
        for (start, samples, error) in [
            (next, vec![], FrameError::InvalidLength),
            (next, vec![0.0], FrameError::InvalidLength),
            (
                next,
                vec![0.0; format.sample_count() * MAX_CHUNK_FRAMES + 2],
                FrameError::InvalidLength,
            ),
            (next + 10, invalid_tail, FrameError::InvalidSample),
            (next, vec![f32::INFINITY; 2], FrameError::InvalidSample),
            (next, vec![-1.001; 2], FrameError::InvalidSample),
            (next - 1, vec![0.0; 2], FrameError::InvalidTime),
            (u64::MAX, vec![0.0; 2], FrameError::InvalidTime),
            (u64::MAX - 1, vec![0.0; 2], FrameError::InvalidTime),
        ] {
            assert_eq!(assembler.push(start, &samples).unwrap_err(), error);
            assert_eq!(assembler.pending, pending);
            assert_eq!(assembler.next_sample, next);
            assert_eq!(assembler.pending_start, pending_start);
            assert_eq!(assembler.analyzer.previous, previous);
            assert!(!assembler.discontinuity);
        }
        let recovered = assembler
            .push(next, &vec![0.5; format.sample_count() - 2])
            .unwrap();
        assert_eq!(recovered.features.len(), 1);
        assert_eq!(recovered.features[0].rms_change, Some(0.0));
        assert!(!recovered.features[0].discontinuity);
    }

    #[test]
    fn reset_discards_partial_source_and_marks_new_origin_as_discontinuous() {
        let format = Format::new(48_000, 2).unwrap();
        let mut assembler = FrameAssembler::new(format, 99_000);
        assembler
            .push(0, &vec![1.0; format.sample_count() + 4])
            .unwrap();
        assert_eq!(assembler.reset(123), 2);
        assert!(assembler.pending.is_empty());
        assert_eq!(assembler.next_sample, 0);
        let batch = assembler
            .push(0, &vec![0.0; format.sample_count()])
            .unwrap();
        assert!(batch.gap.is_none());
        assert!(batch.features[0].discontinuity);
        assert_eq!(batch.features[0].frame_start_ns, 123);
        assert_eq!(batch.features[0].rms, 0.0);
        assert_eq!(batch.features[0].rms_change, None);
        assert_eq!(assembler.reset(0), 0);
    }

    #[test]
    fn maximum_chunk_output_and_pending_memory_stay_bounded() {
        let format = Format::new(96_000, 8).unwrap();
        let mut assembler = FrameAssembler::new(format, 0);
        assembler
            .push(0, &vec![0.0; format.sample_count() - 8])
            .unwrap();
        let capacity = assembler.pending.capacity();
        let start = (format.sample_count() / 8 - 1) as u64;
        let batch = assembler
            .push(start, &vec![0.0; format.sample_count() * MAX_CHUNK_FRAMES])
            .unwrap();
        assert_eq!(batch.features.len(), MAX_CHUNK_FRAMES);
        assert_eq!(assembler.pending.len(), format.sample_count() - 8);
        assert_eq!(assembler.pending.capacity(), capacity);
        assert_eq!(
            batch.buffered_samples_per_channel,
            format.sample_count() / 8 - 1
        );
        assert!(
            batch
                .features
                .iter()
                .all(|feature| feature.channel_rms.len() == 8)
        );
    }

    #[test]
    fn absolute_sample_mapping_avoids_rounding_drift_over_many_small_chunks() {
        let format = Format::new(44_100, 1).unwrap();
        let mut assembler = FrameAssembler::new(format, 999);
        let chunk = [0.0; 37];
        let mut count = 0;
        for index in 0..10_000 {
            let batch = assembler.push(index * 37, &chunk).unwrap();
            for feature in batch.features {
                assert_eq!(
                    feature.frame_start_ns,
                    999 + count * super::super::FRAME_DURATION_NS
                );
                count += 1;
            }
        }
        assert_eq!(count, 370_000 / 882);
        assert_eq!(assembler.pending.len(), 370_000 % 882);
    }

    #[test]
    fn clock_overflow_rejects_complete_chunk_without_partial_acceptance() {
        let format = Format::new(8_000, 1).unwrap();
        let mut assembler = FrameAssembler::new(format, u64::MAX - super::super::FRAME_DURATION_NS);
        assert_eq!(
            assembler
                .push(0, &vec![0.0; format.sample_count() + 1])
                .unwrap_err(),
            FrameError::InvalidTime
        );
        assert!(assembler.pending.is_empty());
        assert_eq!(assembler.next_sample, 0);
        let batch = assembler
            .push(0, &vec![0.0; format.sample_count()])
            .unwrap();
        assert_eq!(batch.features.len(), 1);
        assert_eq!(
            batch.features[0].frame_start_ns + batch.features[0].duration_ns,
            u64::MAX
        );
    }

    #[test]
    fn fractional_sample_origin_and_pending_pcm_are_atomic_at_clock_limit() {
        let format = Format::new(44_100, 1).unwrap();
        let end_sample = 3 + format.sample_count() as u64;
        let elapsed = end_sample * 1_000_000_000 / 44_100;
        let mut assembler = FrameAssembler::new(format, u64::MAX - elapsed);
        assembler.push(3, &[0.5]).unwrap();
        assert_eq!(
            assembler
                .push(4, &vec![0.0; format.sample_count()])
                .unwrap_err(),
            FrameError::InvalidTime
        );
        assert_eq!(assembler.pending, vec![0.5]);
        assert_eq!(assembler.next_sample, 4);
        assert!(assembler.discontinuity);
        let batch = assembler
            .push(4, &vec![0.5; format.sample_count() - 1])
            .unwrap();
        assert_eq!(batch.features.len(), 1);
        assert_eq!(batch.features[0].rms, 0.5);
        assert!(batch.features[0].discontinuity);
        assert_eq!(batch.features[0].rms_change, None);
        assert_eq!(
            batch.features[0].frame_start_ns + batch.features[0].duration_ns,
            u64::MAX
        );
        assert!(assembler.pending.is_empty());
    }
}
