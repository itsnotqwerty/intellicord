use std::time::Duration;

use anyhow::{Result, ensure};
use tokio::{io::AsyncWrite, time::Instant};

use super::{ProcessHealth, Source, Summary, writer::Writer};
use crate::observe::lifecycle::{Lifecycle, Phase, Reason};

const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

pub struct Reporter<Output> {
    lifecycle: Lifecycle,
    writer: Writer<Output>,
    next_sample: Option<Instant>,
    failed: bool,
}

impl<Output: AsyncWrite + Unpin> Reporter<Output> {
    pub fn new(session: &str, source: Source, output: Output) -> Result<Self> {
        Ok(Self {
            lifecycle: Lifecycle::new(source == Source::GeneratedReplay),
            writer: Writer::new(session, source, output)?,
            next_sample: None,
            failed: false,
        })
    }

    pub async fn emit(
        &mut self,
        phase: Phase,
        reason: Option<Reason>,
        health: Option<ProcessHealth>,
    ) -> Result<()> {
        ensure!(!self.failed, "progress reporter unavailable");
        self.failed = true;
        let status = self.lifecycle.advance(phase, reason, health)?;
        self.writer.lifecycle(status).await?;
        self.next_sample = (phase == Phase::Observing).then(|| Instant::now() + SAMPLE_PERIOD);
        self.failed = false;
        Ok(())
    }

    pub async fn periodic_health(&mut self, health: ProcessHealth) -> Result<bool> {
        ensure!(!self.failed, "progress reporter unavailable");
        if self.next_sample.is_none_or(|next| Instant::now() < next) {
            return Ok(false);
        }
        self.failed = true;
        self.writer.health(health).await?;
        self.next_sample = Some(Instant::now() + SAMPLE_PERIOD);
        self.failed = false;
        Ok(true)
    }

    pub fn finish(self, exit_success: bool) -> Result<Summary> {
        ensure!(!self.failed, "progress reporter unavailable");
        self.writer.finish(exit_success)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::{lifecycle::generated_lifecycle, progress::ProgressStream};

    const SESSION: &str = "00000000000000000000000000000003";

    #[tokio::test]
    async fn periodic_samples_require_observation_and_never_catch_up_in_a_burst() {
        let mut bytes = Vec::new();
        let mut reporter = Reporter::new(SESSION, Source::GeneratedReplay, &mut bytes).unwrap();
        let health = ProcessHealth {
            schema_version: 1,
            accepted: 1,
            ..ProcessHealth::default()
        };
        assert!(!reporter.periodic_health(health).await.unwrap());
        for status in generated_lifecycle().into_iter().take(4) {
            reporter
                .emit(status.phase, status.reason, status.health)
                .await
                .unwrap();
        }
        assert!(!reporter.periodic_health(health).await.unwrap());
        tokio::time::sleep_until(reporter.next_sample.unwrap()).await;
        assert!(reporter.periodic_health(health).await.unwrap());
        assert!(!reporter.periodic_health(health).await.unwrap());
        reporter.next_sample = Some(Instant::now() - Duration::from_secs(30));
        assert!(reporter.periodic_health(health).await.unwrap());
        assert!(!reporter.periodic_health(health).await.unwrap());
        for status in generated_lifecycle().into_iter().skip(4) {
            reporter
                .emit(status.phase, status.reason, status.health)
                .await
                .unwrap();
            assert!(!reporter.periodic_health(health).await.unwrap());
        }
        let summary = reporter.finish(true).unwrap();
        assert_eq!(summary.records, 8);
        assert_eq!(summary.health_samples, 2);
        let mut receiver = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        receiver.push(&bytes, 100).unwrap();
        assert_eq!(
            serde_json::to_value(summary).unwrap(),
            serde_json::to_value(receiver.finish(true).unwrap()).unwrap()
        );
    }

    #[tokio::test]
    async fn invalid_transition_and_cancelled_output_permanently_reject() {
        let mut reporter = Reporter::new(SESSION, Source::GeneratedReplay, Vec::new()).unwrap();
        assert!(reporter.emit(Phase::Observing, None, None).await.is_err());
        assert!(reporter.emit(Phase::Validating, None, None).await.is_err());
        assert!(
            reporter
                .periodic_health(ProcessHealth::default())
                .await
                .is_err()
        );
        assert!(reporter.finish(false).is_err());
        let (output, _unread) = tokio::io::duplex(1);
        let mut reporter = Reporter::new(SESSION, Source::GeneratedReplay, output).unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                reporter.emit(Phase::Validating, None, None)
            )
            .await
            .is_err()
        );
        assert!(reporter.emit(Phase::Validating, None, None).await.is_err());
        assert!(reporter.finish(false).is_err());
    }
}
