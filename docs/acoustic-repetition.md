# Offline Spectral Repetition Features

[src/acoustic/repetition.rs](../src/acoustic/repetition.rs) exposes `RepetitionAnalyzer` and serializable `RepetitionFeatures` through the acoustic module. The optional `RepetitionFrameAssembler` accepts supplied PCM chunks through the existing bounded framing path. These are offline feature helpers, not a sensor, content fingerprint service, or micspam detector. No capture, device access, worker, runtime routing, automatic response, file input, or persistence is added.

```sh
cargo run --quiet --example analyze_synthetic_repetition
cargo run --quiet --example assemble_synthetic_repetition
cargo test acoustic::repetition::tests
cargo test --test repetition-pipeline
```

The example generates two tone sequences in memory: a repeated five-frame sequence and a sequence followed by spectrally disjoint tones. It prints only the final summaries to stdout and accepts no arguments. Synthetic separation is not representative accuracy or calibration.

## Input And Continuity

`process(format, frame_start_ns, interleaved)` accepts the same validated normalized interleaved PCM, fixed 20 ms frames, and monotonic timestamps as the [level/spectral analyzers](acoustic-foundation.md). It runs spectral analysis once and borrows its normalized power internally. No public raw spectrum ingestion API is added. Invalid lengths, samples, overlaps, backwards timestamps, or clock overflow leave all state unchanged.

The first frame starts a new descriptor history. A timestamp gap, format change (including channel count), numerical spectral silence, or `reset()` clears comparison history. The first usable frame after an interruption can seed a fresh sequence but cannot match against pre-interruption data. Ten consecutive usable frames are needed for the first comparison. `reset()` also resets the contained spectral analyzer and permits a new timestamp origin. Call it for source changes even when timestamps appear contiguous.

The standalone analyzer accepts complete frames; `RepetitionFrameAssembler` adds chunked input as described below. `SpectralFrameAssembler` still emits spectral-only summaries without repetition history. None of these helpers is wired into the sensor bus. Output summaries alone cannot reconstruct the descriptor used here.

## Chunked Repetition Pipeline

`RepetitionFrameAssembler::new(format, origin_ns)`, `push(start_sample_per_channel, interleaved)`, and `reset(new_origin_ns)` mirror the [existing assembler contract](acoustic-foundation.md). Format is fixed; a change requires a new instance. Chunk start positions count absolute samples per channel, and the trusted origin maps position zero to monotonic nanoseconds. Input is nonempty, channel-aligned, normalized PCM with at most four frames per push. Timestamp mapping uses the same checked absolute-index arithmetic without cumulative rounding drift; it does not synchronize OS clocks.

`push` returns `AssemblyBatch<RepetitionFeatures>` with at most four ordered summaries, shared gap/discard metadata, and the buffered sample count per channel. Each completed frame computes levels and its FFT once before updating repetition history, without serializing PCM, introducing a frame queue, or accepting external descriptor injection. The level-only and spectral-only APIs are unchanged.

All chunk validation finishes before any state mutation. A forward position gap immediately clears spectral state and repetition history, even if the push emits zero features. The first completed recovery frame is discontinuous with unavailable RMS change, spectral flux, and repetition similarity. Ten fresh usable frames are needed to resume repetition comparison; silence restarts that requirement. Repeated gaps are reported separately and cannot bridge descriptor sequences. Explicit reset discards partial PCM, returns its sample count per channel, clears both analyzers/history, and restarts sample position zero at a new origin. Invalid chunks do not clear history or partially emit early frames.

The combined path retains less than one frame of PCM between calls in the existing one-frame allocation, in addition to the bounded descriptor/spectral state below. PCM overwrite on consumption/discard/drop is ordinary clearing, not guaranteed secure zeroization. Output contains nested level/spectral summaries and repetition metrics, not PCM or descriptors. This is synchronous allocating offline processing, not a benchmarked real-time callback or a running worker.

The chunked example generates ten frames of repeated stereo tones at 44.1 kHz, supplies small channel-aligned chunks, then simulates a gap after a partial frame before silent recovery. It prints only the final pre-gap summary, completed-frame count, and recovery batch. Four public-API tests in [tests/repetition-pipeline.rs](../tests/repetition-pipeline.rs) compare direct/chunked outputs across four formats and two chunk sizes, including descriptor eviction and silence, gap recovery requiring ten fresh frames, reset, invalid-whole-chunk atomicity, output bounds, and fractional-clock overflow. A private consumer test also verifies that zero-output gaps reset immediately before returning, while rejected chunks do not reset.

## Descriptor And Comparison

For each usable frame, the analyzer groups the non-DC one-sided normalized power into 32 contiguous bands. Non-DC bin index `k` maps to band `floor((k - 1) * 32 / non_dc_bin_count)`. Band values sum to one up to floating-point rounding. Bands cover relative positions up to Nyquist, not fixed perceptual or frequency ranges. Format changes therefore invalidate history. Hann leakage, spectral resolution, and the numerical floor are exactly those of the spectral analyzer.

The most recent five descriptors (100 ms) are compared in order with five-frame windows delayed by each available lag from 5 through 50 frames (100 ms through 1 second), inclusive. Each candidate window is nonoverlapping with the current window, though candidates can overlap one another. Per-frame similarity is `1 - sum(abs(current_band - prior_band)) / 2`, clamped to `[0, 1]`. A sequence score is the mean of its five frame similarities. The maximum is reported, with exact ties choosing the shortest lag. There is no score threshold, detection flag, cooldown, or abuse probability.

Schema version 1 / feature version `acoustic-repetition/0.1` contains the nested spectral summary, descriptor-band count, comparison-window/minimum-lag/maximum-lag metadata, number of compared lags, and nullable `best_lag_ns` / `best_similarity`. Lag denotes the difference between window start times, not an observed clip length or verified repetition period. When no comparison is available the count is zero and both best fields are null. A low best score still has a best lag; it does not establish a repeated pattern. Searching more lags increases the opportunity for chance similarity.

## Bounds And Privacy

History retains at most 55 descriptors, representing 1.1 seconds of contiguous usable audio: 1,760 `f64` values (14,080 bytes of logical descriptor values), plus container/allocator overhead and the bounded spectral analyzer state. Each accepted frame compares at most 46 lags, five frame pairs each, and 32 bands per pair. FFT workspace/allocation and CPU/latency remain unbenchmarked. Clearing a container discards logical history, not necessarily its allocated capacity. None of this is secure memory erasure.

No PCM or descriptor arrays are serialized or persisted. Temporary FFT buffers still contain sample-derived values; retained descriptors and exported summaries can reveal audio information and are not anonymous or nonidentifying guarantees. Callers remain responsible for consent, source verification, their own input buffers, and summary retention. No participant identity or source attribution is inferred.

## Limitations And Verification

Pure tones, music, steady noise, accessibility audio, or unrelated sounds with similar band powers can score highly. Normalized spectra largely ignore amplitude and phase; different content can share a descriptor. Short five-frame windows, fixed bin grouping, no time-warp alignment, and a one-second search horizon can miss genuine repeated clips, especially with time shifts, pitch changes, interruptions, or mixed playback. This is not a robust audio-content fingerprint or proof of repetition, intent, coordination, or abuse.

Five tests cover repeated versus disjoint synthetic sequences at multiple sample rates with gain/opposite-phase channels, silence/gap/format/reset boundaries, atomic rejection, the unique maximum-lag match and its eviction (including an intermediate 0.8 similarity), retention/search bounds, tie handling, and summary-only serialization. Representative benign/adversarial datasets, independent descriptor arithmetic oracles, longer fingerprints, temporal calibration, mixed-audio evaluation, and measured resource budgets remain future work.