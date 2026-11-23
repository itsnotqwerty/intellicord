use intellicord::acoustic::{
    AcousticSession, BufferedAcousticSession, Format, FrameError, OUTPUT_QUEUE_CAPACITY,
    PcmEncoding, SessionError, SessionFault, SessionStatus,
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
fn byte_input_matches_normalized_batches_and_health_through_partial_gap_and_warmup() {
    for (rate, channels) in [(8_050, 1), (44_100, 2), (96_000, 8)] {
        let format = Format::new(rate, channels).unwrap();
        let frame_samples = rate as usize / 50;
        for encoding in [
            PcmEncoding::Signed16Le,
            PcmEncoding::Signed24Le,
            PcmEncoding::Signed32Le,
            PcmEncoding::Float32Le,
        ] {
            let mut decoded = BufferedAcousticSession::default();
            let mut normalized = BufferedAcousticSession::default();
            let permit = decoded.start(format, 123).unwrap();
            let direct_permit = normalized.start(format, 123).unwrap();
            for (position, count) in [
                (0, 10),
                (15, frame_samples * 4),
                (15 + frame_samples * 4, frame_samples * 4),
                (15 + frame_samples * 8, frame_samples * 2),
            ] {
                let integers: Vec<i16> = (0..count * channels)
                    .map(|index| if index % 4 < 2 { 16_384 } else { -16_384 })
                    .collect();
                let bytes: Vec<_> = integers
                    .iter()
                    .flat_map(|sample| encode(*sample, encoding))
                    .collect();
                let samples: Vec<_> = integers
                    .iter()
                    .map(|sample| f32::from(*sample) / 32_768.0)
                    .collect();
                decoded
                    .push_bytes(&permit, position as u64, encoding, &bytes)
                    .unwrap();
                normalized
                    .push(&direct_permit, position as u64, &samples)
                    .unwrap();
                assert_eq!(decoded.health(), normalized.health());
                assert_eq!(
                    serde_json::to_value(decoded.pop().unwrap().unwrap()).unwrap(),
                    serde_json::to_value(normalized.pop().unwrap().unwrap()).unwrap()
                );
                assert_eq!(decoded.health(), normalized.health());
            }
            assert_eq!(decoded.health().processed.frames, 10);
        }
    }
}

#[test]
fn stale_and_foreign_byte_callbacks_cannot_fault_a_full_current_queue() {
    let format = Format::new(8_000, 1).unwrap();
    let mut session = BufferedAcousticSession::default();
    let old = session.start(format, 0).unwrap();
    session.stop();
    let mut other = AcousticSession::default();
    let foreign = other.start(format, 0).unwrap();
    let stopped = session.health();
    assert_eq!(
        session.push_bytes(&foreign, 0, PcmEncoding::Float32Le, &[]),
        Err(SessionError::Inactive)
    );
    assert_eq!(session.health(), stopped);
    let current = session.start(format, 0).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        session
            .push_bytes(&current, index as u64, PcmEncoding::Signed16Le, &[0; 2])
            .unwrap();
    }
    let before = session.health();
    for permit in [&old, &foreign] {
        for bytes in [vec![], f32::NAN.to_le_bytes().to_vec()] {
            assert_eq!(
                session.push_bytes(permit, 0, PcmEncoding::Float32Le, &bytes),
                Err(SessionError::StalePermit)
            );
            assert_eq!(session.health(), before);
        }
    }
}

#[test]
fn conversion_error_at_capacity_precedes_output_overflow() {
    let mut session = BufferedAcousticSession::default();
    let permit = session.start(Format::new(8_000, 1).unwrap(), 0).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        session.push(&permit, index as u64, &[0.0]).unwrap();
    }
    assert_eq!(
        session.push_bytes(
            &permit,
            OUTPUT_QUEUE_CAPACITY as u64,
            PcmEncoding::Float32Le,
            &f32::NAN.to_le_bytes()
        ),
        Err(SessionError::InvalidFrame(FrameError::InvalidSample))
    );
    let health = session.health();
    assert_eq!(
        health.status,
        SessionStatus::Faulted {
            error: SessionFault::InvalidSample
        }
    );
    assert_eq!(health.processed.batches, OUTPUT_QUEUE_CAPACITY as u64);
    assert_eq!(health.discarded, health.processed);
    assert_eq!(health.overflow_events, 0);
}

#[test]
fn valid_byte_overflow_discards_nine_batches_with_shared_accounting() {
    let mut session = BufferedAcousticSession::default();
    let permit = session.start(Format::new(8_000, 1).unwrap(), 0).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        session
            .push_bytes(
                &permit,
                index as u64 * 640,
                PcmEncoding::Signed16Le,
                &[0; 1280],
            )
            .unwrap();
    }
    assert_eq!(
        session.push_bytes(
            &permit,
            OUTPUT_QUEUE_CAPACITY as u64 * 640,
            PcmEncoding::Signed16Le,
            &[0; 1280]
        ),
        Err(SessionError::OutputOverflow)
    );
    let health = session.health();
    assert_eq!(
        health.status,
        SessionStatus::Faulted {
            error: SessionFault::OutputOverflow
        }
    );
    assert_eq!(health.processed.batches, OUTPUT_QUEUE_CAPACITY as u64 + 1);
    assert_eq!(
        health.processed.frames,
        (OUTPUT_QUEUE_CAPACITY as u64 + 1) * 4
    );
    assert_eq!(health.discarded, health.processed);
    assert_eq!(health.pending.batches, 0);
    assert_eq!(health.overflow_events, 1);
}

#[test]
fn valid_bytes_still_enforce_timestamps_and_restart_uses_new_format() {
    let mut session = BufferedAcousticSession::default();
    let old = session.start(Format::new(8_000, 1).unwrap(), 0).unwrap();
    session.push(&old, 0, &[0.5; 170]).unwrap();
    assert_eq!(
        session.push_bytes(&old, 0, PcmEncoding::Signed16Le, &[0; 320]),
        Err(SessionError::InvalidFrame(FrameError::InvalidTime))
    );
    assert_eq!(
        session.status(),
        SessionStatus::Faulted {
            error: SessionFault::InvalidTime
        }
    );
    let current = session.start(Format::new(8_000, 2).unwrap(), 77).unwrap();
    assert_eq!(
        session.push_bytes(&current, 0, PcmEncoding::Signed16Le, &[0; 2]),
        Err(SessionError::InvalidFrame(FrameError::InvalidLength))
    );
    let current = session.start(Format::new(8_000, 2).unwrap(), 77).unwrap();
    session
        .push_bytes(&current, 0, PcmEncoding::Signed16Le, &[0; 640])
        .unwrap();
    let batch = session.pop().unwrap().unwrap();
    let feature = &batch.features[0];
    assert_eq!(feature.spectral.levels.frame_start_ns, 77);
    assert_eq!(feature.spectral.levels.channels, 2);
    assert_eq!(feature.spectral.levels.rms, 0.0);
    assert!(feature.spectral.levels.rms_change.is_none());
    assert!(feature.best_similarity.is_none());
}

#[test]
fn malformed_current_bytes_fault_and_discard_backlog_without_processing_output() {
    for (encoding, bytes, error) in [
        (PcmEncoding::Signed16Le, vec![0], FrameError::InvalidLength),
        (
            PcmEncoding::Float32Le,
            [0.5_f32, f32::NAN]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect(),
            FrameError::InvalidSample,
        ),
    ] {
        let mut session = BufferedAcousticSession::default();
        let permit = session.start(Format::new(8_000, 1).unwrap(), 0).unwrap();
        session.push(&permit, 0, &[0.5; 170]).unwrap();
        let before = session.health();
        assert_eq!(
            session.push_bytes(&permit, 170, encoding, &bytes),
            Err(SessionError::InvalidFrame(error))
        );
        assert_eq!(
            session.status(),
            SessionStatus::Faulted {
                error: match error {
                    FrameError::InvalidLength => SessionFault::InvalidLength,
                    FrameError::InvalidSample => SessionFault::InvalidSample,
                    _ => unreachable!(),
                }
            }
        );
        let after = session.health();
        assert_eq!(after.pending.batches, 0);
        assert_eq!(after.processed, before.processed);
        assert_eq!(after.discarded, before.pending);
        assert_eq!(after.overflow_events, 0);
        assert!(session.pop().unwrap().is_none());
        assert_eq!(
            session.push_bytes(&permit, 0, encoding, &[]),
            Err(SessionError::Inactive)
        );
        assert_eq!(session.health(), after);
    }
}
