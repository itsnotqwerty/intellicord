use anyhow::{Result, ensure};
use intellicord::acoustic::{Format, SpectralFrameAssembler};
use serde_json::json;

fn main() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "This generated-signal example accepts no arguments"
    );
    let format = Format::new(44_100, 2).expect("fixed supported format");
    let mut assembler = SpectralFrameAssembler::new(format, 1_000_000);
    let samples: Vec<_> = [1_000.0, 6_000.0]
        .into_iter()
        .flat_map(|frequency| {
            (0..format.sample_count() / 2).flat_map(move |index| {
                let value = (0.5
                    * (std::f64::consts::TAU * frequency * index as f64 / 44_100.0).sin())
                    as f32;
                [value, -value]
            })
        })
        .chain([0.5; 20])
        .collect();
    let mut position = 0;
    let mut features = Vec::new();
    for chunk in samples.chunks(74) {
        let batch = assembler
            .push(position, chunk)
            .expect("valid generated chunk");
        features.extend(batch.features);
        position += (chunk.len() / 2) as u64;
    }
    let recovery = assembler
        .push(position + 5, &vec![0.0; format.sample_count()])
        .expect("valid recovery chunk");
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "source": "Generated PCM only; no capture device was opened",
            "limitations": "Unattributed signal summaries; no clock synchronization, repetition matching or micspam classification",
            "features": features,
            "recovery": recovery
        }))?
    );
    Ok(())
}
