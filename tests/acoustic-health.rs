use intellicord::acoustic::{
    BufferedAcousticSession, Format, OutputCounts, OutputHealth, ProducerFailure, SessionError,
};

fn assert_conserved(health: OutputHealth) {
    assert_eq!(
        health.processed.batches,
        health.delivered.batches + health.discarded.batches + health.pending.batches
    );
    assert_eq!(
        health.processed.frames,
        health.delivered.frames + health.discarded.frames + health.pending.frames
    );
}

#[test]
fn lifecycle_discards_are_counted_once_and_survive_restart() {
    for transition in 0..3 {
        let mut session = BufferedAcousticSession::default();
        let format = Format::new(8_000, 1).unwrap();
        let permit = session.start(format, 0).unwrap();
        session.push(&permit, 0, &[0.0; 640]).unwrap();
        session.push(&permit, 640, &[0.0; 10]).unwrap();
        assert_conserved(session.health());
        assert_eq!(
            session.health().pending,
            OutputCounts {
                batches: 2,
                frames: 4
            }
        );
        match transition {
            0 => session.stop(),
            1 => session
                .report_failure(&permit, ProducerFailure::InputOverflow)
                .unwrap(),
            _ => {
                assert!(session.push(&permit, 650, &[f32::NAN]).is_err());
            }
        }
        let health = session.health();
        assert_eq!(
            health.discarded,
            OutputCounts {
                batches: 2,
                frames: 4
            }
        );
        assert_eq!(health.overflow_events, 0);
        assert_conserved(health);
        assert!(session.pop().unwrap().is_none());
        assert_eq!(
            session.push(&permit, 650, &[0.0]),
            Err(SessionError::Inactive)
        );
        assert_eq!(
            session.report_failure(&permit, ProducerFailure::SourceLost),
            Err(SessionError::Inactive)
        );
        assert_eq!(session.health(), health);
        session.stop();
        session.stop();
        let current = session.start(format, 100).unwrap();
        session.push(&current, 0, &[0.0; 160]).unwrap();
        session.pop().unwrap().unwrap();
        let restarted = session.health();
        assert_eq!(restarted.discarded, health.discarded);
        assert_eq!(
            restarted.delivered,
            OutputCounts {
                batches: 1,
                frames: 1
            }
        );
        assert_conserved(restarted);
    }
}

#[test]
fn rejected_callbacks_and_reads_leave_health_unchanged() {
    let mut session = BufferedAcousticSession::default();
    let format = Format::new(8_000, 1).unwrap();
    let old = session.start(format, 0).unwrap();
    session.stop();
    let current = session.start(format, 0).unwrap();
    session.push(&current, 0, &[0.0; 160]).unwrap();
    let before = session.health();
    assert_eq!(
        session.start(format, 1).unwrap_err(),
        SessionError::AlreadyActive
    );
    assert_eq!(
        session.push(&old, 0, &[f32::NAN]),
        Err(SessionError::StalePermit)
    );
    assert_eq!(
        session.report_failure(&old, ProducerFailure::SourceLost),
        Err(SessionError::StalePermit)
    );
    assert_eq!(session.health(), before);
    assert_conserved(before);
}

#[test]
fn health_serialization_has_only_bounded_status_and_accounting_fields() {
    let session = BufferedAcousticSession::default();
    assert_eq!(
        serde_json::to_value(session.health()).unwrap(),
        serde_json::json!({
            "schema_version": 1,
            "status": { "state": "stopped" },
            "capacity_batches": 8,
            "pending": { "batches": 0, "frames": 0 },
            "processed": { "batches": 0, "frames": 0 },
            "delivered": { "batches": 0, "frames": 0 },
            "discarded": { "batches": 0, "frames": 0 },
            "overflow_events": 0,
            "high_water_batches": 0
        })
    );
}

#[test]
fn health_accounts_for_empty_batches_delivery_and_overflow() {
    let mut session = BufferedAcousticSession::default();
    let initial = session.health();
    assert_eq!(initial.schema_version, 1);
    assert_eq!(initial.processed.batches, 0);
    let permit = session.start(Format::new(8_000, 1).unwrap(), 0).unwrap();
    session.push(&permit, 0, &[0.0]).unwrap();
    assert_eq!(session.health().pending.frames, 0);
    session.pop().unwrap().unwrap();
    for index in 0..8 {
        session.push(&permit, 1 + index * 160, &[0.0; 160]).unwrap();
    }
    assert_eq!(
        session.push(&permit, 1281, &[0.0; 160]),
        Err(SessionError::OutputOverflow)
    );
    let health = session.health();
    assert_eq!(health.processed.batches, 10);
    assert_eq!(health.processed.frames, 9);
    assert_eq!(health.delivered.batches, 1);
    assert_eq!(health.delivered.frames, 0);
    assert_eq!(health.discarded.batches, 9);
    assert_eq!(health.discarded.frames, 9);
    assert_eq!(health.pending.batches, 0);
    assert_eq!(health.overflow_events, 1);
    assert_eq!(health.high_water_batches, health.capacity_batches);
    assert_conserved(health);
}
