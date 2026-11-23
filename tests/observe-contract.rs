use intellicord::observe::{ProcessMonitor, generated_replay};
use intellicord_observe_common::{EVENT_BYTES, ProcessEvent};

fn record(schema: u32, kind: u32, process: u32, thread: u32, time: u64) -> Vec<u8> {
    [
        schema.to_ne_bytes().as_slice(),
        kind.to_ne_bytes().as_slice(),
        process.to_ne_bytes().as_slice(),
        thread.to_ne_bytes().as_slice(),
        time.to_ne_bytes().as_slice(),
    ]
    .concat()
}

#[test]
fn kernel_record_constructor_matches_independent_native_layout() {
    for (process, thread, time) in [
        (42, 43, 0),
        (u32::MAX, u32::MAX - 1, u64::MAX),
        (0x0102_0304, 0x0506_0708, 0x0102_0304_0506_0708),
    ] {
        let event = ProcessEvent::exec(process, thread, time);
        assert_eq!(event.schema_version, 1);
        assert_eq!(event.kind, 1);
        assert_eq!(event.process_id, process);
        assert_eq!(event.thread_id, thread);
        assert_eq!(event.monotonic_ns, time);
        assert_eq!(
            event.to_ne_bytes().as_slice(),
            record(1, 1, process, thread, time)
        );
        let mut unaligned = vec![0xff];
        unaligned.extend_from_slice(&event.to_ne_bytes());
        let decoded = ProcessEvent::decode(&unaligned[1..], process).unwrap();
        assert_eq!(
            (decoded.process_id, decoded.thread_id, decoded.monotonic_ns),
            (process, thread, time)
        );
    }
    for (process, thread) in [(0, 42), (42, 0)] {
        assert!(
            ProcessEvent::decode(
                &ProcessEvent::exec(process, thread, 0).to_ne_bytes(),
                process
            )
            .is_none()
        );
    }
}

#[test]
fn ring_loss_arithmetic_saturates_per_cpu_and_across_snapshots() {
    use intellicord_observe_common::{next_ring_drop_count, ring_drop_total};

    for (current, expected) in [
        (0, 1),
        (41, 42),
        (u64::MAX - 1, u64::MAX),
        (u64::MAX, u64::MAX),
    ] {
        assert_eq!(next_ring_drop_count(current), expected);
    }
    for (counters, expected) in [
        (vec![], 0),
        (vec![0, 0], 0),
        (vec![2, 0, 3], 5),
        (vec![u64::MAX - 2, 2], u64::MAX),
        (vec![u64::MAX - 2, 3], u64::MAX),
        (vec![u64::MAX, u64::MAX], u64::MAX),
    ] {
        assert_eq!(ring_drop_total(counters.iter().copied()), expected);
        assert_eq!(ring_drop_total(counters.into_iter().rev()), expected);
    }
    let mut monitor = ProcessMonitor::new(42).unwrap();
    for (counters, expected) in [
        ([2, 3], 5),
        ([2, 3], 5),
        ([1, 0], 5),
        ([4, 3], 7),
        ([u64::MAX, 1], u64::MAX),
        ([0, 0], u64::MAX),
    ] {
        monitor.update_ring_dropped(ring_drop_total(counters));
        assert_eq!(monitor.health().ring_dropped, expected);
    }
}

#[test]
fn kernel_selection_uses_tgid_not_tid_and_rejects_disabled_or_invalid_identity() {
    use intellicord_observe_common::selected_identity;
    for (target, process, thread, expected) in [
        (42, 42, 42, Some((42, 42))),
        (42, 42, 43, Some((42, 43))),
        (42, 43, 42, None),
        (0, 0, 1, None),
        (0, 42, 42, None),
        (42, 0, 42, None),
        (42, 42, 0, None),
        (u32::MAX, u32::MAX, u32::MAX, Some((u32::MAX, u32::MAX))),
    ] {
        let identity = (u64::from(process) << 32) | u64::from(thread);
        assert_eq!(selected_identity(target, identity), expected);
    }
}

#[test]
fn shared_selection_generated_report_is_bounded_and_does_not_expose_identifiers() {
    let checks = intellicord::observe::generated_selection_checks();
    let expected = [
        "disabled_target",
        "foreign_tgid",
        "matching_leader",
        "matching_thread",
        "zero_process",
        "zero_thread",
        "full_width_identity",
    ];
    for (check, name) in checks.iter().zip(expected) {
        assert_eq!(check.name, name);
        assert!(check.passed);
    }
    let value = serde_json::to_value(checks).unwrap();
    assert!(serde_json::to_vec(&value).unwrap().len() < 512);
    for check in value.as_array().unwrap() {
        let object = check.as_object().unwrap();
        assert_eq!(object.len(), 2);
        assert!(object.contains_key("name"));
        assert!(object.contains_key("passed"));
    }
}

#[test]
fn shared_producer_filters_before_reservation_and_counts_only_failed_selected_submissions() {
    use intellicord_observe_common::{ProcessEventSink, PublishOutcome, publish_exec};
    #[derive(Default)]
    struct Sink {
        accepted: Vec<(u32, u32)>,
        attempts: usize,
        drops: usize,
    }
    impl ProcessEventSink for Sink {
        fn try_submit(&mut self, process_id: u32, thread_id: u32) -> bool {
            self.attempts += 1;
            if self.accepted.len() == 2 {
                return false;
            }
            self.accepted.push((process_id, thread_id));
            true
        }
        fn record_drop(&mut self) {
            self.drops += 1;
        }
    }
    let identity = |process: u32, thread: u32| (u64::from(process) << 32) | u64::from(thread);
    let mut sink = Sink::default();
    for (target, process, thread, expected) in [
        (0, 42, 42, PublishOutcome::Filtered),
        (42, 43, 42, PublishOutcome::Filtered),
        (42, 42, 0, PublishOutcome::Filtered),
        (42, 42, 42, PublishOutcome::Submitted),
        (42, 42, 43, PublishOutcome::Submitted),
        (42, 42, 44, PublishOutcome::Dropped),
        (42, 43, 42, PublishOutcome::Filtered),
        (42, 42, 45, PublishOutcome::Dropped),
    ] {
        assert_eq!(
            publish_exec(target, identity(process, thread), &mut sink),
            expected
        );
    }
    assert_eq!(sink.accepted, [(42, 42), (42, 43)]);
    assert_eq!(sink.attempts, 4);
    assert_eq!(sink.drops, 2);
    sink.accepted.clear();
    assert_eq!(
        publish_exec(42, identity(42, 46), &mut sink),
        PublishOutcome::Submitted
    );
    assert_eq!(sink.attempts, 5);
    assert_eq!(sink.drops, 2);
    assert_eq!(sink.accepted, [(42, 46)]);
}

#[test]
fn fixed_native_record_preserves_process_and_clock_without_alignment_assumptions() {
    let bytes = record(1, 1, 42, 43, u64::MAX);
    assert_eq!(bytes.len(), EVENT_BYTES);
    let mut unaligned = vec![0];
    unaligned.extend_from_slice(&bytes);
    let event = ProcessEvent::decode(&unaligned[1..], 42).unwrap();
    assert_eq!(event.process_id, 42);
    assert_eq!(event.thread_id, 43);
    assert_eq!(event.monotonic_ns, u64::MAX);
}

#[test]
fn decoded_kernel_timestamp_maps_to_an_interval_without_changing_decoder_rules() {
    use intellicord::acoustic::ClockError;
    use intellicord::observe::clock::{HORIZON_NS, KernelClockAnchor};
    assert_eq!(intellicord_observe_common::CLOCK_DOMAIN, "linux_monotonic");
    let anchor = KernelClockAnchor::from_bracket(1000, 100, 110).unwrap();
    let bytes = record(1, 1, 42, 43, 2000);
    let event = ProcessEvent::decode(&bytes, 42).unwrap();
    let mapped = anchor.map(event.monotonic_ns).unwrap();
    assert_eq!((mapped.earliest_ns, mapped.latest_ns), (1099, 1111));
    assert_eq!(anchor.map(999), Err(ClockError::BeforeAnchor));
    assert_eq!(anchor.map(1001 + HORIZON_NS), Err(ClockError::Expired));
    assert!(ProcessEvent::decode(&record(1, 1, 41, 43, 2000), 42).is_none());
}

#[test]
fn malformed_foreign_and_future_records_are_rejected() {
    for bytes in [
        vec![],
        vec![0; EVENT_BYTES - 1],
        vec![0; EVENT_BYTES + 1],
        record(2, 1, 42, 42, 0),
        record(1, 2, 42, 42, 0),
        record(1, 1, 41, 41, 0),
        record(1, 1, 42, 0, 0),
    ] {
        assert!(ProcessEvent::decode(&bytes, 42).is_none());
    }
    assert!(ProcessEvent::decode(&record(1, 1, 0, 1, 0), 0).is_none());
}

#[test]
fn rejected_inputs_do_not_advance_the_accepted_clock_or_lose_drop_counts() {
    assert!(ProcessMonitor::new(0).is_none());
    let mut monitor = ProcessMonitor::new(42).unwrap();
    assert!(monitor.accept(&record(1, 1, 42, 42, 10)));
    assert!(!monitor.accept(&record(1, 1, 43, 43, u64::MAX)));
    assert!(!monitor.accept(&record(1, 1, 42, 42, 9)));
    assert!(monitor.accept(&record(1, 1, 42, 42, 11)));
    monitor.update_ring_dropped(8);
    monitor.update_ring_dropped(2);
    let health = monitor.health();
    assert_eq!(health.accepted, 2);
    assert_eq!(health.rejected_record, 1);
    assert_eq!(health.rejected_clock, 1);
    assert_eq!(health.ring_dropped, 8);
}

#[test]
fn generated_replay_is_deterministic_health_only_and_does_not_claim_capture() {
    let expected = serde_json::json!({"schema_version":1,"accepted":2,
        "rejected_record":2,"rejected_clock":1,"ring_dropped":3});
    for _ in 0..2 {
        assert_eq!(serde_json::to_value(generated_replay()).unwrap(), expected);
    }
    assert!(std::mem::size_of::<ProcessMonitor>() <= 128);
}
