use serde::{Deserialize, Serialize};

#[cfg(any(test, all(feature = "observe", target_os = "linux")))]
mod inspection;

use super::{
    lifecycle::{Phase, Reason},
    stream::Source,
    supervisor::{Outcome, SupervisedRun},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureReport {
    pub schema_version: u16,
    pub source: ReportSource,
    pub sensor_running: bool,
    pub run: SupervisedRun,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReportSource {
    #[serde(rename = "supervised_kernel_capture")]
    SupervisedKernelCapture,
}

impl CaptureReport {
    pub fn new(run: SupervisedRun) -> Self {
        Self {
            schema_version: 1,
            source: ReportSource::SupervisedKernelCapture,
            sensor_running: false,
            run,
        }
    }

    pub fn valid_completed(&self) -> bool {
        let run = &self.run;
        self.schema_version == 1
            && !self.sensor_running
            && run.schema_version == 1
            && run.outcome == Outcome::Completed
            && run.collector_started
            && run.collector_reaped
            && run.status.is_some_and(|status| {
                status.schema_version == 1
                    && status.source == Source::KernelTracepoint
                    && status.records == 6
                    && status.terminal_phase == Phase::Stopped
                    && matches!(
                        status.reason,
                        Reason::DurationLimit
                            | Reason::Interrupt
                            | Reason::Terminate
                            | Reason::TargetExited
                    )
                    && status.attachment_confirmed
                    && status.detachment_confirmed
            })
            && run.terminal_health.is_some_and(|health| {
                health.schema_version == 1
                    && [
                        health.accepted,
                        health.rejected_record,
                        health.rejected_clock,
                        health.ring_dropped,
                    ]
                    .into_iter()
                    .all(|count| count <= 9_007_199_254_740_991)
            })
    }
}

#[derive(Serialize)]
pub struct Snapshot {
    pub schema_version: u16,
    pub source: &'static str,
    pub status: &'static str,
    pub sensor_running: bool,
    pub report: Option<CaptureReport>,
}

pub fn decode(bytes: &[u8]) -> Option<CaptureReport> {
    if bytes.len() > 4096 {
        return None;
    }
    let report: CaptureReport = serde_json::from_slice(bytes).ok()?;
    report.valid_completed().then_some(report)
}

pub async fn check() -> Snapshot {
    let mut snapshot = Snapshot {
        schema_version: 1,
        source: "operator_supplied_snapshot",
        status: "not_configured",
        sensor_running: false,
        report: None,
    };
    let Some(_path) = std::env::var_os("INTELLICORD_OBSERVE_HEALTH_REPORT") else {
        return snapshot;
    };
    snapshot.status = "unavailable";
    #[cfg(all(feature = "observe", target_os = "linux"))]
    {
        snapshot.status = "inspection_failed";
        if let Ok(executable) = std::env::current_exe() {
            let mut command = tokio::process::Command::new(executable);
            command.arg("health-snapshot-inspect");
            let result = inspection::run(command).await;
            snapshot.status = result.status();
            if snapshot.status == "supplied" {
                snapshot.report = result.report;
            }
        }
    }
    snapshot
}

pub fn read_configured_report() -> Option<CaptureReport> {
    #[cfg(all(feature = "observe", target_os = "linux"))]
    {
        use std::{io::Read, os::unix::fs::OpenOptionsExt};
        let path = std::env::var_os("INTELLICORD_OBSERVE_HEALTH_REPORT")?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(path)
            .ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > 4096 {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes).ok()?;
        decode(&bytes)
    }
    #[cfg(not(all(feature = "observe", target_os = "linux")))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::{ProcessHealth, stream::ValidatedRun};

    pub(super) fn fixture() -> CaptureReport {
        CaptureReport::new(SupervisedRun {
            schema_version: 1,
            outcome: Outcome::Completed,
            collector_started: true,
            collector_reaped: true,
            status: Some(ValidatedRun {
                schema_version: 1,
                source: Source::KernelTracepoint,
                records: 6,
                terminal_phase: Phase::Stopped,
                reason: Reason::DurationLimit,
                attachment_confirmed: true,
                detachment_confirmed: true,
            }),
            terminal_health: Some(ProcessHealth {
                schema_version: 1,
                accepted: 2,
                rejected_record: 0,
                rejected_clock: 0,
                ring_dropped: 3,
            }),
        })
    }

    #[test]
    fn supplied_report_requires_completed_consistent_private_shape() {
        let value = serde_json::to_value(fixture()).unwrap();
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(bytes.len() < 1024);
        assert!(decode(&bytes).is_some());
        for pointer in [
            "/sensor_running",
            "/run/collector_reaped",
            "/run/status/detachment_confirmed",
        ] {
            let mut invalid = value.clone();
            let field = invalid.pointer_mut(pointer).unwrap();
            *field = serde_json::json!(!field.as_bool().unwrap());
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
        for (pointer, replacement) in [
            ("/run/terminal_health", serde_json::Value::Null),
            ("/run/status/source", serde_json::json!("generated_replay")),
            ("/run/outcome", serde_json::json!("deadline")),
            (
                "/run/terminal_health/accepted",
                serde_json::json!(9_007_199_254_740_992_u64),
            ),
        ] {
            let mut invalid = value.clone();
            *invalid.pointer_mut(pointer).unwrap() = replacement;
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
        let mut invalid = value;
        invalid["path"] = serde_json::json!("private");
        assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
        assert!(decode(&vec![b' '; 4097]).is_none());
    }
}
