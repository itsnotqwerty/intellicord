use intellicord::acoustic::{
    FRAME_DURATION_NS, Format, FrameError, MAX_CHUNK_FRAMES, SpectralAnalyzer,
    SpectralFrameAssembler,
};

fn tone(format: Format, channels: usize, rate: u32) -> Vec<f32> {
    (0..format.sample_count() / channels)
        .flat_map(|index| {
            let value =
                (0.5 * (std::f64::consts::TAU * 1_000.0 * index as f64 / rate as f64).sin()) as f32;
            (0..channels).map(move |channel| if channel % 2 == 0 { value } else { -value })
        })
        .collect()
}

#[test]
fn one_batch_preserves_order_through_tone_changes_and_silence() {
    let format = Format::new(48_000, 2).unwrap();
    let frames: Vec<Vec<f32>> = [1_000.0, 6_000.0, 0.0, 1_000.0]
        .into_iter()
        .map(|frequency| {
            (0..format.sample_count() / 2)
                .flat_map(|index| {
                    let value = (0.5
                        * (std::f64::consts::TAU * frequency * index as f64 / 48_000.0).sin())
                        as f32;
                    [value, -value]
                })
                .collect()
        })
        .collect();
    let samples: Vec<_> = frames.iter().flatten().copied().collect();
    let mut assembler = SpectralFrameAssembler::new(format, 0);
    let batch = assembler.push(0, &samples).unwrap();
    assert_eq!(batch.features.len(), 4);
    assert!(batch.features[1].positive_power_flux.unwrap() > 0.99999);
    assert_eq!(batch.features[2].positive_power_flux, None);
    assert_eq!(batch.features[3].positive_power_flux, None);
    let mut direct = SpectralAnalyzer::default();
    for (index, (actual, pcm)) in batch.features.into_iter().zip(frames).enumerate() {
        let expected = direct
            .process(format, index as u64 * FRAME_DURATION_NS, &pcm)
            .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[test]
fn chunked_spectrum_matches_direct_frames_across_formats() {
    for rate in [8_000, 8_050, 44_100, 48_000, 96_000] {
        for channels in [1, 2, 8] {
            let format = Format::new(rate, channels).unwrap();
            let frame = tone(format, channels, rate);
            let samples: Vec<_> = frame
                .iter()
                .copied()
                .cycle()
                .take(frame.len() * 5)
                .collect();
            let mut assembler = SpectralFrameAssembler::new(format, 123);
            let mut direct = SpectralAnalyzer::default();
            let mut actual = Vec::new();
            let mut position = 0;
            for chunk in samples.chunks(37 * channels) {
                let batch = assembler.push(position, chunk).unwrap();
                assert!(batch.gap.is_none());
                assert!(batch.buffered_samples_per_channel < frame.len() / channels);
                actual.extend(batch.features);
                position += (chunk.len() / channels) as u64;
            }
            assert_eq!(actual.len(), 5);
            for (index, result) in actual.into_iter().enumerate() {
                let expected = direct
                    .process(format, 123 + index as u64 * FRAME_DURATION_NS, &frame)
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
fn repeated_gaps_and_reset_discard_partial_pcm_and_comparisons() {
    let format = Format::new(44_100, 2).unwrap();
    let frame = tone(format, 2, 44_100);
    let mut assembler = SpectralFrameAssembler::new(format, 700);
    assembler.push(0, &frame).unwrap();
    assembler.push(882, &[1.0; 20]).unwrap();
    let gap = assembler.push(900, &[0.5; 2]).unwrap();
    assert!(gap.features.is_empty());
    assert_eq!(gap.gap.unwrap().discarded_partial_samples_per_channel, 10);
    let gap = assembler.push(903, &frame[..2]).unwrap();
    assert_eq!(gap.gap.unwrap().skipped_samples_per_channel, 2);
    let recovery = assembler.push(904, &frame[2..]).unwrap();
    let first = &recovery.features[0];
    assert!(first.levels.discontinuity);
    assert_eq!(first.levels.rms_change, None);
    assert_eq!(first.positive_power_flux, None);
    assert_eq!(
        first.levels.frame_start_ns,
        700 + 903_u64 * 1_000_000_000 / 44_100
    );
    let next = assembler.push(1785, &frame).unwrap();
    assert!(next.features[0].positive_power_flux.unwrap() < 1e-12);
    assembler.push(2667, &frame[..6]).unwrap();
    assert_eq!(assembler.reset(19), 3);
    let reset = assembler.push(0, &frame).unwrap();
    assert_eq!(reset.features[0].levels.frame_start_ns, 19);
    assert!(reset.features[0].levels.discontinuity);
    assert_eq!(reset.features[0].positive_power_flux, None);
}

#[test]
fn invalid_whole_chunks_leave_partial_pcm_and_spectrum_unchanged() {
    let format = Format::new(8_000, 2).unwrap();
    let frame = tone(format, 2, 8_000);
    let mut candidate = SpectralFrameAssembler::new(format, 0);
    let mut control = SpectralFrameAssembler::new(format, 0);
    for assembler in [&mut candidate, &mut control] {
        assembler.push(0, &frame).unwrap();
        assembler.push(160, &frame[..2]).unwrap();
    }
    for (position, samples, error) in [
        (161, vec![], FrameError::InvalidLength),
        (161, vec![0.0], FrameError::InvalidLength),
        (
            161,
            vec![0.0; frame.len() * MAX_CHUNK_FRAMES + 2],
            FrameError::InvalidLength,
        ),
        (160, frame.clone(), FrameError::InvalidTime),
        (u64::MAX, frame.clone(), FrameError::InvalidTime),
    ] {
        assert!(matches!(candidate.push(position, &samples), Err(found) if found == error));
    }
    for value in [f32::NAN, f32::INFINITY, 1.01] {
        let mut bad = frame.repeat(MAX_CHUNK_FRAMES);
        *bad.last_mut().unwrap() = value;
        assert!(matches!(
            candidate.push(200, &bad),
            Err(FrameError::InvalidSample)
        ));
    }
    let actual = candidate.push(161, &frame[2..]).unwrap();
    let expected = control.push(161, &frame[2..]).unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn maximum_output_and_fractional_clock_overflow_remain_bounded() {
    let format = Format::new(96_000, 8).unwrap();
    let frame = tone(format, 8, 96_000);
    let mut assembler = SpectralFrameAssembler::new(format, 0);
    assembler.push(0, &frame[..frame.len() - 8]).unwrap();
    let batch = assembler
        .push(1919, &frame.repeat(MAX_CHUNK_FRAMES))
        .unwrap();
    assert_eq!(batch.features.len(), MAX_CHUNK_FRAMES);
    assert_eq!(batch.buffered_samples_per_channel, 1919);

    let format = Format::new(44_100, 1).unwrap();
    let frame = tone(format, 1, 44_100);
    let mut assembler =
        SpectralFrameAssembler::new(format, u64::MAX - 885_u64 * 1_000_000_000 / 44_100);
    assembler.push(3, &frame[..1]).unwrap();
    assert!(matches!(
        assembler.push(4, &frame),
        Err(FrameError::InvalidTime)
    ));
    let batch = assembler.push(4, &frame[1..]).unwrap();
    assert_eq!(batch.features.len(), 1);
    assert_eq!(
        batch.features[0].levels.frame_start_ns + FRAME_DURATION_NS,
        u64::MAX
    );
    assert!(batch.features[0].levels.discontinuity);
    assert_eq!(batch.features[0].positive_power_flux, None);
}
