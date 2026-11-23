use std::time::Duration;

use serde::Serialize;

use super::{ProcessHealth, Source, supervisor::HealthReceiver};

const MAX_AGE: Duration = Duration::from_secs(2);
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerStatus {
    NotConfigured,
    AwaitingCompletion,
    Finished,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct OwnerResult {
    pub outcome: crate::observe::supervisor::Outcome,
    pub collector_started: bool,
    pub collector_reaped: bool,
    pub endpoint_removed: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct OwnerSnapshot {
    pub schema_version: u16,
    pub source: &'static str,
    pub status: OwnerStatus,
    pub result: Option<OwnerResult>,
}

impl OwnerSnapshot {
    pub fn pending() -> Self {
        Self {
            schema_version: 1,
            source: "capture_owner_outcome",
            status: OwnerStatus::AwaitingCompletion,
            result: None,
        }
    }

    pub fn read(receiver: Option<&tokio::sync::watch::Receiver<Self>>) -> Self {
        let Some(receiver) = receiver else {
            return Self {
                status: OwnerStatus::NotConfigured,
                ..Self::pending()
            };
        };
        let mut snapshot = *receiver.borrow();
        if snapshot.status == OwnerStatus::AwaitingCompletion && receiver.has_changed().is_err() {
            snapshot.status = OwnerStatus::Unavailable;
        }
        snapshot
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;

    #[test]
    fn closed_pending_owner_is_unavailable_but_finished_result_survives_drop() {
        assert_eq!(OwnerSnapshot::read(None).status, OwnerStatus::NotConfigured);
        let (sender, receiver) = tokio::sync::watch::channel(OwnerSnapshot::pending());
        assert_eq!(
            OwnerSnapshot::read(Some(&receiver)).status,
            OwnerStatus::AwaitingCompletion
        );
        drop(sender);
        assert_eq!(
            OwnerSnapshot::read(Some(&receiver)).status,
            OwnerStatus::Unavailable
        );
        let (sender, receiver) = tokio::sync::watch::channel(OwnerSnapshot::pending());
        sender.send_replace(OwnerSnapshot {
            status: OwnerStatus::Finished,
            result: Some(OwnerResult {
                outcome: crate::observe::supervisor::Outcome::CollectorFault,
                collector_started: true,
                collector_reaped: true,
                endpoint_removed: false,
            }),
            ..OwnerSnapshot::pending()
        });
        drop(sender);
        let value = serde_json::to_value(OwnerSnapshot::read(Some(&receiver))).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert_eq!(value["status"], "finished");
        assert_eq!(value["result"].as_object().unwrap().len(), 4);
        assert_eq!(value["result"]["endpoint_removed"], false);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    NotConnected,
    NoRecentSample,
    GeneratedRejected,
    InvalidSample,
    RecentKernelSample,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub schema_version: u16,
    pub source: &'static str,
    pub status: Status,
    pub sensor_running: bool,
    pub valid_for_ms: u64,
    pub sample: Option<ProcessHealth>,
}

impl Snapshot {
    pub fn read(receiver: Option<&HealthReceiver>) -> Self {
        let mut snapshot = Self {
            schema_version: 1,
            source: "owner_progress_subscription",
            status: Status::NotConnected,
            sensor_running: false,
            valid_for_ms: 0,
            sample: None,
        };
        let Some(receiver) = receiver else {
            return snapshot;
        };
        snapshot.status = Status::NoRecentSample;
        let Some((sample, remaining)) = receiver.latest_with_remaining(MAX_AGE) else {
            return snapshot;
        };
        if sample.source != Source::KernelTracepoint {
            snapshot.status = Status::GeneratedRejected;
            return snapshot;
        }
        let health = sample.health;
        if health.schema_version != 1
            || [
                health.accepted,
                health.rejected_record,
                health.rejected_clock,
                health.ring_dropped,
            ]
            .into_iter()
            .any(|count| count > MAX_SAFE_INTEGER)
        {
            snapshot.status = Status::InvalidSample;
            return snapshot;
        }
        let valid_for_ms = remaining.as_millis() as u64;
        if valid_for_ms == 0 {
            return snapshot;
        }
        snapshot.status = Status::RecentKernelSample;
        snapshot.valid_for_ms = valid_for_ms;
        snapshot.sample = Some(health);
        snapshot
    }
}
