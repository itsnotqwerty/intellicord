#![cfg(feature = "observe")]

use std::process::Command;

#[test]
fn final_progress_reports_are_evaluated_offline_without_identity_output() {
    use std::{io::Write, process::Stdio};

    let report = serde_json::json!({
        "schema_version":1,
        "source":"supervised_kernel_socket_progress",
        "transport":"kernel_verified_unix_socket",
        "session_binding":"parent_selected",
        "endpoint_removed":true,
        "run":{"schema_version":1,"outcome":"completed","collector_started":true,
            "collector_reaped":true,"deadline":null,"summary":{"schema_version":1,
            "records":10,"health_samples":4,
            "lifecycle":{"schema_version":1,"source":"kernel_tracepoint","records":6,
                "terminal_phase":"stopped","reason":"duration_limit",
                "attachment_confirmed":true,"detachment_confirmed":true},
            "terminal_health":{"schema_version":1,"accepted":7,"rejected_record":1,
                "rejected_clock":2,"ring_dropped":3}}}
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args(["evaluate-progress-reports", "--seconds", "5"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{report}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let evaluation: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(evaluation["source"], "offline_progress_report_evaluation");
    assert_eq!(evaluation["runs"], 1);
    assert_eq!(evaluation["health_samples"], 4);
    assert_eq!(evaluation["totals"]["ring_dropped"], 3);
    assert_eq!(evaluation["cleanup_confirmed"], true);
    let output = String::from_utf8(output.stdout).unwrap();
    for excluded in ["session", "pid", "socket", "path", "timestamp"] {
        assert!(!output.contains(excluded), "{excluded}");
    }

    let rejected = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args(["evaluate-progress-reports", "--seconds", "5", "extra"])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
}

#[cfg(target_os = "linux")]
fn progress_arguments() -> Vec<&'static str> {
    vec![
        "capture-progress",
        "--object",
        "/dev/null",
        "--pid",
        "1",
        "--seconds",
        "1",
        "--session",
        "fedcba9876543210fedcba9876543210",
        "--acknowledge-process-progress",
    ]
}

#[cfg(target_os = "linux")]
#[test]
fn owning_progress_capture_requires_distinct_consent_and_no_session_override() {
    let base = vec![
        "supervise-progress-capture",
        "--object",
        "/dev/null",
        "--pid",
        "1",
        "--seconds",
        "1",
        "--acknowledge-supervised-progress",
    ];
    let mut cases = vec![vec!["supervise-progress-capture"], base[..7].to_vec()];
    for (index, replacement) in [
        (2, ""),
        (4, "0"),
        (4, "2147483648"),
        (4, "not-a-pid"),
        (6, "0"),
        (6, "61"),
        (6, "18446744073709551615"),
        (7, "--acknowledge-process-progress"),
        (7, "--acknowledge-process-observation"),
        (7, "--acknowledge-supervised-observation"),
    ] {
        let mut arguments = base.clone();
        arguments[index] = replacement;
        cases.push(arguments);
    }
    for extra in [
        vec!["--session", "fedcba9876543210fedcba9876543210"],
        vec!["--executable", "/bin/true"],
        vec!["--events"],
        vec!["--status-json"],
        vec!["--clock-handshake"],
        vec!["--idle", "60"],
        vec!["extra"],
    ] {
        let mut arguments = base.clone();
        arguments.extend(extra);
        cases.push(arguments);
    }
    for arguments in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(&arguments)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owning_progress_capture_returns_only_final_reaped_preload_fault() {
    for seconds in ["1", "60"] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args([
                "supervise-progress-capture",
                "--object",
                "/dev/null",
                "--pid",
                "1",
                "--seconds",
                seconds,
                "--acknowledge-supervised-progress",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.len() < 2048);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report.as_object().unwrap().len(), 4);
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["source"], "supervised_kernel_progress");
        assert_eq!(report["session_binding"], "parent_selected");
        let run = &report["run"];
        assert_eq!(run.as_object().unwrap().len(), 6);
        assert_eq!(run["schema_version"], 1);
        assert_eq!(run["outcome"], "collector_fault");
        assert_eq!(run["collector_started"], true);
        assert_eq!(run["collector_reaped"], true);
        assert!(run["deadline"].is_null());
        assert_eq!(run["summary"]["records"], 2);
        assert_eq!(run["summary"]["health_samples"], 0);
        assert_eq!(run["summary"]["lifecycle"]["source"], "kernel_tracepoint");
        assert_eq!(run["summary"]["lifecycle"]["terminal_phase"], "faulted");
        assert_eq!(run["summary"]["lifecycle"]["reason"], "invalid_object");
        assert_eq!(run["summary"]["lifecycle"]["attachment_confirmed"], false);
        assert_eq!(run["summary"]["lifecycle"]["detachment_confirmed"], false);
        let stdout = String::from_utf8(output.stdout).unwrap();
        for excluded in [
            "/dev/null",
            "\"pid\"",
            "\"session\"",
            "\"payload\"",
            "\"sensor_running\"",
        ] {
            assert!(!stdout.contains(excluded), "{excluded}");
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn progress_capture_gates_precede_output_and_object_access() {
    let base = progress_arguments();
    let mut cases = vec![vec!["capture-progress"], base[..9].to_vec()];
    for (index, replacement) in [
        (2, ""),
        (4, "0"),
        (4, "2147483648"),
        (6, "0"),
        (6, "61"),
        (8, "bad"),
        (8, "0000000000000000000000000000000g"),
        (9, "--acknowledge-process-observation"),
        (9, "--acknowledge-supervised-observation"),
    ] {
        let mut arguments = base.clone();
        arguments[index] = replacement;
        cases.push(arguments);
    }
    for extra in [
        "--events",
        "--status-json",
        "--clock-handshake",
        "--attach",
        "extra",
    ] {
        let mut arguments = base.clone();
        arguments.push(extra);
        cases.push(arguments);
    }
    for arguments in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(&arguments)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn progress_capture_output_failure_precedes_sensor_setup() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args(progress_arguments())
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

#[cfg(target_os = "linux")]
#[test]
fn progress_capture_preload_fault_is_session_bound_without_summary_contamination() {
    use intellicord::observe::{progress::ProgressStream, stream::Source};
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args(progress_arguments())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.len() < 4096);
    let mut stream =
        ProgressStream::new("fedcba9876543210fedcba9876543210", Source::KernelTracepoint).unwrap();
    for chunk in output.stdout.chunks(7) {
        stream.push(chunk, 100).unwrap();
    }
    let report = stream.finish(output.status.success()).unwrap();
    assert_eq!(report.records, 2);
    assert_eq!(report.health_samples, 0);
    assert_eq!(
        report.lifecycle.reason,
        intellicord::observe::lifecycle::Reason::InvalidObject
    );
    assert!(!report.lifecycle.attachment_confirmed && !report.lifecycle.detachment_confirmed);
    let mut foreign =
        ProgressStream::new("00000000000000000000000000000001", Source::KernelTracepoint).unwrap();
    assert!(foreign.push(&output.stdout, 100).is_err());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn progress_capture_preload_fault_is_reaped_by_progress_owner() {
    use intellicord::observe::{
        progress::supervisor::{health_channel, supervise_with_health},
        stream::Source,
        supervisor::Outcome,
    };
    use std::time::Duration;
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_intellicord-observe"));
    command.args(progress_arguments());
    let (sender, receiver) = health_channel();
    let report = supervise_with_health(
        command,
        "fedcba9876543210fedcba9876543210",
        Source::KernelTracepoint,
        Duration::from_secs(3),
        Duration::from_secs(2),
        std::future::pending(),
        sender,
    )
    .await;
    assert_eq!(report.outcome, Outcome::CollectorFault);
    assert!(report.collector_started && report.collector_reaped);
    let summary = report.summary.unwrap();
    assert_eq!(summary.records, 2);
    assert_eq!(summary.health_samples, 0);
    assert!(!summary.lifecycle.attachment_confirmed && !summary.lifecycle.detachment_confirmed);
    assert!(receiver.latest(Duration::MAX).is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn supervised_capture_requires_own_consent_and_preserves_preload_fault() {
    let base = [
        "supervise-capture",
        "--object",
        "/dev/null",
        "--pid",
        "1",
        "--seconds",
        "1",
    ];
    for ending in [
        vec![],
        vec!["--acknowledge-process-observation"],
        vec!["--acknowledge-bpf-delivery"],
        vec!["--acknowledge-supervised-observation", "--events"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(base)
            .args(ending)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args(base)
        .arg("--acknowledge-supervised-observation")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["source"], "supervised_kernel_capture");
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["run"]["outcome"], "collector_fault");
    assert_eq!(report["run"]["collector_reaped"], true);
    assert_eq!(report["run"]["status"]["reason"], "invalid_object");
    assert_eq!(report["run"]["status"]["attachment_confirmed"], false);
    assert!(report["run"]["terminal_health"].is_null());
}

#[cfg(target_os = "linux")]
#[test]
fn delivery_probe_requires_distinct_consent_and_no_external_target() {
    for arguments in [
        vec!["verify-delivery"],
        vec!["verify-delivery", "--object", "/dev/null"],
        vec![
            "verify-delivery",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-load",
        ],
        vec![
            "verify-delivery",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-attach",
        ],
        vec![
            "verify-delivery",
            "--object",
            "",
            "--acknowledge-bpf-delivery",
        ],
        vec![
            "verify-delivery",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-delivery",
            "--pid",
            "1",
        ],
        vec![
            "verify-delivery",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-delivery",
            "--executable",
            "/bin/true",
        ],
        vec![
            "verify-delivery",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-delivery",
            "--status-json",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args([
            "verify-delivery",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-delivery",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.stdout.len() < 1024);
    assert_eq!(report.as_object().unwrap().len(), 12);
    assert_eq!(report["outcome"], "object_unreadable");
    assert_eq!(report["source"], "explicit_kernel_delivery_probe");
    for field in [
        "program_loaded",
        "child_started",
        "child_reaped",
        "attachment_confirmed",
        "detachment_confirmed",
        "event_verified",
        "target_cleared",
        "sensor_running",
    ] {
        assert_eq!(report[field], false);
    }
    assert_eq!(report["owned_handles_dropped"], true);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn controlled_delivery_child_expires_while_release_pipe_stays_open() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    use tokio::{io::AsyncReadExt, process::Command, time::timeout};
    let mut child = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .arg("delivery-child")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let mut ready = [0];
    timeout(Duration::from_secs(2), output.read_exact(&mut ready))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready, *b"R");
    let started = Instant::now();
    assert!(
        !timeout(Duration::from_secs(7), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert!(started.elapsed() >= Duration::from_secs(4));
    drop(input);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn controlled_delivery_child_waits_for_release_and_execs_with_same_pid() {
    use std::{process::Stdio, time::Duration};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        process::Command,
        time::timeout,
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .arg("delivery-child")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let process_id = child.id().unwrap();
    let mut output = child.stdout.take().unwrap();
    let mut ready = [0];
    timeout(Duration::from_secs(2), output.read_exact(&mut ready))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready, *b"R");
    assert!(child.try_wait().unwrap().is_none());
    let mut input = child.stdin.take().unwrap();
    input.write_all(b"X").await.unwrap();
    drop(input);
    assert!(
        timeout(Duration::from_secs(2), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert_eq!(unsafe { libc::kill(process_id as i32, 0) }, -1);
    for token in [b"!".as_slice(), b"".as_slice()] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .arg("delivery-child")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        input.write_all(token).await.unwrap();
        drop(input);
        assert!(
            !timeout(Duration::from_secs(2), child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn attach_only_probe_requires_distinct_consent_and_no_target_or_capture_arguments() {
    for arguments in [
        vec!["verify-attach"],
        vec!["verify-attach", "--object", "/dev/null"],
        vec![
            "verify-attach",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-load",
        ],
        vec![
            "verify-attach",
            "--object",
            "/dev/null",
            "--acknowledge-process-observation",
        ],
        vec!["verify-attach", "--object", "", "--acknowledge-bpf-attach"],
        vec![
            "verify-attach",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-attach",
            "--pid",
            "1",
        ],
        vec![
            "verify-attach",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-attach",
            "--events",
        ],
        vec![
            "verify-attach",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-attach",
            "--status-json",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args([
            "verify-attach",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-attach",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.len() < 768);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["source"], "explicit_kernel_attach_probe");
    assert_eq!(report["outcome"], "object_unreadable");
    for field in [
        "map_setup_attempted",
        "program_load_attempted",
        "program_loaded",
        "target_disabled_confirmed",
        "attachment_attempted",
        "attachment_confirmed",
        "detachment_attempted",
        "detachment_confirmed",
        "sensor_running",
    ] {
        assert_eq!(report[field], false);
    }
    assert_eq!(report["owned_handles_dropped"], true);
}

#[cfg(target_os = "linux")]
#[test]
fn load_only_probe_requires_its_own_acknowledgement_and_no_activation_arguments() {
    for arguments in [
        vec!["verify-load"],
        vec!["verify-load", "--object", "/dev/null"],
        vec![
            "verify-load",
            "--object",
            "/dev/null",
            "--acknowledge-process-observation",
        ],
        vec!["verify-load", "--object", "", "--acknowledge-bpf-load"],
        vec![
            "verify-load",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-load",
            "--pid",
            "1",
        ],
        vec![
            "verify-load",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-load",
            "--attach",
        ],
        vec![
            "verify-load",
            "--object",
            "/dev/null",
            "--acknowledge-bpf-load",
            "--status-json",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn load_only_probe_rejects_bad_files_before_any_kernel_operation() {
    let path =
        std::env::temp_dir().join(format!("intellicord-invalid-probe-{}", std::process::id()));
    struct Remove(std::path::PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _remove = Remove(path.clone());
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    std::io::Write::write_all(&mut file, b"not an ELF").unwrap();
    for (input, outcome) in [
        (std::path::Path::new("/dev/null"), "object_unreadable"),
        (path.as_path(), "object_rejected"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .arg("verify-load")
            .arg("--object")
            .arg(input)
            .arg("--acknowledge-bpf-load")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.len() < 512);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["source"], "explicit_kernel_load_probe");
        assert_eq!(report["outcome"], outcome);
        for field in [
            "map_setup_attempted",
            "program_load_attempted",
            "program_loaded",
            "attachment_attempted",
            "sensor_running",
        ] {
            assert_eq!(report[field], false);
        }
        assert_eq!(report["owned_handles_dropped"], true);
        assert!(!String::from_utf8_lossy(&output.stdout).contains(path.to_str().unwrap()));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn negotiated_export_requires_short_explicit_duration_and_rejects_objects_before_preamble() {
    let make = |duration_flag: &str, duration: &str, consent: bool, trailing: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"));
        command.args([
            "capture",
            "--object",
            "/dev/null",
            "--pid",
            "1",
            duration_flag,
            duration,
        ]);
        if consent {
            command.arg("--acknowledge-process-observation");
        }
        command.args(trailing).output().unwrap()
    };
    let flags = ["--clock-handshake", "--events", "--status-json"];
    for duration in ["1", "500"] {
        let output = make("--milliseconds", duration, true, &flags);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let records: Vec<serde_json::Value> = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1]["reason"], "invalid_object");
        assert_eq!(records[1]["attachment_confirmed"], false);
    }
    for (duration_flag, duration, consent, trailing) in [
        ("--seconds", "1", true, flags.as_slice()),
        ("--milliseconds", "0", true, flags.as_slice()),
        ("--milliseconds", "501", true, flags.as_slice()),
        ("--milliseconds", "1", false, flags.as_slice()),
        (
            "--milliseconds",
            "1",
            true,
            &["--clock-handshake", "--status-json"],
        ),
        (
            "--milliseconds",
            "1",
            true,
            &["--events", "--clock-handshake", "--status-json"],
        ),
        (
            "--milliseconds",
            "1",
            true,
            &[
                "--clock-handshake",
                "--clock-handshake",
                "--events",
                "--status-json",
            ],
        ),
    ] {
        let output = make(duration_flag, duration, consent, trailing);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let records: Vec<serde_json::Value> = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.last().unwrap()["reason"], "invalid_request");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn event_export_requires_status_and_consent_and_rejects_bad_objects_before_bpf() {
    let base = [
        "capture",
        "--object",
        "/dev/null",
        "--pid",
        "1",
        "--seconds",
        "1",
        "--acknowledge-process-observation",
    ];
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args(base)
        .args(["--events", "--status-json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let records: Vec<serde_json::Value> = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1]["reason"], "invalid_object");
    assert_eq!(records[1]["attachment_confirmed"], false);
    for args in [
        base.iter().copied().chain(["--events"]).collect::<Vec<_>>(),
        base[..7]
            .iter()
            .copied()
            .chain(["--events", "--status-json"])
            .collect(),
        base.iter()
            .copied()
            .chain(["--events", "--events", "--status-json"])
            .collect(),
        base.iter()
            .copied()
            .chain(["--status-json", "--events"])
            .collect(),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn offline_event_inspector_accepts_complete_native_records_and_rejects_bad_streams() {
    use std::{io::Write, process::Stdio};
    let record = |process_id: u32, time: u64| {
        [
            1_u32.to_ne_bytes().as_slice(),
            1_u32.to_ne_bytes().as_slice(),
            process_id.to_ne_bytes().as_slice(),
            process_id.to_ne_bytes().as_slice(),
            time.to_ne_bytes().as_slice(),
        ]
        .concat()
    };
    let inspect = |bytes: &[u8], receipt: &str, exit: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args([
                "inspect-events",
                "--pid",
                "42",
                "--source-anchor",
                "1000",
                "--before",
                "100",
                "--after",
                "120",
                "--receipt",
                receipt,
                "--exit",
                exit,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        child.wait_with_output().unwrap()
    };
    let valid = [record(42, 2000), record(42, 2100)].concat();
    let output = inspect(&valid, "1300", "success");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 5);
    assert_eq!(report["source"], "event_stream_validation");
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    assert_eq!(report["report"]["records"], 2);
    assert_eq!(report["report"]["health"]["accepted"], 2);
    assert_eq!(report["report"].as_object().unwrap().len(), 3);
    for (bytes, receipt, exit) in [
        (valid[..47].to_vec(), "1300", "success"),
        (vec![], "1300", "success"),
        (record(43, 2000), "1300", "success"),
        (valid.clone(), "1300", "failure"),
        (valid.clone(), "1000000121", "success"),
        (valid.clone(), "250001300", "success"),
        (record(42, 0), "1300", "success"),
        (record(42, 2000).repeat(257), "1300", "success"),
    ] {
        let output = inspect(&bytes, receipt, exit);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    for arguments in [
        vec!["inspect-events"],
        vec!["inspect-events", "--attach"],
        vec!["inspect-events", "--object", "/dev/null"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

fn inspect_status(bytes: &[u8], source: &str, exit: &str) -> std::process::Output {
    use std::{io::Write, process::Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args(["inspect-status", "--source", source, "--exit", exit])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn offline_status_inspection_validates_real_cli_replay_and_rejects_bad_streams() {
    let replay = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .arg("replay-lifecycle")
        .output()
        .unwrap();
    assert!(replay.status.success());
    let valid = inspect_status(&replay.stdout, "generated_replay", "success");
    assert!(valid.status.success());
    assert!(valid.stderr.is_empty());
    assert!(valid.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&valid.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 5);
    assert_eq!(report["source"], "status_stream_validation");
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    assert_eq!(report["report"]["terminal_phase"], "stopped");
    assert_eq!(report["report"]["records"], 6);
    assert_eq!(report["report"]["attachment_confirmed"], false);
    for (bytes, source, exit) in [
        (&replay.stdout[..], "kernel_tracepoint", "success"),
        (&replay.stdout[..], "generated_replay", "failure"),
        (
            &replay.stdout[..replay.stdout.len() - 1],
            "generated_replay",
            "success",
        ),
        (&b""[..], "generated_replay", "success"),
        (&b"{}\n"[..], "generated_replay", "success"),
    ] {
        let invalid = inspect_status(bytes, source, exit);
        assert!(!invalid.status.success());
        assert!(invalid.stdout.is_empty());
    }
    for arguments in [
        vec!["inspect-status"],
        vec!["inspect-status", "--source", "unknown", "--exit", "success"],
        vec![
            "inspect-status",
            "--source",
            "generated_replay",
            "--exit",
            "unknown",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn lifecycle_replay_is_deterministic_bounded_ndjson_without_kernel_claims() {
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .arg("replay-lifecycle")
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(run().stdout, output.stdout);
    let records: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            assert!(line.len() < 2048);
            serde_json::from_str(line).unwrap()
        })
        .collect();
    assert_eq!(records.len(), 6);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record.as_object().unwrap().len(), 9);
        assert_eq!(record["sequence"], index + 1);
        assert_eq!(record["record_type"], "process_status");
        assert_eq!(record["source"], "generated_replay");
        assert_eq!(record["attachment_confirmed"], false);
        assert_eq!(record["detachment_confirmed"], false);
    }
    assert_eq!(records[5]["phase"], "stopped");
}

#[cfg(target_os = "linux")]
#[test]
fn json_startup_rejection_has_typed_terminal_status_without_loading_bpf() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args([
            "capture",
            "--object",
            "/dev/null",
            "--pid",
            "1",
            "--seconds",
            "1",
            "--acknowledge-process-observation",
            "--status-json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let records: Vec<serde_json::Value> = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["phase"], "validating");
    assert_eq!(records[1]["sequence"], 2);
    assert_eq!(records[1]["phase"], "faulted");
    assert_eq!(records[1]["reason"], "invalid_object");
    assert_eq!(records[1]["attachment_confirmed"], false);
    assert_eq!(records[1]["detachment_confirmed"], false);
    assert_eq!(records[1].as_object().unwrap().len(), 9);
    let bytes = records
        .iter()
        .flat_map(|record| {
            let mut line = serde_json::to_vec(record).unwrap();
            line.push(b'\n');
            line
        })
        .collect::<Vec<_>>();
    let inspected = inspect_status(&bytes, "kernel_tracepoint", "failure");
    assert!(inspected.status.success());
    assert!(inspected.stderr.is_empty());
    let summary: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(summary["report"]["terminal_phase"], "faulted");
    assert_eq!(summary["report"]["reason"], "invalid_object");
    assert_eq!(summary["report"]["attachment_confirmed"], false);
}

#[cfg(target_os = "linux")]
#[test]
fn json_invalid_requests_report_faults_without_acknowledging_observation() {
    for arguments in [
        vec!["capture", "--status-json"],
        vec![
            "capture",
            "--object",
            "/dev/null",
            "--pid",
            "1",
            "--seconds",
            "1",
            "--status-json",
        ],
        vec![
            "capture",
            "--object",
            "/dev/null",
            "--pid",
            "0",
            "--seconds",
            "1",
            "--acknowledge-process-observation",
            "--status-json",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let records: Vec<serde_json::Value> = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1]["reason"], "invalid_request");
        assert_eq!(records[1]["phase"], "faulted");
        assert_eq!(records[1]["attachment_confirmed"], false);
        assert_eq!(records[1]["detachment_confirmed"], false);
    }
}

#[test]
fn target_probe_reports_capability_without_selecting_or_identifying_a_target() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .arg("probe-target")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 6);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["source"], "self_pidfd_probe");
    assert_eq!(report["target_selected"], false);
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    #[cfg(target_os = "linux")]
    assert!(["available", "unavailable"].contains(&report["status"].as_str().unwrap()));
    #[cfg(not(target_os = "linux"))]
    assert_eq!(report["status"], "unsupported");
    for arguments in [
        vec!["probe-target", "extra"],
        vec!["probe-target", "--pid", "1"],
    ] {
        let invalid = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!invalid.status.success());
        assert!(invalid.stdout.is_empty());
    }
}

#[test]
fn generated_replay_needs_no_privileges_and_serializes_no_process_identity() {
    let mut previous = None;
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .arg("replay")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert!(output.stdout.len() < 1024);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report.as_object().unwrap().len(), 5);
        assert_eq!(report["source"], "generated_replay");
        assert_eq!(report["sensor_running"], false);
        assert_eq!(report["health"]["accepted"], 2);
        assert_eq!(report["health"]["rejected_record"], 2);
        assert_eq!(report["health"]["rejected_clock"], 1);
        assert_eq!(report["health"]["ring_dropped"], 3);
        assert_eq!(report["health"].as_object().unwrap().len(), 5);
        if let Some(previous) = &previous {
            assert_eq!(previous, &output.stdout);
        }
        previous = Some(output.stdout);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn invalid_or_unacknowledged_live_requests_never_emit_success() {
    for arguments in [
        vec![],
        vec!["replay", "extra"],
        vec!["capture"],
        vec![
            "capture",
            "--object",
            "/dev/null",
            "--pid",
            "1",
            "--seconds",
            "1",
        ],
        vec![
            "capture",
            "--object",
            "/dev/null",
            "--pid",
            "0",
            "--seconds",
            "1",
            "--acknowledge-process-observation",
        ],
        vec![
            "capture",
            "--object",
            "/dev/null",
            "--pid",
            "1",
            "--seconds",
            "61",
            "--acknowledge-process-observation",
        ],
        vec![
            "capture",
            "--object",
            "/dev/null",
            "--pid",
            "1",
            "--seconds",
            "1",
            "--acknowledge-process-observation",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("OBSERVATION REQUESTED"));
    }
}
