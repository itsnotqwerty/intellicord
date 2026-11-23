use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
};

use serde::Serialize;

use crate::{Action, HISTORY_LIMIT, History, detection::ObservationSource};

pub const BUS_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Producer {
    Application,
    LocalDiagnostics,
    Acoustic,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcousticSource {
    GeneratedPcm,
    MixedOutput,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcousticState {
    Starting,
    Receiving,
    Stopped,
    Faulted,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AcousticAnalysis {
    pub centroid_hz: Option<f64>,
    pub high_band_power_ratio: Option<f64>,
    pub positive_power_flux: Option<f64>,
    pub compared_lags: u64,
    pub best_lag_ns: Option<u64>,
    pub best_similarity: Option<f64>,
}

impl AcousticAnalysis {
    fn from_features(features: &crate::acoustic::RepetitionFeatures) -> Option<Self> {
        let analysis = Self {
            centroid_hz: features.spectral.centroid_hz,
            high_band_power_ratio: features.spectral.high_band_power_ratio,
            positive_power_flux: features.spectral.positive_power_flux,
            compared_lags: u64::try_from(features.compared_lags).ok()?,
            best_lag_ns: features.best_lag_ns,
            best_similarity: features.best_similarity,
        };
        (features.schema_version == 1
            && features.feature_version == "acoustic-repetition/0.1"
            && features.spectral.schema_version == 1
            && features.spectral.feature_version == "acoustic-spectrum/0.1"
            && analysis.valid())
        .then_some(analysis)
    }

    fn valid(&self) -> bool {
        self.centroid_hz
            .is_none_or(|value| value.is_finite() && (0.0..=48_000.0).contains(&value))
            && [
                self.high_band_power_ratio,
                self.positive_power_flux,
                self.best_similarity,
            ]
            .into_iter()
            .all(|value| {
                value.is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            })
            && (self.best_lag_ns.is_some() == self.best_similarity.is_some())
            && (self.compared_lags > 0 || self.best_lag_ns.is_none())
            && self.compared_lags <= 46
            && self.best_lag_ns.is_none_or(|lag| {
                (100_000_000..=1_000_000_000).contains(&lag)
                    && lag.is_multiple_of(crate::acoustic::FRAME_DURATION_NS)
            })
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AcousticSessionInterval {
    pub earliest_ns: u64,
    pub latest_ns: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct AcousticHealth {
    pub source: AcousticSource,
    pub health: crate::acoustic::OutputHealth,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AcousticFrame {
    pub source: AcousticSource,
    pub frame_start_ns: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_start_interval_ns: Option<AcousticSessionInterval>,
    pub rms: f64,
    pub peak: f64,
    pub clipped_sample_ratio: f64,
    pub discontinuity: bool,
    pub analysis: Option<AcousticAnalysis>,
}

impl AcousticFrame {
    pub fn from_features(
        features: &crate::acoustic::Features,
        source: AcousticSource,
    ) -> Option<Self> {
        let frame = Self {
            source,
            frame_start_ns: features.frame_start_ns,
            session_start_interval_ns: None,
            rms: features.rms,
            peak: features.peak,
            clipped_sample_ratio: features.clipped_sample_ratio,
            discontinuity: features.discontinuity,
            analysis: None,
        };
        (features.schema_version == 1
            && features.feature_version == "acoustic-levels/0.1"
            && features.duration_ns == crate::acoustic::FRAME_DURATION_NS
            && frame.valid())
        .then_some(frame)
    }

    pub fn from_repetition_features(
        features: &crate::acoustic::RepetitionFeatures,
        source: AcousticSource,
    ) -> Option<Self> {
        let mut frame = Self::from_features(&features.spectral.levels, source)?;
        frame.analysis = Some(AcousticAnalysis::from_features(features)?);
        Some(frame)
    }

    fn valid(&self) -> bool {
        self.frame_start_ns
            .checked_add(crate::acoustic::FRAME_DURATION_NS)
            .is_some()
            && [self.rms, self.peak, self.clipped_sample_ratio]
                .into_iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            && self
                .session_start_interval_ns
                .as_ref()
                .is_none_or(|interval| {
                    interval.earliest_ns <= interval.latest_ns
                        && interval
                            .latest_ns
                            .checked_add(crate::acoustic::FRAME_DURATION_NS)
                            .is_some()
                })
            && self.analysis.as_ref().is_none_or(AcousticAnalysis::valid)
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Available,
    Degraded,
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct VoiceContext {
    pub call_id: u64,
    pub channel_id: String,
}

#[derive(Clone, Debug)]
pub struct VoiceTransition {
    pub sequence: u64,
    pub user_id: String,
    pub channel_id: String,
    pub action: Action,
}

#[derive(Clone, Debug)]
pub struct VoiceFrame {
    pub context: Option<VoiceContext>,
    pub latest_sequence: u64,
    pub transitions: Vec<VoiceTransition>,
    pub source: ObservationSource,
    pub capture_gap: bool,
}

impl VoiceFrame {
    pub fn from_history(history: &History, source: ObservationSource, capture_gap: bool) -> Self {
        Self {
            context: history.channel.as_ref().and_then(|channel| {
                history.current_call.as_ref().map(|call| VoiceContext {
                    call_id: call.id,
                    channel_id: channel.id.clone(),
                })
            }),
            latest_sequence: history.sequence,
            transitions: history
                .events
                .iter()
                .map(|event| VoiceTransition {
                    sequence: event.sequence,
                    user_id: event.user.id.clone(),
                    channel_id: event.channel.id.clone(),
                    action: event.action.clone(),
                })
                .collect(),
            source,
            capture_gap,
        }
    }
}

#[derive(Clone, Debug)]
pub enum SensorPayload {
    Voice(VoiceFrame),
    Health(HealthState),
    Acoustic(AcousticFrame),
    AcousticState(AcousticSource, AcousticState),
    AcousticHealth(AcousticHealth),
}

#[derive(Clone, Debug)]
pub struct SensorMessage {
    pub schema_version: u16,
    pub session_id: String,
    pub producer: Producer,
    pub producer_sequence: u64,
    pub monotonic_ns: u64,
    pub wall_time_unix_ms: u64,
    pub payload: SensorPayload,
}

impl SensorMessage {
    pub fn valid(&self) -> bool {
        let valid_id = |value: &str| {
            !value.is_empty()
                && value.len() <= 20
                && value.bytes().all(|byte| byte.is_ascii_digit())
        };
        if self.schema_version != 1
            || self.session_id.is_empty()
            || self.session_id.len() > 128
            || self.producer_sequence == 0
        {
            return false;
        }
        match (&self.producer, &self.payload) {
            (Producer::LocalDiagnostics, SensorPayload::Health(_)) => true,
            (Producer::Acoustic, SensorPayload::Acoustic(frame)) => frame.valid(),
            (Producer::Acoustic, SensorPayload::AcousticState(_, _)) => true,
            (Producer::Acoustic, SensorPayload::AcousticHealth(report)) => {
                let health = report.health;
                let conserved_batches = health
                    .delivered
                    .batches
                    .checked_add(health.discarded.batches)
                    .and_then(|value| value.checked_add(health.pending.batches));
                let conserved_frames = health
                    .delivered
                    .frames
                    .checked_add(health.discarded.frames)
                    .and_then(|value| value.checked_add(health.pending.frames));
                health.schema_version == 1
                    && health.capacity_batches == crate::acoustic::OUTPUT_QUEUE_CAPACITY
                    && health.pending.batches <= health.capacity_batches as u64
                    && health.high_water_batches <= health.capacity_batches
                    && conserved_batches == Some(health.processed.batches)
                    && conserved_frames == Some(health.processed.frames)
            }
            (Producer::Application, SensorPayload::Voice(frame)) => {
                frame.transitions.len() <= HISTORY_LIMIT
                    && frame
                        .context
                        .as_ref()
                        .is_none_or(|context| context.call_id > 0 && valid_id(&context.channel_id))
                    && frame.transitions.iter().all(|event| {
                        event.sequence > 0
                            && event.sequence <= frame.latest_sequence
                            && valid_id(&event.user_id)
                            && valid_id(&event.channel_id)
                    })
                    && frame
                        .transitions
                        .windows(2)
                        .all(|pair| pair[0].sequence < pair[1].sequence)
                    && frame.context.as_ref().is_none_or(|context| {
                        frame
                            .transitions
                            .iter()
                            .all(|event| event.channel_id == context.channel_id)
                    })
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
pub struct BusMetrics {
    pub dropped_full: u64,
    pub rejected_invalid: u64,
    pub rejected_closed: u64,
}

#[derive(Default)]
struct Counters {
    full: AtomicU64,
    invalid: AtomicU64,
    closed: AtomicU64,
}

#[derive(Clone)]
pub struct SensorPublisher {
    sender: SyncSender<SensorMessage>,
    counters: Arc<Counters>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PublishError {
    Invalid,
    Full,
    Closed,
}

impl SensorPublisher {
    pub fn publish(&self, message: SensorMessage) -> Result<(), PublishError> {
        if !message.valid() {
            self.counters.invalid.fetch_add(1, Ordering::Relaxed);
            return Err(PublishError::Invalid);
        }
        match self.sender.try_send(message) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.counters.full.fetch_add(1, Ordering::Relaxed);
                Err(PublishError::Full)
            }
            Err(TrySendError::Disconnected(_)) => {
                self.counters.closed.fetch_add(1, Ordering::Relaxed);
                Err(PublishError::Closed)
            }
        }
    }

    pub fn metrics(&self) -> BusMetrics {
        BusMetrics {
            dropped_full: self.counters.full.load(Ordering::Relaxed),
            rejected_invalid: self.counters.invalid.load(Ordering::Relaxed),
            rejected_closed: self.counters.closed.load(Ordering::Relaxed),
        }
    }
}

pub struct SensorBus {
    receiver: Receiver<SensorMessage>,
    publisher: SensorPublisher,
}

impl Default for SensorBus {
    fn default() -> Self {
        let (sender, receiver) = sync_channel(BUS_CAPACITY);
        Self {
            receiver,
            publisher: SensorPublisher {
                sender,
                counters: Arc::default(),
            },
        }
    }
}

impl SensorBus {
    pub fn publisher(&self) -> SensorPublisher {
        self.publisher.clone()
    }

    pub fn drain(&self) -> (Vec<SensorMessage>, BusMetrics) {
        let messages = self.receiver.try_iter().take(BUS_CAPACITY).collect();
        (messages, self.publisher.metrics())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health(sequence: u64) -> SensorMessage {
        SensorMessage {
            schema_version: 1,
            session_id: "session".into(),
            producer: Producer::LocalDiagnostics,
            producer_sequence: sequence,
            monotonic_ns: sequence,
            wall_time_unix_ms: sequence,
            payload: SensorPayload::Health(HealthState::Available),
        }
    }

    fn voice(transition_count: usize) -> SensorMessage {
        SensorMessage {
            schema_version: 1,
            session_id: "session".into(),
            producer: Producer::Application,
            producer_sequence: 1,
            monotonic_ns: 0,
            wall_time_unix_ms: 0,
            payload: SensorPayload::Voice(VoiceFrame {
                context: Some(VoiceContext {
                    call_id: 1,
                    channel_id: "100".into(),
                }),
                latest_sequence: transition_count as u64,
                transitions: (1..=transition_count as u64)
                    .map(|sequence| VoiceTransition {
                        sequence,
                        user_id: "9007199254740993".into(),
                        channel_id: "100".into(),
                        action: if sequence % 2 == 0 {
                            Action::Left
                        } else {
                            Action::Joined
                        },
                    })
                    .collect(),
                source: ObservationSource::Reconciliation,
                capture_gap: false,
            }),
        }
    }

    fn frame_mut(message: &mut SensorMessage) -> &mut VoiceFrame {
        match &mut message.payload {
            SensorPayload::Voice(frame) => frame,
            _ => panic!("expected a voice message"),
        }
    }

    #[test]
    fn voice_payload_accepts_exactly_five_hundred_transitions_and_twenty_digit_ids() {
        assert_eq!(HISTORY_LIMIT, 500);
        let bus = SensorBus::default();
        let publisher = bus.publisher();
        let mut message = voice(500);
        let exact_id = "00000000000000000001";
        let frame = frame_mut(&mut message);
        frame.context.as_mut().unwrap().channel_id = exact_id.into();
        frame.capture_gap = true;
        for transition in &mut frame.transitions {
            transition.user_id = exact_id.into();
            transition.channel_id = exact_id.into();
        }
        publisher.publish(message).unwrap();
        let (messages, metrics) = bus.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(metrics, BusMetrics::default());
        let SensorPayload::Voice(frame) = &messages[0].payload else {
            panic!("expected voice")
        };
        assert_eq!(frame.latest_sequence, 500);
        assert_eq!(frame.transitions.len(), 500);
        assert_eq!(frame.context.as_ref().unwrap().channel_id, exact_id);
        assert_eq!(frame.context.as_ref().unwrap().call_id, 1);
        assert_eq!(frame.source, ObservationSource::Reconciliation);
        assert!(frame.capture_gap);
        for (index, transition) in frame.transitions.iter().enumerate() {
            assert_eq!(transition.sequence, index as u64 + 1);
            assert_eq!(transition.user_id, exact_id);
            assert_eq!(transition.channel_id, exact_id);
            assert_eq!(
                transition.action,
                if index % 2 == 0 {
                    Action::Joined
                } else {
                    Action::Left
                }
            );
        }
        assert_eq!(publisher.publish(voice(501)), Err(PublishError::Invalid));
        assert!(bus.drain().0.is_empty());
        assert_eq!(publisher.metrics().rejected_invalid, 1);
        let mut disconnected = voice(0);
        frame_mut(&mut disconnected).context = None;
        publisher.publish(disconnected).unwrap();
        assert_eq!(bus.drain().0.len(), 1);
    }

    #[test]
    fn malformed_voice_frames_are_rejected_before_entering_the_queue() {
        let cases = [
            "too_many",
            "long_user",
            "long_channel",
            "long_event_channel",
            "empty_user",
            "empty_channel",
            "nondigit_user",
            "nondigit_channel",
            "zero_call",
            "zero_sequence",
            "future_sequence",
            "unsorted",
            "duplicate",
            "mismatched_channel",
            "wrong_producer",
        ];
        let bus = SensorBus::default();
        let publisher = bus.publisher();
        for (index, case) in cases.into_iter().enumerate() {
            let mut message = voice(3);
            if case == "wrong_producer" {
                message.producer = Producer::LocalDiagnostics;
            } else {
                let frame = frame_mut(&mut message);
                match case {
                    "too_many" => message = voice(501),
                    "long_user" => frame.transitions[0].user_id = "1".repeat(21),
                    "long_channel" => frame.context.as_mut().unwrap().channel_id = "1".repeat(21),
                    "long_event_channel" => {
                        frame.context = None;
                        frame.transitions[0].channel_id = "1".repeat(21);
                    }
                    "empty_user" => frame.transitions[0].user_id.clear(),
                    "empty_channel" => frame.context.as_mut().unwrap().channel_id.clear(),
                    "nondigit_user" => frame.transitions[0].user_id = "1e3".into(),
                    "nondigit_channel" => {
                        frame.context.as_mut().unwrap().channel_id = "+100".into()
                    }
                    "zero_call" => frame.context.as_mut().unwrap().call_id = 0,
                    "zero_sequence" => frame.transitions[0].sequence = 0,
                    "future_sequence" => frame.latest_sequence = 2,
                    "unsorted" => frame.transitions.swap(0, 1),
                    "duplicate" => frame.transitions[1].sequence = 1,
                    "mismatched_channel" => frame.transitions[0].channel_id = "200".into(),
                    _ => unreachable!(),
                }
            }
            assert_eq!(
                publisher.publish(message),
                Err(PublishError::Invalid),
                "{case}"
            );
            let (messages, metrics) = bus.drain();
            assert!(messages.is_empty(), "{case}");
            assert_eq!(metrics.rejected_invalid, index as u64 + 1, "{case}");
            assert_eq!(metrics.dropped_full, 0, "{case}");
            assert_eq!(metrics.rejected_closed, 0, "{case}");
        }
    }

    #[test]
    fn envelope_validation_applies_to_both_producers_with_session_boundaries() {
        for template in [health(1), voice(0)] {
            let bus = SensorBus::default();
            let publisher = bus.publisher();
            let mut boundary = template.clone();
            boundary.session_id = "s".repeat(128);
            publisher.publish(boundary).unwrap();
            assert_eq!(bus.drain().0[0].session_id.len(), 128);
            for case in [
                "empty_session",
                "long_session",
                "huge_session",
                "version_zero",
                "version_two",
                "zero_sequence",
            ] {
                let mut invalid = template.clone();
                match case {
                    "empty_session" => invalid.session_id.clear(),
                    "long_session" => invalid.session_id = "s".repeat(129),
                    "huge_session" => invalid.session_id = "s".repeat(1_048_576),
                    "version_zero" => invalid.schema_version = 0,
                    "version_two" => invalid.schema_version = 2,
                    "zero_sequence" => invalid.producer_sequence = 0,
                    _ => unreachable!(),
                }
                assert_eq!(
                    publisher.publish(invalid),
                    Err(PublishError::Invalid),
                    "{case}"
                );
            }
            assert!(bus.drain().0.is_empty());
            assert_eq!(publisher.metrics().rejected_invalid, 6);
        }
        let bus = SensorBus::default();
        for state in [
            HealthState::Available,
            HealthState::Degraded,
            HealthState::Unavailable,
        ] {
            let mut message = health(1);
            message.payload = SensorPayload::Health(state);
            message.producer = Producer::Application;
            assert_eq!(bus.publisher().publish(message), Err(PublishError::Invalid));
            let mut valid = health(1);
            valid.payload = SensorPayload::Health(state);
            bus.publisher().publish(valid).unwrap();
            assert!(
                matches!(bus.drain().0[0].payload, SensorPayload::Health(actual) if actual == state)
            );
        }
    }

    #[test]
    fn full_voice_queue_returns_without_waiting_and_clones_share_metrics() {
        let bus = SensorBus::default();
        let publisher = bus.publisher();
        assert_eq!(BUS_CAPACITY, 64);
        for sequence in 1..=64 {
            let mut message = voice(0);
            message.producer_sequence = sequence;
            publisher.publish(message).unwrap();
        }
        let clone = publisher.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let overflow = clone.publish(voice(0));
            let mut invalid = voice(0);
            invalid.session_id.clear();
            let invalid = clone.publish(invalid);
            sender.send((overflow, invalid, clone.metrics())).unwrap();
        });
        let (overflow, invalid, metrics) = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("publish must not wait for a consumer when the queue is full");
        worker.join().unwrap();
        assert_eq!(overflow, Err(PublishError::Full));
        assert_eq!(invalid, Err(PublishError::Invalid));
        assert_eq!(
            metrics,
            BusMetrics {
                dropped_full: 1,
                rejected_invalid: 1,
                rejected_closed: 0
            }
        );
        assert_eq!(publisher.metrics(), metrics);
        let (messages, drained_metrics) = bus.drain();
        assert_eq!(drained_metrics, metrics);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.producer_sequence)
                .collect::<Vec<_>>(),
            (1..=64).collect::<Vec<_>>()
        );
        assert!(
            messages
                .iter()
                .all(|message| matches!(message.payload, SensorPayload::Voice(_)))
        );
        assert!(bus.drain().0.is_empty());
        publisher.publish(voice(0)).unwrap();
        assert_eq!(bus.drain().0.len(), 1);
        let clone = publisher.clone();
        drop(bus);
        assert_eq!(clone.publish(voice(0)), Err(PublishError::Closed));
        let mut invalid = voice(0);
        invalid.schema_version = 0;
        assert_eq!(publisher.publish(invalid), Err(PublishError::Invalid));
        assert_eq!(
            publisher.metrics(),
            BusMetrics {
                dropped_full: 1,
                rejected_invalid: 2,
                rejected_closed: 1
            }
        );
        assert_eq!(clone.metrics(), publisher.metrics());
    }

    #[test]
    fn bounded_nonblocking_overflow_and_fifo() {
        let bus = SensorBus::default();
        let publisher = bus.publisher();
        for sequence in 1..=BUS_CAPACITY as u64 {
            assert_eq!(publisher.publish(health(sequence)), Ok(()));
        }
        assert_eq!(publisher.publish(health(1000)), Err(PublishError::Full));
        let (messages, metrics) = bus.drain();
        assert_eq!(messages.len(), BUS_CAPACITY);
        assert_eq!(messages.first().unwrap().producer_sequence, 1);
        assert_eq!(
            messages.last().unwrap().producer_sequence,
            BUS_CAPACITY as u64
        );
        assert_eq!(metrics.dropped_full, 1);
        assert!(bus.drain().0.is_empty());
        assert_eq!(publisher.publish(health(1001)), Ok(()));
    }

    #[test]
    fn invalid_routing_version_and_disconnect_are_explicit() {
        let bus = SensorBus::default();
        let publisher = bus.publisher();
        let mut message = health(1);
        message.producer = Producer::Application;
        assert_eq!(publisher.publish(message), Err(PublishError::Invalid));
        let mut message = health(2);
        message.schema_version = 99;
        assert_eq!(publisher.publish(message), Err(PublishError::Invalid));
        assert_eq!(publisher.metrics().rejected_invalid, 2);
        drop(bus);
        assert_eq!(publisher.publish(health(3)), Err(PublishError::Closed));
        assert_eq!(publisher.metrics().rejected_closed, 1);
    }

    #[test]
    fn independent_producer_threads_share_a_bounded_consumer() {
        let bus = SensorBus::default();
        let publisher = bus.publisher();
        let handles: Vec<_> = (0..4)
            .map(|thread| {
                let publisher = publisher.clone();
                std::thread::spawn(move || {
                    for index in 1..=8 {
                        publisher.publish(health(thread * 8 + index)).unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(bus.drain().0.len(), 32);
    }
}
