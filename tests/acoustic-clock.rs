use intellicord::acoustic::{
    ClockError, ClockInterval, Format, RepetitionFrameAssembler, SourceClockMapping,
};

#[test]
fn invalid_anchors_are_rejected_and_maximum_rate_error_is_explicit() {
    for (before, after, horizon, ppm) in [(2, 1, 1, 0), (0, 0, 0, 0), (0, 0, 1, 1_000_001)] {
        assert!(matches!(
            SourceClockMapping::new(0, before, after, horizon, ppm),
            Err(ClockError::InvalidAnchor)
        ));
    }
    let mapping = SourceClockMapping::new(0, 10, 20, 100, 1_000_000).unwrap();
    assert_eq!(
        mapping.map(100).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 10,
            latest_ns: 220
        }
    );
    let last_source = SourceClockMapping::new(u64::MAX, 0, 0, 1, 0).unwrap();
    assert_eq!(last_source.map(u64::MAX).unwrap().latest_ns, 0);
    assert_eq!(last_source.map(0), Err(ClockError::BeforeAnchor));
}

#[test]
fn maximum_integer_arithmetic_fails_without_wrapping_or_changing_mapping() {
    let exact = SourceClockMapping::new(0, 0, 0, u64::MAX, 0).unwrap();
    assert_eq!(exact.map(u64::MAX).unwrap().latest_ns, u64::MAX);
    let drift = SourceClockMapping::new(0, 0, 0, u64::MAX, 1_000_000).unwrap();
    assert_eq!(drift.map(u64::MAX), Err(ClockError::Overflow));
    assert_eq!(drift.map(1).unwrap().latest_ns, 2);
    let near_end = SourceClockMapping::new(10, u64::MAX - 2, u64::MAX - 1, 10, 0).unwrap();
    assert_eq!(near_end.map(12), Err(ClockError::Overflow));
    assert_eq!(near_end.map(11).unwrap().latest_ns, u64::MAX);
    assert_eq!(near_end.map(21), Err(ClockError::Expired));
    assert_eq!(near_end.map(9), Err(ClockError::BeforeAnchor));
    assert_eq!(near_end.map(10).unwrap().earliest_ns, u64::MAX - 2);
}

#[test]
fn intervals_enclose_rate_extremes_with_less_than_one_nanosecond_rounding() {
    for ppm in [0_u32, 1, 100, 999_999, 1_000_000] {
        let mapping = SourceClockMapping::new(77, 100, 123, 10_000_000, ppm).unwrap();
        let mut previous = mapping.map(77).unwrap();
        for elapsed in [0_u64, 1, 2, 999, 10_001, 999_999, 1_000_000, 10_000_000] {
            let interval = mapping.map(77 + elapsed).unwrap();
            let delta = i128::from(elapsed);
            let error_scaled = delta * i128::from(ppm);
            let lower_scaled = (100 + delta) * 1_000_000 - error_scaled;
            let upper_scaled = (123 + delta) * 1_000_000 + error_scaled;
            let lower_slack = lower_scaled - i128::from(interval.earliest_ns) * 1_000_000;
            let upper_slack = i128::from(interval.latest_ns) * 1_000_000 - upper_scaled;
            assert!((0..1_000_000).contains(&lower_slack));
            assert!((0..1_000_000).contains(&upper_slack));
            assert!(interval.earliest_ns >= previous.earliest_ns);
            assert!(interval.latest_ns >= previous.latest_ns);
            previous = interval;
        }
    }
}

#[test]
fn exact_mapping_composes_with_fractional_sample_clock_without_hidden_uncertainty() {
    let mapping = SourceClockMapping::new(9_000, 123, 123, 1_000, 0).unwrap();
    let origin = mapping.map(9_050).unwrap();
    assert_eq!(
        serde_json::to_value(origin).unwrap(),
        serde_json::json!({"schema_version": 1, "earliest_ns": 173, "latest_ns": 173})
    );
    let format = Format::new(44_100, 1).unwrap();
    let mut frames = RepetitionFrameAssembler::new(format, origin.earliest_ns);
    let batch = frames.push(1, &vec![0.0; 882]).unwrap();
    assert_eq!(
        batch.features[0].spectral.levels.frame_start_ns,
        173 + 1_000_000_000 / 44_100
    );
    assert_eq!(mapping.map(9_050).unwrap(), origin);
}

#[test]
fn bracket_and_rate_error_produce_explicit_rounded_intervals() {
    let mapping = SourceClockMapping::new(500, 1_000, 1_020, 2_000_000, 100).unwrap();
    assert_eq!(
        mapping.map(500).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 1_000,
            latest_ns: 1_020
        }
    );
    assert_eq!(
        mapping.map(501).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 1_000,
            latest_ns: 1_022
        }
    );
    assert_eq!(
        mapping.map(1_000_500).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 1_000_900,
            latest_ns: 1_001_120
        }
    );
    assert_eq!(mapping.map(499), Err(ClockError::BeforeAnchor));
    assert!(mapping.map(2_000_500).is_ok());
    assert_eq!(mapping.map(2_000_501), Err(ClockError::Expired));
}
