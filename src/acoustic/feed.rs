use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::ingestion::{
    AcousticFrame, AcousticHealth, AcousticSessionInterval, AcousticSource, AcousticState,
    Producer, SensorMessage, SensorPayload, SensorPublisher,
};

pub struct AcousticFeed {
    publisher: SensorPublisher,
    session_id: String,
    started: Instant,
    source: AcousticSource,
    sequence: u64,
    frame_earliest_ns: Option<u64>,
    frame_anchor: Option<(u64, u64)>,
}

impl AcousticFeed {
    pub fn new(
        publisher: SensorPublisher,
        session_id: String,
        started: Instant,
        source: AcousticSource,
    ) -> Self {
        Self {
            publisher,
            session_id,
            started,
            source,
            sequence: 0,
            frame_earliest_ns: None,
            frame_anchor: None,
        }
    }

    pub(crate) fn elapsed_ns(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    pub(crate) fn prepare_frame_clock(&mut self, earliest_ns: u64) -> Result<(), &'static str> {
        if self.frame_earliest_ns.is_some() || self.frame_anchor.is_some() {
            return Err("invalid acoustic frame clock anchor");
        }
        self.frame_earliest_ns = Some(earliest_ns);
        Ok(())
    }

    pub fn frame(&mut self, features: &super::RepetitionFeatures) -> Result<(), &'static str> {
        let mut frame = AcousticFrame::from_repetition_features(features, self.source)
            .ok_or("invalid acoustic features")?;
        if let Some(earliest_ns) = self.frame_earliest_ns.take() {
            let latest_ns = self.elapsed_ns();
            if earliest_ns > latest_ns {
                return Err("invalid acoustic frame clock anchor");
            }
            self.frame_anchor = Some((earliest_ns, latest_ns));
        }
        if let Some((earliest_ns, latest_ns)) = self.frame_anchor {
            frame.session_start_interval_ns = Some(AcousticSessionInterval {
                earliest_ns: earliest_ns
                    .checked_add(frame.frame_start_ns)
                    .ok_or("acoustic frame clock overflow")?,
                latest_ns: latest_ns
                    .checked_add(frame.frame_start_ns)
                    .ok_or("acoustic frame clock overflow")?,
            });
        }
        self.publish(SensorPayload::Acoustic(frame));
        Ok(())
    }

    pub fn state(&mut self, state: AcousticState) {
        self.publish(SensorPayload::AcousticState(self.source, state));
    }

    pub fn health(&mut self, health: super::OutputHealth) {
        self.publish(SensorPayload::AcousticHealth(AcousticHealth {
            source: self.source,
            health,
        }));
    }

    fn publish(&mut self, payload: SensorPayload) {
        self.sequence = self.sequence.saturating_add(1);
        let _ = self.publisher.publish(SensorMessage {
            schema_version: 1,
            session_id: self.session_id.clone(),
            producer: Producer::Acoustic,
            producer_sequence: self.sequence,
            monotonic_ns: self.elapsed_ns(),
            wall_time_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |time| {
                    u64::try_from(time.as_millis()).unwrap_or(u64::MAX)
                }),
            payload,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        acoustic::{Format, RepetitionAnalyzer},
        ingestion::{SensorBus, SensorPayload},
    };

    fn features(frame_start_ns: u64) -> super::super::RepetitionFeatures {
        let format = Format::new(48_000, 2).unwrap();
        RepetitionAnalyzer::default()
            .process(format, frame_start_ns, &vec![0.0; format.sample_count()])
            .unwrap()
    }

    #[test]
    fn frame_anchor_maps_relative_time_to_a_checked_session_interval() {
        let bus = SensorBus::default();
        let mut feed = AcousticFeed::new(
            bus.publisher(),
            "session".into(),
            Instant::now(),
            AcousticSource::MixedOutput,
        );
        feed.prepare_frame_clock(0).unwrap();
        feed.frame(&features(20_000_000)).unwrap();
        let (messages, _) = bus.drain();
        let SensorPayload::Acoustic(frame) = &messages[0].payload else {
            panic!("expected acoustic frame");
        };
        assert_eq!(frame.frame_start_ns, 20_000_000);
        assert_eq!(
            frame.session_start_interval_ns,
            Some(AcousticSessionInterval {
                earliest_ns: 20_000_000,
                latest_ns: frame.session_start_interval_ns.as_ref().unwrap().latest_ns,
            })
        );
        assert!(frame.session_start_interval_ns.as_ref().unwrap().latest_ns >= 20_000_000);
        assert!(feed.prepare_frame_clock(300).is_err());

        let mut overflow = AcousticFeed::new(
            bus.publisher(),
            "session".into(),
            Instant::now(),
            AcousticSource::MixedOutput,
        );
        overflow.frame_anchor = Some((u64::MAX, u64::MAX));
        assert_eq!(
            overflow.frame(&features(20_000_000)),
            Err("acoustic frame clock overflow")
        );
    }
}
