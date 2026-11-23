use std::cell::{Cell, RefCell};

use intellicord_observe_common::{ProcessEvent, submit_reserved_exec};

#[test]
fn failed_reservation_never_reads_clock_or_submits() {
    assert!(!submit_reserved_exec(
        None::<()>,
        42,
        43,
        || panic!("a rejected reservation must not read the clock"),
        |_, _| panic!("a rejected reservation must not submit"),
    ));
}

#[test]
fn successful_reservation_samples_once_before_single_exact_submission() {
    let calls = RefCell::new(Vec::new());
    let bytes = Cell::new(None);
    let reservation = {
        calls.borrow_mut().push("reserve");
        Some(17)
    };
    assert!(submit_reserved_exec(
        reservation,
        u32::MAX,
        u32::MAX - 1,
        || {
            calls.borrow_mut().push("clock");
            u64::MAX
        },
        |slot, event| {
            assert_eq!(slot, 17);
            calls.borrow_mut().push("submit");
            bytes.set(Some(event.to_ne_bytes()));
        },
    ));
    assert_eq!(*calls.borrow(), ["reserve", "clock", "submit"]);
    let bytes = bytes.get().unwrap();
    let decoded = ProcessEvent::decode(&bytes, u32::MAX).unwrap();
    assert_eq!(decoded.thread_id, u32::MAX - 1);
    assert_eq!(decoded.monotonic_ns, u64::MAX);
    assert_eq!(&bytes[..4], &1_u32.to_ne_bytes());
    assert_eq!(&bytes[4..8], &1_u32.to_ne_bytes());
    assert_eq!(&bytes[8..12], &u32::MAX.to_ne_bytes());
    assert_eq!(&bytes[12..16], &(u32::MAX - 1).to_ne_bytes());
    assert_eq!(&bytes[16..], &u64::MAX.to_ne_bytes());
}

#[test]
fn rejected_reservation_does_not_replace_previously_submitted_record() {
    let output = Cell::new(None);
    for (slot, timestamp) in [(Some(()), 0), (None, 1), (Some(()), 2)] {
        let before = output.get();
        let accepted = submit_reserved_exec(
            slot,
            42,
            43,
            || timestamp,
            |_, event| output.set(Some(event.to_ne_bytes())),
        );
        assert_eq!(accepted, slot.is_some());
        if accepted {
            let event = ProcessEvent::decode(&output.get().unwrap(), 42).unwrap();
            assert_eq!(event.monotonic_ns, timestamp);
        } else {
            assert_eq!(output.get(), before);
        }
    }
}
