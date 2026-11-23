use anyhow::{Result, ensure};
use intellicord::acoustic::{FRAME_DURATION_NS, Format, RepetitionAnalyzer};
use serde_json::json;

fn main() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "This generated-signal example accepts no arguments"
    );
    let format = Format::new(48_000, 1).expect("fixed supported format");
    let mut repeated = RepetitionAnalyzer::default();
    let mut disjoint = RepetitionAnalyzer::default();
    let mut summaries = Vec::new();
    for index in 0..10 {
        let frequency = 500.0 + (index % 5) as f64 * 500.0;
        for (label, analyzer, hz) in [
            ("repeated", &mut repeated, frequency),
            (
                "disjoint",
                &mut disjoint,
                frequency + if index < 5 { 0.0 } else { 6_000.0 },
            ),
        ] {
            let samples: Vec<_> = (0..format.sample_count())
                .map(|sample| {
                    (0.5 * (std::f64::consts::TAU * hz * sample as f64 / 48_000.0).sin()) as f32
                })
                .collect();
            let result = analyzer
                .process(format, index * FRAME_DURATION_NS, &samples)
                .expect("generated valid PCM");
            if index == 9 {
                summaries.push(json!({"case": label, "result": result}));
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "source": "Generated PCM only; no capture device was opened",
            "limitations": "Coarse spectral sequence similarity; not content identity, abuse probability or speaker attribution",
            "summaries": summaries
        }))?
    );
    Ok(())
}
