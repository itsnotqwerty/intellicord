use intellicord::{
    acoustic::{Analyzer, FRAME_DURATION_NS, Format, RepetitionAnalyzer},
    ingestion::{
        AcousticFrame, AcousticSource, Producer, PublishError, SensorBus, SensorMessage,
        SensorPayload,
    },
};

#[test]
fn owner_completion_survives_a_full_feature_bus() {
    use intellicord::{acoustic::AcousticFeed, detection::Intelligence, ingestion::AcousticState};
    let bus = SensorBus::default();
    let mut feed = AcousticFeed::new(
        bus.publisher(),
        "session".into(),
        std::time::Instant::now(),
        AcousticSource::GeneratedPcm,
    );
    for _ in 0..intellicord::ingestion::BUS_CAPACITY {
        feed.state(AcousticState::Starting);
    }
    feed.state(AcousticState::Stopped);
    let mut engine = Intelligence::new("session".into());
    engine.drain(&bus);
    assert_eq!(engine.ingestion.queue.dropped_full, 1);
    assert_eq!(
        engine.acoustic.as_ref().unwrap().status,
        AcousticState::Starting
    );
    engine.finish_acoustic(AcousticSource::GeneratedPcm, true, 1_000_000_000);
    assert_eq!(
        engine.acoustic.as_ref().unwrap().status,
        AcousticState::Stopped
    );
    assert_eq!(engine.acoustic.as_ref().unwrap().frames, 0);
}

#[test]
fn processing_health_requires_valid_accounting_and_an_existing_acoustic_state() {
    use intellicord::{
        acoustic::{OutputCounts, OutputHealth, SessionStatus},
        detection::Intelligence,
        ingestion::{AcousticHealth, AcousticState},
    };

    let bus = SensorBus::default();
    let publisher = bus.publisher();
    let health = OutputHealth {
        schema_version: 1,
        status: SessionStatus::Active,
        capacity_batches: 8,
        pending: OutputCounts::default(),
        processed: OutputCounts {
            batches: 2,
            frames: 2,
        },
        delivered: OutputCounts {
            batches: 2,
            frames: 2,
        },
        discarded: OutputCounts::default(),
        overflow_events: 0,
        high_water_batches: 1,
    };
    let message = |sequence, payload| SensorMessage {
        schema_version: 1,
        session_id: "session".into(),
        producer: Producer::Acoustic,
        producer_sequence: sequence,
        monotonic_ns: sequence * 100,
        wall_time_unix_ms: 20,
        payload,
    };
    publisher
        .publish(message(
            1,
            SensorPayload::AcousticHealth(AcousticHealth {
                source: AcousticSource::MixedOutput,
                health,
            }),
        ))
        .unwrap();
    let mut engine = Intelligence::new("session".into());
    engine.drain(&bus);
    assert_eq!(engine.ingestion.rejected_state, 1);
    assert!(engine.acoustic.is_none());

    publisher
        .publish(message(
            2,
            SensorPayload::AcousticState(AcousticSource::MixedOutput, AcousticState::Starting),
        ))
        .unwrap();
    publisher
        .publish(message(
            3,
            SensorPayload::AcousticHealth(AcousticHealth {
                source: AcousticSource::MixedOutput,
                health,
            }),
        ))
        .unwrap();
    publisher
        .publish(message(
            4,
            SensorPayload::AcousticHealth(AcousticHealth {
                source: AcousticSource::GeneratedPcm,
                health,
            }),
        ))
        .unwrap();
    publisher
        .publish(message(
            5,
            SensorPayload::AcousticHealth(AcousticHealth {
                source: AcousticSource::MixedOutput,
                health: OutputHealth {
                    status: SessionStatus::Stopped,
                    ..health
                },
            }),
        ))
        .unwrap();
    let mut invalid = health;
    invalid.processed.frames = 3;
    assert_eq!(
        publisher.publish(message(
            6,
            SensorPayload::AcousticHealth(AcousticHealth {
                source: AcousticSource::MixedOutput,
                health: invalid,
            }),
        )),
        Err(PublishError::Invalid)
    );
    engine.drain(&bus);
    let summary = engine.acoustic.as_ref().unwrap();
    assert_eq!(summary.schema_version, 4);
    assert_eq!(summary.processing_health, Some(health));
    assert_eq!(engine.ingestion.rejected_state, 3);
}

#[test]
fn shared_session_accepts_features_without_voice_incidents_and_rejects_revival() {
    use intellicord::{detection::Intelligence, ingestion::AcousticState};
    let bus = SensorBus::default();
    let publisher = bus.publisher();
    let mut engine = Intelligence::new("session".into());
    let frame = AcousticFrame {
        source: AcousticSource::GeneratedPcm,
        frame_start_ns: 0,
        session_start_interval_ns: None,
        rms: 0.5,
        peak: 0.5,
        clipped_sample_ratio: 0.0,
        discontinuity: false,
        analysis: None,
    };
    let message = |sequence, payload| SensorMessage {
        schema_version: 1,
        session_id: "session".into(),
        producer: Producer::Acoustic,
        producer_sequence: sequence,
        monotonic_ns: sequence * 100,
        wall_time_unix_ms: 20,
        payload,
    };
    publisher
        .publish(message(
            1,
            SensorPayload::AcousticState(frame.source, AcousticState::Starting),
        ))
        .unwrap();
    publisher
        .publish(message(3, SensorPayload::Acoustic(frame.clone())))
        .unwrap();
    engine.drain(&bus);
    assert_eq!(engine.acoustic.as_ref().unwrap().frames, 1);
    assert_eq!(engine.acoustic.as_ref().unwrap().missing_messages, 1);
    publisher
        .publish(message(4, SensorPayload::Acoustic(frame.clone())))
        .unwrap();
    engine.drain(&bus);
    assert_eq!(engine.ingestion.rejected_state, 1);
    publisher
        .publish(message(
            4,
            SensorPayload::AcousticState(frame.source, AcousticState::Stopped),
        ))
        .unwrap();
    publisher
        .publish(message(
            5,
            SensorPayload::Acoustic(AcousticFrame {
                frame_start_ns: 20_000_000,
                ..frame
            }),
        ))
        .unwrap();
    engine.drain(&bus);
    assert_eq!(engine.ingestion.rejected_state, 2);
    assert_eq!(
        engine.acoustic.as_ref().unwrap().status,
        AcousticState::Stopped
    );
    assert!(engine.observations.is_empty());
    assert!(engine.incidents.is_empty());
    engine.reset_review();
    assert_eq!(engine.acoustic.as_ref().unwrap().frames, 1);
    let summary = serde_json::to_value(&engine).unwrap();
    assert_eq!(summary["acoustic"]["source"], "generated_pcm");
    assert_eq!(summary["acoustic"]["schema_version"], 2);
    assert_eq!(summary["acoustic"].as_object().unwrap().len(), 8);
}

#[test]
fn feature_only_lane_rejects_nonfinite_values_and_wrong_producers() {
    let format = Format::new(48_000, 2).unwrap();
    let features = Analyzer::default()
        .process(format, 0, &vec![0.5; format.sample_count()])
        .unwrap();
    let frame = AcousticFrame::from_features(&features, AcousticSource::GeneratedPcm).unwrap();
    let bus = SensorBus::default();
    let publisher = bus.publisher();
    let message = SensorMessage {
        schema_version: 1,
        session_id: "session".into(),
        producer: Producer::Acoustic,
        producer_sequence: 1,
        monotonic_ns: 10,
        wall_time_unix_ms: 20,
        payload: SensorPayload::Acoustic(frame.clone()),
    };
    assert_eq!(publisher.publish(message.clone()), Ok(()));
    assert_eq!(
        publisher.publish(SensorMessage {
            producer: Producer::Application,
            ..message.clone()
        }),
        Err(PublishError::Invalid)
    );
    for rms in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert_eq!(
            publisher.publish(SensorMessage {
                payload: SensorPayload::Acoustic(AcousticFrame {
                    rms,
                    ..frame.clone()
                }),
                ..message.clone()
            }),
            Err(PublishError::Invalid)
        );
    }
    let (messages, metrics) = bus.drain();
    assert_eq!(messages.len(), 1);
    assert_eq!(metrics.rejected_invalid, 5);
    let json = serde_json::to_value(frame).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 7);
    assert_eq!(json["source"], "generated_pcm");
}

#[test]
fn repetition_features_reduce_to_latest_bounded_analysis_without_descriptors() {
    let format = Format::new(48_000, 2).unwrap();
    let frame = (0..960)
        .flat_map(|index| {
            let sample =
                (std::f64::consts::TAU * 1_000.0 * index as f64 / 48_000.0).sin() as f32 * 0.5;
            [sample, sample]
        })
        .collect::<Vec<_>>();
    let mut analyzer = RepetitionAnalyzer::default();
    let mut features = None;
    for index in 0..10 {
        features = Some(
            analyzer
                .process(format, index * FRAME_DURATION_NS, &frame)
                .unwrap(),
        );
    }
    let reduced = AcousticFrame::from_repetition_features(
        features.as_ref().unwrap(),
        AcousticSource::GeneratedPcm,
    )
    .unwrap();
    let analysis = reduced.analysis.as_ref().unwrap();
    assert!((analysis.centroid_hz.unwrap() - 1_000.0).abs() < 0.1);
    assert_eq!(analysis.compared_lags, 1);
    assert_eq!(analysis.best_lag_ns, Some(100_000_000));
    assert!(analysis.best_similarity.unwrap() > 0.999);
    let json = serde_json::to_value(&reduced).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 7);
    assert_eq!(json["analysis"].as_object().unwrap().len(), 6);
    assert!(json.to_string().len() < 512);
    for omitted in ["descriptor", "normalized_power", "bins", "spectrum"] {
        assert!(!json.to_string().contains(omitted));
    }

    let bus = SensorBus::default();
    let mut invalid = reduced.clone();
    invalid.analysis.as_mut().unwrap().best_similarity = Some(f64::NAN);
    assert_eq!(
        bus.publisher().publish(SensorMessage {
            schema_version: 1,
            session_id: "session".into(),
            producer: Producer::Acoustic,
            producer_sequence: 1,
            monotonic_ns: 1,
            wall_time_unix_ms: 1,
            payload: SensorPayload::Acoustic(invalid),
        }),
        Err(PublishError::Invalid)
    );
}
