use std::{
    fs::{File, OpenOptions},
    io::{BufReader, Read},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use hound::{SampleFormat, WavReader};
use intellicord::acoustic::{Format, RepetitionFeatures, RepetitionFrameAssembler};
use serde::Serialize;

const MAX_SECONDS: u32 = 60;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u16,
    evaluator_version: &'static str,
    package_version: &'static str,
    source: &'static str,
    limitations: &'static str,
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    encoding: &'static str,
    input_samples_per_channel: u32,
    completed_frames: usize,
    analyzed_duration_ns: u64,
    mean_frame_rms: f64,
    peak: f64,
    frames_with_spectrum: usize,
    frames_with_repetition_comparison: usize,
    maximum_coarse_similarity: Option<f64>,
    final_feature: RepetitionFeatures,
}

fn evaluate(input: impl Read) -> Result<Report> {
    let mut reader = WavReader::new(BufReader::new(input.take(MAX_FILE_BYTES)))
        .context("Invalid or unsupported WAV layout")?;
    let spec = reader.spec();
    let format = Format::new(spec.sample_rate, usize::from(spec.channels))
        .map_err(|error| anyhow::anyhow!("Unsupported acoustic format: {error:?}"))?;
    ensure!(
        matches!(
            (spec.sample_format, spec.bits_per_sample),
            (SampleFormat::Int, 8 | 16 | 24 | 32) | (SampleFormat::Float, 32)
        ),
        "Supported encodings: integer PCM 8/16/24/32-bit or IEEE float32"
    );
    let scalar_samples = reader.len();
    ensure!(scalar_samples > 0, "Empty WAV input");
    ensure!(
        u64::from(scalar_samples)
            <= u64::from(spec.sample_rate) * u64::from(spec.channels) * u64::from(MAX_SECONDS),
        "WAV duration exceeds 60 seconds"
    );
    ensure!(
        (scalar_samples as usize).is_multiple_of(format.sample_count()),
        "WAV must contain complete channel groups and complete 20 ms frames; no samples are silently discarded"
    );
    let mut samples: Box<dyn Iterator<Item = Result<f32>> + '_> = match spec.sample_format {
        SampleFormat::Float => Box::new(reader.samples::<f32>().map(|value| Ok(value?))),
        SampleFormat::Int => {
            let divisor = (1_u64 << (spec.bits_per_sample - 1)) as f64;
            Box::new(
                reader
                    .samples::<i32>()
                    .map(move |value| Ok((f64::from(value?) / divisor) as f32)),
            )
        }
    };
    let mut assembler = RepetitionFrameAssembler::new(format, 0);
    let mut frame = vec![0.0; format.sample_count()];
    let expected_frames = scalar_samples as usize / frame.len();
    let mut rms_sum = 0.0;
    let mut peak = 0.0_f64;
    let mut spectra = 0;
    let mut comparisons = 0;
    let mut maximum_similarity: Option<f64> = None;
    let mut final_feature = None;
    for index in 0..expected_frames {
        for value in &mut frame {
            *value = samples.next().context("Truncated WAV data")??;
        }
        let position = index as u64 * u64::from(spec.sample_rate / 50);
        let batch = assembler
            .push(position, &frame)
            .map_err(|error| anyhow::anyhow!("Invalid PCM frame {index}: {error:?}"))?;
        ensure!(
            batch.gap.is_none()
                && batch.buffered_samples_per_channel == 0
                && batch.features.len() == 1,
            "Unexpected frame accounting"
        );
        let feature = batch.features.into_iter().next().unwrap();
        rms_sum += feature.spectral.levels.rms;
        peak = peak.max(feature.spectral.levels.peak);
        spectra += usize::from(feature.spectral.centroid_hz.is_some());
        if let Some(similarity) = feature.best_similarity {
            comparisons += 1;
            maximum_similarity =
                Some(maximum_similarity.map_or(similarity, |previous| previous.max(similarity)));
        }
        final_feature = Some(feature);
    }
    ensure!(samples.next().is_none(), "Unexpected extra decoded samples");
    Ok(Report {
        schema_version: 1,
        evaluator_version: "offline-acoustic-wav/0.1",
        package_version: env!("CARGO_PKG_VERSION"),
        source: "Explicitly supplied WAV; no capture device opened; input path omitted",
        limitations: "Audio-derived sensitive summaries; no labels, classifier, abuse probability, speaker attribution or accuracy measurement; no resampling; mean_frame_rms is not whole-file RMS",
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        bits_per_sample: spec.bits_per_sample,
        encoding: if spec.sample_format == SampleFormat::Int {
            "integer PCM"
        } else {
            "IEEE float"
        },
        input_samples_per_channel: scalar_samples / u32::from(spec.channels),
        completed_frames: expected_frames,
        analyzed_duration_ns: expected_frames as u64 * 20_000_000,
        mean_frame_rms: rms_sum / expected_frames as f64,
        peak,
        frames_with_spectrum: spectra,
        frames_with_repetition_comparison: comparisons,
        maximum_coarse_similarity: maximum_similarity,
        final_feature: final_feature.unwrap(),
    })
}

fn open_input(filename: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(filename).context("Cannot open supplied WAV")?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_FILE_BYTES,
        "Input must be a regular file no larger than 128 MiB"
    );
    Ok(file)
}

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let filename = arguments
        .next()
        .context("Usage: evaluate_acoustic INPUT.wav")?;
    ensure!(arguments.next().is_none(), "Expected exactly one WAV path");
    let file = open_input(Path::new(&filename))?;
    let report = evaluate(file)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{WavSpec, WavWriter};
    use std::io::Cursor;

    fn wav(spec: WavSpec, values: impl IntoIterator<Item = f32>) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = WavWriter::new(&mut output, spec).unwrap();
            for value in values {
                if spec.sample_format == SampleFormat::Float {
                    writer.write_sample(value).unwrap();
                } else {
                    writer
                        .write_sample(
                            (f64::from(value) * (1_u64 << (spec.bits_per_sample - 1)) as f64)
                                as i32,
                        )
                        .unwrap();
                }
            }
            writer.finalize().unwrap();
        }
        output.into_inner()
    }

    fn spec() -> WavSpec {
        WavSpec {
            channels: 2,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        }
    }

    #[test]
    fn integer_encodings_preserve_energy_and_channel_signs() {
        for bits in [8, 16, 24, 32] {
            let spec = WavSpec {
                bits_per_sample: bits,
                ..spec()
            };
            let report = evaluate(Cursor::new(wav(
                spec,
                [0.5, -0.5].into_iter().cycle().take(320),
            )))
            .unwrap();
            assert_eq!(report.completed_frames, 1);
            assert_eq!(report.analyzed_duration_ns, 20_000_000);
            assert_eq!(report.mean_frame_rms, 0.5);
            assert_eq!(report.peak, 0.5);
            assert_eq!(report.frames_with_spectrum, 0);
            assert!(report.maximum_coarse_similarity.is_none());
        }
    }

    #[test]
    fn float_tone_uses_full_repetition_history_without_serializing_pcm() {
        let spec = WavSpec {
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
            ..spec()
        };
        let values = (0..320 * 60).map(|index| {
            (std::f32::consts::TAU * 1_000.0 * (index / 2) as f32 / 8_000.0).sin()
                * if index % 2 == 0 { 0.5 } else { -0.5 }
        });
        let report = evaluate(Cursor::new(wav(spec, values))).unwrap();
        assert_eq!(report.completed_frames, 60);
        assert_eq!(report.frames_with_spectrum, 60);
        assert_eq!(report.frames_with_repetition_comparison, 51);
        assert_eq!(report.final_feature.compared_lags, 46);
        assert!(report.maximum_coarse_similarity.unwrap() > 0.999);
        let json = serde_json::to_value(report).unwrap();
        assert!(json.get("samples").is_none());
        assert!(json.get("path").is_none());
    }

    #[test]
    fn rejects_empty_partial_unsupported_and_truncated_files() {
        for bytes in [
            wav(spec(), []),
            wav(spec(), [0.0; 2]),
            wav(
                WavSpec {
                    sample_rate: 11_025,
                    ..spec()
                },
                [0.0; 320],
            ),
            vec![0; 44],
        ] {
            assert!(evaluate(Cursor::new(bytes)).is_err());
        }
        let mut truncated = wav(spec(), [0.0; 320]);
        truncated.pop();
        assert!(evaluate(Cursor::new(truncated)).is_err());
    }

    #[test]
    fn rejects_invalid_float_tail_after_valid_frames() {
        let spec = WavSpec {
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
            ..spec()
        };
        for invalid in [f32::NAN, f32::INFINITY, 1.01, -1.01] {
            let mut values = vec![0.0; 640];
            *values.last_mut().unwrap() = invalid;
            assert!(evaluate(Cursor::new(wav(spec, values))).is_err());
        }
    }

    #[test]
    fn rejects_overlong_declared_audio_before_decoding() {
        let mut bytes = wav(spec(), [0.0; 320]);
        let position = bytes.windows(4).position(|chunk| chunk == b"data").unwrap();
        let declared = (8_000_u32 * 2 * 2 * 61).to_le_bytes();
        bytes[position + 4..position + 8].copy_from_slice(&declared);
        let error = evaluate(Cursor::new(bytes)).unwrap_err();
        assert!(error.to_string().contains("60 seconds"));
    }

    #[test]
    fn buffers_scalar_reads_and_preserves_known_ancillary_layout_limit() {
        struct Counted<'reader> {
            input: Cursor<Vec<u8>>,
            calls: &'reader std::cell::Cell<usize>,
        }
        impl Read for Counted<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.calls.set(self.calls.get() + 1);
                self.input.read(buffer)
            }
        }
        let calls = std::cell::Cell::new(0);
        evaluate(Counted {
            input: Cursor::new(wav(spec(), [0.0; 3200])),
            calls: &calls,
        })
        .unwrap();
        assert!(calls.get() < 10);
        for size in [1_u32, 2] {
            let mut bytes = wav(spec(), [0.0; 320]);
            let position = bytes.windows(4).position(|chunk| chunk == b"data").unwrap();
            let mut chunk = b"JUNK".to_vec();
            chunk.extend(size.to_le_bytes());
            chunk.extend([0, 0]);
            bytes.splice(position..position, chunk);
            let riff_size = (bytes.len() as u32 - 8).to_le_bytes();
            bytes[4..8].copy_from_slice(&riff_size);
            assert_eq!(evaluate(Cursor::new(bytes)).is_ok(), size == 2);
        }
    }

    #[test]
    #[cfg(unix)]
    fn rejects_nonregular_descriptor_without_blocking() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let filename =
            std::env::temp_dir().join(format!("intellicord-wav-fifo-{}", std::process::id()));
        let c_filename = CString::new(filename.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_filename.as_ptr(), 0o600) }, 0);
        let result = open_input(&filename);
        std::fs::remove_file(filename).unwrap();
        assert!(result.unwrap_err().to_string().contains("regular file"));
    }
}
