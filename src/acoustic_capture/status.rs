use std::{io::Write, sync::mpsc, time::Duration};

use crate::acoustic::{AcousticFeed, OutputHealth, RepetitionFeatures};
use crate::ingestion::AcousticState;
use anyhow::Result;
use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapturePhase {
    Discovering,
    RecorderRequested,
    AwaitingFrames,
    ReceivingFrames,
    Stopping,
    Stopped,
    Faulted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CaptureReason {
    DurationLimit,
    Interrupt,
    Terminate,
    OwnerCancelled,
    DiscoveryFailed,
    SignalSetupFailed,
    SourceRejected,
    RecorderStartFailed,
    InputStalled,
    StreamEnded,
    StreamReadFailed,
    ProcessingFailed,
    DeliveryFailed,
    RecorderExited,
    CleanupFailed,
    InternalFailure,
}

impl CaptureReason {
    pub(super) fn from_stop(reason: &str) -> Self {
        match reason {
            "duration_limit" => Self::DurationLimit,
            "interrupt" => Self::Interrupt,
            "terminate" => Self::Terminate,
            "owner_cancelled" => Self::OwnerCancelled,
            _ => Self::InternalFailure,
        }
    }

    pub(super) fn from_error(error: &anyhow::Error) -> Self {
        error
            .downcast_ref::<Self>()
            .copied()
            .unwrap_or(Self::InternalFailure)
    }
}

impl std::fmt::Display for CaptureReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for CaptureReason {}

#[derive(Serialize)]
struct CaptureStatus {
    schema_version: u16,
    record_type: &'static str,
    sequence: u64,
    phase: CapturePhase,
    reason: Option<CaptureReason>,
    recorder_reaped: bool,
    health: Option<OutputHealth>,
}

#[derive(Default)]
pub(super) struct StatusReporter {
    json: bool,
    sequence: u64,
    terminal: bool,
    writer: Option<StatusWriter>,
    feed: Option<AcousticFeed>,
}

impl StatusReporter {
    pub(super) fn with_feed(feed: AcousticFeed) -> Self {
        Self {
            feed: Some(feed),
            ..Self::default()
        }
    }

    pub(super) fn frame(&mut self, features: &RepetitionFeatures) -> Result<()> {
        if let Some(feed) = &mut self.feed {
            feed.frame(features).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }

    pub(super) fn frame_clock_now(&self) -> Option<u64> {
        self.feed.as_ref().map(AcousticFeed::elapsed_ns)
    }

    pub(super) fn prepare_frame_clock(&mut self, earliest_ns: Option<u64>) -> Result<()> {
        match (&mut self.feed, earliest_ns) {
            (Some(feed), Some(earliest_ns)) => feed
                .prepare_frame_clock(earliest_ns)
                .map_err(anyhow::Error::msg),
            (None, None) => Ok(()),
            _ => Err(anyhow::anyhow!("incomplete acoustic frame clock anchor")),
        }
    }

    pub(super) fn new(json: bool) -> Self {
        Self {
            json,
            writer: json.then(StatusWriter::new),
            ..Self::default()
        }
    }

    pub(super) fn is_json(&self) -> bool {
        self.json
    }

    pub(super) fn is_terminal(&self) -> bool {
        self.terminal
    }

    pub(super) fn emit(
        &mut self,
        phase: CapturePhase,
        health: Option<OutputHealth>,
        recorder_reaped: bool,
    ) -> Result<()> {
        self.emit_reason(phase, health, recorder_reaped, None)
    }

    pub(super) fn emit_reason(
        &mut self,
        phase: CapturePhase,
        health: Option<OutputHealth>,
        recorder_reaped: bool,
        reason: Option<CaptureReason>,
    ) -> Result<()> {
        if self.terminal {
            return Ok(());
        }
        if let Some(feed) = &mut self.feed {
            let state = match phase {
                CapturePhase::Discovering
                | CapturePhase::RecorderRequested
                | CapturePhase::AwaitingFrames => Some(AcousticState::Starting),
                CapturePhase::ReceivingFrames => Some(AcousticState::Receiving),
                CapturePhase::Stopped => Some(AcousticState::Stopped),
                CapturePhase::Faulted => Some(AcousticState::Faulted),
                CapturePhase::Stopping => None,
            };
            if let Some(state) = state {
                feed.state(state);
                if let Some(health) = health {
                    feed.health(health);
                }
            }
        }
        if !self.json {
            self.terminal = matches!(phase, CapturePhase::Stopped | CapturePhase::Faulted);
            return Ok(());
        }
        self.sequence += 1;
        let record = CaptureStatus {
            schema_version: 2,
            record_type: "capture_status",
            sequence: self.sequence,
            phase,
            reason,
            recorder_reaped,
            health,
        };
        let mut bytes = serde_json::to_vec(&record)?;
        anyhow::ensure!(bytes.len() < 2048, "status record exceeds limit");
        bytes.push(b'\n');
        self.writer
            .as_mut()
            .expect("JSON status writer")
            .write(bytes)?;
        self.terminal = matches!(phase, CapturePhase::Stopped | CapturePhase::Faulted);
        Ok(())
    }
}

struct StatusWriter {
    sender: mpsc::SyncSender<Vec<u8>>,
    acknowledged: mpsc::Receiver<std::io::Result<()>>,
    failed: bool,
}

impl StatusWriter {
    fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(1);
        let (acknowledge, acknowledged) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(bytes) = receiver.recv() {
                let result = std::io::stderr().lock().write_all(&bytes);
                let failed = result.is_err();
                if acknowledge.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Self {
            sender,
            acknowledged,
            failed: false,
        }
    }

    fn write(&mut self, bytes: Vec<u8>) -> Result<()> {
        anyhow::ensure!(!self.failed, "status output unavailable");
        self.failed = true;
        self.sender.try_send(bytes)?;
        self.acknowledged
            .recv_timeout(Duration::from_millis(100))??;
        self.failed = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_classification_uses_typed_context_not_diagnostic_text() {
        let misleading = anyhow::anyhow!("terminate input_stalled discovery_failed");
        assert_eq!(
            CaptureReason::from_error(&misleading),
            CaptureReason::InternalFailure
        );
        let typed = misleading
            .context(CaptureReason::StreamReadFailed)
            .context("outer diagnostic");
        assert_eq!(
            CaptureReason::from_error(&typed),
            CaptureReason::StreamReadFailed
        );
        assert_eq!(
            CaptureReason::from_stop("unknown"),
            CaptureReason::InternalFailure
        );
    }

    #[test]
    fn stalled_status_writer_times_out_and_never_queues_more_output() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (_acknowledge, acknowledged) = mpsc::sync_channel(1);
        let mut writer = StatusWriter {
            sender,
            acknowledged,
            failed: false,
        };
        assert!(writer.write(vec![1]).is_err());
        assert!(writer.failed);
        assert!(writer.write(vec![2]).is_err());
        assert_eq!(receiver.try_recv().unwrap(), vec![1]);
        assert!(receiver.try_recv().is_err());
    }
}
