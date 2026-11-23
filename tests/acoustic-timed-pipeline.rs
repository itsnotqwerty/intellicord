use intellicord::acoustic::{
    ClockError, Format, FrameError, RepetitionFrameAssembler, SampleClockMapping,
    SourceClockMapping, TimedFrameError, TimedRepetitionFrameAssembler,
};

fn mapping(rate: u32, channels: usize, horizon: u64) -> SampleClockMapping {
    SampleClockMapping::new(
        SourceClockMapping::new(0, 100, 120, horizon, 100).unwrap(),
        Format::new(rate, channels).unwrap(),
        0,
    )
    .unwrap()
}

#[test]
fn timed_frames_match_nominal_processing_and_mapping_across_formats() {
    for (rate, channels) in [(8_050, 1), (44_100, 2), (96_000, 8)] {
        let frame = rate as usize / 50;
        let format = Format::new(rate, channels).unwrap();
        let mut timed = TimedRepetitionFrameAssembler::new(mapping(rate, channels, 1_000_000_000));
        let reference_clock = mapping(rate, channels, 1_000_000_000);
        let mut direct = RepetitionFrameAssembler::new(format, 0);
        let mut position = 0;
        let mut emitted = 0;
        for count in [10, frame * 4, frame * 4, frame * 2 - 10] {
            let samples: Vec<_> = (0..count * channels)
                .map(|index| {
                    if (position as usize + index / channels) % 8 < 4 {
                        0.5
                    } else {
                        -0.5
                    }
                })
                .collect();
            let batch = timed.push(position, &samples).unwrap();
            let expected = direct.push(position, &samples).unwrap();
            assert_eq!(batch.features.len(), expected.features.len());
            assert_eq!(
                batch.buffered_samples_per_channel,
                expected.buffered_samples_per_channel
            );
            for (feature, nominal) in batch.features.iter().zip(expected.features) {
                let start = emitted * frame as u64;
                assert_eq!(feature.start_sample_per_channel, start);
                assert_eq!(feature.end_sample_per_channel, start + frame as u64);
                assert_eq!(feature.start_time, reference_clock.map(start).unwrap());
                assert_eq!(
                    feature.end_time,
                    reference_clock.map(start + frame as u64).unwrap()
                );
                assert_eq!(
                    serde_json::to_value(&feature.nominal_sample_clock_features).unwrap(),
                    serde_json::to_value(nominal).unwrap()
                );
                assert_eq!(
                    feature
                        .nominal_sample_clock_features
                        .best_similarity
                        .is_some(),
                    emitted == 9
                );
                emitted += 1;
            }
            position += count as u64;
        }
        assert_eq!(emitted, 10);
    }
}

#[test]
fn gap_metadata_and_delayed_recovery_keep_absolute_sample_timing() {
    let mut frames = TimedRepetitionFrameAssembler::new(mapping(44_100, 2, 1_000_000_000));
    let clock = mapping(44_100, 2, 1_000_000_000);
    frames.push(0, &[0.5; 20]).unwrap();
    let gap = frames.push(15, &[0.5; 20]).unwrap();
    assert!(gap.features.is_empty());
    let gap = gap.gap.unwrap();
    assert_eq!(gap.skipped_samples_per_channel, 5);
    assert_eq!(gap.discarded_partial_samples_per_channel, 10);
    let batch = frames.push(25, &vec![0.5; 872 * 2]).unwrap();
    let feature = &batch.features[0];
    assert_eq!(feature.start_sample_per_channel, 15);
    assert_eq!(feature.end_sample_per_channel, 897);
    assert_eq!(feature.start_time, clock.map(15).unwrap());
    assert_eq!(feature.end_time, clock.map(897).unwrap());
    assert_eq!(batch.buffered_samples_per_channel, 0);
    assert!(batch.gap.is_none());
    let nominal = &feature.nominal_sample_clock_features;
    assert!(nominal.spectral.levels.discontinuity);
    assert!(nominal.spectral.levels.rms_change.is_none());
    assert!(nominal.spectral.positive_power_flux.is_none());
    assert!(nominal.best_similarity.is_none());
}

#[test]
fn malformed_chunks_and_clock_overflow_do_not_mutate_partial_state() {
    let mut frames = TimedRepetitionFrameAssembler::new(mapping(8_000, 1, 1_000_000_000));
    frames.push(0, &[0.5; 10]).unwrap();
    let mut invalid = vec![0.5; 310];
    invalid[309] = f32::NAN;
    for (start, samples, error) in [
        (10, invalid.as_slice(), FrameError::InvalidSample),
        (10, &[][..], FrameError::InvalidLength),
        (0, &[0.0][..], FrameError::InvalidTime),
        (u64::MAX, &[0.0][..], FrameError::InvalidTime),
    ] {
        assert!(
            matches!(frames.push(start, samples), Err(TimedFrameError::Frame(actual)) if actual == error)
        );
    }
    let batch = frames.push(10, &[0.5; 150]).unwrap();
    assert_eq!(batch.features[0].start_sample_per_channel, 0);
    assert_eq!(
        batch.features[0]
            .nominal_sample_clock_features
            .spectral
            .levels
            .rms,
        0.5
    );
    let source =
        SourceClockMapping::new(0, u64::MAX - 125_000, u64::MAX - 125_000, 1_000_000_000, 0)
            .unwrap();
    let clock = SampleClockMapping::new(source, Format::new(8_000, 1).unwrap(), 0).unwrap();
    let mut frames = TimedRepetitionFrameAssembler::new(clock);
    assert!(matches!(
        frames.push(0, &[0.0; 2]),
        Err(TimedFrameError::Clock(ClockError::Overflow))
    ));
    assert_eq!(
        frames.push(0, &[0.0]).unwrap().buffered_samples_per_channel,
        1
    );
}

#[test]
fn output_shape_distinguishes_intervals_from_nominal_timestamps_and_stays_bounded() {
    let mut frames = TimedRepetitionFrameAssembler::new(mapping(96_000, 8, 1_000_000_000));
    let batch = frames.push(0, &vec![0.0; 1_920 * 8 * 4]).unwrap();
    assert_eq!(batch.features.len(), 4);
    let json = serde_json::to_value(&batch.features[0]).unwrap();
    let mut keys: Vec<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "end_sample_per_channel",
            "end_time",
            "nominal_sample_clock_features",
            "schema_version",
            "start_sample_per_channel",
            "start_time"
        ]
    );
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["start_time"]["earliest_ns"], 100);
    assert_eq!(json["start_time"]["latest_ns"], 120);
    assert_eq!(
        json["nominal_sample_clock_features"]["spectral"]["levels"]["frame_start_ns"],
        0
    );
    assert!(matches!(
        frames.push(7_680, &vec![0.0; 1_920 * 8 * 4 + 8]),
        Err(TimedFrameError::Frame(FrameError::InvalidLength))
    ));
}

#[test]
fn expired_completing_chunk_does_not_consume_buffered_prefix() {
    let source = SourceClockMapping::new(0, 100, 120, 20_000_000, 0).unwrap();
    let clock = SampleClockMapping::new(source, Format::new(8_000, 1).unwrap(), 0).unwrap();
    let mut frames = TimedRepetitionFrameAssembler::new(clock);
    assert!(frames.push(0, &[0.5; 10]).unwrap().features.is_empty());
    assert!(matches!(
        frames.push(10, &[0.5; 151]),
        Err(TimedFrameError::Clock(ClockError::Expired))
    ));
    let batch = frames.push(10, &[0.5; 150]).unwrap();
    assert_eq!(batch.features.len(), 1);
    let feature = &batch.features[0];
    assert_eq!(feature.start_sample_per_channel, 0);
    assert_eq!(feature.end_sample_per_channel, 160);
    assert_eq!(feature.start_time.earliest_ns, 100);
    assert_eq!(feature.start_time.latest_ns, 120);
    assert_eq!(feature.end_time.latest_ns, 20_000_120);
    assert_eq!(
        feature.nominal_sample_clock_features.spectral.levels.rms,
        0.5
    );
}
