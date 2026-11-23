#![cfg(target_os = "linux")]

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use intellicord::observe::{
    stream::Source,
    supervisor::{Outcome, SupervisedRun, supervise},
};
use tokio::{io::AsyncReadExt, net::UnixListener, process::Command, sync::oneshot, time::timeout};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn negotiated_collection_owns_clock_exchange_data_and_child_cleanup() {
    for (mode, expected) in [
        ("negotiated_ok", Outcome::Completed),
        ("negotiated_quiet", Outcome::Completed),
        ("negotiated_quiet_missing", Outcome::InvalidStream),
        ("negotiated_quiet_partial", Outcome::InvalidStream),
        ("negotiated_quiet_failure", Outcome::InvalidStream),
        ("negotiated_nonce", Outcome::InvalidStream),
        ("negotiated_partial", Outcome::InvalidStream),
        ("negotiated_failure", Outcome::InvalidStream),
        ("negotiated_eof", Outcome::Deadline),
        ("negotiated_stall", Outcome::Deadline),
    ] {
        let fixture = Fixture::new();
        let run = fixture.run_collection(mode, false).await;
        assert_eq!(run.outcome, expected, "{mode}");
        assert_eq!(run.collection.is_some(), expected == Outcome::Completed);
        if let Some(summary) = run.collection {
            let expected_records = if mode == "negotiated_quiet" { 0 } else { 2 };
            assert_eq!(summary.events.records, expected_records);
            assert_eq!(summary.collector_health.accepted, expected_records as u64);
        }
    }
    let fixture = Fixture::new();
    let run = fixture.run_collection("negotiated_stall", true).await;
    assert_eq!(run.outcome, Outcome::Cancelled);
    assert!(run.collection.is_none());
}

struct Fixture {
    directory: PathBuf,
    listener: UnixListener,
}

#[tokio::test]
async fn quiet_collection_requires_terminal_count_agreement_across_owned_pipes() {
    for (mode, expected) in [
        ("events_quiet", Outcome::Completed),
        ("events_quiet_missing", Outcome::InvalidStream),
        ("events_quiet_partial", Outcome::InvalidStream),
    ] {
        let fixture = Fixture::new();
        let run = fixture.run_collection(mode, false).await;
        assert_eq!(run.outcome, expected, "{mode}");
        assert_eq!(run.collection.is_some(), expected == Outcome::Completed);
        if let Some(summary) = run.collection {
            assert_eq!(summary.events.records, 0);
            assert_eq!(summary.events.health.accepted, 0);
            assert_eq!(summary.collector_health.ring_dropped, 3);
        }
    }
}

impl Fixture {
    async fn run_collection(
        &self,
        mode: &str,
        cancel: bool,
    ) -> intellicord::observe::supervisor::SupervisedCollection {
        use intellicord::observe::{
            collection::Collection, events::EventStream, supervisor::supervise_collection,
        };
        let mut command = Command::new("node");
        command
            .arg("--max-old-space-size=64")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/process-sensor.mjs"
            ))
            .arg(mode)
            .arg(self.directory.join("events"));
        let origin = std::time::Instant::now();
        let collection = Collection::new(
            EventStream::new(42, 0, 0, 0).unwrap(),
            Source::GeneratedReplay,
        );
        let (sender, receiver) = oneshot::channel::<()>();
        let negotiated = mode.starts_with("negotiated_");
        let mut job = tokio::spawn(async move {
            let cancellation = async {
                let _ = receiver.await;
            };
            if negotiated {
                intellicord::observe::supervisor::supervise_negotiated_collection(
                    command,
                    Source::GeneratedReplay,
                    42,
                    7,
                    Duration::from_secs(1),
                    cancellation,
                )
                .await
            } else {
                supervise_collection(
                    command,
                    collection,
                    origin,
                    Duration::from_secs(1),
                    cancellation,
                )
                .await
            }
        });
        let (socket, _) = timeout(Duration::from_secs(5), self.listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = Vec::new();
        timeout(
            Duration::from_secs(5),
            socket.take(257).read_to_end(&mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let process_id = value["pid"].as_u64().unwrap();
        if cancel {
            sender.send(()).unwrap();
        }
        let result = timeout(Duration::from_secs(5), &mut job).await;
        if result.is_err() {
            job.abort();
            panic!("dual-pipe supervisor exceeded watchdog");
        }
        let report = result.unwrap().unwrap();
        assert!(report.collector_started);
        assert!(report.collector_reaped);
        assert!(!PathBuf::from(format!("/proc/{process_id}")).exists());
        report
    }

    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "intellicord-supervisor-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let listener = UnixListener::bind(directory.join("events")).unwrap();
        Self {
            directory,
            listener,
        }
    }

    async fn run(&self, mode: &str, cancel: bool) -> (SupervisedRun, u32) {
        let mut command = Command::new("node");
        command
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/process-sensor.mjs"
            ))
            .arg(mode)
            .arg(self.directory.join("events"));
        let (sender, receiver) = oneshot::channel::<()>();
        let mut job = tokio::spawn(supervise(
            command,
            Source::GeneratedReplay,
            Duration::from_secs(1),
            async {
                let _ = receiver.await;
            },
        ));
        let (mut socket, _) = timeout(Duration::from_secs(5), self.listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = Vec::new();
        timeout(
            Duration::from_secs(5),
            (&mut socket).take(257).read_to_end(&mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
        drop(socket);
        assert!(bytes.len() <= 256);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let process_id = u32::try_from(value["pid"].as_u64().unwrap()).unwrap();
        if cancel {
            sender.send(()).unwrap();
        }
        let result = timeout(Duration::from_secs(5), &mut job).await;
        if result.is_err() {
            job.abort();
            panic!("supervisor exceeded test watchdog");
        }
        let report = result.unwrap().unwrap();
        assert!(report.collector_started);
        assert!(report.collector_reaped);
        assert!(!PathBuf::from(format!("/proc/{process_id}")).exists());
        assert!(serde_json::to_vec(&report).unwrap().len() < 2048);
        (report, process_id)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[tokio::test]
async fn joint_collection_requires_both_eofs_exit_and_matching_event_counts() {
    for (mode, outcome) in [
        ("events_complete", Outcome::Completed),
        ("events_missing", Outcome::InvalidStream),
        ("events_partial", Outcome::InvalidStream),
        ("events_foreign", Outcome::InvalidStream),
        ("events_overflow", Outcome::InvalidStream),
        ("events_failure", Outcome::InvalidStream),
        ("events_status_eof", Outcome::Deadline),
        ("events_data_eof", Outcome::Deadline),
        ("events_both_eof", Outcome::Deadline),
    ] {
        let report = Fixture::new().run_collection(mode, false).await;
        assert_eq!(report.outcome, outcome, "{mode}");
        assert_eq!(report.collection.is_some(), outcome == Outcome::Completed);
        if let Some(summary) = report.collection {
            assert_eq!(summary.events.records, 2);
            assert_eq!(summary.collector_health.accepted, 2);
        }
    }
    let report = Fixture::new().run_collection("events_hold", true).await;
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert!(report.collection.is_none());
}

#[tokio::test]
async fn generated_completion_fault_and_fragmentation_reap_owned_children() {
    for (mode, outcome) in [
        ("complete", Outcome::Completed),
        ("fragmented", Outcome::Completed),
        ("stdout_flood", Outcome::Completed),
        ("fault", Outcome::CollectorFault),
    ] {
        let (report, _) = Fixture::new().run(mode, false).await;
        assert_eq!(report.outcome, outcome, "{mode}");
        assert_eq!(report.status.unwrap().source, Source::GeneratedReplay);
        assert!(!report.status.unwrap().attachment_confirmed);
        assert!(!report.status.unwrap().detachment_confirmed);
        if outcome == Outcome::Completed {
            let health = report.terminal_health.expect("validated terminal counters");
            assert_eq!(health.accepted, 2);
            assert_eq!(health.ring_dropped, 3);
        }
    }
}

#[tokio::test]
async fn invalid_truncated_and_inconsistent_streams_fail_closed_and_reap() {
    for mode in ["invalid", "oversize", "truncated", "exit_mismatch"] {
        let (report, _) = Fixture::new().run(mode, false).await;
        assert_eq!(report.outcome, Outcome::InvalidStream, "{mode}");
        assert!(report.status.is_none());
        assert!(report.terminal_health.is_none());
    }
}

#[tokio::test]
async fn deadline_requires_exit_even_after_eof_or_a_terminal_record() {
    for mode in ["stall", "eof_stall", "terminal_stall"] {
        let (report, _) = Fixture::new().run(mode, false).await;
        assert_eq!(report.outcome, Outcome::Deadline, "{mode}");
        assert!(report.status.is_none());
        assert!(report.terminal_health.is_none());
    }
}

#[tokio::test]
async fn cancellation_after_readiness_kills_and_reaps_the_collector() {
    let (report, _) = Fixture::new().run("stall", true).await;
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert!(report.status.is_none());
}

#[test]
fn generated_production_exporter_fails_when_event_consumer_is_closed() {
    use std::{
        os::{fd::OwnedFd, unix::net::UnixStream},
        process::Stdio,
    };
    let (reader, writer) = UnixStream::pair().unwrap();
    drop(reader);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("process-collection-replay")
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("process event output rejected"));
    assert!(!error.contains("\"phase\":\"stopped\""));
}

#[test]
fn cli_supervision_replay_is_bounded_and_rejects_collector_arguments() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("supervise-replay")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 2048);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["source"], "generated_replay");
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    assert_eq!(report["quiet_collection_check"], true);
    assert_eq!(report["supervision"]["outcome"], "completed");
    assert_eq!(report["supervision"]["collector_reaped"], true);
    assert_eq!(report["collection"]["outcome"], "completed");
    assert_eq!(report["collection"]["collector_reaped"], true);
    assert_eq!(report["collection"]["collection"]["events"]["records"], 2);
    assert_eq!(report["negotiated_collection"]["outcome"], "completed");
    assert_eq!(report["negotiated_collection"]["collector_reaped"], true);
    assert_eq!(
        report["negotiated_collection"]["collection"]["events"]["records"],
        2
    );
    assert_eq!(
        report["negotiated_collection"]["collection"]["status"]["source"],
        "generated_replay"
    );
    for arguments in [
        vec!["supervise-replay", "--pid", "1"],
        vec!["supervise-replay", "--object", "/dev/null"],
        vec!["process-replay", "--status-json", "--pid", "1"],
        vec!["process-collection-replay", "--pid", "1"],
        vec!["process-collection-replay", "--object", "/dev/null"],
        vec!["process-negotiated-replay", "--pid", "1"],
        vec!["process-negotiated-replay", "--object", "/dev/null"],
    ] {
        let invalid = std::process::Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!invalid.status.success());
        assert!(invalid.stdout.is_empty());
    }
}

#[cfg(feature = "observe")]
#[tokio::test]
async fn actual_reader_clock_request_times_out_without_parent_termination() {
    use std::process::Stdio;
    use tokio::io::AsyncBufReadExt;
    let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"));
    command
        .arg("replay-negotiated")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().unwrap();
    let request_pipe = child.stdin.take().unwrap();
    let mut output = tokio::io::BufReader::new(child.stdout.take().unwrap());
    let mut ready = String::new();
    timeout(Duration::from_secs(3), output.read_line(&mut ready))
        .await
        .unwrap()
        .unwrap();
    let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(ready["record_type"], "clock_ready");
    let started = std::time::Instant::now();
    let exit = timeout(Duration::from_secs(3), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(1500));
    drop(request_pipe);
    assert!(!exit.success());
    let mut tail = Vec::new();
    output.read_to_end(&mut tail).await.unwrap();
    assert!(tail.is_empty());
}

#[cfg(feature = "observe")]
#[tokio::test]
async fn actual_reader_negotiated_replay_uses_generated_provenance_and_reaps() {
    use intellicord::observe::supervisor::supervise_negotiated_collection;
    let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"));
    command.arg("replay-negotiated");
    let run = supervise_negotiated_collection(
        command,
        Source::GeneratedReplay,
        42,
        17,
        Duration::from_secs(2),
        std::future::pending(),
    )
    .await;
    assert_eq!(run.outcome, Outcome::Completed);
    assert!(run.collector_started && run.collector_reaped);
    let summary = run.collection.unwrap();
    assert_eq!(summary.events.records, 2);
    assert!(!summary.status.attachment_confirmed && !summary.status.detachment_confirmed);
    for argument in ["--pid", "--object", "--attach"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(["replay-negotiated", argument])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"));
    command.args([
        "capture",
        "--object",
        "/dev/null",
        "--pid",
        "1",
        "--milliseconds",
        "500",
        "--acknowledge-process-observation",
        "--clock-handshake",
        "--events",
        "--status-json",
    ]);
    let run = supervise_negotiated_collection(
        command,
        Source::KernelTracepoint,
        1,
        18,
        Duration::from_secs(2),
        std::future::pending(),
    )
    .await;
    assert_eq!(run.outcome, Outcome::InvalidStream);
    assert!(run.collector_started && run.collector_reaped);
    assert!(run.collection.is_none());
}

#[cfg(feature = "observe")]
#[tokio::test]
async fn actual_reader_preload_rejection_is_a_reaped_fault_not_kernel_success() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"));
    command.args([
        "capture",
        "--object",
        "/dev/null",
        "--pid",
        "1",
        "--seconds",
        "1",
        "--acknowledge-process-observation",
        "--status-json",
    ]);
    let report = supervise(
        command,
        Source::KernelTracepoint,
        Duration::from_secs(2),
        std::future::pending(),
    )
    .await;
    assert_eq!(report.outcome, Outcome::CollectorFault);
    assert!(report.collector_reaped);
    let status = report.status.unwrap();
    assert_eq!(
        status.reason,
        intellicord::observe::lifecycle::Reason::InvalidObject
    );
    assert!(!status.attachment_confirmed);
    assert!(!status.detachment_confirmed);
}
