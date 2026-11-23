use intellicord::acoustic::{
    Format, FrameError, MAX_CHUNK_FRAMES, PcmEncoding, PcmPlane, RepetitionFrameAssembler,
    decode_pcm_chunk, decode_pcm_planes,
};

fn encode(sample: i16, encoding: PcmEncoding) -> Vec<u8> {
    match encoding {
        PcmEncoding::Signed16Le => sample.to_le_bytes().to_vec(),
        PcmEncoding::Signed24Le => (i32::from(sample) << 8).to_le_bytes()[..3].to_vec(),
        PcmEncoding::Signed32Le => (i32::from(sample) << 16).to_le_bytes().to_vec(),
        PcmEncoding::Float32Le => (f32::from(sample) / 32_768.0).to_le_bytes().to_vec(),
    }
}

#[test]
fn all_encodings_match_packed_decoding_and_pipeline_across_layouts() {
    for (rate, channels) in [(8_050, 1), (44_100, 2), (96_000, 8)] {
        let format = Format::new(rate, channels).unwrap();
        let count = rate as usize / 50;
        for encoding in [
            PcmEncoding::Signed16Le,
            PcmEncoding::Signed24Le,
            PcmEncoding::Signed32Le,
            PcmEncoding::Float32Le,
        ] {
            let width = encode(0, encoding).len();
            let mut packed = Vec::new();
            let mut planar = vec![vec![0xff; 3]; channels];
            for index in 0..count {
                for (channel, plane) in planar.iter_mut().enumerate() {
                    let value = if (index + channel) % 2 == 0 {
                        16_384
                    } else {
                        -8_192
                    };
                    let encoded = encode(value, encoding);
                    packed.extend_from_slice(&encoded);
                    plane.extend_from_slice(&encoded);
                    plane.extend_from_slice(&[0xff; 2]);
                }
            }
            let expected = decode_pcm_chunk(format, encoding, &packed).unwrap();
            let planes: Vec<_> = planar
                .iter()
                .map(|bytes| PcmPlane {
                    bytes,
                    offset_bytes: 3,
                    stride_bytes: width + 2,
                })
                .collect();
            let decoded = decode_pcm_planes(format, encoding, count, &planes).unwrap();
            assert_eq!(decoded, expected);
            let shared_views: Vec<_> = (0..channels)
                .map(|channel| PcmPlane {
                    bytes: &packed,
                    offset_bytes: channel * width,
                    stride_bytes: channels * width,
                })
                .collect();
            assert_eq!(
                decode_pcm_planes(format, encoding, count, &shared_views).unwrap(),
                expected
            );
            let mut direct = RepetitionFrameAssembler::new(format, 123);
            let mut converted = RepetitionFrameAssembler::new(format, 123);
            assert_eq!(
                serde_json::to_value(direct.push(0, &expected).unwrap()).unwrap(),
                serde_json::to_value(converted.push(0, &decoded).unwrap()).unwrap()
            );
        }
    }
}

#[test]
fn dimensions_and_all_plane_ranges_are_checked_before_sample_values() {
    let format = Format::new(96_000, 8).unwrap();
    let maximum = 96_000 / 50 * MAX_CHUNK_FRAMES;
    let bytes = vec![0; maximum * 4];
    let mut planes: Vec<_> = (0..8)
        .map(|_| PcmPlane {
            bytes: &bytes,
            offset_bytes: 0,
            stride_bytes: 4,
        })
        .collect();
    assert_eq!(
        decode_pcm_planes(format, PcmEncoding::Float32Le, maximum, &planes)
            .unwrap()
            .len(),
        maximum * 8
    );
    for count in [0, maximum + 1, usize::MAX] {
        assert_eq!(
            decode_pcm_planes(format, PcmEncoding::Float32Le, count, &planes),
            Err(FrameError::InvalidLength)
        );
    }
    assert_eq!(
        decode_pcm_planes(format, PcmEncoding::Float32Le, 1, &planes[..7]),
        Err(FrameError::InvalidLength)
    );
    planes.push(PcmPlane {
        bytes: &bytes,
        offset_bytes: 0,
        stride_bytes: 4,
    });
    assert_eq!(
        decode_pcm_planes(format, PcmEncoding::Float32Le, 1, &planes),
        Err(FrameError::InvalidLength)
    );
    let nan = f32::NAN.to_le_bytes();
    let invalid = [
        PcmPlane {
            bytes: &nan,
            offset_bytes: 0,
            stride_bytes: 4,
        },
        PcmPlane {
            bytes: &[],
            offset_bytes: 0,
            stride_bytes: 4,
        },
    ];
    assert_eq!(
        decode_pcm_planes(
            Format::new(8_000, 2).unwrap(),
            PcmEncoding::Float32Le,
            1,
            &invalid
        ),
        Err(FrameError::InvalidLength)
    );
}

#[test]
fn padding_is_not_decoded_and_invalid_last_sample_returns_no_output() {
    let format = Format::new(8_000, 1).unwrap();
    let mut bytes = Vec::new();
    for value in [0.5_f32, -0.25] {
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes.extend_from_slice(&f32::NAN.to_le_bytes());
    }
    let planes = [PcmPlane {
        bytes: &bytes,
        offset_bytes: 0,
        stride_bytes: 8,
    }];
    assert_eq!(
        decode_pcm_planes(format, PcmEncoding::Float32Le, 2, &planes).unwrap(),
        [0.5, -0.25]
    );
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.01, -1.01] {
        bytes[8..12].copy_from_slice(&invalid.to_le_bytes());
        let planes = [PcmPlane {
            bytes: &bytes,
            offset_bytes: 0,
            stride_bytes: 8,
        }];
        assert_eq!(
            decode_pcm_planes(format, PcmEncoding::Float32Le, 2, &planes),
            Err(FrameError::InvalidSample)
        );
    }
}

#[test]
fn planar_offsets_and_padding_preserve_channel_order() {
    let left = [99, 0, 0x40, 99, 0, 0xc0];
    let right = [99, 0, 0x20, 99, 0, 0xe0];
    let planes = [
        PcmPlane {
            bytes: &left,
            offset_bytes: 1,
            stride_bytes: 3,
        },
        PcmPlane {
            bytes: &right,
            offset_bytes: 1,
            stride_bytes: 3,
        },
    ];
    assert_eq!(
        decode_pcm_planes(
            Format::new(8_000, 2).unwrap(),
            PcmEncoding::Signed16Le,
            2,
            &planes
        )
        .unwrap(),
        [0.5, 0.25, -0.5, -0.25]
    );
}

#[test]
fn address_overflow_truncation_and_overlapping_samples_are_rejected() {
    let format = Format::new(8_000, 1).unwrap();
    for (offset, stride, count) in [(usize::MAX, 2, 1), (0, usize::MAX, 3), (0, 2, 2), (0, 1, 1)] {
        let planes = [PcmPlane {
            bytes: &[0; 2],
            offset_bytes: offset,
            stride_bytes: stride,
        }];
        assert_eq!(
            decode_pcm_planes(format, PcmEncoding::Signed16Le, count, &planes),
            Err(FrameError::InvalidLength)
        );
    }
}
