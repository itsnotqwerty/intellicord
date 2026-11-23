use std::{future::Future, process::Stdio, time::Duration};

use tokio::{net::UnixListener, process::Command, time::Instant};

use super::{
    ProgressStream, Source, peer, socket,
    supervisor::{Deadline, HealthSender, Run},
};
use crate::observe::{lifecycle::Phase, supervisor::Outcome};

pub struct Limits {
    pub total: Duration,
    pub idle: Duration,
    pub accept: Duration,
}

pub async fn supervise(
    mut command: Command,
    listener: UnixListener,
    session: &str,
    source: Source,
    limits: Limits,
    cancel: impl Future<Output = ()>,
    health: HealthSender,
) -> Run {
    let mut report = Run {
        schema_version: 1,
        outcome: Outcome::InvalidDuration,
        collector_started: false,
        collector_reaped: false,
        deadline: None,
        summary: None,
    };
    if limits.total.is_zero()
        || limits.total > Duration::from_secs(65)
        || limits.idle.is_zero()
        || limits.idle > limits.total
        || limits.accept.is_zero()
        || limits.accept > Duration::from_secs(2)
        || limits.accept > limits.total
    {
        return report;
    }
    if ProgressStream::new(session, source).is_err() {
        report.outcome = Outcome::InvalidStream;
        return report;
    }
    tokio::pin!(cancel);
    if futures_util::FutureExt::now_or_never(cancel.as_mut()).is_some() {
        report.outcome = Outcome::Cancelled;
        return report;
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let origin = Instant::now();
    let Ok(mut child) = command.spawn() else {
        report.outcome = Outcome::SpawnFailed;
        return report;
    };
    report.collector_started = true;
    let result = async {
        let expected = peer::ExpectedPeer::new(
            unsafe { libc::geteuid() },
            child.id().ok_or(Outcome::WaitFailed)?,
        )
        .map_err(|_| Outcome::InvalidStream)?;
        let remaining = limits.total.saturating_sub(origin.elapsed());
        if remaining.is_zero() {
            report.deadline = Some(Deadline::Total);
            return Err(Outcome::Deadline);
        }
        let accepted = peer::accept(
            &listener,
            expected,
            limits.accept.min(remaining),
            cancel.as_mut(),
        )
        .await
        .map_err(|error| match error {
            peer::Error::Cancelled => Outcome::Cancelled,
            peer::Error::Deadline => {
                report.deadline = Some(if remaining <= limits.accept {
                    Deadline::Total
                } else {
                    Deadline::Idle
                });
                Outcome::Deadline
            }
            peer::Error::Accept(_) => Outcome::ReadFailed,
            _ => Outcome::InvalidStream,
        })?;
        drop(listener);
        let remaining = limits.total.saturating_sub(origin.elapsed());
        if remaining.is_zero() {
            report.deadline = Some(Deadline::Total);
            return Err(Outcome::Deadline);
        }
        let exit = async {
            let status = child.wait().await?;
            report.collector_reaped = true;
            Ok(status.success())
        };
        socket::receive(
            accepted,
            session,
            source,
            socket::Limits {
                total: remaining,
                idle: limits.idle.min(remaining),
            },
            exit,
            cancel.as_mut(),
            health,
        )
        .await
        .map_err(|error| match error {
            socket::Error::Cancelled => Outcome::Cancelled,
            socket::Error::TotalDeadline | socket::Error::IdleDeadline => {
                report.deadline = Some(if matches!(error, socket::Error::TotalDeadline) {
                    Deadline::Total
                } else {
                    Deadline::Idle
                });
                Outcome::Deadline
            }
            socket::Error::Read(_) => Outcome::ReadFailed,
            socket::Error::Exit(_) => Outcome::WaitFailed,
            _ => Outcome::InvalidStream,
        })
    }
    .await;
    match result {
        Ok(summary) => {
            report.outcome = if summary.lifecycle.terminal_phase == Phase::Stopped {
                Outcome::Completed
            } else {
                Outcome::CollectorFault
            };
            report.summary = Some(summary);
        }
        Err(outcome) => report.outcome = outcome,
    }
    if !report.collector_reaped {
        let _ = child.start_kill();
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(_)) => report.collector_reaped = true,
            _ => report.outcome = Outcome::CleanupFailed,
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::progress::{generated_stream, supervisor::health_channel};
    use std::{
        os::{linux::net::SocketAddrExt, unix::net::SocketAddr},
        sync::atomic::{AtomicU64, Ordering},
    };

    const SESSION: &str = "00000000000000000000000000000001";

    fn fixture(mode: &str) -> (Command, UnixListener) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "intellicord-socket-owner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let address = SocketAddr::from_abstract_name(&name).unwrap();
        let listener = std::os::unix::net::UnixListener::bind_addr(&address).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "observe::progress::socket_owner::tests::socket_child",
            ])
            .env("INTELLICORD_TEST_OWNER_SOCKET", name)
            .env("INTELLICORD_TEST_OWNER_MODE", mode);
        (command, UnixListener::from_std(listener).unwrap())
    }

    fn limits() -> Limits {
        Limits {
            total: Duration::from_secs(2),
            idle: Duration::from_millis(200),
            accept: Duration::from_secs(1),
        }
    }

    #[test]
    fn socket_child() {
        use std::io::Write;
        let Ok(name) = std::env::var("INTELLICORD_TEST_OWNER_SOCKET") else {
            return;
        };
        let mode = std::env::var("INTELLICORD_TEST_OWNER_MODE").unwrap();
        if mode == "absent" {
            std::thread::sleep(Duration::from_secs(10));
            return;
        }
        if mode == "delayed" {
            std::thread::sleep(Duration::from_millis(200));
        }
        let address = SocketAddr::from_abstract_name(name).unwrap();
        let mut socket = std::os::unix::net::UnixStream::connect_addr(&address).unwrap();
        let bytes = generated_stream();
        match mode.as_str() {
            "malformed" => socket.write_all(b"{}\n").unwrap(),
            "prefix" | "delayed" => {
                let records: Vec<_> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
                socket.write_all(&records[..6].concat()).unwrap();
                std::thread::sleep(Duration::from_secs(10));
            }
            "truncated" => socket.write_all(&bytes[..bytes.len() - 1]).unwrap(),
            _ => {
                for fragment in bytes.chunks(7) {
                    socket.write_all(fragment).unwrap();
                }
            }
        }
        if mode == "no_eof" {
            std::thread::sleep(Duration::from_secs(10));
        }
        drop(socket);
        if mode == "no_exit" {
            std::thread::sleep(Duration::from_secs(10));
        }
        if mode == "bad_exit" {
            std::process::exit(1);
        }
    }

    #[tokio::test]
    async fn owned_child_completion_requires_verified_stream_and_reaped_exit() {
        let (command, listener) = fixture("complete");
        let (sender, receiver) = health_channel();
        let run = supervise(
            command,
            listener,
            SESSION,
            Source::GeneratedReplay,
            limits(),
            std::future::pending(),
            sender,
        )
        .await;
        assert_eq!(run.outcome, Outcome::Completed);
        assert!(run.collector_started && run.collector_reaped);
        let summary = run.summary.unwrap();
        assert_eq!(summary.records, 8);
        assert_eq!(summary.health_samples, 2);
        assert!(receiver.latest(Duration::MAX).is_none());
    }

    #[tokio::test]
    async fn invalid_streams_and_exit_mismatch_are_reaped_without_summary() {
        for mode in ["malformed", "truncated", "bad_exit"] {
            let (command, listener) = fixture(mode);
            let (sender, receiver) = health_channel();
            let run = supervise(
                command,
                listener,
                SESSION,
                Source::GeneratedReplay,
                limits(),
                std::future::pending(),
                sender,
            )
            .await;
            assert_eq!(run.outcome, Outcome::InvalidStream, "{mode}");
            assert!(run.collector_started && run.collector_reaped, "{mode}");
            assert!(run.summary.is_none());
            assert!(receiver.latest(Duration::MAX).is_none());
        }
    }

    #[tokio::test]
    async fn missing_connection_eof_or_exit_cannot_escape_deadlines() {
        for mode in ["absent", "no_eof", "no_exit", "prefix"] {
            let (command, listener) = fixture(mode);
            let (sender, receiver) = health_channel();
            let mut policy = limits();
            policy.accept = Duration::from_millis(200);
            let run = supervise(
                command,
                listener,
                SESSION,
                Source::GeneratedReplay,
                policy,
                std::future::pending(),
                sender,
            )
            .await;
            assert_eq!(run.outcome, Outcome::Deadline, "{mode}");
            assert_eq!(run.deadline, Some(Deadline::Idle), "{mode}");
            assert!(run.collector_started && run.collector_reaped, "{mode}");
            assert!(run.summary.is_none());
            assert!(receiver.latest(Duration::MAX).is_none());
        }
    }

    #[tokio::test]
    async fn cancellation_after_health_revokes_and_reaps() {
        let (command, listener) = fixture("prefix");
        let (sender, mut receiver) = health_channel();
        let retained = receiver.clone();
        let cancel = async move {
            loop {
                receiver.changed().await.unwrap();
                if let Some(sample) = receiver.latest(Duration::from_secs(1)) {
                    assert_eq!(sample.source, Source::GeneratedReplay);
                    assert!((1..=2).contains(&sample.health.accepted));
                    break;
                }
            }
        };
        let run = supervise(
            command,
            listener,
            SESSION,
            Source::GeneratedReplay,
            limits(),
            cancel,
            sender,
        )
        .await;
        assert_eq!(run.outcome, Outcome::Cancelled);
        assert!(run.collector_started && run.collector_reaped);
        assert!(run.summary.is_none());
        assert!(retained.latest(Duration::MAX).is_none());
    }

    #[tokio::test]
    async fn total_budget_includes_acceptance_and_ready_cancel_prevents_spawn() {
        let (command, listener) = fixture("delayed");
        let (sender, _) = health_channel();
        let origin = Instant::now();
        let run = supervise(
            command,
            listener,
            SESSION,
            Source::GeneratedReplay,
            Limits {
                total: Duration::from_millis(500),
                idle: Duration::from_millis(500),
                accept: Duration::from_millis(500),
            },
            std::future::pending(),
            sender,
        )
        .await;
        assert_eq!(run.outcome, Outcome::Deadline);
        assert_eq!(run.deadline, Some(Deadline::Total));
        assert!(run.collector_reaped);
        assert!(origin.elapsed() >= Duration::from_millis(450));
        assert!(origin.elapsed() < Duration::from_millis(650));
        for (session, policy, outcome) in [
            (SESSION, limits(), Outcome::Cancelled),
            ("invalid", limits(), Outcome::InvalidStream),
            (
                SESSION,
                Limits {
                    accept: Duration::ZERO,
                    ..limits()
                },
                Outcome::InvalidDuration,
            ),
        ] {
            let (command, listener) = fixture("absent");
            let (sender, _) = health_channel();
            let run = supervise(
                command,
                listener,
                session,
                Source::GeneratedReplay,
                policy,
                std::future::ready(()),
                sender,
            )
            .await;
            assert_eq!(run.outcome, outcome);
            assert!(!run.collector_started && !run.collector_reaped);
        }
    }

    #[tokio::test]
    async fn queued_foreign_pid_is_rejected_and_acceptance_cancellation_reaps() {
        let (command, listener) = fixture("absent");
        let address: SocketAddr = listener.local_addr().unwrap().into();
        let mut foreign = std::os::unix::net::UnixStream::connect_addr(&address).unwrap();
        let (sender, receiver) = health_channel();
        let run = supervise(
            command,
            listener,
            SESSION,
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
        use std::io::Read;
        foreign
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(foreign.read(&mut [0]).unwrap(), 0);

        let (command, listener) = fixture("absent");
        let (sender, _) = health_channel();
        let run = supervise(
            command,
            listener,
            SESSION,
            Source::GeneratedReplay,
            limits(),
            async { tokio::time::sleep(Duration::from_millis(50)).await },
            sender,
        )
        .await;
        assert_eq!(run.outcome, Outcome::Cancelled);
        assert!(run.collector_started && run.collector_reaped);
        assert!(run.summary.is_none());
    }

    #[tokio::test]
    async fn spawn_failure_has_no_process_or_health() {
        let (_, listener) = fixture("absent");
        let (sender, receiver) = health_channel();
        let run = supervise(
            Command::new("/nonexistent/intellicord-socket-owner-test"),
            listener,
            SESSION,
            Source::GeneratedReplay,
            limits(),
            std::future::pending(),
            sender,
        )
        .await;
        assert_eq!(run.outcome, Outcome::SpawnFailed);
        assert!(!run.collector_started && !run.collector_reaped);
        assert!(receiver.latest(Duration::MAX).is_none());
    }
}
