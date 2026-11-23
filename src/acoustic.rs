use serde::Serialize;

mod feed;
pub use feed::AcousticFeed;

mod clock;
pub use clock::{ClockError, ClockInterval, SampleClockMapping, SourceClockMapping};

mod pcm;
pub use pcm::{PcmEncoding, PcmPlane, decode_pcm_chunk, decode_pcm_planes};

mod session;
pub use session::{
    AcousticSession, BufferedAcousticSession, BufferedTimedAcousticSession, OUTPUT_QUEUE_CAPACITY,
    OutputCounts, OutputHealth, ProducerFailure, SessionError, SessionFault, SessionOutput,
    SessionPermit, SessionStatus, TimedAcousticSession, TimedSessionOutput,
};

mod framing;
pub use framing::{
    AssemblyBatch, FrameAssembler, MAX_CHUNK_FRAMES, RepetitionFrameAssembler, SampleGap,
    SpectralFrameAssembler, TimedFrameError, TimedRepetitionFeatures,
    TimedRepetitionFrameAssembler,
};
mod spectral;
pub use spectral::{SpectralAnalyzer, SpectralFeatures};
mod repetition;
pub use repetition::{RepetitionAnalyzer, RepetitionFeatures};

pub const FRAME_DURATION_NS: u64 = 20_000_000;
pub const MAX_CHANNELS: usize = 8;
pub const MAX_SAMPLES: usize = 96_000 / 50 * MAX_CHANNELS;
const CLIP_LEVEL: f64 = 0.999;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    UnsupportedFormat,
    InvalidLength,
    InvalidSample,
    InvalidTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    sample_rate: u32,
    channels: usize,
}

impl Format {
    pub fn new(sample_rate: u32, channels: usize) -> Result<Self, FrameError> {
        if !(8_000..=96_000).contains(&sample_rate)
            || !sample_rate.is_multiple_of(50)
            || !(1..=MAX_CHANNELS).contains(&channels)
        {
            return Err(FrameError::UnsupportedFormat);
        }
        Ok(Self {
            sample_rate,
            channels,
        })
    }

    pub fn sample_count(self) -> usize {
        self.sample_rate as usize / 50 * self.channels
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Features {
    pub schema_version: u16,
    pub feature_version: &'static str,
    pub frame_start_ns: u64,
    pub duration_ns: u64,
    pub sample_rate: u32,
    pub channels: usize,
    pub rms: f64,
    pub peak: f64,
    pub clipped_sample_ratio: f64,
    pub clipping_threshold: f64,
    pub max_channel_rms: f64,
    pub channel_rms: Vec<f64>,
    pub rms_change: Option<f64>,
    pub discontinuity: bool,
    pub attribution: &'static str,
}

#[derive(Default)]
pub struct Analyzer {
    previous: Option<(Format, u64, f64)>,
}

impl Analyzer {
    pub fn reset(&mut self) {
        self.previous = None;
    }

    pub fn process(
        &mut self,
        format: Format,
        frame_start_ns: u64,
        interleaved: &[f32],
    ) -> Result<Features, FrameError> {
        if interleaved.len() != format.sample_count() || interleaved.len() > MAX_SAMPLES {
            return Err(FrameError::InvalidLength);
        }
        if interleaved
            .iter()
            .any(|sample| !sample.is_finite() || sample.abs() > 1.0)
        {
            return Err(FrameError::InvalidSample);
        }
        let frame_end_ns = frame_start_ns
            .checked_add(FRAME_DURATION_NS)
            .ok_or(FrameError::InvalidTime)?;
        if self
            .previous
            .is_some_and(|(_, end, _)| frame_start_ns < end)
        {
            return Err(FrameError::InvalidTime);
        }
        let mut energy = [0.0; MAX_CHANNELS];
        let mut peak: f64 = 0.0;
        let mut clipped = 0;
        for frame in interleaved.chunks_exact(format.channels) {
            for (channel, sample) in frame.iter().enumerate() {
                let sample = f64::from(*sample);
                energy[channel] += sample * sample;
                peak = peak.max(sample.abs());
                clipped += usize::from(sample.abs() >= CLIP_LEVEL);
            }
        }
        let samples_per_channel = format.sample_rate as f64 / 50.0;
        let channel_rms: Vec<_> = energy[..format.channels]
            .iter()
            .map(|sum| (sum / samples_per_channel).sqrt())
            .collect();
        let rms = (energy[..format.channels].iter().sum::<f64>() / interleaved.len() as f64).sqrt();
        let contiguous = self.previous.filter(|(previous_format, end, _)| {
            *previous_format == format && *end == frame_start_ns
        });
        let features = Features {
            schema_version: 1,
            feature_version: "acoustic-levels/0.1",
            frame_start_ns,
            duration_ns: FRAME_DURATION_NS,
            sample_rate: format.sample_rate,
            channels: format.channels,
            rms,
            peak,
            clipped_sample_ratio: clipped as f64 / interleaved.len() as f64,
            clipping_threshold: CLIP_LEVEL,
            max_channel_rms: channel_rms.iter().copied().fold(0.0, f64::max),
            channel_rms,
            rms_change: contiguous.map(|(_, _, previous_rms)| rms - previous_rms),
            discontinuity: self.previous.is_some() && contiguous.is_none(),
            attribution: "Unattributed supplied PCM; no participant or capture-source verification",
        };
        self.previous = Some((format, frame_end_ns, rms));
        Ok(features)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    #[test]
    fn silence_and_constant_level_have_known_energy() {
        let format = Format::new(48_000, 1).unwrap();
        let mut analyzer = Analyzer::default();
        let silent = analyzer
            .process(format, 0, &vec![0.0; format.sample_count()])
            .unwrap();
        close(silent.rms, 0.0);
        close(silent.peak, 0.0);
        assert_eq!(silent.rms_change, None);
        assert!(!silent.discontinuity);
        let level = analyzer
            .process(
                format,
                FRAME_DURATION_NS,
                &vec![-0.5; format.sample_count()],
            )
            .unwrap();
        close(level.rms, 0.5);
        close(level.peak, 0.5);
        close(level.max_channel_rms, 0.5);
        close(level.rms_change.unwrap(), 0.5);
        close(level.clipped_sample_ratio, 0.0);
    }

    #[test]
    fn sine_wave_matches_analytic_rms() {
        for sample_rate in [8_000, 44_100, 48_000, 96_000] {
            let format = Format::new(sample_rate, 1).unwrap();
            let samples: Vec<_> = (0..format.sample_count())
                .map(|index| {
                    (0.5 * (std::f64::consts::TAU * 1000.0 * index as f64 / sample_rate as f64)
                        .sin()) as f32
                })
                .collect();
            let features = Analyzer::default().process(format, 0, &samples).unwrap();
            close(features.rms, 0.5 / 2.0_f64.sqrt());
            close(features.clipped_sample_ratio, 0.0);
        }
    }

    #[test]
    fn opposite_phase_channels_do_not_cancel_energy() {
        let format = Format::new(48_000, 2).unwrap();
        let samples: Vec<_> = [0.5, -0.5]
            .into_iter()
            .cycle()
            .take(format.sample_count())
            .collect();
        let features = Analyzer::default().process(format, 0, &samples).unwrap();
        close(features.rms, 0.5);
        close(features.max_channel_rms, 0.5);
        assert_eq!(features.channel_rms, vec![0.5, 0.5]);
        let samples: Vec<_> = [1.0, 0.0]
            .into_iter()
            .cycle()
            .take(format.sample_count())
            .collect();
        let features = Analyzer::default().process(format, 0, &samples).unwrap();
        close(features.rms, 1.0 / 2.0_f64.sqrt());
        close(features.max_channel_rms, 1.0);
        close(features.clipped_sample_ratio, 0.5);
    }

    #[test]
    fn clipping_counts_both_polarities_at_the_documented_threshold() {
        let format = Format::new(8_000, 1).unwrap();
        let samples: Vec<_> = [1.0, -1.0, 0.998, -0.998]
            .into_iter()
            .cycle()
            .take(format.sample_count())
            .collect();
        let features = Analyzer::default().process(format, 0, &samples).unwrap();
        close(features.clipped_sample_ratio, 0.5);
        close(features.peak, 1.0);
    }

    #[test]
    fn gaps_format_changes_and_reset_do_not_invent_level_changes() {
        let mono = Format::new(48_000, 1).unwrap();
        let stereo = Format::new(48_000, 2).unwrap();
        let mut analyzer = Analyzer::default();
        analyzer
            .process(mono, 0, &vec![0.0; mono.sample_count()])
            .unwrap();
        let gap = analyzer
            .process(mono, 2 * FRAME_DURATION_NS, &vec![0.5; mono.sample_count()])
            .unwrap();
        assert!(gap.discontinuity);
        assert_eq!(gap.rms_change, None);
        let changed = analyzer
            .process(
                stereo,
                3 * FRAME_DURATION_NS,
                &vec![0.75; stereo.sample_count()],
            )
            .unwrap();
        assert!(changed.discontinuity);
        assert_eq!(changed.rms_change, None);
        let lower = analyzer
            .process(
                stereo,
                4 * FRAME_DURATION_NS,
                &vec![0.5; stereo.sample_count()],
            )
            .unwrap();
        close(lower.rms_change.unwrap(), -0.25);
        assert!(!lower.discontinuity);
        analyzer.reset();
        let restarted = analyzer
            .process(mono, 0, &vec![0.0; mono.sample_count()])
            .unwrap();
        assert_eq!(restarted.rms_change, None);
        assert!(!restarted.discontinuity);
    }

    #[test]
    fn adjacent_sample_thresholds_and_final_valid_timestamp_are_explicit() {
        let format = Format::new(8_000, 1).unwrap();
        let above = 0.999_f32;
        let below = f32::from_bits(above.to_bits() - 1);
        assert!(f64::from(above) >= CLIP_LEVEL);
        assert!(f64::from(below) < CLIP_LEVEL);
        let samples: Vec<_> = [above, below, -above, -below]
            .into_iter()
            .cycle()
            .take(format.sample_count())
            .collect();
        let mut analyzer = Analyzer::default();
        let features = analyzer
            .process(format, u64::MAX - FRAME_DURATION_NS, &samples)
            .unwrap();
        close(features.clipped_sample_ratio, 0.5);
        assert_eq!(features.clipping_threshold, CLIP_LEVEL);
        assert_eq!(analyzer.previous.unwrap().1, u64::MAX);
        assert_eq!(
            analyzer.process(format, u64::MAX, &samples).unwrap_err(),
            FrameError::InvalidTime
        );
        analyzer.reset();
        let over_unit = f32::from_bits(1.0_f32.to_bits() + 1);
        assert_eq!(
            analyzer
                .process(format, 0, &vec![over_unit; format.sample_count()])
                .unwrap_err(),
            FrameError::InvalidSample
        );
        assert!(analyzer.previous.is_none());
    }

    #[test]
    fn invalid_frames_are_atomic_and_never_advance_time() {
        let format = Format::new(8_000, 1).unwrap();
        let mut analyzer = Analyzer::default();
        let samples = vec![0.5; format.sample_count()];
        analyzer.process(format, 0, &samples).unwrap();
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.001, -1.001] {
            let mut bad = samples.clone();
            bad[0] = invalid;
            assert_eq!(
                analyzer
                    .process(format, FRAME_DURATION_NS, &bad)
                    .unwrap_err(),
                FrameError::InvalidSample
            );
        }
        for count in [
            0,
            format.sample_count() - 1,
            format.sample_count() + 1,
            MAX_SAMPLES + 1,
        ] {
            assert_eq!(
                analyzer
                    .process(format, FRAME_DURATION_NS, &vec![0.0; count])
                    .unwrap_err(),
                FrameError::InvalidLength
            );
        }
        for time in [0, FRAME_DURATION_NS - 1, u64::MAX] {
            assert_eq!(
                analyzer.process(format, time, &samples).unwrap_err(),
                FrameError::InvalidTime
            );
        }
        let recovered = analyzer
            .process(format, FRAME_DURATION_NS, &samples)
            .unwrap();
        assert!(!recovered.discontinuity);
        close(recovered.rms_change.unwrap(), 0.0);
        assert_eq!(analyzer.previous.unwrap().1, 2 * FRAME_DURATION_NS);
    }

    #[test]
    fn format_and_feature_resources_are_bounded_and_no_pcm_is_serialized() {
        for (rate, channels) in [
            (0, 1),
            (7_999, 1),
            (96_001, 1),
            (48_001, 1),
            (48_000, 0),
            (48_000, 9),
        ] {
            assert_eq!(
                Format::new(rate, channels).unwrap_err(),
                FrameError::UnsupportedFormat
            );
        }
        let format = Format::new(96_000, MAX_CHANNELS).unwrap();
        assert_eq!(format.sample_count(), MAX_SAMPLES);
        let mut samples = vec![0.0; format.sample_count()];
        let mut analyzer = Analyzer::default();
        let features = analyzer.process(format, 0, &samples).unwrap();
        samples.fill(1.0);
        assert_eq!(features.channel_rms.len(), MAX_CHANNELS);
        close(features.rms, 0.0);
        let serialized = serde_json::to_value(features).unwrap();
        assert_eq!(serialized["feature_version"], "acoustic-levels/0.1");
        assert!(
            serialized["attribution"]
                .as_str()
                .unwrap()
                .starts_with("Unattributed")
        );
        for absent in [
            "samples",
            "pcm",
            "user_id",
            "channel_id",
            "confidence",
            "micspam",
        ] {
            assert!(serialized.get(absent).is_none());
        }
        assert!(std::mem::size_of::<Analyzer>() < 128);
    }
}
