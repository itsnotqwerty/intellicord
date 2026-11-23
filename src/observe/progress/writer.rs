use std::time::Duration;

use anyhow::{Context, Result, ensure};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::{ProcessHealth, Source, Summary, producer::Encoder};
use crate::observe::lifecycle::Status;

const OUTPUT_DEADLINE: Duration = Duration::from_millis(100);

pub struct Writer<Output> {
    encoder: Encoder,
    output: Output,
    failed: bool,
}

impl<Output: AsyncWrite + Unpin> Writer<Output> {
    pub fn new(session: &str, source: Source, output: Output) -> Result<Self> {
        Ok(Self {
            encoder: Encoder::new(session, source)?,
            output,
            failed: false,
        })
    }

    pub async fn lifecycle(&mut self, status: Status) -> Result<()> {
        ensure!(!self.failed, "progress output unavailable");
        self.failed = true;
        let bytes = self.encoder.lifecycle(status)?;
        self.deliver(bytes).await
    }

    pub async fn health(&mut self, health: ProcessHealth) -> Result<()> {
        ensure!(!self.failed, "progress output unavailable");
        self.failed = true;
        let bytes = self.encoder.health(health)?;
        self.deliver(bytes).await
    }

    async fn deliver(&mut self, bytes: Vec<u8>) -> Result<()> {
        tokio::time::timeout(OUTPUT_DEADLINE, async {
            self.output.write_all(&bytes).await?;
            self.output.flush().await
        })
        .await
        .context("progress output deadline exceeded")?
        .context("progress output failed")?;
        self.failed = false;
        Ok(())
    }

    pub fn finish(self, exit_success: bool) -> Result<Summary> {
        ensure!(!self.failed, "progress output unavailable");
        Ok(self.encoder.finish(exit_success)?)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        pin::Pin,
        task::{Context as TaskContext, Poll},
    };

    use super::*;
    use crate::observe::{
        lifecycle::{Phase, generated_lifecycle},
        progress::ProgressStream,
    };

    const SESSION: &str = "00000000000000000000000000000002";

    #[tokio::test]
    async fn flushed_output_round_trips_without_changing_protocol() {
        let mut bytes = Vec::new();
        let mut writer = Writer::new(SESSION, Source::GeneratedReplay, &mut bytes).unwrap();
        for status in generated_lifecycle() {
            writer.lifecycle(status).await.unwrap();
            if status.phase == Phase::Observing {
                for accepted in [1, 2] {
                    writer
                        .health(ProcessHealth {
                            schema_version: 1,
                            accepted,
                            ..ProcessHealth::default()
                        })
                        .await
                        .unwrap();
                }
            }
        }
        let summary = writer.finish(true).unwrap();
        let mut receiver = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        for chunk in bytes.chunks(7) {
            receiver.push(chunk, 100).unwrap();
        }
        assert_eq!(
            serde_json::to_value(summary).unwrap(),
            serde_json::to_value(receiver.finish(true).unwrap()).unwrap()
        );
    }

    enum Failure {
        WritePending,
        FlushPending,
        PartialError,
        FlushError,
    }

    struct FaultyOutput {
        failure: Failure,
        calls: usize,
    }

    impl AsyncWrite for FaultyOutput {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut TaskContext<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.calls += 1;
            match self.failure {
                Failure::WritePending => Poll::Pending,
                Failure::PartialError if self.calls == 1 => Poll::Ready(Ok(1)),
                Failure::PartialError => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
                _ => Poll::Ready(Ok(bytes.len())),
            }
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
            match self.failure {
                Failure::FlushPending => Poll::Pending,
                Failure::FlushError => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
                _ => Poll::Ready(Ok(())),
            }
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn deadlines_partial_output_and_flush_errors_permanently_reject() {
        for failure in [
            Failure::WritePending,
            Failure::FlushPending,
            Failure::PartialError,
            Failure::FlushError,
        ] {
            let mut writer = Writer::new(
                SESSION,
                Source::GeneratedReplay,
                FaultyOutput { failure, calls: 0 },
            )
            .unwrap();
            let started = tokio::time::Instant::now();
            let error = writer
                .lifecycle(generated_lifecycle()[0])
                .await
                .unwrap_err();
            assert!(error.to_string().contains(match writer.output.failure {
                Failure::WritePending | Failure::FlushPending => "deadline",
                Failure::PartialError | Failure::FlushError => "output failed",
            }));
            assert!(started.elapsed() < Duration::from_secs(2));
            let calls = writer.output.calls;
            assert!(writer.lifecycle(generated_lifecycle()[0]).await.is_err());
            assert_eq!(writer.output.calls, calls);
            assert!(writer.finish(false).is_err());
        }
    }

    #[tokio::test]
    async fn backpressured_pipe_times_out_without_a_consumer() {
        let (output, _unread) = tokio::io::duplex(16);
        let mut writer = Writer::new(SESSION, Source::GeneratedReplay, output).unwrap();
        let error = writer
            .lifecycle(generated_lifecycle()[0])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("deadline"));
        assert!(writer.lifecycle(generated_lifecycle()[0]).await.is_err());
        assert!(writer.finish(false).is_err());
    }

    #[tokio::test]
    async fn cancelled_operation_and_invalid_record_cannot_resume_output() {
        let mut writer = Writer::new(
            SESSION,
            Source::GeneratedReplay,
            FaultyOutput {
                failure: Failure::WritePending,
                calls: 0,
            },
        )
        .unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                writer.lifecycle(generated_lifecycle()[0])
            )
            .await
            .is_err()
        );
        let calls = writer.output.calls;
        assert!(writer.lifecycle(generated_lifecycle()[0]).await.is_err());
        assert_eq!(writer.output.calls, calls);
        let mut bytes = Vec::new();
        let mut writer = Writer::new(SESSION, Source::GeneratedReplay, &mut bytes).unwrap();
        assert!(
            writer
                .health(ProcessHealth {
                    schema_version: 1,
                    ..ProcessHealth::default()
                })
                .await
                .is_err()
        );
        assert!(writer.lifecycle(generated_lifecycle()[0]).await.is_err());
        drop(writer);
        assert!(bytes.is_empty());
    }
}
