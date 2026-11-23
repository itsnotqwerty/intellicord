use super::{Envelope, Payload, ProcessHealth, ProgressStream, Source, StreamError, Summary};
use crate::observe::lifecycle::Status;

pub struct Encoder {
    session: String,
    stream: ProgressStream,
    rejected: bool,
}

impl Encoder {
    pub fn new(session: &str, source: Source) -> Result<Self, StreamError> {
        Ok(Self {
            session: session.into(),
            stream: ProgressStream::new(session, source)?,
            rejected: false,
        })
    }

    pub fn lifecycle(&mut self, status: Status) -> Result<Vec<u8>, StreamError> {
        if self.rejected {
            return Err(StreamError::Rejected);
        }
        let status = match serde_json::value::to_raw_value(&status) {
            Ok(status) => status,
            Err(_) => {
                self.rejected = true;
                return Err(StreamError::InvalidRecord);
            }
        };
        self.encode(Payload::Lifecycle { status })
    }

    pub fn health(&mut self, health: ProcessHealth) -> Result<Vec<u8>, StreamError> {
        self.encode(Payload::Health { health })
    }

    fn encode(&mut self, payload: Payload) -> Result<Vec<u8>, StreamError> {
        if self.rejected {
            return Err(StreamError::Rejected);
        }
        self.rejected = true;
        let mut bytes = serde_json::to_vec(&Envelope {
            schema_version: 1,
            session: self.session.clone(),
            sequence: self.stream.records() + 1,
            payload,
        })
        .map_err(|_| StreamError::InvalidRecord)?;
        bytes.push(b'\n');
        self.stream.push(&bytes, 0)?;
        self.rejected = false;
        Ok(bytes)
    }

    pub fn finish(self, exit_success: bool) -> Result<Summary, StreamError> {
        if self.rejected {
            return Err(StreamError::Rejected);
        }
        self.stream.finish(exit_success)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::lifecycle::{Phase, generated_lifecycle};

    const SESSION: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn generated_producer_round_trips_with_independent_session_and_fragmentation() {
        let mut encoder = Encoder::new(SESSION, Source::GeneratedReplay).unwrap();
        let mut receiver = ProgressStream::new(SESSION, Source::GeneratedReplay).unwrap();
        let mut records = 0;
        for mut status in generated_lifecycle() {
            if matches!(status.phase, Phase::Stopping | Phase::Stopped) {
                status.health = Some(ProcessHealth {
                    schema_version: 1,
                    accepted: 59,
                    ..status.health.unwrap_or_default()
                });
            }
            let mut output = encoder.lifecycle(status).unwrap();
            records += 1;
            if status.phase == Phase::Observing {
                for accepted in 0..60 {
                    output.extend(
                        encoder
                            .health(ProcessHealth {
                                schema_version: 1,
                                accepted,
                                ..ProcessHealth::default()
                            })
                            .unwrap(),
                    );
                    records += 1;
                }
            }
            for chunk in output.chunks(7) {
                receiver.push(chunk, records).unwrap();
            }
        }
        let produced = encoder.finish(true).unwrap();
        let received = receiver.finish(true).unwrap();
        assert_eq!(produced.records, 66);
        assert_eq!(produced.health_samples, 60);
        assert_eq!(
            serde_json::to_value(produced).unwrap(),
            serde_json::to_value(received).unwrap()
        );
    }

    #[test]
    fn invalid_input_poisoning_and_limits_match_receiver() {
        assert!(Encoder::new("bad", Source::GeneratedReplay).is_err());
        let mut encoder = Encoder::new(SESSION, Source::GeneratedReplay).unwrap();
        assert!(encoder.health(ProcessHealth::default()).is_err());
        assert_eq!(
            encoder.lifecycle(generated_lifecycle()[0]),
            Err(StreamError::Rejected)
        );
        assert!(matches!(encoder.finish(false), Err(StreamError::Rejected)));

        let mut encoder = Encoder::new(SESSION, Source::KernelTracepoint).unwrap();
        assert!(encoder.lifecycle(generated_lifecycle()[0]).is_err());

        let mut encoder = Encoder::new(SESSION, Source::GeneratedReplay).unwrap();
        for status in generated_lifecycle().into_iter().take(4) {
            encoder.lifecycle(status).unwrap();
        }
        for _ in 4..super::super::COUNT_LIMIT {
            encoder
                .health(ProcessHealth {
                    schema_version: 1,
                    ..ProcessHealth::default()
                })
                .unwrap();
        }
        assert_eq!(
            encoder.health(ProcessHealth {
                schema_version: 1,
                ..ProcessHealth::default()
            }),
            Err(StreamError::Limit)
        );
        assert_eq!(
            encoder.health(ProcessHealth::default()),
            Err(StreamError::Rejected)
        );
    }

    #[test]
    fn regression_terminal_and_exit_agreement_are_fail_closed() {
        let valid = |accepted| ProcessHealth {
            schema_version: 1,
            accepted,
            ..ProcessHealth::default()
        };
        let mut encoder = Encoder::new(SESSION, Source::GeneratedReplay).unwrap();
        for status in generated_lifecycle().into_iter().take(4) {
            encoder.lifecycle(status).unwrap();
        }
        encoder.health(valid(2)).unwrap();
        assert_eq!(encoder.health(valid(1)), Err(StreamError::HealthRegression));
        assert!(matches!(encoder.finish(true), Err(StreamError::Rejected)));

        let mut encoder = Encoder::new(SESSION, Source::GeneratedReplay).unwrap();
        for status in generated_lifecycle() {
            encoder.lifecycle(status).unwrap();
        }
        assert_eq!(
            encoder.health(valid(2)),
            Err(StreamError::InvalidTransition)
        );

        let mut encoder = Encoder::new(SESSION, Source::GeneratedReplay).unwrap();
        for status in generated_lifecycle() {
            encoder.lifecycle(status).unwrap();
        }
        assert!(matches!(
            encoder.finish(false),
            Err(StreamError::ExitMismatch)
        ));
        assert!(
            Encoder::new(SESSION, Source::GeneratedReplay)
                .unwrap()
                .finish(true)
                .is_err()
        );
    }
}
