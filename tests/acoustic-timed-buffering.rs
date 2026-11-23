use intellicord::acoustic::{
    BufferedTimedAcousticSession, ClockError, Format, FrameError, OUTPUT_QUEUE_CAPACITY,
    PcmEncoding, PcmPlane, ProducerFailure, SampleClockMapping, SessionError, SessionFault,
    SessionStatus, SourceClockMapping, TimedRepetitionFrameAssembler,
};

fn clock(horizon: u64) -> SampleClockMapping {
    SampleClockMapping::new(
        SourceClockMapping::new(0, 100, 120, horizon, 0).unwrap(),
        Format::new(8_000, 1).unwrap(),
        0,
    )
    .unwrap()
}

#[test]
fn ninth_valid_batch_faults_and_accounts_for_all_discarded_output() {
    let mut session = BufferedTimedAcousticSession::default();
    let permit = session.start(clock(1_000_000_000)).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        session
            .push(&permit, index as u64 * 640, &[0.0; 640])
            .unwrap();
    }
    assert_eq!(session.pending_batches(), 8);
    assert_eq!(
        session.push(&permit, 5120, &[0.0; 640]),
        Err(SessionError::OutputOverflow)
    );
    assert_eq!(
        session.status(),
        SessionStatus::Faulted {
            error: SessionFault::OutputOverflow
        }
    );
    let health = session.health();
    assert_eq!(health.processed.batches, 9);
    assert_eq!(health.discarded.batches, 9);
    assert_eq!(health.discarded.frames, 36);
    assert_eq!(health.pending.batches, 0);
    assert_eq!(health.overflow_events, 1);
    assert_eq!(health.high_water_batches, 8);
    assert!(session.pop().unwrap().is_none());
}

fn conserved(session: &BufferedTimedAcousticSession) {
    let health = session.health();
    assert_eq!(
        health.processed.batches,
        health.delivered.batches + health.discarded.batches + health.pending.batches
    );
    assert_eq!(
        health.processed.frames,
        health.delivered.frames + health.discarded.frames + health.pending.frames
    );
    assert_eq!(session.pending_batches() as u64, health.pending.batches);
    assert!(health.pending.batches <= OUTPUT_QUEUE_CAPACITY as u64);
}

#[test]
fn fifo_matches_direct_output_for_mixed_layouts_partial_gaps_and_warmup() {
    for (rate, channels) in [(8_050, 1), (44_100, 2), (96_000, 8)] {
        for encoding in [
            PcmEncoding::Signed16Le,
            PcmEncoding::Signed24Le,
            PcmEncoding::Signed32Le,
            PcmEncoding::Float32Le,
        ] {
            let mapping = || {
                SampleClockMapping::new(
                    SourceClockMapping::new(0, 100, 120, 1_000_000_000, 100).unwrap(),
                    Format::new(rate, channels).unwrap(),
                    0,
                )
                .unwrap()
            };
            let mut session = BufferedTimedAcousticSession::default();
            let permit = session.start_encoded(mapping(), encoding).unwrap();
            let mut direct = TimedRepetitionFrameAssembler::new(mapping());
            let frame = rate as usize / 50;
            let mut expected = Vec::new();
            for (method, (position, count)) in [
                (0, 10),
                (15, frame * 4),
                (15 + frame * 4, frame * 4),
                (15 + frame * 8, frame * 2),
            ]
            .into_iter()
            .enumerate()
            {
                let samples: Vec<_> = (0..count * channels)
                    .map(|index| if index % 8 < 4 { 0.5_f32 } else { -0.5 })
                    .collect();
                let bytes: Vec<_> = samples
                    .iter()
                    .flat_map(|sample| match encoding {
                        PcmEncoding::Signed16Le => {
                            ((*sample * 32768.0) as i16).to_le_bytes().to_vec()
                        }
                        PcmEncoding::Signed24Le => {
                            ((*sample * 8388608.0) as i32).to_le_bytes()[..3].to_vec()
                        }
                        PcmEncoding::Signed32Le => {
                            ((*sample * 2147483648.0) as i32).to_le_bytes().to_vec()
                        }
                        PcmEncoding::Float32Le => sample.to_le_bytes().to_vec(),
                    })
                    .collect();
                let width = bytes.len() / samples.len();
                let planes: Vec<_> = (0..channels)
                    .map(|channel| PcmPlane {
                        bytes: &bytes,
                        offset_bytes: channel * width,
                        stride_bytes: channels * width,
                    })
                    .collect();
                match method % 3 {
                    0 => session.push(&permit, position as u64, &samples),
                    1 => session.push_bytes(&permit, position as u64, encoding, &bytes),
                    _ => session.push_planes(&permit, position as u64, encoding, count, &planes),
                }
                .unwrap();
                expected.push(
                    serde_json::to_value(direct.push(position as u64, &samples).unwrap()).unwrap(),
                );
                conserved(&session);
            }
            for batch in expected {
                assert_eq!(
                    serde_json::to_value(session.pop().unwrap().unwrap()).unwrap(),
                    batch
                );
                conserved(&session);
            }
            assert!(session.pop().unwrap().is_none());
            assert_eq!(session.health().delivered.batches, 4);
            assert_eq!(session.health().delivered.frames, 10);
        }
    }
}

#[test]
fn stale_callbacks_and_rejected_starts_preserve_full_queue_and_binding() {
    let mut session = BufferedTimedAcousticSession::default();
    let old = session.start(clock(1_000_000_000)).unwrap();
    session.stop();
    let current = session
        .start_encoded(clock(1_000_000_000), PcmEncoding::Signed16Le)
        .unwrap();
    let mut other = BufferedTimedAcousticSession::default();
    let foreign = other.start(clock(1_000_000_000)).unwrap();
    for index in 0..8 {
        session.push(&current, index * 160, &[0.0; 160]).unwrap();
    }
    let before = session.health();
    for permit in [&old, &foreign] {
        assert_eq!(
            session.push(permit, 0, &[f32::NAN]),
            Err(SessionError::StalePermit)
        );
        assert_eq!(
            session.push_bytes(permit, u64::MAX, PcmEncoding::Float32Le, &[]),
            Err(SessionError::StalePermit)
        );
        assert_eq!(
            session.push_planes(permit, u64::MAX, PcmEncoding::Float32Le, usize::MAX, &[]),
            Err(SessionError::StalePermit)
        );
        assert_eq!(
            session.report_failure(permit, ProducerFailure::SourceLost),
            Err(SessionError::StalePermit)
        );
        assert_eq!(session.health(), before);
    }
    assert!(matches!(
        session.start(clock(1)),
        Err(SessionError::AlreadyActive)
    ));
    assert!(matches!(
        session.start_encoded(clock(1), PcmEncoding::Float32Le),
        Err(SessionError::AlreadyActive)
    ));
    assert_eq!(session.health(), before);
    session.pop().unwrap().unwrap();
    session
        .push_bytes(&current, 1280, PcmEncoding::Signed16Le, &[0; 320])
        .unwrap();
    assert_eq!(session.pending_batches(), 8);
    conserved(&session);
}

#[test]
fn current_faults_precede_overflow_discard_backlog_and_preserve_first_reason() {
    for method in 0..7 {
        let mut session = BufferedTimedAcousticSession::default();
        let permit = session
            .start_encoded(clock(160_000_000), PcmEncoding::Signed16Le)
            .unwrap();
        for index in 0..8 {
            session.push(&permit, index * 160, &[0.0; 160]).unwrap();
        }
        let (result, expected) = match method {
            0 => (
                session.push(&permit, 1280, &[f32::NAN]),
                SessionError::InvalidFrame(FrameError::InvalidSample),
            ),
            1 => (
                session.push_bytes(&permit, 1280, PcmEncoding::Signed16Le, &[0]),
                SessionError::InvalidFrame(FrameError::InvalidLength),
            ),
            2 => (
                session.push_planes(&permit, 1280, PcmEncoding::Signed16Le, 1, &[]),
                SessionError::InvalidFrame(FrameError::InvalidLength),
            ),
            3 => (
                session.push_bytes(&permit, 1280, PcmEncoding::Float32Le, &[]),
                SessionError::FormatChanged,
            ),
            4 => (
                session.push_planes(&permit, 1280, PcmEncoding::Float32Le, usize::MAX, &[]),
                SessionError::FormatChanged,
            ),
            5 => (
                session.push_bytes(&permit, 1280, PcmEncoding::Signed16Le, &[0; 2]),
                SessionError::InvalidClock(ClockError::Expired),
            ),
            _ => (
                session.push_planes(
                    &permit,
                    1280,
                    PcmEncoding::Signed16Le,
                    1,
                    &[PcmPlane {
                        bytes: &[0; 2],
                        offset_bytes: 0,
                        stride_bytes: 2,
                    }],
                ),
                SessionError::InvalidClock(ClockError::Expired),
            ),
        };
        assert_eq!(result, Err(expected));
        let first = session.health();
        assert_eq!(first.discarded.batches, 8);
        assert_eq!(first.processed.batches, 8);
        assert_eq!(first.discarded.frames, 8);
        assert_eq!(first.overflow_events, 0);
        assert!(session.pop().unwrap().is_none());
        assert_eq!(
            session.push(&permit, 0, &[0.0]),
            Err(SessionError::Inactive)
        );
        assert_eq!(
            session.report_failure(&permit, ProducerFailure::SourceLost),
            Err(SessionError::Inactive)
        );
        assert_eq!(session.health(), first);
        conserved(&session);
        let fresh = session.start(clock(1_000_000_000)).unwrap();
        session.push(&fresh, 0, &[0.5; 160]).unwrap();
        let batch = session.pop().unwrap().unwrap();
        assert_eq!(batch.features[0].start_sample_per_channel, 0);
        assert!(
            batch.features[0]
                .nominal_sample_clock_features
                .best_similarity
                .is_none()
        );
        conserved(&session);
    }
}

#[test]
fn empty_batches_are_bounded_and_lifetime_counts_survive_failure_and_stop() {
    let mut session = BufferedTimedAcousticSession::default();
    assert!(session.pop().unwrap().is_none());
    assert_eq!(session.status(), SessionStatus::Stopped);
    for reason in [
        ProducerFailure::SourceLost,
        ProducerFailure::FormatChanged,
        ProducerFailure::InputOverflow,
    ] {
        let permit = session.start(clock(1_000_000_000)).unwrap();
        for position in 0..8 {
            session.push(&permit, position, &[0.0]).unwrap();
        }
        assert_eq!(session.health().pending.frames, 0);
        session.report_failure(&permit, reason).unwrap();
        assert_eq!(
            session.status(),
            SessionStatus::Faulted {
                error: SessionFault::Producer(reason)
            }
        );
        assert!(session.pop().unwrap().is_none());
        conserved(&session);
    }
    let permit = session.start(clock(1_000_000_000)).unwrap();
    for position in 0..8 {
        session.push(&permit, position, &[0.0]).unwrap();
    }
    session.pop().unwrap().unwrap();
    session.push(&permit, 8, &[0.0]).unwrap();
    assert_eq!(
        session.push(&permit, 9, &[0.0]),
        Err(SessionError::OutputOverflow)
    );
    let permit = session.start(clock(1_000_000_000)).unwrap();
    session.push(&permit, 0, &[0.5; 10]).unwrap();
    session.stop();
    let stopped = session.health();
    session.stop();
    assert_eq!(session.health(), stopped);
    assert_eq!(stopped.processed.batches, 35);
    assert_eq!(stopped.delivered.batches, 1);
    assert_eq!(stopped.discarded.batches, 34);
    assert_eq!(stopped.processed.frames, 0);
    let permit = session.start(clock(1_000_000_000)).unwrap();
    session.push(&permit, 0, &[0.0; 160]).unwrap();
    let batch = session.pop().unwrap().unwrap();
    assert_eq!(
        batch.features[0]
            .nominal_sample_clock_features
            .spectral
            .levels
            .rms,
        0.0
    );
    conserved(&session);
    let json = serde_json::to_value(session.health()).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["capacity_batches"], 8);
    assert_eq!(json.as_object().unwrap().len(), 9);
}
