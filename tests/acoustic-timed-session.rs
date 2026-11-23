use intellicord::acoustic::{
    AcousticSession, ClockError, Format, FrameError, PcmEncoding, PcmPlane, ProducerFailure,
    SampleClockMapping, SessionError, SessionFault, SessionStatus, SourceClockMapping,
    TimedAcousticSession, TimedRepetitionFrameAssembler,
};

fn encode(sample: i16, encoding: PcmEncoding) -> Vec<u8> {
    match encoding {
        PcmEncoding::Signed16Le => sample.to_le_bytes().to_vec(),
        PcmEncoding::Signed24Le => (i32::from(sample) << 8).to_le_bytes()[..3].to_vec(),
        PcmEncoding::Signed32Le => (i32::from(sample) << 16).to_le_bytes().to_vec(),
        PcmEncoding::Float32Le => (f32::from(sample) / 32_768.0).to_le_bytes().to_vec(),
    }
}

#[test]
fn all_input_paths_match_direct_timed_batches_across_formats_and_encodings() {
    for (rate, channels) in [(8_050, 1), (44_100, 2), (96_000, 8)] {
        for encoding in [
            PcmEncoding::Signed16Le,
            PcmEncoding::Signed24Le,
            PcmEncoding::Signed32Le,
            PcmEncoding::Float32Le,
        ] {
            let make_clock = || {
                SampleClockMapping::new(
                    SourceClockMapping::new(0, 100, 120, 1_000_000_000, 100).unwrap(),
                    Format::new(rate, channels).unwrap(),
                    0,
                )
                .unwrap()
            };
            let mut session = TimedAcousticSession::default();
            let permit = session.start_encoded(make_clock(), encoding).unwrap();
            let mut direct = TimedRepetitionFrameAssembler::new(make_clock());
            let frame = rate as usize / 50;
            for (method, (position, count)) in [
                (0, 10),
                (15, frame * 4),
                (15 + frame * 4, frame * 4),
                (15 + frame * 8, frame * 2),
            ]
            .into_iter()
            .enumerate()
            {
                let integers: Vec<i16> = (0..count * channels)
                    .map(|index| if index % 8 < 4 { 16_384 } else { -8_192 })
                    .collect();
                let samples: Vec<_> = integers
                    .iter()
                    .map(|sample| f32::from(*sample) / 32_768.0)
                    .collect();
                let bytes: Vec<_> = integers
                    .iter()
                    .flat_map(|sample| encode(*sample, encoding))
                    .collect();
                let width = encode(0, encoding).len();
                let planes: Vec<_> = (0..channels)
                    .map(|channel| PcmPlane {
                        bytes: &bytes,
                        offset_bytes: channel * width,
                        stride_bytes: channels * width,
                    })
                    .collect();
                let output = match method % 3 {
                    0 => session.push(&permit, position as u64, &samples),
                    1 => session.push_bytes(&permit, position as u64, encoding, &bytes),
                    _ => session.push_planes(&permit, position as u64, encoding, count, &planes),
                }
                .unwrap();
                let batch = session.accept_output(output).unwrap();
                assert_eq!(
                    serde_json::to_value(batch).unwrap(),
                    serde_json::to_value(direct.push(position as u64, &samples).unwrap()).unwrap()
                );
            }
        }
    }
}

#[test]
fn stale_and_foreign_callbacks_and_outputs_cannot_mutate_restarted_session() {
    let mut session = TimedAcousticSession::default();
    let old = session.start(clock(100_000_000)).unwrap();
    let old_output = session.push(&old, 0, &[0.5; 160]).unwrap();
    session.stop();
    session.stop();
    assert!(matches!(
        session.push_bytes(&old, 0, PcmEncoding::Float32Le, &[]),
        Err(SessionError::Inactive)
    ));
    let current = session
        .start_encoded(clock(100_000_000), PcmEncoding::Signed16Le)
        .unwrap();
    let mut other = TimedAcousticSession::default();
    let foreign = other.start(clock(100_000_000)).unwrap();
    let foreign_output = other.push(&foreign, 0, &[0.0; 160]).unwrap();
    let mut untimed = AcousticSession::default();
    let untimed_permit = untimed.start(Format::new(8_000, 1).unwrap(), 0).unwrap();
    session.push(&current, 0, &[0.5; 10]).unwrap();
    for permit in [&old, &foreign, &untimed_permit] {
        assert!(matches!(
            session.push(permit, 0, &[f32::NAN]),
            Err(SessionError::StalePermit)
        ));
        assert!(matches!(
            session.push_bytes(permit, u64::MAX, PcmEncoding::Float32Le, &[]),
            Err(SessionError::StalePermit)
        ));
        assert!(matches!(
            session.push_planes(permit, u64::MAX, PcmEncoding::Float32Le, usize::MAX, &[]),
            Err(SessionError::StalePermit)
        ));
        assert_eq!(
            session.report_failure(permit, ProducerFailure::SourceLost),
            Err(SessionError::StalePermit)
        );
    }
    for output in [old_output, foreign_output] {
        assert!(matches!(
            session.accept_output(output),
            Err(SessionError::StalePermit)
        ));
    }
    let output = session.push(&current, 10, &[0.5; 150]).unwrap();
    assert_eq!(
        session.accept_output(output).unwrap().features[0]
            .nominal_sample_clock_features
            .spectral
            .levels
            .rms,
        0.5
    );
    assert_eq!(session.status(), SessionStatus::Active);
}

#[test]
fn encoding_checks_precede_decode_and_rejected_starts_preserve_current_binding() {
    let encodings = [
        PcmEncoding::Signed16Le,
        PcmEncoding::Signed24Le,
        PcmEncoding::Signed32Le,
        PcmEncoding::Float32Le,
    ];
    for expected in encodings {
        for supplied in encodings {
            for planar in [false, true] {
                let mut session = TimedAcousticSession::default();
                let permit = session.start_encoded(clock(100_000_000), expected).unwrap();
                assert!(matches!(
                    session.start(clock(1)),
                    Err(SessionError::AlreadyActive)
                ));
                assert!(matches!(
                    session.start_encoded(clock(1), supplied),
                    Err(SessionError::AlreadyActive)
                ));
                let prior = session.push(&permit, 0, &[0.5; 10]).unwrap();
                let bytes = encode(16_384, supplied);
                let planes = [PcmPlane {
                    bytes: &bytes,
                    offset_bytes: 0,
                    stride_bytes: bytes.len(),
                }];
                let result = if planar {
                    session.push_planes(&permit, 10, supplied, 1, &planes)
                } else {
                    session.push_bytes(&permit, 10, supplied, &bytes)
                };
                if expected == supplied {
                    let output = result.unwrap();
                    assert_eq!(
                        session
                            .accept_output(output)
                            .unwrap()
                            .buffered_samples_per_channel,
                        11
                    );
                    assert!(session.accept_output(prior).is_ok());
                } else {
                    assert!(matches!(result, Err(SessionError::FormatChanged)));
                    assert_eq!(
                        session.status(),
                        SessionStatus::Faulted {
                            error: SessionFault::Producer(ProducerFailure::FormatChanged)
                        }
                    );
                    assert!(matches!(
                        session.accept_output(prior),
                        Err(SessionError::Inactive)
                    ));
                }
            }
        }
    }
    let mut session = TimedAcousticSession::default();
    let permit = session
        .start_encoded(clock(1), PcmEncoding::Signed16Le)
        .unwrap();
    assert!(matches!(
        session.push_planes(&permit, u64::MAX, PcmEncoding::Float32Le, usize::MAX, &[]),
        Err(SessionError::FormatChanged)
    ));
}

#[test]
fn current_failures_drop_state_preserve_first_fault_and_serialize_clock_reason() {
    for method in 0..3 {
        let mut session = TimedAcousticSession::default();
        let permit = session.start(clock(20_000_000)).unwrap();
        let queued = session.push(&permit, 0, &[0.5; 10]).unwrap();
        let nan = f32::NAN.to_le_bytes();
        let views = [PcmPlane {
            bytes: &nan,
            offset_bytes: 0,
            stride_bytes: 4,
        }];
        let error = match method {
            0 => session.push(&permit, 10, &[f32::NAN]),
            1 => session.push_bytes(&permit, 10, PcmEncoding::Float32Le, &nan),
            _ => session.push_planes(&permit, 10, PcmEncoding::Float32Le, 1, &views),
        };
        assert!(matches!(
            error,
            Err(SessionError::InvalidFrame(FrameError::InvalidSample))
        ));
        let first = session.status();
        assert_eq!(
            first,
            SessionStatus::Faulted {
                error: SessionFault::InvalidSample
            }
        );
        assert!(matches!(
            session.accept_output(queued),
            Err(SessionError::Inactive)
        ));
        assert_eq!(
            session.report_failure(&permit, ProducerFailure::SourceLost),
            Err(SessionError::Inactive)
        );
        assert!(matches!(
            session.push(&permit, 0, &[0.0]),
            Err(SessionError::Inactive)
        ));
        assert_eq!(session.status(), first);
    }
    for (error, name) in [
        (ClockError::InvalidAnchor, "invalid_anchor"),
        (ClockError::BeforeAnchor, "before_anchor"),
        (ClockError::Expired, "expired"),
        (ClockError::Overflow, "overflow"),
    ] {
        assert_eq!(
            serde_json::to_value(SessionStatus::Faulted {
                error: SessionFault::Clock(error)
            })
            .unwrap(),
            serde_json::json!({"state":"faulted","error":{"clock":name}})
        );
    }
    let mapping = SampleClockMapping::new(
        SourceClockMapping::new(0, u64::MAX, u64::MAX, 1_000_000, 0).unwrap(),
        Format::new(8_000, 1).unwrap(),
        0,
    )
    .unwrap();
    let mut session = TimedAcousticSession::default();
    let permit = session.start(mapping).unwrap();
    assert!(matches!(
        session.push_bytes(&permit, 0, PcmEncoding::Signed16Le, &[0; 2]),
        Err(SessionError::InvalidClock(ClockError::Overflow))
    ));
}

#[test]
fn producer_faults_and_stop_revoke_results_and_restart_rebuilds_warmup() {
    for reason in [
        ProducerFailure::SourceLost,
        ProducerFailure::FormatChanged,
        ProducerFailure::InputOverflow,
    ] {
        let mut session = TimedAcousticSession::default();
        let permit = session.start(clock(1_000_000_000)).unwrap();
        let tone: Vec<_> = (0..160)
            .map(|index| if index % 8 < 4 { 0.5 } else { -0.5 })
            .collect();
        for index in 0..10 {
            session.push(&permit, index * 160, &tone).unwrap();
        }
        let prior = session.push(&permit, 1600, &[0.5; 10]).unwrap();
        session.report_failure(&permit, reason).unwrap();
        let fault = session.status();
        assert_eq!(
            fault,
            SessionStatus::Faulted {
                error: SessionFault::Producer(reason)
            }
        );
        assert!(matches!(
            session.accept_output(prior),
            Err(SessionError::Inactive)
        ));
        assert_eq!(
            session.report_failure(&permit, ProducerFailure::SourceLost),
            Err(SessionError::Inactive)
        );
        assert_eq!(session.status(), fault);
        let current = session.start(clock(1_000_000_000)).unwrap();
        for index in 0..10 {
            let output = session.push(&current, index * 160, &tone).unwrap();
            let batch = session.accept_output(output).unwrap();
            assert_eq!(
                batch.features[0]
                    .nominal_sample_clock_features
                    .best_similarity
                    .is_some(),
                index == 9
            );
        }
        let output = session.push(&current, 1600, &[0.0]).unwrap();
        session.stop();
        assert_eq!(session.status(), SessionStatus::Stopped);
        assert!(matches!(
            session.accept_output(output),
            Err(SessionError::Inactive)
        ));
    }
}

fn clock(horizon: u64) -> SampleClockMapping {
    SampleClockMapping::new(
        SourceClockMapping::new(0, 100, 120, horizon, 0).unwrap(),
        Format::new(8_000, 1).unwrap(),
        0,
    )
    .unwrap()
}

#[test]
fn clock_fault_revokes_output_and_requires_fresh_activation() {
    let mut session = TimedAcousticSession::default();
    assert_eq!(session.status(), SessionStatus::Stopped);
    let old = session.start(clock(20_000_000)).unwrap();
    let output = session.push(&old, 0, &[0.5; 160]).unwrap();
    assert!(matches!(
        session.push(&old, 160, &[0.5]),
        Err(SessionError::InvalidClock(ClockError::Expired))
    ));
    assert_eq!(
        session.status(),
        SessionStatus::Faulted {
            error: SessionFault::Clock(ClockError::Expired)
        }
    );
    assert!(matches!(
        session.accept_output(output),
        Err(SessionError::Inactive)
    ));
    let current = session.start(clock(40_000_000)).unwrap();
    assert!(matches!(
        session.push(&old, 0, &[f32::NAN]),
        Err(SessionError::StalePermit)
    ));
    let output = session.push(&current, 0, &[0.0; 160]).unwrap();
    let batch = session.accept_output(output).unwrap();
    assert_eq!(batch.features[0].start_time.earliest_ns, 100);
    assert!(
        batch.features[0]
            .nominal_sample_clock_features
            .best_similarity
            .is_none()
    );
}
