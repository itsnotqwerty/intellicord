use std::process::Command;

const BINDING: &str = "INTELLICORD_GENERATED_SOCKET_SESSION";

#[test]
fn socket_replay_commands_reject_options_before_output() {
    for command in [
        "supervise-socket-progress-replay",
        "socket-progress-replay-child",
    ] {
        for extra in ["--session", "--pid", "--object", "--attach", "extra"] {
            let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
                .args([command, extra])
                .env_remove(BINDING)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("accepts no options"));
        }
    }
}

#[cfg(not(all(target_os = "linux", feature = "observe")))]
#[test]
fn unavailable_socket_commands_fail_without_starting_a_replay() {
    for command in [
        "supervise-socket-progress-replay",
        "socket-progress-replay-child",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .arg(command)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("requires Linux and the observe feature")
        );
    }
}

#[cfg(all(target_os = "linux", feature = "observe"))]
mod supported {
    use super::*;
    use intellicord::observe::{
        progress::{socket_owner, supervisor::health_channel},
        stream::Source,
        supervisor::Outcome,
    };
    use std::{
        os::{linux::net::SocketAddrExt, unix::net::SocketAddr},
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    #[test]
    fn parent_reports_generated_socket_health_and_awaited_cleanup() {
        for _ in 0..2 {
            let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
                .arg("supervise-socket-progress-replay")
                .env(BINDING, "invalid-inherited-value")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stderr.is_empty());
            assert!(output.stdout.len() < 2048);
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report.as_object().unwrap().len(), 10);
            assert_eq!(report["schema_version"], 1);
            assert_eq!(report["source"], "generated_replay");
            assert_eq!(report["sensor_running"], false);
            assert_eq!(report["load_test_performed"], false);
            assert_eq!(report["transport"], "kernel_verified_unix_socket");
            assert_eq!(report["session_binding"], "parent_selected");
            for field in ["observed_health", "generated_only", "health_revoked"] {
                assert_eq!(report[field], true, "{field}");
            }
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
            let text = String::from_utf8(output.stdout).unwrap();
            for forbidden in [
                "intellicord-generated-progress-",
                "invalid-inherited-value",
                BINDING,
            ] {
                assert!(!text.contains(forbidden));
            }
        }
    }

    #[test]
    fn child_rejects_missing_or_malformed_binding_without_progress() {
        for binding in [
            None,
            Some(""),
            Some("../socket"),
            Some("A0000000000000000000000000000000"),
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord"));
            command
                .arg("socket-progress-replay-child")
                .env_remove(BINDING);
            if let Some(value) = binding {
                command.env(BINDING, value);
            }
            let output = command.output().unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("binding required")
                    || error.contains("Invalid generated socket replay binding")
            );
        }
    }

    fn fixture() -> (tokio::process::Command, tokio::net::UnixListener, String) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let session = format!(
            "{:016x}{:016x}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let address =
            SocketAddr::from_abstract_name(format!("intellicord-generated-progress-{session}"))
                .unwrap();
        let listener = std::os::unix::net::UnixListener::bind_addr(&address).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_intellicord"));
        command
            .arg("socket-progress-replay-child")
            .env_clear()
            .env(BINDING, &session);
        (
            command,
            tokio::net::UnixListener::from_std(listener).unwrap(),
            session,
        )
    }

    fn limits() -> socket_owner::Limits {
        socket_owner::Limits {
            total: Duration::from_secs(6),
            idle: Duration::from_secs(3),
            accept: Duration::from_secs(2),
        }
    }

    #[tokio::test]
    async fn actual_socket_child_health_can_cancel_owner_and_revoke() {
        let (command, listener, session) = fixture();
        let (sender, mut receiver) = health_channel();
        let retained = receiver.clone();
        let mut observed = false;
        let started = tokio::time::Instant::now();
        let cancel = async {
            while receiver.changed().await.is_ok() {
                if let Some(sample) = receiver.latest(Duration::from_secs(2)) {
                    assert_eq!(sample.source, Source::GeneratedReplay);
                    assert!((1..=2).contains(&sample.health.accepted));
                    let handoff =
                        intellicord::observe::progress::client::Snapshot::read(Some(&receiver));
                    assert_eq!(
                        handoff.status,
                        intellicord::observe::progress::client::Status::GeneratedRejected
                    );
                    assert!(handoff.sample.is_none());
                    assert_eq!(handoff.valid_for_ms, 0);
                    assert!(started.elapsed() >= Duration::from_millis(900));
                    observed = true;
                    return;
                }
            }
        };
        let run = socket_owner::supervise(
            command,
            listener,
            &session,
            Source::GeneratedReplay,
            limits(),
            cancel,
            sender,
        )
        .await;
        assert!(observed);
        assert_eq!(run.outcome, Outcome::Cancelled);
        assert!(run.collector_started && run.collector_reaped);
        assert!(run.summary.is_none());
        assert!(retained.latest(Duration::MAX).is_none());
    }

    #[tokio::test]
    async fn actual_socket_child_foreign_session_is_rejected_and_reaped() {
        let (command, listener, _) = fixture();
        let (sender, receiver) = health_channel();
        let run = socket_owner::supervise(
            command,
            listener,
            "ffffffffffffffffffffffffffffffff",
            Source::GeneratedReplay,
            limits(),
            std::future::pending(),
            sender,
        )
        .await;
        assert_eq!(run.outcome, Outcome::InvalidStream);
        assert!(run.collector_started && run.collector_reaped);
        assert!(run.summary.is_none());
        assert!(receiver.latest(Duration::MAX).is_none());
    }
}
