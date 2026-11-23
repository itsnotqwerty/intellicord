use intellicord::acoustic::{
    ClockError, ClockInterval, Format, SampleClockMapping, SourceClockMapping,
};

#[test]
fn absolute_sample_offsets_do_not_accumulate_rounding_across_rates_or_channels() {
    for rate in [8_000, 8_050, 44_100, 95_950, 96_000] {
        for channels in [1, 2, 8] {
            let source = SourceClockMapping::new(500, 7, 7, 201_000_000_000, 0).unwrap();
            let mapping =
                SampleClockMapping::new(source, Format::new(rate, channels).unwrap(), 500).unwrap();
            for frame in 0..10_000_u64 {
                let sample = frame * u64::from(rate / 50) + 1;
                let interval = mapping.map(sample).unwrap();
                let whole = sample / u64::from(rate) * 1_000_000_000;
                let fraction = sample % u64::from(rate) * 1_000_000_000;
                assert_eq!(interval.earliest_ns, 7 + whole + fraction / u64::from(rate));
                assert_eq!(
                    interval.latest_ns,
                    7 + whole + fraction.div_ceil(u64::from(rate))
                );
            }
        }
    }
}

#[test]
fn source_origin_offset_and_rate_uncertainty_are_preserved() {
    let source = SourceClockMapping::new(500, 1_000, 1_020, 1_000_000_000, 100).unwrap();
    let mapping = SampleClockMapping::new(source, Format::new(44_100, 1).unwrap(), 600).unwrap();
    assert_eq!(
        mapping.map(0).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 1_099,
            latest_ns: 1_121
        }
    );
    assert_eq!(
        mapping.map(1).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 23_772,
            latest_ns: 23_799
        }
    );
    let maximum_rate = SourceClockMapping::new(0, 10, 20, 1_000_000, 1_000_000).unwrap();
    let mapping =
        SampleClockMapping::new(maximum_rate, Format::new(44_100, 1).unwrap(), 0).unwrap();
    assert_eq!(
        mapping.map(1).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 10,
            latest_ns: 45_372
        }
    );
}

#[test]
fn origins_and_both_rounded_endpoints_must_fit_the_source_horizon() {
    for (origin, expected) in [(99, ClockError::BeforeAnchor), (111, ClockError::Expired)] {
        let source = SourceClockMapping::new(100, 0, 0, 10, 0).unwrap();
        assert!(
            matches!(SampleClockMapping::new(source, Format::new(8_000, 1).unwrap(), origin), Err(error) if error == expected)
        );
    }
    let source = SourceClockMapping::new(0, 0, 0, 22_675, 0).unwrap();
    let mapping = SampleClockMapping::new(source, Format::new(44_100, 1).unwrap(), 0).unwrap();
    assert_eq!(mapping.map(1), Err(ClockError::Expired));
    assert_eq!(mapping.map(0).unwrap().latest_ns, 0);
    let source = SourceClockMapping::new(0, 0, 0, 20_000_000, 0).unwrap();
    let mapping = SampleClockMapping::new(source, Format::new(44_100, 1).unwrap(), 0).unwrap();
    assert_eq!(mapping.map(882).unwrap().latest_ns, 20_000_000);
    assert_eq!(mapping.map(883), Err(ClockError::Expired));
}

#[test]
fn source_and_backend_overflow_are_checked_without_poisoning_valid_calls() {
    let format = Format::new(8_000, 1).unwrap();
    let source = SourceClockMapping::new(u64::MAX, 0, 0, u64::MAX, 0).unwrap();
    let mapping = SampleClockMapping::new(source, format, u64::MAX).unwrap();
    assert_eq!(mapping.map(1), Err(ClockError::Overflow));
    assert_eq!(mapping.map(u64::MAX), Err(ClockError::Overflow));
    assert_eq!(mapping.map(0).unwrap().latest_ns, 0);
    let source =
        SourceClockMapping::new(0, u64::MAX - 125_000, u64::MAX - 125_000, u64::MAX, 0).unwrap();
    let mapping = SampleClockMapping::new(source, format, 0).unwrap();
    assert_eq!(mapping.map(1).unwrap().latest_ns, u64::MAX);
    assert_eq!(mapping.map(2), Err(ClockError::Overflow));
    assert_eq!(mapping.map(0).unwrap().earliest_ns, u64::MAX - 125_000);
    let source = SourceClockMapping::new(0, u64::MAX, u64::MAX, 10, 0).unwrap();
    assert!(matches!(
        SampleClockMapping::new(source, format, 1),
        Err(ClockError::Overflow)
    ));
}

#[test]
fn fractional_offsets_preserve_bracket_and_nanosecond_rounding() {
    let source = SourceClockMapping::new(1_000, 100, 120, 1_000_000_000, 0).unwrap();
    let mapping = SampleClockMapping::new(source, Format::new(44_100, 2).unwrap(), 1_000).unwrap();
    assert_eq!(
        mapping.map(0).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 100,
            latest_ns: 120
        }
    );
    assert_eq!(
        mapping.map(1).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 22_775,
            latest_ns: 22_796
        }
    );
    assert_eq!(
        mapping.map(882).unwrap(),
        ClockInterval {
            schema_version: 1,
            earliest_ns: 20_000_100,
            latest_ns: 20_000_120
        }
    );
}
