use std::{future::Future, process::Stdio, time::Duration};

use serde::Serialize;
use tokio::{io::AsyncReadExt, process::Command, sync::watch, time::Instant};

use super::{ProgressStream, Summary};
use crate::observe::{ProcessHealth, lifecycle::Phase, stream::Source, supervisor::Outcome};

#[derive(Clone, Copy, Debug)]
pub struct HealthSample {
    pub source: Source,
    pub health: ProcessHealth,
}

pub struct HealthSender(watch::Sender<Option<(HealthSample, Instant)>>);

impl HealthSender {
    pub(super) fn publish(&self, stream: &ProgressStream, source: Source, origin: Instant) {
        self.0
            .send_replace(stream.observing_health().map(|(health, receipt)| {
                (
                    HealthSample { source, health },
                    origin + Duration::from_nanos(receipt),
                )
            }));
    }
}

#[derive(Clone)]
pub struct HealthReceiver(watch::Receiver<Option<(HealthSample, Instant)>>);

pub fn health_channel() -> (HealthSender, HealthReceiver) {
    let (sender, receiver) = watch::channel(None);
    (HealthSender(sender), HealthReceiver(receiver))
}

impl HealthReceiver {
    pub fn latest(&self, max_age: Duration) -> Option<HealthSample> {
        self.latest_with_remaining(max_age)
            .map(|(sample, _)| sample)
    }

    pub(super) fn latest_with_remaining(
        &self,
        max_age: Duration,
    ) -> Option<(HealthSample, Duration)> {
        self.0.borrow().as_ref().and_then(|(sample, receipt)| {
            Instant::now()
                .checked_duration_since(*receipt)
                .filter(|age| *age <= max_age)
                .map(|age| (*sample, max_age - age))
        })
    }

    pub async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.0.changed().await
    }
}

impl Drop for HealthSender {
    fn drop(&mut self) {
        self.0.send_replace(None);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Deadline {
    Total,
    Idle,
}

#[derive(Debug, Serialize)]
pub struct Run {
    pub schema_version: u16,
    pub outcome: Outcome,
    pub collector_started: bool,
    pub collector_reaped: bool,
    pub deadline: Option<Deadline>,
    pub summary: Option<Summary>,
}

pub async fn supervise(
    command: Command,
    session: &str,
    source: Source,
    limit: Duration,
    idle: Duration,
    cancel: impl Future<Output = ()>,
) -> Run {
    supervise_inner(command, session, source, limit, idle, cancel, None).await
}

pub async fn supervise_with_health(
    command: Command,
    session: &str,
    source: Source,
    limit: Duration,
    idle: Duration,
    cancel: impl Future<Output = ()>,
    health: HealthSender,
) -> Run {
    supervise_inner(command, session, source, limit, idle, cancel, Some(health)).await
}

async fn supervise_inner(
    mut command: Command,
    session: &str,
    source: Source,
    limit: Duration,
    idle: Duration,
    cancel: impl Future<Output = ()>,
    health: Option<HealthSender>,
) -> Run {
    let mut report = Run {
        schema_version: 1,
        outcome: Outcome::InvalidDuration,
        collector_started: false,
        collector_reaped: false,
        deadline: None,
        summary: None,
    };
    if limit.is_zero() || limit > Duration::from_secs(65) || idle.is_zero() || idle > limit {
        return report;
    }
    let Ok(mut stream) = ProgressStream::new(session, source) else {
        report.outcome = Outcome::InvalidStream;
        return report;
    };
    tokio::pin!(cancel);
    if futures_util::FutureExt::now_or_never(cancel.as_mut()).is_some() {
        report.outcome = Outcome::Cancelled;
        return report;
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        report.outcome = Outcome::SpawnFailed;
        return report;
    };
    report.collector_started = true;
    let mut output = child.stdout.take().expect("piped progress stdout");
    let origin = Instant::now();
    let total_deadline = tokio::time::sleep_until(origin + limit);
    let idle_deadline = tokio::time::sleep_until(origin + idle);
    tokio::pin!(total_deadline, idle_deadline);
    let mut eof = false;
    let mut exit = None;
    let mut buffer = [0_u8; 1024];
    report.outcome = loop {
        if eof && let Some(success) = exit {
            match stream.finish(success) {
                Ok(summary) => {
                    report.outcome = if summary.lifecycle.terminal_phase == Phase::Stopped {
                        Outcome::Completed
                    } else {
                        Outcome::CollectorFault
                    };
                    report.summary = Some(summary);
                    return report;
                }
                Err(_) => break Outcome::InvalidStream,
            }
        }
        tokio::select! {
            biased;
            _ = &mut cancel => break Outcome::Cancelled,
            _ = &mut total_deadline => {
                report.deadline = Some(Deadline::Total);
                break Outcome::Deadline;
            }
            _ = &mut idle_deadline => {
                report.deadline = Some(Deadline::Idle);
                break Outcome::Deadline;
            }
            read = output.read(&mut buffer), if !eof => match read {
                Ok(0) => eof = true,
                Ok(count) => {
                    let previous = stream.records();
                    let now = Instant::now();
                    let Ok(receipt) = u64::try_from(now.duration_since(origin).as_nanos()) else {
                        break Outcome::InvalidStream;
                    };
                    if stream.push(&buffer[..count], receipt).is_err() {
                        break Outcome::InvalidStream;
                    }
                    if let Some(sender) = &health {
                        sender.publish(&stream, source, origin);
                    }
                    if stream.records() > previous {
                        idle_deadline.as_mut().reset(now + idle);
                    }
                }
                Err(_) => break Outcome::ReadFailed,
            },
            result = child.wait(), if exit.is_none() => match result {
                Ok(status) => {
                    report.collector_reaped = true;
                    exit = Some(status.success());
                }
                Err(_) => break Outcome::WaitFailed,
            },
        }
    };
    if let Some(sender) = &health {
        sender.0.send_replace(None);
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

    const SESSION: &str = "00000000000000000000000000000001";

    #[test]
    fn client_handoff_withholds_generated_invalid_and_revoked_samples() {
        use super::super::client::{Snapshot, Status};
        assert_eq!(Snapshot::read(None).status, Status::NotConnected);
        let (sender, receiver) = health_channel();
        assert_eq!(
            Snapshot::read(Some(&receiver)).status,
            Status::NoRecentSample
        );
        let health = ProcessHealth {
            schema_version: 1,
            accepted: 2,
            ..ProcessHealth::default()
        };
        for (source, health, status) in [
            (Source::GeneratedReplay, health, Status::GeneratedRejected),
            (
                Source::KernelTracepoint,
                ProcessHealth {
                    schema_version: 0,
                    ..health
                },
                Status::InvalidSample,
            ),
            (
                Source::KernelTracepoint,
                ProcessHealth {
                    ring_dropped: 9_007_199_254_740_992,
                    ..health
                },
                Status::InvalidSample,
            ),
            (Source::KernelTracepoint, health, Status::RecentKernelSample),
        ] {
            sender
                .0
                .send_replace(Some((HealthSample { source, health }, Instant::now())));
            let snapshot = Snapshot::read(Some(&receiver));
            assert_eq!(snapshot.status, status);
            assert!(!snapshot.sensor_running);
            assert_eq!(
                snapshot.sample.is_some(),
                status == Status::RecentKernelSample
            );
            assert_eq!(snapshot.valid_for_ms > 0, snapshot.sample.is_some());
        }
        drop(sender);
        let snapshot = Snapshot::read(Some(&receiver));
        assert_eq!(snapshot.status, Status::NoRecentSample);
        assert!(snapshot.sample.is_none());
        assert_eq!(snapshot.valid_for_ms, 0);
    }

    #[test]
    fn client_handoff_preserves_remaining_budget_and_has_no_identity_fields() {
        use super::super::client::{Snapshot, Status};
        let (sender, receiver) = health_channel();
        let sample = HealthSample {
            source: Source::KernelTracepoint,
            health: ProcessHealth {
                schema_version: 1,
                accepted: 2,
                ..ProcessHealth::default()
            },
        };
        sender
            .0
            .send_replace(Some((sample, Instant::now() - Duration::from_secs(1))));
        let snapshot = Snapshot::read(Some(&receiver));
        assert_eq!(snapshot.status, Status::RecentKernelSample);
        assert!((1..=1000).contains(&snapshot.valid_for_ms));
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 6);
        assert_eq!(json["sample"].as_object().unwrap().len(), 5);
        assert_eq!(json["source"], "owner_progress_subscription");
        for receipt in [
            Instant::now() - Duration::from_secs(2),
            Instant::now() + Duration::from_secs(1),
        ] {
            sender.0.send_replace(Some((sample, receipt)));
            let snapshot = Snapshot::read(Some(&receiver));
            assert_eq!(snapshot.status, Status::NoRecentSample);
            assert!(snapshot.sample.is_none());
            assert_eq!(snapshot.valid_for_ms, 0);
        }
    }

    #[tokio::test]
    async fn health_subscription_expires_coalesces_and_revokes_on_sender_drop() {
        let (sender, receiver) = health_channel();
        let clone = receiver.clone();
        assert!(receiver.latest(Duration::MAX).is_none());
        for accepted in 1..=128 {
            sender.0.send_replace(Some((
                HealthSample {
                    source: Source::GeneratedReplay,
                    health: ProcessHealth {
                        accepted,
                        ..ProcessHealth::default()
                    },
                },
                Instant::now() - Duration::from_secs(1),
            )));
        }
        assert!(receiver.latest(Duration::from_millis(500)).is_none());
        let sample = receiver.latest(Duration::from_secs(2)).unwrap();
        assert_eq!(sample.source, Source::GeneratedReplay);
        assert_eq!(sample.health.accepted, 128);
        sender
            .0
            .send_replace(Some((sample, Instant::now() + Duration::from_secs(1))));
        assert!(receiver.latest(Duration::MAX).is_none());
        drop(sender);
        assert!(receiver.latest(Duration::MAX).is_none());
        assert!(clone.latest(Duration::MAX).is_none());
    }

    #[tokio::test]
    async fn health_subscription_revokes_on_terminal_failure_cancellation_and_abort() {
        let payload = String::from_utf8(super::super::generated_stream()).unwrap();
        let lines: Vec<_> = payload.split_inclusive('\n').collect();
        let prefix = lines[..6].concat();
        let suffix = lines[6..].concat();
        for mode in ["terminal", "invalid", "cancel", "abort", "idle"] {
            let script = match mode {
                "terminal" => {
                    "process.stdout.write(process.argv[1]);setTimeout(()=>process.stdout.write(process.argv[2]),300);setInterval(()=>{},1000)"
                }
                "invalid" => {
                    "process.stdout.write(process.argv[1]);setTimeout(()=>process.stdout.write('{}\\n'),300);setInterval(()=>{},1000)"
                }
                _ => "process.stdout.write(process.argv[1]);setInterval(()=>{},1000)",
            };
            let mut command = Command::new("node");
            command.args(["-e", script, &prefix, &suffix]);
            let (sender, mut receiver) = health_channel();
            let (cancel, cancelled) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(supervise_with_health(
                command,
                SESSION,
                Source::GeneratedReplay,
                Duration::from_secs(5),
                Duration::from_secs(1),
                async {
                    let _ = cancelled.await;
                },
                sender,
            ));
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    receiver.changed().await.unwrap();
                    if let Some(sample) = receiver.latest(Duration::from_secs(1)) {
                        assert_eq!(sample.health.accepted, 2);
                        assert_eq!(sample.source, Source::GeneratedReplay);
                        break;
                    }
                }
            })
            .await
            .unwrap();
            if mode == "abort" {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                if mode == "cancel" {
                    cancel.send(()).unwrap();
                }
                if mode == "terminal" {
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while receiver.latest(Duration::MAX).is_some() {
                            receiver.changed().await.unwrap();
                        }
                    })
                    .await
                    .unwrap();
                    assert!(!task.is_finished());
                }
                let report = tokio::time::timeout(Duration::from_secs(4), task)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(report.collector_reaped);
                assert!(report.summary.is_none());
                assert_eq!(
                    report.outcome,
                    match mode {
                        "cancel" => Outcome::Cancelled,
                        "invalid" => Outcome::InvalidStream,
                        _ => Outcome::Deadline,
                    }
                );
            }
            assert!(receiver.latest(Duration::MAX).is_none(), "{mode}");
        }
    }

    #[tokio::test]
    async fn health_subscription_withholds_terminal_and_invalid_batches() {
        let payload = String::from_utf8(super::super::generated_stream()).unwrap();
        for invalid in [false, true] {
            let bytes = if invalid {
                payload.clone() + "{}\n"
            } else {
                payload.clone()
            };
            let (sender, receiver) = health_channel();
            let mut command = Command::new("node");
            command.args(["-e", "process.stdout.write(process.argv[1])", &bytes]);
            let report = supervise_with_health(
                command,
                SESSION,
                Source::GeneratedReplay,
                Duration::from_secs(3),
                Duration::from_secs(2),
                std::future::pending(),
                sender,
            )
            .await;
            assert_eq!(
                report.outcome,
                if invalid {
                    Outcome::InvalidStream
                } else {
                    Outcome::Completed
                }
            );
            assert!(report.collector_reaped);
            assert_eq!(report.summary.is_some(), !invalid);
            assert!(receiver.latest(Duration::MAX).is_none());
        }
        let lines: Vec<_> = payload.split_inclusive('\n').collect();
        let mut stream = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        stream.push(lines[..6].concat().as_bytes(), 100).unwrap();
        assert_eq!(stream.observing_health().unwrap().1, 100);
        stream.push(&lines[6].as_bytes()[..1], 200).unwrap();
        assert_eq!(stream.observing_health().unwrap().1, 100);
        stream.push(&lines[6].as_bytes()[1..], 300).unwrap();
        assert!(stream.observing_health().is_none());
    }

    #[tokio::test]
    async fn cancellation_deadlines_and_pre_spawn_rejections_are_explicit() {
        let command = || {
            let mut command = Command::new("node");
            command.args(["-e", "setInterval(() => {}, 1000)"]);
            command
        };
        let report = supervise(
            command(),
            SESSION,
            Source::GeneratedReplay,
            Duration::from_secs(3),
            Duration::from_secs(1),
            async {
                tokio::time::sleep(Duration::from_millis(100)).await;
            },
        )
        .await;
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert!(report.collector_started && report.collector_reaped);
        assert!(report.summary.is_none() && report.deadline.is_none());
        let report = supervise(
            command(),
            SESSION,
            Source::GeneratedReplay,
            Duration::from_millis(100),
            Duration::from_millis(100),
            std::future::pending(),
        )
        .await;
        assert_eq!(report.outcome, Outcome::Deadline);
        assert_eq!(report.deadline, Some(Deadline::Total));
        assert!(report.collector_reaped && report.summary.is_none());
        for (limit, idle, session, expected) in [
            (
                Duration::ZERO,
                Duration::ZERO,
                SESSION,
                Outcome::InvalidDuration,
            ),
            (
                Duration::from_secs(66),
                Duration::from_secs(1),
                SESSION,
                Outcome::InvalidDuration,
            ),
            (
                Duration::from_secs(1),
                Duration::ZERO,
                SESSION,
                Outcome::InvalidDuration,
            ),
            (
                Duration::from_secs(1),
                Duration::from_secs(2),
                SESSION,
                Outcome::InvalidDuration,
            ),
            (
                Duration::from_secs(1),
                Duration::from_secs(1),
                "bad",
                Outcome::InvalidStream,
            ),
        ] {
            let report = supervise(
                command(),
                session,
                Source::GeneratedReplay,
                limit,
                idle,
                std::future::pending(),
            )
            .await;
            assert_eq!(report.outcome, expected);
            assert!(!report.collector_started && !report.collector_reaped);
        }
        let report = supervise(
            command(),
            SESSION,
            Source::GeneratedReplay,
            Duration::from_secs(1),
            Duration::from_secs(1),
            std::future::ready(()),
        )
        .await;
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert!(!report.collector_started && !report.collector_reaped);
        let report = supervise(
            Command::new("/nonexistent/intellicord-progress-child"),
            SESSION,
            Source::GeneratedReplay,
            Duration::from_secs(1),
            Duration::from_secs(1),
            std::future::pending(),
        )
        .await;
        assert_eq!(report.outcome, Outcome::SpawnFailed);
        assert!(!report.collector_started && !report.collector_reaped);
    }

    #[tokio::test]
    async fn valid_records_refresh_idle_but_never_the_total_deadline() {
        let payload = String::from_utf8(super::super::generated_stream()).unwrap();
        let script = "const lines=process.argv[1].trimEnd().split('\\n');let index=0;const timer=setInterval(()=>{process.stdout.write(lines[index++]+'\\n');if(index===lines.length)clearInterval(timer)},150)";
        let mut command = Command::new("node");
        command.args(["-e", script, &payload]);
        let report = supervise(
            command,
            SESSION,
            Source::GeneratedReplay,
            Duration::from_secs(5),
            Duration::from_secs(1),
            std::future::pending(),
        )
        .await;
        assert_eq!(report.outcome, Outcome::Completed);
        assert!(report.collector_reaped);
        assert_eq!(report.summary.unwrap().health_samples, 2);
        let mut command = Command::new("node");
        command.args(["-e", script, &payload]);
        let report = supervise(
            command,
            SESSION,
            Source::GeneratedReplay,
            Duration::from_millis(700),
            Duration::from_millis(500),
            std::future::pending(),
        )
        .await;
        assert_eq!(report.outcome, Outcome::Deadline);
        assert_eq!(report.deadline, Some(Deadline::Total));
        assert!(report.collector_reaped && report.summary.is_none());
    }

    #[tokio::test]
    async fn coherent_fault_retains_only_validated_terminal_summary() {
        let status = crate::observe::lifecycle::Lifecycle::new(true)
            .advance(
                Phase::Faulted,
                Some(crate::observe::lifecycle::Reason::InvalidObject),
                None,
            )
            .unwrap();
        let payload = serde_json::json!({"schema_version":1,"session":SESSION,"sequence":1,
            "payload":{"lifecycle":{"status":status}}})
        .to_string()
            + "\n";
        let mut command = Command::new("node");
        command.args([
            "-e",
            "process.stdout.write(process.argv[1]); process.exitCode=1",
            &payload,
        ]);
        let report = supervise(
            command,
            SESSION,
            Source::GeneratedReplay,
            Duration::from_secs(3),
            Duration::from_secs(1),
            std::future::pending(),
        )
        .await;
        assert_eq!(report.outcome, Outcome::CollectorFault);
        assert!(report.collector_reaped);
        assert_eq!(
            report.summary.unwrap().lifecycle.terminal_phase,
            Phase::Faulted
        );
    }

    #[tokio::test]
    async fn subprocesses_require_terminal_eof_exit_and_bound_silence() {
        let payload = String::from_utf8(super::super::generated_stream()).unwrap();
        for (script, expected, deadline) in [
            (
                "process.stdout.write(process.argv[1])",
                Outcome::Completed,
                None,
            ),
            (
                "process.stdout.write('{}\\n')",
                Outcome::InvalidStream,
                None,
            ),
            (
                "process.stdout.write(process.argv[1]); process.exitCode = 1",
                Outcome::InvalidStream,
                None,
            ),
            (
                "process.stdout.write(process.argv[1].slice(0,-1))",
                Outcome::InvalidStream,
                None,
            ),
            (
                "setInterval(() => {}, 1000)",
                Outcome::Deadline,
                Some(Deadline::Idle),
            ),
            (
                "setInterval(() => process.stdout.write(' '), 10)",
                Outcome::Deadline,
                Some(Deadline::Idle),
            ),
            (
                "require('node:fs').writeSync(1, process.argv[1]); require('node:fs').closeSync(1); setInterval(() => {}, 1000)",
                Outcome::Deadline,
                Some(Deadline::Idle),
            ),
        ] {
            let mut command = Command::new("node");
            command.args(["-e", script, &payload]);
            let report = supervise(
                command,
                "00000000000000000000000000000001",
                Source::GeneratedReplay,
                Duration::from_secs(3),
                Duration::from_millis(500),
                std::future::pending(),
            )
            .await;
            assert_eq!(report.outcome, expected, "{script}");
            assert_eq!(report.deadline, deadline);
            assert!(report.collector_started);
            assert!(report.collector_reaped);
            assert_eq!(report.summary.is_some(), expected == Outcome::Completed);
        }
    }
}
