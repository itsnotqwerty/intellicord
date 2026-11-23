use intellicord_observe_common::{
    EVENT_BYTES, ProcessEvent, ProcessEventSink, PublishOutcome, next_ring_drop_count,
    publish_exec, ring_drop_total, submit_reserved_exec,
};
use serde::Serialize;

const CAPACITY: usize = 2;

#[derive(Serialize)]
pub struct Check {
    pub name: &'static str,
    pub passed: bool,
}

#[derive(Serialize)]
pub struct Replay {
    pub schema_version: u16,
    pub source: &'static str,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub capacity: usize,
    pub attempts: u64,
    pub submitted: u64,
    pub dropped: u64,
    pub cases: [Check; 6],
    pub accounting: [Check; 6],
    pub reservation: [Check; 6],
}

#[derive(Default)]
struct ReservedSink {
    slots: [Option<[u8; EVENT_BYTES]>; CAPACITY],
    pending: usize,
    attempts: u64,
    clock_reads: u64,
    submissions: u64,
    drops: u64,
    ordered: bool,
}

impl ProcessEventSink for ReservedSink {
    fn try_submit(&mut self, process_id: u32, thread_id: u32) -> bool {
        self.attempts += 1;
        let reservation = (self.pending < CAPACITY).then_some(self.pending);
        let stage = core::cell::Cell::new(1);
        let accepted = submit_reserved_exec(
            reservation,
            process_id,
            thread_id,
            || {
                self.clock_reads += 1;
                stage.set(2);
                self.clock_reads
            },
            |slot, event| {
                self.ordered &= stage.get() == 2;
                stage.set(3);
                self.slots[slot] = Some(event.to_ne_bytes());
                self.pending += 1;
                self.submissions += 1;
            },
        );
        self.ordered &= stage.get() == if accepted { 3 } else { 1 };
        accepted
    }

    fn record_drop(&mut self) {
        self.drops = next_ring_drop_count(self.drops);
    }
}

fn reservation_checks() -> [Check; 6] {
    let identity = |thread| (42_u64 << 32) | thread;
    let mut sink = ReservedSink {
        ordered: true,
        ..ReservedSink::default()
    };
    let filtered = publish_exec(43, identity(42), &mut sink) == PublishOutcome::Filtered
        && sink.attempts == 0
        && sink.clock_reads == 0
        && sink.submissions == 0;
    let leader = publish_exec(42, identity(42), &mut sink);
    let worker = publish_exec(42, identity(43), &mut sink);
    let single = leader == PublishOutcome::Submitted
        && worker == PublishOutcome::Submitted
        && sink.attempts == 2
        && sink.clock_reads == 2
        && sink.submissions == 2;
    let records = sink.slots;
    let native = records.into_iter().enumerate().all(|(index, bytes)| {
        bytes.is_some_and(|bytes| {
            let mut expected = [0; EVENT_BYTES];
            expected[..4].copy_from_slice(&1_u32.to_ne_bytes());
            expected[4..8].copy_from_slice(&1_u32.to_ne_bytes());
            expected[8..12].copy_from_slice(&42_u32.to_ne_bytes());
            expected[12..16].copy_from_slice(&(42_u32 + index as u32).to_ne_bytes());
            expected[16..].copy_from_slice(&(1_u64 + index as u64).to_ne_bytes());
            bytes == expected && ProcessEvent::decode(&bytes, 42).is_some()
        })
    });
    let full = publish_exec(42, identity(44), &mut sink) == PublishOutcome::Dropped
        && sink.slots == records
        && sink.clock_reads == 2
        && sink.submissions == 2
        && sink.drops == 1;
    sink.slots.fill(None);
    sink.pending = 0;
    let recovered = publish_exec(42, identity(45), &mut sink) == PublishOutcome::Submitted
        && sink.slots[0].is_some_and(|bytes| {
            ProcessEvent::decode(&bytes, 42)
                .is_some_and(|event| event.thread_id == 45 && event.monotonic_ns == 3)
        })
        && sink.slots[1].is_none()
        && sink.pending == 1
        && sink.clock_reads == 3
        && sink.submissions == 3
        && sink.drops == 1;
    [
        ("filtered_reservation", filtered),
        ("reserve_before_clock", sink.ordered),
        ("single_submission", single),
        ("native_record_delivery", native),
        ("full_ring_preservation", full),
        ("fresh_recovery", recovered),
    ]
    .map(|(name, passed)| Check { name, passed })
}

#[derive(Default)]
struct GeneratedSink {
    slots: [Option<(u32, u32)>; CAPACITY],
    pending: usize,
    attempts: u64,
    submitted: u64,
    dropped: u64,
}

impl ProcessEventSink for GeneratedSink {
    fn try_submit(&mut self, process_id: u32, thread_id: u32) -> bool {
        self.attempts += 1;
        if self.pending == CAPACITY {
            return false;
        }
        self.slots[self.pending] = Some((process_id, thread_id));
        self.pending += 1;
        self.submitted += 1;
        true
    }

    fn record_drop(&mut self) {
        self.dropped = next_ring_drop_count(self.dropped);
    }
}

fn accounting_checks() -> [Check; 6] {
    let identity = (42_u64 << 32) | 42;
    let mut sink = GeneratedSink {
        slots: [Some((42, 42)); CAPACITY],
        pending: CAPACITY,
        dropped: u64::MAX - 1,
        ..GeneratedSink::default()
    };
    let first = publish_exec(42, identity, &mut sink);
    let second = publish_exec(42, identity, &mut sink);
    let saturated = first == PublishOutcome::Dropped
        && second == PublishOutcome::Dropped
        && sink.dropped == u64::MAX
        && sink.attempts == 2;
    let filtered = publish_exec(43, identity, &mut sink) == PublishOutcome::Filtered
        && sink.dropped == u64::MAX
        && sink.attempts == 2;
    sink.slots.fill(None);
    sink.pending = 0;
    let recovered = publish_exec(42, identity, &mut sink) == PublishOutcome::Submitted
        && sink.dropped == u64::MAX
        && sink.submitted == 1
        && sink.attempts == 3;
    let mut monitor = super::ProcessMonitor::new(42).expect("generated process is nonzero");
    let snapshots = [
        ([2, 3], 5),
        ([2, 3], 5),
        ([1, 0], 5),
        ([4, 3], 7),
        ([u64::MAX, 1], u64::MAX),
        ([0, 0], u64::MAX),
    ]
    .into_iter()
    .all(|(counters, expected)| {
        monitor.update_ring_dropped(ring_drop_total(counters));
        monitor.health().ring_dropped == expected
    });
    [
        ("counter_saturation", saturated),
        (
            "per_cpu_sum",
            ring_drop_total([2, 0, 3]) == 5 && ring_drop_total([]) == 0,
        ),
        (
            "aggregate_saturation",
            ring_drop_total([u64::MAX - 1, 2]) == u64::MAX
                && ring_drop_total([u64::MAX, u64::MAX]) == u64::MAX,
        ),
        ("snapshot_high_water", snapshots),
        ("saturated_filter", filtered),
        ("saturated_recovery", recovered),
    ]
    .map(|(name, passed)| Check { name, passed })
}

pub fn generated_replay() -> Replay {
    let identity = |process: u32, thread: u32| (u64::from(process) << 32) | u64::from(thread);
    let mut sink = GeneratedSink::default();
    let filtered =
        [(0, 42, 42), (42, 43, 42), (42, 42, 0)]
            .into_iter()
            .all(|(target, process, thread)| {
                publish_exec(target, identity(process, thread), &mut sink)
                    == PublishOutcome::Filtered
            })
            && sink.attempts == 0
            && sink.dropped == 0;
    let leader = publish_exec(42, identity(42, 42), &mut sink);
    let worker = publish_exec(42, identity(42, 43), &mut sink);
    let preserved = leader == PublishOutcome::Submitted
        && worker == PublishOutcome::Submitted
        && sink.slots == [Some((42, 42)), Some((42, 43))];
    let full = publish_exec(42, identity(42, 44), &mut sink) == PublishOutcome::Dropped
        && sink.pending == CAPACITY
        && sink.attempts == 3
        && sink.dropped == 1;
    let foreign = publish_exec(42, identity(43, 42), &mut sink) == PublishOutcome::Filtered
        && sink.attempts == 3
        && sink.dropped == 1;
    let second_drop = publish_exec(42, identity(42, 45), &mut sink);
    sink.slots.fill(None);
    sink.pending = 0;
    let recovered = publish_exec(42, identity(42, 46), &mut sink) == PublishOutcome::Submitted
        && sink.pending == 1
        && sink.slots == [Some((42, 46)), None];
    let agreement = second_drop == PublishOutcome::Dropped
        && sink.attempts == 5
        && sink.submitted == 3
        && sink.dropped == 2;
    Replay {
        schema_version: 1,
        source: "generated_replay",
        sensor_running: false,
        load_test_performed: false,
        capacity: CAPACITY,
        attempts: sink.attempts,
        submitted: sink.submitted,
        dropped: sink.dropped,
        cases: [
            ("filter_before_reservation", filtered),
            ("submitted_identity", preserved),
            ("capacity_rejection", full),
            ("full_ring_filter", foreign),
            ("recovery_after_drain", recovered),
            ("drop_agreement", agreement),
        ]
        .map(|(name, passed)| Check { name, passed }),
        accounting: accounting_checks(),
        reservation: reservation_checks(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_producer_is_bounded_deterministic_and_inactive() {
        let report = generated_replay();
        assert!(report.cases.iter().all(|case| case.passed));
        assert!(report.accounting.iter().all(|case| case.passed));
        assert!(report.reservation.iter().all(|case| case.passed));
        assert_eq!(
            report.reservation.each_ref().map(|case| case.name),
            [
                "filtered_reservation",
                "reserve_before_clock",
                "single_submission",
                "native_record_delivery",
                "full_ring_preservation",
                "fresh_recovery",
            ]
        );
        assert_eq!(
            report.accounting.each_ref().map(|case| case.name),
            [
                "counter_saturation",
                "per_cpu_sum",
                "aggregate_saturation",
                "snapshot_high_water",
                "saturated_filter",
                "saturated_recovery",
            ]
        );
        assert_eq!(
            (
                report.capacity,
                report.attempts,
                report.submitted,
                report.dropped
            ),
            (2, 5, 3, 2)
        );
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value, serde_json::to_value(generated_replay()).unwrap());
        assert_eq!(value.as_object().unwrap().len(), 11);
        assert_eq!(value["source"], "generated_replay");
        assert_eq!(value["sensor_running"], false);
        assert_eq!(value["load_test_performed"], false);
        let bytes = serde_json::to_string(&value).unwrap();
        assert!(bytes.len() < 1536);
        assert!(!bytes.contains(&u64::MAX.to_string()));
        for check in value["accounting"].as_array().unwrap() {
            assert_eq!(check.as_object().unwrap().len(), 2);
            assert!(check["name"].is_string());
            assert!(check["passed"].is_boolean());
        }
        for field in ["process_id", "thread_id", "monotonic_ns", "slots"] {
            assert!(!bytes.contains(field));
        }
    }
}
