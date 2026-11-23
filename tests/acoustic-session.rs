use intellicord::acoustic::{
    AcousticSession, Format, FrameError, ProducerFailure, SessionError, SessionFault, SessionStatus,
};

#[test]
fn status_serialization_preserves_existing_faults_and_labels_every_producer_reason() {
    for (fault, encoded) in [
        (
            SessionFault::UnsupportedFormat,
            serde_json::json!("unsupported_format"),
        ),
        (
            SessionFault::InvalidLength,
            serde_json::json!("invalid_length"),
        ),
        (
            SessionFault::InvalidSample,
            serde_json::json!("invalid_sample"),
        ),
        (SessionFault::InvalidTime, serde_json::json!("invalid_time")),
        (
            SessionFault::Producer(ProducerFailure::SourceLost),
            serde_json::json!({"producer":"source_lost"}),
        ),
        (
            SessionFault::Producer(ProducerFailure::FormatChanged),
            serde_json::json!({"producer":"format_changed"}),
        ),
        (
            SessionFault::Producer(ProducerFailure::InputOverflow),
            serde_json::json!({"producer":"input_overflow"}),
        ),
    ] {
        assert_eq!(
            serde_json::to_value(SessionStatus::Faulted { error: fault }).unwrap(),
            serde_json::json!({"state":"faulted","error":encoded})
        );
    }
    assert_eq!(
        serde_json::to_value(SessionStatus::Active).unwrap(),
        serde_json::json!({"state":"active"})
    );
    assert_eq!(
        serde_json::to_value(SessionStatus::Stopped).unwrap(),
        serde_json::json!({"state":"stopped"})
    );
}

#[test]
fn public_lifecycle_requires_fresh_activation_and_exposes_fault_reason() {
    let mut session = AcousticSession::default();
    assert_eq!(session.status(), SessionStatus::Stopped);
    let format = Format::new(44_100, 2).unwrap();
    let original = session.start(format, 1_000).unwrap();
    let bad = session.push(&original, 0, &[f32::NAN, 0.0]);
    assert!(matches!(
        bad,
        Err(SessionError::InvalidFrame(FrameError::InvalidSample))
    ));
    assert_eq!(
        session.status(),
        SessionStatus::Faulted {
            error: SessionFault::InvalidSample
        }
    );
    let current = session.start(format, 2_000).unwrap();
    assert!(matches!(
        session.push(&original, 0, &[0.0; 2]),
        Err(SessionError::StalePermit)
    ));
    let feature = session
        .push(&current, 0, &vec![0.0; format.sample_count()])
        .unwrap()
        .features
        .remove(0);
    assert_eq!(feature.spectral.levels.frame_start_ns, 2_000);
    assert!(feature.best_similarity.is_none());
    session.stop();
    assert_eq!(session.status(), SessionStatus::Stopped);
    assert!(matches!(
        session.push(&current, 882, &[0.0; 2]),
        Err(SessionError::Inactive)
    ));
}

#[test]
fn producer_failures_clear_processing_and_require_explicit_restart() {
    let format = Format::new(8_000, 1).unwrap();
    for reason in [
        ProducerFailure::SourceLost,
        ProducerFailure::FormatChanged,
        ProducerFailure::InputOverflow,
    ] {
        let mut session = AcousticSession::default();
        let permit = session.start(format, 0).unwrap();
        let tone: Vec<_> = (0..160)
            .map(|index| (std::f32::consts::TAU * index as f32 / 8.0).sin() * 0.5)
            .collect();
        for index in 0..10 {
            let batch = session.push(&permit, index * 160, &tone).unwrap();
            assert_eq!(batch.features[0].best_similarity.is_some(), index == 9);
        }
        session.push(&permit, 1600, &[0.5; 10]).unwrap();
        assert_eq!(session.report_failure(&permit.clone(), reason), Ok(()));
        let faulted = SessionStatus::Faulted {
            error: SessionFault::Producer(reason),
        };
        assert_eq!(session.status(), faulted);
        assert!(matches!(
            session.push(&permit, 1610, &[0.0; 150]),
            Err(SessionError::Inactive)
        ));
        assert_eq!(
            session.report_failure(&permit, ProducerFailure::SourceLost),
            Err(SessionError::Inactive)
        );
        assert_eq!(session.status(), faulted);
        let current = session.start(format, 1_000).unwrap();
        for index in 0..10 {
            let batch = session.push(&current, index * 160, &tone).unwrap();
            assert_eq!(batch.features[0].best_similarity.is_some(), index == 9);
            if index == 0 {
                assert_eq!(batch.features[0].spectral.levels.frame_start_ns, 1_000);
                assert!(batch.features[0].spectral.levels.rms_change.is_none());
            }
        }
    }
}

#[test]
fn delayed_and_foreign_failure_callbacks_do_not_stop_current_activation() {
    let format = Format::new(8_000, 1).unwrap();
    let mut session = AcousticSession::default();
    let old = session.start(format, 0).unwrap();
    session.stop();
    assert_eq!(
        session.report_failure(&old, ProducerFailure::SourceLost),
        Err(SessionError::Inactive)
    );
    assert_eq!(session.status(), SessionStatus::Stopped);
    let current = session.start(format, 5_000).unwrap();
    let mut other = AcousticSession::default();
    let foreign = other.start(format, 0).unwrap();
    session.push(&current, 0, &[0.5; 10]).unwrap();
    for permit in [&old, &foreign] {
        for reason in [
            ProducerFailure::SourceLost,
            ProducerFailure::FormatChanged,
            ProducerFailure::InputOverflow,
        ] {
            assert_eq!(
                session.report_failure(permit, reason),
                Err(SessionError::StalePermit)
            );
            assert_eq!(session.status(), SessionStatus::Active);
        }
    }
    let batch = session.push(&current, 10, &[0.5; 150]).unwrap();
    assert_eq!(batch.features[0].spectral.levels.rms, 0.5);
    assert_eq!(batch.features[0].spectral.levels.frame_start_ns, 5_000);
    assert_eq!(other.status(), SessionStatus::Active);
}

#[test]
fn producer_failure_status_has_bounded_source_labeled_serialization() {
    let mut session = AcousticSession::default();
    let permit = session.start(Format::new(8_000, 1).unwrap(), 0).unwrap();
    session
        .report_failure(&permit, ProducerFailure::SourceLost)
        .unwrap();
    assert_eq!(
        serde_json::to_value(session.status()).unwrap(),
        serde_json::json!({"state":"faulted","error":{"producer":"source_lost"}})
    );
    session.stop();
    assert_eq!(session.status(), SessionStatus::Stopped);
}
