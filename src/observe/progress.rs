use serde::{Deserialize, Serialize};

#[cfg(all(target_os = "linux", feature = "observe"))]
pub mod capture;
pub mod client;
#[cfg(all(target_os = "linux", feature = "observe"))]
pub mod endpoint;
pub mod evaluation;
#[cfg(all(target_os = "linux", feature = "observe"))]
pub mod peer;
pub mod producer;
pub mod reporter;
#[cfg(all(target_os = "linux", feature = "observe"))]
pub mod socket;
#[cfg(all(target_os = "linux", feature = "observe"))]
pub mod socket_owner;
pub mod supervisor;
pub mod writer;

use super::{
    ProcessHealth,
    stream::{Source, StatusStream, StreamError, ValidatedRun},
};

const RECORD_LIMIT: usize = 2048;
const BYTE_LIMIT: usize = 64 * 1024;
const COUNT_LIMIT: u64 = 128;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema_version: u16,
    session: String,
    sequence: u64,
    payload: Payload,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Payload {
    Lifecycle {
        status: Box<serde_json::value::RawValue>,
    },
    Health {
        health: ProcessHealth,
    },
}

pub struct ProgressStream {
    session: String,
    lifecycle: StatusStream,
    pending: Vec<u8>,
    bytes: usize,
    records: u64,
    samples: u64,
    last_receipt: Option<u64>,
    last_sample: Option<u64>,
    last_health: Option<ProcessHealth>,
    rejected: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub schema_version: u16,
    pub records: u64,
    pub health_samples: u64,
    pub lifecycle: ValidatedRun,
    pub terminal_health: Option<ProcessHealth>,
}

impl ProgressStream {
    fn observing_health(&self) -> Option<(ProcessHealth, u64)> {
        if self.rejected || !self.lifecycle.is_observing() {
            return None;
        }
        self.last_health.zip(self.last_sample)
    }

    pub(super) fn records(&self) -> u64 {
        self.records
    }

    pub fn new(session: &str, source: Source) -> Result<Self, StreamError> {
        if session.len() != 32 || !session.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(StreamError::InvalidRecord);
        }
        Ok(Self {
            session: session.into(),
            lifecycle: StatusStream::new(source),
            pending: Vec::with_capacity(RECORD_LIMIT),
            bytes: 0,
            records: 0,
            samples: 0,
            last_receipt: None,
            last_sample: None,
            last_health: None,
            rejected: false,
        })
    }

    pub fn push(&mut self, bytes: &[u8], receipt_ns: u64) -> Result<(), StreamError> {
        if self.rejected {
            return Err(StreamError::Rejected);
        }
        let result = self.push_inner(bytes, receipt_ns);
        if result.is_err() {
            self.rejected = true;
        }
        result
    }

    fn push_inner(&mut self, bytes: &[u8], receipt_ns: u64) -> Result<(), StreamError> {
        if self
            .last_receipt
            .is_some_and(|previous| receipt_ns < previous)
        {
            return Err(StreamError::InvalidRecord);
        }
        if bytes.len() > BYTE_LIMIT - self.bytes {
            return Err(StreamError::Limit);
        }
        self.bytes += bytes.len();
        self.last_receipt = Some(receipt_ns);
        for &byte in bytes {
            if byte == b'\n' {
                self.accept(receipt_ns)?;
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

    fn accept(&mut self, receipt_ns: u64) -> Result<(), StreamError> {
        if self.records >= COUNT_LIMIT {
            return Err(StreamError::Limit);
        }
        let envelope: Envelope =
            serde_json::from_slice(&self.pending).map_err(|_| StreamError::InvalidRecord)?;
        if envelope.schema_version != 1
            || envelope.session != self.session
            || envelope.sequence != self.records + 1
        {
            return Err(StreamError::InvalidRecord);
        }
        match envelope.payload {
            Payload::Lifecycle { status } => {
                let mut bytes = status.get().as_bytes().to_vec();
                bytes.push(b'\n');
                self.lifecycle.push(&bytes)?;
            }
            Payload::Health { health } => {
                self.lifecycle.observe_health(health)?;
                self.samples += 1;
                self.last_sample = Some(receipt_ns);
                self.last_health = Some(health);
            }
        }
        self.records += 1;
        Ok(())
    }

    pub fn sample_fresh(&self, now_ns: u64, max_age_ns: u64) -> bool {
        !self.rejected
            && self.last_receipt.is_none_or(|receipt| now_ns >= receipt)
            && self.last_sample.is_some_and(|receipt| {
                now_ns
                    .checked_sub(receipt)
                    .is_some_and(|age| age <= max_age_ns)
            })
    }

    pub fn latest_health(&self, now_ns: u64, max_age_ns: u64) -> Option<ProcessHealth> {
        self.sample_fresh(now_ns, max_age_ns)
            .then_some(self.last_health)
            .flatten()
    }

    pub fn finish(self, exit_success: bool) -> Result<Summary, StreamError> {
        if self.rejected {
            return Err(StreamError::Rejected);
        }
        if !self.pending.is_empty() {
            return Err(StreamError::Incomplete);
        }
        let terminal_health = self.lifecycle.terminal_health();
        Ok(Summary {
            schema_version: 1,
            records: self.records,
            health_samples: self.samples,
            lifecycle: self.lifecycle.finish(exit_success)?,
            terminal_health,
        })
    }
}

pub fn generated_stream() -> Vec<u8> {
    let mut encoder =
        producer::Encoder::new("00000000000000000000000000000001", Source::GeneratedReplay)
            .unwrap();
    let mut bytes = Vec::new();
    for status in super::lifecycle::generated_lifecycle() {
        bytes.extend(encoder.lifecycle(status).unwrap());
        if status.phase == super::lifecycle::Phase::Observing {
            for accepted in [1, 2] {
                bytes.extend(
                    encoder
                        .health(ProcessHealth {
                            schema_version: 1,
                            accepted,
                            ..ProcessHealth::default()
                        })
                        .unwrap(),
                );
            }
        }
    }
    encoder.finish(true).unwrap();
    bytes
}

#[derive(Serialize)]
pub struct Replay {
    pub schema_version: u16,
    pub source: &'static str,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub checks: Vec<Check>,
}

#[derive(Serialize)]
pub struct Check {
    pub name: &'static str,
    pub passed: bool,
}

pub fn generated_replay() -> Replay {
    let session = "00000000000000000000000000000001";
    let bytes = generated_stream();
    let receiver = || ProgressStream::new(session, Source::GeneratedReplay).unwrap();
    let mut fragmented = receiver();
    let fragmented_delivery = bytes
        .chunks(7)
        .all(|chunk| fragmented.push(chunk, 100).is_ok())
        && fragmented
            .finish(true)
            .is_ok_and(|summary| summary.records == 8 && summary.health_samples == 2);
    let records: Vec<serde_json::Value> = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    let rejects_change = |index: usize, pointer: &str, value: serde_json::Value| {
        let mut records = records.clone();
        *records[index].pointer_mut(pointer).unwrap() = value;
        let mut stream = receiver();
        records.into_iter().any(|record| {
            let mut line = serde_json::to_vec(&record).unwrap();
            line.push(b'\n');
            stream.push(&line, 100).is_err()
        }) && stream.finish(true).is_err()
    };
    let mut before = records[4].clone();
    before["sequence"] = 1.into();
    let mut line = serde_json::to_vec(&before).unwrap();
    line.push(b'\n');
    let before_observation = receiver().push(&line, 100).is_err();
    let mut fresh = receiver();
    fresh.push(&bytes, 100).unwrap();
    let freshness_bounds =
        fresh.sample_fresh(110, 10) && !fresh.sample_fresh(111, 10) && !fresh.sample_fresh(99, 10);
    let mut truncated = receiver();
    truncated.push(&bytes[..bytes.len() - 1], 100).unwrap();
    let mut mismatch = receiver();
    mismatch.push(&bytes, 100).unwrap();
    let checks = [
        ("fragmented_delivery", fragmented_delivery),
        (
            "foreign_session",
            rejects_change(
                4,
                "/session",
                serde_json::json!("00000000000000000000000000000002"),
            ),
        ),
        (
            "replayed_sequence",
            rejects_change(4, "/sequence", serde_json::json!(4)),
        ),
        (
            "counter_regression",
            rejects_change(5, "/payload/health/health/accepted", serde_json::json!(0)),
        ),
        ("before_observation", before_observation),
        ("freshness_bounds", freshness_bounds),
        ("truncated_record", truncated.finish(true).is_err()),
        ("exit_mismatch", mismatch.finish(false).is_err()),
    ]
    .into_iter()
    .map(|(name, passed)| Check { name, passed })
    .collect();
    Replay {
        schema_version: 1,
        source: "generated_replay",
        sensor_running: false,
        load_test_performed: false,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SESSION: &str = "00000000000000000000000000000001";

    #[test]
    fn fragmented_progress_requires_terminal_and_consistent_exit() {
        let bytes = generated_stream();
        for width in [1, 7, 1024, bytes.len()] {
            let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
            for fragment in bytes.chunks(width) {
                stream.push(fragment, 100).unwrap();
            }
            assert!(stream.sample_fresh(110, 10));
            assert!(!stream.sample_fresh(111, 10));
            assert!(!stream.sample_fresh(99, 10));
            assert_eq!(stream.latest_health(110, 10).unwrap().accepted, 2);
            assert!(stream.latest_health(111, 10).is_none());
            stream.push(b"", 120).unwrap();
            assert!(!stream.sample_fresh(110, 100));
            let summary = stream.finish(true).unwrap();
            assert_eq!(summary.records, 8);
            assert_eq!(summary.health_samples, 2);
            assert_eq!(summary.lifecycle.records, 6);
            assert_eq!(summary.terminal_health.unwrap().accepted, 2);
            assert!(!summary.lifecycle.attachment_confirmed);
        }
        let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        stream.push(&bytes, 100).unwrap();
        assert!(stream.finish(false).is_err());
        let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        stream.push(&bytes[..bytes.len() - 1], 100).unwrap();
        assert!(stream.finish(true).is_err());
    }

    #[test]
    fn foreign_replayed_out_of_phase_and_regressing_samples_poison_stream() {
        let records: Vec<serde_json::Value> = generated_stream()
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        for (index, pointer, replacement) in [
            (
                4,
                "/session",
                serde_json::json!("00000000000000000000000000000002"),
            ),
            (4, "/sequence", serde_json::json!(4)),
            (5, "/payload/health/health/accepted", serde_json::json!(0)),
            (
                7,
                "/payload/lifecycle/status/health/accepted",
                serde_json::json!(0),
            ),
            (
                4,
                "/payload/health/health/schema_version",
                serde_json::json!(2),
            ),
        ] {
            let mut invalid = records.clone();
            *invalid[index].pointer_mut(pointer).unwrap() = replacement;
            let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
            let mut rejected = false;
            for record in invalid {
                let mut line = serde_json::to_vec(&record).unwrap();
                line.push(b'\n');
                if stream.push(&line, 100).is_err() {
                    rejected = true;
                    break;
                }
            }
            assert!(rejected, "{pointer}");
            assert!(!stream.sample_fresh(100, 10));
            assert!(stream.push(b"", 100).is_err());
            assert!(stream.finish(true).is_err());
        }
        let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        let mut health = records[4].clone();
        health["sequence"] = 1.into();
        let mut line = serde_json::to_vec(&health).unwrap();
        line.push(b'\n');
        assert!(stream.push(&line, 100).is_err());
        let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        stream.push(b"", 100).unwrap();
        assert!(stream.push(b"", 99).is_err());
        assert!(ProgressStream::new("bad", Source::GeneratedReplay).is_err());
    }

    #[test]
    fn strict_fields_duplicates_and_all_budgets_are_enforced() {
        let bytes = generated_stream();
        let first = bytes.split_inclusive(|byte| *byte == b'\n').next().unwrap();
        let line = std::str::from_utf8(first).unwrap();
        for invalid in [
            line.replacen(
                "\"schema_version\":1",
                "\"schema_version\":1,\"schema_version\":1",
                1,
            ),
            line.replace(
                "\"record_type\":",
                "\"phase\":\"validating\",\"record_type\":",
            ),
            line.replacen("\"sequence\":1", "\"extra\":true,\"sequence\":1", 1),
            line.replace(
                "\"record_type\":",
                "\"private_path\":\"hidden\",\"record_type\":",
            ),
        ] {
            let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
            assert!(stream.push(invalid.as_bytes(), 100).is_err());
            assert!(stream.finish(true).is_err());
        }
        for bytes in [vec![b' '; RECORD_LIMIT], vec![b' '; BYTE_LIMIT + 1]] {
            let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
            assert_eq!(stream.push(&bytes, 100), Err(StreamError::Limit));
        }
        let records: Vec<_> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
        let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        for record in &records[..4] {
            stream.push(record, 100).unwrap();
        }
        let mut sample: serde_json::Value = serde_json::from_slice(records[4]).unwrap();
        for sequence in 5..=COUNT_LIMIT + 1 {
            sample["sequence"] = sequence.into();
            let mut line = serde_json::to_vec(&sample).unwrap();
            line.push(b'\n');
            let result = stream.push(&line, 100);
            if sequence <= COUNT_LIMIT {
                result.unwrap();
            } else {
                assert_eq!(result, Err(StreamError::Limit));
            }
        }
        assert!(stream.finish(true).is_err());
        let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        for record in &records[..4] {
            stream.push(record, 100).unwrap();
        }
        let mut sample: serde_json::Value = serde_json::from_slice(records[4]).unwrap();
        let mut sequence = 5;
        loop {
            sample["sequence"] = sequence.into();
            let mut line = serde_json::to_vec(&sample).unwrap();
            line.resize(RECORD_LIMIT - 2, b' ');
            line.push(b'\n');
            let would_overflow = stream.bytes + line.len() > BYTE_LIMIT;
            let result = stream.push(&line, 100);
            if would_overflow {
                assert_eq!(result, Err(StreamError::Limit));
                break;
            }
            result.unwrap();
            sequence += 1;
        }
        assert!(stream.latest_health(100, 10).is_none());
    }
}
