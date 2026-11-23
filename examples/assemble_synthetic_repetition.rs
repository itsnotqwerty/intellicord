use anyhow::{Result, ensure};
use intellicord::acoustic::{Format, RepetitionFrameAssembler};
use serde_json::json;

fn main() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "This generated-signal example accepts no arguments"
    );
    let format = Format::new(44_100, 2).expect("fixed supported format");
    let samples: Vec<_> = (0..10)
        .flat_map(|frame| {
            let frequency = 500.0 + (frame % 5) as f64 * 500.0;
            (0..882).flat_map(move |index| {
                let value = (0.5
                    * (std::f64::consts::TAU * frequency * index as f64 / 44_100.0).sin())
                    as f32;
                [value, -value]
            })
        })
        .chain([0.5; 20])
        .collect();
    let mut assembler = RepetitionFrameAssembler::new(format, 1_000_000);
    let mut position = 0;
    let mut last_feature = None;
    let mut completed_frames = 0;
    for chunk in samples.chunks(74) {
        let batch = assembler
            .push(position, chunk)
            .expect("valid generated chunk");
        completed_frames += batch.features.len();
        if let Some(feature) = batch.features.into_iter().last() {
            last_feature = Some(feature);
        }
        position += (chunk.len() / 2) as u64;
    }
    let recovery = assembler
        .push(position + 5, &vec![0.0; format.sample_count()])
        .expect("valid silent recovery");
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "source": "Generated PCM only; no capture device was opened",
            "limitations": "Coarse spectral sequence similarity only; not content identity, abuse probability or attribution",
            "completed_frames": completed_frames,
            "last_feature": last_feature,
            "recovery": recovery
        }))?
    );
    Ok(())
}
