use serde::{Deserialize, Serialize};

use super::{
    ProcessHealth,
    lifecycle::{Lifecycle, Phase, Reason, Status},
};

pub const RECORD_LIMIT: usize = 2048;
pub const STREAM_LIMIT: usize = 16 * 1024;
pub const RECORD_COUNT_LIMIT: u64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    GeneratedReplay,
    KernelTracepoint,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Self::GeneratedReplay => "generated_replay",
            Self::KernelTracepoint => "kernel_tracepoint",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    Limit,
    InvalidRecord,
    InvalidTransition,
    HealthRegression,
    Incomplete,
    ExitMismatch,
    Rejected,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "process status stream rejected: {self:?}")
    }
}

impl std::error::Error for StreamError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Incoming<'buffer> {
    schema_version: u16,
    record_type: &'buffer str,
    source: &'buffer str,
    sequence: u64,
    phase: Phase,
    #[serde(deserialize_with = "deserialize_optional")]
    reason: Option<Reason>,
    attachment_confirmed: bool,
    detachment_confirmed: bool,
    #[serde(deserialize_with = "deserialize_optional")]
    health: Option<ProcessHealth>,
}

fn deserialize_optional<'input, Deserializer, Value>(
    deserializer: Deserializer,
) -> Result<Option<Value>, Deserializer::Error>
where
    Deserializer: serde::Deserializer<'input>,
    Value: Deserialize<'input>,
{
    Option::deserialize(deserializer)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidatedRun {
    pub schema_version: u16,
    pub source: Source,
    pub records: u64,
    pub terminal_phase: Phase,
    pub reason: Reason,
    pub attachment_confirmed: bool,
    pub detachment_confirmed: bool,
}

pub struct StatusStream {
    source: Source,
    lifecycle: Lifecycle,
    pending: Vec<u8>,
    bytes: usize,
    records: u64,
    last: Option<Status>,
    health: Option<ProcessHealth>,
    rejected: bool,
}

impl StatusStream {
    pub(super) fn is_observing(&self) -> bool {
        !self.rejected
            && self
                .last
                .is_some_and(|status| status.phase == Phase::Observing)
    }

    pub(super) fn observe_health(&mut self, health: ProcessHealth) -> Result<(), StreamError> {
        if self.rejected {
            return Err(StreamError::Rejected);
        }
        let result = if self
            .last
            .is_none_or(|status| status.phase != Phase::Observing)
        {
            Err(StreamError::InvalidTransition)
        } else {
            self.validate_health(health)
        };
        if result.is_err() {
            self.rejected = true;
        } else {
            self.health = Some(health);
        }
        result
    }

    fn validate_health(&self, health: ProcessHealth) -> Result<(), StreamError> {
        if health.schema_version != 1 {
            return Err(StreamError::InvalidRecord);
        }
        if self.health.is_some_and(|previous| {
            health.accepted < previous.accepted
                || health.rejected_record < previous.rejected_record
                || health.rejected_clock < previous.rejected_clock
                || health.ring_dropped < previous.ring_dropped
        }) {
            return Err(StreamError::HealthRegression);
        }
        Ok(())
    }

    pub(super) fn terminal_health(&self) -> Option<ProcessHealth> {
        self.last.as_ref().and_then(|status| status.health)
    }

    pub fn new(source: Source) -> Self {
        Self {
            source,
            lifecycle: Lifecycle::new(source == Source::GeneratedReplay),
            pending: Vec::with_capacity(RECORD_LIMIT),
            bytes: 0,
            records: 0,
            last: None,
            health: None,
            rejected: false,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        if self.rejected {
            return Err(StreamError::Rejected);
        }
        let result = self.push_inner(bytes);
        if result.is_err() {
            self.rejected = true;
        }
        result
    }

    fn push_inner(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        if bytes.len() > STREAM_LIMIT - self.bytes {
            return Err(StreamError::Limit);
        }
        self.bytes += bytes.len();
        for &byte in bytes {
            if byte == b'\n' {
                self.accept_record()?;
                self.pending.clear();
            } else {
                if self.pending.len() >= RECORD_LIMIT - 1 {
                    return Err(StreamError::Limit);
                }
                self.pending.push(byte);
            }
        }
        Ok(())
    }

    fn accept_record(&mut self) -> Result<(), StreamError> {
        if self.records >= RECORD_COUNT_LIMIT {
            return Err(StreamError::Limit);
        }
        let incoming: Incoming<'_> =
            serde_json::from_slice(&self.pending).map_err(|_| StreamError::InvalidRecord)?;
        if incoming.schema_version != 1
            || incoming.record_type != "process_status"
            || incoming.source != self.source.name()
            || incoming.sequence != self.records + 1
        {
            return Err(StreamError::InvalidRecord);
        }
        if let Some(health) = incoming.health {
            self.validate_health(health)?;
        }
        let needs_health = incoming.phase == Phase::Stopped
            || (self.source == Source::KernelTracepoint
                && matches!(incoming.phase, Phase::Observing | Phase::Stopping));
        if needs_health && incoming.health.is_none() {
            return Err(StreamError::InvalidRecord);
        }
        if matches!(
            incoming.phase,
            Phase::Validating | Phase::Loading | Phase::Attaching
        ) && incoming.health.is_some()
        {
            return Err(StreamError::InvalidRecord);
        }
        let mut next = self.lifecycle.clone();
        let expected = next
            .advance(incoming.phase, incoming.reason, incoming.health)
            .map_err(|_| StreamError::InvalidTransition)?;
        if expected.attachment_confirmed != incoming.attachment_confirmed
            || expected.detachment_confirmed != incoming.detachment_confirmed
        {
            return Err(StreamError::InvalidRecord);
        }
        self.lifecycle = next;
        self.records += 1;
        self.last = Some(expected);
        if incoming.health.is_some() {
            self.health = incoming.health;
        }
        Ok(())
    }

    pub fn finish(self, exit_success: bool) -> Result<ValidatedRun, StreamError> {
        if self.rejected {
            return Err(StreamError::Rejected);
        }
        if !self.pending.is_empty() {
            return Err(StreamError::Incomplete);
        }
        let terminal = self
            .last
            .filter(|record| matches!(record.phase, Phase::Stopped | Phase::Faulted))
            .ok_or(StreamError::Incomplete)?;
        if exit_success != (terminal.phase == Phase::Stopped) {
            return Err(StreamError::ExitMismatch);
        }
        Ok(ValidatedRun {
            schema_version: 1,
            source: self.source,
            records: self.records,
            terminal_phase: terminal.phase,
            reason: terminal.reason.ok_or(StreamError::Incomplete)?,
            attachment_confirmed: terminal.attachment_confirmed,
            detachment_confirmed: terminal.detachment_confirmed,
        })
    }
}

fn encoded(records: &[Status]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| {
            let mut line = serde_json::to_vec(record).unwrap();
            line.push(b'\n');
            line
        })
        .collect()
}

#[derive(Serialize)]
pub struct ValidationCase {
    pub name: &'static str,
    pub passed: bool,
}

#[derive(Serialize)]
pub struct ValidationCheck {
    pub schema_version: u16,
    pub source: &'static str,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub cases: Vec<ValidationCase>,
}

pub fn generated_check() -> ValidationCheck {
    let records = super::lifecycle::generated_lifecycle();
    let bytes = encoded(&records);
    let check = |source, bytes: &[u8], success| {
        let mut stream = StatusStream::new(source);
        for chunk in bytes.chunks(7) {
            stream.push(chunk)?;
        }
        stream.finish(success)
    };
    let mut lifecycle = Lifecycle::new(true);
    let faults = [
        lifecycle
            .advance(Phase::Validating, None, None)
            .expect("fixed validation phase"),
        lifecycle
            .advance(Phase::Faulted, Some(Reason::InvalidObject), None)
            .expect("fixed fault phase"),
    ];
    let mut replayed = records.clone();
    replayed[2].sequence = 2;
    let mut forged = records.clone();
    forged[0].attachment_confirmed = true;
    let mut regression = records.clone();
    regression[3].health = Some(ProcessHealth {
        accepted: 3,
        ..super::generated_replay()
    });
    let cases = [
        (
            "normal_completion",
            check(Source::GeneratedReplay, &bytes, true).is_ok(),
        ),
        (
            "fault_completion",
            check(Source::GeneratedReplay, &encoded(&faults), false).is_ok(),
        ),
        (
            "truncated_record",
            check(Source::GeneratedReplay, &bytes[..bytes.len() - 1], true)
                == Err(StreamError::Incomplete),
        ),
        (
            "sequence_replay",
            check(Source::GeneratedReplay, &encoded(&replayed), true)
                == Err(StreamError::InvalidRecord),
        ),
        (
            "source_mismatch",
            check(Source::KernelTracepoint, &bytes, true) == Err(StreamError::InvalidRecord),
        ),
        (
            "confirmation_forgery",
            check(Source::GeneratedReplay, &encoded(&forged), true)
                == Err(StreamError::InvalidRecord),
        ),
        (
            "health_regression",
            check(Source::GeneratedReplay, &encoded(&regression), true)
                == Err(StreamError::HealthRegression),
        ),
        (
            "exit_mismatch",
            check(Source::GeneratedReplay, &bytes, false) == Err(StreamError::ExitMismatch),
        ),
    ]
    .into_iter()
    .map(|(name, passed)| ValidationCase { name, passed })
    .collect();
    ValidationCheck {
        schema_version: 1,
        source: "generated_replay",
        sensor_running: false,
        load_test_performed: false,
        cases,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_suite_exercises_all_expected_rejections() {
        let check = generated_check();
        assert_eq!(check.cases.len(), 8);
        for case in check.cases {
            assert!(case.passed, "{}", case.name);
        }
        assert!(serde_json::to_vec(&generated_check()).unwrap().len() < 2048);
    }

    #[test]
    fn strict_schema_and_budgets_reject_untrusted_input() {
        let records = super::super::lifecycle::generated_lifecycle();
        let original = serde_json::to_value(records[0]).unwrap();
        let mut candidates = Vec::new();
        for key in ["reason", "health", "schema_version", "source", "sequence"] {
            let mut candidate = original.clone();
            candidate.as_object_mut().unwrap().remove(key);
            candidates.push(candidate);
        }
        let mut extra = original.clone();
        extra["path"] = serde_json::json!("not permitted");
        candidates.push(extra);
        let mut health = original;
        health["health"] = serde_json::json!({"schema_version":1,"accepted":0,"rejected_record":0,"rejected_clock":0,"ring_dropped":0,"extra":0});
        candidates.push(health);
        for candidate in candidates {
            let mut line = serde_json::to_vec(&candidate).unwrap();
            line.push(b'\n');
            assert!(
                StatusStream::new(Source::GeneratedReplay)
                    .push(&line)
                    .is_err()
            );
        }
        for bytes in [vec![0xff, b'\n'], vec![b'\n'], vec![b' '; STREAM_LIMIT + 1]] {
            assert!(
                StatusStream::new(Source::GeneratedReplay)
                    .push(&bytes)
                    .is_err()
            );
        }
        let bytes = encoded(&records);
        let mut stream = StatusStream::new(Source::GeneratedReplay);
        stream.push(&bytes).unwrap();
        assert!(stream.push(&encoded(&records[..1])).is_err());
        assert_eq!(stream.finish(true), Err(StreamError::Rejected));
        assert_eq!(
            StatusStream::new(Source::GeneratedReplay).finish(false),
            Err(StreamError::Incomplete)
        );
    }

    #[test]
    fn kernel_adapter_records_are_validated_without_authenticating_their_claims() {
        let mut lifecycle = Lifecycle::new(false);
        let health = Some(super::super::generated_replay());
        let records: Vec<_> = [
            Phase::Validating,
            Phase::Loading,
            Phase::Attaching,
            Phase::Observing,
            Phase::Stopping,
            Phase::Stopped,
        ]
        .into_iter()
        .map(|phase| {
            lifecycle
                .advance(
                    phase,
                    (phase == Phase::Stopped).then_some(Reason::DurationLimit),
                    matches!(phase, Phase::Observing | Phase::Stopping | Phase::Stopped)
                        .then_some(health)
                        .flatten(),
                )
                .unwrap()
        })
        .collect();
        let mut stream = StatusStream::new(Source::KernelTracepoint);
        stream.push(&encoded(&records)).unwrap();
        let report = stream.finish(true).unwrap();
        assert!(report.attachment_confirmed);
        assert!(report.detachment_confirmed);
        let mut missing_health = records;
        missing_health[3].health = None;
        assert!(
            StatusStream::new(Source::KernelTracepoint)
                .push(&encoded(&missing_health))
                .is_err()
        );
    }

    #[test]
    fn terminal_reasons_fail_during_ingestion_and_fault_health_is_retained_legally() {
        let records = super::super::lifecycle::generated_lifecycle();
        for reason in [None, Some(Reason::DetachFailed)] {
            let mut invalid = records.clone();
            invalid[5].reason = reason;
            assert_eq!(
                StatusStream::new(Source::GeneratedReplay).push(&encoded(&invalid)),
                Err(StreamError::InvalidTransition)
            );
        }
        let mut lifecycle = Lifecycle::new(true);
        let fault = [
            lifecycle.advance(Phase::Validating, None, None).unwrap(),
            lifecycle
                .advance(
                    Phase::Faulted,
                    Some(Reason::InternalFailure),
                    Some(super::super::generated_replay()),
                )
                .unwrap(),
        ];
        let mut stream = StatusStream::new(Source::GeneratedReplay);
        stream.push(&encoded(&fault)).unwrap();
        assert_eq!(stream.finish(false).unwrap().terminal_phase, Phase::Faulted);
    }

    #[test]
    fn fragmented_generated_stream_requires_complete_terminal_and_exit_agreement() {
        let records = super::super::lifecycle::generated_lifecycle();
        let bytes = encoded(&records);
        for chunk_size in [1, 7, bytes.len()] {
            let mut stream = StatusStream::new(Source::GeneratedReplay);
            for chunk in bytes.chunks(chunk_size) {
                stream.push(chunk).unwrap();
            }
            let report = stream.finish(true).unwrap();
            assert_eq!(report.records, 6);
            assert_eq!(report.terminal_phase, Phase::Stopped);
            assert!(!report.attachment_confirmed);
        }
        let mut truncated = StatusStream::new(Source::GeneratedReplay);
        truncated.push(&bytes[..bytes.len() - 1]).unwrap();
        assert_eq!(truncated.finish(true), Err(StreamError::Incomplete));
        let mut mismatch = StatusStream::new(Source::GeneratedReplay);
        mismatch.push(&bytes).unwrap();
        assert_eq!(mismatch.finish(false), Err(StreamError::ExitMismatch));
    }

    #[test]
    fn rejection_is_sticky_and_cannot_be_repaired_by_later_valid_input() {
        let bytes = encoded(&super::super::lifecycle::generated_lifecycle());
        let mut stream = StatusStream::new(Source::KernelTracepoint);
        assert_eq!(stream.push(&bytes), Err(StreamError::InvalidRecord));
        assert_eq!(stream.push(&[]), Err(StreamError::Rejected));
        assert_eq!(stream.finish(true), Err(StreamError::Rejected));
        let mut oversized = StatusStream::new(Source::GeneratedReplay);
        assert_eq!(
            oversized.push(&vec![b' '; RECORD_LIMIT]),
            Err(StreamError::Limit)
        );
        assert!(oversized.pending.len() < RECORD_LIMIT);
    }
}
