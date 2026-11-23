# Offline WAV Evaluation

The [WAV evaluator](../examples/evaluate_acoustic.rs) lets you inspect acoustic feature summaries from one explicitly supplied local recording. It is an offline development example, not part of the installed injector and not audio capture. Use only recordings you are authorized to process.

```sh
cargo run --release --quiet --example evaluate_acoustic -- /path/to/recording.wav
```

The command reads the file and writes one JSON report to stdout only after successful decoding and analysis. Errors go to stderr with a nonzero exit status and no partial JSON report. It neither writes a report file itself nor uploads anything. Redirecting stdout is an explicit persistence action; the resulting summaries are sensitive audio-derived data even though they contain no PCM or input path.

## Accepted Input

- Exactly one regular file of at most 128 MiB, containing at least one complete 20 ms frame and no more than 60 seconds of declared audio.
- Sample rates from 8,000 through 96,000 Hz divisible by 50; one through eight channels.
- Integer PCM at 8, 16, 24 or 32 bits, or 32-bit IEEE floating-point PCM, decoded using `hound` 3.5.1.
- Integer samples are normalized by `2^(bits - 1)`; 8-bit WAV unsigned storage is handled by the decoder. Int32-to-float32 rounding may map the maximum positive value to 1.0. Floating samples must be finite and within [-1, 1]; values are never silently clamped.
- The declared data sample count must comprise complete channel groups and complete 20 ms frames. Partial final frames are rejected, not padded or discarded. No resampling, mixing or channel conversion occurs.

Compressed WAV, RF64 and arbitrary RIFF layouts are not promised. In particular, `hound` 3.5.1 rejects some otherwise valid WAVs containing odd-length ancillary chunks because it does not skip their padding correctly. This fails visibly as an unsupported layout; it does not justify calling the source recording corrupt. An even-length ancillary chunk is covered by regression tests. The evaluator processes the first declared data chunk and does not validate all trailing chunks or physical EOF. Successful evaluation is not a whole-file integrity verdict.

Regular-file checks use the opened descriptor; Unix opens use nonblocking mode so a named FIFO is rejected without waiting for a writer. Reads are buffered over a 128 MiB byte-limited stream. The byte limit bounds consumption, not I/O latency or wall-clock execution time. Files changed concurrently are not guaranteed to be a consistent snapshot; use an immutable input copy when reproducibility matters.

## Report Semantics

Reports include schema/evaluator/package versions, format/encoding, per-channel input sample count, completed frames, analyzed duration, arithmetic mean of frame RMS values, peak over all frames, count of frames with usable spectra, count with available coarse sequence comparisons, maximum coarse similarity, and the final nested repetition/spectral/level feature record. The final feature supplies the underlying feature-version metadata.

`mean_frame_rms` is the mean of equally sized frame RMS values, not the square root of whole-file mean energy and not perceived loudness. DC can have nonzero RMS with no usable spectrum because spectral processing removes the mean. Frames with available comparisons are not detections. Maximum similarity has selection bias and can be high for unrelated content sharing a spectrum; it is not content identity, abuse probability or a validated repetition decision. The final feature describes only the last 20 ms frame and its bounded recent history, not every event in the recording.

No labels, classification thresholds, automatic actions, attribution, precision/recall, false alerts/hour or performance metrics are introduced. There is no association with Discord participants or channel/session context. Representative independent speech/music/gameplay/laughter/adversarial datasets still need to be sourced with appropriate rights, labeled, and evaluated separately. For synthetic timing measurements use [the performance harness](acoustic-performance.md).

## Resource And Privacy Boundaries

The evaluator streams one frame-sized input vector, with at most 15,360 scalar `f32` values (61,440 logical bytes), in addition to the decoder's bounded read buffer and the existing [assembler and analyzer state](acoustic-foundation.md). It retains aggregate counters and one final feature record rather than all per-frame summaries. These logical bounds are not measured allocator/RSS budgets. Processing time grows with duration, rate and channel count; no timeout or CPU budget is enforced.

No PCM is serialized, queued to the runtime bus or intentionally persisted by the evaluator. Input files remain untouched and are not deleted. Input/decoder/FFT buffers and derived summaries are not securely zeroized, and normal OS caching/swapping is outside this guarantee. No microphone, PipeWire stream, Discord attachment, UI, network sensor, scoring or attenuation is enabled. `hound` and the Unix file-opening helper dependency are development dependencies, not runtime capture components.

## Verification

`cargo test --example evaluate_acoustic` covers all four integer encodings, constant opposite-phase channel energy, float tones through the full repetition-history search, empty/unsupported/truncated input, invalid float tails, declared-duration rejection, buffered reads, known ancillary-layout compatibility and Unix FIFO rejection. Normal `cargo test` includes these example tests. Command-level checks additionally exercised generated WAVs at 8,050 Hz mono, 44.1 kHz stereo and 96 kHz/eight channels, exact duration/frame counts, omitted input paths, and empty stdout on malformed input or invalid invocation. These synthetic checks do not establish field accuracy or compatibility with every WAV exporter.