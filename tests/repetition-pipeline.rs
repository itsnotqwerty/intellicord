use intellicord::acoustic::{
    FRAME_DURATION_NS, Format, FrameError, MAX_CHUNK_FRAMES, RepetitionAnalyzer,
    RepetitionFrameAssembler,
};

fn tone(rate: u32, channels: usize, frequency: f64) -> Vec<f32> {
    (0..rate as usize / 50)
        .flat_map(|index| {
            let value = (0.5
                * (std::f64::consts::TAU * frequency * index as f64 / rate as f64).sin())
                as f32;
            (0..channels).map(move |channel| if channel % 2 == 0 { value } else { -value })
        })
        .collect()
}

#[test]
fn chunked_repetition_matches_direct_analysis_across_formats_and_eviction() {
    for (rate, channels) in [(8_050, 1), (44_100, 2), (48_000, 1), (96_000, 8)] {
        let format = Format::new(rate, channels).unwrap();
        let frames: Vec<_> = (0..62)
            .map(|index| {
                tone(
                    rate,
                    channels,
                    if index == 60 {
                        0.0
                    } else {
                        500.0 + (index % 5) as f64 * 500.0
                    },
                )
            })
            .collect();
        let samples: Vec<_> = frames.iter().flatten().copied().collect();
        for chunk_size in [37 * channels, format.sample_count() * MAX_CHUNK_FRAMES] {
            let mut assembler = RepetitionFrameAssembler::new(format, 123);
            let mut results = Vec::new();
            let mut position = 0;
            for chunk in samples.chunks(chunk_size) {
                let batch = assembler.push(position, chunk).unwrap();
                assert!(batch.gap.is_none());
                assert!(batch.features.len() <= MAX_CHUNK_FRAMES);
                assert!(batch.buffered_samples_per_channel < rate as usize / 50);
                results.extend(batch.features);
                position += (chunk.len() / channels) as u64;
            }
            assert_eq!(results.len(), frames.len());
            assert!(results[9].best_similarity.unwrap() > 0.99999);
            assert_eq!(results[54].compared_lags, 46);
            assert_eq!(results[60].best_similarity, None);
            assert_eq!(results[61].best_similarity, None);
            let mut direct = RepetitionAnalyzer::default();
            for (index, (result, frame)) in results.into_iter().zip(&frames).enumerate() {
                let expected = direct
                    .process(format, 123 + index as u64 * FRAME_DURATION_NS, frame)
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(result).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
        }
    }
}

#[test]
fn partial_gap_recovery_requires_ten_fresh_frames_and_reset_discards_history() {
    let format = Format::new(44_100, 2).unwrap();
    let frame = tone(44_100, 2, 1_000.0);
    let mut assembler = RepetitionFrameAssembler::new(format, 700);
    for index in 0..10 {
        assembler.push(index * 882, &frame).unwrap();
    }
    assembler.push(8_820, &frame[..20]).unwrap();
    let gap = assembler.push(8_835, &frame[..2]).unwrap();
    assert!(gap.features.is_empty());
    let gap = gap.gap.unwrap();
    assert_eq!(gap.skipped_samples_per_channel, 5);
    assert_eq!(gap.discarded_partial_samples_per_channel, 10);
    let first = assembler.push(8_836, &frame[2..]).unwrap();
    let first = &first.features[0];
    assert!(first.spectral.levels.discontinuity);
    assert_eq!(first.spectral.levels.rms_change, None);
    assert_eq!(first.spectral.positive_power_flux, None);
    assert_eq!(first.best_similarity, None);
    for index in 1..10 {
        let batch = assembler.push(8_835 + index * 882, &frame).unwrap();
        if index < 9 {
            assert_eq!(batch.features[0].compared_lags, 0);
        } else {
            assert_eq!(batch.features[0].compared_lags, 1);
            assert!(batch.features[0].best_similarity.unwrap() > 0.99999);
        }
    }
    assembler.push(17_655, &frame[..6]).unwrap();
    assert_eq!(assembler.reset(19), 3);
    let reset = assembler.push(0, &frame).unwrap();
    assert_eq!(reset.features[0].spectral.levels.frame_start_ns, 19);
    assert!(reset.features[0].spectral.levels.discontinuity);
    assert_eq!(reset.features[0].best_similarity, None);
}

#[test]
fn invalid_chunk_does_not_clear_history_or_accept_early_frames() {
    let format = Format::new(8_000, 2).unwrap();
    let frame = tone(8_000, 2, 1_000.0);
    let mut candidate = RepetitionFrameAssembler::new(format, 0);
    let mut control = RepetitionFrameAssembler::new(format, 0);
    for assembler in [&mut candidate, &mut control] {
        for index in 0..9 {
            assembler.push(index * 160, &frame).unwrap();
        }
        assembler.push(1_440, &frame[..2]).unwrap();
    }
    for (position, samples) in [
        (1_441, vec![]),
        (1_441, vec![0.0]),
        (1_441, frame.repeat(MAX_CHUNK_FRAMES + 1)),
        (1_440, frame.clone()),
        (u64::MAX, frame.clone()),
    ] {
        assert!(candidate.push(position, &samples).is_err());
    }
    for value in [f32::NAN, f32::INFINITY, 1.01] {
        let mut bad = frame.repeat(MAX_CHUNK_FRAMES);
        *bad.last_mut().unwrap() = value;
        assert!(matches!(
            candidate.push(1_500, &bad),
            Err(FrameError::InvalidSample)
        ));
    }
    let actual = candidate.push(1_441, &frame[2..]).unwrap();
    assert!(actual.features[0].best_similarity.unwrap() > 0.99999);
    let expected = control.push(1_441, &frame[2..]).unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn maximum_output_and_fractional_clock_overflow_keep_existing_bounds() {
    let format = Format::new(96_000, 8).unwrap();
    let frame = tone(96_000, 8, 1_000.0);
    let mut assembler = RepetitionFrameAssembler::new(format, 0);
    assembler.push(0, &frame[..frame.len() - 8]).unwrap();
    let batch = assembler
        .push(1_919, &frame.repeat(MAX_CHUNK_FRAMES))
        .unwrap();
    assert_eq!(batch.features.len(), MAX_CHUNK_FRAMES);
    assert_eq!(batch.buffered_samples_per_channel, 1_919);
    let format = Format::new(44_100, 1).unwrap();
    let frame = tone(44_100, 1, 1_000.0);
    let mut assembler =
        RepetitionFrameAssembler::new(format, u64::MAX - 885_u64 * 1_000_000_000 / 44_100);
    assembler.push(3, &frame[..1]).unwrap();
    assert!(matches!(
        assembler.push(4, &frame),
        Err(FrameError::InvalidTime)
    ));
    let batch = assembler.push(4, &frame[1..]).unwrap();
    assert_eq!(
        batch.features[0].spectral.levels.frame_start_ns + FRAME_DURATION_NS,
        u64::MAX
    );
    assert!(batch.features[0].spectral.levels.discontinuity);
    assert_eq!(batch.features[0].compared_lags, 0);
}
