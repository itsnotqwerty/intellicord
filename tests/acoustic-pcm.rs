use intellicord::acoustic::{
    Format, FrameError, MAX_CHUNK_FRAMES, PcmEncoding, RepetitionFrameAssembler, decode_pcm_chunk,
};

fn encodings() -> [(PcmEncoding, usize); 4] {
    [
        (PcmEncoding::Signed16Le, 2),
        (PcmEncoding::Signed24Le, 3),
        (PcmEncoding::Signed32Le, 4),
        (PcmEncoding::Float32Le, 4),
    ]
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
fn lengths_enforce_complete_channel_groups_and_four_frame_limit() {
    for (rate, channels) in [(8_000, 1), (44_100, 2), (96_000, 8)] {
        let format = Format::new(rate, channels).unwrap();
        for (encoding, width) in encodings() {
            let maximum = format.sample_count() * MAX_CHUNK_FRAMES * width;
            assert_eq!(
                decode_pcm_chunk(format, encoding, &vec![0; maximum])
                    .unwrap()
                    .len(),
                maximum / width
            );
            for length in [0, 1, maximum - 1, maximum + channels * width] {
                assert_eq!(
                    decode_pcm_chunk(format, encoding, &vec![0; length]),
                    Err(FrameError::InvalidLength)
                );
            }
            if channels > 1 {
                assert_eq!(
                    decode_pcm_chunk(format, encoding, &vec![0; width]),
                    Err(FrameError::InvalidLength)
                );
            }
        }
    }
}

#[test]
fn decoded_chunks_match_direct_pipeline_across_formats_and_encodings() {
    for (rate, channels) in [(8_050, 1), (44_100, 2), (96_000, 8)] {
        let format = Format::new(rate, channels).unwrap();
        let integers: Vec<i16> = (0..format.sample_count() * 10)
            .map(|index| match index % 4 {
                0 => -16_384,
                1 => 8_192,
                2 => 24_576,
                _ => -8_192,
            })
            .collect();
        for (encoding, _) in encodings() {
            let mut decoded_pipeline = RepetitionFrameAssembler::new(format, 123);
            let mut direct_pipeline = RepetitionFrameAssembler::new(format, 123);
            let mut position = 0;
            let mut frames = 0;
            for chunk in integers.chunks(37 * channels) {
                let bytes: Vec<_> = chunk
                    .iter()
                    .flat_map(|sample| encode(*sample, encoding))
                    .collect();
                let decoded = decode_pcm_chunk(format, encoding, &bytes).unwrap();
                let expected: Vec<_> = chunk
                    .iter()
                    .map(|sample| f32::from(*sample) / 32_768.0)
                    .collect();
                assert_eq!(decoded, expected);
                let batch = decoded_pipeline.push(position, &decoded).unwrap();
                frames += batch.features.len();
                assert_eq!(
                    serde_json::to_value(batch).unwrap(),
                    serde_json::to_value(direct_pipeline.push(position, &expected).unwrap())
                        .unwrap()
                );
                position += (chunk.len() / channels) as u64;
            }
            assert_eq!(frames, 10);
        }
    }
}

#[test]
fn signed_integer_low_bits_and_endianness_match_independent_scaling() {
    let format = Format::new(8_000, 1).unwrap();
    for (encoding, width) in encodings().into_iter().take(3) {
        let bits = width * 8;
        let divisor = (1_u64 << (bits - 1)) as f64;
        for value in [
            -(1_i64 << (bits - 1)),
            -65_537,
            -257,
            -1,
            0,
            1,
            257,
            65_537,
            (1_i64 << (bits - 1)) - 1,
        ] {
            if (value as f64) < -divisor || (value as f64) >= divisor {
                continue;
            }
            let encoded = value.to_le_bytes();
            let decoded = decode_pcm_chunk(format, encoding, &encoded[..width]).unwrap();
            assert_eq!(decoded, [(value as f64 / divisor) as f32]);
        }
    }
}

#[test]
fn integer_extrema_and_channel_order_have_explicit_scaling() {
    let format = Format::new(8_000, 2).unwrap();
    let cases = [
        (
            PcmEncoding::Signed16Le,
            vec![0x00, 0x80, 0xff, 0x7f, 0, 0, 0, 0x40],
            32_767.0 / 32_768.0,
        ),
        (
            PcmEncoding::Signed24Le,
            vec![0, 0, 0x80, 0xff, 0xff, 0x7f, 0, 0, 0, 0, 0, 0x40],
            8_388_607.0 / 8_388_608.0,
        ),
        (
            PcmEncoding::Signed32Le,
            vec![
                0, 0, 0, 0x80, 0xff, 0xff, 0xff, 0x7f, 0, 0, 0, 0, 0, 0, 0, 0x40,
            ],
            1.0,
        ),
    ];
    for (encoding, bytes, positive_max) in cases {
        assert_eq!(
            decode_pcm_chunk(format, encoding, &bytes).unwrap(),
            [-1.0, positive_max, 0.0, 0.5]
        );
    }
}

#[test]
fn float_input_rejects_nonfinite_and_out_of_range_tails() {
    let format = Format::new(8_000, 1).unwrap();
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0001, -1.0001] {
        let bytes: Vec<_> = [0.5_f32, invalid]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        assert_eq!(
            decode_pcm_chunk(format, PcmEncoding::Float32Le, &bytes),
            Err(FrameError::InvalidSample)
        );
    }
    let expected = [-1.0_f32, -0.0, 0.0, 1.0, f32::MIN_POSITIVE];
    let bytes: Vec<_> = expected.into_iter().flat_map(f32::to_le_bytes).collect();
    let decoded = decode_pcm_chunk(format, PcmEncoding::Float32Le, &bytes).unwrap();
    assert_eq!(
        decoded
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        expected.map(f32::to_bits)
    );
}
