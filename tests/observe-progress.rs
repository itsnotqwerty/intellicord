use std::process::Command;

use intellicord::observe::{progress::ProgressStream, stream::Source};

#[test]
fn periodic_replay_observes_generated_health_and_revokes_after_reaping() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("supervise-periodic-progress-replay")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 2048);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 8);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["source"], "generated_replay");
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    for field in ["observed_health", "generated_only", "health_revoked"] {
        assert_eq!(report[field], true, "{field}");
    }
    let run = &report["run"];
    assert_eq!(run["outcome"], "completed");
    assert_eq!(run["collector_started"], true);
    assert_eq!(run["collector_reaped"], true);
    assert!(run["deadline"].is_null());
    assert_eq!(run["summary"]["records"], 8);
    assert_eq!(run["summary"]["health_samples"], 2);
    assert_eq!(run["summary"]["lifecycle"]["source"], "generated_replay");
    assert_eq!(run["summary"]["lifecycle"]["attachment_confirmed"], false);
    assert_eq!(run["summary"]["lifecycle"]["detachment_confirmed"], false);
    for command in [
        "supervise-periodic-progress-replay",
        "periodic-progress-replay-child",
    ] {
        for extra in ["--session", "--pid", "--object", "--attach", "extra"] {
            let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
                .args([command, extra])
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
    }
}

#[tokio::test]
async fn periodic_child_sample_can_trigger_cancellation_and_foreign_session_fails_closed() {
    use intellicord::observe::{
        progress::supervisor::{health_channel, supervise_with_health},
        supervisor::Outcome,
    };
    use std::time::Duration;
    let command = || {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_intellicord"));
        command.arg("periodic-progress-replay-child");
        command
    };
    let (sender, mut receiver) = health_channel();
    let retained = receiver.clone();
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    let owner = supervise_with_health(
        command(),
        "00000000000000000000000000000005",
        Source::GeneratedReplay,
        Duration::from_secs(6),
        Duration::from_secs(3),
        async {
            let _ = cancelled.await;
        },
        sender,
    );
    let consumer = async move {
        let started = tokio::time::Instant::now();
        loop {
            if receiver.changed().await.is_err() {
                return false;
            }
            if let Some(sample) = receiver.latest(Duration::from_secs(2)) {
                assert_eq!(sample.source, Source::GeneratedReplay);
                assert!((1..=2).contains(&sample.health.accepted));
                assert!(started.elapsed() >= Duration::from_millis(900));
                cancel.send(()).unwrap();
                return true;
            }
        }
    };
    let (run, observed) = tokio::join!(owner, consumer);
    assert!(observed);
    assert_eq!(run.outcome, Outcome::Cancelled);
    assert!(run.collector_reaped);
    assert!(run.summary.is_none());
    assert!(retained.latest(Duration::MAX).is_none());

    let (sender, receiver) = health_channel();
    let run = supervise_with_health(
        command(),
        "00000000000000000000000000000006",
        Source::GeneratedReplay,
        Duration::from_secs(6),
        Duration::from_secs(3),
        std::future::pending(),
        sender,
    )
    .await;
    assert_eq!(run.outcome, Outcome::InvalidStream);
    assert!(run.collector_reaped);
    assert!(run.summary.is_none());
    assert!(receiver.latest(Duration::MAX).is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn generated_child_output_failure_never_reports_success() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("progress-replay-child")
        .stdout(
            std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("progress output failed"));
}

#[test]
fn owning_progress_replay_reports_awaited_cleanup_and_generated_provenance() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("supervise-progress-replay")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 5);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["source"], "generated_replay");
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    let run = &report["run"];
    assert_eq!(run.as_object().unwrap().len(), 6);
    assert_eq!(run["outcome"], "completed");
    assert_eq!(run["collector_started"], true);
    assert_eq!(run["collector_reaped"], true);
    assert!(run["deadline"].is_null());
    assert_eq!(run["summary"]["records"], 8);
    assert_eq!(run["summary"]["health_samples"], 2);
    assert_eq!(run["summary"]["lifecycle"]["source"], "generated_replay");
    assert_eq!(run["summary"]["lifecycle"]["attachment_confirmed"], false);
    assert_eq!(run["summary"]["lifecycle"]["detachment_confirmed"], false);
    for argument in ["--pid", "--session", "--object", "--attach", "extra"] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["supervise-progress-replay", argument])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn progress_diagnostic_is_bounded_generated_and_fixed_argument() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("replay-process-progress")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report,
        serde_json::to_value(intellicord::observe::progress::generated_replay()).unwrap()
    );
    assert_eq!(report["source"], "generated_replay");
    assert_eq!(report["sensor_running"], false);
    let checks = report["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 8);
    assert!(checks.iter().all(|check| check["passed"] == true));
    for argument in ["--pid", "--session", "--object", "--attach", "extra"] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["replay-process-progress", argument])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn generated_child_progress_round_trip_requires_observed_exit_and_eof() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("progress-replay-child")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 8192);
    assert_eq!(
        output.stdout,
        intellicord::observe::progress::generated_stream()
    );
    let mut stream =
        ProgressStream::new("00000000000000000000000000000001", Source::GeneratedReplay).unwrap();
    for fragment in output.stdout.chunks(13) {
        stream.push(fragment, 100).unwrap();
    }
    let result = stream.finish(output.status.success()).unwrap();
    assert_eq!(result.records, 8);
    assert_eq!(result.health_samples, 2);
    assert_eq!(result.lifecycle.records, 6);
    assert!(!result.lifecycle.attachment_confirmed);
    assert!(!result.lifecycle.detachment_confirmed);
    for argument in ["--pid", "--session", "--object", "--attach", "extra"] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["progress-replay-child", argument])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
