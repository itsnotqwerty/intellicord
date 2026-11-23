use intellicord::acoustic::{
    ClockError, Format, FrameError, PcmEncoding, PcmPlane, SampleClockMapping, SourceClockMapping,
    TimedFrameError, TimedRepetitionFrameAssembler,
};

fn mono(bytes: &[u8], stride_bytes: usize) -> [PcmPlane<'_>; 1] {
    [PcmPlane {
        bytes,
        offset_bytes: 0,
        stride_bytes,
    }]
}

#[test]
fn conversion_errors_precede_clock_and_position_errors_without_consuming_pcm() {
    for planar in [false, true] {
        let mut frames = assembler(8_000, 1, 20_000_000);
        frames.push(0, &[0.5; 10]).unwrap();
        let invalid: Vec<_> = [0.5_f32, f32::NAN]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        for position in [0, u64::MAX] {
            let result = if planar {
                frames.push_planes(position, PcmEncoding::Float32Le, 2, &mono(&invalid, 4))
            } else {
                frames.push_bytes(position, PcmEncoding::Float32Le, &invalid)
            };
            assert!(matches!(
                result,
                Err(TimedFrameError::Frame(FrameError::InvalidSample))
            ));
            let result = if planar {
                frames.push_planes(position, PcmEncoding::Float32Le, usize::MAX, &[])
            } else {
                frames.push_bytes(position, PcmEncoding::Float32Le, &[0])
            };
            assert!(matches!(
                result,
                Err(TimedFrameError::Frame(FrameError::InvalidLength))
            ));
        }
        let batch = frames.push(10, &[0.5; 150]).unwrap();
        assert_eq!(batch.features[0].start_sample_per_channel, 0);
        assert_eq!(
            batch.features[0]
                .nominal_sample_clock_features
                .spectral
                .levels
                .rms,
            0.5
        );
    }
}

#[test]
fn decoded_clock_expiry_allows_valid_retry_for_both_layouts() {
    for planar in [false, true] {
        let mut frames = assembler(8_000, 1, 20_000_000);
        frames.push(0, &[0.0; 10]).unwrap();
        let bytes = vec![0; 302];
        let result = if planar {
            frames.push_planes(10, PcmEncoding::Signed16Le, 151, &mono(&bytes, 2))
        } else {
            frames.push_bytes(10, PcmEncoding::Signed16Le, &bytes)
        };
        assert!(matches!(
            result,
            Err(TimedFrameError::Clock(ClockError::Expired))
        ));
        let result = if planar {
            frames.push_planes(10, PcmEncoding::Signed16Le, 150, &mono(&bytes[..300], 2))
        } else {
            frames.push_bytes(10, PcmEncoding::Signed16Le, &bytes[..300])
        };
        let batch = result.unwrap();
        assert_eq!(batch.features.len(), 1);
        assert_eq!(batch.features[0].start_sample_per_channel, 0);
        assert_eq!(batch.features[0].end_sample_per_channel, 160);
        assert_eq!(batch.buffered_samples_per_channel, 0);
    }
}

#[test]
fn invalid_encoded_tails_preserve_repetition_history_until_retry() {
    for planar in [false, true] {
        let mut frames = assembler(8_000, 1, 1_000_000_000);
        let tone: Vec<f32> = (0..160)
            .map(|index| if index % 8 < 4 { 0.5 } else { -0.5 })
            .collect();
        for index in 0..9 {
            frames.push(index * 160, &tone).unwrap();
        }
        let mut bytes: Vec<_> = tone
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        bytes.extend_from_slice(&f32::NAN.to_le_bytes());
        let result = if planar {
            frames.push_planes(1440, PcmEncoding::Float32Le, 161, &mono(&bytes, 4))
        } else {
            frames.push_bytes(1440, PcmEncoding::Float32Le, &bytes)
        };
        assert!(matches!(
            result,
            Err(TimedFrameError::Frame(FrameError::InvalidSample))
        ));
        let result = if planar {
            frames.push_planes(1440, PcmEncoding::Float32Le, 160, &mono(&bytes[..640], 4))
        } else {
            frames.push_bytes(1440, PcmEncoding::Float32Le, &bytes[..640])
        };
        let batch = result.unwrap();
        assert!(
            batch.features[0]
                .nominal_sample_clock_features
                .best_similarity
                .is_some()
        );
        assert_eq!(batch.features[0].start_sample_per_channel, 1440);
    }
}

#[test]
fn mixed_inputs_and_shared_views_ignore_padding_and_enforce_four_frame_limit() {
    let mut frames = assembler(96_000, 8, 1_000_000_000);
    let mut direct = assembler(96_000, 8, 1_000_000_000);
    frames.push(0, &[0.0; 80]).unwrap();
    direct.push(0, &[0.0; 80]).unwrap();
    let count = 1_920 * 4;
    let oversized = vec![0; (count + 1) * 8 * 2];
    assert!(matches!(
        frames.push_bytes(10, PcmEncoding::Signed16Le, &oversized),
        Err(TimedFrameError::Frame(FrameError::InvalidLength))
    ));
    let oversized_views: Vec<_> = (0..8)
        .map(|channel| PcmPlane {
            bytes: &oversized,
            offset_bytes: channel * 2,
            stride_bytes: 16,
        })
        .collect();
    assert!(matches!(
        frames.push_planes(10, PcmEncoding::Signed16Le, count + 1, &oversized_views),
        Err(TimedFrameError::Frame(FrameError::InvalidLength))
    ));
    let mut bytes = vec![0xff; 3];
    let mut samples = Vec::new();
    for _ in 0..count {
        for channel in 0..8 {
            let sample: f32 = if channel % 2 == 0 { 0.5 } else { -0.25 };
            bytes.extend_from_slice(&sample.to_le_bytes());
            samples.push(sample);
        }
        bytes.extend_from_slice(&f32::NAN.to_le_bytes());
    }
    let views: Vec<_> = (0..8)
        .map(|channel| PcmPlane {
            bytes: &bytes,
            offset_bytes: 3 + channel * 4,
            stride_bytes: 36,
        })
        .collect();
    let batch = frames
        .push_planes(10, PcmEncoding::Float32Le, count, &views)
        .unwrap();
    let expected = direct.push(10, &samples).unwrap();
    assert_eq!(batch.features.len(), 4);
    assert_eq!(batch.buffered_samples_per_channel, 10);
    assert_eq!(
        serde_json::to_value(batch).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let tail = vec![0; 1_910 * 8 * 2];
    assert_eq!(
        serde_json::to_value(
            frames
                .push_bytes(7_690, PcmEncoding::Signed16Le, &tail)
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(direct.push(7_690, &vec![0.0; 1_910 * 8]).unwrap()).unwrap()
    );
}

fn assembler(rate: u32, channels: usize, horizon: u64) -> TimedRepetitionFrameAssembler {
    let source = SourceClockMapping::new(0, 100, 120, horizon, 100).unwrap();
    let mapping = SampleClockMapping::new(source, Format::new(rate, channels).unwrap(), 0).unwrap();
    TimedRepetitionFrameAssembler::new(mapping)
}

fn encode(sample: i16, encoding: PcmEncoding) -> Vec<u8> {
    match encoding {
        PcmEncoding::Signed16Le => sample.to_le_bytes().to_vec(),
        PcmEncoding::Signed24Le => (i32::from(sample) << 8).to_le_bytes()[..3].to_vec(),
        PcmEncoding::Signed32Le => (i32::from(sample) << 16).to_le_bytes().to_vec(),
        PcmEncoding::Float32Le => (f32::from(sample) / 32_768.0).to_le_bytes().to_vec(),
    }
}

#[test]
fn packed_and_padded_planes_match_normalized_timing_across_encodings_and_formats() {
    for (rate, channels) in [(8_050, 1), (44_100, 2), (96_000, 8)] {
        let frame = rate as usize / 50;
        for encoding in [
            PcmEncoding::Signed16Le,
            PcmEncoding::Signed24Le,
            PcmEncoding::Signed32Le,
            PcmEncoding::Float32Le,
        ] {
            let mut packed = assembler(rate, channels, 1_000_000_000);
            let mut planar = assembler(rate, channels, 1_000_000_000);
            let mut direct = assembler(rate, channels, 1_000_000_000);
            let width = encode(0, encoding).len();
            for (position, count) in [
                (0, 10),
                (15, frame * 4),
                (15 + frame * 4, frame * 4),
                (15 + frame * 8, frame * 2),
            ] {
                let mut bytes = Vec::new();
                let mut buffers = vec![vec![0xff]; channels];
                let mut samples = Vec::new();
                for index in 0..count {
                    for (channel, buffer) in buffers.iter_mut().enumerate() {
                        let sample = if (index + channel) % 8 < 4 {
                            16_384
                        } else {
                            -8_192
                        };
                        let encoded = encode(sample, encoding);
                        bytes.extend_from_slice(&encoded);
                        buffer.extend_from_slice(&encoded);
                        buffer.extend_from_slice(&[0xff; 2]);
                        samples.push(f32::from(sample) / 32_768.0);
                    }
                }
                let planes: Vec<_> = buffers
                    .iter()
                    .map(|bytes| PcmPlane {
                        bytes,
                        offset_bytes: 1,
                        stride_bytes: width + 2,
                    })
                    .collect();
                let expected =
                    serde_json::to_value(direct.push(position as u64, &samples).unwrap()).unwrap();
                assert_eq!(
                    serde_json::to_value(
                        packed
                            .push_bytes(position as u64, encoding, &bytes)
                            .unwrap()
                    )
                    .unwrap(),
                    expected
                );
                assert_eq!(
                    serde_json::to_value(
                        planar
                            .push_planes(position as u64, encoding, count, &planes)
                            .unwrap()
                    )
                    .unwrap(),
                    expected
                );
            }
        }
    }
}
