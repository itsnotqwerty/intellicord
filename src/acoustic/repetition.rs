use std::collections::VecDeque;

use serde::Serialize;

use super::{FRAME_DURATION_NS, Features, Format, FrameError, SpectralAnalyzer, SpectralFeatures};

const BANDS: usize = 32;
const WINDOW_FRAMES: usize = 5;
const MAX_LAG_FRAMES: usize = 50;
const HISTORY_FRAMES: usize = WINDOW_FRAMES + MAX_LAG_FRAMES;

#[derive(Debug, Serialize)]
pub struct RepetitionFeatures {
    pub schema_version: u16,
    pub feature_version: &'static str,
    pub spectral: SpectralFeatures,
    pub descriptor_bands: usize,
    pub comparison_window_ns: u64,
    pub minimum_lag_ns: u64,
    pub maximum_lag_ns: u64,
    pub compared_lags: usize,
    pub best_lag_ns: Option<u64>,
    pub best_similarity: Option<f64>,
}

#[derive(Default)]
pub struct RepetitionAnalyzer {
    spectral: SpectralAnalyzer,
    history: VecDeque<[f64; BANDS]>,
}

impl RepetitionAnalyzer {
    pub fn reset(&mut self) {
        self.spectral.reset();
        self.history.clear();
    }

    pub fn process(
        &mut self,
        format: Format,
        frame_start_ns: u64,
        interleaved: &[f32],
    ) -> Result<RepetitionFeatures, FrameError> {
        let spectral = self.spectral.process(format, frame_start_ns, interleaved)?;
        Ok(self.summarize(spectral))
    }

    pub(super) fn process_levels(&mut self, levels: Features, pcm: &[f32]) -> RepetitionFeatures {
        let spectral = self.spectral.process_levels(levels, pcm);
        self.summarize(spectral)
    }

    fn summarize(&mut self, spectral: SpectralFeatures) -> RepetitionFeatures {
        if spectral.levels.rms_change.is_none() {
            self.history.clear();
        }
        if let Some(power) = self.spectral.normalized_power() {
            let mut descriptor = [0.0; BANDS];
            for (index, energy) in power.iter().enumerate().skip(1) {
                descriptor[(index - 1) * BANDS / (power.len() - 1)] += energy;
            }
            if self.history.len() == HISTORY_FRAMES {
                self.history.pop_front();
            }
            self.history.push_back(descriptor);
        } else {
            self.history.clear();
        }
        let mut best_similarity = None;
        let mut best_lag_ns = None;
        let mut compared_lags = 0;
        if self.history.len() >= 2 * WINDOW_FRAMES {
            let current = self.history.len() - WINDOW_FRAMES;
            for lag in WINDOW_FRAMES..=MAX_LAG_FRAMES.min(current) {
                let similarity = (0..WINDOW_FRAMES)
                    .map(|offset| {
                        let distance = self.history[current + offset]
                            .iter()
                            .zip(self.history[current - lag + offset].iter())
                            .map(|(left, right)| (left - right).abs())
                            .sum::<f64>();
                        (1.0 - distance / 2.0).clamp(0.0, 1.0)
                    })
                    .sum::<f64>()
                    / WINDOW_FRAMES as f64;
                compared_lags += 1;
                if best_similarity.is_none_or(|previous| similarity > previous) {
                    best_similarity = Some(similarity);
                    best_lag_ns = Some(lag as u64 * FRAME_DURATION_NS);
                }
            }
        }
        RepetitionFeatures {
            schema_version: 1,
            feature_version: "acoustic-repetition/0.1",
            spectral,
            descriptor_bands: BANDS,
            comparison_window_ns: WINDOW_FRAMES as u64 * FRAME_DURATION_NS,
            minimum_lag_ns: WINDOW_FRAMES as u64 * FRAME_DURATION_NS,
            maximum_lag_ns: MAX_LAG_FRAMES as u64 * FRAME_DURATION_NS,
            compared_lags,
            best_lag_ns,
            best_similarity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(format: Format, hz: f64, gain: f64) -> Vec<f32> {
        (0..format.sample_count() / format.channels)
            .flat_map(|index| {
                let value = (gain
                    * (std::f64::consts::TAU * hz * index as f64 / format.sample_rate as f64).sin())
                    as f32;
                (0..format.channels)
                    .map(move |channel| if channel % 2 == 0 { value } else { -value })
            })
            .collect()
    }

    #[test]
    fn repeated_sequence_beats_unrelated_sequence_despite_gain_and_phase() {
        for rate in [8_050, 44_100, 96_000] {
            let format = Format::new(rate, 2).unwrap();
            let mut repeated = RepetitionAnalyzer::default();
            let mut unrelated = RepetitionAnalyzer::default();
            for index in 0..10 {
                let hz = 200.0 + (index % 5) as f64 * 300.0;
                let first = repeated
                    .process(
                        format,
                        index * FRAME_DURATION_NS,
                        &tone(format, hz, if index < 5 { 0.5 } else { 0.1 }),
                    )
                    .unwrap();
                let other = unrelated
                    .process(
                        format,
                        index * FRAME_DURATION_NS,
                        &tone(format, if index < 5 { hz } else { hz + 2_000.0 }, 0.5),
                    )
                    .unwrap();
                if index < 9 {
                    assert_eq!(first.best_similarity, None);
                } else {
                    assert!(first.best_similarity.unwrap() > 0.99999);
                    assert_eq!(first.best_lag_ns, Some(100_000_000));
                    assert_eq!(first.compared_lags, 1);
                    assert!(other.best_similarity.unwrap() < 0.01);
                }
            }
        }
    }

    #[test]
    fn silence_gaps_format_changes_and_reset_clear_history() {
        let format = Format::new(48_000, 1).unwrap();
        let mut analyzer = RepetitionAnalyzer::default();
        for index in 0..10 {
            analyzer
                .process(
                    format,
                    index * FRAME_DURATION_NS,
                    &tone(format, 1_000.0, 0.5),
                )
                .unwrap();
        }
        let silent = analyzer
            .process(
                format,
                10 * FRAME_DURATION_NS,
                &vec![0.0; format.sample_count()],
            )
            .unwrap();
        assert_eq!(silent.best_similarity, None);
        assert!(analyzer.history.is_empty());
        for index in 11..21 {
            analyzer
                .process(
                    format,
                    index * FRAME_DURATION_NS,
                    &tone(format, 1_000.0, 0.5),
                )
                .unwrap();
        }
        let gap = analyzer
            .process(format, 22 * FRAME_DURATION_NS, &tone(format, 1_000.0, 0.5))
            .unwrap();
        assert!(gap.spectral.levels.discontinuity);
        assert_eq!(gap.best_similarity, None);
        assert_eq!(analyzer.history.len(), 1);
        let stereo = Format::new(48_000, 2).unwrap();
        analyzer
            .process(stereo, 23 * FRAME_DURATION_NS, &tone(stereo, 1_000.0, 0.5))
            .unwrap();
        assert_eq!(analyzer.history.len(), 1);
        analyzer.reset();
        assert!(analyzer.history.is_empty());
        assert_eq!(
            analyzer
                .process(format, 0, &tone(format, 1_000.0, 0.5))
                .unwrap()
                .compared_lags,
            0
        );
    }

    #[test]
    fn rejected_input_preserves_history_and_comparisons() {
        let format = Format::new(8_000, 1).unwrap();
        let samples = tone(format, 1_000.0, 0.5);
        let mut analyzer = RepetitionAnalyzer::default();
        for index in 0..9 {
            analyzer
                .process(format, index * FRAME_DURATION_NS, &samples)
                .unwrap();
        }
        let previous = analyzer.history.clone();
        let mut invalid = samples.clone();
        *invalid.last_mut().unwrap() = f32::NAN;
        for (time, pcm) in [
            (9 * FRAME_DURATION_NS, invalid.as_slice()),
            (0, samples.as_slice()),
            (u64::MAX, samples.as_slice()),
            (9 * FRAME_DURATION_NS, &[]),
        ] {
            assert!(analyzer.process(format, time, pcm).is_err());
            assert_eq!(analyzer.history, previous);
        }
        let next = analyzer
            .process(format, 9 * FRAME_DURATION_NS, &samples)
            .unwrap();
        assert!(next.best_similarity.unwrap() > 0.99999);
    }

    #[test]
    fn maximum_lag_match_moves_out_of_the_bounded_history() {
        let format = Format::new(8_000, 1).unwrap();
        let samples = tone(format, 1_000.0, 0.5);
        let mut analyzer = RepetitionAnalyzer::default();
        analyzer.process(format, 0, &samples).unwrap();
        let matching = analyzer.history[0];
        let mut other = [0.0; BANDS];
        other[31] = 1.0;
        analyzer.history = (0..54)
            .map(|index| {
                if !(5..50).contains(&index) {
                    matching
                } else {
                    other
                }
            })
            .collect();
        let result = analyzer
            .process(format, FRAME_DURATION_NS, &samples)
            .unwrap();
        assert_eq!(result.compared_lags, 46);
        assert_eq!(result.best_lag_ns, Some(1_000_000_000));
        assert!(result.best_similarity.unwrap() > 0.99999);
        let evicted = analyzer
            .process(format, 2 * FRAME_DURATION_NS, &samples)
            .unwrap();
        assert_eq!(analyzer.history.len(), HISTORY_FRAMES);
        assert_eq!(evicted.best_lag_ns, Some(1_000_000_000));
        assert!((evicted.best_similarity.unwrap() - 0.8).abs() < 1e-8);
    }

    #[test]
    fn history_and_lag_search_are_bounded_and_do_not_export_descriptors() {
        let format = Format::new(8_000, 1).unwrap();
        let samples = tone(format, 1_000.0, 0.5);
        let mut analyzer = RepetitionAnalyzer::default();
        for index in 0..300 {
            let result = analyzer
                .process(format, index * FRAME_DURATION_NS, &samples)
                .unwrap();
            assert!(analyzer.history.len() <= HISTORY_FRAMES);
            assert!(result.compared_lags <= 46);
            if index >= 54 {
                assert_eq!(result.compared_lags, 46);
                assert_eq!(result.best_lag_ns, Some(100_000_000));
            }
            let json = serde_json::to_value(result).unwrap();
            assert!(json.get("history").is_none());
            assert!(json.get("descriptor").is_none());
            assert!(json.get("pcm").is_none());
        }
    }
}
