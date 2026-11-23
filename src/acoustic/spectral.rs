use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::Serialize;

use super::{Analyzer, Features, Format, FrameError};

const HIGH_BAND_START_HZ: f64 = 4_000.0;
const SILENCE_POWER: f64 = 1e-20;

#[derive(Debug, Serialize)]
pub struct SpectralFeatures {
    pub schema_version: u16,
    pub feature_version: &'static str,
    pub levels: Features,
    pub window: &'static str,
    pub bin_width_hz: f64,
    pub high_band_start_hz: f64,
    pub minimum_windowed_power: f64,
    pub centroid_hz: Option<f64>,
    pub high_band_power_ratio: Option<f64>,
    pub positive_power_flux: Option<f64>,
}

#[derive(Default)]
pub struct SpectralAnalyzer {
    levels: Analyzer,
    plan: Option<(usize, Arc<dyn Fft<f64>>)>,
    previous_power: Option<Vec<f64>>,
}

impl SpectralAnalyzer {
    pub(super) fn normalized_power(&self) -> Option<&[f64]> {
        self.previous_power.as_deref()
    }

    pub fn reset(&mut self) {
        self.levels.reset();
        self.previous_power = None;
        self.plan = None;
    }

    pub fn process(
        &mut self,
        format: Format,
        frame_start_ns: u64,
        interleaved: &[f32],
    ) -> Result<SpectralFeatures, FrameError> {
        let levels = self.levels.process(format, frame_start_ns, interleaved)?;
        Ok(self.process_levels(levels, interleaved))
    }

    pub(super) fn process_levels(
        &mut self,
        levels: Features,
        interleaved: &[f32],
    ) -> SpectralFeatures {
        let format = Format {
            sample_rate: levels.sample_rate,
            channels: levels.channels,
        };
        let sample_count = format.sample_count() / format.channels;
        if self
            .plan
            .as_ref()
            .is_none_or(|(size, _)| *size != sample_count)
        {
            self.plan = Some((
                sample_count,
                FftPlanner::new().plan_fft_forward(sample_count),
            ));
        }
        let plan = &self
            .plan
            .as_ref()
            .expect("validated frame has an FFT plan")
            .1;
        let mut buffer = vec![Complex::new(0.0, 0.0); sample_count];
        let mut scratch = vec![Complex::new(0.0, 0.0); plan.get_inplace_scratch_len()];
        let mut power = vec![0.0; sample_count / 2 + 1];
        for channel in 0..format.channels {
            let mean = interleaved
                .iter()
                .skip(channel)
                .step_by(format.channels)
                .map(|sample| f64::from(*sample))
                .sum::<f64>()
                / sample_count as f64;
            for (index, value) in buffer.iter_mut().enumerate() {
                let window =
                    0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / sample_count as f64).cos();
                *value = Complex::new(
                    (f64::from(interleaved[index * format.channels + channel]) - mean) * window,
                    0.0,
                );
            }
            plan.process_with_scratch(&mut buffer, &mut scratch);
            for (index, total) in power.iter_mut().enumerate().skip(1) {
                let weight = if index * 2 == sample_count { 1.0 } else { 2.0 };
                *total += weight * buffer[index].norm_sqr()
                    / (sample_count * sample_count * format.channels) as f64;
            }
        }
        let total = power.iter().sum::<f64>();
        let bin_width_hz = format.sample_rate as f64 / sample_count as f64;
        let audible = total > SILENCE_POWER;
        let centroid_hz = audible.then(|| {
            power
                .iter()
                .enumerate()
                .map(|(index, energy)| index as f64 * bin_width_hz * energy)
                .sum::<f64>()
                / total
        });
        let high_band_power_ratio =
            (audible && format.sample_rate as f64 / 2.0 > HIGH_BAND_START_HZ).then(|| {
                power
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index as f64 * bin_width_hz >= HIGH_BAND_START_HZ)
                    .map(|(_, energy)| energy)
                    .sum::<f64>()
                    / total
            });
        if audible {
            for energy in &mut power {
                *energy /= total;
            }
        }
        let positive_power_flux = if levels.rms_change.is_some() && audible {
            self.previous_power.as_ref().map(|previous| {
                power
                    .iter()
                    .zip(previous)
                    .map(|(current, prior)| (current - prior).max(0.0))
                    .sum::<f64>()
                    .clamp(0.0, 1.0)
            })
        } else {
            None
        };
        self.previous_power = audible.then_some(power);
        SpectralFeatures {
            schema_version: 1,
            feature_version: "acoustic-spectrum/0.1",
            levels,
            window: "Periodic Hann; per-channel mean removed; DC bin excluded; pooled one-sided power",
            bin_width_hz,
            high_band_start_hz: HIGH_BAND_START_HZ,
            minimum_windowed_power: SILENCE_POWER,
            centroid_hz,
            high_band_power_ratio,
            positive_power_flux,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acoustic::FRAME_DURATION_NS;

    fn oracle(format: Format, samples: &[f32]) -> (f64, f64) {
        let length = samples.len() / format.channels;
        let mut power = vec![0.0; length / 2 + 1];
        for channel in 0..format.channels {
            let mean = samples
                .iter()
                .skip(channel)
                .step_by(format.channels)
                .map(|value| f64::from(*value))
                .sum::<f64>()
                / length as f64;
            for (bin, energy) in power.iter_mut().enumerate().skip(1) {
                let mut real = 0.0;
                let mut imaginary = 0.0;
                for index in 0..length {
                    let phase = std::f64::consts::TAU * index as f64 / length as f64;
                    let value = (f64::from(samples[index * format.channels + channel]) - mean)
                        * (0.5 - 0.5 * phase.cos());
                    real += value * (phase * bin as f64).cos();
                    imaginary -= value * (phase * bin as f64).sin();
                }
                *energy += (real * real + imaginary * imaginary)
                    * if bin * 2 == length { 1.0 } else { 2.0 };
            }
        }
        let total = power.iter().sum::<f64>();
        let centroid = power
            .iter()
            .enumerate()
            .map(|(bin, energy)| bin as f64 * 50.0 * energy)
            .sum::<f64>()
            / total;
        let high = power
            .iter()
            .enumerate()
            .filter(|(bin, _)| *bin >= 80)
            .map(|(_, energy)| energy)
            .sum::<f64>()
            / total;
        (centroid, high)
    }

    #[test]
    fn odd_lengths_and_nyquist_match_direct_dft() {
        for rate in [8_050, 8_100, 95_950, 96_000] {
            let format = Format::new(rate, 1).unwrap();
            let samples: Vec<_> = (0..format.sample_count())
                .map(|index| {
                    let phase = std::f64::consts::TAU * index as f64 / rate as f64;
                    (0.2 * (phase * 1_000.0).sin() + 0.3 * (phase * (rate as f64 / 2.0)).cos())
                        as f32
                })
                .collect();
            let (centroid, high) = oracle(format, &samples);
            let actual = SpectralAnalyzer::default()
                .process(format, 0, &samples)
                .unwrap();
            assert!((actual.centroid_hz.unwrap() - centroid).abs() < 1e-6);
            assert!((actual.high_band_power_ratio.unwrap() - high).abs() < 1e-10);
        }
    }

    #[test]
    fn band_boundary_and_unequal_channels_are_power_weighted() {
        let mono = Format::new(48_000, 1).unwrap();
        for (frequency, expected) in [(3_950.0, 1.0 / 6.0), (4_000.0, 5.0 / 6.0), (4_050.0, 1.0)] {
            let result = SpectralAnalyzer::default()
                .process(mono, 0, &tone(mono, frequency))
                .unwrap();
            assert!((result.high_band_power_ratio.unwrap() - expected).abs() < 1e-7);
        }
        let stereo = Format::new(48_000, 2).unwrap();
        let low = tone(mono, 1_000.0);
        let high = tone(mono, 6_000.0);
        let samples: Vec<_> = low
            .iter()
            .zip(high)
            .flat_map(|(left, right)| [*left + 0.1, right * 0.5 - 0.2])
            .collect();
        let result = SpectralAnalyzer::default()
            .process(stereo, 0, &samples)
            .unwrap();
        assert!((result.centroid_hz.unwrap() - 2_000.0).abs() < 0.001);
        assert!((result.high_band_power_ratio.unwrap() - 0.2).abs() < 1e-7);
    }

    #[test]
    fn rejected_full_frames_do_not_replace_plan_or_spectral_state() {
        let format = Format::new(48_000, 1).unwrap();
        let changed = Format::new(44_100, 2).unwrap();
        let mut analyzer = SpectralAnalyzer::default();
        analyzer.process(format, 0, &tone(format, 1_000.0)).unwrap();
        let previous = analyzer.previous_power.clone();
        for value in [f32::NAN, f32::INFINITY, 1.01, -1.01] {
            let mut invalid = tone(changed, 1_000.0);
            *invalid.last_mut().unwrap() = value;
            assert!(matches!(
                analyzer.process(changed, FRAME_DURATION_NS, &invalid),
                Err(FrameError::InvalidSample)
            ));
            assert_eq!(analyzer.plan.as_ref().unwrap().0, 960);
            assert_eq!(analyzer.previous_power, previous);
        }
        let quiet = vec![0.0; format.sample_count()];
        assert_eq!(
            analyzer
                .process(format, FRAME_DURATION_NS, &quiet)
                .unwrap()
                .positive_power_flux,
            None
        );
        let recovery = analyzer
            .process(format, 2 * FRAME_DURATION_NS, &tone(format, 6_000.0))
            .unwrap();
        assert_eq!(recovery.positive_power_flux, None);
        assert!(
            analyzer
                .process(format, 3 * FRAME_DURATION_NS, &tone(format, 6_000.0))
                .unwrap()
                .positive_power_flux
                .unwrap()
                < 1e-12
        );
        let stereo = Format::new(48_000, 2).unwrap();
        assert_eq!(
            analyzer
                .process(stereo, 4 * FRAME_DURATION_NS, &tone(stereo, 6_000.0))
                .unwrap()
                .positive_power_flux,
            None
        );
        let serialized = serde_json::to_value(recovery).unwrap();
        assert_eq!(serialized["schema_version"], 1);
        assert!(serialized.get("pcm").is_none());
        assert!(serialized.get("power").is_none());
        assert!(analyzer.previous_power.as_ref().unwrap().len() <= 961);
    }

    fn tone(format: Format, hz: f64) -> Vec<f32> {
        (0..format.sample_count() / format.channels)
            .flat_map(|index| {
                let sample = (0.5
                    * (std::f64::consts::TAU * hz * index as f64 / format.sample_rate as f64).sin())
                    as f32;
                (0..format.channels)
                    .map(move |channel| if channel % 2 == 0 { sample } else { -sample })
            })
            .collect()
    }

    #[test]
    fn known_tones_and_antiphase_channels_preserve_centroid_and_band() {
        for rate in [8_000, 44_100, 48_000, 96_000] {
            for channels in [1, 2, 8] {
                let format = Format::new(rate, channels).unwrap();
                let mut analyzer = SpectralAnalyzer::default();
                let low = analyzer.process(format, 0, &tone(format, 1_000.0)).unwrap();
                assert!((low.centroid_hz.unwrap() - 1_000.0).abs() < 0.001);
                assert_eq!(low.bin_width_hz, 50.0);
                assert_eq!(low.positive_power_flux, None);
                if rate == 8_000 {
                    assert_eq!(low.high_band_power_ratio, None);
                } else {
                    assert!(low.high_band_power_ratio.unwrap() < 1e-10);
                    let high = analyzer
                        .process(format, FRAME_DURATION_NS, &tone(format, 6_000.0))
                        .unwrap();
                    assert!((high.centroid_hz.unwrap() - 6_000.0).abs() < 0.001);
                    assert!(high.high_band_power_ratio.unwrap() > 0.99999);
                    assert!(high.positive_power_flux.unwrap() > 0.99999);
                }
            }
        }
    }

    #[test]
    fn silence_dc_and_gain_do_not_fabricate_spectral_change() {
        let format = Format::new(48_000, 2).unwrap();
        let mut analyzer = SpectralAnalyzer::default();
        for (index, value) in [0.0, 0.5, -1.0].into_iter().enumerate() {
            let features = analyzer
                .process(
                    format,
                    index as u64 * FRAME_DURATION_NS,
                    &vec![value; format.sample_count()],
                )
                .unwrap();
            assert_eq!(features.centroid_hz, None);
            assert_eq!(features.high_band_power_ratio, None);
            assert_eq!(features.positive_power_flux, None);
        }
        let samples = tone(format, 1_000.0);
        let first = analyzer
            .process(format, 3 * FRAME_DURATION_NS, &samples)
            .unwrap();
        assert_eq!(first.positive_power_flux, None);
        let quiet: Vec<_> = samples.iter().map(|sample| sample * 0.25).collect();
        let second = analyzer
            .process(format, 4 * FRAME_DURATION_NS, &quiet)
            .unwrap();
        assert!(second.positive_power_flux.unwrap() < 1e-12);
    }

    #[test]
    fn invalid_frames_preserve_comparison_and_gaps_reset_it() {
        let format = Format::new(44_100, 1).unwrap();
        let samples = tone(format, 1_000.0);
        let mut analyzer = SpectralAnalyzer::default();
        analyzer.process(format, 0, &samples).unwrap();
        let previous = analyzer.previous_power.clone();
        assert!(analyzer.process(format, 0, &samples).is_err());
        assert!(
            analyzer
                .process(format, FRAME_DURATION_NS, &[f32::NAN])
                .is_err()
        );
        assert!(analyzer.process(format, u64::MAX, &samples).is_err());
        assert_eq!(analyzer.previous_power, previous);
        let next = analyzer
            .process(format, FRAME_DURATION_NS, &samples)
            .unwrap();
        assert!(next.positive_power_flux.unwrap() < 1e-12);
        let gap = analyzer
            .process(format, 3 * FRAME_DURATION_NS, &samples)
            .unwrap();
        assert!(gap.levels.discontinuity);
        assert_eq!(gap.positive_power_flux, None);
        let changed = Format::new(48_000, 1).unwrap();
        let changed = analyzer
            .process(changed, 4 * FRAME_DURATION_NS, &tone(changed, 1_000.0))
            .unwrap();
        assert_eq!(changed.positive_power_flux, None);
        analyzer.reset();
        assert!(analyzer.plan.is_none());
        assert!(analyzer.previous_power.is_none());
        assert_eq!(
            analyzer
                .process(format, 0, &samples)
                .unwrap()
                .positive_power_flux,
            None
        );
    }
}
