use intellicord_observe_common::EVENT_BYTES;
use serde::Serialize;

use super::timed::{Rejection, TimedHealth, TimedProcessMonitor};

pub const RECORD_LIMIT: usize = 256;
pub const BYTE_LIMIT: usize = RECORD_LIMIT * EVENT_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    InvalidAnchor,
    Limit,
    ReceiptClock,
    Event(Rejection),
    Incomplete,
    FailedExit,
    Rejected,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "process event stream rejected: {self:?}")
    }
}

impl std::error::Error for StreamError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct EventSummary {
    pub schema_version: u16,
    pub records: usize,
    pub health: TimedHealth,
}

pub struct EventStream {
    monitor: TimedProcessMonitor,
    pending: [u8; EVENT_BYTES],
    pending_len: usize,
    bytes: usize,
    last_receipt_ns: u64,
    rejected: bool,
}

impl EventStream {
    pub fn new(
        process_id: u32,
        source_ns: u64,
        before_ns: u64,
        after_ns: u64,
    ) -> Result<Self, StreamError> {
        Ok(Self {
            monitor: TimedProcessMonitor::new(process_id, source_ns, before_ns, after_ns)
                .ok_or(StreamError::InvalidAnchor)?,
            pending: [0; EVENT_BYTES],
            pending_len: 0,
            bytes: 0,
            last_receipt_ns: after_ns,
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

    fn push_inner(&mut self, mut bytes: &[u8], receipt_ns: u64) -> Result<(), StreamError> {
        if receipt_ns < self.last_receipt_ns {
            return Err(StreamError::ReceiptClock);
        }
        let total = self
            .bytes
            .checked_add(bytes.len())
            .ok_or(StreamError::Limit)?;
        if total > BYTE_LIMIT {
            return Err(StreamError::Limit);
        }
        self.bytes = total;
        self.last_receipt_ns = receipt_ns;
        while !bytes.is_empty() {
            let count = bytes.len().min(EVENT_BYTES - self.pending_len);
            self.pending[self.pending_len..self.pending_len + count]
                .copy_from_slice(&bytes[..count]);
            self.pending_len += count;
            bytes = &bytes[count..];
            if self.pending_len == EVENT_BYTES {
                self.monitor
                    .accept(&self.pending, receipt_ns)
                    .map_err(StreamError::Event)?;
                self.pending_len = 0;
            }
        }
        Ok(())
    }

    pub fn finish(self, exit_success: bool) -> Result<EventSummary, StreamError> {
        self.finish_inner(exit_success, false)
    }

    pub(super) fn finish_collection(self, exit_success: bool) -> Result<EventSummary, StreamError> {
        self.finish_inner(exit_success, true)
    }

    fn finish_inner(
        self,
        exit_success: bool,
        allow_empty: bool,
    ) -> Result<EventSummary, StreamError> {
        if self.rejected {
            return Err(StreamError::Rejected);
        }
        if !exit_success {
            return Err(StreamError::FailedExit);
        }
        if (!allow_empty && self.bytes == 0) || self.pending_len != 0 {
            return Err(StreamError::Incomplete);
        }
        Ok(EventSummary {
            schema_version: 1,
            records: self.bytes / EVENT_BYTES,
            health: self.monitor.health(),
        })
    }
}

pub fn generated_checks() -> Vec<super::timed::ReplayCase> {
    use super::timed::{ReplayCase, generated_record};
    let stream = || EventStream::new(42, 1000, 100, 120).expect("fixed generated event stream");
    let bytes = [generated_record(42, 2000), generated_record(42, 2100)].concat();
    let mut fragmented = stream();
    let delivered = bytes
        .chunks(1)
        .all(|chunk| fragmented.push(chunk, 1300).is_ok())
        && fragmented
            .finish(true)
            .is_ok_and(|report| report.records == 2 && report.health.accepted == 2);
    let mut truncated = stream();
    let incomplete = truncated.push(&bytes[..47], 1300).is_ok()
        && truncated.finish(true) == Err(StreamError::Incomplete);
    let mut foreign = stream();
    let foreign_rejected = foreign.push(&generated_record(43, 2000), 1300)
        == Err(StreamError::Event(Rejection::Record))
        && foreign.finish(true) == Err(StreamError::Rejected);
    let mut limited = stream();
    let limit_rejected = limited.push(&vec![0; BYTE_LIMIT + 1], 1300) == Err(StreamError::Limit)
        && limited.push(&bytes, 1300) == Err(StreamError::Rejected);
    let mut backwards = stream();
    let receipt_rejected = backwards.push(&bytes[..1], 1300).is_ok()
        && backwards.push(&bytes[1..], 1299) == Err(StreamError::ReceiptClock);
    let mut failed = stream();
    let exit_rejected =
        failed.push(&bytes, 1300).is_ok() && failed.finish(false) == Err(StreamError::FailedExit);
    let mut delayed = stream();
    let delayed_rejected = delayed.push(&bytes[..1], 120).is_ok()
        && delayed.push(&bytes[1..24], 250_002_000) == Err(StreamError::Event(Rejection::Stale));
    [
        ("fragmented_delivery", delivered),
        ("truncated_record", incomplete),
        ("foreign_record", foreign_rejected),
        ("stream_limit", limit_rejected),
        ("receipt_regression", receipt_rejected),
        ("failed_exit", exit_rejected),
        (
            "empty_stream",
            stream().finish(true) == Err(StreamError::Incomplete),
        ),
        ("delayed_fragment", delayed_rejected),
    ]
    .into_iter()
    .map(|(name, passed)| ReplayCase { name, passed })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_after_a_valid_prefix_never_returns_a_partial_success_summary() {
        let mut bytes = record(2000);
        let mut foreign = record(2100);
        foreign[8..12].copy_from_slice(&43_u32.to_ne_bytes());
        bytes.extend_from_slice(&foreign);
        let mut stream = EventStream::new(42, 1000, 100, 120).unwrap();
        assert_eq!(
            stream.push(&bytes, 1300),
            Err(StreamError::Event(Rejection::Record))
        );
        assert_eq!(stream.push(&record(2200), 1400), Err(StreamError::Rejected));
        assert_eq!(stream.finish(true), Err(StreamError::Rejected));
    }

    #[test]
    fn generated_transport_checks_cover_all_cases_without_sensor_claims() {
        let cases = generated_checks();
        assert_eq!(cases.len(), 8);
        assert!(cases.iter().all(|case| case.passed));
    }

    fn record(time: u64) -> Vec<u8> {
        [
            1_u32.to_ne_bytes().as_slice(),
            1_u32.to_ne_bytes().as_slice(),
            42_u32.to_ne_bytes().as_slice(),
            43_u32.to_ne_bytes().as_slice(),
            time.to_ne_bytes().as_slice(),
        ]
        .concat()
    }

    #[test]
    fn fragmented_delivery_requires_complete_records_and_successful_exit() {
        let bytes = [record(2000), record(2100)].concat();
        for chunk_size in [1, 7, 24, 48] {
            let mut stream = EventStream::new(42, 1000, 100, 120).unwrap();
            for chunk in bytes.chunks(chunk_size) {
                stream.push(chunk, 1300).unwrap();
            }
            let report = stream.finish(true).unwrap();
            assert_eq!(report.records, 2);
            assert_eq!(report.health.accepted, 2);
        }
        let mut partial = EventStream::new(42, 1000, 100, 120).unwrap();
        partial.push(&bytes[..47], 1300).unwrap();
        assert_eq!(partial.finish(true), Err(StreamError::Incomplete));
        let mut failed = EventStream::new(42, 1000, 100, 120).unwrap();
        failed.push(&bytes, 1300).unwrap();
        assert_eq!(failed.finish(false), Err(StreamError::FailedExit));
        assert!(std::mem::size_of::<EventStream>() <= 320);
    }

    #[test]
    fn limits_and_receipt_regressions_permanently_reject_the_stream() {
        let mut stream = EventStream::new(42, 0, 0, 0).unwrap();
        stream.push(&vec![0; BYTE_LIMIT + 1], 0).unwrap_err();
        assert_eq!(stream.push(&record(0), 0), Err(StreamError::Rejected));
        assert_eq!(stream.finish(true), Err(StreamError::Rejected));
        let mut stream = EventStream::new(42, 0, 0, 0).unwrap();
        stream.push(&[1], 10).unwrap();
        assert_eq!(stream.push(&[0], 9), Err(StreamError::ReceiptClock));
        assert_eq!(stream.finish(true), Err(StreamError::Rejected));
        let mut exact = EventStream::new(42, 0, 0, 0).unwrap();
        exact.push(&record(0).repeat(RECORD_LIMIT), 0).unwrap();
        assert_eq!(exact.finish(true).unwrap().records, RECORD_LIMIT);
    }
}
