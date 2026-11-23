use serde::Serialize;

use super::ProcessHealth;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Validating,
    Loading,
    Attaching,
    Observing,
    Stopping,
    Stopped,
    Faulted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    DurationLimit,
    Interrupt,
    Terminate,
    TargetExited,
    InvalidObject,
    TargetUnavailable,
    SignalSetupFailed,
    LoadFailed,
    AttachFailed,
    InputRejected,
    ReadFailed,
    DeliveryFailed,
    DetachFailed,
    Cancelled,
    InvalidRequest,
    InternalFailure,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for Reason {}

impl Reason {
    pub fn from_error(error: &anyhow::Error) -> Self {
        error
            .downcast_ref::<Self>()
            .copied()
            .unwrap_or(Self::InternalFailure)
    }

    pub fn from_stop(reason: &str) -> Self {
        match reason {
            "duration_limit" => Self::DurationLimit,
            "interrupt" => Self::Interrupt,
            "terminate" => Self::Terminate,
            "target_exited" => Self::TargetExited,
            _ => Self::InternalFailure,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub schema_version: u16,
    pub record_type: &'static str,
    pub source: &'static str,
    pub sequence: u64,
    pub phase: Phase,
    pub reason: Option<Reason>,
    pub attachment_confirmed: bool,
    pub detachment_confirmed: bool,
    pub health: Option<ProcessHealth>,
}

#[derive(Clone)]
pub struct Lifecycle {
    generated: bool,
    sequence: u64,
    phase: Option<Phase>,
    attachment_confirmed: bool,
    detachment_confirmed: bool,
}

impl Lifecycle {
    pub fn new(generated: bool) -> Self {
        Self {
            generated,
            sequence: 0,
            phase: None,
            attachment_confirmed: false,
            detachment_confirmed: false,
        }
    }

    pub fn advance(
        &mut self,
        phase: Phase,
        reason: Option<Reason>,
        health: Option<ProcessHealth>,
    ) -> anyhow::Result<Status> {
        use Phase::*;
        let terminal = matches!(phase, Stopped | Faulted);
        anyhow::ensure!(
            terminal == reason.is_some(),
            "terminal status requires a reason only at termination"
        );
        if let Some(reason) = reason {
            let normal = matches!(
                reason,
                Reason::DurationLimit
                    | Reason::Interrupt
                    | Reason::Terminate
                    | Reason::TargetExited
            );
            anyhow::ensure!(
                (phase == Stopped) == normal,
                "termination reason does not match phase"
            );
        }
        let allowed = matches!(
            (self.phase, phase),
            (None, Validating)
                | (Some(Validating), Loading)
                | (Some(Loading), Attaching)
                | (Some(Attaching), Observing)
                | (Some(Observing), Stopping)
                | (Some(Stopping), Stopped)
        ) || (phase == Faulted && !matches!(self.phase, Some(Stopped | Faulted)));
        anyhow::ensure!(allowed, "invalid process lifecycle transition");
        if !self.generated {
            if phase == Observing {
                self.attachment_confirmed = true;
            }
            if phase == Stopped {
                self.detachment_confirmed = true;
            }
        }
        self.sequence += 1;
        self.phase = Some(phase);
        Ok(Status {
            schema_version: 1,
            record_type: "process_status",
            source: if self.generated {
                "generated_replay"
            } else {
                "kernel_tracepoint"
            },
            sequence: self.sequence,
            phase,
            reason,
            attachment_confirmed: self.attachment_confirmed,
            detachment_confirmed: self.detachment_confirmed,
            health,
        })
    }
}

pub fn generated_lifecycle() -> Vec<Status> {
    let mut lifecycle = Lifecycle::new(true);
    [
        Phase::Validating,
        Phase::Loading,
        Phase::Attaching,
        Phase::Observing,
        Phase::Stopping,
        Phase::Stopped,
    ]
    .into_iter()
    .map(|phase| {
        lifecycle
            .advance(
                phase,
                (phase == Phase::Stopped).then_some(Reason::DurationLimit),
                (phase == Phase::Stopped).then(super::generated_replay),
            )
            .expect("fixed generated lifecycle")
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_records_are_bounded_ordered_and_never_confirm_kernel_operations() {
        let records = generated_lifecycle();
        assert_eq!(records.len(), 6);
        for (index, record) in records.iter().enumerate() {
            assert_eq!(record.sequence, index as u64 + 1);
            assert_eq!(record.source, "generated_replay");
            assert!(!record.attachment_confirmed);
            assert!(!record.detachment_confirmed);
            assert!(serde_json::to_vec(record).unwrap().len() < 2048);
        }
        assert_eq!(records[5].reason, Some(Reason::DurationLimit));
        assert_eq!(records[5].health.unwrap().accepted, 2);
    }

    #[test]
    fn invalid_transitions_do_not_advance_and_faults_never_confirm_detachment() {
        let mut lifecycle = Lifecycle::new(false);
        assert!(lifecycle.advance(Phase::Observing, None, None).is_err());
        assert_eq!(
            lifecycle
                .advance(Phase::Validating, None, None)
                .unwrap()
                .sequence,
            1
        );
        assert!(
            lifecycle
                .advance(Phase::Stopped, Some(Reason::DurationLimit), None)
                .is_err()
        );
        assert!(
            lifecycle
                .advance(Phase::Faulted, Some(Reason::DurationLimit), None)
                .is_err()
        );
        let fault = lifecycle
            .advance(Phase::Faulted, Some(Reason::InvalidObject), None)
            .unwrap();
        assert_eq!(fault.sequence, 2);
        assert!(!fault.detachment_confirmed);
        assert!(lifecycle.advance(Phase::Loading, None, None).is_err());
        assert_eq!(
            Reason::from_error(&anyhow::anyhow!("LoadFailed")),
            Reason::InternalFailure
        );
        assert_eq!(
            Reason::from_error(&anyhow::anyhow!("detail").context(Reason::LoadFailed)),
            Reason::LoadFailed
        );
    }

    #[test]
    fn observed_failure_preserves_attachment_but_does_not_claim_detachment() {
        let mut lifecycle = Lifecycle::new(false);
        for phase in [
            Phase::Validating,
            Phase::Loading,
            Phase::Attaching,
            Phase::Observing,
            Phase::Stopping,
        ] {
            let status = lifecycle.advance(phase, None, None).unwrap();
            assert_eq!(
                status.attachment_confirmed,
                matches!(phase, Phase::Observing | Phase::Stopping)
            );
            assert!(!status.detachment_confirmed);
        }
        let fault = lifecycle
            .advance(Phase::Faulted, Some(Reason::DetachFailed), None)
            .unwrap();
        assert!(fault.attachment_confirmed);
        assert!(!fault.detachment_confirmed);
        assert!(
            lifecycle
                .advance(Phase::Stopped, Some(Reason::DurationLimit), None)
                .is_err()
        );
    }

    #[test]
    fn normal_termination_confirms_detachment_only_after_stopping() {
        let mut lifecycle = Lifecycle::new(false);
        for phase in [
            Phase::Validating,
            Phase::Loading,
            Phase::Attaching,
            Phase::Observing,
            Phase::Stopping,
        ] {
            lifecycle.advance(phase, None, None).unwrap();
        }
        assert!(
            lifecycle
                .advance(Phase::Stopped, Some(Reason::DetachFailed), None)
                .is_err()
        );
        let stopped = lifecycle
            .advance(Phase::Stopped, Some(Reason::Interrupt), None)
            .unwrap();
        assert!(stopped.attachment_confirmed);
        assert!(stopped.detachment_confirmed);
        assert_eq!(stopped.sequence, 6);
        assert!(
            lifecycle
                .advance(Phase::Faulted, Some(Reason::InternalFailure), None)
                .is_err()
        );
    }
}
