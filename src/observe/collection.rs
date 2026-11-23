use serde::Serialize;

use super::{
    ProcessHealth,
    events::{EventStream, EventSummary},
    lifecycle::Phase,
    stream::{Source, StatusStream, ValidatedRun},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CollectionSummary {
    pub schema_version: u16,
    pub status: ValidatedRun,
    pub events: EventSummary,
    pub collector_health: ProcessHealth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionError {
    Events,
    Status,
    Agreement,
    Rejected,
}

pub struct Collection {
    events: EventStream,
    status: StatusStream,
    rejected: bool,
}

impl Collection {
    pub fn new(events: EventStream, source: Source) -> Self {
        Self {
            events,
            status: StatusStream::new(source),
            rejected: false,
        }
    }

    pub fn push_events(&mut self, bytes: &[u8], receipt_ns: u64) -> Result<(), CollectionError> {
        if self.rejected {
            return Err(CollectionError::Rejected);
        }
        if self.events.push(bytes, receipt_ns).is_err() {
            self.rejected = true;
            return Err(CollectionError::Events);
        }
        Ok(())
    }

    pub fn push_status(&mut self, bytes: &[u8]) -> Result<(), CollectionError> {
        if self.rejected {
            return Err(CollectionError::Rejected);
        }
        if self.status.push(bytes).is_err() {
            self.rejected = true;
            return Err(CollectionError::Status);
        }
        Ok(())
    }

    pub fn finish(self, exit_success: bool) -> Result<CollectionSummary, CollectionError> {
        if self.rejected {
            return Err(CollectionError::Rejected);
        }
        let health = self
            .status
            .terminal_health()
            .ok_or(CollectionError::Agreement)?;
        let status = self
            .status
            .finish(exit_success)
            .map_err(|_| CollectionError::Status)?;
        if status.terminal_phase != Phase::Stopped {
            return Err(CollectionError::Agreement);
        }
        let events = self
            .events
            .finish_collection(exit_success)
            .map_err(|_| CollectionError::Events)?;
        if health.accepted != events.health.accepted {
            return Err(CollectionError::Agreement);
        }
        Ok(CollectionSummary {
            schema_version: 1,
            status,
            events,
            collector_health: health,
        })
    }
}

pub fn quiet_check() -> bool {
    let mut collection = Collection::new(
        EventStream::new(42, 1000, 100, 120).expect("fixed generated quiet clock anchor"),
        Source::GeneratedReplay,
    );
    for mut record in super::lifecycle::generated_lifecycle() {
        if let Some(health) = record.health.as_mut() {
            health.accepted = 0;
        }
        let mut bytes = serde_json::to_vec(&record).expect("fixed generated quiet lifecycle");
        bytes.push(b'\n');
        if collection.push_status(&bytes).is_err() {
            return false;
        }
    }
    collection.finish(true).is_ok_and(|summary| {
        summary.events.records == 0
            && summary.events.health.accepted == 0
            && summary.collector_health.accepted == 0
            && summary.collector_health.ring_dropped == 3
            && !summary.status.attachment_confirmed
            && !summary.status.detachment_confirmed
    })
}

pub fn write_generated() -> anyhow::Result<()> {
    write_generated_at(0)
}

pub async fn write_negotiated_generated() -> anyhow::Result<()> {
    super::handshake::generated_response().await?;
    write_generated_at(1000)
}

fn write_generated_at(source_ns: u64) -> anyhow::Result<()> {
    use std::io::Write;
    let mut output = super::export::EventExporter::new(42, std::io::stdout())?;
    for _ in 0..2 {
        output.emit(&super::timed::generated_record(42, source_ns))?;
    }
    let mut status = std::io::stderr().lock();
    for mut record in super::lifecycle::generated_lifecycle() {
        if let Some(health) = record.health.as_mut() {
            *health = output.health();
        }
        writeln!(status, "{}", serde_json::to_string(&record)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_collection_requires_complete_lifecycle_zero_count_and_successful_exit() {
        assert!(quiet_check());
        for (accepted, event_bytes, complete, exit, expected) in [
            (0, 0, true, true, true),
            (1, 0, true, true, false),
            (0, 1, true, true, false),
            (0, 0, false, true, false),
            (0, 0, true, false, false),
        ] {
            let mut collection = Collection::new(
                EventStream::new(42, 1000, 100, 120).unwrap(),
                Source::GeneratedReplay,
            );
            let mut status = Vec::new();
            for mut record in super::super::lifecycle::generated_lifecycle() {
                if let Some(health) = record.health.as_mut() {
                    health.accepted = accepted;
                }
                serde_json::to_writer(&mut status, &record).unwrap();
                status.push(b'\n');
            }
            if !complete {
                status.pop();
            }
            collection.push_status(&status).unwrap();
            collection.push_events(&vec![1; event_bytes], 1300).unwrap();
            let summary = collection.finish(exit);
            assert_eq!(summary.is_ok(), expected);
            if let Ok(summary) = summary {
                assert_eq!(summary.events.records, 0);
                assert_eq!(summary.events.health.accepted, 0);
                assert_eq!(summary.collector_health.ring_dropped, 3);
            }
        }
        assert!(
            EventStream::new(42, 1000, 100, 120)
                .unwrap()
                .finish(true)
                .is_err()
        );
    }

    #[test]
    fn either_stream_failure_poisoning_blocks_the_other_stream() {
        let make = || {
            Collection::new(
                EventStream::new(42, 0, 0, 0).unwrap(),
                Source::GeneratedReplay,
            )
        };
        let bytes = super::super::timed::generated_record(42, 0);
        let mut bad_status = make();
        assert_eq!(
            bad_status.push_status(b"{}\n"),
            Err(CollectionError::Status)
        );
        assert_eq!(
            bad_status.push_events(&bytes, 0),
            Err(CollectionError::Rejected)
        );
        assert_eq!(bad_status.finish(true), Err(CollectionError::Rejected));
        let mut bad_event = make();
        assert_eq!(
            bad_event.push_events(&[0; 24], 0),
            Err(CollectionError::Events)
        );
        assert_eq!(
            bad_event.push_status(b"{}\n"),
            Err(CollectionError::Rejected)
        );
        assert_eq!(bad_event.finish(true), Err(CollectionError::Rejected));
    }

    #[test]
    fn terminal_counts_detect_whole_record_loss_and_require_both_complete_streams() {
        let mut status = Vec::new();
        for record in super::super::lifecycle::generated_lifecycle() {
            serde_json::to_writer(&mut status, &record).unwrap();
            status.push(b'\n');
        }
        let bytes = super::super::timed::generated_record(42, 2000).repeat(2);
        for (events, lifecycle, exit, expected) in [
            (&bytes[..], &status[..], true, true),
            (&bytes[..24], &status[..], true, false),
            (&bytes[..47], &status[..], true, false),
            (&bytes[..], &status[..status.len() - 1], true, false),
            (&bytes[..], &status[..], false, false),
        ] {
            let mut collection = Collection::new(
                EventStream::new(42, 1000, 100, 120).unwrap(),
                Source::GeneratedReplay,
            );
            for chunk in lifecycle.chunks(7) {
                collection.push_status(chunk).unwrap();
            }
            for chunk in events.chunks(1) {
                collection.push_events(chunk, 1300).unwrap();
            }
            let summary = collection.finish(exit);
            assert_eq!(summary.is_ok(), expected);
            if let Ok(summary) = summary {
                assert_eq!(summary.events.records, 2);
                assert_eq!(summary.collector_health.ring_dropped, 3);
                assert!(!summary.status.attachment_confirmed);
            }
        }
    }
}
