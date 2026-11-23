use std::sync::Arc;

mod buffered;
pub use buffered::BufferedTimedAcousticSession;

use super::{ProducerFailure, SessionError, SessionFault, SessionPermit, SessionStatus};
use crate::acoustic::{
    AssemblyBatch, PcmEncoding, PcmPlane, SampleClockMapping, TimedFrameError,
    TimedRepetitionFeatures, TimedRepetitionFrameAssembler,
};

pub struct TimedSessionOutput {
    permit: SessionPermit,
    batch: AssemblyBatch<TimedRepetitionFeatures>,
}

struct Active {
    permit: SessionPermit,
    encoding: Option<PcmEncoding>,
    assembler: TimedRepetitionFrameAssembler,
}

pub struct TimedAcousticSession {
    active: Option<Active>,
    status: SessionStatus,
}

impl Default for TimedAcousticSession {
    fn default() -> Self {
        Self {
            active: None,
            status: SessionStatus::Stopped,
        }
    }
}

impl TimedAcousticSession {
    pub fn status(&self) -> SessionStatus {
        self.status
    }

    pub fn start(&mut self, clock: SampleClockMapping) -> Result<SessionPermit, SessionError> {
        self.start_with_encoding(clock, None)
    }

    pub fn start_encoded(
        &mut self,
        clock: SampleClockMapping,
        encoding: PcmEncoding,
    ) -> Result<SessionPermit, SessionError> {
        self.start_with_encoding(clock, Some(encoding))
    }

    fn start_with_encoding(
        &mut self,
        clock: SampleClockMapping,
        encoding: Option<PcmEncoding>,
    ) -> Result<SessionPermit, SessionError> {
        if self.active.is_some() {
            return Err(SessionError::AlreadyActive);
        }
        let permit = SessionPermit(Arc::new(()));
        self.active = Some(Active {
            permit: permit.clone(),
            encoding,
            assembler: TimedRepetitionFrameAssembler::new(clock),
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
        self.validate_permit(permit)?;
        self.fault(SessionFault::Producer(reason));
        Ok(())
    }

    fn validate_permit(&self, permit: &SessionPermit) -> Result<(), SessionError> {
        let active = self.active.as_ref().ok_or(SessionError::Inactive)?;
        if !Arc::ptr_eq(&active.permit.0, &permit.0) {
            return Err(SessionError::StalePermit);
        }
        Ok(())
    }

    fn fault(&mut self, error: SessionFault) {
        self.active = None;
        self.status = SessionStatus::Faulted { error };
    }

    fn process(
        &mut self,
        permit: &SessionPermit,
        encoding: Option<PcmEncoding>,
        operation: impl FnOnce(
            &mut TimedRepetitionFrameAssembler,
        )
            -> Result<AssemblyBatch<TimedRepetitionFeatures>, TimedFrameError>,
    ) -> Result<TimedSessionOutput, SessionError> {
        self.validate_permit(permit)?;
        let active = self.active.as_mut().expect("validated active permit");
        if encoding.is_some() && active.encoding.is_some() && encoding != active.encoding {
            self.fault(SessionFault::Producer(ProducerFailure::FormatChanged));
            return Err(SessionError::FormatChanged);
        }
        match operation(&mut active.assembler) {
            Ok(batch) => Ok(TimedSessionOutput {
                permit: permit.clone(),
                batch,
            }),
            Err(TimedFrameError::Frame(error)) => {
                self.fault(error.into());
                Err(SessionError::InvalidFrame(error))
            }
            Err(TimedFrameError::Clock(error)) => {
                self.fault(SessionFault::Clock(error));
                Err(SessionError::InvalidClock(error))
            }
        }
    }

    pub fn push(
        &mut self,
        permit: &SessionPermit,
        position: u64,
        interleaved: &[f32],
    ) -> Result<TimedSessionOutput, SessionError> {
        self.process(permit, None, |assembler| {
            assembler.push(position, interleaved)
        })
    }

    pub fn push_bytes(
        &mut self,
        permit: &SessionPermit,
        position: u64,
        encoding: PcmEncoding,
        bytes: &[u8],
    ) -> Result<TimedSessionOutput, SessionError> {
        self.process(permit, Some(encoding), |assembler| {
            assembler.push_bytes(position, encoding, bytes)
        })
    }

    pub fn push_planes(
        &mut self,
        permit: &SessionPermit,
        position: u64,
        encoding: PcmEncoding,
        samples_per_channel: usize,
        planes: &[PcmPlane<'_>],
    ) -> Result<TimedSessionOutput, SessionError> {
        self.process(permit, Some(encoding), |assembler| {
            assembler.push_planes(position, encoding, samples_per_channel, planes)
        })
    }

    pub fn accept_output(
        &self,
        output: TimedSessionOutput,
    ) -> Result<AssemblyBatch<TimedRepetitionFeatures>, SessionError> {
        self.validate_permit(&output.permit)?;
        Ok(output.batch)
    }
}
