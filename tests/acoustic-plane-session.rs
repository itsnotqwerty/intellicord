use intellicord::acoustic::{
    BufferedAcousticSession, Format, FrameError, OUTPUT_QUEUE_CAPACITY, PcmEncoding, PcmPlane,
    ProducerFailure, SessionError, SessionFault, SessionStatus,
};

fn plane(bytes: &[u8], stride_bytes: usize) -> [PcmPlane<'_>; 1] {
    [PcmPlane {
        bytes,
        offset_bytes: 0,
        stride_bytes,
    }]
}

#[test]
fn stale_and_foreign_malformed_views_preserve_full_current_queue() {
    let format = Format::new(8_000, 1).unwrap();
    let mut session = BufferedAcousticSession::default();
    let old = session.start(format, 0).unwrap();
    session.stop();
    let before = session.health();
    assert_eq!(
        session.push_planes(&old, u64::MAX, PcmEncoding::Float32Le, usize::MAX, &[]),
        Err(SessionError::Inactive)
    );
    assert_eq!(session.health(), before);
    let current = session
        .start_encoded(format, 0, PcmEncoding::Signed16Le)
        .unwrap();
    let mut other = BufferedAcousticSession::default();
    let foreign = other.start(format, 0).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        session
            .push_planes(
                &current,
                index as u64,
                PcmEncoding::Signed16Le,
                1,
                &plane(&[0; 2], 2),
            )
            .unwrap();
    }
    let before = session.health();
    for permit in [&old, &foreign] {
        assert_eq!(
            session.push_planes(permit, u64::MAX, PcmEncoding::Float32Le, usize::MAX, &[]),
            Err(SessionError::StalePermit)
        );
        assert_eq!(session.health(), before);
    }
    assert_eq!(
        session.push_planes(&current, u64::MAX, PcmEncoding::Float32Le, usize::MAX, &[]),
        Err(SessionError::FormatChanged)
    );
    let after = session.health();
    assert_eq!(
        after.status,
        SessionStatus::Faulted {
            error: SessionFault::Producer(ProducerFailure::FormatChanged)
        }
    );
    assert_eq!(after.processed, before.processed);
    assert_eq!(after.discarded, before.pending);
    assert_eq!(after.pending.batches, 0);
    assert_eq!(after.overflow_events, 0);
}

#[test]
fn conversion_fault_precedes_time_and_capacity_and_preserves_first_fault() {
    for invalid_sample in [false, true] {
        let mut session = BufferedAcousticSession::default();
        let permit = session
            .start_encoded(Format::new(8_000, 1).unwrap(), 0, PcmEncoding::Float32Le)
            .unwrap();
        for index in 0..OUTPUT_QUEUE_CAPACITY {
            session
                .push(&permit, index as u64 * 160, &[0.5; 160])
                .unwrap();
        }
        let before = session.health();
        let bytes: Vec<_> = [0.5_f32, f32::NAN]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let views = plane(&bytes, if invalid_sample { 4 } else { usize::MAX });
        let error = if invalid_sample {
            FrameError::InvalidSample
        } else {
            FrameError::InvalidLength
        };
        assert_eq!(
            session.push_planes(&permit, 0, PcmEncoding::Float32Le, 2, &views),
            Err(SessionError::InvalidFrame(error))
        );
        let after = session.health();
        assert_eq!(
            after.status,
            SessionStatus::Faulted {
                error: if invalid_sample {
                    SessionFault::InvalidSample
                } else {
                    SessionFault::InvalidLength
                }
            }
        );
        assert_eq!(after.processed, before.processed);
        assert_eq!(after.discarded, before.pending);
        assert_eq!(after.pending.batches, 0);
        assert_eq!(after.overflow_events, 0);
        assert!(session.pop().unwrap().is_none());
        assert_eq!(
            session.push_planes(&permit, 0, PcmEncoding::Signed16Le, 0, &[]),
            Err(SessionError::Inactive)
        );
        assert_eq!(session.health(), after);
    }
}

#[test]
fn valid_planes_use_shared_overflow_accounting() {
    let mut session = BufferedAcousticSession::default();
    let permit = session.start(Format::new(8_000, 1).unwrap(), 0).unwrap();
    for index in 0..OUTPUT_QUEUE_CAPACITY {
        session
            .push_planes(
                &permit,
                index as u64 * 640,
                PcmEncoding::Signed16Le,
                640,
                &plane(&[0; 1280], 2),
            )
            .unwrap();
    }
    assert_eq!(
        session.push_planes(
            &permit,
            OUTPUT_QUEUE_CAPACITY as u64 * 640,
            PcmEncoding::Signed16Le,
            640,
            &plane(&[0; 1280], 2)
        ),
        Err(SessionError::OutputOverflow)
    );
    let health = session.health();
    assert_eq!(
        health.status,
        SessionStatus::Faulted {
            error: SessionFault::OutputOverflow
        }
    );
    assert_eq!(health.processed.batches, 9);
    assert_eq!(health.processed.frames, 36);
    assert_eq!(health.discarded, health.processed);
    assert_eq!(health.pending.batches, 0);
    assert_eq!(health.overflow_events, 1);
}

#[test]
fn mixed_input_checks_time_and_restart_uses_new_format_and_encoding() {
    let mut session = BufferedAcousticSession::default();
    let old = session
        .start_encoded(Format::new(8_000, 1).unwrap(), 0, PcmEncoding::Signed16Le)
        .unwrap();
    session
        .push_planes(&old, 0, PcmEncoding::Signed16Le, 170, &plane(&[0; 340], 2))
        .unwrap();
    assert_eq!(
        session.push_bytes(&old, 0, PcmEncoding::Signed16Le, &[0; 2]),
        Err(SessionError::InvalidFrame(FrameError::InvalidTime))
    );
    let current = session
        .start_encoded(Format::new(8_000, 2).unwrap(), 77, PcmEncoding::Float32Le)
        .unwrap();
    let bytes: Vec<_> = [0.5_f32, -0.25]
        .into_iter()
        .cycle()
        .take(320)
        .flat_map(f32::to_le_bytes)
        .collect();
    let views = [
        PcmPlane {
            bytes: &bytes,
            offset_bytes: 0,
            stride_bytes: 8,
        },
        PcmPlane {
            bytes: &bytes,
            offset_bytes: 4,
            stride_bytes: 8,
        },
    ];
    assert_eq!(
        session.push_planes(&old, 0, PcmEncoding::Signed16Le, 0, &[]),
        Err(SessionError::StalePermit)
    );
    session
        .push_planes(&current, 0, PcmEncoding::Float32Le, 10, &views)
        .unwrap();
    session
        .push_bytes(&current, 10, PcmEncoding::Float32Le, &bytes[80..])
        .unwrap();
    assert_eq!(
        session.pop().unwrap().unwrap().buffered_samples_per_channel,
        10
    );
    let batch = session.pop().unwrap().unwrap();
    let feature = &batch.features[0];
    assert_eq!(feature.spectral.levels.frame_start_ns, 77);
    assert_eq!(feature.spectral.levels.channels, 2);
    assert_eq!(feature.spectral.levels.channel_rms, [0.5, 0.25]);
    assert!(feature.spectral.levels.rms_change.is_none());
    assert!(feature.best_similarity.is_none());
    session.stop();
    assert_eq!(
        session.push_planes(&current, 160, PcmEncoding::Float32Le, 160, &views),
        Err(SessionError::Inactive)
    );
}

#[test]
fn planes_match_normalized_input_through_partial_gap_and_repetition_warmup() {
    for (rate, channels) in [(8_050, 1), (44_100, 2), (96_000, 8)] {
        let format = Format::new(rate, channels).unwrap();
        let frame = rate as usize / 50;
        for encoding in [
            PcmEncoding::Signed16Le,
            PcmEncoding::Signed24Le,
            PcmEncoding::Signed32Le,
            PcmEncoding::Float32Le,
        ] {
            let mut session = BufferedAcousticSession::default();
            let mut direct = BufferedAcousticSession::default();
            let permit = session.start_encoded(format, 123, encoding).unwrap();
            let reference = direct.start(format, 123).unwrap();
            for (position, count) in [
                (0, 10),
                (15, frame * 4),
                (15 + frame * 4, frame * 4),
                (15 + frame * 8, frame * 2),
            ] {
                let mut buffers = vec![vec![0xff]; channels];
                let mut normalized = Vec::new();
                for index in 0..count {
                    for (channel, buffer) in buffers.iter_mut().enumerate() {
                        let sample: i16 = if (index + channel) % 4 < 2 {
                            16_384
                        } else {
                            -16_384
                        };
                        normalized.push(f32::from(sample) / 32_768.0);
                        match encoding {
                            PcmEncoding::Signed16Le => {
                                buffer.extend_from_slice(&sample.to_le_bytes())
                            }
                            PcmEncoding::Signed24Le => buffer
                                .extend_from_slice(&(i32::from(sample) << 8).to_le_bytes()[..3]),
                            PcmEncoding::Signed32Le => {
                                buffer.extend_from_slice(&(i32::from(sample) << 16).to_le_bytes())
                            }
                            PcmEncoding::Float32Le => buffer
                                .extend_from_slice(&(f32::from(sample) / 32_768.0).to_le_bytes()),
                        }
                        buffer.push(0xff);
                    }
                }
                let width = match encoding {
                    PcmEncoding::Signed16Le => 2,
                    PcmEncoding::Signed24Le => 3,
                    _ => 4,
                };
                let planes: Vec<_> = buffers
                    .iter()
                    .map(|bytes| PcmPlane {
                        bytes,
                        offset_bytes: 1,
                        stride_bytes: width + 1,
                    })
                    .collect();
                session
                    .push_planes(&permit, position as u64, encoding, count, &planes)
                    .unwrap();
                direct
                    .push(&reference, position as u64, &normalized)
                    .unwrap();
                assert_eq!(session.health(), direct.health());
                assert_eq!(
                    serde_json::to_value(session.pop().unwrap().unwrap()).unwrap(),
                    serde_json::to_value(direct.pop().unwrap().unwrap()).unwrap()
                );
                assert_eq!(session.health(), direct.health());
            }
            assert_eq!(session.health().processed.frames, 10);
        }
    }
}
