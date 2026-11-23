# Offline Voice Evaluation

This deterministic synthetic harness is not a validated abuse classifier or a field benchmark. It does not attach to Discord, read account data, start sensors, request privileges, or execute moderation. Production thresholds are unchanged.

## Run

```sh
cargo run --quiet --example evaluate_voice
cargo test --example evaluate_voice
```

The first command prints versioned JSON to stdout. Saving a report is explicit shell redirection:

```sh
cargo run --quiet --example evaluate_voice > /tmp/intellicord-voice-evaluation.json
```

There are no arguments or arbitrary external input paths; fixtures are embedded at compile time. Malformed fixtures fail before a report is emitted. A rule-presence regression prints the report with `matches_expected_output: false`, then exits unsuccessfully. The example's tests also run under normal `cargo test`.

## Contract

[The evaluator](../examples/evaluate_voice.rs) replays [voice-patterns.json](../tests/fixtures/voice-patterns.json) through production `History`, the typed sensor bus, and `Intelligence`. Each case has an isolated history, session, and bus; any publication/fusion rejection fails evaluation. Replay uses synthetic backend receipt times and a fixed UTC origin, without sleeping or reading the machine clock. The first snapshot is a baseline; later samples have reconciliation provenance. Participant IDs remain strings and are omitted from the aggregate report.

[voice-evaluation.json](../tests/fixtures/voice-evaluation.json) defines dataset `synthetic-voice-evaluation/0.1`. Each case names a source trace and records author-written rationale, positive rule labels, labeled onset time, expected detected rules, and optional timing offsets/capture-gap steps.

- `positive_rules` is the hand-authored target label for scoring. Absence of a rule means a negative case for that rule. A positive case requires nonnegative `onset_ms`; a wholly negative case has no onset.
- `expected_detected_rules` records current algorithm behavior, independently of target labels. A benign case can match the expected output while counting as a false positive; an interrupted positive case can match while counting as a miss.
- `timing_offsets_ms` applies a signed offset per sample, preserving a zero start and nondecreasing adjusted timestamps. This perturbs receipt times, not remote actions. `onset_ms` uses the adjusted receipt timeline.
- `capture_gap_steps` marks interruptions through the production frame flag. Receipt gaps over five seconds use existing engine gap handling. Unknown intermediate transitions are not fabricated.

Bounds are 128 cases, 64 source traces, 512 steps per trace, 100 participants per snapshot, one hour per case, and 1 MiB per fixture input. Invalid IDs, duplicate participants/names/rule labels, unknown fields/rules, unmatched offsets, and invalid timestamps fail evaluation. Evidence-retention eviction also fails instead of silently scoring incomplete findings. Long-duration evaluation needs a streaming finding collector, not larger retention defaults.

## Metrics

The unit is one binary decision per case per rule: did the rule produce at least one incident? Repeated incidents do not create extra true positives; incident counts are reported separately. Each rule reports true/false positives/negatives, precision, and recall. Precision is true positives divided by detected-positive cases; recall is true positives divided by target-positive cases. Empty denominators produce JSON `null`. Scores use target labels, never expected outputs.

Each fired rule reports its actual detector version, first detection receipt time, and incident count. Labeled-positive detections also report receipt latency from onset; negative detections have `null` latency. These are simulated time intervals, not CPU benchmarks, transport-delay measurements, or time from a remote person's action. Misses have no latency sample, not zero latency.

## Synthetic Results

The initial ten-case dataset produces:

| Rule | True Positive | False Positive | True Negative | False Negative | Precision | Recall |
| --- | --- | --- | --- | --- | --- | --- |
| Join burst | 2 | 1 | 5 | 2 | 2/3 | 1/2 |
| Rapid re-entry | 2 | 1 | 7 | 0 | 2/3 | 1 |

Burst receipt latency is 4,000 ms, or 3,875 ms with the supplied jitter. Re-entry latency is 5,000 ms, or 5,125 ms with jitter. Capture interruption and a seven-second receipt interval each produce a gap and suppress burst inference: both count as misses against the positive target label. Known missing transition counts stay zero because the fixture does not establish missing intermediate transitions.

Two intentionally indistinguishable pairs expose a fundamental limitation: scheduled group entry has the same observed facts as the disruptive burst label, and benign reconnection has the same facts as disruptive cycling. Both benign lookalikes count as false positives. A transition-only rule cannot infer their differing intent.

These hand-authored cases reuse traces and include deterministic variants. They are not independent population samples or a training/holdout split. The numbers are reproducible fixture scores only, not Discord field accuracy, calibrated confidence, acceptable false-alert rates, or improvements over independent baselines. Do not tune defaults or enable punitive responses based on these results.

## Remaining Work

- Collect consented, purpose-scoped, representative traces; define independent splits and simple comparison baselines without tuning to the evaluation set.
- Measure false alerts/hour using representative negative exposure and explicit capture-coverage/censoring rules. These short fixtures are not a valid exposure denominator.
- Exercise queue pressure and longer sessions separately; immediate per-frame draining does not stress concurrency or overflow.
- Extend regression gates for incident multiplicity, latency distributions, and detector-version policy. The CLI currently compares rule presence; selected metrics have unit-test assertions.
- Evaluate mixed audio and speaker uncertainty only after a separately consented acoustic pipeline exists. This harness evaluates neither audio nor attribution.