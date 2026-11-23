use std::{collections::VecDeque, sync::Arc};

use serde::Serialize;

use super::{
    AssemblyBatch, ClockError, Format, FrameError, PcmEncoding, PcmPlane, RepetitionFeatures,
    RepetitionFrameAssembler, decode_pcm_chunk, decode_pcm_planes,
};

mod timed;
pub use timed::{BufferedTimedAcousticSession, TimedAcousticSession, TimedSessionOutput};

#[derive(Clone, Debug)]
pub struct SessionPermit(Arc<()>);

pub struct SessionOutput {
    permit: SessionPermit,
    batch: AssemblyBatch<RepetitionFeatures>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionStatus {
    Stopped,
    Active,
    Faulted { error: SessionFault },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionFault {
    UnsupportedFormat,
    InvalidLength,
    InvalidSample,
    InvalidTime,
    Producer(ProducerFailure),
    OutputOverflow,
    Clock(ClockError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProducerFailure {
    SourceLost,
    FormatChanged,
    InputOverflow,
}

impl From<FrameError> for SessionFault {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::UnsupportedFormat => Self::UnsupportedFormat,
            FrameError::InvalidLength => Self::InvalidLength,
            FrameError::InvalidSample => Self::InvalidSample,
            FrameError::InvalidTime => Self::InvalidTime,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    AlreadyActive,
    Inactive,
    StalePermit,
    InvalidFrame(FrameError),
    OutputOverflow,
    FormatChanged,
    InvalidClock(ClockError),
}

pub const OUTPUT_QUEUE_CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct OutputCounts {
    pub batches: u64,
    pub frames: u64,
}

impl OutputCounts {
    fn add(&mut self, frames: usize) {
        self.batches = self.batches.saturating_add(1);
        self.frames = self.frames.saturating_add(frames as u64);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct OutputHealth {
    pub schema_version: u16,
    pub status: SessionStatus,
    pub capacity_batches: usize,
    pub pending: OutputCounts,
    pub processed: OutputCounts,
    pub delivered: OutputCounts,
    pub discarded: OutputCounts,
    pub overflow_events: u64,
    pub high_water_batches: usize,
}

#[derive(Default)]
pub struct BufferedAcousticSession {
    session: AcousticSession,
    pending: VecDeque<SessionOutput>,
    processed: OutputCounts,
    delivered: OutputCounts,
    discarded: OutputCounts,
    overflow_events: u64,
    high_water_batches: usize,
}

impl BufferedAcousticSession {
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

    pub fn start(&mut self, format: Format, origin_ns: u64) -> Result<SessionPermit, SessionError> {
        let permit = self.session.start(format, origin_ns)?;
        self.discard_pending();
        Ok(permit)
    }

    pub fn start_encoded(
        &mut self,
        format: Format,
        origin_ns: u64,
        encoding: PcmEncoding,
    ) -> Result<SessionPermit, SessionError> {
        let permit = self
            .session
            .start_with_encoding(format, origin_ns, Some(encoding))?;
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

    pub fn push(
        &mut self,
        permit: &SessionPermit,
        start_sample_per_channel: u64,
        interleaved: &[f32],
    ) -> Result<(), SessionError> {
        let output = match self
            .session
            .push_tagged(permit, start_sample_per_channel, interleaved)
        {
            Ok(output) => output,
            Err(error) => {
                if self.session.status() != SessionStatus::Active {
                    self.discard_pending();
                }
                return Err(error);
            }
        };
        self.processed.add(output.batch.features.len());
        if self.pending.len() == OUTPUT_QUEUE_CAPACITY {
            self.session.active = None;
            self.session.status = SessionStatus::Faulted {
                error: SessionFault::OutputOverflow,
            };
            self.overflow_events = self.overflow_events.saturating_add(1);
            self.discarded.add(output.batch.features.len());
            self.discard_pending();
            return Err(SessionError::OutputOverflow);
        }
        self.pending.push_back(output);
        self.high_water_batches = self.high_water_batches.max(self.pending.len());
        Ok(())
    }

    pub fn push_bytes(
        &mut self,
        permit: &SessionPermit,
        start_sample_per_channel: u64,
        encoding: PcmEncoding,
        bytes: &[u8],
    ) -> Result<(), SessionError> {
        self.push_converted(permit, start_sample_per_channel, encoding, |format| {
            decode_pcm_chunk(format, encoding, bytes)
        })
    }

    pub fn push_planes(
        &mut self,
        permit: &SessionPermit,
        start_sample_per_channel: u64,
        encoding: PcmEncoding,
        samples_per_channel: usize,
        planes: &[PcmPlane<'_>],
    ) -> Result<(), SessionError> {
        self.push_converted(permit, start_sample_per_channel, encoding, |format| {
            decode_pcm_planes(format, encoding, samples_per_channel, planes)
        })
    }

    fn push_converted(
        &mut self,
        permit: &SessionPermit,
        start_sample_per_channel: u64,
        encoding: PcmEncoding,
        decode: impl FnOnce(Format) -> Result<Vec<f32>, FrameError>,
    ) -> Result<(), SessionError> {
        let active = self.session.active.as_ref().ok_or(SessionError::Inactive)?;
        if !Arc::ptr_eq(&active.permit.0, &permit.0) {
            return Err(SessionError::StalePermit);
        }
        if active.encoding.is_some_and(|expected| expected != encoding) {
            self.report_failure(permit, ProducerFailure::FormatChanged)?;
            return Err(SessionError::FormatChanged);
        }
        let samples = match decode(active.format) {
            Ok(samples) => samples,
            Err(error) => {
                self.session.active = None;
                self.session.status = SessionStatus::Faulted {
                    error: error.into(),
                };
                self.discard_pending();
                return Err(SessionError::InvalidFrame(error));
            }
        };
        self.push(permit, start_sample_per_channel, &samples)
    }

    pub fn pop(&mut self) -> Result<Option<AssemblyBatch<RepetitionFeatures>>, SessionError> {
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

struct Active {
    permit: SessionPermit,
    format: Format,
    encoding: Option<PcmEncoding>,
    assembler: RepetitionFrameAssembler,
}

pub struct AcousticSession {
    active: Option<Active>,
    status: SessionStatus,
}

impl Default for AcousticSession {
    fn default() -> Self {
        Self {
            active: None,
            status: SessionStatus::Stopped,
        }
    }
}

impl AcousticSession {
    pub fn status(&self) -> SessionStatus {
        self.status
    }

    pub fn start(&mut self, format: Format, origin_ns: u64) -> Result<SessionPermit, SessionError> {
        self.start_with_encoding(format, origin_ns, None)
    }

    fn start_with_encoding(
        &mut self,
        format: Format,
        origin_ns: u64,
        encoding: Option<PcmEncoding>,
    ) -> Result<SessionPermit, SessionError> {
        if self.active.is_some() {
            return Err(SessionError::AlreadyActive);
        }
        let permit = SessionPermit(Arc::new(()));
        self.active = Some(Active {
            permit: permit.clone(),
            format,
            encoding,
            assembler: RepetitionFrameAssembler::new(format, origin_ns),
        });
        self.status = SessionStatus::Active;
        Ok(permit)
    }

    pub fn stop(&mut self) {
        self.active = None;
        self.status = SessionStatus::Stopped;
    }

    pub fn report_failure(
        &mut self,
        permit: &SessionPermit,
        reason: ProducerFailure,
    ) -> Result<(), SessionError> {
        let active = self.active.as_ref().ok_or(SessionError::Inactive)?;
        if !Arc::ptr_eq(&active.permit.0, &permit.0) {
            return Err(SessionError::StalePermit);
        }
        self.active = None;
        self.status = SessionStatus::Faulted {
            error: SessionFault::Producer(reason),
        };
        Ok(())
    }

    pub fn push_tagged(
        &mut self,
        permit: &SessionPermit,
        start_sample_per_channel: u64,
        interleaved: &[f32],
    ) -> Result<SessionOutput, SessionError> {
        let batch = self.push(permit, start_sample_per_channel, interleaved)?;
        Ok(SessionOutput {
            permit: permit.clone(),
            batch,
        })
    }

    pub fn accept_output(
        &self,
        output: SessionOutput,
    ) -> Result<AssemblyBatch<RepetitionFeatures>, SessionError> {
        let active = self.active.as_ref().ok_or(SessionError::Inactive)?;
        if !Arc::ptr_eq(&active.permit.0, &output.permit.0) {
            return Err(SessionError::StalePermit);
        }
        Ok(output.batch)
    }

    pub fn push(
        &mut self,
        permit: &SessionPermit,
        start_sample_per_channel: u64,
        interleaved: &[f32],
    ) -> Result<AssemblyBatch<RepetitionFeatures>, SessionError> {
        let active = self.active.as_mut().ok_or(SessionError::Inactive)?;
        if !Arc::ptr_eq(&active.permit.0, &permit.0) {
            return Err(SessionError::StalePermit);
        }
        match active.assembler.push(start_sample_per_channel, interleaved) {
            Ok(batch) => Ok(batch),
            Err(error) => {
                self.active = None;
                self.status = SessionStatus::Faulted {
                    error: error.into(),
                };
                Err(SessionError::InvalidFrame(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_counters_saturate_without_wrapping() {
        let mut session = BufferedAcousticSession {
            processed: OutputCounts {
                batches: u64::MAX,
                frames: u64::MAX,
            },
            delivered: OutputCounts {
                batches: u64::MAX,
                frames: u64::MAX,
            },
            discarded: OutputCounts {
                batches: u64::MAX,
                frames: u64::MAX,
            },
            overflow_events: u64::MAX,
            ..Default::default()
        };
        let permit = session.start(format(), 0).unwrap();
        session.push(&permit, 0, &[0.0; 160]).unwrap();
        session.pop().unwrap().unwrap();
        for index in 1..=OUTPUT_QUEUE_CAPACITY {
            session
                .push(&permit, index as u64 * 160, &[0.0; 160])
                .unwrap();
        }
        assert_eq!(
            session.push(&permit, 1440, &[0.0; 160]),
            Err(SessionError::OutputOverflow)
        );
        let health = session.health();
        let maximum = OutputCounts {
            batches: u64::MAX,
            frames: u64::MAX,
        };
        assert_eq!(health.processed, maximum);
        assert_eq!(health.delivered, maximum);
        assert_eq!(health.discarded, maximum);
        assert_eq!(health.overflow_events, u64::MAX);
    }

    fn format() -> Format {
        Format::new(8_000, 1).unwrap()
    }

    #[test]
    fn defaults_off_and_does_not_replace_active_session() {
        let mut session = AcousticSession::default();
        assert_eq!(session.status(), SessionStatus::Stopped);
        let permit = session.start(format(), 10).unwrap();
        assert_eq!(
            session.start(format(), 20).unwrap_err(),
            SessionError::AlreadyActive
        );
        let batch = session.push(&permit, 0, &[0.0; 160]).unwrap();
        assert_eq!(batch.features[0].spectral.levels.frame_start_ns, 10);
        session.stop();
        session.stop();
        assert_eq!(session.status(), SessionStatus::Stopped);
        assert!(matches!(
            session.push(&permit, 160, &[0.0; 160]),
            Err(SessionError::Inactive)
        ));
    }

    #[test]
    fn stale_clones_and_foreign_permits_cannot_mutate_new_session() {
        let mut session = AcousticSession::default();
        let old = session.start(format(), 0).unwrap().clone();
        session.stop();
        let current = session.start(format(), 100).unwrap();
        let mut other = AcousticSession::default();
        let foreign = other.start(format(), 100).unwrap();
        for permit in [&old, &foreign] {
            assert!(matches!(
                session.push(permit, 1_000, &[f32::NAN; 160]),
                Err(SessionError::StalePermit)
            ));
            assert_eq!(session.status(), SessionStatus::Active);
        }
        assert_eq!(
            session.push(&current, 0, &[0.0; 160]).unwrap().features[0]
                .spectral
                .levels
                .frame_start_ns,
            100
        );
    }

    #[test]
    fn invalid_active_input_faults_until_explicit_restart() {
        let mut session = AcousticSession::default();
        let permit = session.start(format(), 0).unwrap();
        session.push(&permit, 0, &[0.5; 10]).unwrap();
        assert!(matches!(
            session.push(&permit, 10, &[f32::NAN]),
            Err(SessionError::InvalidFrame(FrameError::InvalidSample))
        ));
        assert_eq!(
            session.status(),
            SessionStatus::Faulted {
                error: SessionFault::InvalidSample
            }
        );
        assert!(session.active.is_none());
        assert!(matches!(
            session.push(&permit, 10, &[0.0; 150]),
            Err(SessionError::Inactive)
        ));
        let current = session.start(format(), 2_000).unwrap();
        let batch = session.push(&current, 0, &[0.0; 160]).unwrap();
        assert_eq!(batch.features[0].spectral.levels.frame_start_ns, 2_000);
        assert_eq!(batch.features[0].spectral.levels.rms, 0.0);
        assert!(batch.features[0].best_similarity.is_none());
    }

    #[test]
    fn stop_discards_pending_pcm_and_repetition_history() {
        let mut session = AcousticSession::default();
        let permit = session.start(format(), 0).unwrap();
        let tone: Vec<_> = (0..160)
            .map(|index| (std::f32::consts::TAU * index as f32 / 8.0).sin() * 0.5)
            .collect();
        for index in 0..10 {
            let batch = session.push(&permit, index * 160, &tone).unwrap();
            assert_eq!(batch.features[0].best_similarity.is_some(), index == 9);
        }
        session.push(&permit, 1600, &[0.5; 10]).unwrap();
        session.stop();
        assert!(session.active.is_none());
        let current = session.start(format(), 0).unwrap();
        for index in 0..10 {
            let batch = session.push(&current, index * 160, &tone).unwrap();
            assert_eq!(batch.features[0].best_similarity.is_some(), index == 9);
            if index == 0 {
                assert!(batch.features[0].spectral.levels.rms_change.is_none());
            }
        }
    }

    #[test]
    fn forward_gaps_recover_but_bad_time_faults_and_status_is_summary_only() {
        let mut session = AcousticSession::default();
        let permit = session.start(format(), 0).unwrap();
        session.push(&permit, 0, &[0.0; 10]).unwrap();
        let batch = session.push(&permit, 15, &[0.0; 160]).unwrap();
        assert_eq!(batch.gap.unwrap().skipped_samples_per_channel, 5);
        assert!(batch.features[0].spectral.levels.discontinuity);
        assert_eq!(session.status(), SessionStatus::Active);
        assert!(matches!(
            session.push(&permit, 0, &[0.0; 160]),
            Err(SessionError::InvalidFrame(FrameError::InvalidTime))
        ));
        assert_eq!(
            serde_json::to_value(session.status()).unwrap(),
            serde_json::json!({"state":"faulted","error":"invalid_time"})
        );
        session.stop();
        assert_eq!(
            serde_json::to_value(session.status()).unwrap(),
            serde_json::json!({"state":"stopped"})
        );
    }
}
