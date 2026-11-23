# Intellicord — Future Architecture Proposal

**Status:** Phase 1 prototypes, offline Phase 2 helpers, explicit PipeWire CLI capture and optional Aya process-sensor source/reader implemented; real BPF-target build and offline instruction inventory verified locally; operator-reported load, attach/detach and controlled-child delivery success on Linux 6.18.58-1-lts; supervised capture and supplied client health snapshots implemented, with valid-object supervised capture untested; sustained collection compatibility and protections remain unvalidated or unfinished  
**Prepared:** 2026-11-17  
**Project:** [itsnotqwerty/intellicord](https://github.com/itsnotqwerty/intellicord)  
**Target:** Linux desktop, with Arch Linux as the initial reference environment  
**Primary implementation languages:** Rust, JavaScript, eBPF (where justified)

## Executive summary

Intellicord should evolve from an opt-in Discord desktop enhancement into a **modular, local-first defensive intelligence platform** for recognizing and responding to advanced disruptive activity: micspam, coordinated voice raids, repetitive harassment, automated account behavior, and security anomalies involving the local client. The proposed architecture joins four sources of evidence:

1. **Discord application events**, observed through Intellicord's existing renderer adapter, for member identities and call transitions.
2. **Local audio analysis**, preferably via PipeWire, for detecting acoustic abuse in audio the user already receives.
3. **Linux process and network telemetry**, primarily via existing eBPF interfaces, for client integrity, resilience, and local anomaly detection.
4. **Rust correlation and detection**, to build explainable incidents, calculate calibrated confidence, and offer proportionate local defenses.

**Important boundary:** Linux kernel telemetry cannot identify remote Discord participants, decrypt Discord voice traffic, or reliably attribute a mixed audio stream to one speaker. Audio and behavior detection should therefore lead the product; kernel instrumentation should remain an optional supporting capability. Prefer eBPF and existing Linux facilities to custom loadable kernel modules (`.ko`).

## 1. Existing system and design constraints

As inspected on 2026-11-17, Intellicord is a Rust-backed, opt-in extension for the Discord desktop client. A JavaScript adapter uses existing client voice stores; a Rust process handles typed snapshots and bounded join/leave history. Attachment uses a **loopback-only Chromium DevTools endpoint**, not a supported Discord plugin API.

Current capabilities described in the repository include:

- Callmon history for observed voice joins/leaves, channel switching, searchable user/activity filters, and limited in-memory call archives.
- Native user context menus, exact user IDs, and explicit Markdown log exports.
- A user-invoked public-profile lookup shortcut.
- Optional sponsored-content UI suppression, with a locally persisted preference.
- A one-second reconciliation poll in addition to store subscriptions.

Relevant current implementation seams:

| File | Current responsibility | Proposed evolution |
|---|---|---|
| `src/web/bridge.js` | Discord voice-store discovery and observation | Emit validated, provenance-marked events at the highest practical fidelity |
| `src/web/client.js` | Injection entry and lifecycle | Manage opt-in feature activation, cleanup, and health reporting |
| `src/lib.rs` | Typed snapshots and bounded call history | Extract a reusable event domain and correlation interfaces |
| `src/main.rs` | DevTools transport and injection lifecycle | Supervise sensor services and health without exposing extra remote interfaces |
| `src/web/menu.js` | User controls and presentation | Surface alerts, context, review, and mitigation controls |
| `src/preferences.rs` | Local preference persistence | Support per-feature privacy and retention settings |

**Non-negotiable constraints:** No remote telemetry by default; no user credentials needed; minimize privileged code; preserve reversible client changes; avoid unsupported claims about user identity or coordinated control; make every sensitive sensor opt-in. Discord's private client interfaces may change, and client modification may introduce terms-of-service or account risk.

## 2. Goals and non-goals

### Goals

- Recognize rapid voice joins/leaves, suspicious re-entry cycles, and unusual group movement.
- Identify likely micspam from locally audible signal features while minimizing false positives.
- Maintain explainable incident timelines and confidence-weighted alerts.
- Correlate local client/network failures with Discord state to prevent misleading conclusions.
- Harden the Linux client and Intellicord backend against accidental or malicious local disruption.
- Support local controls and moderator review, not opaque punishment.
- Provide reproducible tests and documented privacy boundaries.

### Non-goals

- Decrypting Discord traffic, bypassing Discord permissions, or extracting authentication tokens.
- Obtaining remote users' IP addresses or hardware/device fingerprints via the local kernel.
- Claiming to identify a speaker from generic network packets or a mixed playback stream.
- Covert audio monitoring or default indefinite audio retention.
- Automatically accusing or punishing users based on weak or uncalibrated signals.
- Building custom kernel modules simply to replace standard Linux interfaces.

## 3. Proposed architecture

```text
                      ┌──────────────────────────────┐
                      │ Discord desktop client       │
                      │ voice stores + UI context    │
                      └──────────────┬───────────────┘
                                     │ validated events
                      ┌──────────────▼───────────────┐
                      │ JS adapter / DevTools bridge  │
                      └──────────────┬───────────────┘
                                     │
      ┌──────────────────────┐       │        ┌────────────────────────┐
      │ PipeWire audio tap   ├───────┤        │ eBPF sensor (optional) │
      │ local PCM features   │       │        │ sockets/process events │
      └──────────────────────┘       │        └───────────┬────────────┘
                                     │                    │
                            ┌────────▼────────────────────▼──┐
                            │ Rust event fusion engine       │
                            │ schema, time, provenance, IDs  │
                            └───────────────┬────────────────┘
                                            │
                            ┌───────────────▼────────────────┐
                            │ Detection / incident engine    │
                            │ acoustic, raids, coordination  │
                            │ evidence + calibrated scores   │
                            └───────────────┬────────────────┘
                                            │
                            ┌───────────────▼────────────────┐
                            │ Intellicord client UI           │
                            │ alerts, review, local controls │
                            │ explicit evidence export       │
                            └────────────────────────────────┘
```

### Core data contract

Create a versioned `Observation` envelope in Rust (and corresponding JS schema):

```rust
struct Observation {
    schema_version: u16,
    observation_id: String,
    source: ObservationSource,
    monotonic_ns: u64,
    wall_time_utc: String,
    session_id: String,
    subject: Option<SubjectRef>,
    kind: ObservationKind,
    evidence_quality: EvidenceQuality,
}
```

Requirements:

- Preserve Discord snowflake IDs as **strings**, including through JS serialization.
- Separate **observed facts** (`joined voice channel`) from **inference** (`possible coordinated raid`).
- Use monotonic clocks for local intervals and UTC timestamps for human-readable evidence; record clock uncertainty when crossing processes.
- Track whether evidence comes from a store notification, reconciliation snapshot, audio feature, or OS sensor.
- Deduplicate replayed observations and prevent negative or impossible duration calculations.
- Avoid persisting raw audio or identity histories without explicit settings; scope retention to purpose.

## 4. Candidate Linux components and priorities

The impact/effort estimates below are architectural judgments, **not measured benchmarks**. A “kernel component” can be a userspace service consuming a kernel interface.

| Priority | Component | Primary benefit | Impact | Effort | Implementation approach |
|---:|---|---|---|---|---|
| 1 | `intellicord-observe` | Local process/socket telemetry and debugging | High | Medium | Aya/libbpf eBPF programs plus unprivileged Rust reader |
| 2 | `intellicord-acoustic` | Local micspam detection and playback protection | Very high | Medium–high | PipeWire stream + Rust DSP; **not** a kernel module |
| 3 | `intellicord-contain` | Resource and process resilience | High | Low–medium | cgroup v2, systemd user service, sandbox settings |
| 4 | `intellicord-netintel` | Connection anomaly correlation | Medium | Medium | eBPF socket statistics; no payload decryption |
| 5 | `intellicord-integrity` | Local client/process integrity | High for local compromise | High | Linux audit/fanotify/eBPF; signature and allowlist controls |
| 6 | `intellicord-clock` | Accurate local event correlation | Medium | Low–medium | Monotonic timestamp APIs, source normalization |
| 7 | `intellicord-netguard` | Optional local traffic limits/containment | Medium for local risks | Medium | nftables/cgroup controls; carefully scoped and reversible |
| 8 | `intellicord-devices` | Local audio-device provenance and diagnostics | Low–medium | Medium | udev/PipeWire/ALSA metadata; not remote-user detection |

### 4.1 `intellicord-observe`: eBPF process/socket observation

Collect only locally meaningful metadata: process identity, connection initiation, socket errors, interface counters, traffic rate, and retransmission indicators where available. Stream bounded events into Rust through a ring buffer. Prefer CO-RE-compatible programs and explicit kernel-feature checks.

Use cases: confirm local client instability, identify unexpected local connections associated with the process, and correlate disruptions with voice-state observations.

Limitations: packets are encrypted and routed through Discord infrastructure; they do not carry trustworthy remote-user attribution. Do not treat a remote endpoint as a Discord user.

### 4.2 `intellicord-acoustic`: opt-in micspam detector

Capture locally audible Discord output from PipeWire, then process short frames in a dedicated userspace worker. Candidate features:

- Short-time RMS/peak energy and abrupt amplitude changes.
- Clipping and sustained high-energy intervals.
- Spectral flux, high-frequency energy, and distortion characteristics.
- Repeated audio patterns using locally computed fingerprints.
- Burst timing, periodicity, and short-term changes from a rolling baseline.

Output: intervals, features, uncertainty, and anonymized alert evidence where practical. Apply smoothing, cool-downs, and thresholds calibrated against normal gaming, music, laughter, and accessibility use cases.

**Attribution:** Discord's mixed output should not be assigned to one user without a reliable additional source (e.g., trustworthy participant-specific voice activity or separate streams). Even when speaking-state correlation is available, label it *tentative* unless evidence is strong.

Responses: local volume attenuation, optional warning, reversible mute suggestion, and explicit incident review. Do not enable automatic participant-specific moderation from audio alone.

### 4.3 `intellicord-contain`: cgroups and isolation

Run the privileged sensor separately from the ordinary Intellicord backend. Use cgroup v2/systemd controls for memory, CPU, task count, and service restart behavior. Explore restricted filesystem access and syscall filtering compatible with the injector's legitimate behavior.

Protect the DevTools endpoint: bind **127.0.0.1 only**, verify the listener, avoid network forwarding, and never expose it externally. Keep sensors read-only by default. Use least-privilege permissions and refuse operation if safe attachment assumptions fail.

### 4.4 `intellicord-netintel`: network anomaly correlation

Correlate temporal changes in local latency proxies, errors, throughput, and socket transitions with app events. Detect possible local congestion or application transport problems, **not** remote account identity.

### 4.5 `intellicord-integrity`: local tamper signals

Use native audit/fanotify/eBPF observations to flag unexpected executable changes, suspicious auxiliary processes, and unauthorized local access to debugging services. Maintain explicit allowlists for Intellicord itself and known normal Discord updaters. Favor human-readable evidence over punitive actions.

### 4.6 Remaining ideas

- `intellicord-clock`: consolidate monotonic timing and event-order reconciliation; mostly userspace.
- `intellicord-netguard`: optional, narrowly scoped nftables controls for the **local host**; avoid blocking shared Discord service endpoints based on weak association.
- `intellicord-devices`: surface local microphone/virtual-device changes for diagnostics only. It cannot discover remote users' input-device provenance.

## 5. Behavioral intelligence engine

This is the most important detection layer for hostile Discord participants.

| Detector | Signals | Output | Suggested action |
|---|---|---|---|
| Voice raid | Join/leave rates, synchronized entry, rapid re-entry, group movements | Incident with timeline and rate features | Notify; export evidence |
| Micspam | Acoustic bursts, clipping, repetition, duration | Time-bounded acoustic alert | Attenuate locally; suggest review/mute |
| Suspected automation | Repetitive timings, high-volume observable actions, repeated patterns | Uncertain coordination hypothesis | Group events for review |
| Repeat harassment | Observed repeated targeting across sessions, if separately supported by available events | Escalation context | Moderator review; evidence export |
| Local client integrity | Suspicious local process behavior, abnormal debugging access | Host-side security finding | Warn and assist safe shutdown |
| Network instability | Socket errors, resource saturation, transport changes | Diagnostic explanation | Avoid false actor attribution |

Detection principles:

- Establish per-session and per-community baselines; do not hardcode a universal “raid” rate.
- Score **independent evidence classes** rather than counting the same store change multiple times.
- Represent uncertainty and unknowns explicitly; simultaneous joins are not proof of coordination.
- Store evidence alongside each finding so users can inspect why it triggered.
- Separate informational warnings, suggested local controls, and moderator-authorized actions.
- Measure precision, recall, false-alert frequency, and time to detection with labeled test fixtures.

## 6. UI and user experience

Add an optional **Safety & Intelligence** section without interrupting Callmon's existing history workflow.

Planned views:

1. **Live activity:** current call, bursts, anomaly indicators, and sensor status.
2. **Incidents:** grouped findings with timeline, detector version, and contributing observations.
3. **Evidence review:** exact observed facts, uncertainties, affected session/channel context, and export controls.
4. **Local defenses:** opt-in audio attenuation, warning sensitivity, and resource protection settings.
5. **Sensor diagnostics:** permission status, eBPF support, PipeWire stream health, dropped events, and feature compatibility.

Every alert should include a clear distinction between an **observation**, a **possible explanation**, and a **recommended action**. Provide suppressions and feedback on false positives. Preserve explicit, user-initiated Markdown export, escaping untrusted fields as Callmon already does.

## 7. Suggested repository layout

```text
intellicord/
├── src/
│   ├── web/                 # Existing Discord adapter / menus
│   ├── detection/           # Event fusion, rules, scoring
│   ├── acoustic/            # PipeWire interfaces and DSP
│   ├── investigation/       # Incidents, evidence, exports
│   └── platform/            # Sensor coordination + OS support
├── crates/
│   ├── intellicord-ebpf/    # Optional eBPF programs
│   ├── intellicord-sensors/ # Rust collectors + bounded queues
│   └── intellicord-dsp/     # Audio feature extraction
├── packaging/
│   └── linux/              # Service units, permissions, packaging
└── tests/
    ├── fixtures/           # Benign/abusive synthetic traces
    └── integration/        # End-to-end consent/cleanup checks
```

These are proposed paths, not a representation of currently existing files. Actual crate boundaries may change after prototyping, especially where Rust target constraints make a shared eBPF/userspace crate undesirable.

## 8. Phased implementation roadmap

### Initial implementation status (2026-11-17)

The first local-only slice is implemented in [src/detection.rs](../src/detection.rs), with renderer provenance in [src/web/bridge.js](../src/web/bridge.js), transport integration in [src/main.rs](../src/main.rs), and a read-only **Safety & Intelligence** review modal in [src/web/intelligence.js](../src/web/intelligence.js). Callmon's header shield icon opens the review. This is a prototype, not a validated raid detector.

- Schema version 1 observations carry session/observation IDs, exact string user/channel references, call and history sequence references, source, backend monotonic nanoseconds, and UTC wall time encoded as `wall_time_unix_ms`. Human-readable UTC ISO timestamps are formatted in the UI rather than stored as strings. `clock_uncertainty` explicitly describes unmeasured renderer sampling and transport latency.
- A bounded fusion log retains 512 observations and 50 incidents, with explicit eviction counters. Each incident embeds at most 20 evidence observations, independent of later observation eviction. A bounded in-process sensor bus now feeds this log, as described below; no separate sensor processes are present.
- Initial/switch participants establish baselines, not arrivals. Repeated history sequences are ignored. Capture recovery, receipt intervals over five seconds, backwards monotonic input, and known transitions lost before fusion produce gap observations. Correlation windows reset and detection is suppressed for the interrupted batch. Gap count is not a count of lost transitions: unknown intermediate transitions remain unknown.
- Unchanged capture sends a reconciliation health sample at most every two seconds (subject to renderer scheduling). Changed snapshots retain notification/reconciliation provenance. A process stall can only be recorded on the next received sample, not while no input is arriving.
- Two experimental rules group five distinct arrivals or three leave/return pairs within ten seconds of backend receipt time, with a 30-second per-rule cooldown per call. These are fixed prototype thresholds, not universal raid rates or measured confidence. Findings are labeled `uncalibrated`, explain benign alternatives, and never trigger alerts or moderation automatically. Unusual group movement, learned community baselines, and calibrated scoring remain unimplemented.
- Synthetic benign and disruptive-pattern traces live in [tests/fixtures/voice-patterns.json](../tests/fixtures/voice-patterns.json). Tests cover baselines, provenance, duplicate suppression, gaps, channel isolation, cooldown boundaries, retention, and real loopback transport. Demo desktop/mobile checks do not prove live Discord compatibility or detection accuracy.

No audio, eBPF, privileged process, network-payload inspection, persistent identity store, or automatic mitigation has been added. All intelligence data expires with the injector session; clearing Callmon history does not clear this separate bounded review log. Explicit reset/export controls were added in the workflow follow-up below; per-feature configuration remains future work. Rebuild both layers to use this protocol; installation or live activation is not part of this implementation milestone.

### Bounded ingestion follow-up (2026-11-17)

[src/ingestion.rs](../src/ingestion.rs) now provides an in-process, multi-producer/single-consumer bus using a bounded Rust channel. Publishers use nonblocking `try_send`; a full queue rejects the newest message without blocking existing call capture. At most 64 messages are queued or consumed per drain. Voice frames carry up to 500 retained transitions, exact user/channel IDs, call context, and capture provenance, but no usernames or avatars. Payload sizes, version, ordering, ID shape, and producer/payload pairing are checked before enqueueing.

Two typed producer classes exist: application voice frames and local diagnostic health reports. The latter is a tested integration contract only, not a running OS sensor. Future producers must use the backend session's monotonic clock domain and a single increasing sequence per producer class. Cloned senders do not create new producer identities; concurrent writers to the same identity must coordinate ordering. New sensor classes require explicit typed routing rather than arbitrary, unattributed event injection.

Fusion rejects stale sessions, repeated/out-of-order producer sequences, backwards producer clocks, regressed history, reused ended calls, and channel changes under an existing call ID. Rejections do not advance accepted producer watermarks or mutate participant evidence. Diagnostic health reports have no participant subject and cannot reset voice context, affect voice timing, or contribute to moderation/detection evidence. The UI shows the last accepted diagnostic state; there is no sensor heartbeat/liveness guarantee yet. With no producer connected, diagnostics remains **Not connected**. Saved join-burst evidence retains distinct triggering users so repeated arrivals from one person cannot crowd out the facts supporting the finding.

Application producer sequences start at one and must advance for every attempted publication, including a rejected/full-queue publication. A gap in accepted application sequence resets correlation and excludes that recovery frame from inference, although retained facts can still be recorded; inference resumes on fresh contiguous input. Earlier valid findings are preserved: a concurrently updated aggregate queue-drop counter is never applied retroactively to older frames. Diagnostic-report loss does not contaminate voice evidence. Queue drops count messages, not missing voice transitions. Known transition loss is counted separately using history sequences; unseen intermediate activity is never fabricated. A missing application message can only receive an ordered gap record when another application message is accepted; until then the aggregate queue metric still exposes overflow.

Production snapshots now traverse this bus, and the backend heartbeat drains pending reports while voice capture is idle. **Ingestion health** exposes queue capacity, accepted input count, overflow/invalid/closed-publisher counts, and rejected session/replay/clock/state counts. Retention evictions remain separately labeled. This adds no threads, sensors, listeners, privileges, IPC endpoint, network telemetry, persistence, or automatic responses. External sensor supervision, out-of-process clock normalization, and calibrated/community-specific rules remain future work.

### Evidence workflow follow-up (2026-11-17)

The review now offers explicit Markdown export and a separately confirmed intelligence reset. [src/web/intelligence-export.js](../src/web/intelligence-export.js) serializes only whitelisted fields, escaping untrusted text and retaining exact string IDs, provenance, detector versions, uncertainties, gaps, and health/retention metadata. Export covers all bounded retained observations/incidents and their embedded evidence, independently of UI display limits. When present, the same strict client parsers add a reduced session-integration snapshot: combined ownership, evidence verdict, timing inventory, acoustic terminal classification and processor conservation counts. It omits process sample counters because their freshness budget is maintained by the live client handoff rather than the static backend object. The export states that process freshness is publication-time only and does not establish continuing liveness, attribution, source identity or calibrated correlation. Unknown fields are never copied; malformed optional contracts render as unavailable. Older backends omit the section. Unsupported schemas, incomplete retained evidence, or oversized collections fail visibly. The output warns that findings are uncalibrated hypotheses and that IDs are personal data. It makes no network requests; the browser handles the explicit file download. Exported files remain until the user deletes them. Pending object URLs are capped at four and revoked on expiry or review close/disposal.

Reset requires a second explicit action with Cancel initially focused. Requests bind to the backend session and current reset count; stale sessions and replayed requests do nothing. The backend drains accepted pending messages before clearing the review, then publishes an acknowledgement. The renderer never clears optimistically and does not retry failed/unacknowledged requests automatically. Reset erases observations, incidents and their embedded evidence, detector windows/cooldowns, and review eviction/gap/missing-transition counters. It preserves Callmon history/archives, membership baseline, session identity, unique observation sequencing, accepted producer/history watermarks, and session-lifetime ingestion health. Capture continues: only fresh post-reset evidence can contribute to subsequent findings. Reset cannot delete exported files or erase IDs retained in Callmon's separate history.

Tests cover pending and populated evidence deletion, fresh findings without reused IDs, Callmon isolation, stale/replayed requests over real loopback transport, UI acknowledgement/failure/timeout/focus, download lifecycle bounds, escaped hostile fields, exact IDs, and full-retention export. Standalone demo checks at 1280px and 320px verified viewport bounds, export payload, reset cancellation/acknowledgement, unchanged history/health, and focus restoration. Automated screenshots of the inactive shared browser tab were unreliable; these checks do not prove live Discord compatibility or successful file saving through Discord. This workflow does not install, attach, capture audio, add sensors, or enable moderation. Rebuild frontend and backend together before deliberate activation.

### Offline evaluation follow-up (2026-11-17)

[examples/evaluate_voice.rs](../examples/evaluate_voice.rs) now replays ten labeled synthetic cases through production history, the typed bus, and fusion. Run `cargo run --quiet --example evaluate_voice` for a deterministic JSON report. Labels in [voice-evaluation.json](../tests/fixtures/voice-evaluation.json) are separate from expected rule outputs; intentional false positives and gap-related misses remain visible even when regression checks pass. The report includes per-rule confusion counts, precision/recall (null without a denominator), actual detector versions, receipt-time latency, and gap counts. Invalid or retention-truncated input fails evaluation; no participant identities are copied into the aggregate report.

Cases cover baseline safety, spaced arrivals, disruptive-pattern labels, identical benign synchronized-arrival/reconnect lookalikes, deterministic receipt jitter, explicit capture interruption, and delayed receipt. Burst counts are 2 true positives / 1 false positive / 5 true negatives / 2 misses; re-entry counts are 2 / 1 / 7 / 0. These are hand-authored, correlated fixture results, not measured Discord accuracy or calibrated confidence. Representative datasets, independent baselines, false alerts/hour, mixed audio, and field evaluation remain unimplemented. Thresholds and runtime behavior are unchanged. See [the evaluation guide](voice-evaluation.md) for exact definitions and limitations. Six evaluator tests run in normal `cargo test`; this milestone neither installs nor activates anything.

### Acoustic foundation follow-up (2026-11-17)

[src/acoustic.rs](../src/acoustic.rs) now supplies offline, bounded feature extraction for caller-supplied normalized interleaved PCM. Fixed 20 ms frames support 8-96 kHz rates divisible by 50 and one to eight channels, with a maximum of 15,360 scalar samples. Features include pooled/per-channel/max-channel RMS, peak, a near-full-scale sample ratio with explicit threshold, and a signed RMS change only across contiguous frames of the same format. Gaps and format changes suppress the comparison; invalid samples, sizes, overlap/backwards time, and timestamp overflow fail atomically. The analyzer retains only the previous format/end time/RMS, never PCM. Output has no participant attribution or detection score.

Run `cargo run --quiet --example analyze_synthetic_audio` for generated silence, opposite-phase tone, full-scale square-wave, and gap examples without opening any device. Eight analytic/boundary tests cover feature math, stereo cancellation safety, boundedness, invalid-input atomicity, and continuity/reset. These features are not a validated micspam detector: near-full-scale samples do not prove clipping, and high energy is compatible with ordinary content. No audio capture, PipeWire dependency, UI, producer routing, audio storage, privileges, or moderation has been added. Spectral/repetition features, capture consent/health, scoring, calibration, and reversible attenuation remain future work. See [the acoustic contract](acoustic-foundation.md) for details.

### Acoustic framing follow-up (2026-11-17)

[src/acoustic/framing.rs](../src/acoustic/framing.rs) now assembles bounded, caller-supplied interleaved PCM chunks into the analyzer's fixed 20 ms frames. Chunks must contain complete channel groups and at most four frames of samples; each push emits at most four feature records and retains less than one frame of partial PCM. Absolute per-channel sample positions map to a trusted monotonic anchor using checked integer arithmetic, avoiding cumulative rounding drift at rates such as 44.1 kHz. This is sample-clock mapping, not synchronization with PipeWire or another process.

Forward position jumps report known skipped samples separately from discarded partial samples, reset comparison state, and mark the first completed recovery feature discontinuous even when it arrives on a later push. Explicit reset discards the partial buffer, restarts sample position zero at a new origin, and marks recovery similarly. Invalid whole chunks and mapped-clock overflow fail before mutation. Buffered values are ordinarily overwritten when consumed/discarded or dropped; this is not guaranteed secure zeroization. No PCM is serialized or persisted. The direct analyzer still retains no PCM; the assembler owns a bounded temporary PCM buffer.

Run `cargo run --quiet --example assemble_synthetic_audio` for a device-free demonstration. Nine framing tests cover multi-rate chunk equivalence, 10,000 small chunks, gaps/restarts, atomic rejection, bounds, and fractional-clock overflow. The helper is synchronous and allocates output vectors; it is not a benchmarked real-time callback. No capture, PipeWire dependency, producer/worker, UI, attribution, scoring, attenuation, or live activation is added. Consent controls, application-output selection, buffer-format conversion, cross-clock normalization, health/reconnect supervision, and spectral/repetition features remain unimplemented. See [the expanded acoustic contract](acoustic-foundation.md).

### Offline spectral follow-up (2026-11-17)

[src/acoustic/spectral.rs](../src/acoustic/spectral.rs) adds an optional complete-frame `SpectralAnalyzer` using RustFFT, separate from the existing level analyzer and chunk assembler. Per-channel mean removal and a periodic Hann window precede exact-length transforms; one-sided channel powers are pooled without stereo phase cancellation. Schema version 1 / `acoustic-spectrum/0.1` provides level summaries plus spectral centroid, an explicit 4 kHz high-band power ratio, and positive differences between unit-sum power spectra. Numerical silence yields unavailable spectral measurements, and flux comparisons are suppressed across silence, gaps, resets, or format changes. No repetition matching, acoustic event score, calibrated confidence, or speaker attribution is implied.

State retains one FFT plan and at most 961 previous power bins, not a waveform. Temporary transform buffers contain audio-derived values and are not securely zeroized; spectra and summaries are not anonymous. Inputs, outputs, and working lengths are bounded, but FFT planning/allocation and latency have not been benchmarked for capture callbacks. Six tests include direct-DFT checks at odd/even lengths and Nyquist, known tones, unequal-channel power, band-boundary leakage, silence/DC/gain behavior, invalid-input atomicity, and continuity. Run `cargo run --quiet --example analyze_synthetic_spectrum` for generated-only summaries. The level-only assembler is not wired to spectral processing, the bus, or runtime UI. Capture consent, PipeWire integration, OS-clock normalization, repetition, scoring, calibration, and attenuation remain future work; nothing is installed or activated. See [the acoustic contract](acoustic-foundation.md).

### Combined acoustic pipeline follow-up (2026-11-17)

`SpectralFrameAssembler` in [src/acoustic/framing.rs](../src/acoustic/framing.rs) now connects supplied PCM chunks to level and spectral summaries through the existing validated assembly path. `AssemblyBatch` defaults to the existing level output and supports typed spectral output without changing level-only callers. Complete frames compute levels once; private spectral processing consumes that validated frame without exposing PCM or adding a frame queue. Bounds remain four input frames/four output summaries per push and less than one frame of retained PCM, plus a bounded prior spectrum and single FFT plan.

Whole-chunk validation precedes all state mutation. Gaps reset spectral state immediately even when recovery takes later pushes; the first recovered frame is discontinuous with neither RMS difference nor spectral flux. Later frames in a batch update comparisons in order, with silence still suppressing flux. Reset clears both processing paths and returns the partial sample discard count. Five public-API integration tests cover 15 formats, changing spectra/silence in a single batch, repeated gaps/delayed recovery/reset, atomic rejections, resource bounds, and fractional-clock overflow. Run `cargo run --quiet --example assemble_synthetic_spectrum` for a device-free example. This remains an allocating synchronous offline helper, not a sensor or real-time callback. No capture, PipeWire dependency, bus/UI routing, repetition matching, scoring, attenuation, installation, or live activation is added. See [the acoustic contract](acoustic-foundation.md).

### Offline repetition feature follow-up (2026-11-17)

[src/acoustic/repetition.rs](../src/acoustic/repetition.rs) adds a separate complete-frame `RepetitionAnalyzer` on top of existing spectral analysis. It groups normalized non-DC power into 32 relative-frequency bands, retains at most 55 descriptors (1.1 seconds), and compares the newest five-frame sequence with nonoverlapping sequences at available lags from 100 ms through one second. Mean per-frame normalized L1 similarity supplies a best lag and score, with comparison-count/window/lag metadata; no threshold, detection flag, calibrated confidence, or automatic response is added. Silence, gaps, any format change, and reset clear history; invalid frames fail before mutation.

Five tests cover synthetic repeated/disjoint sequences, gain/opposite-phase behavior, continuity, atomicity, search/retention bounds, maximum-lag eviction, and summary-only serialization. Run `cargo run --quiet --example analyze_synthetic_repetition` for a device-free example. Logical descriptor values occupy at most 14,080 bytes plus container/allocator and existing spectral state; exact allocations and CPU/latency remain unbenchmarked. Descriptors are audio-derived sensitive state, not anonymous fingerprints, and ordinary clearing is not secure erasure. Similar spectra do not establish matching content, repetition, or abuse; short fixed windows and no alignment can miss real repeated content. This helper is not connected to chunk assembly, capture, the sensor bus, UI, or scoring. Robust fingerprints, representative evaluation, and capture protections remain future work. See [the repetition contract](acoustic-repetition.md).

### Chunked repetition follow-up (2026-11-17)

`RepetitionFrameAssembler` in [src/acoustic/framing.rs](../src/acoustic/framing.rs) now connects repetition features to the same validated chunk-processing path used by level/spectral assembly. It emits `AssemblyBatch<RepetitionFeatures>` with nested level/spectral summaries, computing levels and the FFT once per completed frame. Bounds remain four input frames/four output summaries per push and less than one frame of partial PCM, alongside the existing 55-descriptor history, one prior spectrum, and one FFT plan. Existing level-only/spectral-only APIs remain unchanged; no PCM callback, descriptor injection, or intermediate frame queue is added.

Whole-chunk validation precedes all state mutation. Gaps reset spectral/repetition state immediately even with no completed frame, and the first recovery suppresses RMS change, spectral flux, and repetition comparison. Ten fresh usable frames are required for new sequence comparison; silence restarts the requirement. Reset clears processing/history, discards buffered PCM, and restarts the supplied clock origin. Four public integration tests cover direct/chunked equivalence, history eviction/silence, delayed gap recovery/reset, atomic invalid chunks, resource bounds, and fractional-clock overflow. A private test confirms immediate reset timing. Run `cargo run --quiet --example assemble_synthetic_repetition` for a device-free example. This is allocating offline code with unbenchmarked CPU/latency, not capture or a real-time worker. Robust fingerprints, representative evaluation, scoring, PipeWire, runtime bus/UI routing, and attenuation remain unfinished; no installation or live activation occurs. See [the repetition contract](acoustic-repetition.md).

### Offline performance follow-up

The generated-only [acoustic performance harness](acoustic-performance.md) measures the public chunk-to-repetition pipeline across twelve fixed format/chunk/signal combinations. It verifies frame/pending accounting, separates fresh-assembler first-push timing from 60 excluded warmup frames, and reports nearest-rank p50/p95/max for all pushes and frame-producing pushes. The latter avoids hiding FFT work behind small-chunk buffering calls. Each case measures 200 frames/four seconds with build/workload metadata. This establishes a local synthetic wall-clock baseline only, not CPU/memory budgets, representative-content results, capture latency or a real-time guarantee. Nothing is installed or live-activated.

### Offline supplied-recording follow-up

The standalone [WAV evaluator](acoustic-evaluation.md) now streams one explicitly supplied regular file through `RepetitionFrameAssembler`, emitting aggregate summaries and the final nested feature record only after complete successful analysis. It supports bounded integer/float PCM with strict format, 60-second and 128 MiB limits; incomplete frames, unsupported layouts and invalid samples fail visibly. Buffered bounded reads, Unix nonblocking file opening, decoder/feature arithmetic, truncation and invalid-tail behavior have regression coverage. Input paths and PCM are omitted from JSON. The parser has documented ancillary-chunk compatibility limitations; success is not a whole-file integrity verdict. This is tooling for future representative evaluation, not a labeled dataset, accuracy measurement, classifier or performance benchmark. No capture, participant attribution, sensor/UI integration, installation or live activation occurs.

### Offline acoustic lifecycle follow-up

The [offline acoustic session wrapper](acoustic-session.md) adds explicit stopped/active/faulted processing state around the existing chunk-to-repetition pipeline. Each activation returns a fresh opaque in-process permit; stale or foreign input cannot mutate a newer session. Stop or invalid active input drops processing/history state, and fault recovery requires explicit restart. Five unit tests and one public integration test cover activation, isolation, teardown, gaps and typed fault status. This prepares a future worker lifecycle but does not implement consent UI, source selection, capture indicators, PipeWire, worker queues or actual stream shutdown. Existing direct analyzers remain unchanged; nothing is installed or live-activated.

### Offline producer-failure follow-up

The [acoustic lifecycle](acoustic-session.md) now accepts permit-checked producer reports for source loss, format change and input overflow. A report from the current activation drops processing state, preserves a bounded producer-labeled fault reason and requires explicit restart. Delayed or foreign callbacks cannot stop or alter a newer activation; repeated reports while faulted do not overwrite the accepted reason. Four additional public integration tests cover all reasons, teardown/fresh warmup, stale callbacks with pending PCM, first-fault preservation and serialization compatibility. This is an offline failure contract, not actual source monitoring, worker supervision, reconnect logic or stream shutdown. No capture, installation or live activation is added.

### Offline output-lifecycle follow-up

The [acoustic lifecycle](acoustic-session.md) now offers `push_tagged` and consuming `accept_output` validation. Private `SessionOutput` envelopes retain activation identity so queued summaries from stopped/faulted or replaced activations cannot be accepted by the current processor; foreign sessions are rejected as well. Four public tests cover direct-batch equivalence, partial/gap/maximum metadata, both fault paths and stop/restart/foreign isolation. This is a point-in-time acceptance check, not revocation of caller-owned data: final delivery still needs controller synchronization, and bounded queues/order/expiry are future work. The original synchronous `push` API remains unchanged. No worker, capture, UI, bus integration, installation or live activation is added.

### Offline bounded-output follow-up

The [acoustic lifecycle](acoustic-session.md) now offers an owning `BufferedAcousticSession` with an eight-batch FIFO, including partial/gap metadata. Stop and accepted faults clear the backlog; the ninth successful processing push faults explicitly with output overflow and discards all pending results. Stale/foreign producers cannot mutate a full current queue, and current invalid input retains analyzer-fault precedence. Six public tests cover ordering, capacity, teardown and restart isolation. Bounds are logical, with nine transient batches during overflow, not a measured memory or real-time budget. Returned batches still need final-delivery synchronization with revocation. No input queue, TTL, worker, capture, runtime routing, installation or live activation is added.

### Offline output-health follow-up

The [bounded acoustic controller](acoustic-session.md) now exposes schema-versioned output health: current state/capacity, pending counts, lifetime processed/delivered/discarded batch and summary counts, output overflow events and retained queue high-water depth. Counts survive restart and saturate instead of wrapping. Overflow accounts for all nine discarded batches; empty-feature batches remain visible, while invalid/stale input is not counted as output. Four public tests and a saturation unit test verify conservation, lifecycle cleanup, rejected callbacks and serialization. Snapshots contain no features, identifiers or PCM and do not imply capture liveness, downstream receipt, lost-input counts or measured resource budgets. No capture, runtime sensor/UI integration, installation or live activation is added.

### Offline PCM conversion follow-up

The [acoustic foundation](acoustic-foundation.md) now provides bounded `decode_pcm_chunk` conversion for explicitly supplied little-endian interleaved signed 16/packed 24/32-bit and float32 bytes. It shares the four-frame chunk limit, rejects malformed/channel-incomplete input before allocation and rejects invalid floats without returning partial PCM. Five public tests cover scaling, sign/endian arithmetic, float validation, bounds and direct/decoded pipeline equivalence across three rates/channel counts. Returned PCM is caller-owned and not persisted or securely erased. This does not negotiate PipeWire layouts, resample, map OS timestamps, report conversion failures to a session automatically, verify a source, open a device, start a worker or enable capture. No installation or live activation occurs.

### Offline byte-lifecycle follow-up

The [owning acoustic controller](acoustic-session.md) now connects explicit raw PCM bytes through `push_bytes`. Permit validation precedes conversion/allocation; activation-bound rate/channel count is reused, while encoding remains caller-declared per call. Current conversion errors fault processing and discard/account for retained output without generating a processed batch. Valid input follows the existing timestamp, gap, FIFO and output-health path. Six public tests cover direct equivalence, partial/gap metadata, stale malformed callbacks at capacity, conversion-fault precedence, valid overflow accounting and format-isolated restart. Temporary decoded PCM is bounded and ordinarily dropped, not securely zeroized. This adds no source negotiation, capture, worker, runtime routing, installation or live activation.

### Offline encoding-contract follow-up

The [owning acoustic lifecycle](acoustic-session.md) now offers optional `start_encoded` to bind a byte encoding for an activation. `push_bytes` validates the permit before comparing encoding; a current mismatch reports the existing producer format-change fault, drops processing/backlog and requires explicit restart, without decoding or counting output. Ordinary starts remain unbound and normalized input remains available. Five public tests cover all encoding pairs, stale mismatches at capacity, error precedence, first-fault preservation, rejected-start isolation and bound/unbound restart. This checks producer declarations, not actual source metadata, permissions, PipeWire negotiation or bytes mislabeled with the expected encoding. No capture, installation or live activation is added.

### Offline lifecycle replay follow-up

The generated-only [acoustic lifecycle replay](acoustic-session.md) now exercises the public owning controller end to end through encoded byte input, partial/gap metadata, consumption, queue overflow, stale callbacks, source loss, format mismatch and explicit restarts/stop. It validates conservation and queue bounds at 26 deterministic checkpoints, emitting health/status only after successful checks. Three tests pin final accounting (14 processed/2 delivered/12 discarded batches), report shape and fault sequence; argument misuse produces no JSON. This establishes synchronous offline composition, not threaded handoff, capture cleanup, concurrency, timing/resource budgets or detection accuracy. No devices, installation or live activation are involved.

### Offline PCM layout follow-up

The [acoustic conversion helpers](acoustic-foundation.md) now support explicit borrowed per-channel offset/stride views through `decode_pcm_planes`, covering separate planar slices or shared interleaved buffers with padding. All plane ranges and checked address arithmetic are validated before allocation, with normalized interleaved output limited to four frames. Five public tests cover encoding/layout equivalence, bounds, overflow, truncation, ignored padding and invalid selected floats. Caller backing buffers are not size-capped or retained; this does not establish a total memory budget or validate unselected bytes. OS mapping/lifetimes, source permissions, layout negotiation and planar/session failure integration remain unfinished. No capture, installation or live activation occurs.

### Offline plane-lifecycle follow-up

The [owning acoustic lifecycle](acoustic-session.md) now accepts `push_planes`, connecting borrowed planar/strided PCM views to the same permit-checked conversion path as packed bytes. Inactive/stale checks precede optional encoding binding, followed by bounded conversion and normalized timeline/FIFO processing. Current conversion faults drop processing/backlog with discard accounting and no generated output; stale/foreign malformed views cannot alter a full current queue. Five public tests cover all encodings across three formats, partial/gap/warmup equivalence, fault precedence, overflow accounting and mixed-layout restart isolation. View descriptors are not retained; ordinary temporary PCM destruction is not secure erasure. Source validation, consent UI, OS buffer mapping, clock normalization, PipeWire, workers and runtime routing remain unfinished. No capture, installation or live activation occurs.

### Offline source-clock follow-up

The [acoustic foundation](acoustic-foundation.md) now offers immutable `SourceClockMapping` for explicitly supplied nanosecond clock readings. A backend bracket around a source anchor, caller-asserted rate-error bound and inclusive source-time horizon produce versioned earliest/latest backend intervals with outward rounding. Invalid anchors, backwards-from-anchor readings, expiry and overflow fail explicitly without state mutation. Five public tests cover arithmetic, boundary/error behavior, signed scaled-integer interval checks, serialization and exact-origin assembler composition. This is a conditional arithmetic contract, not measured synchronization: it does not read clocks, detect frozen/reset clocks, fit drift or propagate uncertainty into current point-valued features. Source/sample correlation, actual PipeWire clock mapping, suspend handling, liveness and runtime integration remain unfinished. No capture, installation or live activation occurs.

### Offline sample-clock interval follow-up

The [acoustic timing helpers](acoustic-foundation.md) now expose `SampleClockMapping`, composing an owned source-clock mapping with validated format and an explicitly supplied source time for sample position zero. Absolute sample offsets use wide rational arithmetic and outward floor/ceiling mapping, preserving clock intervals without cumulative rounding or choosing an exact timestamp. Both endpoints must satisfy source expiry and integer bounds. Five public tests cover fractional offsets, clock/rate uncertainty, multirate long-run rounding, expiry and overflow. This assumes ideal sample cadence in source-clock time; it does not measure device drift or establish the sample/source relationship. Intervals are not yet attached to processing outputs, activation identity or fusion. Capture, runtime clock-domain checks, consent UI and PipeWire remain unfinished; nothing is installed or live-activated.

### Offline timed-processing follow-up

The [acoustic timing pipeline](acoustic-foundation.md) now offers `TimedRepetitionFrameAssembler`, attaching sample positions and start/exclusive-end clock intervals to repetition output while explicitly naming existing nested timestamps as nominal sample-clock features. Chunk and timing validation precede processing, preserving partial PCM and comparison state on expiry, overflow or malformed input; gap/reset-of-comparison semantics and four-frame bounds are retained. Five public tests cover multiformat equivalence, repetition warmup, delayed gap recovery, atomic rejection and serialization. This remains a standalone normalized-PCM path with caller-asserted sample/clock assumptions, not permit-bound capture or measured synchronization. Lifecycle, byte/plane, fusion and UI integration, real clock/source validation and PipeWire remain unfinished. No devices, installation or live activation are involved.

### Offline encoded-timing follow-up

The [timed acoustic pipeline](acoustic-foundation.md) now accepts packed bytes and planar/strided views through `TimedRepetitionFrameAssembler::push_bytes` and `push_planes`. Both reuse bounded converters with the mapping's fixed format, then preserve the existing interval/nominal distinction and timing preflight. Whole conversion precedes timeline/clock validation; malformed tails and timing errors leave partial PCM and comparison state unchanged for a valid retry. Five public tests cover all encodings across three formats, complete-batch equivalence, gaps/warmup, error precedence, retries, history preservation, mixed layouts and bounds. Temporary normalized PCM is bounded and ordinarily dropped, not securely erased. This is not the permit-bound session path: source negotiation, lifecycle/encoding binding, capture, runtime fusion/UI and PipeWire remain unfinished. No installation or live activation occurs.

### Offline timed-lifecycle follow-up

The [timed acoustic lifecycle](acoustic-session.md) now offers `TimedAcousticSession`, binding a clock mapping and optional declared encoding to one fresh activation permit. Normalized, packed and planar input checks permits before processing; current frame/clock errors or producer reports drop processing and require explicit restart. Opaque tagged results require current-activation acceptance, preventing stale/foreign output acceptance after stop, fault or restart. Clock faults have a bounded source-labeled status variant; existing status forms are preserved. Six public tests cover batch equivalence, encoding checks, stale callbacks/results, fault teardown, serialization and restart warmup. This adds no bounded output queue or final-delivery synchronization; caller-retained outputs remain unbounded and accepted data cannot be recalled. Real source/clock verification, capture consent, PipeWire, worker and runtime fusion/UI remain unfinished. No installation or live activation occurs.

### Offline bounded timed-output follow-up

The [timed acoustic lifecycle](acoustic-session.md) now offers an owning `BufferedTimedAcousticSession` with an eight-batch tagged FIFO and shared versioned output-health accounting. Normalized, packed and planar input use the existing permit/encoding and clock-fault path; current input faults retain precedence at capacity and discard backlog without generating processed output. A ninth successful push faults explicitly and accounts for all nine discarded batches, with at most 36 transient summaries. Stop and accepted producer faults clear backlog; stale callbacks and rejected starts preserve current state. Five public tests and a saturation unit test cover FIFO/timing equivalence, partial/gap/warmup metadata, empty and maximum batches, error precedence, restart isolation and lifetime conservation. These are logical bounds, not measured resource budgets; popped batches remain caller-owned and final-delivery synchronization, TTL, worker, capture consent, PipeWire, source/clock verification and runtime fusion/UI remain unfinished. No installation or live activation occurs.

### Offline timed-lifecycle replay follow-up

The generated-only [timed lifecycle replay](acoustic-session.md) now exercises `BufferedTimedAcousticSession` through normalized, packed and padded strided PCM at 8.05 kHz. Thirty-one fixed health checkpoints check output conservation and bounds through fractional interval delivery, queued-partial clock expiry, restart/stale-callback rejection, maximum-batch overflow, producer loss, encoding mismatch and fresh mapping/analysis state. Three example tests pin health-only deterministic output, final totals (16 processed/3 delivered/13 discarded batches; 40/2/38 summaries) and fault/restart sequence. The executable rejects arguments without stdout and produces one report only after successful checks. Asserted mapping constants and synchronous generated input do not establish calibration, source/permission validation, concurrency, final-delivery synchronization, capture compatibility, resource budgets or detection quality. No devices, worker, runtime routing, installation or live activation are involved.

### Explicit PipeWire CLI capture prototype

The separate [capture command](acoustic-capture.md) adds an actual opt-in PipeWire sink-monitor path using installed `pw-dump`/`pw-record` tools. Output-only serial selection, explicit acknowledgement and a mandatory 1..60 second duration precede capture; ordinary injector startup is unchanged. Generated float32 frames flow through the untimed buffered controller and are immediately consumed/dropped, with foreground status and health-only final reporting. Signal/deadline/error paths terminate and reap the child; no automatic reconnect or default-target fallback is requested. Six synthetic/child-process tests cover consent, metadata filtering, bounded fragmented input, faults, timeout and teardown. No live recording was started during validation. A selected sink includes all routed applications, not verified Discord-only output; actual graph/format verification, calibrated timing, capture UI, runtime fusion, resource budgets and crash supervision remain unfinished. The full capture milestone stays unchecked; this is a CLI prototype, not validated protection.

### Capture startup and executable-supervision follow-up

The [capture CLI](acoustic-capture.md) now uses the same signal/deadline cancellation while discovering metadata and processing audio, and checks pending cancellation again before recorder spawn. This fixes a startup gap where a signal during discovery could otherwise be followed by recording starting. Six Linux executable tests substitute generated-only fake tools, send real SIGINT/SIGTERM after readiness notifications and verify prompt discovery cancellation, active shutdown, deadlines, invalid PCM/child-exit/stall failures, target rejection and consent gates. Known child PIDs are gone after exit; failures emit no success report. Node.js is required for these fixtures. No real PipeWire recording or Discord attachment was performed; live graph/source compatibility, crash cleanup, timing calibration, capture UI and protection remain unfinished.

### Capture lifecycle protocol follow-up

The [capture CLI](acoustic-capture.md#machine-readable-lifecycle) adds optional `--status-json` newline-delimited stderr records without changing default human output or the final success report. Versioned phases distinguish metadata discovery, recorder request, awaiting input, validated local frame delivery, stopping and terminal success/fault. Records contain only sequence, phase, bounded health and recorder-reaping confirmation, not audio features or target identifiers. A bounded writer with a 100 ms acknowledgement timeout makes closed/stalled status output a capture failure rather than an indefinite controller write. Five new executable tests cover schema/order, startup cancellation, preserved processing faults and closed consumers before/after recording starts; a unit test covers stalled-writer timeout. All inputs are generated, with no PipeWire capture or Discord attachment. This supports future UI integration but does not implement that UI, verify routing, classify every startup/cleanup failure or add crash supervision; the full capture milestone remains incomplete.

The protocol now uses status schema version 2 with typed terminal `reason` codes for signals/deadlines, discovery/source/launch failures, stalled input, EOF/read/processing/delivery failures, unexpected recorder exit and cleanup failure. Nonterminal reasons are null; stdout and nested health remain schema version 1. Typed error context, not diagnostic-text parsing, selects failure reasons. Generated executable tests cover active signal reasons, startup cancellation/deadlines, malformed metadata, missing tools, target rejection, EOF while the child remains alive, and the nondeterministic EOF/exit observation order. Cleanup failure never confirms reaping; failed status output can still prevent a terminal event. Some OS failure categories remain defined but not fault-injected. This advances the same CLI protocol foundation, not live routing verification or a completed capture UI.

### Explicit Linux prerequisite diagnostics follow-up

[src/platform.rs](../src/platform.rs) supplies a bounded read-only kernel prerequisite snapshot exposed by `intellicord diagnostics` without Discord attachment, and by the session-bound refresh control in Safety & Intelligence. Schema v2 reports BTF/cgroup metadata, unprivileged BPF and perf policies, effective BPF/perfmon/sys-admin capabilities, process-exec tracepoint metadata accessibility and lockdown state. The client accepts v1 reports with newer fields unavailable. Host requests are replay-protected and rate-limited; client timeout, close/disposal and session changes invalidate pending requests without retries. Invalid responses discard prior displayed reports. Preview responses are explicitly labeled fixtures.

Six parser fixture tests, two backend request-gate tests, two executable tests and client review tests cover malformed/oversized metadata, bounded report shape, inactive-sensor claims, replay/session/rate-limit behavior, timeout and late-response teardown. DOM absence assertions use scalar checks so failures cannot recursively format large LinkeDOM graphs and exhaust test-runner memory. The executable performs no load or attach test, changes no kernel settings and never starts a sensor. These prerequisites describe the backend's current context, not a future privileged loader's permissions. The optional Aya source/reader follow-up below advances program declarations and ring-loss accounting; privilege separation, sensor transport/clock normalization and live compatibility remain unfinished, so full Phase 3 telemetry remains unchecked.

### Optional Aya process observation follow-up

The [process observation prototype](process-observation.md) adds a separate Aya eBPF source crate for `sched_process_exec`, a shared fixed 24-byte native event ABI, and an opt-in `observe` feature with a bounded reader. The supplied kernel program filters one explicit numeric PID before writing to a 64 KiB ring and counts failed reservations per CPU. The reader validates size/schema/source/clock ordering, retains health only and requires a trusted object, acknowledgement and a 1..60 second duration. Generated replay exercises rejection/drop accounting without kernel access and is separately labeled in the client's explicit diagnostics response. It never routes process facts into participant evidence or enables client-driven live attachment.

Contract, executable consent-gate, backend response and client provenance tests are runnable without privileges. The optional Aya reader compiles; the separate kernel crate passes host typechecking/linting, not BPF-target compilation or verifier/live validation. Required nightly Rust/rust-src and BPF linker are absent in the development environment. The full Phase 3 telemetry prototype remains unchecked pending those checks. Numeric PID reuse, kernel setup deadlines, live cleanup, privilege separation, measured resource budgets, IPC and cross-process clock normalization remain limitations. No real process sensor was attached, privileged operation performed or Discord process modified.

### Offline process-object validation follow-up

The optional reader now offers `inspect --object PATH` and applies the same structured metadata check before loading capture bytes. It rejects wrong architecture/endianness, malformed symbol ranges, unexpected programs/maps, incorrect map sizes, pinning and flags before BPF syscalls. Accepted metadata declares one tracepoint, a 64 KiB ring, one target entry and one per-CPU drop-counter entry; this is not an instruction-level proof of behavior, record ABI, kernel permission or verifier compatibility. Objects must still be trusted and reviewed.

The explicit client diagnostics request reports inspector build availability and, only when the backend has `INTELLICORD_OBSERVE_OBJECT` configured, the trusted object's offline metadata status. The renderer cannot choose a file or request loading. Client validation rejects active/load-tested claims and clears stale session results; paths and metadata results do not enter participant evidence. Generated ELF and executable fixtures verify inspection and pre-load rejection without privileges. The standalone preview labels its result as a fixture. BPF-target compilation and live verification remain blocked/unperformed as described above.

### Process lifetime safety follow-up

The optional process reader now requires a pidfd-backed lifetime guard before BPF loading, rechecks it immediately before attachment, and polls it before each bounded collection batch. Visible target exit stops collection with `target_exited`; final draining is skipped or stopped when exit is visible. This advances fail-closed target lifecycle handling without changing the kernel event/map ABI. Kernel filtering remains numeric-TGID based: the guard does not verify an executable or eliminate selection, polling or PID-namespace races. Generation-bound in-kernel identity remains future work, and partial counters do not measure discarded tail records.

`intellicord-observe probe-target` and the client's explicit **Process target safety** diagnostics exercise only a self-process pidfd open/poll. Versioned reports contain no PID/path, never imply a selected target or running sensor, and distinguish unavailable syscall/policy from a disabled build or unsupported platform. Renderer-selected targets and sensor activation are not exposed. Client tests reject malformed/active claims and clear stale results; controlled child-process tests cover exit, forced termination, reaping and isolation from other processes without BPF privileges. These tests are not live sensor compatibility or cleanup validation. Full Phase 3 telemetry remains unchecked.

### BPF build pipeline follow-up

`scripts/build-ebpf.sh` adds an explicit build-and-inspect path for the real kernel source: prerequisite checks, dated nightly/rust-src, locked release compilation to `bpfel-unknown-none` with an explicit linker and target directory, then userspace inspection of that exact artifact. Missing tools or failed compilation/inspection stop the pipeline; no host-build or stale-object fallback is used. It installs nothing and performs no BPF load, attachment or Discord activation. The existing backend-controlled object setting lets the client inspect a reviewed local build without adding renderer file paths or sensor activation.

`.github/workflows/ebpf-build.yml` configures the same pipeline on a read-only Ubuntu job with pinned compiler choices, a SHA-256-verified linker release, bounded test concurrency and build-only object/metadata artifacts. Five generated tool/YAML tests exercise command arguments, prerequisite/error gates and CI configuration without a BPF toolchain. The local environment still lacks rustup/rust-src/linker, and the workflow has not been dispatched or observed passing here. Real BPF code generation, verifier acceptance and live compatibility therefore remain unverified; full Phase 3 telemetry remains unchecked. A future passing target build establishes compilation and declared layout only, not safe kernel operation.

### Process sensor lifecycle follow-up

The optional process reader now offers a bounded versioned NDJSON status protocol through a trailing `--status-json` on explicitly acknowledged capture. Typed phases/reasons distinguish validation, loading, attachment request, confirmed observation, stopping and terminal success/fault. Attachment confirmation follows successful attach; detachment confirmation is conservative and never asserted on faults. A one-slot writer with a 100 ms acknowledgement timeout prevents blocked status output from indefinitely delaying the controller; owned unpinned handles remain the error cleanup boundary. Output failure can prevent terminal delivery, and synchronous kernel setup remains outside the collection duration bound.

`replay-lifecycle` exercises the schema without opening a process or BPF handle. The client's existing session-bound explicit diagnostics display the same generated sequence with no kernel-operation confirmations, reject invalid ordering/provenance/active claims, and keep it outside participant evidence/export. Transition, bounded-writer, executable startup-rejection and client tests are unprivileged. No renderer activation, live status stream, external supervisor, heartbeat, privileged service or kernel ABI change is introduced. Actual post-attach lifecycle/cleanup, target compilation and live compatibility remain unverified; full Phase 3 telemetry remains unchecked. See the [process lifecycle contract](process-observation.md#machine-readable-lifecycle).

### Process status receiver follow-up

A bounded incremental `StatusStream` receiver now validates the process reader's NDJSON contract with explicit source/exit assumptions, strict fields, ordered transitions, monotonic health, operation-confirmation checks and permanent rejection after malformed input. EOF alone is never success: a complete terminal record and consistent process exit are required. The optional `inspect-status` stdin command exercises this path without launching a collector or accessing BPF. Memory/input bounds do not provide a wall-clock deadline, process/session binding, transport authentication or actual sensor supervision.

The client's explicit diagnostics run eight generated acceptance/rejection cases and expose passed/failed protocol checks separately from participant evidence. Invalid or active claims and stale-session results are rejected, while an actual failed generated check remains visible. Receiver unit tests and executable replay/startup-rejection round trips are added to the build-only BPF validation path. This advances a testable consumer boundary for a future supervisor without changing the kernel ABI, enabling renderer activation, or claiming trusted kernel facts from supplied text. Target compilation, verifier acceptance, live cleanup, privilege separation and full Phase 3 telemetry remain unverified or unfinished. See [offline status validation](process-observation.md#offline-status-validation).

### Process collector supervision follow-up

An owning asynchronous supervisor now combines bounded status parsing with direct-child exit/wait, cancellation, deadline and kill/reap handling. It discards child stdout, requires terminal status plus EOF plus consistent exit, and never confirms reaping without a successful wait. Malformed input, stalled collectors and terminal-but-running children fail closed. Cleanup has its own two-second bound; task abortion retains kill-on-drop fallback, not guaranteed reaping. Process trees, privileged services, authenticated IPC, crash cleanup, resource containment and actual BPF removal remain outside this prototype.

`intellicord supervise-replay` and the client's explicit diagnostics run only a fixed same-executable generated child, with no paths, PIDs or arguments supplied by the renderer. One background task avoids blocking voice processing and bounds concurrent launches. The client separates generated completion/fault/deadline outcomes from process-reaping confirmation and rejects live-sensor claims. Generated Node subprocess fixtures verify awaited cancellation/deadlines, early EOF, fragmentation, malformed/truncated output and known-child reaping; an optional-feature test covers actual reader rejection before BPF loading. CI configuration includes these tests. No real process sensor or Discord modification was activated; BPF-target compilation, verifier compatibility and live cleanup remain unverified. This advances testable external-collector ownership, not completion of the full supervision/IPC/clock-normalization roadmap item. See [supervision limits](process-observation.md#supervised-generated-collector).

### Offline object inspection isolation follow-up

The client's explicit diagnostic request now moves configured sensor-object parsing into a fixed same-executable offline helper, running alongside the generated replay in the existing single asynchronous task. `intellicord check-object` exposes the same bounded JSON protocol, two-second inspection deadline and separate two-second direct-child cleanup budget. The helper uses the optional reader's metadata inspector, not its capture path; it accepts no renderer-selected object, executable, target or activation arguments. Strict inactive-source/status validation, EOF/exit agreement and awaited reaping gate the client result. Layout rejection remains distinct from helper failure. Default builds report the inspector unavailable, while preview reports remain labeled fixtures.

Generated ELF and subprocess tests cover metadata matching/rejection, oversized/malformed replies, nonzero exits, stalled helpers and valid EOF without process exit. This advances testable client diagnostics and keeps parsing out of voice-event handling; it does not add BPF loading, verifier proof, a privileged loader, authenticated IPC, process-tree containment or abort-time reaping guarantees. The full supervision/IPC/clock milestone remains open. See [offline helper boundaries](process-observation.md#supervised-offline-object-check).

### Kernel clock-domain follow-up

The shared process ABI now explicitly names `linux_monotonic`, matching `bpf_ktime_get_ns` without changing the 24-byte record layout. A bracketed userspace `CLOCK_MONOTONIC` anchor maps source timestamps into backend `Instant` intervals using the existing checked interval arithmetic, with before-anchor/expiry/overflow rejection. A one-second source-time horizon and assumed 100 ppm relative rate bound are explicit provisional parameters, not measured synchronization or liveness guarantees.

`intellicord probe-clock` and the client's explicit **Kernel clock diagnostics** expose a local read's bracket width, feature availability and timing assumptions without opening a process, starting a sensor or loading BPF. Deterministic ABI/mapping tests, a real local clock-read overlap check, executable argument/privacy tests and client provenance/session tests cover the contract. Same-host/boot/time-namespace identity, receipt freshness, cross-process exchange, suspend/reset handling, drift calibration and fusion integration remain future work; the full supervision/IPC/cross-clock milestone stays unchecked. See [clock boundaries](process-observation.md#kernel-clock-diagnostics).

### Timed process-reader follow-up

A bounded `TimedProcessMonitor` now composes shared kernel-ABI decoding with clock-interval mapping, receipt/source ordering, a receipt-based anchor deadline and conservative event-age limits. Invalid, foreign, expired, future and stale inputs have separate saturating counters; rejected input cannot poison accepted watermarks. Returned intervals are clipped to receipt time, not collapsed to fabricated exact timestamps. Equal timestamps remain allowed and do not imply deduplication. Receipt epochs and source identity remain trusted caller inputs; timing thresholds are provisional rather than calibrated.

`intellicord replay-timed-process` and the client's explicit diagnostics exercise nine generated success/rejection cases and expose synthetic counters plus visible failed checks without loading BPF. Unit, public-API and executable tests cover exact boundaries, stale uncertainty, frozen source expiry, counter saturation and inactive/argument contracts; build-only CI includes them. The real ring reader and fusion are unchanged. This advances a testable kernel-event consumer, not live correlation, authenticated IPC, process-generation binding, synchronization or completion of the full sensor milestone. See [timed process boundaries](process-observation.md#timed-process-replay).

### Joint event/lifecycle supervision follow-up

The [joint collector transport](process-observation.md#joint-collector-transport) now combines bounded native event and NDJSON lifecycle validation with concurrent pipe draining, backend `Instant` receipt times, successful exit and exact terminal accepted-count agreement. A missing whole record, malformed input in either pipe, incomplete tail, exit disagreement or stalled child cannot produce a successful collection summary; awaited failures use the existing direct-child kill/reap path. Ring reservation drops remain distinct from received-record counts. Zero-record collections remain unsupported.

The fixed `supervise-replay` diagnostic now runs separate status-only and joint generated children concurrently; the client shows joint completion/failure, cleanup and matched counts without renderer-selected commands or live activation. Synthetic source clocks still require declared mapping assumptions, and real receipts may legitimately fail the provisional freshness policy on a slow host. Tests cover generated byte fragmentation, omissions, bounded input, one/both EOF stalls, cancellation and reaping, plus client schema/provenance/count/cleanup guards and old-report compatibility. This advances a testable sensor transport consumer, not authenticated IPC, calibrated cross-process synchronization or live ring export. Kernel ABI, real reader and participant fusion are unchanged; actual BPF-target/verifier/live validation and full Phase 3 remain open.

### Phase 1 — Foundation and event fidelity (highest priority)

The subsequent [explicit event-export follow-up](process-observation.md#explicit-event-export) adds opt-in `capture ... --events --status-json` to the optional reader. A shared bounded acknowledged writer validates native ABI records, caps export at 256 events and faults on output failure rather than mixing binary bytes with a JSON summary or claiming successful truncation. The existing generated joint client diagnostic now uses that producer path; tests cover byte fidelity, input/output rejection, budgets and executable consent/pre-load gates without kernel access. Kernel layout is unchanged, and live export, verifier acceptance, post-attach cleanup and a trusted clock handshake remain unverified or unfinished. Default capture stays health-only, and no renderer-driven live activation is added.

The subsequent offline event-transport adapter now buffers fragmented native ABI input into the timed reader with a one-record pending buffer, a 256-record budget and permanent rejection after malformed input. Finite `inspect-events` stdin validation and eight client-visible generated transport checks cover completion, truncation, receipt ordering, freshness, foreign input and exit assumptions. This is not authenticated IPC or a live sensor transport: receipt/exit assertions remain caller supplied, EOF cannot detect omitted whole records, and live lifecycle/drop accounting and external deadlines remain open. See [offline event transport](process-observation.md#offline-event-transport).

- [x] Define a versioned observation envelope and subject references (v1 prototype above).
- [x] Introduce a bounded synchronous fusion log in Rust with retention/drop counters.
- [x] Add a bounded typed in-process sensor bus with independent publishers, replay guards, overflow metrics, and fusion integration.
- [ ] Add external sensor supervision, IPC, and cross-process clock normalization when sensors are introduced.
- [x] Mark snapshot provenance and known reconciliation/capture gaps explicitly.
- [ ] Capture higher-fidelity transitions beyond sampled store snapshots.
- [x] Implement experimental rapid re-entry and burst grouping rules.
- [ ] Implement unusual group-movement rules and configurable/calibrated community baselines.
- [x] Expose explanation and uncalibrated confidence metadata in a basic incident view.
- [x] Create synthetic benign and disruptive-pattern event fixtures.

**Exit criteria:** no fabricated joins/leaves; exact IDs preserved; event gaps visible; incident rules reproducible under tests.

### Bounded clock handshake follow-up

The [clock handshake](process-observation.md#bounded-clock-handshake) adds a fixed sensor-free helper and a bounded ready/request/reply exchange before constructing the existing timed event receiver. Readiness excludes process startup from the bracket; strict schema/domain/nonce/framing checks, a 25 ms bracket policy, EOF/exit agreement, a two-second deadline and awaited direct-child cleanup fail closed. Anchors expired before handoff are discarded. The optional Linux helper reads `CLOCK_MONOTONIC`; default builds report disabled without spawning. No kernel event ABI change, BPF load, target selection or renderer activation is added.

The client's explicit diagnostics now show the exchange outcome, bracket and cleanup separately, reject contradictory/active/malformed claims, and clear stale results. Namespace identity remains unverified, live collector handoff remains unconnected, and preview values are labeled fixtures. Generated subprocess, clock-contract, executable and client tests cover rejection, cancellation, expiry and cleanup without privileges. The nonce is not authentication; host/boot/time-namespace trust, measured drift, actual BPF timestamp comparison and renewal beyond the existing one-second anchor remain unresolved. BPF-target compilation, verifier/live compatibility and the full cross-process normalization milestone stay unverified or unfinished.

### Same-child negotiated transport follow-up

The [negotiated collector supervisor](process-observation.md#negotiated-collector-transport) now retains one child and one stdout pipe across bounded clock negotiation and binary event delivery. It constructs the existing timed collection from the validated reply, preserves coalesced trailing event bytes, and applies cancellation and one overall deadline to both stages. Invalid negotiation or collection yields no successful summary; direct-child reaping remains separately confirmed. Pre-anchored and status-only supervisor APIs retain their behavior. The 24-byte kernel ABI, record budgets and existing short-lived timing policies are unchanged.

The explicit client diagnostics add a fixed generated child with synthetic source readings, native ABI records through the production exporter, and matching lifecycle health. Optional client rows expose negotiated completion/failure, cleanup and matched counts without implying kernel capture, and reject active/provenance/count contradictions or stale sessions. Generated subprocess tests exercise delayed readiness, combined reply/event output, malformed nonce, truncated events, failed exits, stalls, cancellation and known-child cleanup; executable and guard tests cover bounds and forbidden arguments. No renderer-selected commands, BPF operation, Discord attachment or audio capture is added. Real reader clock-preamble support, source/namespace authentication, anchor renewal, least-privilege separation and BPF target/verifier/live validation remain unfinished or unverified, so the full normalization and Phase 3 milestones remain unchecked.

### Negotiated reader export follow-up

The optional reader now implements the [clock-preamble producer](process-observation.md#negotiated-reader-export) behind explicit `--clock-handshake --events --status-json` and `--milliseconds 1..500`, retaining object/PID/consent gates. Negotiation follows validation and BPF setup but precedes attachment; bounded response failure or cancellation prevents attachment, with signal and pidfd rechecks before attach. Ordinary capture formats remain unchanged. The one-shot responder has its own two-second deadline; blocked I/O threads are not forcibly cancelled or joined and rely on CLI process exit. The short duration leaves nominal timing margin but is not a guarantee of accepted timestamps or hard real-time shutdown.

Both the generated client replay and the optional reader's new `replay-negotiated` role use the same bounded responder/exporter without BPF. The client exposes the request deadline as optional validated metadata while retaining synthetic provenance. Unit and executable tests cover cancellation, rejection, stalled open stdin timeout, actual-reader generated count agreement/reaping, consent/duration/flag-order gates and invalid-object rejection before a preamble. No live sensor, Discord attachment or audio capture was started. Actual BPF target/verifier/live timestamp compatibility, namespace authentication, anchor renewal, privilege separation and live cleanup remain unverified or unfinished; full Phase 3 and cross-clock milestones stay unchecked.

### Quiet process collection follow-up

The [joint collection receiver](process-observation.md#joint-collector-transport) now accepts a zero-byte event stream only with a complete stopped lifecycle, zero terminal accepted count and successful exit. The owning supervisor still requires both EOFs and awaited child reaping. Partial bytes, nonzero count disagreement, failed exit and malformed/truncated status remain failures; standalone event inspection still requires at least one record. This supports short process-exec windows in which no event occurs without weakening the native ABI or timing validation of delivered records.

The client accepts validated zero totals and exposes an optional generated **Quiet collection replay** result, including visible failed checks and strict boolean validation. Generated subprocess tests cover quiet pre-anchored and negotiated captures, missing/partial events, failed exits and known-child cleanup. Quiet completion is a transport result, not evidence that a tracepoint fired, timing compatibility was exercised, or no activity/loss occurred; rejection and ring-drop counters remain separate. No BPF load, Discord attachment, audio capture, renderer activation or kernel ABI change is introduced. BPF-target/verifier/live validation and full Phase 3 remain unfinished or unverified.

### Shared kernel selection follow-up

The process tracepoint now uses an inline `no_std` target-selection helper shared with host contract tests and the generated `replay-process-selection` command. The helper selects exact nonzero TGIDs, preserves the separate nonzero TID, rejects a matching TID under a foreign process, and filters invalid zero-thread identities before reserving ring space. Event/map ABI and capacity remain unchanged; ordinary valid kernel identities retain the prior selection behavior.

The client's explicit diagnostics expose seven ordered generated checks with visible failure names, strict optional-list validation and older-backend compatibility. Tests verify identity boundaries, report bounds/privacy, forbidden CLI arguments and client pass/fail/malformed handling. See [shared kernel selection](process-observation.md#shared-kernel-target-selection). These host checks exercise the actual shared decision but do not validate BPF code generation, verifier behavior, helper runtime values, PID namespaces/generations, attachment or cleanup. No BPF loading, Discord attachment or audio capture occurs; full Phase 3 remains unfinished or unverified.

### Backend execution-context diagnostics follow-up

The explicit Linux prerequisite report now uses schema v3, adding nullable no-new-privileges and a bounded seccomp mode from the same size-limited main-thread status read as capabilities. Strict independent parsing keeps missing/duplicate/malformed fields unknown without hiding neighboring valid facts. The client displays the backend main-thread context, preserves schema-v1/v2 compatibility, and rejects incomplete or mistyped v3 fields while clearing stale results. Fixture and executable tests cover valid modes/flags, unknown input, bounds, report shape and client lifecycle handling.

See [execution-context diagnostics](process-observation.md#backend-execution-context). These facts help identify restrictions relevant to a future sensor launcher but do not reveal seccomp policy, describe every backend thread, authorize BPF, or establish least-privilege containment. No credentials, policies, kernel settings, BPF program or Discord process are modified. Full containment, privilege separation and live telemetry validation remain unfinished or unverified; their roadmap checkboxes remain open.

### Shared producer control-flow follow-up

The process tracepoint now calls an inline, statically dispatched `no_std` producer shared with host tests and `replay-process-producer`. The shared code filters before attempting submission and requests one drop notification per failed selected submission. Aya-specific reservation, timestamps, event writes and saturating per-CPU map accounting stay in the kernel adapter; event/map ABI is unchanged.

A fixed two-slot generated sink exposes six checks covering identity preservation, full-capacity filtering/rejection, drain recovery and accounting agreement through the existing explicit client diagnostics. Reports are bounded, inactive and excluded from participant evidence; malformed reports clear diagnostics, failed checks remain visible, and older backends remain compatible. This validates shared control flow, not byte-accurate kernel ring behavior, map updates, verifier acceptance or live attachment. See [producer replay contracts](process-observation.md#shared-kernel-producer-replay). BPF-target and live validation remain unverified; this does not complete privileged sensor integration.

### Shared loss-accounting follow-up

Kernel per-CPU increments and both live-reader aggregation sites now call shared `no_std` saturating arithmetic, also used by the generated producer sink. Six additional `replay-process-producer` checks cover counter/aggregate overflow, ordinary and empty sums, production-monitor high-water snapshots, filtering at saturation and successful submission after draining. Only named boolean outcomes are serialized for full-width boundary fixtures; the complete report remains below 1 KiB.

The existing explicit client diagnostics expose accounting source, pass totals and failed case names, including all-failed results, with bounded ordered validation, older-report compatibility and session clearing. No activation path or event/map ABI change is introduced. These are shared arithmetic/control-flow checks, not kernel map-update, atomic-snapshot, CPU availability, verifier or live-compatibility proof. Saturated totals are lower bounds; zero loss is not a completeness guarantee. See [loss accounting contracts](process-observation.md#loss-accounting-contract). Privilege separation, BPF-target validation and live telemetry remain unfinished or unverified.

### Shared record-construction follow-up

The actual kernel ring writer and generated timed/exported transport records now use the same inline `no_std` event constructor. A safe native-endian encoder and compile-time 24-byte/field-offset assertions make layout drift testable without kernel access; schemas and maps are unchanged. The sensor-free `replay-process-record` reports four named checks for independent native-byte fixtures, full-width field preservation, unaligned decoding and malformed/foreign record rejection in less than 512 bytes, without fixture identities, timestamps or raw records.

Existing client diagnostics expose optional strict record-contract results, preserve failed checks and older-backend compatibility, and clear malformed/stale reports. Exporter, receiver and owned-subprocess replays now consume shared-constructor records. These checks cover host construction and consumer composition, not native-endian portability, authentication, BPF code generation, helper values, verifier acceptance or live delivery. See [record contracts](process-observation.md#shared-kernel-record-contract). No sensor is activated; BPF-target/live validation and full privileged sensor integration remain unverified or unfinished.

### Shared reservation-submission follow-up

The actual Aya ring writer now uses an inline, allocation-free shared submission routine that samples the clock and constructs/submits one event only after successful reservation. A generated byte-storing sink exercises the same ordering through the production selection/drop path. Six additive producer checks cover filtered input, ordering, single submission, independent native-byte expectations, full-ring preservation and fresh recovery; direct tests additionally cover full-width fields, zero timestamps and callbacks that must not execute on reservation failure. Kernel maps and the 24-byte event ABI are unchanged.

Existing explicit client diagnostics display optional reservation-source, pass-count and failed-case rows, preserve failed results and older backends, and reject malformed or stale reports. The complete producer CLI report stays below 1536 bytes without identities, timestamps or raw records; the preview remains a fixture. The BPF build workflow now includes the reservation integration tests. Host release typechecking is not BPF code-generation/verifier proof or live ring delivery, helper, atomicity or cleanup validation. The explicit target-build gate still requires the missing local toolchain; no real sensor or Discord process was activated. See [submission contracts](process-observation.md#reservation-submission-contract). Full Phase 3 telemetry remains unchecked.

### Compiled BPF inventory follow-up

The installed pinned nightly/rust-src/linker now build the actual `bpfel-unknown-none` sensor successfully through the existing build-and-inspect gate. Structured inspection reports a bounded instruction inventory from every parsed function: function/slot totals, expected helper categories, other helpers, local calls and other call forms. The local artifact has 100 instruction slots across four functions and the expected map lookup/clock/identity/ring helpers; these are observed compiler outputs, not portable instruction-count guarantees. Earlier missing-toolchain notes above describe historical milestones and are superseded by this successful local target build.

The existing isolated object helper carries optional inventory only for a matched layout. Explicit client diagnostics expose offline code/call-site counts with strict schema, bounds, count-consistency and session validation, no renderer-selected object or activation, and no participant evidence/export routing. Independent ELF fixtures cover helper, local and other call encodings. An explicit real-artifact test checks required helper presence and bounded code without kernel access; CI now runs it after compilation, although the workflow itself has not been dispatched here. The preview remains a fixture. See [compiled code inventory](process-observation.md#compiled-code-inventory) for reproducible commands.

This establishes actual target code generation and offline code inspection, not reachability, verifier acceptance, safe helper arguments, authentic source behavior, live ring delivery or cleanup. No BPF program was loaded, no privileged operation performed and no Discord process modified. Full Phase 3 telemetry remains unchecked pending controlled live validation, privilege separation and integration.

### Explicit load-only verification follow-up

The optional Linux reader now provides `verify-load --object PATH --acknowledge-bpf-load` as an operator-only step between offline compilation and future attachment tests. After bounded offline object checks, it creates temporary maps, calls the tracepoint program's load operation, drops owned handles and returns typed stage results. It has no target selection, attach, capture or pin operation, and does not change host permissions/settings. Successful loading would establish acceptance in that specific context, not live delivery or cleanup; the synchronous kernel operations have no controller deadline.

Client diagnostics expose optional build availability and explicitly CLI-only activation without starting the probe or accepting a load result as trusted evidence. Existing **Object load and attach** remains **Not tested**, older backends remain compatible, and malformed/contradictory availability clears stale results. Executable consent/invalid-object tests and real compiled-artifact preflight with injected loader outcomes run without BPF syscalls; build-only CI includes the latter after compilation. No valid-object kernel load was invoked here. Actual verifier acceptance, attachment and live cleanup remain unverified. See [load-only verification](process-observation.md#explicit-load-only-verification) for the operator command, report fields and privilege boundaries. Full Phase 3 telemetry remains unchecked.

### Attach-only verification and client replay follow-up

The operator reported `verified` from the privileged load-only command on Linux 6.18.58-1-lts, with map setup/program loading successful and attachment explicitly false. This supersedes earlier untested-verifier status for that object/host only; no live collection or cleanup claim follows.

The optional reader now provides a separate `verify-attach --object PATH --acknowledge-bpf-attach` gate. The reviewed object is preflighted before map setup, loaded with no target write, and required to read back `TARGET[0] == 0` before a brief real tracepoint attachment and immediate explicit detach. Owned handles are dropped before the bounded typed result. Failure stages remain distinct and detach failure is never reported as confirmed cleanup. This command has not been run against a valid object here and is never invoked by client diagnostics or CI.

The real adapter and sensor-free generated driver share an ordered lifecycle controller. Six checks cover each failure boundary, the disabled target, operation confirmations and destruction before reporting. `intellicord replay-process-attachment` and the existing explicit client diagnostics expose these generated checks, including visible failed cases, separately from live attachment status and participant evidence. Older reports remain compatible, invalid/active reports and stale sessions clear results, and preview values remain fixtures. Object ABI and kernel code are unchanged. Synchronous kernel-operation deadlines, privilege separation, actual event delivery, crash/live cleanup and full Phase 3 telemetry remain unfinished. See [attach-only verification](process-observation.md#explicit-attach-only-verification).

### Controlled child delivery follow-up

The operator subsequently reported successful disabled-target attach-only verification, including explicit detach, on Linux 6.18.58-1-lts. This supersedes earlier untested-attachment status for that object/host, not event delivery or collection cleanup.

The optional reader now offers `verify-delivery --object PATH --acknowledge-bpf-delivery`. It preflights the object, starts a fixed same-executable waiting helper, targets only that child, attaches, releases a known exec and validates exactly one leader-exec ABI record in a local monotonic release/receipt bracket. Success also requires child exit success, zero reported ring reservation drops, target clearing, explicit detach and owned-handle disposal. The child stays unreaped until filter teardown to avoid numeric PID recycling while the filter is active. Readiness/delivery/reaping have separate bounded asynchronous waits, but synchronous kernel operations remain outside those deadlines. No external PID/executable or renderer activation is accepted. This real delivery probe was not run during implementation.

The client and `intellicord replay-process-delivery` expose seven generated checks of the same record predicate without BPF or child spawning. Failures remain visible, live claims/invalid reports/stale sessions are rejected, and older backends remain compatible. Helper release/EOF/error and actual waiting-child cleanup tests are unprivileged; they do not prove live ring delivery or kernel cleanup. Diagnostics remain outside participant evidence/export. The existing build-only CI includes these tests, never a valid-object probe. No kernel ABI change is needed. Full Phase 3 telemetry remains unchecked pending live delivery validation, privilege separation, supervision integration and crash/resource guarantees. See [controlled child delivery](process-observation.md#controlled-child-event-delivery).

### Supervised capture health handoff follow-up

The operator reported `verified` from the controlled-child delivery probe on Linux 6.18.58-1-lts, including matching event validation, child reaping, target clearing and explicit detach. That supersedes earlier untested-delivery status for this object/host only, not sustained observation, overload behavior or crash cleanup.

The existing supervisor now preserves optional terminal health only after status/EOF/direct-child-exit validation. A separately acknowledged `intellicord-observe supervise-capture` command launches only its own existing health-only capture mode with a bounded lifetime, signal cancellation and awaited kill/reap handling. No automatic elevation, new listener, arbitrary collector executable or renderer activation is added. Real valid-object supervised capture remains untested. Numerical target lifetime/namespace and forced-cleanup limits remain those of the reader/supervisor.

For a testable client handoff without a privileged IPC service, an operator may explicitly save that final report and configure the ordinary backend's `INTELLICORD_OBSERVE_HEALTH_REPORT`. The explicit diagnostics job reads a bounded nonsymlink regular file off the voice-processing path, accepting completed detached/reaped reports only. `intellicord check-health-snapshot` exercises the same read without attachment. The client independently checks shape, source, operation consistency and safe counters, and labels provenance as operator-supplied historical data with unknown freshness. Invalid files/responses and stale sessions clear results; older backends may omit the report. Preview data is a labeled fixture, and no snapshot enters participant evidence/export or claims current sensor liveness. This deliberately does not implement authenticated continuous live-health IPC, a privileged loader, cgroup containment or complete Phase 3 telemetry. See [health handoff boundaries](process-observation.md#supervised-capture-health-handoff).

### Isolated health inspection follow-up

Configured report reads now execute in a fixed same-executable helper, with a 4096-byte reply limit, strict report validation, a two-second reply/EOF/exit deadline and a separate two-second kill/reap wait. Stalled output, valid output without exit, oversized/malformed replies, active-sensor claims and nonzero exits fail closed without changing the client snapshot schema. Default builds still report configured snapshots as unavailable. Executable tests cover the actual helper route and report-file round trip; generated child tests cover deadline and cleanup paths without starting a sensor. Synchronous spawn, uninterruptible filesystem I/O, task abortion and descendant cleanup retain explicit limits. This improves bounded diagnostic isolation, not authenticated live health or privilege separation. See [health handoff boundaries](process-observation.md#supervised-capture-health-handoff).

### Health inspection failure diagnostics follow-up

The supplied snapshot now distinguishes report rejection, a timed-out but reaped helper, other inspection failures and unconfirmed helper cleanup. These statuses use the existing five-field envelope, carry no report on failure and clear previously displayed counters. The client validates each inactive/null-report combination and keeps inspection cleanup distinct from collector detach or live sensor status. Subprocess tests exercise actual timeout/nonzero/malformed paths; deterministic tests pin cleanup-uncertainty precedence without claiming a real unkillable child was reproduced. Older clients fail closed by omitting unknown snapshot statuses. This adds diagnostic visibility only, not a sensor activation path or authenticated live health.

### Bounded progress protocol follow-up

A separate session-bound progress envelope now accepts periodic health samples only during the observing phase, reusing strict lifecycle and nonregressing-counter validation without changing the existing reader protocol. Bounded incremental parsing, sequence/session checks, caller-clock receipt ordering, age-limited latest-sample access and terminal/exit agreement establish a testable consumer boundary for future continuous health. A fixed generated child emits six lifecycle records and two samples; executable tests consume its actual EOF and exit. Eight generated acceptance/rejection checks appear in explicit client diagnostics, clearly separated from live delivery and participant evidence. The binding is not authentication; timers, peer credentials, production supervision, live reader emission and trusted clock/session ownership remain future work. See the [progress protocol contract](process-observation.md#bounded-progress-protocol).

### Owning progress supervision follow-up

An asynchronous owner now connects a direct child's stdout to the bounded progress receiver with parent-measured receipt times, total and validated-record idle deadlines, cancellation, EOF/exit agreement and a separate bounded kill/reap wait. Partial byte dribbles cannot keep the collector alive; terminal output without exit is not completion. Coherent faults retain validated summaries, while failed/invalid/cancelled runs do not. `intellicord supervise-progress-replay` owns only a fixed same-executable generated child and returns explicitly generated, inactive diagnostics. Real child tests cover cancellation, progress/idle behavior, total lifetime, faults and reaping. This establishes process ownership around the consumer, not a live client subscription, authenticated transport, real reader progress emission, kernel cleanup proof or privilege boundary. See [owning progress supervision](process-observation.md#owning-progress-supervision).

### Revocable health subscription follow-up

The progress owner now offers an optional bounded in-process latest-health channel. Samples carry caller-declared source provenance, retain parent-clock receipt age, and are exposed only while observing after a whole read chunk validates. Slow consumers coalesce updates; partial bytes do not refresh sample age. Failure clears health before cleanup, and terminal status, completion, cancellation or task abort revoke retained values. Receivers must recheck age on their own timer and must not treat interim samples as final success or authenticated liveness. Generated subprocess tests exercise the handoff without sensors. Real reader periodic emission, independent session selection, authenticated IPC and client subscription remain unconnected. See [subscription boundaries](process-observation.md#revocable-in-process-health-subscription).

### Validated progress producer follow-up

A reusable encoder now assigns progress envelope sequences and validates serialized lifecycle/health records through the same bounded receiver before returning bytes. The existing generated child uses it, while independent-session tests deliver sixty samples through a fragmented round trip and verify summary agreement. Invalid records permanently reject the encoder; terminal counters must remain consistent with earlier samples. Encoding is not delivery: callers still need bounded output handling, independent session selection and observed child exit. No real reader emission or sensor activation is added. See [producer boundaries](process-observation.md#validated-progress-producer).

### Deadline-bounded progress output follow-up

An optional asynchronous writer now validates progress envelopes and applies one 100 ms deadline across each record's write and flush. Failures or cancelled in-flight operations permanently disable output; no retries or queued records can turn a partial stream into successful completion. The generated child exercises this writer without changing its wire fixture. Unit tests cover backpressure, write/flush stalls, partial errors and cancellation; executable tests check actual stdout success and Linux output failure. These are cooperative async bounds, not guaranteed interruption of underlying blocking I/O: Tokio stdout and runtime shutdown still require the parent's lifetime/cleanup supervision. Real reader periodic emission, session ownership, authenticated IPC and client delivery remain unconnected. See [output boundaries](process-observation.md#deadline-bounded-progress-output).

### Opt-in periodic reader emission follow-up

The optional Linux reader now has a distinct `capture-progress` command requiring an explicit object, PID, bounded duration, caller-selected session and `--acknowledge-process-progress`. Its lifecycle-aware reporter sends session-bound progress to stdout and samples current monitor health at most once per second during observation, with no catch-up bursts or separate task. Legacy capture/export/handshake formats remain unchanged. Unit tests cover cadence and failure semantics; real executable tests use invalid requests and `/dev/null` to verify preload faults, output failure and parent reaping without BPF activation. This implements an opt-in producer path, not verified live compatibility, authenticated transport, independent session negotiation or renderer delivery. A valid-object run was not performed. See [periodic capture boundaries](process-observation.md#opt-in-periodic-capture).

### Parent-owned periodic capture follow-up

A dedicated `supervise-progress-capture` command now selects an OS-random 128-bit session binding, launches only its fixed same-executable periodic capture child and validates the expected binding with total/idle deadlines, signal cancellation and awaited cleanup. Distinct consent and exact argument gates reject caller-selected sessions or executables. Its final report exposes the typed supervisor result without raw bindings, target paths/PIDs, interim samples or an unconditional sensor-state claim. Unit tests check session selection; executable tests exercise strict gates and actual minimum/maximum-duration preload faults with `/dev/null`, never a valid BPF object. Session binding is not authentication, and setup/final output retain explicit timing limits. Valid-object supervised periodic compatibility, authenticated IPC and renderer delivery remain unverified or unimplemented. See [parent ownership boundaries](process-observation.md#parent-owned-periodic-capture).

### Timed subscription replay follow-up

A fixed generated integration check now joins the periodic reporter and bounded stdout writer to a real owned child, supervisor and interim health subscriber. `supervise-periodic-progress-replay` checks that generated health is observed during ownership and revoked afterward, alongside the terminal/EOF/exit and reaping summary. Executable tests also cancel after an actual periodic sample and reject a foreign session, with no BPF activation. This verifies composition of the existing components rather than a new live or authenticated channel. The fixture session is public, sample delivery may coalesce and scheduling can fail the bounded diagnostic. See [timed subscription replay](process-observation.md#timed-subscription-replay).

### Kernel-verified Unix peer follow-up

A Linux/observe-only library gate now accepts one caller-provided Unix listener connection under a bounded wait and requires exact kernel-reported UID and PID before returning the untouched stream. Tests use actual abstract sockets, mismatched credentials, cancellation/deadlines and a distinct owned child process. The gate creates no production endpoint and is not wired into startup, capture or renderer delivery. Expected identity, PID lifetime, namespaces, endpoint permissions, descriptor transfer and mutual authentication remain explicit caller obligations; peer credentials alone are not authenticated application health. Session/framing validation and sample-age/cleanup rules still apply. See [Unix peer acceptance boundaries](process-observation.md#kernel-verified-unix-peer-acceptance).

### Verified socket progress follow-up

A Linux/observe receiver now consumes only a kernel-verified Unix stream and applies the existing session/source/framing/counter rules with total and validated-record idle deadlines. It joins socket EOF with an independent owner-supplied process-exit observation before returning a terminal summary, and shares revocable age-limited health publication with the stdout supervisor. Real socket tests cover interim data, terminal revocation before EOF, malformed/foreign/truncated input, missing EOF/exit, dribbles, cancellation and task abort. No socket-backed collector, production endpoint, process supervisor or renderer delivery is added; owner cleanup and peer/namespace/endpoint trust remain explicit obligations. See [verified socket reception](process-observation.md#verified-socket-progress-reception).

### Owned socket progress follow-up

A Linux/observe library owner now joins a caller-provided listener and direct child
to the verified socket receiver. It validates policy/session before spawn, derives
the expected peer from effective UID and its owned child PID, includes acceptance
in the total budget, and requires actual child wait plus socket EOF for completion.
Failure revokes health and performs bounded kill/reap cleanup; cleanup failure takes
precedence while preserving a triggering deadline. Eight real-child/abstract-socket
tests cover success, invalid input/exit, wrong PID, missing connection/EOF/exit,
total-budget accounting, cancellation and pre-spawn gates. Endpoint creation,
socket-backed collector commands, privileged service and renderer delivery remain
unimplemented. Aborting an owner future is not awaited cleanup; direct-child and
endpoint/namespace trust limits remain explicit. See [owned socket supervision](process-observation.md#owned-socket-progress-supervision).

### Generated socket replay follow-up

The no-options Linux/observe `supervise-socket-progress-replay` diagnostic now
creates an ephemeral abstract endpoint and random parent-selected session, starts
its own generated-only child with an isolated environment, and drives the shared
periodic reporter through verified socket ownership and a concurrent health
subscriber. Completion requires actual child reaping, validated terminal progress,
observed generated health and revocation. Six-second total, three-second idle and
two-second acceptance bounds apply; SIGINT/SIGTERM cancel and await cleanup. The
bounded final report declares generated/inactive provenance and omits the raw
binding/endpoint. Executable tests cover normal and rejected bootstrap/arguments,
foreign sessions, sample-triggered cancellation and feature gates. No BPF, audio,
Discord, production capture endpoint or renderer service is activated. Endpoint
security and trusted live collector/client wiring remain future work; the binding
is not secret and peer verification is not mutual authentication. See [generated
socket replay](process-observation.md#generated-socket-progress-replay).

### Parent-verified socket collector follow-up

The Linux/observe `capture-socket-progress` child command now reuses the existing
collector lifecycle and periodic reporter over a separately acknowledged pathname
socket transport. Strict arguments and session validation precede connection;
a same-user 0700 directory and actual-parent PID are required, followed by bounded
connect and exact server UID/PID checks before any progress or object access.
Stdout stays empty. The socket owner can receive and reap a real collector's
session-bound invalid-object fault; six executable tests cover pre-connect gates,
endpoint type/mode, wrong server PID and fault/reaping without loading BPF. The
transport-neutral reporter preserves existing stdout and legacy behavior. This is
not cross-UID privilege separation, executable attestation, race-free filesystem
pinning or verified valid-object capture. Production endpoint creation/removal,
a parent socket capture CLI, renderer handoff and separately approved live
compatibility testing remain future work. See [socket capture transport](process-observation.md#parent-verified-socket-capture-transport).

### Parent-owned socket capture follow-up

The separately acknowledged Linux/observe `supervise-socket-progress-capture`
command now selects a random session, owns a fresh private pathname endpoint,
launches its fixed socket collector child with an empty environment and awaits
peer-verified supervision plus endpoint removal. Policy gates precede filesystem
creation; the caller cannot override the endpoint/session/executable. Total duration
plus three seconds, three-second idle, two-second acceptance and existing kill/reap
bounds apply. The endpoint owner uses exclusive 0700 creation and nonrecursive
identity-checked cleanup; unexpected contents/replacements are preserved. The final
six-field report separates child reaping from `endpoint_removed` and omits raw
bindings, paths and PIDs. Five endpoint tests and the nine-test socket CLI suite
cover collisions, refused cleanup, setup/consent gates and actual invalid-object
collector fault/reaping/removal. This does not establish valid-object live
compatibility, atomic filesystem pinning, cross-UID privilege separation, crash
recovery or renderer delivery. See [parent-owned socket capture](process-observation.md#parent-owned-socket-capture).

### Bounded client handoff follow-up

A six-field owner-subscription snapshot now exports only safe kernel health
counters and the remaining portion of a two-second receipt-age budget. Generated,
invalid, revoked and stale samples carry no counters. The review validates matched
host responses, deducts request elapsed time, expires copies locally and clears on
new checks, close or session replacement. It never promotes recent counters into
historical reports or sensor-liveness claims. Real generated socket-child tests
also verify rejection at this boundary. Ordinary injection sends an explicit
`not_connected` snapshot and never automatically launches a collector. The
separately acknowledged connection is described below; continuous IPC,
remote revocation and live compatibility remain unfinished. See
[bounded client health handoff](process-observation.md#bounded-client-health-handoff).

### Reusable capture ownership follow-up

The socket-capture CLI now delegates to `progress::capture::Capture`, a reusable
prepared owner returning a health subscription before any child starts. Trusted
backend callers supply an absolute collector executable and explicit capture
policy; preparation validates before filesystem work, and only polling `run`
can spawn. Existing private endpoint/session selection, peer verification,
deadlines, reaping and six-field cleanup report are shared unchanged. New tests
cover prepared and unpolled drop, ready cancellation, real preload faults, spawn
failure and independent endpoint cleanup refusal. The library is not a consent
mechanism, executable attestation or privileged service. This owner is now used by
the separately acknowledged injector mode below; default injection stays
disconnected. See [reusable socket capture owner](process-observation.md#reusable-socket-capture-owner).

### Explicit injector ownership follow-up

Direct injection now accepts a Linux/observe-only, all-or-nothing object/PID/
duration policy with `--acknowledge-injected-observation`. Gates precede DevTools
access. After successful injection, a fixed sibling collector runs under the
shared private owner, with its receiver sampled only on matched diagnostics.
Ordinary launch/injection remains sensor-free. `Capture::run_with` cancels on
tracking return and awaits child/endpoint cleanup before client disposal; capture
completion itself does not end tracking or restart collection. SIGINT/SIGTERM
use the same teardown. The final report is deferred until detach, and failures
produce a nonzero exit; collector outcomes are sampled on explicit diagnostics,
not pushed to the renderer.
Real-binary tests use fake DevTools, an invalid object and a connected silent
child, not Discord or live BPF. Valid-object integration, privileged separation,
live liveness/status presentation and crash guarantees remain unverified or
unfinished. See [explicit injected observation](process-observation.md#explicit-injected-observation).

### Owner outcome visibility follow-up

Capture owners now retain one bounded outcome snapshot, published only after
supervision and endpoint cleanup finish. Matched host diagnostics expose pending,
finished, unavailable or not-configured state; a finished result separates the
typed supervisor outcome, child start/reap facts and endpoint removal. The review
strictly validates this optional field and clears stale results across check,
close and session boundaries. Pending is explicitly not activity verification;
finished describes one past run, not liveness, and uses no health counters or TTL.
Tests cover publication before tracking ends, closed publishers, actual mock-CDP
delivery and independent cleanup labels. Immediate notifications, live valid-object
compatibility and privilege separation remain unfinished. See
[sampled owner outcome](process-observation.md#sampled-owner-outcome).

### Optional setup fallback follow-up

An explicitly requested process sensor can now fail during private-owner
preparation without detaching baseline Callmon tracking. The injector warns once,
continues voice updates and matched diagnostics, and reports owner unavailable
with no result and no connected health sample. There is no retry or elevation.
Setup failure remains visible as a nonzero exit after detach; tracking errors keep
their own cause. Fake-DevTools tests cover missing and regular-file endpoint
roots, voice-history updates after diagnostics, reload and handled signals, and
sentinel/endpoint preservation. This closes the preparation-error fallback gap,
not the broader live compatibility, resource containment or privilege-separation
milestones. See [explicit injected observation](process-observation.md#explicit-injected-observation).

### Automatic session integration follow-up

Process owner outcomes now join the existing backend-to-client session view,
including an initial state before voice input and watched completion/unavailability
updates without a diagnostic request. The production menu renders these separately
from sampled prerequisite results, and Session Health exposes waiting/fault/cleanup
state. The fake-DevTools test feeds real Rust view JSON into the real menu component
in a Node renderer child and checks visible outcomes before diagnostics, continued
voice history and handled teardown. Health freshness, evidence and liveness claims
are unchanged. This integrates the existing process-owner path into the application;
it does not connect acoustic capture, correlate kernel events, separate privileges
or establish valid-object/live Discord compatibility. See
[automatic session integration](process-observation.md#automatic-session-integration).

### Combined sensor ownership follow-up

The backend now derives one bounded `session_sensors` lifecycle verdict from the
existing process and acoustic owner contracts and publishes it with every session
view. It reports only configured, pending and finished owner counts, aggregate
cleanup state, and whether the session is sensor-free, pending, settled or needs
attention. The production Session Health and Safety & Intelligence views validate
and display that contract while retaining each detailed owner section. Process or
acoustic faults, unavailable outcomes and unconfirmed cleanup remain visible as
attention. This is lifecycle integration only: it adds no samples, identities,
timestamps, liveness claims, clock alignment, evidence correlation or automatic
activation. Real sensor compatibility remains separately unverified.

### Automatic process health handoff follow-up

The injector now publishes the existing bounded process `Snapshot` in every
session view and watches the owner health receiver for changes independently of
terminal outcome updates. Production Session Health and Safety & Intelligence
use the same strict parser and locally expire recent counters within the
backend-supplied remainder of the two-second receipt budget. Automatic status is
kept separate from explicit diagnostic progress and from participant evidence.
The handoff contains no PID, endpoint, timestamps or raw records and always says
`sensor_running: false`; **Recent kernel sample** is freshness, not liveness.
Generated/fake-DevTools integration verifies sensor-free and configured-no-sample
states plus concurrent acoustic ownership. Valid live BPF samples, sustained
collection, clock normalization and privilege separation remain unverified.

### Session timing inventory follow-up

Every backend session view now includes a strict seven-field `session_timing`
inventory derived from accepted process-owner and acoustic-summary state. Voice
and acoustic message receipts are identified as sharing the backend session
monotonic clock. Acoustic frames are classified separately as unavailable,
capture-relative, or bounded by the recorder-start/first-frame session interval.
Configured process observation is explicitly freshness-only because health
counters carry no event timestamp. Session Health and Safety & Intelligence use
the same exact-shape decoder, reject contradictory combinations, and always show
calibrated cross-sensor correlation as unavailable. Production-renderer tests
cover sensor-free, process-only and concurrent generated acoustic states. This
inventory does not calibrate PipeWire, align participant voice events, add a
process event clock, establish liveness or activate any sensor. Cross-clock
normalization therefore remains unfinished.

### Runtime acoustic processing health follow-up

The capture owner's existing bounded `OutputHealth` snapshots now traverse the
acoustic producer lane and are retained in schema-4 runtime summaries. Ingestion
checks queue capacity, high-water/pending bounds and exact batch/frame
conservation; fusion rejects source or lifecycle contradictions. The production
review shows processed, delivered, pending and discarded frames, overflow events
and queue high-water depth, while Session Health marks discarded processing data
as attention even after clean owner teardown. Generated capture tests exercise
the real processor, bus, Rust session JSON and production renderer. These are
processor disposition counters only, not PipeWire/device liveness, source
identity, downstream receipt, reconnect supervision or live compatibility.

### Integrated session evidence follow-up

Every published backend session now derives a strict `session_evidence` verdict
from the exact owner aggregate and process snapshot in that view plus the retained
acoustic summary. It distinguishes sensor-free, awaiting, partial, independently
available and degraded evidence. Owner attention, rejected process samples,
acoustic faults, discarded frames or processor overflow force degradation. The
client recomputes these consistency rules before rendering, and process-only and
generated-acoustic production-renderer tests cover the serialized contract.
Independent availability does not imply synchronized events: correlation remains
explicitly unavailable, acoustic data remains unattributed, and this verdict does
not establish source/device liveness or live PipeWire/BPF compatibility.

### Injected acoustic fault classification follow-up

Acoustic owner schema 2 now carries a bounded terminal reason for faults, mapped
from the capture controller's typed lifecycle classification. The shared session
and production review can distinguish discovery, source rejection, recorder,
stream, processing, delivery, cleanup and internal failures without exporting
diagnostic text or sink identity. Completed runs deliberately use a null reason
because duration and owner-requested completion are not retained by the reusable
API. Schema-1 owner results remain accepted by the client. Automatic reconnect
remains unfinished and disabled: a rediscovered serial is not sufficient proof
of the same consented physical sink, and deferred capture remains one-shot and
call-bound.

### Acoustic startup source continuity follow-up

The capture owner now takes a second bounded PipeWire metadata snapshot after
starting the recorder but before frame-clock anchoring, processor creation or PCM
acceptance. The selected serial must remain an output sink with the same node
name. Replacement, disappearance or reclassification stops and reaps the recorder
and produces the existing bounded `source_rejected` owner reason. Generated CLI
and injected production-renderer tests cover this transition and zero retained
frames. This is a narrow startup continuity check, not physical device identity,
ongoing source monitoring, liveness, reconnect supervision or live compatibility.

### Live-capable user packaging follow-up

The default user installer and desktop entry remain sensor-free. An explicit
`--with-live-sensors` installer mode now builds the host with `observe`, runs the
pinned real eBPF build-and-offline-inspection gate, and stages compatible
user-owned artifacts. A separate root-only script installs protected copies of
the object and collector plus a dedicated broker carrying `CAP_BPF`,
`CAP_PERFMON`, and `CAP_DAC_READ_SEARCH`; the last capability permits reading
the host's root-only tracefs event metadata. The general observer and injector remain unprivileged. The broker
accepts one fixed no-object collection command, verifies its parent and same-user
target, then `exec`s the fixed collector so existing socket peer checks and owner
cleanup remain intact. Another root-only script removes those artifacts. The
existing separately acknowledged process and acoustic policies can therefore be
combined in one installed `launch` invocation without arbitrary privileged
bytecode or a privileged Discord-facing process. Generated tests prove the
default mode omits sensor artifacts, pin the broker command and capability target,
and reject general observer surfaces without privileges. Installation itself
does not open Discord, load BPF or start PipeWire. Valid privileged collection
and sustained three-path operation remain operator-approved live validation work.

A subsequent approved bounded run used the release artifacts with Discord
desktop 1.0.170. The live PipeWire owner delivered 250 complete frames over five
seconds and reaped the recorder; renderer teardown was confirmed and Discord
remained running. The concurrent real process owner reached BPF load but the
unprivileged host policy rejected it, producing `load_failed` with zero health
samples, confirmed child reaping and endpoint removal. This validates combined
ownership, live acoustic delivery and process-failure cleanup, not successful
kernel observation. The constrained broker now implements the next privilege
boundary. After adding `CAP_DAC_READ_SEARCH` for the host's root-only tracefs
metadata, an approved five-second combined retry completed with six process
lifecycle records, four periodic health samples, confirmed attach/detach, child
reaping and endpoint removal alongside 250 processed acoustic frames. No owner,
helper or recorder survived teardown and Discord remained running. This verifies
one bounded three-owner production path, not sustained capture, event completeness,
crash cleanup, calibrated timing, attribution or cross-sensor correlation.

### Phase 2 — Acoustic protection

- [ ] Add explicit opt-in, per-session PipeWire capture with clear on/off indicators.
- [x] Connect separately acknowledged bounded output capture to shared session ingestion, automatic review/health status and awaited injector teardown (generated end-to-end tests only; live compatibility unverified).
- [x] Route latest-only bounded spectral and coarse repetition scalars through runtime ingestion and review without PCM, descriptor vectors, acoustic history, participant evidence or scoring.
- [x] Add an explicit bounded-duration PipeWire output-sink CLI prototype with foreground status and supervised child teardown (live compatibility unverified).
- [x] Add offline supplied-frame RMS/peak, per-channel energy, near-full-scale ratio, and continuity-aware level differences.
- [x] Add offline bounded chunk-to-frame assembly, sample-clock mapping, and explicit partial-buffer/gap handling.
- [ ] Integrate PipeWire buffer conversion, cross-clock normalization, capture health, and reconnect supervision.
- [x] Add offline centroid, high-band power ratio, and continuity-aware spectral flux features.
- [x] Connect bounded frame assembly to spectral processing with shared gap/reset semantics.
- [x] Add offline bounded coarse spectral-sequence similarity features (not robust content fingerprints).
- [x] Integrate repetition with bounded chunk assembly and shared atomic gap/reset handling.
- [ ] Evaluate robust repetition fingerprints/alignment on representative audio.
- [ ] Implement local acoustic event scoring and cooldowns.
- [ ] Add reversible playback attenuation and review controls.
- [ ] Ensure no raw audio persistence by default.
- [ ] Benchmark across speech, music, gameplay, laughter, and synthetic micspam.
- [x] Add an offline generated-PCM processing baseline with explicit timing scope and verified workload accounting.
- [x] Add bounded explicit local WAV evaluation tooling with documented decoding limits and summary-only output.
- [x] Add offline explicit acoustic activation, stale-producer isolation and fail-closed processing teardown.
- [x] Add permit-checked offline producer failure reports that cannot stop newer activations.
- [x] Add offline activation-tagged results and rejection of stale queued outputs at acceptance.
- [x] Add an owning bounded offline output FIFO with explicit overflow faulting and lifecycle-coupled disposal.
- [x] Add offline output disposition and high-water accounting with bounded, versioned health snapshots.
- [x] Add bounded offline interleaved PCM byte conversion with explicit encoding and normalized-sample validation.
- [x] Connect offline byte input to permit-checked activation, fail-closed teardown and shared output accounting.
- [x] Add optional offline activation-bound encoding with permit-checked format-change teardown.
- [x] Add generated-only lifecycle replay with stepwise output conservation and explicit fault/restart checks.
- [x] Add offline planar/strided PCM views with checked ranges and bounded interleaved output.
- [x] Connect offline plane input to permit/encoding checks, fail-closed teardown and shared output accounting.
- [x] Add offline source-clock interval mapping with explicit bracket/rate assumptions, expiry and checked arithmetic.
- [x] Add offline sample-position interval mapping with outward rounding and explicit ideal-cadence assumptions.
- [x] Attach offline clock intervals to repetition frames with atomic timing validation and explicitly nominal nested features.
- [x] Connect bounded offline packed/planar PCM conversion to interval-bearing processing with atomic rejection.
- [x] Add offline permit-bound timed processing, typed clock-fault teardown and activation-tagged output acceptance.
- [x] Add an owning bounded offline timed-output FIFO with lifecycle disposal, overflow faults and lifetime output-health accounting.
- [x] Add generated-only timed lifecycle replay with interval-delivery checks, clock-expiry teardown and stepwise output conservation.
- [x] Add a session-bound stop-only mixed-output control that awaits cleanup without detaching voice tracking or process observation; capture startup remains explicit CLI opt-in.
- [x] Add an optional one-shot, session-bound client confirmation for a fully CLI-authorized sink and duration; no PipeWire tool starts when the prepared session detaches unconfirmed.
- [x] Integrate bounded acoustic owner lifecycle and cleanup outcome into the shared session and Session Health without exporting diagnostics, device identity or audio-derived history.
- [x] Bind deferred client-confirmed capture to a validated active voice call and stop it on call end or switch without exporting call identity; preserve immediate CLI-authorized behavior.
- [x] Carry the existing complete observe/acoustic consent policies through automatic CLI launch while keeping the installed desktop launcher sensor-free and refusing silent policy changes to an owned session.
- [x] Derive and render one strict backend-owned lifecycle/cleanup verdict across optional process and acoustic owners without claiming liveness or evidence correlation.

**Exit criteria:** bounded CPU/memory; documented test metrics; resilient PipeWire reconnects; no unsupported per-user speaker attribution.

### Phase 3 — Linux telemetry and containment

- [x] Add explicit read-only Linux kernel prerequisite diagnostics in the client and standalone CLI (not a program load/attach test).
- [x] Add optional Aya process-exec source/reader and generated event-contract replay in client diagnostics (host-checked only).
- [x] Compile the real BPF-target process sensor locally and expose offline code inventory with explicit artifact regression tests (not a verifier/load test).
- [ ] Prototype Aya-based process/socket telemetry on supported Arch kernels.
- [ ] Separate privileged sensor from ordinary Rust backend.
- [ ] Correlate kernel events with application timelines using monotonic clocks.
- [ ] Add cgroup v2 resource limits and service health checks.
- [ ] Detect unprotected or externally bound debugging endpoints.
- [ ] Support graceful fallback when eBPF, privileges, or kernel features are unavailable.

**Exit criteria:** least-privilege operation, zero remote listener exposure by design, event-loss metrics, and no loss of baseline Intellicord functionality without eBPF.

### Phase 4 — Intelligence workflow and evaluation

- [ ] Combine independent signals using calibrated, explainable scoring.
- [x] Add basic incident evidence review, explicit Markdown export, and confirmed intelligence reset (prototype workflow above).
- [ ] Extend incident grouping/context and add false-positive feedback/suppressions.
- [ ] Add per-community baselines and conservative alert thresholds.
- [x] Add an initial labeled synthetic fixture set and record reproducible per-rule precision/recall counts (not field accuracy).
- [x] Add synthetic voice replay cases for receipt jitter, capture gaps, and benign synchronized joins/reconnects.
- [ ] Establish representative independent datasets and compare with simple detection baselines.
- [ ] Measure false-alert exposure rates and evaluate mixed audio after an opt-in acoustic pipeline exists.
- [ ] Publish limitations and a compatibility matrix across tested Discord builds.

**Exit criteria:** measurable performance versus simple baselines; acceptable false-alert rates established before any default alerts; no automatic punitive actions without authorization and reliable evidence.

## 9. Evaluation plan

| Dimension | Metric | Evaluation method |
|---|---|---|
| Detection quality | Precision, recall, false alerts/hour | Labeled benign and adversarial fixtures |
| Detection latency | Time from observable onset to alert | Synchronized replay tests |
| Audio performance | CPU, memory, processing lag, drop rate | PipeWire stress tests and profiling |
| Event reliability | Missed/duplicated transitions, queue overflow | Voice-state trace replay and live diagnostics |
| Attribution calibration | Fraction of unsupported attribution claims | Manual incident audit |
| Resilience | Restarts, degraded-mode behavior | Fault injection and disconnect/reconnect tests |
| Security | Open ports, privileges, permissions | Endpoint audits and sandbox review |
| Privacy | Actual retention vs declared policy | File/system inspection and lifecycle tests |

Use prospective test thresholds only after collecting baselines. Avoid inventing benchmark targets or marketing claims before measurement.

## 10. Security, privacy, and operational risks

| Risk | Mitigation |
|---|---|
| Discord private store/API changes | Compatibility checks, fail closed, versioned adapter tests |
| DevTools endpoint offers broad client access | Loopback-only binding, local verification, process isolation, short attachment lifetime |
| False accusation of normal users | Explanation, uncertainty, review-before-action, feedback mechanisms |
| Audio privacy concerns | Off by default; visible capture indicator; no raw audio disk storage by default |
| Privileged eBPF component vulnerabilities | Minimal sensor, narrow permissions, signed/reviewed builds, userspace parsing |
| Kernel/version compatibility | Capability detection, CO-RE where possible, optional fallback |
| Kernel telemetry misinterpreted as remote identity | Strict source labels and prohibition on user attribution from socket metadata |
| Performance impact | Bounded queues, throttling, load shedding, CPU/memory budgets |
| Client-policy/account risk | Clear documentation that client modification is unsupported; user opt-in |
| OS-wide traffic rules disrupt Discord | Dry-run, narrow process scope, rollback, and no automatic endpoint blocking |

## 11. Key architectural decisions

1. **Keep Intellicord local-first.** No cloud service or central database is required for the initial plan.
2. **Prefer existing kernel facilities.** Do not implement a `.ko` for functionality already provided safely by eBPF, cgroups, audit, or PipeWire.
3. **Ship core behavioral detection before kernel work.** Discord application events carry the strongest signal about remote actors.
4. **Treat acoustic detection as an independent sensor.** Detection of micspam does not itself establish the responsible speaker.
5. **Make every sensor optional.** Intellicord must remain useful without elevated privileges or audio capture.
6. **Require explainability.** Alerts must show source facts, timing, uncertainty, and detector version.
7. **Avoid automatic punishments.** Favor local attenuation, notifications, and authorized moderation workflow.
8. **Preserve reversible installation and cleanup.** Detaching should end observation and restore affected local UI or audio state.

## 12. Recommended immediate work

**First milestone: a reliable voice-event fusion and incident prototype.** Add the `Observation` contract, better visibility into lost transitions, a small set of burst/re-entry detectors, and an incident timeline. This validates the defensive value without introducing kernel privilege or audio privacy complexity.

**Second milestone: opt-in acoustic anomaly detection.** It directly targets micspam and provides a stronger practical return than packet inspection.

**Third milestone: optional eBPF diagnostics and Linux containment.** This adds host-side resilience and context while remaining honest about the visibility limits of encrypted Discord traffic.

---

### Research and implementation references

- [Intellicord repository](https://github.com/itsnotqwerty/intellicord)
- [Aya — Rust eBPF framework](https://aya-rs.dev/)
- [Linux kernel eBPF documentation](https://docs.kernel.org/bpf/)
- [Linux kernel cgroup v2 documentation](https://docs.kernel.org/admin-guide/cgroup-v2.html)
- [PipeWire documentation](https://docs.pipewire.org/)
- [systemd resource control](https://www.freedesktop.org/software/systemd/man/latest/systemd.resource-control.html)

*Only the prototypes and diagnostic handoffs described above are implemented. Local BPF-target compilation and offline inspection pass, and the operator reported load, attach/detach and controlled-child delivery success on the reference host. Valid-object supervised capture, sustained collection compatibility, authenticated live health, integrated sensors, later detection engines, calibrated accuracy and protections remain unimplemented or unvalidated.*
