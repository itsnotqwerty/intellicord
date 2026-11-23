#![cfg(all(target_os = "linux", feature = "observe"))]

use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use intellicord::observe::{
    lifecycle::Reason,
    progress::{
        ProgressStream,
        capture::Capture,
        client::{Snapshot, Status},
        socket_owner,
        supervisor::health_channel,
    },
    stream::Source,
    supervisor::Outcome,
};
use tokio::{io::AsyncReadExt, net::UnixListener, process::Command};

const SESSION: &str = "fedcba9876543210fedcba9876543210";

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "intellicord-capture-socket-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }

    fn endpoint(&self) -> PathBuf {
        self.0.join("progress.sock")
    }

    fn listener(&self) -> UnixListener {
        UnixListener::bind(self.endpoint()).unwrap()
    }

    fn arguments(&self) -> Vec<String> {
        [
            "capture-socket-progress".to_owned(),
            "--object".into(),
            "/dev/null".into(),
            "--pid".into(),
            std::process::id().to_string(),
            "--seconds".into(),
            "1".into(),
            "--session".into(),
            SESSION.into(),
            "--socket".into(),
            self.endpoint().to_str().unwrap().into(),
            "--parent-pid".into(),
            std::process::id().to_string(),
            "--acknowledge-socket-progress".into(),
        ]
        .into()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.endpoint());
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn command(arguments: &[String]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"));
    command.args(arguments).kill_on_drop(true);
    command
}

fn parent_arguments() -> Vec<String> {
    [
        "supervise-socket-progress-capture",
        "--object",
        "/dev/null",
        "--pid",
        "1",
        "--seconds",
        "1",
        "--acknowledge-supervised-socket-progress",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[tokio::test]
async fn run_with_cancels_and_reaps_connected_child_when_work_fails() {
    let directory = Directory::new();
    let listener = directory.listener();
    let collector = directory.0.join("collector");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/observe-silent-collector.mjs"),
        &collector,
    )
    .unwrap();
    std::fs::set_permissions(&collector, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (capture, receiver) =
        Capture::prepare(&collector, &directory.0, &directory.endpoint(), 1, 1).unwrap();
    let mut child_pid = 0_u32;
    let work = async {
        let (mut control, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        control.read_to_end(&mut bytes).await.unwrap();
        child_pid = std::str::from_utf8(&bytes).unwrap().trim().parse().unwrap();
        assert!(Path::new(&format!("/proc/{child_pid}")).exists());
        Err::<(), _>("mock tracking failure")
    };
    let (result, report) = tokio::time::timeout(Duration::from_secs(8), capture.run_with(work))
        .await
        .unwrap();
    assert_eq!(result, Err("mock tracking failure"));
    assert_eq!(report.run.outcome, Outcome::Cancelled);
    assert!(report.run.collector_started && report.run.collector_reaped);
    assert!(report.endpoint_removed);
    assert!(report.run.summary.is_none());
    assert!(receiver.latest(Duration::MAX).is_none());
    assert!(!Path::new(&format!("/proc/{child_pid}")).exists());
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 2);
    std::fs::remove_file(collector).unwrap();
}

#[tokio::test]
async fn run_with_keeps_work_alive_after_collector_fault_and_cancels_before_spawn() {
    let directory = Directory::new();
    let (capture, mut receiver) = Capture::prepare(
        Path::new(env!("CARGO_BIN_EXE_intellicord-observe")),
        &directory.0,
        Path::new("/dev/null"),
        1,
        1,
    )
    .unwrap();
    let mut outcome = capture.outcome();
    let work = async {
        while receiver.changed().await.is_ok() {}
        while outcome.borrow().status
            != intellicord::observe::progress::client::OwnerStatus::Finished
        {
            outcome.changed().await.unwrap();
        }
        let snapshot = intellicord::observe::progress::client::OwnerSnapshot::read(Some(&outcome));
        let result = snapshot.result.unwrap();
        assert_eq!(result.outcome, Outcome::CollectorFault);
        assert!(result.collector_reaped && result.endpoint_removed);
        "work survived owner completion"
    };
    let (result, report) = tokio::time::timeout(Duration::from_secs(8), capture.run_with(work))
        .await
        .unwrap();
    assert_eq!(result, "work survived owner completion");
    assert_eq!(report.run.outcome, Outcome::CollectorFault);
    assert!(report.run.collector_reaped && report.endpoint_removed);
    let (capture, receiver) = Capture::prepare(
        Path::new("/nonexistent-intellicord-collector"),
        &directory.0,
        Path::new("/dev/null"),
        1,
        1,
    )
    .unwrap();
    let (result, report) = capture.run_with(std::future::ready(42)).await;
    assert_eq!(result, 42);
    assert_eq!(report.run.outcome, Outcome::Cancelled);
    assert!(!report.run.collector_started);
    assert!(report.endpoint_removed);
    assert!(receiver.latest(Duration::MAX).is_none());
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
}

#[tokio::test]
async fn prepared_owner_validates_before_filesystem_and_drop_closes_subscription() {
    let directory = Directory::new();
    let collector = Path::new(env!("CARGO_BIN_EXE_intellicord-observe"));
    for (process_id, seconds) in [(0, 1), (u32::MAX, 1), (1, 0), (1, 61)] {
        assert!(
            Capture::prepare(
                collector,
                &directory.0,
                Path::new("/dev/null"),
                process_id,
                seconds
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }
    for (collector, object) in [
        (Path::new(""), Path::new("/dev/null")),
        (Path::new("relative-collector"), Path::new("/dev/null")),
        (collector, Path::new("")),
    ] {
        assert!(Capture::prepare(collector, &directory.0, object, 1, 1).is_err());
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }
    let (capture, mut receiver) =
        Capture::prepare(collector, &directory.0, Path::new("/dev/null"), 1, 1).unwrap();
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    assert_eq!(
        Snapshot::read(Some(&receiver)).status,
        Status::NoRecentSample
    );
    drop(capture);
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    assert!(receiver.latest(Duration::MAX).is_none());
    let _ = receiver.changed().await;
    assert!(receiver.changed().await.is_err());
}

#[tokio::test]
async fn prepared_owner_ready_cancellation_and_unpolled_run_never_spawn() {
    let directory = Directory::new();
    for poll in [false, true] {
        let (capture, receiver) = Capture::prepare(
            Path::new("/nonexistent-intellicord-collector"),
            &directory.0,
            Path::new("/dev/null"),
            1,
            1,
        )
        .unwrap();
        let run = capture.run(std::future::ready(()));
        if poll {
            let report = run.await;
            assert_eq!(report.run.outcome, Outcome::Cancelled);
            assert!(!report.run.collector_started);
            assert!(!report.run.collector_reaped);
            assert!(report.endpoint_removed);
            assert!(!report.completed());
        } else {
            drop(run);
        }
        assert!(receiver.latest(Duration::MAX).is_none());
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn shared_owner_reaps_preload_fault_and_withholds_health() {
    let directory = Directory::new();
    for (collector, expected) in [
        (
            Path::new(env!("CARGO_BIN_EXE_intellicord-observe")),
            Outcome::CollectorFault,
        ),
        (
            Path::new("/nonexistent-intellicord-collector"),
            Outcome::SpawnFailed,
        ),
    ] {
        let (capture, receiver) =
            Capture::prepare(collector, &directory.0, Path::new("/dev/null"), 1, 1).unwrap();
        let report =
            tokio::time::timeout(Duration::from_secs(8), capture.run(std::future::pending()))
                .await
                .unwrap();
        assert!(report.endpoint_removed);
        assert!(!report.completed());
        assert_eq!(report.run.outcome, expected);
        assert_eq!(
            report.run.collector_started,
            expected == Outcome::CollectorFault
        );
        if report.run.collector_started {
            assert_eq!(report.run.outcome, Outcome::CollectorFault);
            assert!(report.run.collector_reaped);
            let summary = report.run.summary.as_ref().unwrap();
            assert_eq!(summary.records, 2);
            assert_eq!(summary.health_samples, 0);
        } else {
            assert_eq!(report.run.outcome, Outcome::SpawnFailed);
            assert!(!report.run.collector_reaped);
        }
        assert_eq!(
            Snapshot::read(Some(&receiver)).status,
            Status::NoRecentSample
        );
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 6);
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn shared_owner_preserves_foreign_contents_and_reports_cleanup_independently() {
    let directory = Directory::new();
    let (capture, receiver) = Capture::prepare(
        Path::new("/nonexistent-intellicord-collector"),
        &directory.0,
        Path::new("/dev/null"),
        1,
        1,
    )
    .unwrap();
    let endpoint_directory = std::fs::read_dir(&directory.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let sentinel = endpoint_directory.join("sentinel");
    std::fs::write(&sentinel, b"preserve").unwrap();
    let report = capture.run(std::future::ready(())).await;
    assert_eq!(report.run.outcome, Outcome::Cancelled);
    assert!(!report.run.collector_started);
    assert!(!report.endpoint_removed);
    assert!(!report.completed());
    assert!(receiver.latest(Duration::MAX).is_none());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"preserve");
    assert!(!endpoint_directory.join("progress.sock").exists());
    std::fs::remove_file(sentinel).unwrap();
    std::fs::remove_dir(endpoint_directory).unwrap();
}

#[tokio::test]
async fn parent_socket_capture_gates_precede_endpoint_creation() {
    let directory = Directory::new();
    let base = parent_arguments();
    let mut cases = vec![vec![base[0].clone()], base[..7].to_vec()];
    for (index, replacement) in [
        (1, "--events"),
        (2, ""),
        (3, "--target"),
        (4, "0"),
        (4, "2147483648"),
        (5, "--duration"),
        (6, "0"),
        (6, "61"),
        (7, "--acknowledge-supervised-progress"),
        (7, "--acknowledge-socket-progress"),
    ] {
        let mut arguments = base.clone();
        arguments[index] = replacement.into();
        cases.push(arguments);
    }
    for extra in [
        "--session",
        "--socket",
        "--parent-pid",
        "--events",
        "--status-json",
        "--attach",
    ] {
        let mut arguments = base.clone();
        arguments.push(extra.into());
        cases.push(arguments);
    }
    for arguments in cases {
        let output = command(&arguments)
            .env("TMPDIR", directory.0.join("must-not-be-accessed"))
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("Usage:") || error.contains("invalid target or duration"),
            "{error}"
        );
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn parent_socket_capture_reports_reaping_and_endpoint_removal_on_preload_fault() {
    let directory = Directory::new();
    for seconds in ["1", "60"] {
        let mut arguments = parent_arguments();
        arguments[6] = seconds.into();
        let output = command(&arguments)
            .env("TMPDIR", &directory.0)
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.len() < 2048);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        assert_eq!(report.as_object().unwrap().len(), 6);
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["source"], "supervised_kernel_socket_progress");
        assert_eq!(report["transport"], "kernel_verified_unix_socket");
        assert_eq!(report["session_binding"], "parent_selected");
        assert_eq!(report["endpoint_removed"], true);
        let run = &report["run"];
        assert_eq!(run.as_object().unwrap().len(), 6);
        assert_eq!(run["outcome"], "collector_fault");
        assert_eq!(run["collector_started"], true);
        assert_eq!(run["collector_reaped"], true);
        assert!(run["deadline"].is_null());
        assert_eq!(run["summary"]["records"], 2);
        assert_eq!(run["summary"]["health_samples"], 0);
        assert_eq!(run["summary"]["lifecycle"]["reason"], "invalid_object");
        assert_eq!(run["summary"]["lifecycle"]["source"], "kernel_tracepoint");
        assert_eq!(run["summary"]["lifecycle"]["attachment_confirmed"], false);
        assert_eq!(run["summary"]["lifecycle"]["detachment_confirmed"], false);
        for forbidden in [
            directory.0.to_str().unwrap(),
            "progress.sock",
            "/dev/null",
            "ic-progress-",
        ] {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(forbidden));
        }
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn parent_endpoint_setup_failure_emits_no_report_and_preserves_existing_files() {
    let directory = Directory::new();
    let sentinel = directory.0.join("sentinel");
    std::fs::write(&sentinel, b"preserve").unwrap();
    for root in [directory.0.join("missing"), sentinel.clone()] {
        let output = command(&parent_arguments())
            .env("TMPDIR", root)
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"preserve");
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }
    std::fs::remove_file(sentinel).unwrap();
}

#[tokio::test]
async fn strict_socket_capture_arguments_fail_before_connection() {
    let directory = Directory::new();
    let listener = directory.listener();
    let base = directory.arguments();
    let mut cases = vec![vec![base[0].clone()], base[..13].to_vec()];
    for (index, replacement) in [
        (1, "--events"),
        (2, ""),
        (3, "--target"),
        (4, "0"),
        (4, "2147483648"),
        (5, "--duration"),
        (6, "0"),
        (6, "61"),
        (7, "--binding"),
        (8, "invalid"),
        (9, "--endpoint"),
        (10, ""),
        (10, "relative.sock"),
        (11, "--server"),
        (12, "0"),
        (12, "2147483648"),
        (13, "--acknowledge-process-progress"),
    ] {
        let mut arguments = base.clone();
        arguments[index] = replacement.into();
        cases.push(arguments);
    }
    let mut extra = base.clone();
    extra.push("--status-json".into());
    cases.push(extra);
    for arguments in cases {
        let output = command(&arguments).output().await.unwrap();
        assert!(!output.status.success(), "{arguments:?}");
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn socket_capture_preload_fault_is_bound_and_stdout_stays_empty() {
    let directory = Directory::new();
    let listener = directory.listener();
    let collect = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        bytes
    };
    let (output, bytes) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(command(&directory.arguments()).output(), collect)
    })
    .await
    .unwrap();
    let output = output.unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(bytes.len() < 4096);
    let mut stream = ProgressStream::new(SESSION, Source::KernelTracepoint).unwrap();
    for fragment in bytes.chunks(7) {
        stream.push(fragment, 100).unwrap();
    }
    let summary = stream.finish(false).unwrap();
    assert_eq!(summary.records, 2);
    assert_eq!(summary.health_samples, 0);
    assert_eq!(summary.lifecycle.reason, Reason::InvalidObject);
    assert!(!summary.lifecycle.attachment_confirmed && !summary.lifecycle.detachment_confirmed);
    let mut foreign =
        ProgressStream::new("00000000000000000000000000000001", Source::KernelTracepoint).unwrap();
    assert!(foreign.push(&bytes, 100).is_err());
}

#[tokio::test]
async fn actual_collector_preload_fault_is_reaped_by_socket_owner() {
    let directory = Directory::new();
    let listener = directory.listener();
    let (sender, receiver) = health_channel();
    let run = socket_owner::supervise(
        command(&directory.arguments()),
        listener,
        SESSION,
        Source::KernelTracepoint,
        socket_owner::Limits {
            total: Duration::from_secs(3),
            idle: Duration::from_secs(2),
            accept: Duration::from_secs(1),
        },
        std::future::pending(),
        sender,
    )
    .await;
    assert_eq!(run.outcome, Outcome::CollectorFault);
    assert!(run.collector_started && run.collector_reaped);
    let summary = run.summary.unwrap();
    assert_eq!(summary.records, 2);
    assert_eq!(summary.health_samples, 0);
    assert_eq!(summary.lifecycle.reason, Reason::InvalidObject);
    assert!(!summary.lifecycle.attachment_confirmed && !summary.lifecycle.detachment_confirmed);
    assert!(receiver.latest(Duration::MAX).is_none());
}

#[tokio::test]
async fn private_directory_and_socket_type_checks_precede_connection() {
    for mode in ["public", "symlink", "regular", "missing", "wrong_parent"] {
        let directory = Directory::new();
        let listener = if matches!(mode, "public" | "wrong_parent") {
            Some(directory.listener())
        } else {
            None
        };
        let mut arguments = directory.arguments();
        match mode {
            "public" => {
                std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o755))
                    .unwrap()
            }
            "symlink" => std::os::unix::fs::symlink("/dev/null", directory.endpoint()).unwrap(),
            "regular" => std::fs::write(directory.endpoint(), []).unwrap(),
            "wrong_parent" => arguments[12] = "1".into(),
            _ => {}
        }
        let output = command(&arguments).output().await.unwrap();
        assert!(!output.status.success(), "{mode}");
        assert!(output.stdout.is_empty());
        if let Some(listener) = listener {
            assert!(
                tokio::time::timeout(Duration::from_millis(30), listener.accept())
                    .await
                    .is_err()
            );
        }
    }
}

#[test]
fn foreign_server_child() {
    use std::io::{Read, Write};
    let Ok(endpoint) = std::env::var("INTELLICORD_TEST_FOREIGN_CAPTURE_SOCKET") else {
        return;
    };
    let listener = std::os::unix::net::UnixListener::bind(endpoint).unwrap();
    std::io::stdout().write_all(b"R").unwrap();
    std::io::stdout().flush().unwrap();
    let (mut stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
}

#[tokio::test]
async fn same_uid_foreign_server_pid_receives_no_progress() {
    use std::process::Stdio;
    let directory = Directory::new();
    let mut server = Command::new(std::env::current_exe().unwrap());
    server
        .args(["--exact", "foreign_server_child", "--nocapture"])
        .env(
            "INTELLICORD_TEST_FOREIGN_CAPTURE_SOCKET",
            directory.endpoint(),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let mut server = server.spawn().unwrap();
    let mut output = server.stdout.take().unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let mut byte = [0];
            output.read_exact(&mut byte).await.unwrap();
            if byte == *b"R" {
                break;
            }
        }
    })
    .await
    .unwrap();
    let result = command(&directory.arguments()).output().await.unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("server credentials did not match"));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), server.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
}
