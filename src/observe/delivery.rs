use intellicord_observe_common::ProcessEvent;
use serde::Serialize;

fn matches_exec(bytes: &[u8], child: u32, before: u64, receipt: u64) -> bool {
    before <= receipt
        && ProcessEvent::decode(bytes, child).is_some_and(|event| {
            event.thread_id == child
                && event.monotonic_ns >= before
                && event.monotonic_ns <= receipt
        })
}

#[derive(Serialize)]
pub struct Replay {
    pub schema_version: u16,
    pub source: &'static str,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub checks: Vec<super::SelectionCheck>,
}

pub fn generated_replay() -> Replay {
    let checks = [
        (
            "matching_exec",
            ProcessEvent::exec(42, 42, 15).to_ne_bytes().to_vec(),
            10,
            20,
            true,
        ),
        (
            "foreign_process",
            ProcessEvent::exec(43, 43, 15).to_ne_bytes().to_vec(),
            10,
            20,
            false,
        ),
        (
            "worker_thread",
            ProcessEvent::exec(42, 43, 15).to_ne_bytes().to_vec(),
            10,
            20,
            false,
        ),
        (
            "before_release",
            ProcessEvent::exec(42, 42, 9).to_ne_bytes().to_vec(),
            10,
            20,
            false,
        ),
        (
            "after_receipt",
            ProcessEvent::exec(42, 42, 21).to_ne_bytes().to_vec(),
            10,
            20,
            false,
        ),
        ("malformed_record", vec![0], 10, 20, false),
        (
            "backwards_bracket",
            ProcessEvent::exec(42, 42, 15).to_ne_bytes().to_vec(),
            20,
            10,
            false,
        ),
    ]
    .into_iter()
    .map(
        |(name, bytes, before, receipt, expected)| super::SelectionCheck {
            name,
            passed: matches_exec(&bytes, 42, before, receipt) == expected,
        },
    )
    .collect();
    Replay {
        schema_version: 1,
        source: "generated_replay",
        sensor_running: false,
        load_test_performed: false,
        checks,
    }
}

#[cfg(all(feature = "observe", target_os = "linux"))]
pub mod live;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controlled_exec_record_checks_are_bounded_and_sensor_free() {
        let report = generated_replay();
        assert_eq!(report.checks.len(), 7);
        for check in &report.checks {
            assert!(check.passed, "{}", check.name);
        }
        assert!(serde_json::to_string(&report).unwrap().len() < 1024);
    }

    #[test]
    fn timestamp_boundaries_are_inclusive_and_invalid_identity_is_rejected() {
        for time in [10, 20] {
            assert!(matches_exec(
                &ProcessEvent::exec(42, 42, time).to_ne_bytes(),
                42,
                10,
                20
            ));
        }
        assert!(!matches_exec(
            &ProcessEvent::exec(0, 0, 15).to_ne_bytes(),
            0,
            10,
            20
        ));
        let mut bytes = ProcessEvent::exec(42, 42, 15).to_ne_bytes();
        bytes[0..4].copy_from_slice(&2_u32.to_ne_bytes());
        assert!(!matches_exec(&bytes, 42, 10, 20));
    }
}
