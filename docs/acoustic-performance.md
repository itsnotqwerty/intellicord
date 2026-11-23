# Offline Acoustic Performance

Run `cargo run --release --quiet --example benchmark_acoustic` for JSON on stdout. The [harness](../examples/benchmark_acoustic.rs) opens no devices, accesses no Discord process, and accepts no arguments. It exercises the public `RepetitionFrameAssembler`, including level, spectral, and coarse repetition processing. It does not change runtime behavior.

## Workloads

Twelve fixed cases combine 8,050 Hz mono, 44,100 Hz stereo, and 96,000 Hz/eight channels with two chunk sizes and two signals. Chunks contain either 37 samples per channel or four complete 20 ms frames. Signals are silence or a repeated 20 ms two-tone frame (1 kHz and 2.5 kHz); alternate channels have opposite signs. The frame restarts its phase at each boundary, including the odd-length 8,050 Hz case. These synthetic signals are not representative speech, music, gameplay, laughter, or adversarial audio.

Each case pre-generates input, creates an assembler, times its first single-frame push, then excludes 60 further warmup frames. The warmed usable-tone history has reached the maximum repetition search; silence keeps that history empty. Measurement processes exactly 200 frames representing four seconds. Every push is checked for absent gaps and bounded output/pending samples, and final accounting requires exactly 200 completed frames and no partial remainder. The largest generated measured PCM vector is 3,072,000 `f32` values (12,288,000 logical bytes), independent of the analyzer's much smaller retained state. This is input storage, not a measured memory footprint.

## Timing Contract

`Instant` measures elapsed wall-clock nanoseconds around each synchronous push, result metadata extraction, and returned-summary destruction. Generation, assembler construction, warmup, external accounting checks, timing-vector appends, percentile sorting, and JSON serialization are outside those intervals. `black_box` consumes results before the timer ends. Timer reads and small metadata operations are included; overhead is not subtracted. Scheduling interruptions can affect results.

`cold_first_frame_ns` includes FFT planning on a fresh assembler, not process startup or guaranteed cold CPU caches. Cases run sequentially in a fixed order without randomized trials. The 60 warmup frames follow that first frame.

`measured_pushes` summarizes every measured push. `frame_producing_pushes` summarizes only pushes that complete at least one frame. Both report call count, summed duration, maximum, and nearest-rank p50/p95: sorted index `ceil(percent * count / 100) - 1`. Small-chunk all-push percentiles can predominantly measure buffering, so inspect frame-producing timings as well. Four-frame-push timings cover four frames, not one. Neither distribution is end-to-end capture latency.

`processing_to_audio_ratio` is summed all-push duration divided by represented audio duration; lower means less measured processing wall time per unit audio. It excludes between-push bookkeeping and is neither whole-program throughput nor CPU utilization. No timing threshold is enforced in tests.

Reports include schema/benchmark/package versions, target OS/architecture and `debug_assertions`. The latter is not a complete compiler-profile description. Record the command, compiler, CPU, build settings and system load when comparing runs; package version alone does not identify a source revision. Unit tests check percentile arithmetic and workload counts, not speed.

## Local Baseline

One Linux x86_64 release-command run on the development host reported these two-tone frame-producing p95 values, rounded to microseconds:

| Format | 37-sample chunks (one output frame) | Four-frame chunks |
| --- | ---: | ---: |
| 8,050 Hz / mono | 11 | 42 |
| 44,100 Hz / stereo | 48 | 206 |
| 96,000 Hz / eight channels | 342 | 1,439 |

The 96 kHz/eight-channel cases used about 0.017 measured processing seconds per represented audio second. These are illustrative single-run observations, not portable budgets or guarantees; host CPU/load/compiler details were not captured in the report. Rerun locally rather than treating this table as a regression oracle.

Still unmeasured: CPU time, allocation count/peak resident memory, long-duration contention/tails, representative content, capture scheduling/loss, PipeWire reconnects, and real-time worker handoff. No classification accuracy, consent/capture integration, speaker attribution, or safe callback execution follows from this benchmark. See [the acoustic contract](acoustic-foundation.md) and [the roadmap](intellicord-future-architecture.md).