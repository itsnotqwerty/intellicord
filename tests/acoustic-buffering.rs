use intellicord::acoustic::{
    AcousticSession, BufferedAcousticSession, Format, FrameError, OUTPUT_QUEUE_CAPACITY,
    ProducerFailure, SessionError, SessionFault, SessionStatus,
};

fn format() -> Format {
    Format::new(8_000, 1).unwrap()
}

#[test]
fn invalid_current_input_takes_precedence_over_full_output_queue() {
    let mut buffered = BufferedAcousticSession::default();
    let permit = buffered.start(format(), 0).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        buffered.push(&permit, index as u64, &[0.0]).unwrap();
    }
    assert_eq!(
        buffered.push(&permit, OUTPUT_QUEUE_CAPACITY as u64, &[f32::NAN]),
        Err(SessionError::InvalidFrame(FrameError::InvalidSample))
    );
    let faulted = SessionStatus::Faulted {
        error: SessionFault::InvalidSample,
    };
    assert_eq!(buffered.status(), faulted);
    assert_eq!(buffered.pending_batches(), 0);
    assert!(buffered.pop().unwrap().is_none());
    assert_eq!(
        buffered.report_failure(&permit, ProducerFailure::SourceLost),
        Err(SessionError::Inactive)
    );
    assert_eq!(buffered.status(), faulted);
}

#[test]
fn fifo_preserves_partial_gap_and_maximum_batch_outputs() {
    let mut buffered = BufferedAcousticSession::default();
    assert_eq!(buffered.status(), SessionStatus::Stopped);
    assert!(buffered.pop().unwrap().is_none());
    let permit = buffered.start(format(), 10).unwrap();
    let mut direct = AcousticSession::default();
    let direct_permit = direct.start(format(), 10).unwrap();
    let mut expected = Vec::new();
    for (position, samples) in [
        (0, vec![0.5; 10]),
        (15, vec![0.0; 160]),
        (175, vec![0.25; 640]),
    ] {
        buffered.push(&permit, position, &samples).unwrap();
        expected.push(direct.push(&direct_permit, position, &samples).unwrap());
    }
    assert_eq!(buffered.pending_batches(), 3);
    for batch in expected {
        assert_eq!(
            serde_json::to_value(buffered.pop().unwrap().unwrap()).unwrap(),
            serde_json::to_value(batch).unwrap()
        );
    }
    assert!(buffered.pop().unwrap().is_none());
    assert_eq!(buffered.status(), SessionStatus::Active);
}

#[test]
fn overflow_clears_backlog_and_faults_until_fresh_activation() {
    let mut buffered = BufferedAcousticSession::default();
    let permit = buffered.start(format(), 0).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        buffered
            .push(&permit, index as u64 * 640, &[0.0; 640])
            .unwrap();
        assert_eq!(buffered.pending_batches(), index + 1);
    }
    assert_eq!(
        buffered.push(&permit, OUTPUT_QUEUE_CAPACITY as u64 * 640, &[0.0; 640]),
        Err(SessionError::OutputOverflow)
    );
    assert_eq!(
        buffered.status(),
        SessionStatus::Faulted {
            error: SessionFault::OutputOverflow
        }
    );
    assert_eq!(
        serde_json::to_value(buffered.status()).unwrap(),
        serde_json::json!({"state":"faulted","error":"output_overflow"})
    );
    assert_eq!(buffered.pending_batches(), 0);
    assert!(buffered.pop().unwrap().is_none());
    assert_eq!(
        buffered.push(&permit, 0, &[0.0; 160]),
        Err(SessionError::Inactive)
    );
    let current = buffered.start(format(), 1_000).unwrap();
    buffered.push(&current, 0, &[0.0; 160]).unwrap();
    let feature = buffered.pop().unwrap().unwrap().features.remove(0);
    assert_eq!(feature.spectral.levels.frame_start_ns, 1_000);
    assert!(feature.spectral.levels.rms_change.is_none());
}

#[test]
fn pop_frees_capacity_and_partial_only_batches_are_bounded() {
    let mut buffered = BufferedAcousticSession::default();
    let permit = buffered.start(format(), 0).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        buffered.push(&permit, index as u64, &[0.0]).unwrap();
    }
    let batch = buffered.pop().unwrap().unwrap();
    assert!(batch.features.is_empty());
    assert_eq!(batch.buffered_samples_per_channel, 1);
    buffered
        .push(&permit, OUTPUT_QUEUE_CAPACITY as u64, &[0.0])
        .unwrap();
    assert_eq!(buffered.pending_batches(), OUTPUT_QUEUE_CAPACITY);
    assert_eq!(
        buffered.push(&permit, OUTPUT_QUEUE_CAPACITY as u64 + 1, &[0.0]),
        Err(SessionError::OutputOverflow)
    );
}

#[test]
fn stale_callbacks_and_repeated_start_preserve_full_queue() {
    let mut buffered = BufferedAcousticSession::default();
    let old = buffered.start(format(), 0).unwrap();
    buffered.stop();
    let current = buffered.start(format(), 0).unwrap();
    let mut foreign_session = AcousticSession::default();
    let foreign = foreign_session.start(format(), 0).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        buffered.push(&current, index as u64, &[0.5]).unwrap();
    }
    assert_eq!(
        buffered.start(format(), 10).unwrap_err(),
        SessionError::AlreadyActive
    );
    for permit in [&old, &foreign] {
        assert_eq!(
            buffered.push(permit, 0, &[f32::NAN]),
            Err(SessionError::StalePermit)
        );
        assert_eq!(
            buffered.report_failure(permit, ProducerFailure::SourceLost),
            Err(SessionError::StalePermit)
        );
        assert_eq!(buffered.pending_batches(), OUTPUT_QUEUE_CAPACITY);
        assert_eq!(buffered.status(), SessionStatus::Active);
    }
    for _ in 0..OUTPUT_QUEUE_CAPACITY {
        buffered.pop().unwrap().unwrap();
    }
    buffered
        .push(
            &current,
            OUTPUT_QUEUE_CAPACITY as u64,
            &vec![0.5; 160 - OUTPUT_QUEUE_CAPACITY],
        )
        .unwrap();
    assert_eq!(
        buffered.pop().unwrap().unwrap().features[0]
            .spectral
            .levels
            .rms,
        0.5
    );
}

#[test]
fn faults_and_stop_discard_backlog_and_do_not_leak_into_restart() {
    for transition in 0..3 {
        let mut buffered = BufferedAcousticSession::default();
        let permit = buffered.start(format(), 0).unwrap();
        buffered.push(&permit, 0, &[0.5; 170]).unwrap();
        match transition {
            0 => {
                buffered
                    .report_failure(&permit, ProducerFailure::SourceLost)
                    .unwrap();
            }
            1 => {
                assert_eq!(
                    buffered.push(&permit, 170, &[f32::NAN]),
                    Err(SessionError::InvalidFrame(FrameError::InvalidSample))
                );
            }
            _ => {
                buffered.stop();
                buffered.stop();
            }
        }
        assert_eq!(buffered.pending_batches(), 0);
        assert!(buffered.pop().unwrap().is_none());
        let status = buffered.status();
        assert_eq!(
            buffered.report_failure(&permit, ProducerFailure::FormatChanged),
            Err(SessionError::Inactive)
        );
        assert_eq!(buffered.status(), status);
        let current = buffered.start(format(), 0).unwrap();
        buffered.push(&current, 0, &[0.0; 160]).unwrap();
        let feature = buffered.pop().unwrap().unwrap().features.remove(0);
        assert_eq!(feature.spectral.levels.rms, 0.0);
        assert!(feature.best_similarity.is_none());
    }
}
