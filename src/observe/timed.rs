use intellicord_observe_common::ProcessEvent;
use serde::Serialize;

use super::clock::{HORIZON_NS, KernelClockAnchor};
use crate::acoustic::ClockInterval;

pub const MAX_EVENT_AGE_NS: u64 = 250_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    Record,
    ReceiptClock,
    AnchorExpired,
    SourceClock,
    Mapping,
    Future,
    Stale,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize)]
pub struct TimedHealth {
    pub accepted: u64,
    pub rejected_record: u64,
    pub rejected_receipt_clock: u64,
    pub rejected_anchor_expired: u64,
    pub rejected_source_clock: u64,
    pub rejected_mapping: u64,
    pub rejected_future: u64,
    pub rejected_stale: u64,
}

#[derive(Serialize)]
pub struct ReplayCase {
    pub name: &'static str,
    pub passed: bool,
}

#[derive(Serialize)]
pub struct TimedReplay {
    pub schema_version: u16,
    pub source: &'static str,
    pub clock_domain: &'static str,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub health: TimedHealth,
    pub cases: Vec<ReplayCase>,
    pub transport: Vec<ReplayCase>,
}

pub(super) fn generated_record(process_id: u32, time: u64) -> Vec<u8> {
    ProcessEvent::exec(process_id, process_id, time)
        .to_ne_bytes()
        .to_vec()
}

pub fn generated_replay() -> TimedReplay {
    let mut monitor =
        TimedProcessMonitor::new(42, 1000, 100, 120).expect("fixed generated clock bracket");
    let first = monitor.accept(&generated_record(42, 2000), 1120);
    let interval_bounds =
        first.is_ok_and(|interval| interval.earliest_ns == 1099 && interval.latest_ns == 1120);
    let mut cases = vec![ReplayCase {
        name: "interval_bounds",
        passed: interval_bounds,
    }];
    for (name, process_id, source_ns, receipt_ns, reason) in [
        ("foreign_record", 43, 2100, 1200, Rejection::Record),
        (
            "receipt_regression",
            42,
            2100,
            1119,
            Rejection::ReceiptClock,
        ),
        ("source_regression", 42, 1999, 1200, Rejection::SourceClock),
        (
            "anchor_expiry",
            42,
            2000,
            121 + HORIZON_NS,
            Rejection::AnchorExpired,
        ),
        ("mapping_expiry", 42, u64::MAX, 1200, Rejection::Mapping),
        ("future_event", 42, 3000, 1200, Rejection::Future),
        (
            "stale_event",
            42,
            2000,
            1120 + MAX_EVENT_AGE_NS,
            Rejection::Stale,
        ),
    ] {
        cases.push(ReplayCase {
            name,
            passed: monitor.accept(&generated_record(process_id, source_ns), receipt_ns)
                == Err(reason),
        });
    }
    let recovered = monitor.accept(&generated_record(42, 2100), 1220).is_ok();
    let health = monitor.health();
    cases.push(ReplayCase {
        name: "rejection_isolation",
        passed: recovered
            && health
                == TimedHealth {
                    accepted: 2,
                    rejected_record: 1,
                    rejected_receipt_clock: 1,
                    rejected_anchor_expired: 1,
                    rejected_source_clock: 1,
                    rejected_mapping: 1,
                    rejected_future: 1,
                    rejected_stale: 1,
                },
    });
    TimedReplay {
        schema_version: 1,
        source: "generated_replay",
        clock_domain: intellicord_observe_common::CLOCK_DOMAIN,
        sensor_running: false,
        load_test_performed: false,
        health,
        cases,
        transport: super::events::generated_checks(),
    }
}

pub struct TimedProcessMonitor {
    process_id: u32,
    anchor: KernelClockAnchor,
    anchored_at_ns: u64,
    deadline_ns: u64,
    last_receipt_ns: Option<u64>,
    last_source_ns: Option<u64>,
    health: TimedHealth,
}

impl TimedProcessMonitor {
    pub fn new(process_id: u32, source_ns: u64, before_ns: u64, after_ns: u64) -> Option<Self> {
        if process_id == 0 {
            return None;
        }
        Some(Self {
            process_id,
            anchor: KernelClockAnchor::from_bracket(source_ns, before_ns, after_ns).ok()?,
            anchored_at_ns: after_ns,
            deadline_ns: after_ns.checked_add(HORIZON_NS)?,
            last_receipt_ns: None,
            last_source_ns: None,
            health: TimedHealth::default(),
        })
    }

    pub fn accept(&mut self, bytes: &[u8], receipt_ns: u64) -> Result<ClockInterval, Rejection> {
        let result = self.validate(bytes, receipt_ns);
        match result {
            Ok((source_ns, interval)) => {
                self.last_receipt_ns = Some(receipt_ns);
                self.last_source_ns = Some(source_ns);
                self.health.accepted = self.health.accepted.saturating_add(1);
                Ok(interval)
            }
            Err(reason) => {
                let counter = match reason {
                    Rejection::Record => &mut self.health.rejected_record,
                    Rejection::ReceiptClock => &mut self.health.rejected_receipt_clock,
                    Rejection::AnchorExpired => &mut self.health.rejected_anchor_expired,
                    Rejection::SourceClock => &mut self.health.rejected_source_clock,
                    Rejection::Mapping => &mut self.health.rejected_mapping,
                    Rejection::Future => &mut self.health.rejected_future,
                    Rejection::Stale => &mut self.health.rejected_stale,
                };
                *counter = counter.saturating_add(1);
                Err(reason)
            }
        }
    }

    fn validate(&self, bytes: &[u8], receipt_ns: u64) -> Result<(u64, ClockInterval), Rejection> {
        let event = ProcessEvent::decode(bytes, self.process_id).ok_or(Rejection::Record)?;
        if receipt_ns < self.anchored_at_ns
            || self.last_receipt_ns.is_some_and(|last| receipt_ns < last)
        {
            return Err(Rejection::ReceiptClock);
        }
        if receipt_ns > self.deadline_ns {
            return Err(Rejection::AnchorExpired);
        }
        if self
            .last_source_ns
            .is_some_and(|last| event.monotonic_ns < last)
        {
            return Err(Rejection::SourceClock);
        }
        let mut interval = self
            .anchor
            .map(event.monotonic_ns)
            .map_err(|_| Rejection::Mapping)?;
        if interval.earliest_ns > receipt_ns {
            return Err(Rejection::Future);
        }
        if receipt_ns - interval.earliest_ns > MAX_EVENT_AGE_NS {
            return Err(Rejection::Stale);
        }
        interval.latest_ns = interval.latest_ns.min(receipt_ns);
        Ok((event.monotonic_ns, interval))
    }

    pub fn health(&self) -> TimedHealth {
        self.health
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_saturate_without_changing_acceptance() {
        let mut monitor = TimedProcessMonitor::new(42, 0, 0, 0).unwrap();
        monitor.health.accepted = u64::MAX;
        monitor.health.rejected_record = u64::MAX;
        assert!(monitor.accept(&record(0), 0).is_ok());
        assert_eq!(monitor.accept(&[], 0), Err(Rejection::Record));
        assert_eq!(monitor.health().accepted, u64::MAX);
        assert_eq!(monitor.health().rejected_record, u64::MAX);
    }

    #[test]
    fn generated_timed_replay_is_bounded_deterministic_and_inactive() {
        let report = generated_replay();
        assert_eq!(report.cases.len(), 9);
        assert!(report.cases.iter().all(|case| case.passed));
        assert_eq!(report.health.accepted, 2);
        assert!(!report.sensor_running);
        assert!(!report.load_test_performed);
        let bytes = serde_json::to_vec(&report).unwrap();
        assert!(bytes.len() < 2048);
        assert_eq!(bytes, serde_json::to_vec(&generated_replay()).unwrap());
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
    fn rejected_input_cannot_poison_receipt_or_source_watermarks() {
        let mut monitor = TimedProcessMonitor::new(42, 1000, 100, 120).unwrap();
        assert_eq!(
            monitor.accept(&record(2000), 1120).unwrap().earliest_ns,
            1099
        );
        assert_eq!(
            monitor.accept(&record(u64::MAX), 1200),
            Err(Rejection::Mapping)
        );
        assert_eq!(
            monitor.accept(&record(1999), 1200),
            Err(Rejection::SourceClock)
        );
        assert_eq!(
            monitor.accept(&record(2100), 1119),
            Err(Rejection::ReceiptClock)
        );
        assert_eq!(monitor.accept(&[0], u64::MAX), Err(Rejection::Record));
        assert!(monitor.accept(&record(2100), 1220).is_ok());
        assert_eq!(monitor.health().accepted, 2);
        assert!(std::mem::size_of::<TimedProcessMonitor>() <= 256);
    }

    #[test]
    fn local_deadline_stops_frozen_source_and_stale_uncertainty_is_rejected() {
        let mut monitor = TimedProcessMonitor::new(42, 1000, 100, 120).unwrap();
        assert_eq!(
            monitor.accept(&record(1000), 100),
            Err(Rejection::ReceiptClock)
        );
        assert_eq!(
            monitor.accept(&record(1000), 120 + HORIZON_NS + 1),
            Err(Rejection::AnchorExpired)
        );
        assert_eq!(
            monitor.accept(&record(1000), 101 + MAX_EVENT_AGE_NS),
            Err(Rejection::Stale)
        );
        assert_eq!(monitor.accept(&record(2000), 120), Err(Rejection::Future));
        let clipped = monitor.accept(&record(1000), 120).unwrap();
        assert_eq!((clipped.earliest_ns, clipped.latest_ns), (100, 120));
        assert!(TimedProcessMonitor::new(0, 0, 0, 0).is_none());
        assert!(TimedProcessMonitor::new(42, 0, 2, 1).is_none());
        assert!(TimedProcessMonitor::new(42, 0, u64::MAX, u64::MAX).is_none());
    }
}
