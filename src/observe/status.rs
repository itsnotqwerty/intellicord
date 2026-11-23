use std::{io::Write, sync::mpsc, time::Duration};

use super::{
    ProcessHealth,
    lifecycle::{Lifecycle, Phase, Reason},
};

pub struct StatusReporter {
    lifecycle: Lifecycle,
    writer: Option<StatusWriter>,
}

impl StatusReporter {
    pub fn new(json: bool) -> Self {
        Self {
            lifecycle: Lifecycle::new(false),
            writer: json.then(StatusWriter::new),
        }
    }

    pub fn is_json(&self) -> bool {
        self.writer.is_some()
    }

    pub fn emit(
        &mut self,
        phase: Phase,
        reason: Option<Reason>,
        health: Option<ProcessHealth>,
    ) -> anyhow::Result<()> {
        let record = self.lifecycle.advance(phase, reason, health)?;
        if let Some(writer) = &mut self.writer {
            let mut bytes = serde_json::to_vec(&record)?;
            anyhow::ensure!(bytes.len() < 2048, "process status record exceeds limit");
            bytes.push(b'\n');
            writer.write(bytes)?;
        }
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

    fn write(&mut self, bytes: Vec<u8>) -> anyhow::Result<()> {
        anyhow::ensure!(!self.failed, "process status output unavailable");
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
    fn stalled_writer_times_out_and_never_queues_more_output() {
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
