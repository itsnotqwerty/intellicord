use std::{process::Stdio, time::Duration};

use tokio::{io::AsyncReadExt, process::Command};

use super::{CaptureReport, Outcome};

pub(super) struct Inspection {
    pub outcome: Outcome,
    pub reaped: bool,
    pub report: Option<CaptureReport>,
}

impl Inspection {
    pub(super) fn status(&self) -> &'static str {
        match self.outcome {
            Outcome::CleanupFailed => "cleanup_unconfirmed",
            Outcome::SpawnFailed => "inspection_failed",
            _ if !self.reaped => "cleanup_unconfirmed",
            Outcome::Deadline => "inspection_timed_out",
            Outcome::Completed if self.report.is_some() => "supplied",
            Outcome::Completed | Outcome::InvalidStream => "rejected",
            _ => "inspection_failed",
        }
    }
}

pub(super) async fn run(mut command: Command) -> Inspection {
    let mut inspection = Inspection {
        outcome: Outcome::SpawnFailed,
        reaped: false,
        report: None,
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        return inspection;
    };
    let mut output = child
        .stdout
        .take()
        .expect("piped health helper stdout")
        .take(4097);
    let mut bytes = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        output
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Outcome::ReadFailed)?;
        if bytes.len() > 4096 {
            return Err(Outcome::InvalidStream);
        }
        let report: Option<CaptureReport> =
            serde_json::from_slice(&bytes).map_err(|_| Outcome::InvalidStream)?;
        if report.is_some_and(|report| !report.valid_completed()) {
            return Err(Outcome::InvalidStream);
        }
        let exit = child.wait().await.map_err(|_| Outcome::WaitFailed)?;
        inspection.reaped = true;
        if !exit.success() {
            return Err(Outcome::CollectorFault);
        }
        Ok(report)
    })
    .await;
    match result {
        Ok(Ok(report)) => {
            inspection.outcome = Outcome::Completed;
            inspection.report = report;
        }
        Ok(Err(outcome)) => inspection.outcome = outcome,
        Err(_) => inspection.outcome = Outcome::Deadline,
    }
    if !inspection.reaped {
        let _ = child.start_kill();
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(_)) => inspection.reaped = true,
            _ => inspection.outcome = Outcome::CleanupFailed,
        }
    }
    inspection
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_helper_requires_valid_reply_eof_and_successful_exit() {
        let payload = serde_json::to_string(&super::super::tests::fixture()).unwrap();
        for (script, expected, supplied) in [
            (
                "process.stdout.write(process.argv[1])",
                Outcome::Completed,
                true,
            ),
            ("process.stdout.write('null')", Outcome::Completed, false),
            ("process.stdout.write('{}')", Outcome::InvalidStream, false),
            (
                "process.stdout.write(' '.repeat(4097)); setInterval(() => {}, 1000)",
                Outcome::InvalidStream,
                false,
            ),
            (
                "process.stdout.write(process.argv[1]); process.exitCode = 1",
                Outcome::CollectorFault,
                false,
            ),
            ("setInterval(() => {}, 1000)", Outcome::Deadline, false),
            (
                "require('node:fs').writeSync(1, process.argv[1]); require('node:fs').closeSync(1); setInterval(() => {}, 1000)",
                Outcome::Deadline,
                false,
            ),
            (
                "const report = JSON.parse(process.argv[1]); report.sensor_running = true; process.stdout.write(JSON.stringify(report))",
                Outcome::InvalidStream,
                false,
            ),
        ] {
            let mut command = Command::new("node");
            command.args(["-e", script, &payload]);
            let inspection = tokio::time::timeout(Duration::from_secs(6), run(command))
                .await
                .unwrap();
            assert_eq!(inspection.outcome, expected, "{script}");
            assert!(inspection.reaped);
            assert_eq!(inspection.report.is_some(), supplied);
            assert_eq!(
                inspection.status(),
                match expected {
                    Outcome::Completed if supplied => "supplied",
                    Outcome::Completed | Outcome::InvalidStream => "rejected",
                    Outcome::Deadline => "inspection_timed_out",
                    _ => "inspection_failed",
                }
            );
        }
        let inspection = run(Command::new("/nonexistent/intellicord-health-helper")).await;
        assert_eq!(inspection.outcome, Outcome::SpawnFailed);
        assert!(!inspection.reaped);
        assert!(inspection.report.is_none());
        assert_eq!(inspection.status(), "inspection_failed");
    }

    #[test]
    fn unconfirmed_cleanup_never_claims_rejection_or_timeout_alone() {
        for outcome in [
            Outcome::CleanupFailed,
            Outcome::Deadline,
            Outcome::Completed,
        ] {
            let inspection = Inspection {
                outcome,
                reaped: false,
                report: None,
            };
            assert_eq!(inspection.status(), "cleanup_unconfirmed");
        }
        let inspection = Inspection {
            outcome: Outcome::CleanupFailed,
            reaped: true,
            report: None,
        };
        assert_eq!(inspection.status(), "cleanup_unconfirmed");
    }
}
