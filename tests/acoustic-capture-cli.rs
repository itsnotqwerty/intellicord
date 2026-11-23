#![cfg(target_os = "linux")]

use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde::Deserialize;
use tokio::{
    io::AsyncReadExt,
    net::UnixListener,
    process::{Child, Command},
    time::timeout,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    listener: UnixListener,
}

#[derive(Deserialize)]
struct Event {
    tool: String,
    pid: u32,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "intellicord-capture-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        for tool in ["pw-dump", "pw-record"] {
            let file = directory.join(tool);
            std::fs::write(&file, include_str!("fixtures/pipewire-tool.mjs")).unwrap();
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let listener = UnixListener::bind(directory.join("events")).unwrap();
        Self {
            directory,
            listener,
        }
    }

    fn spawn(&self, mode: &str, seconds: u64) -> Child {
        self.spawn_with_status(mode, seconds, false)
    }

    fn spawn_with_status(&self, mode: &str, seconds: u64, status_json: bool) -> Child {
        let search = std::env::join_paths(std::iter::once(self.directory.clone()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord-acoustic"));
        command
            .args([
                "capture",
                "--sink-serial",
                "42",
                "--acknowledge-output-capture",
                "--seconds",
                &seconds.to_string(),
            ])
            .env("PATH", search)
            .env("INTELLICORD_FIXTURE_MODE", mode)
            .env("INTELLICORD_FIXTURE_SOCKET", self.directory.join("events"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if status_json {
            command.arg("--status-json");
        }
        command.spawn().unwrap()
    }

    async fn next_event(&self) -> Event {
        timeout(Duration::from_secs(5), async {
            let (mut socket, _) = self.listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            socket.read_to_end(&mut bytes).await.unwrap();
            serde_json::from_slice(&bytes).unwrap()
        })
        .await
        .expect("fixture did not start")
    }

    async fn event(&self, expected: &str) -> Event {
        let event = self.next_event().await;
        assert_eq!(event.tool, expected);
        event
    }

    async fn recorder_started(&self) -> (Event, Event, Event) {
        let discovery = self.event("pw-dump").await;
        let first = self.next_event().await;
        let second = self.next_event().await;
        let (recorder, continuity) = if first.tool == "pw-record" {
            (first, second)
        } else {
            (second, first)
        };
        assert_eq!(recorder.tool, "pw-record");
        assert_eq!(continuity.tool, "pw-dump");
        (discovery, recorder, continuity)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn signal(child: &Child, value: libc::c_int) {
    assert_eq!(
        unsafe { libc::kill(child.id().unwrap() as libc::pid_t, value) },
        0
    );
}

fn gone(pid: u32) {
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "fixture child {pid} survived"
    );
}

async fn completed(child: Child) -> std::process::Output {
    timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .expect("capture command did not stop")
        .unwrap()
}

fn report(output: &std::process::Output, reason: &str) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let fields: Vec<_> = report
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(fields, ["health", "schema_version", "stop_reason"]);
    assert_eq!(report["stop_reason"], reason);
    assert_eq!(report["health"]["status"]["state"], "stopped");
    assert_eq!(report["health"]["pending"]["batches"], 0);
    assert_eq!(report["health"]["processed"], report["health"]["delivered"]);
    assert!(report["health"]["high_water_batches"].as_u64().unwrap() <= 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("CAPTURE STOPPED"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("rms"));
    report
}

#[tokio::test]
async fn cancellation_during_discovery_never_starts_a_recorder() {
    for stop_signal in [libc::SIGINT, libc::SIGTERM] {
        let fixture = Fixture::new();
        let child = fixture.spawn("slow-discovery", 30);
        let discovery = fixture.event("pw-dump").await;
        signal(&child, stop_signal);
        let output = timeout(Duration::from_secs(2), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("cancelled before recorder startup")
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("STARTING CAPTURE"));
        gone(discovery.pid);
    }
}

#[tokio::test]
async fn active_signal_stop_reaps_recorder_and_returns_health_only() {
    for (stop_signal, reason) in [(libc::SIGINT, "interrupt"), (libc::SIGTERM, "terminate")] {
        let fixture = Fixture::new();
        let child = fixture.spawn("stream", 30);
        let (discovery, recorder, continuity) = fixture.recorder_started().await;
        signal(&child, stop_signal);
        report(&completed(child).await, reason);
        gone(discovery.pid);
        gone(recorder.pid);
        gone(continuity.pid);
    }
}

#[tokio::test]
async fn duration_deadline_stops_generated_stream_and_counts_delivered_frames() {
    let fixture = Fixture::new();
    let child = fixture.spawn("stream", 1);
    let (discovery, recorder, continuity) = fixture.recorder_started().await;
    let output = completed(child).await;
    let report = report(&output, "duration_limit");
    assert!(report["health"]["delivered"]["frames"].as_u64().unwrap() > 0);
    gone(discovery.pid);
    gone(recorder.pid);
    gone(continuity.pid);
}

#[tokio::test]
async fn duration_expires_during_discovery_without_starting_recorder() {
    let fixture = Fixture::new();
    let child = fixture.spawn("slow-discovery", 1);
    let discovery = fixture.event("pw-dump").await;
    let output = completed(child).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("cancelled before recorder startup: duration_limit"));
    assert!(!error.contains("STARTING CAPTURE"));
    gone(discovery.pid);
}

#[tokio::test]
async fn recorder_faults_and_stalls_reap_child_and_emit_no_success_report() {
    for (mode, expected) in [
        ("invalid", "capture processing fault"),
        ("exit", "unexpectedly"),
        ("stall", "capture timeout"),
    ] {
        let fixture = Fixture::new();
        let child = fixture.spawn(mode, 30);
        let (discovery, recorder, continuity) = fixture.recorder_started().await;
        let output = completed(child).await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "{error}");
        assert!(error.contains("CAPTURE STOPPED"));
        gone(discovery.pid);
        gone(recorder.pid);
        gone(continuity.pid);
    }
}

#[tokio::test]
async fn non_sink_source_and_missing_consent_cannot_launch_recorder() {
    let fixture = Fixture::new();
    let child = fixture.spawn("wrong-source", 30);
    let discovery = fixture.event("pw-dump").await;
    let output = completed(child).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a current output sink"));
    gone(discovery.pid);
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-acoustic"))
        .args(["capture", "--sink-serial", "42", "--seconds", "30"])
        .env("PATH", &fixture.directory)
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("explicit --acknowledge-output-capture is required")
    );
}

fn status_records(output: &std::process::Output) -> Vec<serde_json::Value> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let records: Vec<serde_json::Value> = stderr
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(!records.is_empty());
    assert!(records.len() <= 70);
    for (index, record) in records.iter().enumerate() {
        let fields: Vec<_> = record
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            fields,
            [
                "health",
                "phase",
                "reason",
                "record_type",
                "recorder_reaped",
                "schema_version",
                "sequence"
            ]
        );
        assert_eq!(record["schema_version"], 2);
        assert_eq!(record["record_type"], "capture_status");
        assert_eq!(record["sequence"], index + 1);
        if record["phase"] == "stopped" || record["phase"] == "faulted" {
            assert!(record["reason"].is_string());
        } else {
            assert!(record["reason"].is_null());
        }
        if let Some(health) = record["health"].as_object() {
            let fields: Vec<_> = health.keys().map(String::as_str).collect();
            assert_eq!(
                fields,
                [
                    "capacity_batches",
                    "delivered",
                    "discarded",
                    "high_water_batches",
                    "overflow_events",
                    "pending",
                    "processed",
                    "schema_version",
                    "status"
                ]
            );
        }
    }
    assert_eq!(records[0]["phase"], "discovering");
    assert_eq!(
        records
            .iter()
            .filter(|record| record["phase"] == "stopped" || record["phase"] == "faulted")
            .count(),
        1
    );
    assert!(!stderr.contains("STARTING CAPTURE"));
    assert!(!stderr.contains("CAPTURE REQUEST ACTIVE"));
    assert!(!stderr.contains("CAPTURE STOPPED"));
    assert!(!stderr.contains("generated-test-output"));
    records
}

#[tokio::test]
async fn json_status_distinguishes_request_byte_delivery_and_reaped_stop() {
    let fixture = Fixture::new();
    let child = fixture.spawn_with_status("stream", 1, true);
    let (discovery, recorder, continuity) = fixture.recorder_started().await;
    let output = completed(child).await;
    assert!(output.status.success());
    let final_report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(final_report["stop_reason"], "duration_limit");
    let records = status_records(&output);
    assert_eq!(records[1]["phase"], "recorder_requested");
    assert_eq!(records[2]["phase"], "awaiting_frames");
    assert_eq!(records[2]["health"]["delivered"]["frames"], 0);
    assert_eq!(records[3]["phase"], "receiving_frames");
    assert!(
        records[3]["health"]["delivered"]["frames"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(records[records.len() - 2]["phase"], "stopping");
    assert_eq!(records.last().unwrap()["phase"], "stopped");
    assert_eq!(records.last().unwrap()["reason"], "duration_limit");
    assert_eq!(records.last().unwrap()["recorder_reaped"], true);
    assert_eq!(records.last().unwrap()["health"], final_report["health"]);
    for record in &records[..records.len() - 1] {
        assert_eq!(record["recorder_reaped"], false);
    }
    gone(discovery.pid);
    gone(recorder.pid);
    gone(continuity.pid);
}

#[tokio::test]
async fn json_status_startup_cancellation_does_not_claim_recording_or_reaping() {
    let fixture = Fixture::new();
    let child = fixture.spawn_with_status("slow-discovery", 30, true);
    let discovery = fixture.event("pw-dump").await;
    signal(&child, libc::SIGTERM);
    let output = completed(child).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let records = status_records(&output);
    assert_eq!(records.len(), 2);
    assert_eq!(records[1]["phase"], "faulted");
    assert_eq!(records[1]["reason"], "terminate");
    assert_eq!(records[1]["recorder_reaped"], false);
    assert!(records[1]["health"].is_null());
    gone(discovery.pid);
}

#[tokio::test]
async fn json_status_fault_preserves_processing_reason_after_child_cleanup() {
    let fixture = Fixture::new();
    let child = fixture.spawn_with_status("invalid", 30, true);
    let (discovery, recorder, continuity) = fixture.recorder_started().await;
    let output = completed(child).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let records = status_records(&output);
    assert!(
        !records
            .iter()
            .any(|record| record["phase"] == "receiving_frames")
    );
    let last = records.last().unwrap();
    assert_eq!(last["phase"], "faulted");
    assert_eq!(last["reason"], "processing_failed");
    assert_eq!(last["recorder_reaped"], true);
    assert_eq!(last["health"]["status"]["error"], "invalid_sample");
    assert_eq!(last["health"]["processed"]["batches"], 0);
    gone(discovery.pid);
    gone(recorder.pid);
    gone(continuity.pid);
}

#[tokio::test]
async fn closed_json_status_consumer_fails_before_tool_startup() {
    let fixture = Fixture::new();
    let mut child = fixture.spawn_with_status("stream", 30, true);
    drop(child.stderr.take());
    let output = completed(child).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn closing_active_json_status_consumer_stops_and_reaps_recorder() {
    use tokio::io::AsyncBufReadExt;

    let fixture = Fixture::new();
    let mut child = fixture.spawn_with_status("stream", 30, true);
    let mut lines = tokio::io::BufReader::new(child.stderr.take().unwrap()).lines();
    let (discovery, recorder, continuity) = fixture.recorder_started().await;
    loop {
        let line = tokio::time::timeout(Duration::from_secs(3), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let record: serde_json::Value = serde_json::from_str(&line).unwrap();
        if record["phase"] == "receiving_frames" {
            break;
        }
    }
    drop(lines);
    let output = completed(child).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    gone(discovery.pid);
    gone(recorder.pid);
    gone(continuity.pid);
}

#[tokio::test]
async fn json_status_classifies_active_signals_without_changing_success_reports() {
    for (stop_signal, expected) in [(libc::SIGINT, "interrupt"), (libc::SIGTERM, "terminate")] {
        let fixture = Fixture::new();
        let child = fixture.spawn_with_status("stream", 30, true);
        let (discovery, recorder, continuity) = fixture.recorder_started().await;
        signal(&child, stop_signal);
        let output = completed(child).await;
        assert!(output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["stop_reason"], expected);
        let records = status_records(&output);
        let last = records.last().unwrap();
        assert_eq!(last["phase"], "stopped");
        assert_eq!(last["reason"], expected);
        assert_eq!(last["recorder_reaped"], true);
        gone(discovery.pid);
        gone(recorder.pid);
        gone(continuity.pid);
    }
}

#[tokio::test]
async fn json_status_classifies_startup_failures_without_recorder_confirmation() {
    for (mode, seconds, expected) in [
        ("wrong-source", 30, "source_rejected"),
        ("invalid-metadata", 30, "discovery_failed"),
        ("slow-discovery", 30, "discovery_failed"),
        ("slow-discovery", 1, "duration_limit"),
    ] {
        let fixture = Fixture::new();
        let child = fixture.spawn_with_status(mode, seconds, true);
        let discovery = fixture.event("pw-dump").await;
        let output = completed(child).await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let records = status_records(&output);
        assert_eq!(records.len(), 2);
        let last = records.last().unwrap();
        assert_eq!(last["phase"], "faulted");
        assert_eq!(last["reason"], expected);
        assert_eq!(last["recorder_reaped"], false);
        assert!(last["health"].is_null());
        gone(discovery.pid);
    }
}

#[tokio::test]
async fn json_status_rejects_source_replacement_and_confirms_recorder_cleanup() {
    for mode in ["source-replaced", "source-disappeared"] {
        let fixture = Fixture::new();
        let child = fixture.spawn_with_status(mode, 30, true);
        let (discovery, recorder, continuity) = fixture.recorder_started().await;
        let output = completed(child).await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let records = status_records(&output);
        let last = records.last().unwrap();
        assert_eq!(last["phase"], "faulted");
        assert_eq!(last["reason"], "source_rejected");
        assert_eq!(last["recorder_reaped"], true);
        assert!(last["health"].is_null());
        gone(discovery.pid);
        gone(recorder.pid);
        gone(continuity.pid);
    }
}

#[tokio::test]
async fn json_status_classifies_stall_eof_and_unexpected_exit_after_cleanup() {
    for (mode, expected) in [
        ("stall", &["input_stalled"][..]),
        ("eof", &["stream_ended"][..]),
        ("exit", &["stream_ended", "recorder_exited"][..]),
    ] {
        let fixture = Fixture::new();
        let child = fixture.spawn_with_status(mode, 30, true);
        let (discovery, recorder, continuity) = fixture.recorder_started().await;
        let output = completed(child).await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let records = status_records(&output);
        let last = records.last().unwrap();
        assert_eq!(last["phase"], "faulted");
        assert!(
            expected.contains(&last["reason"].as_str().unwrap()),
            "{mode}: {last}"
        );
        assert_eq!(last["recorder_reaped"], true);
        assert_eq!(last["health"]["status"]["error"]["producer"], "source_lost");
        gone(discovery.pid);
        gone(recorder.pid);
        gone(continuity.pid);
    }
}

#[tokio::test]
async fn json_status_classifies_missing_discovery_and_recorder_tools() {
    let node = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|directory| directory.join("node"))
        .find(|candidate| candidate.is_file())
        .expect("Node.js fixture dependency");
    for (tool, expected) in [
        ("pw-dump", "discovery_failed"),
        ("pw-record", "recorder_start_failed"),
    ] {
        let fixture = Fixture::new();
        std::fs::remove_file(fixture.directory.join(tool)).unwrap();
        std::os::unix::fs::symlink(&node, fixture.directory.join("node")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord-acoustic"));
        command
            .args([
                "capture",
                "--sink-serial",
                "42",
                "--acknowledge-output-capture",
                "--seconds",
                "30",
                "--status-json",
            ])
            .env("PATH", &fixture.directory)
            .env("INTELLICORD_FIXTURE_MODE", "stream")
            .env(
                "INTELLICORD_FIXTURE_SOCKET",
                fixture.directory.join("events"),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = command.spawn().unwrap();
        let discovery = if tool == "pw-record" {
            Some(fixture.event("pw-dump").await)
        } else {
            None
        };
        let output = completed(child).await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let records = status_records(&output);
        let last = records.last().unwrap();
        assert_eq!(last["reason"], expected);
        assert_eq!(last["phase"], "faulted");
        assert_eq!(last["recorder_reaped"], false);
        if let Some(discovery) = discovery {
            gone(discovery.pid);
        }
    }
}
