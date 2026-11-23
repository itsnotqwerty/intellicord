use anyhow::{Result, anyhow, ensure};
use intellicord::acoustic::{
    BufferedTimedAcousticSession, ClockError, Format, OUTPUT_QUEUE_CAPACITY, OutputHealth,
    PcmEncoding, PcmPlane, ProducerFailure, SampleClockMapping, SessionError, SessionFault,
    SessionStatus, SourceClockMapping,
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

fn checked<Output, Error: std::fmt::Debug>(result: Result<Output, Error>) -> Result<Output> {
    result.map_err(|error| anyhow!("unexpected replay error: {error:?}"))
}

fn clock(base: u64, horizon: u64) -> Result<SampleClockMapping> {
    checked(SampleClockMapping::new(
        checked(SourceClockMapping::new(
            0,
            base + 100,
            base + 120,
            horizon,
            100,
        ))?,
        checked(Format::new(8_050, 1))?,
        0,
    ))
}

fn checkpoint(
    session: &BufferedTimedAcousticSession,
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
    let mut session = BufferedTimedAcousticSession::default();
    let mut checkpoints = Vec::new();
    checkpoint(&session, "default_stopped", &mut checkpoints)?;
    let first = checked(session.start_encoded(clock(0, 40_000_000)?, PcmEncoding::Signed16Le))?;
    checkpoint(&session, "first_activation", &mut checkpoints)?;
    let partial: Vec<_> = (0..10).flat_map(|_| 16_384_i16.to_le_bytes()).collect();
    let frame: Vec<_> = (0..161)
        .flat_map(|index| {
            (if index % 8 < 4 {
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
        "incorrect partial output"
    );
    checkpoint(&session, "partial_delivered", &mut checkpoints)?;
    checked(session.push_bytes(&first, 15, PcmEncoding::Signed16Le, &frame))?;
    checkpoint(&session, "gap_queued", &mut checkpoints)?;
    let batch = checked(session.pop())?.ok_or_else(|| anyhow!("missing gap batch"))?;
    let gap = batch.gap.ok_or_else(|| anyhow!("missing sample gap"))?;
    ensure!(
        gap.skipped_samples_per_channel == 5 && gap.discarded_partial_samples_per_channel == 10,
        "incorrect gap accounting"
    );
    ensure!(batch.features.len() == 1, "incorrect gap frame count");
    let feature = &batch.features[0];
    let oracle = clock(0, 40_000_000)?;
    ensure!(
        feature.start_sample_per_channel == 15 && feature.end_sample_per_channel == 176,
        "incorrect sample interval"
    );
    ensure!(
        feature.start_time == checked(oracle.map(15))?
            && feature.end_time == checked(oracle.map(176))?,
        "clock intervals lost in delivery"
    );
    ensure!(
        feature.start_time.latest_ns > feature.start_time.earliest_ns,
        "clock uncertainty lost"
    );
    ensure!(
        feature
            .nominal_sample_clock_features
            .spectral
            .levels
            .frame_start_ns
            == 1_863_354,
        "nominal timestamp changed domains"
    );
    ensure!(
        feature
            .nominal_sample_clock_features
            .spectral
            .levels
            .discontinuity
            && feature
                .nominal_sample_clock_features
                .best_similarity
                .is_none(),
        "gap comparisons not reset"
    );
    checkpoint(&session, "gap_delivered", &mut checkpoints)?;
    checked(session.push(&first, 176, &[0.5]))?;
    checkpoint(&session, "pre_expiry_queued", &mut checkpoints)?;
    ensure!(
        session.push_bytes(&first, 177, PcmEncoding::Signed16Le, &frame)
            == Err(SessionError::InvalidClock(ClockError::Expired)),
        "missing clock expiry fault"
    );
    ensure!(
        session.status()
            == SessionStatus::Faulted {
                error: SessionFault::Clock(ClockError::Expired)
            },
        "incorrect clock status"
    );
    ensure!(checked(session.pop())?.is_none(), "expiry backlog survived");
    checkpoint(&session, "clock_expired", &mut checkpoints)?;

    let second = checked(session.start_encoded(
        clock(1_000_000_000, 1_000_000_000)?,
        PcmEncoding::Signed16Le,
    ))?;
    checkpoint(&session, "restart_after_expiry", &mut checkpoints)?;
    let before = session.health();
    ensure!(
        session.push_planes(&first, u64::MAX, PcmEncoding::Float32Le, usize::MAX, &[])
            == Err(SessionError::StalePermit),
        "stale input accepted"
    );
    ensure!(
        session.report_failure(&first, ProducerFailure::SourceLost)
            == Err(SessionError::StalePermit),
        "stale failure accepted"
    );
    ensure!(session.health() == before, "stale callback changed health");
    checkpoint(&session, "stale_callbacks_rejected", &mut checkpoints)?;
    let maximum = frame.repeat(4);
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        checked(session.push_bytes(
            &second,
            index as u64 * 644,
            PcmEncoding::Signed16Le,
            &maximum,
        ))?;
        checkpoint(&session, "consumer_paused", &mut checkpoints)?;
    }
    ensure!(
        session.push_bytes(&second, 5152, PcmEncoding::Signed16Le, &maximum)
            == Err(SessionError::OutputOverflow),
        "missing output overflow"
    );
    ensure!(
        session.status()
            == SessionStatus::Faulted {
                error: SessionFault::OutputOverflow
            },
        "incorrect overflow status"
    );
    ensure!(
        session.health().discarded.batches == 10 && session.health().discarded.frames == 36,
        "incorrect overflow disposition"
    );
    checkpoint(&session, "output_overflow", &mut checkpoints)?;

    let third = checked(session.start_encoded(
        clock(2_000_000_000, 1_000_000_000)?,
        PcmEncoding::Signed16Le,
    ))?;
    checkpoint(&session, "restart_after_overflow", &mut checkpoints)?;
    let padded: Vec<_> = (0..161).flat_map(|_| [0xff, 0xff, 0, 64]).collect();
    let planes = [PcmPlane {
        bytes: &padded,
        offset_bytes: 2,
        stride_bytes: 4,
    }];
    checked(session.push_planes(&third, 0, PcmEncoding::Signed16Le, 161, &planes))?;
    checkpoint(&session, "producer_output_queued", &mut checkpoints)?;
    checked(session.report_failure(&third, ProducerFailure::SourceLost))?;
    ensure!(
        session.status()
            == SessionStatus::Faulted {
                error: SessionFault::Producer(ProducerFailure::SourceLost)
            },
        "missing source loss"
    );
    checkpoint(&session, "source_lost", &mut checkpoints)?;

    let fourth = checked(session.start_encoded(
        clock(3_000_000_000, 1_000_000_000)?,
        PcmEncoding::Signed16Le,
    ))?;
    checkpoint(&session, "restart_after_source_loss", &mut checkpoints)?;
    checked(session.push_planes(&fourth, 0, PcmEncoding::Signed16Le, 161, &planes))?;
    checkpoint(&session, "encoding_output_queued", &mut checkpoints)?;
    ensure!(
        session.push_planes(&fourth, 161, PcmEncoding::Float32Le, usize::MAX, &[])
            == Err(SessionError::FormatChanged),
        "missing encoding fault precedence"
    );
    ensure!(
        session.status()
            == SessionStatus::Faulted {
                error: SessionFault::Producer(ProducerFailure::FormatChanged)
            },
        "incorrect encoding status"
    );
    checkpoint(&session, "encoding_changed", &mut checkpoints)?;

    let fifth = checked(session.start(clock(4_000_000_000, 1_000_000_000)?))?;
    checkpoint(&session, "final_activation", &mut checkpoints)?;
    checked(session.push(&fifth, 0, &[0.0; 161]))?;
    checkpoint(&session, "fresh_output_queued", &mut checkpoints)?;
    let batch = checked(session.pop())?.ok_or_else(|| anyhow!("missing fresh output"))?;
    ensure!(
        batch.features.len() == 1 && batch.features[0].start_sample_per_channel == 0,
        "stale output survived restart"
    );
    let feature = &batch.features[0];
    ensure!(
        feature.start_time.earliest_ns == 4_000_000_100
            && feature.start_time.latest_ns == 4_000_000_120,
        "old mapping survived restart"
    );
    ensure!(
        feature
            .nominal_sample_clock_features
            .spectral
            .levels
            .frame_start_ns
            == 0
            && feature
                .nominal_sample_clock_features
                .best_similarity
                .is_none(),
        "old analysis survived restart"
    );
    checkpoint(&session, "fresh_output_delivered", &mut checkpoints)?;
    checked(session.push_bytes(&fifth, 161, PcmEncoding::Signed16Le, &partial))?;
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
        replay_version: "timed-acoustic-lifecycle-replay/0.1",
        input: "Fixed generated 8050 Hz mono PCM and asserted clock mappings only; no devices, input files or Discord access",
        limitations: "Deterministic synchronous contract replay; not clock calibration, capture, concurrency, final-delivery synchronization, performance or detection accuracy",
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
    fn generated_replay_conserves_output_through_clock_and_lifecycle_faults() {
        let report = replay().unwrap();
        let health = report.checkpoints.last().unwrap().health;
        assert_eq!(health.status, SessionStatus::Stopped);
        assert_eq!(health.pending.batches, 0);
        assert_eq!(health.processed.batches, 16);
        assert_eq!(health.processed.frames, 40);
        assert_eq!(health.delivered.batches, 3);
        assert_eq!(health.delivered.frames, 2);
        assert_eq!(health.discarded.batches, 13);
        assert_eq!(health.discarded.frames, 38);
        assert_eq!(health.overflow_events, 1);
        assert_eq!(health.high_water_batches, 8);
    }

    #[test]
    fn report_is_deterministic_bounded_and_health_only() {
        let report = replay().unwrap();
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value, serde_json::to_value(replay().unwrap()).unwrap());
        let fields: Vec<_> = value
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
        assert_eq!(report.checkpoints.len(), 31);
        for checkpoint in value["checkpoints"].as_array().unwrap() {
            let fields: Vec<_> = checkpoint
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(fields, ["health", "step"]);
            let fields: Vec<_> = checkpoint["health"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                fields,
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
    fn fault_sequence_is_explicit_and_restart_queues_are_empty() {
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
                ("clock_expired", SessionFault::Clock(ClockError::Expired)),
                ("output_overflow", SessionFault::OutputOverflow),
                (
                    "source_lost",
                    SessionFault::Producer(ProducerFailure::SourceLost)
                ),
                (
                    "encoding_changed",
                    SessionFault::Producer(ProducerFailure::FormatChanged)
                ),
            ]
        );
        for checkpoint in report.checkpoints.iter().filter(|checkpoint| {
            checkpoint.step.starts_with("restart_") || checkpoint.step == "final_activation"
        }) {
            assert_eq!(checkpoint.health.status, SessionStatus::Active);
            assert_eq!(checkpoint.health.pending.batches, 0);
        }
    }
}
