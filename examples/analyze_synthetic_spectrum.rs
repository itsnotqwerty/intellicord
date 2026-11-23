use anyhow::{Result, ensure};
use intellicord::acoustic::{FRAME_DURATION_NS, Format, SpectralAnalyzer};
use serde_json::json;

fn main() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "This generated-signal example accepts no arguments"
    );
    let format = Format::new(48_000, 2).expect("fixed supported format");
    let mut analyzer = SpectralAnalyzer::default();
    let mut frames = Vec::new();
    for (index, frequency) in [1_000.0, 1_000.0, 6_000.0, 0.0].into_iter().enumerate() {
        let samples: Vec<_> = (0..format.sample_count() / 2)
            .flat_map(|sample_index| {
                let value = (0.5
                    * (std::f64::consts::TAU * frequency * sample_index as f64 / 48_000.0).sin())
                    as f32;
                [value, -value]
            })
            .collect();
        frames.push(
            analyzer
                .process(format, index as u64 * FRAME_DURATION_NS, &samples)
                .expect("generated valid PCM"),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "source": "Generated PCM only; no capture device was opened",
            "limitations": "Unattributed signal features only; no repetition matching, micspam classification or calibrated scores",
            "frames": frames
        }))?
    );
    Ok(())
}
