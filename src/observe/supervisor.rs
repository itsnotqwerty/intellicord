use std::{
    future::Future,
    process::Stdio,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, process::Command};

use super::{
    collection::{Collection, CollectionSummary},
    lifecycle::Phase,
    stream::{Source, StatusStream, ValidatedRun},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    CollectorFault,
    InvalidDuration,
    SpawnFailed,
    Cancelled,
    Deadline,
    InvalidStream,
    ReadFailed,
    WaitFailed,
    CleanupFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisedRun {
    pub schema_version: u16,
    pub outcome: Outcome,
    pub collector_started: bool,
    pub collector_reaped: bool,
    pub status: Option<ValidatedRun>,
    pub terminal_health: Option<super::ProcessHealth>,
}

pub async fn supervise(
    command: Command,
    source: Source,
    limit: Duration,
    cancel: impl Future<Output = ()>,
) -> SupervisedRun {
    supervise_inner(command, source, limit, cancel, None, None)
        .await
        .0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SupervisedCollection {
    pub schema_version: u16,
    pub outcome: Outcome,
    pub collector_started: bool,
    pub collector_reaped: bool,
    pub collection: Option<CollectionSummary>,
}

pub async fn supervise_collection(
    command: Command,
    collection: Collection,
    origin: Instant,
    limit: Duration,
    cancel: impl Future<Output = ()>,
) -> SupervisedCollection {
    let (run, collection) = supervise_inner(
        command,
        Source::GeneratedReplay,
        limit,
        cancel,
        Some((collection, origin)),
        None,
    )
    .await;
    SupervisedCollection {
        schema_version: 1,
        outcome: run.outcome,
        collector_started: run.collector_started,
        collector_reaped: run.collector_reaped,
        collection,
    }
}

pub async fn supervise_negotiated_collection(
    command: Command,
    source: Source,
    process_id: u32,
    nonce: u64,
    limit: Duration,
    cancel: impl Future<Output = ()>,
) -> SupervisedCollection {
    let (run, collection) = supervise_inner(
        command,
        source,
        limit,
        cancel,
        None,
        Some((process_id, nonce, Instant::now())),
    )
    .await;
    SupervisedCollection {
        schema_version: 1,
        outcome: run.outcome,
        collector_started: run.collector_started,
        collector_reaped: run.collector_reaped,
        collection,
    }
}

async fn supervise_inner(
    mut command: Command,
    source: Source,
    limit: Duration,
    cancel: impl Future<Output = ()>,
    mut collection: Option<(Collection, Instant)>,
    negotiation: Option<(u32, u64, Instant)>,
) -> (SupervisedRun, Option<CollectionSummary>) {
    let mut report = SupervisedRun {
        schema_version: 1,
        outcome: Outcome::InvalidDuration,
        collector_started: false,
        collector_reaped: false,
        status: None,
        terminal_health: None,
    };
    if limit.is_zero() || limit > Duration::from_secs(65) {
        return (report, None);
    }
    if collection
        .as_ref()
        .is_some_and(|(_, origin)| Instant::now().checked_duration_since(*origin).is_none())
    {
        report.outcome = Outcome::InvalidStream;
        return (report, None);
    }
    if negotiation.is_some_and(|(pid, nonce, _)| pid == 0 || nonce == 0) {
        report.outcome = Outcome::InvalidStream;
        return (report, None);
    }
    tokio::pin!(cancel);
    if futures_util::FutureExt::now_or_never(cancel.as_mut()).is_some() {
        report.outcome = Outcome::Cancelled;
        return (report, None);
    }
    command
        .stdin(if negotiation.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if collection.is_some() || negotiation.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        report.outcome = Outcome::SpawnFailed;
        return (report, None);
    };
    report.collector_started = true;
    let mut stderr = child.stderr.take().expect("piped collector stderr");
    let mut stdout = child.stdout.take();
    let deadline = tokio::time::sleep(limit);
    tokio::pin!(deadline);
    if let Some((pid, nonce, origin)) = negotiation {
        let input = child.stdin.take().expect("piped collector request");
        let result = tokio::select! {
            biased;
            _ = &mut cancel => Err(Outcome::Cancelled),
            _ = &mut deadline => Err(Outcome::Deadline),
            _ = tokio::time::sleep(Duration::from_secs(2)) => Err(Outcome::Deadline),
            result = super::handshake::negotiate(input, stdout.as_mut().expect("piped collector stdout"), origin, nonce) => result,
        };
        match result.and_then(|clock| clock.event_stream(pid).map_err(|_| Outcome::InvalidStream)) {
            Ok(events) => collection = Some((Collection::new(events, source), origin)),
            Err(outcome) => {
                report.outcome = outcome;
                let _ = child.start_kill();
                match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                    Ok(Ok(_)) => report.collector_reaped = true,
                    _ => report.outcome = Outcome::CleanupFailed,
                }
                return (report, None);
            }
        }
    }
    let mut events_eof = collection.is_none();
    let mut event_buffer = [0_u8; 1024];
    let mut stream = StatusStream::new(source);
    let mut buffer = [0_u8; 1024];
    let mut eof = false;
    let mut exit = None;
    let outcome = loop {
        if eof
            && events_eof
            && let Some(exit) = exit
        {
            if let Some((collection, _)) = collection.take() {
                match collection.finish(exit) {
                    Ok(summary) => {
                        report.outcome = Outcome::Completed;
                        report.status = Some(summary.status);
                        report.terminal_health = Some(summary.collector_health);
                        return (report, Some(summary));
                    }
                    Err(_) => break Outcome::InvalidStream,
                }
            }
            let terminal_health = stream.terminal_health();
            match stream.finish(exit) {
                Ok(status) => {
                    report.outcome = if status.terminal_phase == Phase::Stopped {
                        Outcome::Completed
                    } else {
                        Outcome::CollectorFault
                    };
                    report.status = Some(status);
                    report.terminal_health = terminal_health;
                    return (report, None);
                }
                Err(_) => break Outcome::InvalidStream,
            }
        }
        tokio::select! {
            biased;
            _ = &mut cancel => break Outcome::Cancelled,
            _ = &mut deadline => break Outcome::Deadline,
            read = stderr.read(&mut buffer), if !eof => match read {
                Ok(0) => eof = true,
                Ok(count) => {
                    let valid = if let Some((collection, _)) = collection.as_mut() {
                        collection.push_status(&buffer[..count]).is_ok()
                    } else { stream.push(&buffer[..count]).is_ok() };
                    if !valid { break Outcome::InvalidStream; }
                },
                Err(_) => break Outcome::ReadFailed,
            },
            read = async { stdout.as_mut().expect("piped collector stdout").read(&mut event_buffer).await }, if !events_eof => match read {
                Ok(0) => events_eof = true,
                Ok(count) => {
                    let (collection, origin) = collection.as_mut().expect("event collection");
                    let receipt = Instant::now().checked_duration_since(*origin).and_then(|elapsed| u64::try_from(elapsed.as_nanos()).ok());
                    if receipt.is_none_or(|receipt| collection.push_events(&event_buffer[..count], receipt).is_err()) {
                        break Outcome::InvalidStream;
                    }
                }
                Err(_) => break Outcome::ReadFailed,
            },
            result = child.wait(), if exit.is_none() => match result {
                Ok(status) => { report.collector_reaped = true; exit = Some(status.success()); },
                Err(_) => break Outcome::WaitFailed,
            },
        }
    };
    report.outcome = outcome;
    if !report.collector_reaped {
        let _ = child.start_kill();
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(_)) => report.collector_reaped = true,
            _ => report.outcome = Outcome::CleanupFailed,
        }
    }
    (report, None)
}

#[derive(Serialize)]
pub struct ReplayCheck {
    pub schema_version: u16,
    pub source: &'static str,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub supervision: SupervisedRun,
    pub collection: SupervisedCollection,
    pub negotiated_collection: SupervisedCollection,
    pub response_deadline_ms: u64,
    pub quiet_collection_check: bool,
}

pub async fn generated_replay() -> ReplayCheck {
    let (supervision, collection, negotiated_collection) = tokio::join!(
        generated_status(),
        generated_collection(),
        generated_negotiated_collection()
    );
    ReplayCheck {
        schema_version: 1,
        source: "generated_replay",
        sensor_running: false,
        load_test_performed: false,
        supervision,
        collection,
        negotiated_collection,
        response_deadline_ms: super::handshake::RESPONSE_DEADLINE_MS,
        quiet_collection_check: super::collection::quiet_check(),
    }
}

async fn generated_negotiated_collection() -> SupervisedCollection {
    let Ok(executable) = std::env::current_exe() else {
        return SupervisedCollection {
            schema_version: 1,
            outcome: Outcome::SpawnFailed,
            collector_started: false,
            collector_reaped: false,
            collection: None,
        };
    };
    let mut command = Command::new(executable);
    command.arg("process-negotiated-replay");
    supervise_negotiated_collection(
        command,
        Source::GeneratedReplay,
        42,
        1,
        Duration::from_secs(2),
        std::future::pending(),
    )
    .await
}

async fn generated_collection() -> SupervisedCollection {
    let Ok(executable) = std::env::current_exe() else {
        return SupervisedCollection {
            schema_version: 1,
            outcome: Outcome::SpawnFailed,
            collector_started: false,
            collector_reaped: false,
            collection: None,
        };
    };
    let origin = Instant::now();
    let collection = Collection::new(
        super::events::EventStream::new(42, 0, 0, 0).expect("fixed generated clock anchor"),
        Source::GeneratedReplay,
    );
    let mut command = Command::new(executable);
    command.arg("process-collection-replay");
    supervise_collection(
        command,
        collection,
        origin,
        Duration::from_secs(2),
        std::future::pending(),
    )
    .await
}

async fn generated_status() -> SupervisedRun {
    match std::env::current_exe() {
        Ok(executable) => {
            let mut command = Command::new(executable);
            command.args(["process-replay", "--status-json"]);
            supervise(
                command,
                Source::GeneratedReplay,
                Duration::from_secs(2),
                std::future::pending(),
            )
            .await
        }
        Err(_) => SupervisedRun {
            schema_version: 1,
            outcome: Outcome::SpawnFailed,
            collector_started: false,
            collector_reaped: false,
            status: None,
            terminal_health: None,
        },
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn negotiated_guards_reject_before_spawn_without_clock_or_collection_claims() {
        for (pid, nonce, limit, cancelled, expected) in [
            (0, 1, Duration::from_secs(1), false, Outcome::InvalidStream),
            (42, 0, Duration::from_secs(1), false, Outcome::InvalidStream),
            (42, 1, Duration::ZERO, false, Outcome::InvalidDuration),
            (
                42,
                1,
                Duration::from_secs(66),
                false,
                Outcome::InvalidDuration,
            ),
            (42, 1, Duration::from_secs(1), true, Outcome::Cancelled),
            (42, 1, Duration::from_secs(1), false, Outcome::SpawnFailed),
        ] {
            let run = supervise_negotiated_collection(
                Command::new("/no-such-intellicord-collector"),
                Source::GeneratedReplay,
                pid,
                nonce,
                limit,
                async move {
                    if !cancelled {
                        std::future::pending::<()>().await;
                    }
                },
            )
            .await;
            assert_eq!(run.outcome, expected);
            assert!(!run.collector_started && !run.collector_reaped);
            assert!(run.collection.is_none());
        }
    }

    #[tokio::test]
    async fn collection_pre_spawn_guards_never_confirm_a_started_or_reaped_child() {
        for (limit, future_origin, cancel, expected) in [
            (Duration::ZERO, false, false, Outcome::InvalidDuration),
            (
                Duration::from_secs(66),
                false,
                false,
                Outcome::InvalidDuration,
            ),
            (Duration::from_secs(1), true, false, Outcome::InvalidStream),
            (Duration::from_secs(1), false, true, Outcome::Cancelled),
            (Duration::from_secs(1), false, false, Outcome::SpawnFailed),
        ] {
            let origin = Instant::now()
                + if future_origin {
                    Duration::from_secs(60)
                } else {
                    Duration::ZERO
                };
            let collection = Collection::new(
                super::super::events::EventStream::new(42, 0, 0, 0).unwrap(),
                Source::GeneratedReplay,
            );
            let report = supervise_collection(
                Command::new("/nonexistent/intellicord-test"),
                collection,
                origin,
                limit,
                async move {
                    if !cancel {
                        std::future::pending::<()>().await;
                    }
                },
            )
            .await;
            assert_eq!(report.outcome, expected);
            assert!(!report.collector_started);
            assert!(!report.collector_reaped);
            assert!(report.collection.is_none());
        }
    }

    #[tokio::test]
    async fn early_eof_is_not_success_and_owned_child_is_reaped() {
        let report = supervise(
            Command::new("/bin/cat"),
            Source::GeneratedReplay,
            Duration::from_secs(1),
            std::future::pending(),
        )
        .await;
        assert_eq!(report.outcome, Outcome::InvalidStream);
        assert!(report.collector_started);
        assert!(report.collector_reaped);
        assert!(report.status.is_none());
    }

    #[tokio::test]
    async fn invalid_limits_never_spawn_and_spawn_failure_is_not_cleanup_confirmation() {
        for limit in [Duration::ZERO, Duration::from_secs(66)] {
            let report = supervise(
                Command::new("/bin/cat"),
                Source::GeneratedReplay,
                limit,
                std::future::pending(),
            )
            .await;
            assert_eq!(report.outcome, Outcome::InvalidDuration);
            assert!(!report.collector_started);
            assert!(!report.collector_reaped);
        }
        let report = supervise(
            Command::new("/nonexistent/intellicord-generated-test"),
            Source::GeneratedReplay,
            Duration::from_secs(1),
            std::future::pending(),
        )
        .await;
        assert_eq!(report.outcome, Outcome::SpawnFailed);
        assert!(!report.collector_reaped);
        let cancelled = supervise(
            Command::new("/bin/cat"),
            Source::GeneratedReplay,
            Duration::from_secs(1),
            std::future::ready(()),
        )
        .await;
        assert_eq!(cancelled.outcome, Outcome::Cancelled);
        assert!(!cancelled.collector_started);
    }
}
