use intellicord::acoustic::{
    AcousticSession, Format, ProducerFailure, SessionError, SessionOutput, SessionStatus,
};

fn format() -> Format {
    Format::new(8_000, 1).unwrap()
}

#[test]
fn current_output_preserves_direct_batch_and_partial_gap_metadata() {
    let mut tagged = AcousticSession::default();
    let permit = tagged.start(format(), 10).unwrap();
    let mut direct = AcousticSession::default();
    let direct_permit = direct.start(format(), 10).unwrap();
    for (position, samples) in [
        (0, vec![0.5; 10]),
        (15, vec![0.0; 160]),
        (175, vec![0.25; 640]),
    ] {
        let output: SessionOutput = tagged.push_tagged(&permit, position, &samples).unwrap();
        let expected = direct.push(&direct_permit, position, &samples).unwrap();
        let batch = tagged.accept_output(output).unwrap();
        assert_eq!(
            serde_json::to_value(batch).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[test]
fn stopped_or_restarted_session_rejects_queued_outputs() {
    let mut session = AcousticSession::default();
    let old = session.start(format(), 0).unwrap();
    let before_stop = session.push_tagged(&old, 0, &[0.0; 160]).unwrap();
    let before_restart = session.push_tagged(&old, 160, &[0.0; 160]).unwrap();
    session.stop();
    assert!(matches!(
        session.accept_output(before_stop),
        Err(SessionError::Inactive)
    ));
    let current = session.start(format(), 0).unwrap();
    session.push(&current, 0, &[0.5; 10]).unwrap();
    assert!(matches!(
        session.accept_output(before_restart),
        Err(SessionError::StalePermit)
    ));
    assert_eq!(session.status(), SessionStatus::Active);
    let output = session.push_tagged(&current, 10, &[0.5; 150]).unwrap();
    assert_eq!(
        session.accept_output(output).unwrap().features[0]
            .spectral
            .levels
            .rms,
        0.5
    );
}

#[test]
fn both_fault_paths_reject_existing_results_and_cannot_revalidate_them() {
    for producer_fault in [false, true] {
        let mut session = AcousticSession::default();
        let permit = session.start(format(), 0).unwrap();
        let first = session.push_tagged(&permit, 0, &[0.0; 160]).unwrap();
        let second = session.push_tagged(&permit, 160, &[0.0; 160]).unwrap();
        if producer_fault {
            session
                .report_failure(&permit, ProducerFailure::SourceLost)
                .unwrap();
        } else {
            assert!(session.push_tagged(&permit, 320, &[f32::NAN]).is_err());
        }
        let faulted = session.status();
        assert!(matches!(
            session.accept_output(first),
            Err(SessionError::Inactive)
        ));
        assert_eq!(session.status(), faulted);
        session.start(format(), 0).unwrap();
        assert!(matches!(
            session.accept_output(second),
            Err(SessionError::StalePermit)
        ));
    }
}

#[test]
fn foreign_output_is_rejected_without_invalidating_its_origin() {
    let mut origin = AcousticSession::default();
    let permit = origin.start(format(), 0).unwrap();
    let mut other = AcousticSession::default();
    other.start(format(), 0).unwrap();
    let foreign = origin.push_tagged(&permit, 0, &[0.0; 160]).unwrap();
    assert!(matches!(
        other.accept_output(foreign),
        Err(SessionError::StalePermit)
    ));
    assert_eq!(other.status(), SessionStatus::Active);
    let current = origin.push_tagged(&permit, 160, &[0.0; 160]).unwrap();
    assert_eq!(origin.accept_output(current).unwrap().features.len(), 1);
}
