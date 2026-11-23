use std::{future::Future, path::Path, time::Duration};

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use tokio::{net::UnixListener, process::Command};

use super::{
    client::{OwnerResult, OwnerSnapshot, OwnerStatus},
    endpoint::Endpoint,
    socket_owner,
    supervisor::{HealthReceiver, HealthSender, Run, health_channel},
};
use crate::observe::{stream::Source, supervisor::Outcome};

pub struct Capture {
    endpoint: Endpoint,
    listener: UnixListener,
    session: String,
    command: Command,
    seconds: u64,
    sender: HealthSender,
    outcome: tokio::sync::watch::Sender<OwnerSnapshot>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u16,
    pub source: &'static str,
    pub transport: &'static str,
    pub session_binding: &'static str,
    pub endpoint_removed: bool,
    pub run: Run,
}

impl Report {
    pub fn completed(&self) -> bool {
        self.endpoint_removed && self.run.outcome == Outcome::Completed
    }
}

impl Capture {
    pub fn outcome(&self) -> tokio::sync::watch::Receiver<OwnerSnapshot> {
        self.outcome.subscribe()
    }

    pub async fn run_with<T>(self, work: impl Future<Output = T>) -> (T, Report) {
        let (cancel, stopped) = tokio::sync::oneshot::channel();
        let work = async move {
            let result = work.await;
            let _ = cancel.send(());
            result
        };
        tokio::join!(
            work,
            self.run(async {
                let _ = stopped.await;
            })
        )
    }

    pub fn prepare(
        collector: &Path,
        root: &Path,
        object: &Path,
        process_id: u32,
        seconds: u64,
    ) -> Result<(Self, HealthReceiver)> {
        Self::prepare_command(collector, root, Some(object), process_id, seconds)
    }

    pub fn prepare_privileged(
        helper: &Path,
        root: &Path,
        process_id: u32,
        seconds: u64,
    ) -> Result<(Self, HealthReceiver)> {
        Self::prepare_command(helper, root, None, process_id, seconds)
    }

    fn prepare_command(
        collector: &Path,
        root: &Path,
        object: Option<&Path>,
        process_id: u32,
        seconds: u64,
    ) -> Result<(Self, HealthReceiver)> {
        ensure!(
            collector.is_absolute()
                && object.is_none_or(|object| !object.as_os_str().is_empty())
                && (1..=i32::MAX as u32).contains(&process_id)
                && (1..=60).contains(&seconds),
            "invalid collector, object, target or duration"
        );
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).context("Progress session entropy unavailable")?;
        let session: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let (endpoint, listener) = Endpoint::bind(root, &session)?;
        let mut command = Command::new(collector);
        command.env_clear().arg("capture-socket-progress");
        if let Some(object) = object {
            command.arg("--object").arg(object);
        }
        command
            .args([
                "--pid",
                &process_id.to_string(),
                "--seconds",
                &seconds.to_string(),
            ])
            .args(["--session", &session])
            .arg("--socket")
            .arg(endpoint.path())
            .args(["--parent-pid", &std::process::id().to_string()])
            .arg(if object.is_some() {
                "--acknowledge-socket-progress"
            } else {
                "--acknowledge-privileged-observation"
            });
        let (sender, receiver) = health_channel();
        let (outcome, _) = tokio::sync::watch::channel(OwnerSnapshot::pending());
        Ok((
            Self {
                endpoint,
                listener,
                session,
                command,
                seconds,
                sender,
                outcome,
            },
            receiver,
        ))
    }

    pub async fn run(self, cancel: impl Future<Output = ()>) -> Report {
        let run = socket_owner::supervise(
            self.command,
            self.listener,
            &self.session,
            Source::KernelTracepoint,
            socket_owner::Limits {
                total: Duration::from_secs(self.seconds + 3),
                idle: Duration::from_secs(3),
                accept: Duration::from_secs(2),
            },
            cancel,
            self.sender,
        )
        .await;
        let report = Report {
            schema_version: 1,
            source: "supervised_kernel_socket_progress",
            transport: "kernel_verified_unix_socket",
            session_binding: "parent_selected",
            endpoint_removed: self.endpoint.close().is_ok(),
            run,
        };
        self.outcome.send_replace(OwnerSnapshot {
            status: OwnerStatus::Finished,
            result: Some(OwnerResult {
                outcome: report.run.outcome,
                collector_started: report.run.collector_started,
                collector_reaped: report.run.collector_reaped,
                endpoint_removed: report.endpoint_removed,
            }),
            ..OwnerSnapshot::pending()
        });
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn privileged_capture_passes_no_object_and_uses_distinct_consent() {
        let (capture, _receiver) = Capture::prepare_privileged(
            Path::new("/usr/libexec/intellicord/intellicord-bpf-helper"),
            &std::env::temp_dir(),
            123,
            5,
        )
        .unwrap();
        let command = capture.command.as_std();
        assert_eq!(
            command.get_program(),
            "/usr/libexec/intellicord/intellicord-bpf-helper"
        );
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(arguments.len(), 12);
        assert_eq!(arguments[0], "capture-socket-progress");
        assert_eq!(&arguments[1..5], ["--pid", "123", "--seconds", "5"]);
        assert_eq!(arguments[5], "--session");
        assert_eq!(arguments[6].len(), 32);
        assert_eq!(arguments[7], "--socket");
        assert!(Path::new(&arguments[8]).is_absolute());
        assert_eq!(arguments[9], "--parent-pid");
        assert_eq!(arguments[10], std::process::id().to_string());
        assert_eq!(arguments[11], "--acknowledge-privileged-observation");
        assert!(!arguments.iter().any(|argument| argument == "--object"));
    }
}
