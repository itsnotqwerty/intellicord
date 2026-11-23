use anyhow::{Result, ensure};
use intellicord::acoustic::{Analyzer, FRAME_DURATION_NS, Format};
use serde_json::json;

fn main() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "Usage: cargo run --example analyze_synthetic_audio (generated frames only)"
    );
    let format = Format::new(48_000, 2).unwrap();
    let mut analyzer = Analyzer::default();
    let mut frames = Vec::new();
    for (index, label) in [
        "silence",
        "opposite_phase_tone",
        "full_scale_square",
        "silence_after_gap",
    ]
    .into_iter()
    .enumerate()
    {
        let samples: Vec<_> = (0..format.sample_count() / 2)
            .flat_map(|sample_index| {
                let level = match index {
                    1 => {
                        (0.5 * (std::f64::consts::TAU * 1000.0 * sample_index as f64 / 48_000.0)
                            .sin()) as f32
                    }
                    2 => {
                        if sample_index % 2 == 0 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    _ => 0.0,
                };
                [level, -level]
            })
            .collect();
        let start = if index == 3 { 5 } else { index as u64 } * FRAME_DURATION_NS;
        let features = analyzer
            .process(format, start, &samples)
            .map_err(|error| anyhow::anyhow!("Synthetic frame rejected: {error:?}"))?;
        frames.push(json!({"synthetic_label":label,"features":features}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"schema_version":1,
        "source":"Generated synthetic PCM only; no capture device was opened",
        "limitations":"Level/clipping features are not micspam detection, calibrated scores, or speaker attribution",
        "frames":frames}))?
    );
    Ok(())
}
