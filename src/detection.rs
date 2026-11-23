use std::collections::{BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::ingestion::{
    AcousticFrame, AcousticSource, AcousticState, BUS_CAPACITY, BusMetrics, HealthState, Producer,
    SensorBus, SensorPayload, VoiceFrame,
};
use crate::{Action, History};

pub const OBSERVATION_LIMIT: usize = 512;
pub const INCIDENT_LIMIT: usize = 50;
const WINDOW_NS: u64 = 10_000_000_000;
const COOLDOWN_NS: u64 = 30_000_000_000;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    StoreNotification,
    Reconciliation,
    InitialSnapshot,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubjectRef {
    pub user_id: String,
    pub channel_id: String,
    pub call_id: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ObservationKind {
    Joined,
    Left,
    Gap { reason: String, missing: u64 },
    Baseline,
}

#[derive(Clone, Debug, Serialize)]
pub struct Observation {
    pub schema_version: u16,
    pub observation_id: String,
    pub source: ObservationSource,
    pub monotonic_ns: u64,
    pub wall_time_unix_ms: u64,
    pub session_id: String,
    pub subject: Option<SubjectRef>,
    pub kind: ObservationKind,
    pub evidence_quality: &'static str,
    pub clock_uncertainty: &'static str,
    pub history_sequence: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct Incident {
    pub incident_id: String,
    pub detector_version: &'static str,
    pub rule: &'static str,
    pub explanation: &'static str,
    pub uncertainty: &'static str,
    pub confidence: &'static str,
    pub recommended_action: &'static str,
    pub call_id: u64,
    pub channel_id: String,
    pub wall_time_unix_ms: u64,
    pub evidence: Vec<Observation>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct IngestionHealth {
    pub queue_capacity: usize,
    #[serde(flatten)]
    pub queue: BusMetrics,
    pub accepted: u64,
    pub rejected_session: u64,
    pub rejected_replay: u64,
    pub rejected_clock: u64,
    pub rejected_state: u64,
    pub diagnostics: Option<HealthState>,
}

impl Default for IngestionHealth {
    fn default() -> Self {
        Self {
            queue_capacity: BUS_CAPACITY,
            queue: BusMetrics::default(),
            accepted: 0,
            rejected_session: 0,
            rejected_replay: 0,
            rejected_clock: 0,
            rejected_state: 0,
            diagnostics: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AcousticSummary {
    pub schema_version: u16,
    pub source: AcousticSource,
    pub status: AcousticState,
    pub frames: u64,
    pub discontinuities: u64,
    pub missing_messages: u64,
    pub last_receipt_ns: u64,
    pub latest: Option<AcousticFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_health: Option<crate::acoustic::OutputHealth>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcousticOwnerStatus {
    NotConfigured,
    Prepared,
    Running,
    Finished,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcousticOwnerOutcome {
    Completed,
    Faulted,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcousticOwnerReason {
    DiscoveryFailed,
    SignalSetupFailed,
    SourceRejected,
    RecorderStartFailed,
    InputStalled,
    StreamEnded,
    StreamReadFailed,
    ProcessingFailed,
    DeliveryFailed,
    RecorderExited,
    CleanupFailed,
    InternalFailure,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct AcousticOwnerResult {
    pub outcome: AcousticOwnerOutcome,
    pub cleanup_confirmed: bool,
    pub reason: Option<AcousticOwnerReason>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct AcousticOwner {
    pub schema_version: u16,
    pub status: AcousticOwnerStatus,
    pub call_bound: bool,
    pub result: Option<AcousticOwnerResult>,
}

#[derive(Debug, Serialize)]
pub struct Intelligence {
    pub schema_version: u16,
    pub session_id: String,
    pub observations: VecDeque<Observation>,
    pub reset_count: u64,
    pub incidents: VecDeque<Incident>,
    pub dropped_observations: u64,
    pub dropped_incidents: u64,
    pub missing_transitions: u64,
    pub gap_count: u64,
    pub ingestion: IngestionHealth,
    pub acoustic: Option<AcousticSummary>,
    pub acoustic_owner: AcousticOwner,
    pub acoustic_start_available: bool,
    pub acoustic_stop_available: bool,
    #[serde(skip)]
    producer_watermarks: [Option<(u64, u64)>; 3],
    #[serde(skip)]
    last_call_id: u64,
    #[serde(skip)]
    sequence: u64,
    #[serde(skip)]
    last_history_sequence: u64,
    #[serde(skip)]
    last_time: Option<u64>,
    #[serde(skip)]
    context: Option<(u64, String)>,
    #[serde(skip)]
    window: VecDeque<Observation>,
    #[serde(skip)]
    cooldowns: [Option<u64>; 2],
    #[serde(skip)]
    interrupted: bool,
}

impl Intelligence {
    pub fn new(session_id: String) -> Self {
        Self {
            schema_version: 1,
            session_id,
            observations: VecDeque::new(),
            reset_count: 0,
            incidents: VecDeque::new(),
            dropped_observations: 0,
            dropped_incidents: 0,
            missing_transitions: 0,
            gap_count: 0,
            ingestion: IngestionHealth::default(),
            acoustic: None,
            acoustic_owner: AcousticOwner {
                schema_version: 2,
                status: AcousticOwnerStatus::NotConfigured,
                call_bound: false,
                result: None,
            },
            acoustic_start_available: false,
            acoustic_stop_available: false,
            producer_watermarks: [None; 3],
            last_call_id: 0,
            sequence: 0,
            last_history_sequence: 0,
            last_time: None,
            context: None,
            window: VecDeque::new(),
            cooldowns: [None; 2],
            interrupted: false,
        }
    }

    pub fn reset_review(&mut self) {
        self.reset_count = self.reset_count.saturating_add(1);
        self.observations.clear();
        self.incidents.clear();
        self.window.clear();
        self.cooldowns = [None; 2];
        self.dropped_observations = 0;
        self.dropped_incidents = 0;
        self.missing_transitions = 0;
        self.gap_count = 0;
    }

    pub fn configure_acoustic(&mut self, deferred: bool) {
        self.acoustic_owner.call_bound = deferred;
        self.acoustic_owner.status = if deferred {
            AcousticOwnerStatus::Prepared
        } else {
            AcousticOwnerStatus::Running
        };
        self.acoustic_owner.result = None;
    }

    pub fn start_acoustic(&mut self) {
        if self.acoustic_owner.status == AcousticOwnerStatus::Prepared {
            self.acoustic_owner.status = AcousticOwnerStatus::Running;
        }
    }

    pub fn active_call_id(&self) -> Option<u64> {
        self.context.as_ref().map(|(call_id, _)| *call_id)
    }

    pub fn finish_acoustic_owner(
        &mut self,
        completed: bool,
        cleanup_confirmed: bool,
        reason: Option<AcousticOwnerReason>,
    ) {
        if matches!(
            self.acoustic_owner.status,
            AcousticOwnerStatus::Prepared | AcousticOwnerStatus::Running
        ) {
            self.acoustic_owner.status = AcousticOwnerStatus::Finished;
            self.acoustic_owner.result = Some(AcousticOwnerResult {
                outcome: if completed {
                    AcousticOwnerOutcome::Completed
                } else {
                    AcousticOwnerOutcome::Faulted
                },
                cleanup_confirmed,
                reason,
            });
        }
    }

    pub fn finish_acoustic(&mut self, source: AcousticSource, completed: bool, receipt_ns: u64) {
        let summary = self.acoustic.get_or_insert(AcousticSummary {
            schema_version: 2,
            source,
            status: AcousticState::Starting,
            frames: 0,
            discontinuities: 0,
            missing_messages: 0,
            last_receipt_ns: receipt_ns,
            latest: None,
            processing_health: None,
        });
        if summary.source == source {
            summary.status = if completed {
                AcousticState::Stopped
            } else {
                AcousticState::Faulted
            };
            summary.last_receipt_ns = summary.last_receipt_ns.max(receipt_ns);
        }
    }

    pub fn observe(
        &mut self,
        history: &History,
        monotonic_ns: u64,
        wall_time_unix_ms: u64,
        source: ObservationSource,
    ) {
        self.observe_frame(
            &VoiceFrame::from_history(history, source, false),
            monotonic_ns,
            wall_time_unix_ms,
        );
    }

    pub fn drain(&mut self, bus: &SensorBus) -> usize {
        let (messages, metrics) = bus.drain();
        self.ingestion.queue = metrics;
        let count = messages.len();
        for message in messages {
            if message.session_id != self.session_id {
                self.ingestion.rejected_session += 1;
                continue;
            }
            let index = match message.producer {
                Producer::Application => 0,
                Producer::LocalDiagnostics => 1,
                Producer::Acoustic => 2,
            };
            let previous = self.producer_watermarks[index];
            if previous.is_some_and(|(sequence, _)| message.producer_sequence <= sequence) {
                self.ingestion.rejected_replay += 1;
                continue;
            }
            if previous.is_some_and(|(_, time)| message.monotonic_ns < time) {
                self.ingestion.rejected_clock += 1;
                continue;
            }
            let expected_sequence = previous.map_or(1, |(sequence, _)| sequence.saturating_add(1));
            let sequence_gap = message.producer_sequence > expected_sequence;
            match &message.payload {
                SensorPayload::Voice(frame) => {
                    if frame.latest_sequence < self.last_history_sequence
                        || frame.context.as_ref().is_some_and(|context| {
                            context.call_id < self.last_call_id
                                || (context.call_id == self.last_call_id && self.context.is_none())
                                || self.context.as_ref().is_some_and(|(call_id, channel_id)| {
                                    *call_id == context.call_id && channel_id != &context.channel_id
                                })
                        })
                    {
                        self.ingestion.rejected_state += 1;
                        continue;
                    }
                    if sequence_gap {
                        self.gap(message.monotonic_ns, message.wall_time_unix_ms, frame.source,
                            "Sensor ingestion lost messages; intermediate capture coverage is unknown", 0);
                    }
                    if frame.capture_gap {
                        self.capture_gap(
                            message.monotonic_ns,
                            message.wall_time_unix_ms,
                            frame.source,
                        );
                    }
                    if let Some(context) = &frame.context {
                        self.last_call_id = context.call_id;
                    }
                    self.observe_frame(frame, message.monotonic_ns, message.wall_time_unix_ms);
                }
                SensorPayload::Health(state) => {
                    self.ingestion.diagnostics = Some(*state);
                }
                SensorPayload::Acoustic(_) | SensorPayload::AcousticState(_, _) => {
                    let (source, state, frame) = match &message.payload {
                        SensorPayload::Acoustic(frame) => {
                            (frame.source, AcousticState::Receiving, Some(frame))
                        }
                        SensorPayload::AcousticState(source, state) => (*source, *state, None),
                        _ => unreachable!(),
                    };
                    if self.acoustic.as_ref().is_some_and(|summary| {
                        summary.source != source
                            || matches!(
                                summary.status,
                                AcousticState::Stopped | AcousticState::Faulted
                            )
                            || (state == AcousticState::Starting
                                && summary.status != AcousticState::Starting)
                            || frame.is_some_and(|frame| {
                                summary.latest.as_ref().is_some_and(|last| {
                                    frame.frame_start_ns
                                        < last
                                            .frame_start_ns
                                            .saturating_add(crate::acoustic::FRAME_DURATION_NS)
                                })
                            })
                    }) {
                        self.ingestion.rejected_state += 1;
                        continue;
                    }
                    let anchored =
                        frame.is_some_and(|frame| frame.session_start_interval_ns.is_some());
                    let summary = self.acoustic.get_or_insert(AcousticSummary {
                        schema_version: if anchored { 3 } else { 2 },
                        source,
                        status: state,
                        frames: 0,
                        discontinuities: 0,
                        missing_messages: 0,
                        last_receipt_ns: message.monotonic_ns,
                        latest: None,
                        processing_health: None,
                    });
                    if anchored {
                        summary.schema_version = summary.schema_version.max(3);
                    }
                    summary.status = state;
                    summary.last_receipt_ns = message.monotonic_ns;
                    summary.missing_messages = summary.missing_messages.saturating_add(
                        message.producer_sequence.saturating_sub(expected_sequence),
                    );
                    if let Some(frame) = frame {
                        summary.frames = summary.frames.saturating_add(1);
                        summary.discontinuities = summary
                            .discontinuities
                            .saturating_add(u64::from(frame.discontinuity));
                        summary.latest = Some(frame.clone());
                    }
                }
                SensorPayload::AcousticHealth(report) => {
                    let Some(summary) = self.acoustic.as_mut() else {
                        self.ingestion.rejected_state += 1;
                        continue;
                    };
                    let compatible = matches!(
                        (summary.status, report.health.status),
                        (
                            AcousticState::Starting | AcousticState::Receiving,
                            crate::acoustic::SessionStatus::Active
                        ) | (
                            AcousticState::Stopped,
                            crate::acoustic::SessionStatus::Stopped
                        ) | (
                            AcousticState::Faulted,
                            crate::acoustic::SessionStatus::Faulted { .. }
                        )
                    );
                    if summary.source != report.source || !compatible {
                        self.ingestion.rejected_state += 1;
                        continue;
                    }
                    summary.schema_version = 4;
                    summary.last_receipt_ns = message.monotonic_ns;
                    summary.missing_messages = summary.missing_messages.saturating_add(
                        message.producer_sequence.saturating_sub(expected_sequence),
                    );
                    summary.processing_health = Some(report.health);
                }
            }
            self.producer_watermarks[index] =
                Some((message.producer_sequence, message.monotonic_ns));
            self.ingestion.accepted += 1;
        }
        count
    }

    fn observe_frame(&mut self, frame: &VoiceFrame, monotonic_ns: u64, wall_time_unix_ms: u64) {
        let source = frame.source;
        let context = frame
            .context
            .as_ref()
            .map(|context| (context.call_id, context.channel_id.clone()));
        let boundary = self.context != context;
        if boundary {
            self.window.clear();
            self.cooldowns = [None; 2];
            self.context = context;
        }
        if let Some(previous) = self.last_time
            && (monotonic_ns < previous || monotonic_ns.saturating_sub(previous) > 5_000_000_000)
        {
            self.gap(
                monotonic_ns,
                wall_time_unix_ms,
                source,
                "Observation interval interrupted or clock moved backwards",
                0,
            );
        }
        self.last_time = Some(monotonic_ns);
        if boundary {
            self.push(
                monotonic_ns,
                wall_time_unix_ms,
                source,
                None,
                ObservationKind::Baseline,
                None,
            );
        }
        let events: Vec<_> = frame
            .transitions
            .iter()
            .filter(|event| event.sequence > self.last_history_sequence)
            .collect();
        let expected = frame
            .latest_sequence
            .saturating_sub(self.last_history_sequence);
        let missing = expected.saturating_sub(events.len() as u64);
        if missing > 0 {
            self.gap(
                monotonic_ns,
                wall_time_unix_ms,
                source,
                "Transitions unavailable before fusion",
                missing,
            );
        }
        for event in events {
            let Some((call_id, channel_id)) = &self.context else {
                continue;
            };
            if boundary || &event.channel_id != channel_id {
                continue;
            }
            let subject = SubjectRef {
                user_id: event.user_id.clone(),
                channel_id: channel_id.clone(),
                call_id: *call_id,
            };
            let kind = match event.action {
                Action::Joined => ObservationKind::Joined,
                Action::Left => ObservationKind::Left,
            };
            self.push(
                monotonic_ns,
                wall_time_unix_ms,
                source,
                Some(subject),
                kind,
                Some(event.sequence),
            );
        }
        self.last_history_sequence = frame.latest_sequence;
        if self.interrupted {
            self.window.clear();
            self.interrupted = false;
        } else {
            self.detect(monotonic_ns, wall_time_unix_ms);
        }
    }

    pub fn capture_gap(&mut self, time: u64, wall: u64, source: ObservationSource) {
        self.gap(
            time,
            wall,
            source,
            "Renderer capture was unavailable; intermediate transitions are unknown",
            0,
        );
    }

    fn gap(&mut self, time: u64, wall: u64, source: ObservationSource, reason: &str, missing: u64) {
        self.gap_count = self.gap_count.saturating_add(1);
        self.missing_transitions = self.missing_transitions.saturating_add(missing);
        self.window.clear();
        self.interrupted = true;
        self.push(
            time,
            wall,
            source,
            None,
            ObservationKind::Gap {
                reason: reason.into(),
                missing,
            },
            None,
        );
    }

    fn push(
        &mut self,
        time: u64,
        wall: u64,
        source: ObservationSource,
        subject: Option<SubjectRef>,
        kind: ObservationKind,
        history_sequence: Option<u64>,
    ) {
        self.sequence += 1;
        let observation = Observation {
            schema_version: 1,
            observation_id: format!("{}:{}", self.session_id, self.sequence),
            source,
            monotonic_ns: time,
            wall_time_unix_ms: wall,
            session_id: self.session_id.clone(),
            subject,
            kind,
            evidence_quality: "snapshot_difference",
            clock_uncertainty: "Backend receipt time; renderer sampling and transport latency are unmeasured",
            history_sequence,
        };
        if observation.subject.is_some() {
            self.window.push_back(observation.clone());
            if self.window.len() > OBSERVATION_LIMIT {
                self.window.pop_front();
            }
        }
        self.observations.push_back(observation);
        if self.observations.len() > OBSERVATION_LIMIT {
            self.observations.pop_front();
            self.dropped_observations = self.dropped_observations.saturating_add(1);
        }
    }

    fn detect(&mut self, time: u64, wall: u64) {
        self.window.retain(|event| {
            time.checked_sub(event.monotonic_ns)
                .is_some_and(|age| age <= WINDOW_NS)
        });
        for rule_index in 0..2 {
            if self.cooldowns[rule_index]
                .is_some_and(|previous| time.saturating_sub(previous) < COOLDOWN_NS)
            {
                continue;
            }
            let mut evidence = Vec::new();
            if rule_index == 0 {
                let joins: Vec<_> = self
                    .window
                    .iter()
                    .filter(|event| event.kind == ObservationKind::Joined)
                    .collect();
                let users: BTreeSet<_> = joins
                    .iter()
                    .filter_map(|event| event.subject.as_ref().map(|subject| &subject.user_id))
                    .collect();
                if users.len() >= 5 {
                    let mut retained_users = BTreeSet::new();
                    evidence = joins
                        .into_iter()
                        .filter(|event| {
                            event.subject.as_ref().is_some_and(|subject| {
                                retained_users.insert(subject.user_id.clone())
                            })
                        })
                        .take(20)
                        .cloned()
                        .collect();
                }
            } else {
                for user_id in self
                    .window
                    .iter()
                    .filter_map(|event| event.subject.as_ref().map(|subject| &subject.user_id))
                    .collect::<BTreeSet<_>>()
                {
                    let user_events: Vec<_> = self
                        .window
                        .iter()
                        .filter(|event| {
                            event
                                .subject
                                .as_ref()
                                .is_some_and(|subject| &subject.user_id == user_id)
                        })
                        .collect();
                    let returns = user_events
                        .windows(2)
                        .filter(|pair| {
                            pair[0].kind == ObservationKind::Left
                                && pair[1].kind == ObservationKind::Joined
                        })
                        .count();
                    if returns >= 3 {
                        evidence = user_events.into_iter().take(20).cloned().collect();
                        break;
                    }
                }
            }
            if evidence.is_empty() {
                continue;
            }
            let Some((call_id, channel_id)) = &self.context else {
                continue;
            };
            self.incidents.push_back(Incident {
                incident_id: format!("{}:finding:{}:{}", self.session_id, self.sequence, rule_index),
                detector_version: "voice-patterns/0.1-experimental",
                rule: if rule_index == 0 { "join_burst" } else { "rapid_reentry" },
                explanation: if rule_index == 0 { "At least five distinct observed arrivals within ten seconds of backend receipt time." } else { "At least three observed leave/return pairs for one participant within ten seconds of backend receipt time." },
                uncertainty: "Experimental fixed threshold, not calibrated. Reconnects, normal group arrivals, and delayed snapshots can explain this pattern. This is not evidence of intent or coordination.",
                confidence: "uncalibrated", recommended_action: "Review observed facts; no automatic action is taken.",
                call_id: *call_id, channel_id: channel_id.clone(), wall_time_unix_ms: wall, evidence,
            });
            self.cooldowns[rule_index] = Some(time);
            if self.incidents.len() > INCIDENT_LIMIT {
                self.incidents.pop_front();
                self.dropped_incidents += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Channel, Command, User};

    const WALL_ORIGIN_MS: u64 = 1_800_000_000_000;

    fn bus_voice(
        history: &History,
        sequence: u64,
        elapsed_ms: u64,
    ) -> crate::ingestion::SensorMessage {
        crate::ingestion::SensorMessage {
            schema_version: 1,
            session_id: "bus-session".into(),
            producer: Producer::Application,
            producer_sequence: sequence,
            monotonic_ns: elapsed_ms * 1_000_000,
            wall_time_unix_ms: WALL_ORIGIN_MS + elapsed_ms,
            payload: SensorPayload::Voice(VoiceFrame::from_history(
                history,
                ObservationSource::StoreNotification,
                false,
            )),
        }
    }

    fn bus_snapshot(history: &mut History, ids: &[&str], elapsed_ms: u64) {
        history
            .apply(snapshot(ids), WALL_ORIGIN_MS + elapsed_ms)
            .unwrap();
    }

    fn publish_and_drain(
        engine: &mut Intelligence,
        bus: &SensorBus,
        message: crate::ingestion::SensorMessage,
    ) {
        bus.publisher().publish(message).unwrap();
        assert_eq!(engine.drain(bus), 1);
        assert_eq!(engine.drain(bus), 0);
    }

    #[test]
    fn bus_baseline_five_joins_and_cloned_replay_preserve_exact_ids() {
        let bus = SensorBus::default();
        let mut engine = Intelligence::new("bus-session".into());
        let mut history = History::default();
        let ids = ["00000000000000000001", "9007199254740993", "3", "4", "5"];
        bus_snapshot(&mut history, &[], 0);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 1, 0));
        assert_eq!(engine.observations.len(), 1);
        assert_eq!(engine.observations[0].kind, ObservationKind::Baseline);
        assert!(engine.observations[0].subject.is_none());
        assert!(engine.incidents.is_empty());

        bus_snapshot(&mut history, &ids, 1);
        let arrivals = bus_voice(&history, 2, 1);
        publish_and_drain(&mut engine, &bus, arrivals.clone());
        assert_eq!(engine.incidents.len(), 1);
        let incident = &engine.incidents[0];
        assert_eq!(incident.rule, "join_burst");
        assert_eq!(incident.channel_id, "100");
        assert_eq!(incident.call_id, history.current_call.as_ref().unwrap().id);
        assert_eq!(incident.wall_time_unix_ms, WALL_ORIGIN_MS + 1);
        assert_eq!(incident.evidence.len(), 5);
        assert_eq!(
            incident
                .evidence
                .iter()
                .map(|event| {
                    let subject = event.subject.as_ref().unwrap();
                    assert_eq!(subject.channel_id, "100");
                    assert_eq!(subject.call_id, incident.call_id);
                    assert_eq!(event.kind, ObservationKind::Joined);
                    assert_eq!(event.source, ObservationSource::StoreNotification);
                    subject.user_id.as_str()
                })
                .collect::<BTreeSet<_>>(),
            ids.into_iter().collect::<BTreeSet<_>>()
        );
        let observations = serde_json::to_value(&engine.observations).unwrap();
        let incidents = serde_json::to_value(&engine.incidents).unwrap();
        publish_and_drain(&mut engine, &bus, arrivals);
        assert_eq!(engine.ingestion.accepted, 2);
        assert_eq!(engine.ingestion.rejected_replay, 1);
        assert_eq!(
            serde_json::to_value(&engine.observations).unwrap(),
            observations
        );
        assert_eq!(serde_json::to_value(&engine.incidents).unwrap(), incidents);
        assert_eq!(engine.gap_count, 0);
    }

    #[test]
    fn burst_evidence_retains_distinct_triggering_users_after_repeated_joins() {
        let mut engine = Intelligence::new("session".into());
        let mut history = History::default();
        bus_snapshot(&mut history, &[], 0);
        engine.observe(&history, 0, 0, ObservationSource::InitialSnapshot);
        for time in 1..=40 {
            bus_snapshot(&mut history, if time % 2 == 0 { &[] } else { &["1"] }, time);
            engine.observe(
                &history,
                time * 1_000_000,
                time,
                ObservationSource::StoreNotification,
            );
        }
        bus_snapshot(&mut history, &["1", "2", "3", "4", "5"], 41);
        engine.observe(
            &history,
            41_000_000,
            41,
            ObservationSource::StoreNotification,
        );
        let burst = engine
            .incidents
            .iter()
            .find(|incident| incident.rule == "join_burst")
            .unwrap();
        let users: BTreeSet<_> = burst
            .evidence
            .iter()
            .map(|event| event.subject.as_ref().unwrap().user_id.as_str())
            .collect();
        assert_eq!(users, BTreeSet::from(["1", "2", "3", "4", "5"]));
        assert!(burst.evidence.len() <= 20);
    }

    fn bus_frame(message: &mut crate::ingestion::SensorMessage) -> &mut VoiceFrame {
        match &mut message.payload {
            SensorPayload::Voice(frame) => frame,
            _ => panic!("expected a voice message"),
        }
    }

    fn bus_evidence(engine: &Intelligence) -> serde_json::Value {
        serde_json::json!({
            "observations": engine.observations,
            "incidents": engine.incidents,
            "gap_count": engine.gap_count,
            "missing_transitions": engine.missing_transitions,
            "dropped_observations": engine.dropped_observations,
            "dropped_incidents": engine.dropped_incidents,
        })
    }

    #[test]
    fn bus_rejections_are_atomic_and_do_not_poison_recovery_watermarks() {
        for rejection in [
            "session",
            "replay",
            "clock",
            "old_call",
            "remapped_channel",
            "history",
        ] {
            let bus = SensorBus::default();
            let mut engine = Intelligence::new("bus-session".into());
            let mut history = History::default();
            bus_snapshot(&mut history, &[], 0);
            let mut baseline = bus_voice(&history, 1, 0);
            bus_frame(&mut baseline).context.as_mut().unwrap().call_id = 2;
            publish_and_drain(&mut engine, &bus, baseline);
            bus_snapshot(&mut history, &["1", "2", "3", "4"], 10);
            let mut partial = bus_voice(&history, 2, 10);
            bus_frame(&mut partial).context.as_mut().unwrap().call_id = 2;
            publish_and_drain(&mut engine, &bus, partial);
            let before = bus_evidence(&engine);

            bus_snapshot(&mut history, &["1", "2", "3", "4", "5"], 11);
            let mut recovery = bus_voice(&history, 3, 11);
            bus_frame(&mut recovery).context.as_mut().unwrap().call_id = 2;
            let mut rejected = recovery.clone();
            match rejection {
                "session" => {
                    rejected.session_id = "previous-session".into();
                    rejected.producer_sequence = 999;
                    rejected.monotonic_ns = u64::MAX;
                }
                "replay" => {
                    rejected.producer_sequence = 2;
                    rejected.monotonic_ns = u64::MAX;
                }
                "clock" => rejected.monotonic_ns = 9_000_000,
                "old_call" => bus_frame(&mut rejected).context.as_mut().unwrap().call_id = 1,
                "remapped_channel" => {
                    let frame = bus_frame(&mut rejected);
                    frame.context.as_mut().unwrap().channel_id = "200".into();
                    for transition in &mut frame.transitions {
                        transition.channel_id = "200".into();
                    }
                }
                "history" => {
                    let frame = bus_frame(&mut rejected);
                    frame.latest_sequence = 3;
                    frame
                        .transitions
                        .retain(|transition| transition.sequence <= 3);
                }
                _ => unreachable!(),
            }
            publish_and_drain(&mut engine, &bus, rejected);
            assert_eq!(bus_evidence(&engine), before, "{rejection}");
            assert_eq!(engine.ingestion.accepted, 2, "{rejection}");
            assert_eq!(
                engine.ingestion.rejected_session,
                u64::from(rejection == "session")
            );
            assert_eq!(
                engine.ingestion.rejected_replay,
                u64::from(rejection == "replay")
            );
            assert_eq!(
                engine.ingestion.rejected_clock,
                u64::from(rejection == "clock")
            );
            assert_eq!(
                engine.ingestion.rejected_state,
                u64::from(matches!(
                    rejection,
                    "old_call" | "remapped_channel" | "history"
                ))
            );
            publish_and_drain(&mut engine, &bus, recovery);
            assert_eq!(engine.ingestion.accepted, 3, "{rejection}");
            assert_eq!(engine.incidents.len(), 1, "{rejection}");
            assert_eq!(engine.incidents[0].evidence.len(), 5, "{rejection}");
            assert_eq!(engine.gap_count, 0, "{rejection}");
        }
    }

    #[test]
    fn bus_same_call_cannot_remap_channels_after_a_disconnected_context() {
        let bus = SensorBus::default();
        let mut engine = Intelligence::new("bus-session".into());
        let mut history = History::default();
        bus_snapshot(&mut history, &[], 0);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 1, 0));
        let mut disconnected = bus_voice(&history, 2, 1);
        bus_frame(&mut disconnected).context = None;
        publish_and_drain(&mut engine, &bus, disconnected);
        let before = bus_evidence(&engine);

        let mut remapped = bus_voice(&history, 3, 2);
        bus_frame(&mut remapped)
            .context
            .as_mut()
            .unwrap()
            .channel_id = "200".into();
        publish_and_drain(&mut engine, &bus, remapped);
        assert_eq!(
            engine.ingestion.rejected_state, 1,
            "a call ID must retain its original channel after disconnect"
        );
        assert_eq!(engine.ingestion.accepted, 2);
        assert_eq!(bus_evidence(&engine), before);

        history
            .apply(
                Command::Snapshot {
                    channel: None,
                    users: Vec::new(),
                },
                1,
            )
            .unwrap();
        bus_snapshot(&mut history, &[], 2);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 3, 2));
        assert_eq!(engine.ingestion.accepted, 3);
        assert_eq!(engine.ingestion.rejected_replay, 0);
        bus_snapshot(&mut history, &["1", "2", "3", "4", "5"], 3);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 4, 3));
        assert_eq!(engine.incidents.len(), 1);
        assert_eq!(engine.incidents[0].channel_id, "100");
    }

    fn bus_health(sequence: u64, time: u64, state: HealthState) -> crate::ingestion::SensorMessage {
        crate::ingestion::SensorMessage {
            schema_version: 1,
            session_id: "bus-session".into(),
            producer: Producer::LocalDiagnostics,
            producer_sequence: sequence,
            monotonic_ns: time,
            wall_time_unix_ms: u64::MAX,
            payload: SensorPayload::Health(state),
        }
    }

    #[test]
    fn bus_health_has_no_subject_findings_or_voice_context_side_effects() {
        for final_ms in [2, 10_002] {
            let bus = SensorBus::default();
            let mut engine = Intelligence::new("bus-session".into());
            let mut history = History::default();
            for (index, state) in [
                HealthState::Available,
                HealthState::Degraded,
                HealthState::Unavailable,
            ]
            .into_iter()
            .enumerate()
            {
                publish_and_drain(
                    &mut engine,
                    &bus,
                    bus_health(index as u64 + 1, u64::MAX - 1, state),
                );
                assert_eq!(engine.ingestion.diagnostics, Some(state));
                assert!(engine.observations.is_empty());
                assert!(engine.incidents.is_empty());
            }
            bus_snapshot(&mut history, &[], 0);
            publish_and_drain(&mut engine, &bus, bus_voice(&history, 1, 0));
            bus_snapshot(&mut history, &["1", "2", "3", "4"], 1);
            publish_and_drain(&mut engine, &bus, bus_voice(&history, 2, 1));
            let before = bus_evidence(&engine);
            publish_and_drain(
                &mut engine,
                &bus,
                bus_health(400, u64::MAX, HealthState::Available),
            );
            assert_eq!(bus_evidence(&engine), before);

            let mut voice_sequence = 3;
            if final_ms > 5_000 {
                for elapsed_ms in [5_000, 10_000] {
                    publish_and_drain(
                        &mut engine,
                        &bus,
                        bus_voice(&history, voice_sequence, elapsed_ms),
                    );
                    voice_sequence += 1;
                }
            }
            bus_snapshot(&mut history, &["1", "2", "3", "4", "5"], final_ms);
            publish_and_drain(
                &mut engine,
                &bus,
                bus_voice(&history, voice_sequence, final_ms),
            );
            assert_eq!(engine.incidents.len(), usize::from(final_ms == 2));
            assert_eq!(engine.gap_count, 0);
            assert_eq!(engine.ingestion.rejected_clock, 0);
            assert_eq!(engine.ingestion.rejected_replay, 0);
            assert_eq!(engine.observations.len(), 6);
            assert_eq!(
                engine
                    .observations
                    .iter()
                    .filter(|event| event.subject.is_none())
                    .count(),
                1
            );
            assert_eq!(
                engine.observations.back().unwrap().monotonic_ns,
                final_ms * 1_000_000
            );
            assert_eq!(
                engine.observations.back().unwrap().wall_time_unix_ms,
                WALL_ORIGIN_MS + final_ms
            );
        }
    }

    #[test]
    fn bus_diagnostic_rejections_use_independent_sequence_and_clock_watermarks() {
        let bus = SensorBus::default();
        let mut engine = Intelligence::new("bus-session".into());
        publish_and_drain(
            &mut engine,
            &bus,
            bus_health(1, 100, HealthState::Available),
        );
        publish_and_drain(
            &mut engine,
            &bus,
            bus_health(1, 200, HealthState::Unavailable),
        );
        publish_and_drain(
            &mut engine,
            &bus,
            bus_health(2, 99, HealthState::Unavailable),
        );
        assert_eq!(engine.ingestion.diagnostics, Some(HealthState::Available));
        publish_and_drain(&mut engine, &bus, bus_health(2, 100, HealthState::Degraded));
        assert_eq!(engine.ingestion.accepted, 2);
        assert_eq!(engine.ingestion.rejected_replay, 1);
        assert_eq!(engine.ingestion.rejected_clock, 1);
        assert_eq!(engine.ingestion.diagnostics, Some(HealthState::Degraded));
        assert!(engine.observations.is_empty());
        assert!(engine.incidents.is_empty());
    }

    #[test]
    fn bus_voice_overflow_preserves_pre_loss_findings_and_suppresses_recovery() {
        let bus = SensorBus::default();
        let publisher = bus.publisher();
        let mut engine = Intelligence::new("bus-session".into());
        let mut history = History::default();
        bus_snapshot(&mut history, &[], 0);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 1, 0));
        bus_snapshot(&mut history, &["1", "2", "3", "4", "5"], 1);
        assert_eq!(BUS_CAPACITY, 64);
        for sequence in 2..=65 {
            publisher
                .publish(bus_voice(&history, sequence, sequence))
                .unwrap();
        }
        assert_eq!(
            publisher.publish(bus_voice(&history, 66, 66)),
            Err(crate::ingestion::PublishError::Full)
        );
        let mut invalid = bus_voice(&history, 66, 66);
        invalid.schema_version = 2;
        assert_eq!(
            publisher.publish(invalid),
            Err(crate::ingestion::PublishError::Invalid)
        );
        assert_eq!(engine.drain(&bus), 64);
        assert_eq!(engine.drain(&bus), 0);
        assert_eq!(engine.incidents.len(), 1);
        assert_eq!(engine.gap_count, 0);
        assert!(
            engine
                .observations
                .iter()
                .any(|event| event.kind == ObservationKind::Joined)
        );
        assert_eq!(
            serde_json::to_value(&engine.ingestion).unwrap(),
            serde_json::json!({
                "queue_capacity": 64,
                "dropped_full": 1,
                "rejected_invalid": 1,
                "rejected_closed": 0,
                "accepted": 65,
                "rejected_session": 0,
                "rejected_replay": 0,
                "rejected_clock": 0,
                "rejected_state": 0,
                "diagnostics": null,
            })
        );
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 67, 67));
        assert_eq!(engine.incidents.len(), 1);
        assert_eq!(engine.gap_count, 1);
        let gaps = engine.gap_count;
        for sequence in 68..=98 {
            publish_and_drain(
                &mut engine,
                &bus,
                bus_voice(&history, sequence, 67 + (sequence - 67) * 1000),
            );
        }
        bus_snapshot(&mut history, &[], 32067);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 99, 32067));
        bus_snapshot(&mut history, &["6", "7", "8", "9", "10"], 33067);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 100, 33067));
        assert_eq!(engine.incidents.len(), 2);
        assert_eq!(engine.incidents[1].evidence.len(), 5);
        assert!(
            engine.incidents[0]
                .evidence
                .iter()
                .all(|event| event.monotonic_ns == 2_000_000)
        );
        assert!(
            engine.incidents[1]
                .evidence
                .iter()
                .all(|event| event.monotonic_ns == 33_067_000_000)
        );
        assert_eq!(engine.gap_count, gaps);
        assert_eq!(engine.ingestion.queue.dropped_full, 1);
    }

    #[test]
    fn bus_producer_sequence_gap_discards_pending_and_recovery_evidence() {
        let bus = SensorBus::default();
        let mut engine = Intelligence::new("bus-session".into());
        let mut history = History::default();
        bus_snapshot(&mut history, &[], 0);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 1, 0));
        bus_snapshot(&mut history, &["1", "2", "3", "4"], 1);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 2, 1));
        bus_snapshot(&mut history, &["1", "2", "3", "4", "5"], 2);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 4, 2));
        assert_eq!(engine.gap_count, 1);
        assert_eq!(engine.missing_transitions, 0);
        assert!(engine.incidents.is_empty());
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 5, 3));
        assert!(engine.incidents.is_empty());
        bus_snapshot(&mut history, &[], 4);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 6, 4));
        bus_snapshot(&mut history, &["6", "7", "8", "9", "10"], 5);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 7, 5));
        assert_eq!(engine.incidents.len(), 1);
        assert!(
            engine.incidents[0]
                .evidence
                .iter()
                .all(|event| event.monotonic_ns == 5_000_000)
        );
        assert_eq!(engine.gap_count, 1);
        assert_eq!(engine.ingestion.accepted, 6);
    }

    #[test]
    fn bus_history_capture_gap_suppresses_recovery_until_fresh_evidence() {
        let bus = SensorBus::default();
        let mut engine = Intelligence::new("bus-session".into());
        let mut history = History::default();
        bus_snapshot(&mut history, &[], 0);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 1, 0));
        bus_snapshot(&mut history, &["1", "2", "3", "4"], 1);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 2, 1));
        bus_snapshot(&mut history, &["1", "2", "3", "4", "5"], 2);
        let mut interrupted = bus_voice(&history, 3, 2);
        interrupted.payload = SensorPayload::Voice(VoiceFrame::from_history(
            &history,
            ObservationSource::Reconciliation,
            true,
        ));
        publish_and_drain(&mut engine, &bus, interrupted);
        assert_eq!(engine.gap_count, 1);
        assert_eq!(engine.missing_transitions, 0);
        assert!(engine.incidents.is_empty());
        let latest = engine.observations.back().unwrap();
        assert_eq!(latest.kind, ObservationKind::Joined);
        assert_eq!(latest.source, ObservationSource::Reconciliation);
        assert_eq!(latest.subject.as_ref().unwrap().user_id, "5");
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 4, 3));
        assert!(engine.incidents.is_empty());
        bus_snapshot(&mut history, &[], 4);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 5, 4));
        bus_snapshot(&mut history, &["6", "7", "8", "9", "10"], 5);
        publish_and_drain(&mut engine, &bus, bus_voice(&history, 6, 5));
        assert_eq!(engine.incidents.len(), 1);
        assert_eq!(engine.incidents[0].evidence.len(), 5);
        assert!(
            engine.incidents[0]
                .evidence
                .iter()
                .all(|event| event.monotonic_ns == 5_000_000)
        );
        assert_eq!(engine.gap_count, 1);
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PatternTrace {
        name: String,
        expected_rules: Vec<String>,
        expected_transitions: usize,
        steps: Vec<PatternStep>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PatternStep {
        elapsed_ms: u64,
        ids: Vec<String>,
    }

    fn observe_at(engine: &mut Intelligence, history: &History, elapsed_ms: u64) {
        engine.observe(
            history,
            elapsed_ms * 1_000_000,
            WALL_ORIGIN_MS + elapsed_ms,
            ObservationSource::Reconciliation,
        );
    }

    fn apply_at(
        engine: &mut Intelligence,
        history: &mut History,
        channel_id: &str,
        ids: &[&str],
        elapsed_ms: u64,
    ) {
        let mut command = snapshot(ids);
        if let Command::Snapshot {
            channel: Some(channel),
            ..
        } = &mut command
        {
            channel.id = channel_id.into();
        }
        history.apply(command, WALL_ORIGIN_MS + elapsed_ms).unwrap();
        observe_at(engine, history, elapsed_ms);
    }

    fn heartbeat_until(engine: &mut Intelligence, history: &History, elapsed_ms: u64) {
        let mut previous_ms = engine.last_time.unwrap() / 1_000_000;
        assert!(elapsed_ms >= previous_ms);
        while previous_ms < elapsed_ms {
            previous_ms = (previous_ms + 5_000).min(elapsed_ms);
            observe_at(engine, history, previous_ms);
        }
    }

    #[test]
    fn cooldown_expires_at_exactly_thirty_seconds_for_each_rule() {
        for rule in ["join_burst", "rapid_reentry"] {
            let mut history = History::default();
            let mut engine = Intelligence::new(rule.into());
            let baseline: &[&str] = if rule == "join_burst" { &[] } else { &["1"] };
            apply_at(&mut engine, &mut history, "100", baseline, 0);
            let pattern: Vec<&[&str]> = if rule == "join_burst" {
                vec![&[], &["1", "2", "3", "4", "5"]]
            } else {
                vec![&[], &["1"], &[], &["1"], &[], &["1"]]
            };
            for (index, ids) in pattern.iter().enumerate() {
                apply_at(&mut engine, &mut history, "100", ids, index as u64 + 1);
            }
            assert_eq!(engine.incidents.len(), 1, "{rule}");
            assert_eq!(engine.incidents[0].rule, rule);
            let first_ms = engine.incidents[0].wall_time_unix_ms - WALL_ORIGIN_MS;
            let second_start_ms = first_ms + 29_999 - pattern.len() as u64;
            heartbeat_until(&mut engine, &history, second_start_ms);
            for (index, ids) in pattern.iter().enumerate() {
                apply_at(
                    &mut engine,
                    &mut history,
                    "100",
                    ids,
                    second_start_ms + index as u64 + 1,
                );
            }
            assert_eq!(
                engine.incidents.len(),
                1,
                "one millisecond before cooldown: {rule}"
            );
            observe_at(&mut engine, &history, first_ms + 30_000);
            assert_eq!(engine.incidents.len(), 2, "exact cooldown boundary: {rule}");
            assert_eq!(engine.incidents[1].rule, rule);
            assert_eq!(
                engine.incidents[1].wall_time_unix_ms,
                WALL_ORIGIN_MS + first_ms + 30_000
            );
            assert_eq!(engine.gap_count, 0, "{rule}");
            assert_eq!(engine.missing_transitions, 0, "{rule}");
        }
    }

    #[test]
    fn channel_switches_isolate_windows_cooldowns_and_evidence() {
        let mut history = History::default();
        let mut engine = Intelligence::new("switches".into());
        apply_at(&mut engine, &mut history, "100", &[], 0);
        apply_at(
            &mut engine,
            &mut history,
            "100",
            &["1", "2", "3", "4", "5"],
            1,
        );
        let first_call_id = engine.incidents[0].call_id;
        apply_at(&mut engine, &mut history, "100", &[], 2);
        apply_at(&mut engine, &mut history, "100", &["1", "2", "3", "4"], 3);
        apply_at(&mut engine, &mut history, "200", &[], 4);
        apply_at(&mut engine, &mut history, "200", &["5"], 5);
        assert_eq!(
            engine.incidents.len(),
            1,
            "old arrivals must not combine with the new call"
        );
        apply_at(
            &mut engine,
            &mut history,
            "200",
            &["1", "2", "3", "4", "5"],
            6,
        );
        assert_eq!(
            engine.incidents.len(),
            2,
            "old cooldown must not suppress the new call"
        );
        assert_ne!(engine.incidents[1].call_id, first_call_id);
        apply_at(
            &mut engine,
            &mut history,
            "100",
            &["1", "2", "3", "4", "5"],
            7,
        );
        assert_eq!(
            engine.incidents.len(),
            2,
            "switch baseline is not an arrival"
        );
        apply_at(&mut engine, &mut history, "100", &[], 8);
        apply_at(
            &mut engine,
            &mut history,
            "100",
            &["1", "2", "3", "4", "5"],
            9,
        );
        assert_eq!(engine.incidents.len(), 3);
        assert_ne!(engine.incidents[2].call_id, first_call_id);
        for (incident, channel_id) in engine.incidents.iter().zip(["100", "200", "100"]) {
            assert_eq!(incident.channel_id, channel_id);
            assert_eq!(incident.rule, "join_burst");
            assert_eq!(incident.evidence.len(), 5);
            assert!(incident.evidence.iter().all(|observation| {
                let subject = observation.subject.as_ref().unwrap();
                subject.channel_id == channel_id && subject.call_id == incident.call_id
            }));
        }
        assert_eq!(engine.gap_count, 0);
    }

    #[test]
    fn reset_review_preserves_history_and_guards_but_erases_pending_evidence() {
        let mut history = History::default();
        let mut engine = Intelligence::new("reset".into());
        apply_at(&mut engine, &mut history, "100", &[], 0);
        apply_at(&mut engine, &mut history, "100", &["1", "2", "3", "4"], 1);
        let history_before = serde_json::to_value(&history).unwrap();
        let sequence = engine.sequence;
        engine.producer_watermarks[0] = Some((9, 1_000_000_000));
        engine.ingestion.accepted = 9;
        engine.reset_review();
        assert_eq!(serde_json::to_value(&history).unwrap(), history_before);
        assert_eq!(engine.producer_watermarks[0], Some((9, 1_000_000_000)));
        assert_eq!(engine.ingestion.accepted, 9);
        assert_eq!(engine.session_id, "reset");
        observe_at(&mut engine, &history, 2);
        assert!(engine.observations.is_empty());
        apply_at(
            &mut engine,
            &mut history,
            "100",
            &["1", "2", "3", "4", "5"],
            3,
        );
        assert_eq!(engine.observations.len(), 1);
        assert!(engine.incidents.is_empty());
        assert!(engine.sequence > sequence);
        assert_eq!(engine.gap_count, 0);
        assert_eq!(engine.missing_transitions, 0);
    }

    #[test]
    fn reset_review_erases_incident_evidence_counters_and_cooldowns() {
        let mut history = History::default();
        let mut engine = Intelligence::new("reset-evidence".into());
        apply_at(&mut engine, &mut history, "100", &[], 0);
        apply_at(
            &mut engine,
            &mut history,
            "100",
            &["1", "2", "3", "4", "5"],
            1,
        );
        assert_eq!(engine.incidents.len(), 1);
        assert_eq!(engine.incidents[0].evidence.len(), 5);
        let previous_id = engine.incidents[0].incident_id.clone();
        engine.dropped_observations = 2;
        engine.dropped_incidents = 3;
        engine.missing_transitions = 4;
        engine.gap_count = 1;
        engine.reset_review();
        assert!(engine.observations.is_empty());
        assert!(engine.incidents.is_empty());
        assert!(engine.window.is_empty());
        assert_eq!(engine.cooldowns, [None; 2]);
        assert_eq!(engine.dropped_observations, 0);
        assert_eq!(engine.dropped_incidents, 0);
        assert_eq!(engine.missing_transitions, 0);
        assert_eq!(engine.gap_count, 0);
        assert_eq!(engine.reset_count, 1);
        apply_at(&mut engine, &mut history, "100", &[], 2);
        apply_at(
            &mut engine,
            &mut history,
            "100",
            &["1", "2", "3", "4", "5"],
            3,
        );
        assert_eq!(engine.incidents.len(), 1);
        assert_ne!(engine.incidents[0].incident_id, previous_id);
        assert!(
            engine.incidents[0]
                .evidence
                .iter()
                .all(|entry| entry.history_sequence.unwrap() > 5)
        );
    }

    #[test]
    fn clear_preserves_observations_evidence_and_sequence_without_duplicates() {
        let mut history = History::default();
        let mut engine = Intelligence::new("clear".into());
        apply_at(&mut engine, &mut history, "100", &[], 0);
        apply_at(&mut engine, &mut history, "100", &["1", "2", "3", "4"], 1);
        let before_clear = serde_json::to_value(&engine.observations).unwrap();
        history.apply(Command::Clear, WALL_ORIGIN_MS + 2).unwrap();
        observe_at(&mut engine, &history, 2);
        observe_at(&mut engine, &history, 3);
        assert!(history.events.is_empty());
        assert_eq!(
            serde_json::to_value(&engine.observations).unwrap(),
            before_clear
        );
        apply_at(
            &mut engine,
            &mut history,
            "100",
            &["1", "2", "3", "4", "5"],
            4,
        );
        assert_eq!(
            engine.incidents.len(),
            1,
            "Clear must preserve pending evidence"
        );
        assert_eq!(engine.incidents[0].evidence.len(), 5);
        let observations = serde_json::to_value(&engine.observations).unwrap();
        let incidents = serde_json::to_value(&engine.incidents).unwrap();
        history.apply(Command::Clear, WALL_ORIGIN_MS + 5).unwrap();
        observe_at(&mut engine, &history, 5);
        observe_at(&mut engine, &history, 6);
        assert_eq!(
            serde_json::to_value(&engine.observations).unwrap(),
            observations
        );
        assert_eq!(serde_json::to_value(&engine.incidents).unwrap(), incidents);
        apply_at(&mut engine, &mut history, "100", &[], 7);
        let sequences: Vec<_> = engine
            .observations
            .iter()
            .filter_map(|event| event.history_sequence)
            .collect();
        assert_eq!(sequences, (1..=10).collect::<Vec<_>>());
        let ids: BTreeSet<_> = engine
            .observations
            .iter()
            .map(|event| &event.observation_id)
            .collect();
        assert_eq!(ids.len(), engine.observations.len());
        assert_eq!(engine.observations.len(), 11);
        assert_eq!(engine.gap_count, 0);
        assert_eq!(engine.missing_transitions, 0);
    }

    #[test]
    fn capture_gap_suppresses_recovery_trigger_then_allows_fresh_evidence() {
        let mut history = History::default();
        let mut engine = Intelligence::new("capture-gap".into());
        apply_at(&mut engine, &mut history, "100", &[], 0);
        apply_at(&mut engine, &mut history, "100", &["1", "2", "3", "4"], 1);
        engine.capture_gap(2_000_000, WALL_ORIGIN_MS + 2, ObservationSource::Unknown);
        history.apply(snapshot(&[]), WALL_ORIGIN_MS + 2).unwrap();
        apply_at(
            &mut engine,
            &mut history,
            "100",
            &["1", "2", "3", "4", "5"],
            3,
        );
        assert_eq!(engine.gap_count, 1);
        assert_eq!(engine.missing_transitions, 0);
        assert!(
            engine.incidents.is_empty(),
            "five recovery arrivals must not trigger"
        );
        assert_eq!(
            engine.observations.len(),
            15,
            "recovery transitions remain observable"
        );
        assert!(matches!(
            engine.observations[5].kind,
            ObservationKind::Gap { missing: 0, .. }
        ));
        observe_at(&mut engine, &history, 4);
        assert!(
            engine.incidents.is_empty(),
            "recovery evidence must not trigger later"
        );
        apply_at(&mut engine, &mut history, "100", &[], 5);
        apply_at(
            &mut engine,
            &mut history,
            "100",
            &["1", "2", "3", "4", "5"],
            6,
        );
        assert_eq!(engine.incidents.len(), 1);
        assert_eq!(engine.incidents[0].rule, "join_burst");
        assert_eq!(engine.incidents[0].evidence.len(), 5);
        assert!(
            engine.incidents[0]
                .evidence
                .iter()
                .all(|event| event.monotonic_ns == 6_000_000)
        );
        assert_eq!(engine.gap_count, 1);
    }

    #[test]
    fn incident_retention_keeps_latest_fifty_and_counts_every_eviction() {
        let mut history = History::default();
        let mut engine = Intelligence::new("retention".into());
        apply_at(&mut engine, &mut history, "100", &[], 0);
        let mut generated_ids = Vec::new();
        for group in 0..57_u64 {
            let elapsed_ms = group * 31_000 + 1;
            heartbeat_until(&mut engine, &history, elapsed_ms - 1);
            let ids: &[&str] = if group % 2 == 0 {
                &["1", "2", "3", "4", "5"]
            } else {
                &["6", "7", "8", "9", "10"]
            };
            apply_at(&mut engine, &mut history, "100", ids, elapsed_ms);
            generated_ids.push(engine.incidents.back().unwrap().incident_id.clone());
            assert_eq!(engine.incidents.len(), ((group + 1) as usize).min(50));
            assert_eq!(engine.dropped_incidents, (group + 1).saturating_sub(50));
            assert_eq!(
                engine.incidents.back().unwrap().wall_time_unix_ms,
                WALL_ORIGIN_MS + elapsed_ms
            );
        }
        assert_eq!(INCIDENT_LIMIT, 50);
        assert_eq!(engine.incidents.len(), 50);
        assert_eq!(engine.dropped_incidents, 7);
        assert_eq!(
            engine
                .incidents
                .iter()
                .map(|incident| &incident.incident_id)
                .collect::<Vec<_>>(),
            generated_ids[7..].iter().collect::<Vec<_>>()
        );
        assert!(
            engine
                .incidents
                .iter()
                .all(|incident| incident.rule == "join_burst" && incident.evidence.len() == 5)
        );
        assert_eq!(engine.gap_count, 0);
        assert_eq!(engine.missing_transitions, 0);
    }

    #[test]
    fn synthetic_voice_pattern_traces() {
        let traces: Vec<PatternTrace> =
            serde_json::from_str(include_str!("../tests/fixtures/voice-patterns.json")).unwrap();
        assert_eq!(traces.len(), 4);
        for trace in traces {
            let mut history = History::default();
            let mut engine = Intelligence::new(trace.name.clone());
            let mut previous_ms = 0;
            for step in trace.steps {
                assert!(step.elapsed_ms >= previous_ms, "{}", trace.name);
                assert!(step.elapsed_ms - previous_ms <= 5_000, "{}", trace.name);
                let ids: Vec<_> = step.ids.iter().map(String::as_str).collect();
                apply_at(&mut engine, &mut history, "100", &ids, step.elapsed_ms);
                previous_ms = step.elapsed_ms;
            }
            let rules: Vec<_> = engine
                .incidents
                .iter()
                .map(|incident| incident.rule)
                .collect();
            assert_eq!(rules, trace.expected_rules, "{}", trace.name);
            assert_eq!(engine.gap_count, 0, "{}", trace.name);
            assert_eq!(engine.missing_transitions, 0, "{}", trace.name);
            assert_eq!(
                engine.observations.len(),
                trace.expected_transitions + 1,
                "{}",
                trace.name
            );
            assert_eq!(engine.observations[0].kind, ObservationKind::Baseline);
            for observation in &engine.observations {
                assert_eq!(
                    observation.monotonic_ns / 1_000_000,
                    observation.wall_time_unix_ms - WALL_ORIGIN_MS,
                );
            }
            for incident in &engine.incidents {
                assert!(!incident.evidence.is_empty(), "{}", trace.name);
                assert!(
                    incident.evidence.iter().all(|observation| {
                        observation.subject.as_ref().is_some_and(|subject| {
                            subject.channel_id == incident.channel_id
                                && subject.call_id == incident.call_id
                        })
                    }),
                    "{}",
                    trace.name
                );
            }
        }
    }

    fn snapshot(ids: &[&str]) -> Command {
        Command::Snapshot {
            channel: Some(Channel {
                id: "100".into(),
                name: "Call".into(),
                kind: None,
            }),
            users: ids
                .iter()
                .map(|id| User {
                    id: (*id).into(),
                    username: "User".into(),
                    display_name: "User".into(),
                    avatar_url: None,
                })
                .collect(),
        }
    }

    #[test]
    fn baseline_duplicate_and_exact_ids() {
        let mut history = History::default();
        let mut engine = Intelligence::new("session".into());
        history.apply(snapshot(&["9007199254740993"]), 100).unwrap();
        engine.observe(&history, 0, 100, ObservationSource::InitialSnapshot);
        assert_eq!(engine.observations.len(), 1);
        assert_eq!(engine.observations[0].kind, ObservationKind::Baseline);
        history.apply(snapshot(&[]), 200).unwrap();
        engine.observe(&history, 1, 200, ObservationSource::StoreNotification);
        engine.observe(&history, 1, 200, ObservationSource::Reconciliation);
        assert_eq!(engine.observations.len(), 2);
        assert_eq!(
            engine.observations[1].subject.as_ref().unwrap().user_id,
            "9007199254740993"
        );
        assert!(engine.incidents.is_empty());
    }

    #[test]
    fn burst_is_advisory_bounded_and_baseline_safe() {
        let mut history = History::default();
        let mut engine = Intelligence::new("session".into());
        history.apply(snapshot(&[]), 0).unwrap();
        engine.observe(&history, 0, 0, ObservationSource::InitialSnapshot);
        history
            .apply(snapshot(&["1", "2", "3", "4", "5"]), 1)
            .unwrap();
        engine.observe(&history, 1, 1, ObservationSource::Reconciliation);
        assert_eq!(engine.incidents.len(), 1);
        assert_eq!(engine.incidents[0].confidence, "uncalibrated");
        assert_eq!(engine.incidents[0].evidence.len(), 5);
        for time in 2..700 {
            history
                .apply(snapshot(if time % 2 == 0 { &[] } else { &["1"] }), time)
                .unwrap();
            engine.observe(&history, time, time, ObservationSource::Reconciliation);
        }
        assert_eq!(engine.observations.len(), OBSERVATION_LIMIT);
        assert!(engine.dropped_observations > 0);
        assert!(
            engine
                .incidents
                .iter()
                .any(|incident| incident.rule == "rapid_reentry")
        );
    }

    #[test]
    fn gaps_and_clock_rollback_reset_detection() {
        let mut history = History::default();
        let mut engine = Intelligence::new("session".into());
        history.apply(snapshot(&[]), 0).unwrap();
        engine.observe(&history, 100, 0, ObservationSource::InitialSnapshot);
        engine.observe(&history, 50, 0, ObservationSource::Reconciliation);
        engine.observe(
            &history,
            6_000_000_000,
            0,
            ObservationSource::Reconciliation,
        );
        assert_eq!(engine.gap_count, 2);
        assert!(engine.incidents.is_empty());
        for time in 1..600 {
            history
                .apply(snapshot(if time % 2 == 0 { &[] } else { &["1"] }), time)
                .unwrap();
        }
        engine.observe(&history, 6_000_000_001, 0, ObservationSource::Unknown);
        assert_eq!(engine.missing_transitions, 99);
    }
}
