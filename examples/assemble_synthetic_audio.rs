use anyhow::{Result, ensure};
use intellicord::acoustic::{Format, FrameAssembler};
use serde_json::json;

fn main() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "Usage: cargo run --example assemble_synthetic_audio (generated chunks only)"
    );
    let format = Format::new(44_100, 2).unwrap();
    let mut assembler = FrameAssembler::new(format, 1_000_000);
    let samples: Vec<_> = (0..format.sample_count() * 2 + 20)
        .map(|index| if index % 2 == 0 { 0.5 } else { -0.5 })
        .collect();
    let mut start = 0;
    let mut features = Vec::new();
    let mut chunks = 0;
    for chunk in samples.chunks(74) {
        let batch = assembler
            .push(start, chunk)
            .map_err(|error| anyhow::anyhow!("Synthetic chunk rejected: {error:?}"))?;
        ensure!(batch.gap.is_none(), "Unexpected synthetic continuity gap");
        features.extend(batch.features);
        start += (chunk.len() / 2) as u64;
        chunks += 1;
    }
    let recovery = assembler
        .push(start + 5, &vec![0.0; format.sample_count()])
        .map_err(|error| anyhow::anyhow!("Synthetic recovery rejected: {error:?}"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "source": "Generated PCM only; no capture device was opened",
            "limitations": "Sample clock mapping only; no OS clock synchronization, micspam classification, or source verification",
            "input_chunks_before_gap": chunks,
            "contiguous_features": features,
            "recovery_batch": recovery,
        }))?
    );
    Ok(())
}
