use std::collections::VecDeque;

use super::{TimedAcousticSession, TimedSessionOutput};
use crate::acoustic::{
    AssemblyBatch, OUTPUT_QUEUE_CAPACITY, OutputCounts, OutputHealth, PcmEncoding, PcmPlane,
    ProducerFailure, SampleClockMapping, SessionError, SessionFault, SessionPermit, SessionStatus,
    TimedRepetitionFeatures,
};

#[derive(Default)]
pub struct BufferedTimedAcousticSession {
    session: TimedAcousticSession,
    pending: VecDeque<TimedSessionOutput>,
    processed: OutputCounts,
    delivered: OutputCounts,
    discarded: OutputCounts,
    overflow_events: u64,
    high_water_batches: usize,
}

impl BufferedTimedAcousticSession {
    pub fn status(&self) -> SessionStatus {
        self.session.status()
    }

    pub fn pending_batches(&self) -> usize {
        self.pending.len()
    }

    pub fn health(&self) -> OutputHealth {
        let mut pending = OutputCounts::default();
        for output in &self.pending {
            pending.add(output.batch.features.len());
        }
        OutputHealth {
            schema_version: 1,
            status: self.status(),
            capacity_batches: OUTPUT_QUEUE_CAPACITY,
            pending,
            processed: self.processed,
            delivered: self.delivered,
            discarded: self.discarded,
            overflow_events: self.overflow_events,
            high_water_batches: self.high_water_batches,
        }
    }

    fn discard_pending(&mut self) {
        for output in self.pending.drain(..) {
            self.discarded.add(output.batch.features.len());
        }
    }

    pub fn start(&mut self, clock: SampleClockMapping) -> Result<SessionPermit, SessionError> {
        let permit = self.session.start(clock)?;
        self.discard_pending();
        Ok(permit)
    }

    pub fn start_encoded(
        &mut self,
        clock: SampleClockMapping,
        encoding: PcmEncoding,
    ) -> Result<SessionPermit, SessionError> {
        let permit = self.session.start_encoded(clock, encoding)?;
        self.discard_pending();
        Ok(permit)
    }

    pub fn stop(&mut self) {
        self.session.stop();
        self.discard_pending();
    }

    pub fn report_failure(
        &mut self,
        permit: &SessionPermit,
        reason: ProducerFailure,
    ) -> Result<(), SessionError> {
        self.session.report_failure(permit, reason)?;
        self.discard_pending();
        Ok(())
    }

    fn process(
        &mut self,
        operation: impl FnOnce(&mut TimedAcousticSession) -> Result<TimedSessionOutput, SessionError>,
    ) -> Result<(), SessionError> {
        let output = match operation(&mut self.session) {
            Ok(output) => output,
            Err(error) => {
                if self.status() != SessionStatus::Active {
                    self.discard_pending();
                }
                return Err(error);
            }
        };
        self.processed.add(output.batch.features.len());
        if self.pending.len() == OUTPUT_QUEUE_CAPACITY {
            self.session.fault(SessionFault::OutputOverflow);
            self.overflow_events = self.overflow_events.saturating_add(1);
            self.discarded.add(output.batch.features.len());
            self.discard_pending();
            return Err(SessionError::OutputOverflow);
        }
        self.pending.push_back(output);
        self.high_water_batches = self.high_water_batches.max(self.pending.len());
        Ok(())
    }

    pub fn push(
        &mut self,
        permit: &SessionPermit,
        position: u64,
        interleaved: &[f32],
    ) -> Result<(), SessionError> {
        self.process(|session| session.push(permit, position, interleaved))
    }

    pub fn push_bytes(
        &mut self,
        permit: &SessionPermit,
        position: u64,
        encoding: PcmEncoding,
        bytes: &[u8],
    ) -> Result<(), SessionError> {
        self.process(|session| session.push_bytes(permit, position, encoding, bytes))
    }

    pub fn push_planes(
        &mut self,
        permit: &SessionPermit,
        position: u64,
        encoding: PcmEncoding,
        samples_per_channel: usize,
        planes: &[PcmPlane<'_>],
    ) -> Result<(), SessionError> {
        self.process(|session| {
            session.push_planes(permit, position, encoding, samples_per_channel, planes)
        })
    }

    pub fn pop(&mut self) -> Result<Option<AssemblyBatch<TimedRepetitionFeatures>>, SessionError> {
        let Some(output) = self.pending.pop_front() else {
            return Ok(None);
        };
        let frames = output.batch.features.len();
        match self.session.accept_output(output) {
            Ok(batch) => {
                self.delivered.add(frames);
                Ok(Some(batch))
            }
            Err(error) => {
                self.discarded.add(frames);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acoustic::{Format, SourceClockMapping};

    #[test]
    fn lifetime_counters_saturate_through_delivery_and_overflow() {
        let maximum = OutputCounts {
            batches: u64::MAX,
            frames: u64::MAX,
        };
        let mut session = BufferedTimedAcousticSession {
            processed: maximum,
            delivered: maximum,
            discarded: maximum,
            overflow_events: u64::MAX,
            ..Default::default()
        };
        let clock = SampleClockMapping::new(
            SourceClockMapping::new(0, 0, 0, 1_000_000_000, 0).unwrap(),
            Format::new(8_000, 1).unwrap(),
            0,
        )
        .unwrap();
        let permit = session.start(clock).unwrap();
        session.push(&permit, 0, &[0.0; 160]).unwrap();
        session.pop().unwrap().unwrap();
        for index in 1..=8 {
            session.push(&permit, index * 160, &[0.0; 160]).unwrap();
        }
        assert_eq!(
            session.push(&permit, 1440, &[0.0; 160]),
            Err(SessionError::OutputOverflow)
        );
        let health = session.health();
        assert_eq!(health.processed, maximum);
        assert_eq!(health.delivered, maximum);
        assert_eq!(health.discarded, maximum);
        assert_eq!(health.overflow_events, u64::MAX);
    }
}
