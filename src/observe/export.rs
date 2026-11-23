use std::{io::Write, sync::mpsc, time::Duration};

use intellicord_observe_common::EVENT_BYTES;

use super::{ProcessHealth, ProcessMonitor, events::RECORD_LIMIT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportError {
    Record,
    Limit,
    Output,
    Rejected,
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "process event output rejected: {self:?}")
    }
}

impl std::error::Error for ExportError {}

pub struct EventExporter {
    monitor: ProcessMonitor,
    sender: mpsc::SyncSender<[u8; EVENT_BYTES]>,
    acknowledged: mpsc::Receiver<std::io::Result<()>>,
    failed: bool,
}

impl EventExporter {
    pub fn new(process_id: u32, mut output: impl Write + Send + 'static) -> anyhow::Result<Self> {
        let monitor =
            ProcessMonitor::new(process_id).ok_or_else(|| anyhow::anyhow!("invalid process ID"))?;
        let (sender, receiver) = mpsc::sync_channel::<[u8; EVENT_BYTES]>(1);
        let (acknowledge, acknowledged) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("process-events".into())
            .spawn(move || {
                while let Ok(bytes) = receiver.recv() {
                    let result = output.write_all(&bytes).and_then(|()| output.flush());
                    let failed = result.is_err();
                    if acknowledge.send(result).is_err() || failed {
                        break;
                    }
                }
            })?;
        Ok(Self {
            monitor,
            sender,
            acknowledged,
            failed: false,
        })
    }

    pub fn emit(&mut self, bytes: &[u8]) -> Result<(), ExportError> {
        if self.failed {
            return Err(ExportError::Rejected);
        }
        self.failed = true;
        if self.monitor.health().accepted >= RECORD_LIMIT as u64 {
            return Err(ExportError::Limit);
        }
        if !self.monitor.accept(bytes) {
            return Err(ExportError::Record);
        }
        let record = bytes.try_into().map_err(|_| ExportError::Record)?;
        self.sender
            .try_send(record)
            .map_err(|_| ExportError::Output)?;
        self.acknowledged
            .recv_timeout(Duration::from_millis(100))
            .map_err(|_| ExportError::Output)?
            .map_err(|_| ExportError::Output)?;
        self.failed = false;
        Ok(())
    }

    pub fn health(&self) -> ProcessHealth {
        self.monitor.health()
    }

    pub fn update_ring_dropped(&mut self, count: u64) {
        self.monitor.update_ring_dropped(count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct BrokenOutput {
        flush_fails: bool,
    }

    impl Write for BrokenOutput {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.flush_fails {
                Ok(bytes.len())
            } else {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }

    #[test]
    fn write_and_flush_failures_are_sticky_even_after_bytes_may_have_escaped() {
        for flush_fails in [false, true] {
            let mut exporter = EventExporter::new(42, BrokenOutput { flush_fails }).unwrap();
            let record = super::super::timed::generated_record(42, 0);
            assert_eq!(exporter.emit(&record), Err(ExportError::Output));
            assert_eq!(exporter.emit(&record), Err(ExportError::Rejected));
        }
    }

    struct Output(Arc<Mutex<Vec<u8>>>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn output_is_exact_native_records_and_limit_failure_is_sticky() {
        let bytes = super::super::timed::generated_record(42, 0);
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut exporter = EventExporter::new(42, Output(output.clone())).unwrap();
        for _ in 0..RECORD_LIMIT {
            exporter.emit(&bytes).unwrap();
        }
        assert_eq!(*output.lock().unwrap(), bytes.repeat(RECORD_LIMIT));
        assert_eq!(exporter.emit(&bytes), Err(ExportError::Limit));
        assert_eq!(exporter.emit(&bytes), Err(ExportError::Rejected));
        assert_eq!(exporter.health().accepted, RECORD_LIMIT as u64);
    }

    #[test]
    fn malformed_or_foreign_records_never_reach_output() {
        for bytes in [
            vec![0; EVENT_BYTES],
            super::super::timed::generated_record(43, 0),
        ] {
            let output = Arc::new(Mutex::new(Vec::new()));
            let mut exporter = EventExporter::new(42, Output(output.clone())).unwrap();
            assert_eq!(exporter.emit(&bytes), Err(ExportError::Record));
            assert!(output.lock().unwrap().is_empty());
            assert_eq!(
                exporter.emit(&super::super::timed::generated_record(42, 0)),
                Err(ExportError::Rejected)
            );
        }
    }

    #[test]
    fn stalled_acknowledgement_times_out_without_more_output() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (_acknowledge, acknowledged) = mpsc::sync_channel(1);
        let mut exporter = EventExporter {
            monitor: ProcessMonitor::new(42).unwrap(),
            sender,
            acknowledged,
            failed: false,
        };
        let bytes = super::super::timed::generated_record(42, 0);
        assert_eq!(exporter.emit(&bytes), Err(ExportError::Output));
        assert_eq!(exporter.emit(&bytes), Err(ExportError::Rejected));
        assert_eq!(receiver.try_recv().unwrap().as_slice(), bytes);
        assert!(receiver.try_recv().is_err());
    }
}
