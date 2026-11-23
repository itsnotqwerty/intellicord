use anyhow::{Result, anyhow, ensure};
use intellicord::acoustic::{
    BufferedAcousticSession, Format, OUTPUT_QUEUE_CAPACITY, OutputHealth, PcmEncoding,
    ProducerFailure, SessionError, SessionFault, SessionStatus,
};
use serde::Serialize;

#[derive(Serialize)]
struct Checkpoint {
    step: &'static str,
    health: OutputHealth,
}

#[derive(Serialize)]
struct Report {
    schema_version: u16,
    replay_version: &'static str,
    input: &'static str,
    limitations: &'static str,
    checkpoints: Vec<Checkpoint>,
}

fn checked<T>(result: Result<T, SessionError>) -> Result<T> {
    result.map_err(|error| anyhow!("unexpected session error: {error:?}"))
}

fn checkpoint(
    session: &BufferedAcousticSession,
    step: &'static str,
    checkpoints: &mut Vec<Checkpoint>,
) -> Result<()> {
    let health = session.health();
    ensure!(
        health.pending.batches <= OUTPUT_QUEUE_CAPACITY as u64,
        "queue bound exceeded"
    );
    ensure!(
        health.pending.frames <= health.pending.batches * 4,
        "frame bound exceeded"
    );
    ensure!(
        health.high_water_batches <= OUTPUT_QUEUE_CAPACITY,
        "high water exceeded capacity"
    );
    ensure!(
        health.processed.batches
            == health.delivered.batches + health.discarded.batches + health.pending.batches,
        "batch conservation failed"
    );
    ensure!(
        health.processed.frames
            == health.delivered.frames + health.discarded.frames + health.pending.frames,
        "frame conservation failed"
    );
    checkpoints.push(Checkpoint { step, health });
    Ok(())
}

fn replay() -> Result<Report> {
    let format =
        Format::new(8_000, 1).map_err(|error| anyhow!("invalid replay format: {error:?}"))?;
    let mut session = BufferedAcousticSession::default();
    let mut checkpoints = Vec::new();
    checkpoint(&session, "default_stopped", &mut checkpoints)?;
    let first = checked(session.start_encoded(format, 0, PcmEncoding::Signed16Le))?;
    checkpoint(&session, "first_activation", &mut checkpoints)?;
    let partial: Vec<_> = (0..10).flat_map(|_| 16_384_i16.to_le_bytes()).collect();
    let frame: Vec<_> = (0..160)
        .flat_map(|index| {
            (if index % 2 == 0 {
                16_384_i16
            } else {
                -16_384_i16
            })
            .to_le_bytes()
        })
        .collect();
    checked(session.push_bytes(&first, 0, PcmEncoding::Signed16Le, &partial))?;
    checkpoint(&session, "partial_queued", &mut checkpoints)?;
    let batch = checked(session.pop())?.ok_or_else(|| anyhow!("missing partial batch"))?;
    ensure!(
        batch.features.is_empty() && batch.buffered_samples_per_channel == 10,
        "incorrect partial accounting"
    );
    checkpoint(&session, "partial_delivered", &mut checkpoints)?;
    checked(session.push_bytes(&first, 15, PcmEncoding::Signed16Le, &frame))?;
    checkpoint(&session, "gap_queued", &mut checkpoints)?;
    let batch = checked(session.pop())?.ok_or_else(|| anyhow!("missing recovery batch"))?;
    let gap = batch.gap.ok_or_else(|| anyhow!("missing sample gap"))?;
    ensure!(
        gap.skipped_samples_per_channel == 5 && gap.discarded_partial_samples_per_channel == 10,
        "incorrect gap accounting"
    );
    ensure!(
        batch.features.len() == 1 && batch.features[0].spectral.levels.discontinuity,
        "missing recovery discontinuity"
    );
    checkpoint(&session, "gap_delivered", &mut checkpoints)?;
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        checked(session.push_bytes(
            &first,
            175 + index as u64 * 160,
            PcmEncoding::Signed16Le,
            &frame,
        ))?;
        checkpoint(&session, "consumer_paused", &mut checkpoints)?;
    }
    ensure!(
        session.push_bytes(
            &first,
            175 + OUTPUT_QUEUE_CAPACITY as u64 * 160,
            PcmEncoding::Signed16Le,
            &frame
        ) == Err(SessionError::OutputOverflow),
        "missing overflow fault"
    );
    ensure!(
        session.status()
            == SessionStatus::Faulted {
                error: SessionFault::OutputOverflow
            },
        "incorrect overflow status"
    );
    ensure!(
        session.health().discarded.batches == 9 && session.health().discarded.frames == 9,
        "incorrect overflow disposal"
    );
    checkpoint(&session, "output_overflow", &mut checkpoints)?;
    ensure!(
        checked(session.pop())?.is_none(),
        "overflow backlog survived"
    );
    let second = checked(session.start_encoded(format, 1_000_000_000, PcmEncoding::Float32Le))?;
    checkpoint(&session, "restart_after_overflow", &mut checkpoints)?;
    let before = session.health();
    ensure!(
        session.push_bytes(&first, 0, PcmEncoding::Signed16Le, &[])
            == Err(SessionError::StalePermit),
        "stale callback accepted"
    );
    ensure!(session.health() == before, "stale callback changed health");
    checkpoint(&session, "stale_input_rejected", &mut checkpoints)?;
    let float_frame: Vec<_> = (0..160).flat_map(|_| 0.25_f32.to_le_bytes()).collect();
    checked(session.push_bytes(&second, 0, PcmEncoding::Float32Le, &float_frame))?;
    checkpoint(&session, "producer_output_queued", &mut checkpoints)?;
    checked(session.report_failure(&second, ProducerFailure::SourceLost))?;
    ensure!(
        session.status()
            == SessionStatus::Faulted {
                error: SessionFault::Producer(ProducerFailure::SourceLost)
            },
        "missing producer fault"
    );
    checkpoint(&session, "source_lost", &mut checkpoints)?;
    let third = checked(session.start_encoded(format, 2_000_000_000, PcmEncoding::Signed16Le))?;
    checkpoint(&session, "restart_after_source_loss", &mut checkpoints)?;
    checked(session.push_bytes(&third, 0, PcmEncoding::Signed16Le, &frame))?;
    checkpoint(&session, "format_output_queued", &mut checkpoints)?;
    ensure!(
        session.push_bytes(&third, 160, PcmEncoding::Float32Le, &[])
            == Err(SessionError::FormatChanged),
        "missing format change fault"
    );
    ensure!(
        session.status()
            == SessionStatus::Faulted {
                error: SessionFault::Producer(ProducerFailure::FormatChanged)
            },
        "incorrect format change status"
    );
    checkpoint(&session, "format_changed", &mut checkpoints)?;
    let fourth = checked(session.start_encoded(format, 3_000_000_000, PcmEncoding::Signed16Le))?;
    checkpoint(&session, "final_activation", &mut checkpoints)?;
    checked(session.push_bytes(&fourth, 0, PcmEncoding::Signed16Le, &frame))?;
    checkpoint(&session, "stop_output_queued", &mut checkpoints)?;
    session.stop();
    checkpoint(&session, "explicit_stop", &mut checkpoints)?;
    let stopped = session.health();
    session.stop();
    ensure!(
        session.health() == stopped && checked(session.pop())?.is_none(),
        "stop was not idempotent"
    );
    checkpoint(&session, "repeated_stop", &mut checkpoints)?;
    Ok(Report {
        schema_version: 1,
        replay_version: "acoustic-lifecycle-replay/0.1",
        input: "Fixed generated PCM only; no devices, files or Discord access",
        limitations: "Deterministic synchronous contract replay; not capture, concurrency, performance, detection accuracy or source verification",
        checkpoints,
    })
}

fn main() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "this generated-only replay accepts no arguments"
    );
    let report = replay()?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_replay_completes_with_conserved_output() {
        let report = replay().unwrap();
        let final_health = report.checkpoints.last().unwrap().health;
        assert_eq!(final_health.status, SessionStatus::Stopped);
        assert_eq!(final_health.processed.batches, 14);
        assert_eq!(final_health.processed.frames, 13);
        assert_eq!(final_health.delivered.batches, 2);
        assert_eq!(final_health.delivered.frames, 1);
        assert_eq!(final_health.discarded.batches, 12);
        assert_eq!(final_health.discarded.frames, 12);
        assert_eq!(final_health.overflow_events, 1);
    }

    #[test]
    fn report_is_deterministic_and_contains_only_health_checkpoints() {
        let report = replay().unwrap();
        let serialized = serde_json::to_value(&report).unwrap();
        assert_eq!(serialized, serde_json::to_value(replay().unwrap()).unwrap());
        let fields: Vec<_> = serialized
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            fields,
            [
                "checkpoints",
                "input",
                "limitations",
                "replay_version",
                "schema_version"
            ]
        );
        assert_eq!(report.checkpoints.len(), 26);
        for checkpoint in serialized["checkpoints"].as_array().unwrap() {
            let fields: Vec<_> = checkpoint
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(fields, ["health", "step"]);
            let health_fields: Vec<_> = checkpoint["health"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                health_fields,
                [
                    "capacity_batches",
                    "delivered",
                    "discarded",
                    "high_water_batches",
                    "overflow_events",
                    "pending",
                    "processed",
                    "schema_version",
                    "status"
                ]
            );
        }
    }

    #[test]
    fn fault_sequence_and_restart_checkpoints_remain_explicit() {
        let report = replay().unwrap();
        let faults: Vec<_> = report
            .checkpoints
            .iter()
            .filter_map(|checkpoint| match checkpoint.health.status {
                SessionStatus::Faulted { error } => Some((checkpoint.step, error)),
                _ => None,
            })
            .collect();
        assert_eq!(
            faults,
            [
                ("output_overflow", SessionFault::OutputOverflow),
                (
                    "source_lost",
                    SessionFault::Producer(ProducerFailure::SourceLost)
                ),
                (
                    "format_changed",
                    SessionFault::Producer(ProducerFailure::FormatChanged)
                ),
            ]
        );
        for checkpoint in &report.checkpoints {
            if matches!(
                checkpoint.step,
                "restart_after_overflow" | "restart_after_source_loss" | "final_activation"
            ) {
                assert_eq!(checkpoint.health.status, SessionStatus::Active);
                assert_eq!(checkpoint.health.pending.batches, 0);
                assert_eq!(checkpoint.health.high_water_batches, OUTPUT_QUEUE_CAPACITY);
            }
        }
    }
}
