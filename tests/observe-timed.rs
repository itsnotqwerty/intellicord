use std::process::Command;

use intellicord::observe::{
    clock::HORIZON_NS,
    timed::{MAX_EVENT_AGE_NS, Rejection, TimedProcessMonitor},
};
use intellicord_observe_common::{EVENT_BYTES, PROCESS_EXEC, SCHEMA_VERSION};

fn record(source_ns: u64) -> Vec<u8> {
    [
        SCHEMA_VERSION.to_ne_bytes().as_slice(),
        PROCESS_EXEC.to_ne_bytes().as_slice(),
        42_u32.to_ne_bytes().as_slice(),
        43_u32.to_ne_bytes().as_slice(),
        source_ns.to_ne_bytes().as_slice(),
    ]
    .concat()
}

#[test]
fn exact_age_and_deadline_limits_are_inclusive_without_claiming_deduplication() {
    let mut monitor = TimedProcessMonitor::new(42, 0, 0, 0).unwrap();
    let bytes = record(0);
    assert_eq!(bytes.len(), EVENT_BYTES);
    let interval = monitor.accept(&bytes, MAX_EVENT_AGE_NS).unwrap();
    assert_eq!((interval.earliest_ns, interval.latest_ns), (0, 0));
    assert!(monitor.accept(&bytes, MAX_EVENT_AGE_NS).is_ok());
    assert_eq!(
        monitor.accept(&bytes, MAX_EVENT_AGE_NS + 1),
        Err(Rejection::Stale)
    );
    assert!(monitor.accept(&record(HORIZON_NS), HORIZON_NS).is_ok());
    assert_eq!(
        monitor.accept(&record(HORIZON_NS), HORIZON_NS + 1),
        Err(Rejection::AnchorExpired)
    );
    assert_eq!(monitor.health().accepted, 3);
    assert_eq!(monitor.health().rejected_stale, 1);
}

#[test]
fn uncertain_intervals_are_clipped_to_receipt_and_must_be_wholly_fresh() {
    let mut monitor = TimedProcessMonitor::new(42, 1000, 100, 120).unwrap();
    let interval = monitor.accept(&record(2000), 1100).unwrap();
    assert_eq!((interval.earliest_ns, interval.latest_ns), (1099, 1100));
    let mut wide = TimedProcessMonitor::new(42, 0, 0, 100).unwrap();
    assert_eq!(
        wide.accept(&record(0), MAX_EVENT_AGE_NS + 1),
        Err(Rejection::Stale)
    );
    assert_eq!(
        wide.accept(&record(0), HORIZON_NS + 101),
        Err(Rejection::AnchorExpired)
    );
    assert_eq!(wide.health().accepted, 0);
}

#[test]
fn generated_timed_command_has_no_target_or_activation_surface() {
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .arg("replay-timed-process")
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 2048);
    assert_eq!(output.stdout, run().stdout);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 8);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["source"], "generated_replay");
    assert_eq!(report["clock_domain"], "linux_monotonic");
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    assert_eq!(report["health"].as_object().unwrap().len(), 8);
    assert_eq!(report["health"]["accepted"], 2);
    assert_eq!(report["health"]["rejected_anchor_expired"], 1);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 9);
    assert!(cases.iter().all(|case| case["passed"] == true));
    let transport = report["transport"].as_array().unwrap();
    assert_eq!(transport.len(), 8);
    assert!(transport.iter().all(|case| case["passed"] == true));
    for argument in ["--object", "--pid", "--load", "--attach", "extra"] {
        let rejected = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["replay-timed-process", argument])
            .output()
            .unwrap();
        assert!(!rejected.status.success());
        assert!(rejected.stdout.is_empty());
    }
}
