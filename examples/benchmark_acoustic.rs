use std::{hint::black_box, time::Instant};

use anyhow::{Result, ensure};
use intellicord::acoustic::{Format, MAX_CHUNK_FRAMES, RepetitionFrameAssembler};
use serde::Serialize;

const WARMUP_FRAMES: usize = 60;
const MEASURED_FRAMES: usize = 200;

#[derive(Debug, Serialize)]
struct Timing {
    calls: usize,
    total_ns: u128,
    p50_ns: u128,
    p95_ns: u128,
    maximum_ns: u128,
}

fn timing(mut durations: Vec<u128>) -> Result<Timing> {
    ensure!(!durations.is_empty(), "No timing samples");
    durations.sort_unstable();
    let percentile = |percent: usize| durations[(durations.len() * percent).div_ceil(100) - 1];
    Ok(Timing {
        calls: durations.len(),
        total_ns: durations.iter().sum(),
        p50_ns: percentile(50),
        p95_ns: percentile(95),
        maximum_ns: *durations.last().unwrap(),
    })
}

#[derive(Debug, Serialize)]
struct Case {
    sample_rate: u32,
    channels: usize,
    signal: &'static str,
    chunk_samples_per_channel: usize,
    cold_first_frame_ns: u128,
    warmup_frames: usize,
    measured_frames: usize,
    represented_audio_ns: u128,
    measured_pushes: Timing,
    frame_producing_pushes: Timing,
    processing_to_audio_ratio: f64,
}

fn run_case(rate: u32, channels: usize, small_chunks: bool, silence: bool) -> Result<Case> {
    let format = Format::new(rate, channels).map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let frame_samples = rate as usize / 50;
    let chunk_samples = if small_chunks {
        37
    } else {
        frame_samples * MAX_CHUNK_FRAMES
    };
    let frame: Vec<f32> = (0..frame_samples)
        .flat_map(|index| {
            let value = if silence {
                0.0
            } else {
                (0.25 * (std::f64::consts::TAU * 1_000.0 * index as f64 / rate as f64).sin()
                    + 0.125 * (std::f64::consts::TAU * 2_500.0 * index as f64 / rate as f64).sin())
                    as f32
            };
            (0..channels).map(move |channel| if channel % 2 == 0 { value } else { -value })
        })
        .collect();
    let samples: Vec<_> = frame
        .iter()
        .copied()
        .cycle()
        .take(format.sample_count() * MEASURED_FRAMES)
        .collect();
    let mut assembler = RepetitionFrameAssembler::new(format, 0);
    let cold_start = Instant::now();
    let cold = assembler
        .push(0, black_box(&frame))
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let cold_frames = cold.features.len();
    black_box(cold);
    let cold_first_frame_ns = cold_start.elapsed().as_nanos();
    ensure!(cold_frames == 1, "Cold frame was not processed");
    let mut position = frame_samples as u64;
    for _ in 0..WARMUP_FRAMES {
        let batch = assembler
            .push(position, black_box(&frame))
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        ensure!(
            batch.features.len() == 1
                && batch.gap.is_none()
                && batch.buffered_samples_per_channel == 0,
            "Warmup accounting failed"
        );
        black_box(batch);
        position += frame_samples as u64;
    }
    let mut durations =
        Vec::with_capacity((MEASURED_FRAMES * frame_samples).div_ceil(chunk_samples));
    let mut completed = 0;
    let mut producing_durations = Vec::with_capacity(MEASURED_FRAMES);
    let mut pending = 0;
    for chunk in samples.chunks(chunk_samples * channels) {
        let started = Instant::now();
        let batch = assembler
            .push(position, black_box(chunk))
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let count = batch.features.len();
        let has_gap = batch.gap.is_some();
        pending = batch.buffered_samples_per_channel;
        black_box(batch);
        let elapsed = started.elapsed().as_nanos();
        ensure!(
            !has_gap && count <= MAX_CHUNK_FRAMES && pending < frame_samples,
            "Measured batch accounting failed"
        );
        completed += count;
        position += (chunk.len() / channels) as u64;
        durations.push(elapsed);
        if count > 0 {
            producing_durations.push(elapsed);
        }
    }
    ensure!(
        completed == MEASURED_FRAMES && pending == 0,
        "Incomplete measured workload"
    );
    let measured_pushes = timing(durations)?;
    let represented_audio_ns = MEASURED_FRAMES as u128 * 20_000_000;
    Ok(Case {
        sample_rate: rate,
        channels,
        signal: if silence {
            "silence"
        } else {
            "steady two-tone opposite-phase channels"
        },
        chunk_samples_per_channel: chunk_samples,
        cold_first_frame_ns,
        warmup_frames: WARMUP_FRAMES,
        measured_frames: completed,
        represented_audio_ns,
        processing_to_audio_ratio: measured_pushes.total_ns as f64 / represented_audio_ns as f64,
        measured_pushes,
        frame_producing_pushes: timing(producing_durations)?,
    })
}

fn main() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "Generated-only benchmark accepts no arguments"
    );
    let mut cases = Vec::new();
    for (rate, channels) in [(8_050, 1), (44_100, 2), (96_000, 8)] {
        for small in [false, true] {
            for silence in [false, true] {
                cases.push(run_case(rate, channels, small, silence)?);
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "benchmark_version": "offline-acoustic-benchmark/0.1",
            "package_version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "debug_assertions": cfg!(debug_assertions),
            "source": "Generated PCM only; no capture device was opened",
            "timing_scope": "Wall-clock push plus returned summary destruction; input generation, timing-vector append and external accounting excluded; cold includes first FFT planning, warmup excludes 60 frames",
            "limitations": "Single-process synthetic microbenchmark; no CPU time, memory/allocator measurement, PipeWire scheduling, capture loss or classification accuracy; not a real-time guarantee",
            "cases": cases
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles_and_total_are_explicit() {
        let result = timing((1..=20).rev().collect()).unwrap();
        assert_eq!(result.p50_ns, 10);
        assert_eq!(result.p95_ns, 19);
        assert_eq!(result.maximum_ns, 20);
        assert_eq!(result.total_ns, 210);
        assert!(timing(vec![]).is_err());
        assert_eq!(timing(vec![7]).unwrap().p95_ns, 7);
    }

    #[test]
    fn workload_accounting_handles_small_and_maximum_chunks() {
        for small in [false, true] {
            for silence in [false, true] {
                let result = run_case(8_050, 1, small, silence).unwrap();
                assert_eq!(result.measured_frames, MEASURED_FRAMES);
                assert_eq!(
                    result.measured_pushes.calls,
                    (161 * MEASURED_FRAMES).div_ceil(result.chunk_samples_per_channel)
                );
                assert!(result.processing_to_audio_ratio.is_finite());
                assert!(result.measured_pushes.p50_ns <= result.measured_pushes.p95_ns);
                assert!(result.measured_pushes.p95_ns <= result.measured_pushes.maximum_ns);
                assert_eq!(
                    result.frame_producing_pushes.calls,
                    if small {
                        MEASURED_FRAMES
                    } else {
                        MEASURED_FRAMES / MAX_CHUNK_FRAMES
                    }
                );
            }
        }
    }
}
