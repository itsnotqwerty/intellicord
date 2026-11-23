use intellicord::acoustic::{
    BufferedAcousticSession, Format, FrameError, OUTPUT_QUEUE_CAPACITY, PcmEncoding,
    ProducerFailure, SessionError, SessionFault, SessionStatus,
};

fn encodings() -> [(PcmEncoding, usize); 4] {
    [
        (PcmEncoding::Signed16Le, 2),
        (PcmEncoding::Signed24Le, 3),
        (PcmEncoding::Signed32Le, 4),
        (PcmEncoding::Float32Le, 4),
    ]
}

#[test]
fn all_encoding_pairs_reject_mismatch_and_accept_matching_input() {
    for (expected, _) in encodings() {
        for (supplied, width) in encodings() {
            let mut session = BufferedAcousticSession::default();
            let permit = session
                .start_encoded(Format::new(8_000, 1).unwrap(), 10, expected)
                .unwrap();
            let result = session.push_bytes(&permit, 0, supplied, &vec![0; 160 * width]);
            if expected != supplied {
                assert_eq!(result, Err(SessionError::FormatChanged));
                assert_eq!(session.health().processed.batches, 0);
                assert_eq!(
                    session.status(),
                    SessionStatus::Faulted {
                        error: SessionFault::Producer(ProducerFailure::FormatChanged)
                    }
                );
                assert_eq!(
                    serde_json::to_value(session.status()).unwrap(),
                    serde_json::json!({"state":"faulted","error":{"producer":"format_changed"}})
                );
            } else {
                result.unwrap();
                assert_eq!(
                    session.pop().unwrap().unwrap().features[0]
                        .spectral
                        .levels
                        .frame_start_ns,
                    10
                );
                session.push(&permit, 160, &[0.0; 160]).unwrap();
                session.pop().unwrap().unwrap();
                assert_eq!(session.status(), SessionStatus::Active);
                assert_eq!(session.health().delivered.frames, 2);
            }
        }
    }
}

#[test]
fn stale_mismatches_preserve_full_queue_and_current_mismatch_precedes_overflow() {
    let format = Format::new(8_000, 1).unwrap();
    let mut session = BufferedAcousticSession::default();
    let old = session
        .start_encoded(format, 0, PcmEncoding::Signed16Le)
        .unwrap();
    session.stop();
    let stopped = session.health();
    assert_eq!(
        session.push_bytes(&old, 0, PcmEncoding::Float32Le, &[]),
        Err(SessionError::Inactive)
    );
    assert_eq!(session.health(), stopped);
    let current = session
        .start_encoded(format, 0, PcmEncoding::Float32Le)
        .unwrap();
    let mut foreign_session = BufferedAcousticSession::default();
    let foreign = foreign_session
        .start_encoded(format, 0, PcmEncoding::Signed16Le)
        .unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        session
            .push_bytes(&current, index as u64, PcmEncoding::Float32Le, &[0; 4])
            .unwrap();
    }
    let full = session.health();
    for permit in [&old, &foreign] {
        assert_eq!(
            session.push_bytes(permit, 0, PcmEncoding::Signed16Le, &[]),
            Err(SessionError::StalePermit)
        );
        assert_eq!(session.health(), full);
    }
    assert_eq!(
        session.push_bytes(&current, 0, PcmEncoding::Signed16Le, &[]),
        Err(SessionError::FormatChanged)
    );
    let after = session.health();
    assert_eq!(after.processed, full.processed);
    assert_eq!(after.discarded, full.pending);
    assert_eq!(after.pending.batches, 0);
    assert_eq!(after.overflow_events, 0);
}

#[test]
fn rejected_starts_preserve_binding_and_restart_replaces_it() {
    let format = Format::new(8_000, 1).unwrap();
    let mut session = BufferedAcousticSession::default();
    let permit = session
        .start_encoded(format, 0, PcmEncoding::Signed16Le)
        .unwrap();
    session
        .push_bytes(&permit, 0, PcmEncoding::Signed16Le, &[0; 20])
        .unwrap();
    let before = session.health();
    assert_eq!(
        session.start(format, 10).unwrap_err(),
        SessionError::AlreadyActive
    );
    assert_eq!(
        session
            .start_encoded(format, 10, PcmEncoding::Float32Le)
            .unwrap_err(),
        SessionError::AlreadyActive
    );
    assert_eq!(session.health(), before);
    assert_eq!(
        session.push_bytes(&permit, 10, PcmEncoding::Float32Le, &[0; 4]),
        Err(SessionError::FormatChanged)
    );
    let current = session
        .start_encoded(Format::new(8_000, 2).unwrap(), 77, PcmEncoding::Float32Le)
        .unwrap();
    session
        .push_bytes(&current, 0, PcmEncoding::Float32Le, &[0; 1280])
        .unwrap();
    let batch = session.pop().unwrap().unwrap();
    assert_eq!(batch.features[0].spectral.levels.channels, 2);
    assert_eq!(batch.features[0].spectral.levels.frame_start_ns, 77);
    assert_eq!(batch.features[0].spectral.levels.rms, 0.0);
    assert!(batch.features[0].spectral.levels.rms_change.is_none());
    assert!(batch.features[0].best_similarity.is_none());
    session.stop();
    let unbound = session.start(format, 0).unwrap();
    session
        .push_bytes(&unbound, 0, PcmEncoding::Signed16Le, &[0; 320])
        .unwrap();
    session
        .push_bytes(&unbound, 160, PcmEncoding::Float32Le, &[0; 640])
        .unwrap();
    assert_eq!(session.pending_batches(), 2);
    assert_eq!(session.status(), SessionStatus::Active);
}

#[test]
fn matching_encoding_preserves_conversion_errors_and_first_fault() {
    let mut session = BufferedAcousticSession::default();
    let format = Format::new(8_000, 1).unwrap();
    let permit = session
        .start_encoded(format, 0, PcmEncoding::Float32Le)
        .unwrap();
    assert_eq!(
        session.push_bytes(&permit, 0, PcmEncoding::Float32Le, &f32::NAN.to_le_bytes()),
        Err(SessionError::InvalidFrame(FrameError::InvalidSample))
    );
    let faulted = session.health();
    assert_eq!(
        faulted.status,
        SessionStatus::Faulted {
            error: SessionFault::InvalidSample
        }
    );
    assert_eq!(
        session.push_bytes(&permit, 0, PcmEncoding::Signed16Le, &[]),
        Err(SessionError::Inactive)
    );
    assert_eq!(
        session.report_failure(&permit, ProducerFailure::FormatChanged),
        Err(SessionError::Inactive)
    );
    assert_eq!(session.health(), faulted);
}

#[test]
fn current_encoding_change_faults_and_discards_without_reinterpreting_bytes() {
    let mut session = BufferedAcousticSession::default();
    let permit = session
        .start_encoded(Format::new(8_000, 1).unwrap(), 0, PcmEncoding::Signed16Le)
        .unwrap();
    session
        .push_bytes(&permit, 0, PcmEncoding::Signed16Le, &[0; 340])
        .unwrap();
    let before = session.health();
    assert_eq!(
        session.push_bytes(&permit, 170, PcmEncoding::Float32Le, &[]),
        Err(SessionError::FormatChanged)
    );
    let after = session.health();
    assert_eq!(
        after.status,
        SessionStatus::Faulted {
            error: SessionFault::Producer(ProducerFailure::FormatChanged)
        }
    );
    assert_eq!(after.processed, before.processed);
    assert_eq!(after.discarded, before.pending);
    assert_eq!(after.pending.batches, 0);
    assert_eq!(after.overflow_events, 0);
    assert!(session.pop().unwrap().is_none());
    assert_eq!(
        session.push_bytes(&permit, 0, PcmEncoding::Signed16Le, &[0; 320]),
        Err(SessionError::Inactive)
    );
    assert_eq!(session.health(), after);
}
