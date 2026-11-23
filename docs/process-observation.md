# Optional Process Observation Prototype

This is an Aya prototype, not validated production telemetry. The optional userspace reader and real `bpfel-unknown-none` kernel artifact compile locally, and offline instruction-inventory tests pass against that artifact. The operator reported successful privileged load-only, disabled-target attach-only and controlled-child delivery probes on Linux 6.18.58-1-lts, including expected event validation, child reaping, target clearing and explicit detach. On 2026-11-22, the constrained helper also completed a five-second combined injected run against Discord desktop 1.0.170: the fixed collector loaded and attached the reviewed tracepoint object, delivered four periodic health samples, emitted a six-record lifecycle ending normally at `duration_limit`, explicitly confirmed attachment and detachment, reaped, and removed its private endpoint. Discord and the concurrent PipeWire owner remained independently supervised and cleaned up. Sustained collection, overload behavior, event completeness and crash cleanup remain untested. Nothing loads or attaches during normal Intellicord startup or client prerequisite checks. Earlier implementation notes describe the limits at their milestones; this current status supersedes earlier untested-probe notes for that object/host only.

## Supervised Capture Health Handoff

The operator-only observation binary can now supervise the existing health-only capture command as a direct child. It uses a fixed same-executable command, never an externally chosen collector binary, and requires a distinct acknowledgement before spawning:

```sh
./target/debug/intellicord-observe supervise-capture \
	--object crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process \
	--pid APPROVED_TEST_PID --seconds 5 \
	--acknowledge-supervised-observation > health-report.json
```

Replace `APPROVED_TEST_PID` with an explicitly approved local test process, not Discord without separate approval. This is a real observation command requiring the same reviewed object, permissions, numeric PID/pidfd and namespace assumptions as capture. Appropriate privilege elevation is an operator decision. Never run Cargo or the injector as root. The command accepts no arbitrary executable, event export, renderer argument, reconnect or daemon option. Output redirection is optional and user-initiated; it creates a file that remains until deleted. The report contains no PID/path, absolute clocks, record bytes, audio, messages or authentication data.

The parent consumes bounded NDJSON status from the child's stderr and discards stdout. The existing supervisor requires validated terminal status, stderr EOF and a consistent direct-child exit before accepting a result. The additive `terminal_health` field in `SupervisedRun` is populated only after those checks pass. Malformed/truncated/replayed streams, stalled children, deadlines and invalid exits leave both status and health absent; coherent collector faults may retain the fault's optional terminal counters. Old generated-supervisor consumers can ignore the additive field. Counters describe the collector, not participant behavior or complete event delivery.

The parent collection limit is requested duration plus three seconds, including child setup after spawn. Failure, SIGINT or SIGTERM requests direct-child kill and awaits reaping with a separate two-second cleanup limit; cancellation is not a graceful child-stop acknowledgement. A deadline or killed child never claims explicit kernel detach. Synchronous spawn and arbitrary runtime scheduling are not hard-bounded; kill-on-drop on task abortion is not awaited reaping. Process descendants, a privileged loader service, authenticated IPC, crash cleanup, cgroups and independent kernel-removal verification remain outside this prototype. The parent starts no network listener or privilege helper.

The final outer JSON has `schema_version: 1`, `source: supervised_kernel_capture`, `sensor_running: false` and `run`. Only `run.outcome: completed` exits zero; typed failure JSON plus nonzero exit is intentional. `collector_reaped` and lifecycle `detachment_confirmed` describe separate past operations. A report does not prove that no orphaned kernel resource exists. A completed quiet capture may correctly report zero accepted events.

### Supplied Client Snapshot

The ordinary, unprivileged backend can display a report explicitly selected through its environment:

```sh
INTELLICORD_OBSERVE_HEALTH_REPORT="$PWD/health-report.json" \
	./target/debug/intellicord check-health-snapshot
```

Use the same environment setting when starting the backend through its normal approved workflow, then explicitly check prerequisites in Safety & Intelligence. This does not start capture, change permissions, select a process or attach Discord by itself. The renderer cannot choose a path. A default build reports `unavailable` for a configured file; optional Linux observe builds read it. With no setting, status is `not_configured`. `check-health-snapshot` takes no additional arguments and exits successfully when a status report is produced, not as a capture-success verdict.

The read opens a nonsymlink regular file nonblocking, accepts at most 4096 bytes and strictly rejects unknown/duplicate fields, malformed JSON, noncompleted/inconsistent reports, missing terminal counters, missing detach/reaping claims and counters outside JavaScript's safe integer range. The displayed snapshot requires a six-record stopped kernel lifecycle and a normal stop reason. Invalid or unreadable files become `rejected` with no retained report or path. The file is not watched, retried or rewritten. Concurrent file writes may legitimately produce rejection.

Configured optional Linux reads run in a fixed same-executable `health-snapshot-inspect` child rather than a blocking task in the backend. The helper accepts no arguments, inherits the operator's report environment setting and emits only a validated report or JSON `null`. Its standalone invocation performs synchronous file I/O and has no independent deadline; use `check-health-snapshot` for supervised inspection. The parent discards stdin/stderr, caps stdout at 4096 bytes, independently validates reports, and requires EOF plus successful direct-child exit within two seconds after spawn. Failure requests kill and awaits exit for up to two additional seconds. No arbitrary helper executable or path can be selected by the renderer.

Snapshot status distinguishes file/protocol rejection (`rejected`), deadline expiry followed by confirmed direct-child reaping (`inspection_timed_out`), executable resolution/spawn/read/wait/nonzero-exit failures (`inspection_failed`), and failed or unconfirmed direct-child cleanup (`cleanup_unconfirmed`). Every failure carries a null report and clears earlier displayed counters; cleanup uncertainty takes priority over an earlier timeout. A completed helper returning `null` remains `rejected`. These are inspection outcomes, not collector lifecycle or kernel-detach claims. The five-field envelope is unchanged; clients predating these additional status values omit the unrecognized snapshot rather than treating it as success.

This bounds asynchronous response/exit/cleanup waits, not synchronous executable resolution/spawn or arbitrary runtime scheduling. A child stuck in uninterruptible filesystem I/O may not reap before cleanup expires; no report is accepted then, but immediate termination is not guaranteed. Dropping or aborting the future retains kill-on-drop without guaranteed awaited reaping. Use a trusted local file, not a remote/special filesystem. This is not a privilege boundary, process-tree sandbox, authentication mechanism or liveness proof.

The five-field snapshot envelope identifies `source: operator_supplied_snapshot` with inactive flags and optional report. It is **untrusted supplied historical data**, not authenticated kernel evidence, a current sensor status, or a fresh result bound to this backend session. It carries no timestamp, so age is explicitly unknown; editing a valid-shaped file can change its claims. The client labels provenance and freshness, independently validates structure/counters/confirmation consistency, clears old reports on malformed responses, rate limits and session changes, and accepts older backends without this optional field. Preview snapshots are labeled outer fixtures. Counters and reported cleanup never enter participant evidence, export, scoring or moderation. A continuous authenticated live-health channel remains future work.

Unit, executable and generated-subprocess tests cover strict decoding, byte/symlink/regular-file/privacy bounds, inactive CLI gates, completed/faulted status handling, counter retention only after EOF/exit agreement, deadline/invalid-stream rejection, client false/active claims, unsafe counters and stale clearing. Tests launch no real sensor. The operator's controlled delivery success is recorded separately; this new valid-object supervised-capture path has not been run during implementation.

## Bounded Progress Protocol

`observe::progress::ProgressStream` is a separate, incremental receiver for future periodic collector health. The existing six-record lifecycle protocol and real kernel reader remain unchanged. The new newline-delimited envelope has exactly `schema_version: 1`, a caller-selected 32-character hexadecimal `session`, a contiguous one-based `sequence`, and `payload`. Payload is an externally tagged enum: either `{"lifecycle":{"status": STATUS_RECORD}}` containing an unchanged lifecycle object, or `{"health":{"health": PROCESS_HEALTH}}` containing the existing five-field health counters. Every envelope is bound to the receiver's expected session and source; lifecycle objects retain their own sequence independently of envelope sequence.

Health samples are accepted only after the observing transition and before stopping. They use the same schema and nonregressing counter checks as lifecycle health, including checks against later lifecycle counters. Unknown or duplicate envelope/payload/status fields, foreign sessions, replayed sequences, invalid phase ordering and malformed input permanently reject the stream. Nested lifecycle JSON is forwarded as raw JSON so duplicate keys cannot disappear during intermediate decoding. A stream is limited to 2047 bytes before each newline, 64 KiB total and 128 envelopes. Pending bytes and retained state are bounded; samples replace the previous sample rather than forming an unbounded history.

Each `push(bytes, receipt_ns)` takes a monotonic receipt time supplied by the owning transport. Fragmented records receive the time of the chunk completing their newline; multiple records in a chunk share its receipt time. Receipt times cannot regress, including on empty pushes. `latest_health(now_ns, max_age_ns)` returns only a validated sample whose receipt age is within the caller's inclusive limit, with `now_ns` not earlier than the latest receipt. Rejection, missing samples, stale samples and backwards time return no sample. The caller must enforce timers and query age as time advances: this receiver starts no timer, task, process or socket. Sample freshness measures receipt age only, not process liveness, source-clock accuracy, delivery latency or authenticated freshness. A recently received sample can remain recent even after a terminal lifecycle; it must not be interpreted as a running collector.

Completion still requires a complete terminal lifecycle record, no pending/truncated envelope, and caller-supplied exit success consistent with the lifecycle. EOF alone is never success. The receiver does not independently observe EOF or process exit; those are obligations of the owning transport. The session value must be selected independently by a future owner, but matching a known string is not authentication. There is no random challenge negotiation, peer credential check, privileged service, live BPF integration, reconnect or renderer-selected target here.

Two fixed, sensor-free CLI commands exercise this boundary:

```sh
./target/debug/intellicord replay-process-progress
./target/debug/intellicord progress-replay-child
```

The first returns eight named generated checks (fragmentation, foreign session, sequence replay, counter regression, pre-observation sample, freshness bounds, truncation and exit mismatch). The second emits a fixed eight-envelope fixture: six lifecycle records and two health samples. Its fixed public session is fixture data, not a credential. Both reject extra arguments. Executable tests collect the actual child's stdout through EOF, await exit and pass the observed exit status to the receiver. This is a finite generated round trip, not a supervised live-progress service or a wall-clock delivery guarantee.

The explicit client diagnostics expose the generated check results and failed case names, labeled separately from live health delivery. Strict client validation rejects malformed or active-sensor claims and clears stale results; older responses may omit `process_replay.progress`. No sample, binding or timestamp is sent to the client by this replay, and the checks never enter participant evidence, exports or scoring. Unit tests cover fragmentation, session/sequence/source rules, counter regression, receipt/freshness boundaries, duplicate/unknown fields and all byte/record-count limits. No real sensor is activated by these checks.

### Validated Progress Producer

`observe::progress::producer::Encoder::new(session, source)` prepares a bounded producer for a caller-selected session and source. `lifecycle(status)` and `health(counters)` return one newline-terminated envelope only after serializing it and accepting it through an internal `ProgressStream`. Envelope sequence numbers are assigned automatically; lifecycle sequence numbers remain the caller's responsibility. The same source, phase, schema, counter and byte/count checks therefore apply before bytes are returned and after transport delivery. Any encoding or validation error permanently rejects the encoder; later calls cannot resume it. `finish(exit_success)` consumes the encoder and checks terminal/exit agreement, but the exit classification is supplied by its caller, not independently observed.

The encoder retains bounded validation state rather than a history of emitted bytes. It uses a fixed internal receipt value solely for protocol validation; actual receipt time and freshness remain the receiving owner's responsibility. Encoding advances state before bytes are delivered. A caller must treat failed or partial output as a failed transport and abandon that encoder rather than assuming delivery or blindly retrying records. The encoder does not write, spawn a writer, apply output deadlines, schedule periodic samples, authenticate a session, or activate a sensor.

The existing generated child now uses this encoder without changing its fixed public session or eight-envelope/two-sample report. Tests also use a different session and sixty health samples, fragment delivery to an independent receiver, and compare validated summaries. Rejection tests cover invalid session/schema/source, pre-observation and post-terminal samples, counter regression, the envelope-count limit, sticky failure, incomplete runs and exit mismatch. This prepares reusable producer framing; live reader periodic emission and its bounded output transport remain unconnected.

### Deadline-Bounded Progress Output

`observe::progress::writer::Writer::new(session, source, output)` wraps the encoder and a caller-supplied `AsyncWrite + Unpin` output. Its asynchronous `lifecycle(status)` and `health(counters)` methods validate one envelope, then await both `write_all` and `flush` under a single fixed 100 ms deadline. There is no output queue or retry, and mutable access permits one active record operation at a time. An encoding error, partial write, write/flush error, timeout, or cancellation after the operation is first polled permanently disables further output and successful `finish`. An operation dropped before first poll has made no changes.

`finish(exit_success)` returns the validated summary only if prior record operations succeeded and lifecycle/exit classification agrees. It performs no additional write or shutdown; every successful record was already flushed. Flush success is a local output fact, not acknowledgement that a peer received, parsed or retained a sample. The receiver still requires its own observed EOF, terminal/exit agreement and deadlines. On failure, callers must abandon output, not attempt to append a successful terminal record or recover the encoder.

This bounds cooperative asynchronous write/flush waits, not serialization, arbitrary runtime scheduling, a blocking `poll_write`/`poll_flush`, synchronous output destruction, or underlying I/O that continues after future cancellation. In particular Tokio stdout uses blocking operations internally: timing out its future does not guarantee those operations stop or that runtime shutdown finishes promptly. The owning parent supervisor must still enforce the child lifetime and bounded kill/reap policy; uninterruptible I/O and task abort retain the previously documented limits. The writer adds no worker thread of its own and makes no process-tree or kernel cleanup claim.

The fixed `progress-replay-child` now uses this writer with Tokio stdout and preserves its eight-envelope/two-sample wire fixture and strict no-argument gate. Tests cover independently received output, pending writes, flush-only stalls, unread bounded-duplex backpressure, partial-write/flush errors, cancellation, sticky failure and invalid records without output. A Linux executable test redirects the actual child's output to `/dev/full` and requires unsuccessful exit, while existing executable tests compare all successful stdout bytes and verify EOF/exit and the owning replay. These are generated transport tests, not live periodic emission or proof that all OS-level blocked writes are interruptible. The real kernel reader and renderer remain unchanged.

### Opt-In Periodic Capture

The optional Linux observe executable now supports a distinct command:

```text
intellicord-observe capture-progress --object PATH --pid PID --seconds 1..60 --session HEX32 --acknowledge-process-progress
```

This command can load and attach the reviewed BPF object to observe the explicitly selected process. It is not invoked by startup, diagnostics, the renderer or CI with a valid object. Use only with appropriate permissions and deliberate operator consent; never run Cargo or the injector as root. The exact ordered arguments, nonempty object path, PID range, duration range, 32-character hexadecimal session and distinct acknowledgement are validated before progress output or object access. Legacy capture consent does not authorize this command. Event export, clock handshake, `--status-json` and extra arguments are rejected. Default builds do not include the observe executable.

The session is supplied by the owning caller and must be independently selected for each real run, not copied from the generated fixture. It binds records to a receiver expectation but is not a secret or authentication. Stdout contains progress envelopes only, without the ordinary final report or raw event export. Stderr may contain human-readable failure details. A supervising library caller should select `Source::KernelTracepoint`, pass the same expected session and apply its own total/idle deadlines, cancellation and awaited cleanup. The dedicated parent command below selects this binding and owns those waits; no client subscription is added.

`progress::reporter::Reporter` owns lifecycle sequence generation and the bounded writer. Its `emit` sends lifecycle envelopes; `periodic_health` may send one health envelope when its one-second period is due during observation. The period starts after successful delivery of the observing transition and restarts after each successful sample. Samples reflect current monitor counters from the existing 20 ms reader loop, including the latest ring-drop read. Missed periods produce no catch-up burst; no separate sampling task, unbounded queue or history is created. Before observation and after stopping/fault, periodic calls send nothing. A one-second capture may complete without a periodic sample. Output waits can delay ring draining and signal/deadline handling, so the existing delivery and parent-supervision limits still apply.

The reporter permanently rejects further operations after a lifecycle, encoding, delivery or cancelled in-flight operation fails. Normal stop follows the existing detach/final-drain path. Failures unwind owned handles and attempt a terminal fault envelope only while output remains usable. Fault notification is best effort; a failed notification does not replace the original collector error or establish coherent completion. The parent must reject missing, partial or inconsistent terminal output. A reported detach is not proof of global sensor cleanup or a replacement for the existing ownership limits.

Reporter tests exercise observation-only cadence, missed-period skipping, terminal suppression, stream agreement and sticky rejection after invalid transitions or cancelled output. Executable tests use only `/dev/null` or invalid requests: no actual BPF load is reached. They verify strict gates, foreign-session rejection, a two-envelope preload fault without a mixed summary, parent-confirmed reaping with no published health, and stdout failure before setup. A safe preload-fault check, expected to exit unsuccessfully, is:

```sh
./target/debug/intellicord-observe capture-progress --object /dev/null --pid 1 --seconds 1 --session 00000000000000000000000000000004 --acknowledge-process-progress
```

Live compatibility, periodic delivery under real load, privilege separation, authenticated IPC and renderer consumption remain unverified or unimplemented. This opt-in producer does not change the existing capture, event-export or clock-handshake wire protocols and records no audio or messages.

### Parent-Owned Periodic Capture

The optional Linux observe executable also provides a dedicated owning command:

```text
intellicord-observe supervise-progress-capture --object PATH --pid PID --seconds 1..60 --acknowledge-supervised-progress
```

It requires its own distinct acknowledgement, exact ordered arguments, a nonempty object path and valid PID/duration before selecting a session or spawning. It accepts no session, executable, event export, clock handshake, timeout override or additional arguments. The parent registers interrupt/terminate handlers, selects sixteen OS-random bytes using `getrandom`, encodes a fresh 32-character hexadecimal binding, then launches only its current executable in the fixed `capture-progress` child mode. Randomness failure aborts before spawn with no fallback fixture, PID, timestamp or reusable binding. This is session separation, not a credential: the binding is passed in the child's arguments and is not protected from local process inspection. The executable and environment must be trusted; there is no peer authentication or privilege separation.

The existing progress supervisor receives the independently selected expected binding and `Source::KernelTracepoint`. Its total deadline is requested duration plus three seconds, its validated-record idle deadline is three seconds, and failure cleanup has the existing separate two-second kill/reap wait. Normal one-second sample cadence fits within idle, but slow object setup, load or delivery may still time out; these are operational bounds, not guaranteed compatibility. SIGINT/SIGTERM requests cancellation and direct-child cleanup, not graceful collector shutdown or signals to Discord. Session selection, signal setup, current-executable resolution/spawn, arbitrary runtime scheduling, and final report output are not covered by the collector deadlines. Uninterruptible I/O and abort retain the existing cleanup limits.

After supervision, stdout contains one four-field final JSON report: `schema_version: 1`, `source: supervised_kernel_progress`, `session_binding: parent_selected`, and `run`. The run uses the existing six-field progress supervisor schema. Child progress stdout is consumed by the parent; stderr is discarded. The final report contains no raw binding, target PID, object path or interim sample stream. Only `completed` exits successfully; collector faults, timeouts, cancellation and other failures print their typed run then exit unsuccessfully. Pre-spawn setup/argument errors produce no report. The report deliberately makes no standalone `sensor_running` claim: inspect cleanup and lifecycle fields, and do not equate direct-child reaping with global sensor removal. It is not a historical `CaptureReport` accepted by the health-snapshot file handoff and adds no renderer subscription.

A sensor-free preload check uses an invalid object and is expected to return a reaped `collector_fault` with `invalid_object`, two lifecycle envelopes and zero health samples, then exit unsuccessfully:

```sh
./target/debug/intellicord-observe supervise-progress-capture --object /dev/null --pid 1 --seconds 1 --acknowledge-supervised-progress
```

Unit tests check fresh, receiver-valid parent bindings. Executable tests reject wrong consent, malformed/range-invalid requests and session/executable overrides, and run the actual parent/child preload fault with both minimum and maximum durations. Existing generated supervisor tests cover cancellation, deadlines, invalid streams and cleanup. No valid BPF object is loaded by these checks; actual supervised periodic collection, sustained delivery and live compatibility remain unverified. A valid-object invocation can load and attach BPF and requires a separate deliberate operator decision.

## Owning Progress Supervision

`observe::progress::supervisor::supervise` now owns a direct child and feeds its stdout to `ProgressStream` using elapsed monotonic receipt times measured by the parent. It discards stdin/stderr, forces piped stdout and enables kill-on-drop. The source and expected session are supplied by the trusted caller, not negotiated or authenticated by the child. Invalid sessions, zero/over-65-second total limits, zero idle limits and idle limits exceeding the total limit are rejected before spawn; already-ready cancellation also prevents spawn.

A total deadline limits the run, and an independent idle deadline starts immediately after spawn. Only completion of at least one fully validated envelope refreshes idle time. Partial records or byte dribbles cannot refresh it. The idle deadline remains active after stdout EOF while waiting for child exit, and after child exit while waiting for EOF. Valid envelopes never extend the total deadline. Cancellation takes priority over deadlines; total expiry takes priority over idle expiry when both are ready. Neither deadline covers synchronous executable resolution/spawn or arbitrary runtime scheduling delays.

Success requires observed stdout EOF, an awaited direct-child exit and receiver completion with consistent terminal/exit status. A coherent faulted lifecycle plus unsuccessful exit becomes `collector_fault` with its validated terminal summary; malformed, truncated, timed-out, cancelled and inconsistent runs retain no summary. A terminal record without process exit is not success. The bounded report contains schema version, outcome, child-started/reaped flags, optional `deadline: total|idle`, and optional summary. Deadline kind records the triggering wait even if a later cleanup failure takes precedence as the outcome.

On failure, the owner requests direct-child kill when needed and awaits it for up to two additional seconds. Cleanup failure overrides the outcome and never sets reaping true without a successful wait. Dropping/aborting the future retains kill-on-drop only, without guaranteed awaited reaping. Descendants, uninterruptible operations, privilege separation, cgroups and crash cleanup remain outside this controller. Reaping is a direct-child ownership fact, not a sensor-detach or global BPF-removal guarantee. The idle timer measures validated-record delivery rather than sample age or sender authenticity.

### Revocable In-Process Health Subscription

Rust callers can create `health_channel()`, retain its cloneable `HealthReceiver`, and move the single `HealthSender` into `supervise_with_health` as the final argument after cancellation. The existing `supervise` API remains unchanged. `receiver.changed().await` observes channel updates or closure; `receiver.latest(max_age)` returns an optional `HealthSample` containing the caller-declared source and validated counters. Always query `latest` again when rendering or consuming health: copied samples cannot be revoked retroactively. A `changed` notification is not a promise of a sample or a new health record.

The channel retains only the latest value, not a history or event queue. Slow consumers may miss intermediate samples, including all samples from a short replay. Updates use short synchronous watch-channel operations rather than awaiting consumer progress. Publication requires a fully validated read chunk, an observing lifecycle and a periodic health sample. A chunk that ends in a terminal/non-observing phase publishes no sample, and a rejected chunk exposes none of its newly parsed values. Fragmentary bytes and other lifecycle records do not renew the sample receipt timestamp.

Age is checked on every `latest` call against the parent's monotonic clock and the receipt of the sample's completing newline, with an inclusive maximum age. Future-dated receipts are rejected. Expiry does not generate a notification: consumers needing timely visual expiry must recheck on their own timer. Health is explicitly cleared before failure cleanup, cleared when leaving observation, and synchronously revoked when the sender is dropped on return or task abort. Channel closure is not proof of child reaping, and abort still has only the kill-on-drop guarantees above. Interim samples can precede a later invalid stream or exit mismatch; they are not final success evidence.

Generated subprocess tests cover publication, expiry, coalescing, provenance, partial-record non-refresh, terminal revocation before child exit, malformed output, idle timeout, cancellation, completion and task abort. This is an in-process library handoff only: no renderer subscription, real reader periodic emission, authentication, process-liveness guarantee or sensor activation is added. Source labels describe the trusted caller's selection, not independently verified peer identity.

The fixed, sensor-free owning replay runs only the same executable's existing generated child, with a three-second total limit and a two-second idle limit:

```sh
./target/debug/intellicord supervise-progress-replay
```

It accepts no additional arguments. Its outer report explicitly identifies generated provenance and false sensor/load flags; only a completed replay exits successfully. Other outcomes emit their typed report and a nonzero exit. Ctrl+C requests cancellation followed by the same awaited cleanup; no signal is sent to Discord. This command is not exposed as a renderer-selected launch facility and does not change the existing protocol-check rows or real kernel reader.

Real generated/Node child tests exercise successful completion, coherent faults, malformed/truncated output, nonzero-exit disagreement, silence, byte dribbles, EOF without exit, cancellation, total/idle precedence, progress-based idle refresh, total expiry despite progress, spawn failure and pre-spawn gates. The executable round trip asserts the actual owning command's final cleanup and eight-envelope/two-sample summary. Build-only CI already runs this test target and the observe unit tests. No real sensor or unkillable child is activated or claimed by these tests.

### Timed Subscription Replay

A separate sensor-free integration check connects the actual periodic reporter, bounded writer, child stdout, owning supervisor and revocable health subscription:

```sh
./target/debug/intellicord supervise-periodic-progress-replay
```

The parent launches only its fixed `periodic-progress-replay-child` mode with no additional arguments. The child uses the generated lifecycle and `Reporter` with a 20 ms polling tick, waits for two one-second sample periods, then leaves a 250 ms observation window before normal stopping. Missed polling ticks are skipped. The parent applies a six-second total deadline, a three-second idle deadline and the existing separate cleanup wait. Ctrl+C requests cancellation. Under ordinary scheduling the run takes about 2.3 seconds; the deadlines are not hard real-time guarantees, and severe scheduling stalls can cause the diagnostic to fail.

During ownership the parent awaits subscription changes and rechecks sample age with a two-second limit. It records only whether at least one sample was observed and whether every observed sample declared generated provenance. It does not queue samples, count notifications as samples, export counters or require delivery of every intermediate value. After supervision ends it checks a retained receiver for revocation. The eight-field final report contains schema/source, false sensor/load flags, `observed_health`, `generated_only`, `health_revoked`, and the ordinary `run`. Success requires all three checks and a completed run; a typed report is still printed before unsuccessful diagnostic exit. This is separate from the existing immediate replay and does not change client diagnostic rows or their schema.

Both new commands reject all extra arguments. Their fixed session `00000000000000000000000000000005` is public generated fixture data, not a credential, OS-random binding or authenticated session. The child mode emits only the eight progress envelopes; the parent emits only the final diagnostic report. No object path, PID, BPF program or real capture is involved. Session selection for real collection remains the separate parent-owned capture command's responsibility.

Executable tests exercise completed observation/revocation, actual periodic sample-triggered cancellation with awaited reaping and no retained summary, foreign-session rejection with no exposed health, and fixed-argument gates. Subscription expiry and other deadline/error paths remain covered by focused supervisor tests. Timing/cadence is tested for generated data only; this does not verify live BPF compatibility, authenticated IPC, a renderer subscription or process liveness from sample age. The same stdout cancellation, scheduling, direct-child cleanup and final report output limits apply.

### Kernel-Verified Unix Peer Acceptance

With Linux and the optional `observe` feature, `observe::progress::peer` provides a library-only boundary for a future local health transport. A trusted owner constructs `ExpectedPeer::new(uid, pid)` with an exact UID and a positive PID no greater than `i32::MAX`, then calls `accept(&listener, expected, limit, cancel)`. The caller supplies an already-bound Tokio `UnixListener`; this module does not create, discover, chmod or remove endpoints. It is not called by startup, diagnostics, capture commands or the renderer.

Acceptance waits for one connection with a nonzero deadline no greater than two seconds. Invalid policy is rejected before accepting. Ready cancellation has priority over the deadline and a queued connection; deadline expiry has priority over accept. After accepting, the module queries Linux kernel peer credentials via Tokio `peer_cred()` and requires both UID and PID to match exactly. Missing PID, credential lookup failure, mismatched credentials and accept errors return no verified stream. A rejected accepted socket is dropped without reading or writing application bytes. There is no retry loop: an unexpected peer fails that attempt. Any outer retry policy must enforce its own overall deadline and denial-of-service limits.

Only a successful check yields `VerifiedStream`; `peer()` returns the matched expectation, and consuming `into_stream()` transfers the socket for a separate framed protocol. The gate consumes no payload, performs no sample parsing, and creates no queues, tasks or sensor activity. Errors distinguish invalid policy, cancellation, deadline, accept failure, credential failure and peer mismatch. The asynchronous deadline covers the accept wait, not synchronous credential lookup, listener setup, arbitrary scheduling or subsequent reads/writes. Cancellation is checked during the wait, not continuously after acceptance has been selected.

SO_PEERCRED describes kernel connection credentials as represented in the receiver's namespaces. Expected UID/PID must come from trusted ownership policy in that same interpretation, never from a self-declared payload. An owner should retain an unreaped child or another appropriate lifetime guard while matching a numeric PID; this API does not pin identity or solve PID reuse. These credentials do not verify executable contents, privilege transitions, application protocol, namespaces, current liveness or which process holds the descriptor after transfer/inheritance. The gate is one-way, with no server authentication or protection against a compromised expected peer. Filesystem directory ownership, socket permissions, endpoint substitution, abstract-socket exposure and descriptor lifecycle remain caller responsibilities. A matched peer must still pass session/schema/framing/sequence/counter validation and transport deadlines before health is exposed.

Real Linux tests use temporary abstract sockets without filesystem artifacts. They verify matching kernel credentials while preserving unread payload, wrong UID and wrong PID rejection with socket closure, deadline/ready-cancellation behavior, and a distinct owned test child whose actual PID is verified before reading its marker and awaiting exit. These tests use no collector, BPF object, Discord process or health payload. They do not prove an authenticated service, mutual authentication, resource containment or live collection compatibility. The production progress path still uses owned child stdout; socket-backed health delivery remains unconnected.

### Verified Socket Progress Reception

Linux/observe library callers can move a successful `peer::VerifiedStream` into `progress::socket::receive(verified, session, source, limits, exit, cancel, health_sender)`. The session and source are trusted owner expectations, not negotiated from application input. `Limits` supplies a nonzero total duration up to 65 seconds and a nonzero idle duration no greater than total. The caller also provides an independent future yielding `io::Result<bool>` from observed process exit, a cancellation future, and the sender from `health_channel()`. No raw unverified socket is accepted by this API.

The receiver owns and closes that connection. It parses bounded 1024-byte reads using `ProgressStream` and parent-measured monotonic elapsed receipt times. Only complete validated envelopes refresh idle; fragmentary bytes do not, and progress never extends total lifetime. Cancellation takes priority over deadlines, total expiry over idle expiry. Both deadlines remain active after EOF while waiting for the exit future, and after exit while waiting for EOF. A missing exit observation therefore times out; terminal status or socket closure alone cannot become successful completion. Socket acceptance has its own separately configured peer-gate deadline; total reception time starts after acceptance, not at listener creation.

Completion requires observed socket EOF, the independently supplied exit result and strict terminal/exit agreement. A coherent faulted lifecycle with unsuccessful exit can return a validated fault summary, which is not successful collection. Invalid/truncated/session-mismatched streams, read errors, exit-observation errors, cancellation and deadlines return typed errors with no summary. The exit result must come from trusted process ownership (for example an awaited child), never from a socket payload. A ready future used by generated tests is fixture input, not proof of a real process exit.

Validated periodic health is published only while observing and only after the entire read chunk validates. The socket and stdout paths share the same latest-value publication helper, parent-clock age checks and no-history watch semantics. Partial bytes and non-health envelopes do not refresh sample age. Terminal status revokes health even if the socket remains open. Return, error, cancellation or task abort drops the health sender and clears retained values; copied samples cannot be retroactively revoked. Notification counts are not sample counts, and repeated clearing or coalesced updates must not affect consumer correctness.

This receiver does not spawn, kill, pin or reap a process. The owner remains responsible for those operations and for cleanup if the receive future fails or is aborted; dropping its exit future is not an awaited-reaping guarantee. The receiver's result contains no child-cleanup claim. Endpoint security, trusted peer expectations, PID/namespace lifetime, descriptor transfer and mutual-authentication limits remain those of the peer gate. Scheduling, synchronous credential/setup operations, blocking custom futures and uninterruptible OS behavior remain outside cooperative deadlines. No production listener, socket-backed collector command, privileged service, reconnect policy or renderer channel is created.

Real abstract-socket tests combine kernel peer acceptance with fragmented generated progress, interim health and revocation. They cover terminal revocation before EOF, missing EOF/exit, exit mismatch, foreign session, malformed/truncated records, byte dribbles, total/idle deadlines, invalid policy, exit errors, cancellation after publication and task abort. Tests use generated data and test-process peers, not a valid BPF object or Discord. This establishes a socket receiver boundary, not verified live sensor compatibility or authenticated application-health delivery.

### Owned Socket Progress Supervision

Linux/observe callers can use `progress::socket_owner::supervise(command,
listener, session, source, limits, cancel, health_sender)` to own one direct
child and its verified progress connection. The caller supplies an already-bound
listener and configures its child to connect; this API does not create a public
endpoint, negotiate a session or activate a collector. Session/source expectations
must be trusted owner inputs. The child's stdin/stdout/stderr are discarded, and
kill-on-drop is enabled. No inherited stdout can masquerade as socket progress.

`Limits` requires total in (0, 65 seconds], idle in (0, total], and accept in
(0, min(2 seconds, total)]. Invalid policy/session and already-ready cancellation
return before spawning. After spawning, acceptance requires the parent's effective
UID and the exact owned child PID reported by kernel peer credentials. A foreign
connection fails closed without retry or payload consumption. Commands that change
UID or delegate connection to another process do not satisfy this contract.

The total budget starts immediately before spawn and includes acceptance. Reception
uses only the remaining budget; acceptance cannot restart total lifetime. The
acceptance limit is independently bounded, and its expiry is reported as an idle
deadline unless total was the limiting budget. After acceptance, only complete
validated progress records refresh reception idle. Successful completion requires
socket EOF, an actual awaited child exit and terminal/exit agreement. A validated
faulted terminal run is `collector_fault`, not completed collection.

The report uses the existing progress supervisor shape. Failures return no summary,
revoke health before cleanup, and kill/await an unreaped child with a separate
two-second cleanup deadline. Failure to reap overrides the outcome with
`cleanup_failed`; a triggering total/idle deadline remains recorded. Normal
completion reports the observed child as reaped. Dropping/aborting the owner future
closes owned handles, revokes health and requests child kill, but cannot guarantee
awaited reaping. Callers needing cleanup acknowledgement must cancel and await the
owner rather than abort it. Only the direct child is owned, not descendants.

Endpoint setup/security, namespace interpretation, inherited descriptor control,
server authentication and the trustworthiness of the expected child remain caller
responsibilities. Scheduling delays and synchronous spawn/credential operations
are not preemptible; uninterruptible OS behavior can exceed cooperative deadlines.
An owner-selected session is a binding, not a secret credential. The library adds
no privileged loader, socket-backed capture command, renderer service or live
compatibility claim.

Eight tests use real abstract sockets and disposable test-executable children.
They exercise generated completion, malformed/truncated progress, exit mismatch,
missing connection/EOF/exit, acceptance-inclusive total bounds, foreign PID,
pre-spawn gates, spawn failure and cancellation before/after health publication.
No valid BPF object, audio or authenticated Discord process is involved.

```sh
cargo test --features observe --lib observe::progress::socket_owner::tests
```

### Generated Socket Progress Replay

Linux builds with `observe` provide an explicit, no-options generated diagnostic:

```sh
cargo build --features observe
./target/debug/intellicord supervise-socket-progress-replay
```

The parent obtains 16 bytes of OS randomness and uses the resulting 32 lowercase
hex digits to bind a fresh Linux abstract Unix listener and select its expected
progress session. There is no deterministic fallback, configurable endpoint,
caller-selected executable, PID, object or capture duration. Entropy/bind failure
returns before spawning and emits no report. Abstract endpoints create no
filesystem entries and disappear when their owned handles close. Their names are
not secret or an access-control mechanism; unexpected peers fail the UID/PID gate
without retry and can cause denial of service.

The parent starts its own executable in `socket-progress-replay-child` mode with
an otherwise empty environment containing only `INTELLICORD_GENERATED_SOCKET_SESSION`.
That helper also takes no arguments, rejects missing/malformed bindings before
connecting, derives only the generated diagnostic endpoint from the binding, and
writes only synthetic progress. The binding is not a credential and is not server
authentication. The helper is an internal diagnostic, not a sensor or service.
Environment visibility, local namespace trust and the existing peer-gate limits
still apply. Abstract socket creation/connection use synchronous standard-library
setup and are outside a hard preemption guarantee.

The child shares the stdout replay's `Reporter` implementation: six lifecycle
records and two health samples emitted at the existing one-second cadence, with
20 ms polling that skips missed ticks and a 250 ms final observation window. The
owner uses six seconds total including acceptance, three seconds validated-record
idle and two seconds acceptance. SIGINT/SIGTERM request cancellation; the owner is
awaited through bounded cleanup. No BPF object is loaded or attached, no audio is
captured, and no Discord process is contacted.

The parent concurrently consumes the revocable latest-health subscription and
prints a final ten-field JSON report. It declares `source: generated_replay`,
`sensor_running: false`, `load_test_performed: false`,
`transport: kernel_verified_unix_socket` and `session_binding: parent_selected`,
alongside schema, the owned run and `observed_health`, `generated_only`,
`health_revoked` checks. It does not print raw sessions, endpoint names, PIDs or
individual samples. Exit success requires completed supervision and all three
checks. Scheduling delays can make this diagnostic fail; it is not a latency or
availability guarantee. A nonzero result with a final report is still a failure.

Executable tests cover repeated successful runs despite an inherited bogus
binding, exact report shape/provenance, rejected arguments/bootstrap, unsupported
builds, sample-triggered cancellation/reaping and foreign-session rejection.
This verifies the generated socket producer/owner/subscriber path, not live
collector compatibility, privileged isolation or renderer delivery. Production
endpoint policy, a socket-backed capture command and client health handoff remain
future work.

```sh
cargo test --features observe --test socket-progress
```

### Parent-Verified Socket Capture Transport

Linux/observe adds a separately acknowledged collector transport for an owning
same-user parent. Its exact argument shape is:

```text
intellicord-observe capture-socket-progress --object PATH --pid PID --seconds 1..60 --session HEX32 --socket ABSOLUTE_PATH --parent-pid PARENT_PID --acknowledge-socket-progress
```

This is a child command for a process that already owns a Unix listener, not a
standalone supervisor. The supplied parent PID must be the collector's actual
parent. A shell launching a child cannot name some other listening service as its
parent. The command accepts no event export, clock handshake, status-JSON option,
arbitrary transport flags or legacy capture acknowledgement. Policy, target,
duration and session validation precede connection or object access.

Before connecting, the collector requires an absolute pathname whose immediate
parent is a real directory owned by its effective UID with mode 0700. The endpoint
must be a same-UID Unix socket, not a symlink or regular file. It connects through
Tokio with a one-second timeout, then requires kernel-reported server UID equal
to its effective UID and server PID equal to the expected actual parent. Actual
parent identity is checked again after connecting. No progress bytes are sent
until these checks pass. This does not support a different-UID privileged loader
or a service reached through process delegation.

The parent remains responsible for creating a fresh private directory and endpoint,
choosing a fresh session, keeping its identity/lifetime stable, accepting only its
owned child's kernel UID/PID, and managing cancellation and kill/reap cleanup. The
existing `socket_owner::supervise` supplies the child acceptance, receive deadlines,
exit agreement and cleanup, but callers must create/remove their own filesystem
directory/socket. There is no new parent CLI or automatic endpoint creation here.
Endpoint metadata checks are not atomic filesystem pinning: ancestor symlinks,
namespace changes, replacement races, same-UID compromise and descriptor transfer
remain owner trust concerns. Post-connect credentials are the final server gate,
not an executable attestation or general authentication scheme. Filesystem metadata
operations and scheduling are outside cooperative timeout preemption.

After verification, the socket uses the same bounded `Reporter`, one-second
periodic health cadence, 100 ms per-envelope write/flush deadline, lifecycle and
detach/drain behavior as `capture-progress`. The initial validating record must
be delivered before object inspection or sensor setup. Pre-load faults are emitted
as session-bound faulted lifecycle records; normal capture output and summaries
never go to stdout. Stdout remains empty, stderr retains ordinary errors, and
dropping the finished/failed reporter closes the connection. The owner must use
actual child exit plus EOF and strict terminal agreement; socket closure alone
does not establish success. Failure while delivering progress follows the shared
collector teardown path. The stdout `capture-progress`, legacy capture and existing
`supervise-progress-capture` commands retain their previous behavior.

Six executable tests use disposable 0700 directories and real pathname sockets.
They cover malformed arguments and consent before connection, unsafe/missing
endpoints, wrong actual parent, a same-UID foreign server PID receiving no progress,
fragmented decoding of an invalid-object fault, empty stdout, foreign-session
rejection and actual socket-owner reaping with no published health. All collector
tests use `/dev/null` as the object and fail inspection before BPF loading. No
valid-object socket capture, live signal/error teardown or sustained compatibility
is claimed. No authenticated Discord process or audio is involved. Production
endpoint ownership, a parent capture command and renderer health delivery remain
future work.

```sh
cargo test --features observe --test observe-socket-cli
```

### Parent-Owned Socket Capture

Linux/observe provides an owning command with separate explicit consent:

```text
intellicord-observe supervise-socket-progress-capture --object PATH --pid PID --seconds 1..60 --acknowledge-supervised-socket-progress
```

Only the exact object, positive target PID, 1..60 second duration and acknowledgement
are accepted. The caller cannot override the session, endpoint, child executable,
parent identity or deadlines. Legacy capture/progress acknowledgements do not
authorize this command. Argument and policy errors occur before session selection,
endpoint creation or child spawn and produce no stdout report. Valid-object use
can load and attach BPF; it remains a separately deliberate operator action under
the permissions and trusted-object constraints described above. No startup or
renderer path invokes it automatically.

After registering SIGINT/SIGTERM cancellation, the parent selects a fresh 128-bit
OS-random session without fallback. `progress::endpoint::Endpoint` canonicalizes
the process's temporary root (`std::env::temp_dir()`, normally `/tmp`, or the
operator's `TMPDIR`) and exclusively creates `ic-progress-HEX32` with mode 0700.
It verifies effective-UID ownership and the actual mode, exclusively binds
`progress.sock` inside it and retains the directory/socket device-and-inode
identities for cleanup. Existing paths are never adopted or overwritten, and
collisions fail rather than retry with an existing directory. The supplied
temporary root and its ancestors must be trusted; environment selection is not
a privileged-service configuration mechanism. Long temporary paths can exceed
the Unix socket pathname limit and fail before spawn; use a short trusted root.

The parent launches only its current executable in `capture-socket-progress`
mode with an empty environment and fixed child consent, session, socket path and
actual parent PID. It uses the existing socket owner with total duration plus
three seconds (4..63 seconds), three seconds validated-record idle, two seconds
acceptance and separate two-second kill/reap cleanup. The child independently
verifies its parent and server credentials, while the parent verifies the child's
kernel UID/PID. EOF, actual child wait and strict terminal/exit agreement remain
required. SIGINT/SIGTERM cancel and await supervision rather than aborting it.
The health channel stays internal; no live renderer or standalone service is added.

Once supervision returns and closes its listener/connection, the parent explicitly
removes the owned endpoint. Cleanup checks directory and socket identities, removes
only the matching socket and empty matching directory, and never recursively
removes contents. Replaced entries and unexpected contents are preserved and
reported as cleanup failure. Drop provides best-effort cleanup on ordinary setup
errors; it is not an acknowledgement. These pathname checks are not race-free
descriptor-relative filesystem pinning and do not defend against a compromised
same-UID process or namespace/ancestor replacement. Abrupt process termination,
SIGKILL, host failure and uninterruptible OS work can leave files or exceed bounds;
this is not a persistent privileged service or crash-recovery mechanism.

The final JSON has six fields: schema, `source: supervised_kernel_socket_progress`,
`transport: kernel_verified_unix_socket`, `session_binding: parent_selected`,
`endpoint_removed` and the existing owned `run`. It does not expose raw sessions,
endpoint paths, target PIDs, object paths or interim records. `endpoint_removed`
acknowledges filesystem cleanup independently of `run.collector_reaped`, which
acknowledges process cleanup. Success requires both completed supervision and
successful endpoint removal. Failure to remove the endpoint leaves the child run
unchanged, reports `endpoint_removed: false` and exits nonzero; failure to reap is
still represented by the run's `cleanup_failed` outcome. Setup failure before
supervision emits no report. A typed collector fault is never successful capture.

The progress summary retains the validated terminal `ProcessHealth` as
`terminal_health`, including accepted, rejected-record, rejected-clock and ring-drop
totals. This is final loss accounting after terminal lifecycle and child-exit
agreement, not a claim that no events were lost before reaching those counters.

### Offline Progress Report Evaluation

Completed final socket reports can be retained by an operator as one JSON object
per line and evaluated without starting a sensor:

```sh
./target/debug/intellicord-observe evaluate-progress-reports --seconds 5 < reports.ndjson
```

The declared duration must be 1..60 seconds and must match the duration requested
for every supplied run. Input is capped at 64 KiB, 60 reports and 4096 bytes per
report. Strict decoding accepts only the six-field socket report above, a completed
and reaped run with no owner deadline, a six-record normally stopped kernel
lifecycle, confirmed attach/detach and endpoint removal. It also requires exact
record conservation (`records = lifecycle records + health samples`), terminal
health, JavaScript-safe counters and at least `seconds - 1` periodic samples per
run. Unknown fields, malformed/truncated data and inconsistent runs reject the
whole evaluation.

Output contains only aggregate run/sample/counter totals and confirmation booleans;
it contains no session, PID, path, endpoint, timestamp or event record. Nonzero
ring drops and rejected records remain explicit totals rather than being silently
treated as a successful zero-loss result. This command evaluates already captured
reports only. It does not load BPF, attach a tracepoint, generate load, prove event
completeness, validate the operator-supplied duration, or test SIGKILL recovery.
Sustained live runs and overload injection therefore remain separate, explicitly
approved operator actions.

Five endpoint unit tests cover mode/ownership, explicit and drop cleanup, collision
preservation, failed-bind cleanup, and refusal to delete replaced entries or foreign
contents. The socket CLI suite now has nine tests, including parent consent/override
gates, setup failures and minimum/maximum-duration `/dev/null` faults with a reaped
child and no endpoint leftovers. Cleanup-refusal behavior is unit tested; a final
CLI `endpoint_removed: false` report and live signal delivery are not fault-injected.
No valid BPF object, audio or authenticated Discord process is involved in this
validation. Valid-object socket collection and sustained compatibility remain
unverified; client live-health delivery is still future work.

```sh
cargo test --features observe --lib observe::progress::endpoint::tests
cargo test --features observe --test observe-socket-cli
```

### Reusable Socket Capture Owner

Linux/observe callers can now use `progress::capture::Capture::prepare` to obtain
an owned capture and its `HealthReceiver` before spawning the collector. The
existing `supervise-socket-progress-capture` CLI uses this same API; its explicit
acknowledgement, fixed current executable, report shape and exit policy remain
unchanged. This removes the CLI-only ownership boundary without adding another
collector implementation or automatically activating observation.

Preparation requires a trusted absolute collector executable path, a trusted
endpoint root, a nonempty object path, a positive signed-range PID and a duration
of 1..60 seconds. Invalid policy is rejected before filesystem creation.
Preparation selects a fresh random binding, creates the private endpoint and
constructs the empty-environment command, but does not spawn it or open the BPF
object. The returned subscription initially has no sample. The executable path
is a trusted backend input, not renderer-selected or attested; an arbitrary
executable can falsify the protocol. Callers must establish explicit authorization
before preparation. The library does not implement a consent UI or privilege
separation.

`Capture::run(cancel)` consumes the owner and applies the existing peer-verified
socket supervision, receipt-age subscription and total/idle/acceptance limits.
Only polling that future can start a child. Ready cancellation prevents spawn;
dropping the prepared owner or an unpolled run future closes the subscription and
performs best-effort endpoint cleanup. After a polled run completes, explicit
endpoint removal is reflected independently of child reaping in the same
six-field report. `Report::completed()` requires both completed supervision and
endpoint removal. A dropped active future still has only the underlying
kill-on-drop and best-effort filesystem behavior, not an awaited reaping guarantee.
All existing same-UID, namespace, pathname-race and scheduling limits apply.

Four additional socket-suite tests cover policy rejection, prepared/unpolled
drop, ready cancellation, actual invalid-object child reaping, spawn failure,
closed/empty subscriptions and cleanup refusal preserving foreign contents.
No valid BPF object is loaded. Active kernel sampling through this API remains
unverified; generated active socket delivery is covered separately. The explicit
injector integration below uses this owner; ordinary startup remains disconnected
and no renderer control can activate it.

### Bounded Client Health Handoff

`progress::client::Snapshot::read` converts an optional owner health subscription
into six allowlisted fields: `schema_version`, `source`, `status`,
`sensor_running`, `valid_for_ms` and `sample`. The source is always
`owner_progress_subscription`; `sensor_running` is always false. A recent sample
is not proof of continuing attachment, collection success or sensor liveness.
Only kernel-tracepoint samples with schema 1 and JavaScript-safe unsigned counters
are eligible. Generated input is explicitly rejected even when its socket peer
was kernel-verified. No PID, session binding, endpoint, object path or history is
exported.

The statuses are `not_connected`, `no_recent_sample`, `generated_rejected`,
`invalid_sample` and `recent_kernel_sample`. Only the last carries the five-field
process health sample and a positive remaining lifetime. Its budget is the
remainder of two seconds since the owner's validated receipt, rounded down to
whole milliseconds, not a new two-second lifetime at serialization. Stale,
future-dated, revoked or sub-millisecond samples carry no counters.

The review accepts this field only in a matched host diagnostic response for the
current session. It validates exact fields, provenance, counters and lifetime,
copies allowlisted values and conservatively deducts time since the diagnostic
request began. A local timer removes expired counters; each read independently
checks the monotonic deadline. Background timer throttling or a blocked renderer
can delay the visible repaint. Invalid input, a new check, close, disposal and
session replacement clear the handoff. Clock regression fails closed. Historical
supplied reports and the existing sensor status remain independent.

Backend revocation prevents subsequent subscription reads immediately; a copy
already sent to the renderer cannot be recalled and may remain until its bounded
deadline or an earlier clear. This is not authenticated continuous IPC or an
instantaneous remote-revocation guarantee.

**Connection boundary:** ordinary injection serializes `Snapshot::read(None)`
at diagnostic send time, explicitly reporting `not_connected`. Only the separately
acknowledged injected-observation mode below connects an owner subscription. Tests
cover contract shape, remaining age, revocation, generated socket rejection,
strict renderer validation, local expiry and state clearing without loading BPF,
capturing audio or attaching to Discord.

### Explicit Injected Observation

Linux builds with the `observe` feature now accept an all-or-nothing injection
policy: `--observe-object PATH --observe-pid PID --observe-seconds 1..60
--acknowledge-injected-observation`. Missing, duplicate, malformed, unsupported or
unacknowledged options fail before DevTools access or endpoint creation. The PID
is explicit, not inferred from Discord or selected by the renderer. Ordinary
`inject`, the installed desktop `launch`, diagnostics and the preview never
enable this mode. The same complete policy may be passed to an explicit CLI
`launch` invocation; launch fixes the DevTools port to 9222 and selects the
Discord target automatically. It refuses to silently add or discard a policy
when another Intellicord launcher already owns the running session.

The following is an operator-only invocation, not a validation command. Build
both binaries together and deliberately review the object, PID and permissions
before use:

```sh
cargo build --locked --features observe --bins
./target/debug/intellicord inject --port 9222 \
	--observe-object PATH_TO_TRUSTED_BPF_OBJECT \
	--observe-pid PID --observe-seconds 10 \
	--acknowledge-injected-observation
```

Replace `inject --port 9222` with `launch` to start Discord and carry this policy
into automatic attachment. The desktop entry supplies no policy and remains
sensor-free.

For caller-selected object paths, the injector uses only `intellicord-observe`
next to its own executable; there is no PATH, environment or renderer override
for that unprivileged collector. The ordinary desktop installation remains
sensor-free. `./scripts/install-user.sh --with-live-sensors` explicitly builds
the host with `observe`, runs the pinned build-and-inspect gate, and stages
compatible user-owned artifacts without granting privileges.

Linux hosts that disable unprivileged BPF can use a separately reviewed broker.
`sudo ./scripts/install-bpf-helper.sh` installs root-owned, group/world
non-writable copies at `/usr/libexec/intellicord/intellicord-bpf-helper`,
`/usr/libexec/intellicord/intellicord-observe`, and
`/usr/lib/intellicord/intellicord-process`, then grants only the helper
`CAP_BPF`, `CAP_PERFMON`, and `CAP_DAC_READ_SEARCH`. The third capability permits
Aya to traverse and read root-only tracefs event metadata before attaching; it
does not replace the perf/BPF capabilities. The general observer and authenticated injector have
no capabilities. The helper accepts exactly one socket-progress command with no
object or executable argument, requires its claimed parent to equal `getppid()`,
requires the target process to have the caller's effective UID, checks all three
system artifacts are protected root-owned regular files, raises those two
capabilities into its ambient set, clears the environment, and `exec`s the fixed
collector with the fixed object. `exec` preserves the child PID, so the existing
kernel peer-credential check, parent-owned deadlines, cancellation and awaited
reaping remain unchanged. The injector selects this broker only when the explicit
object is exactly `/usr/lib/intellicord/intellicord-process`; arbitrary paths
remain unprivileged. `sudo ./scripts/uninstall-bpf-helper.sh` removes the system
artifacts but does not stop a running collector.

These capabilities permit sensitive kernel instrumentation and bypass ordinary
filesystem read/search checks, so the helper must be treated as privileged code,
even with the narrow command surface. Its fixed root-owned artifacts, exact
arguments and cleared environment prevent caller-selected paths from using that
filesystem capability. The helper does not
provide cross-UID observation, arbitrary bytecode, arbitrary execution,
elevation prompts, capability fallback, a daemon, or automatic activation. It
does not protect against compromise of root, the kernel, or the reviewed fixed
collector/object. The desktop entry still supplies no sensor policy. Do not run
the authenticated injector as root.

Only after client injection succeeds does the backend prepare the private owner
and run capture alongside tracking. The existing explicit diagnostic request
serializes the optional receiver at send time, preserving its original receipt-age
budget and kernel-source checks. Health counters remain sampled diagnostics,
not a pushed live-health stream; a slow diagnostic round trip may consume their
entire budget. Owner outcomes also reach the automatic session view described
below, independently of diagnostic requests.
Historical supplied reports, participant evidence and sensor-liveness claims
remain independent. Recent kernel counters still do not establish liveness.

Optional preparation failure does not detach baseline tracking. If executable
resolution or private-owner preparation fails, the injector emits a bounded
warning, continues processing voice snapshots and diagnostics, and makes no retry
or elevation attempt. Diagnostics report owner `unavailable` with no result and
health `not_connected` with no counters. This is not a successful owner run or a
cleanup acknowledgement; partial preparation has only its existing best-effort
endpoint guard cleanup. Malformed or unacknowledged activation options still fail
before attachment, and ordinary injection remains sensor-free.

After this degraded session ends normally, the injector returns the original
setup error and exits nonzero. If tracking itself fails, its error is preserved
with context noting that observation was unavailable. Client disposal and binding
removal still run on normal return, reload and handled signals. Tests use missing
and regular-file temporary roots, require a voice departure after diagnostics,
check reload/SIGINT/SIGTERM teardown, preserve sentinel contents and verify no
endpoint leftovers. These checks do not establish valid-object live compatibility
or prove cleanup after arbitrary partial filesystem failure.

`Capture::run_with(work)` lets tracking continue after capture reaches its bounded
end or fails; no capture is restarted. When tracking returns, including reload,
disconnect or error, the helper requests cancellation and awaits the existing
direct-child kill/reap and endpoint-removal path before client disposal and binding
removal. Unix SIGINT/SIGTERM handlers are registered before tracking can yield to
collector startup. The final six-field owner report is printed to stderr on detach,
not immediately when capture ends. Cleanup uncertainty or a collector fault causes
a nonzero injector exit; session views and explicit diagnostics show the bounded owner outcome
described below independently of an empty health subscription. A generic tracking failure is preserved when the
owner ends normally or is cancelled. Panic, task abort, SIGKILL and host failure
still lack awaited teardown guarantees; synchronous OS work can exceed cooperative
deadlines.

Tests run the real injector against a fake loopback DevTools server, checking
ordinary `not_connected` versus opt-in `no_recent_sample`, invalid-object reaping,
endpoint removal and UI/binding teardown after reload, SIGINT and SIGTERM. A
separate connected silent test child proves `run_with` awaits reaping when work
fails; CLI tests verify rejected flags make no DevTools connection. These tests
load no valid BPF object and touch no authenticated Discord process. Broker tests
pin its exact no-object argument shape, distinct acknowledgement, parent check,
general-command rejection, protected installation targets, and the invariant
that capabilities are never assigned to the general observer. They do not use
capabilities or BPF syscalls. Valid privileged collection, recent kernel samples
through this integration and live compatibility remain unverified. The existing
Unix peer check is local parent authentication, not remote attestation.

An explicitly approved combined release run on 2026-11-21 exercised the
unprivileged owner
against the running Discord process and the exact inspected object. The child
started, passed preflight and reached kernel load, then reported `load_failed`
under the host's unprivileged-BPF policy. The parent retained a three-record
fault lifecycle with no health sample, confirmed child reaping and endpoint
removal, and detached the renderer after the concurrent acoustic run. This
validates the production failure and cleanup path only. It does not validate
attachment, event delivery or recent counters. The constrained broker was added
after this run and has not yet been provisioned or exercised live. Granting or
removing its capabilities remains a separate administrator action; Intellicord
does not request elevation or run the authenticated injector as root.

After the constrained helper was provisioned, an approved retry on 2026-11-22
first proved BPF load but exposed root-only tracefs metadata at attach. Adding
`CAP_DAC_READ_SEARCH` to the fixed helper allowed Aya to read that metadata
without granting `CAP_SYS_ADMIN`. The next five-second run completed: ten progress
envelopes contained six lifecycle records and four health samples, terminal phase
was `stopped` with `duration_limit`, and both attachment and detachment were
confirmed. The collector reaped, its private endpoint was removed, the renderer
detached, and no helper, observer, injector or recorder process remained. This
validates one bounded production ownership path on the reference host. It does
not prove every exec was observed, zero ring loss under load, long-running
stability, crash cleanup, PID-generation identity, source-clock synchronization,
participant attribution or compatibility on other kernels and Discord builds.

The packaged three-path command is:

```sh
~/.local/bin/intellicord launch \
	--observe-object /usr/lib/intellicord/intellicord-process \
	--observe-pid PID --observe-seconds 30 \
	--acknowledge-injected-observation \
	--acoustic-sink-serial SERIAL --acoustic-seconds 30 \
	--acknowledge-injected-output-capture --defer-acoustic-start
```

This single owner session coordinates Discord tracking, the bounded process
collector and mixed-output processing. `PID` and `SERIAL` remain deliberate
operator selections. Process and acoustic evidence stay independent; this does
not add clock calibration, participant attribution, source identity or automatic
reconnect. Do not infer sustained live compatibility from packaging tests.

### Sampled Owner Outcome

The optional `process_owner` diagnostic has exactly four fields:
`schema_version: 1`, `source: capture_owner_outcome`, `status` and `result`.
Ordinary injection reports `not_configured`. A prepared owner reports
`awaiting_completion`, which does not establish that a child started, attached or
is currently active. Once supervision and explicit endpoint cleanup finish, the
owner publishes `finished` with exactly four result fields: typed `outcome`,
`collector_started`, `collector_reaped` and `endpoint_removed`. Endpoint cleanup
failure does not overwrite the supervisor outcome or child-reaping fact. No PID,
path, session binding, health counter or sensor-running flag is included.

The owner stores one snapshot, not an event queue. A finished snapshot survives
publisher closure; a closed publisher with no final snapshot reads `unavailable`,
not success or confirmed cleanup. Prepared drop and active-future abort remain
best-effort cleanup, never a final acknowledgement. There is no heartbeat or
pending-state freshness promise. Final results describe that single completed
run, not later filesystem state or sensor liveness, so they do not use the
separate health sample's two-second expiry.

Injection serializes the outcome at diagnostic send time. The review accepts it
only within a matched host response, validates exact keys, finite outcome names,
boolean types and child-start/reap consistency, and displays outcome and the two
cleanup confirmations independently. It clears on a new check, malformed or
missing data, close, disposal and session replacement; preview responses cannot
supply owner outcomes. An older backend that omits the field simply has no owner
outcome section. This is last-explicit-check visibility, not pushed status or
instantaneous failure notification. Existing live-health and historical-report
claims are unchanged.

### Automatic Session Integration

The injector also embeds the same strict owner snapshot at
`intelligence.process_owner` in its existing view updates. It publishes an initial
view before waiting for voice input, then watches owner changes so completion or
publisher loss reaches the client without a prerequisite check, voice transition
or five-second heartbeat. Closed subscriptions are disabled rather than polled.
This carries one bounded owner state, not a stream of process events or health
counters. DevTools delivery is still cooperative, not a hard notification deadline.

The review shows this state under **Session process observation**, independently
of its last-explicit-check section. Missing, malformed or unsupported session data
clears the automatic state; replacing the session cannot retain the previous
owner. Closing and reopening the review retains the current session's status.
Session Health also summarizes process observation: not configured, waiting,
unavailable, or finished. Faults and unconfirmed cleanup are errors; clean completed
or cancelled runs are ready. Neither pending nor ready asserts sensor liveness.
No renderer control starts or restarts a collector.

Every session view now also includes a strict `session_sensors` aggregate derived
by the backend from this process owner and the acoustic owner. Its seven fields
contain schema/source, a `sensor_free`, `pending`, `settled` or `attention` state,
configured/pending/finished owner counts, and combined cleanup state. The renderer
validates count and state consistency before showing **Combined sensor ownership**
and a Session Health verdict. It retains the detailed owner rows for diagnosis.
The aggregate contains no PID, endpoint, sink, sample, identity, clock or error
text and makes no process/acoustic liveness or cross-sensor correlation claim.

The same automatic view now carries `intelligence.process_progress`, read from
the capture owner's already verified health subscription. A cloned receiver is
watched independently of terminal owner changes, so a validated health sample or
its revocation prompts a view update without a diagnostics request or voice
transition. The existing six-field snapshot remains unchanged: only four bounded
counters can be present, `sensor_running` is always false, and freshness is at
most two seconds from backend receipt. Sensor-free sessions report
`not_connected`; configured sessions without a recent accepted kernel sample
report `no_recent_sample`.

Safety & Intelligence keeps this automatic lane separate from **Owner progress**
returned by an explicit prerequisite check. The renderer deducts transport time
from the supplied remaining budget, expires counters locally, clears malformed
or replacement-session input, and labels liveness **Not established**. Session
Health applies the same strict handoff and expiry. These counters are not copied
into observations, incidents, participant evidence or exports, and they do not
identify a process, endpoint, user or acoustic source. Cooperative DevTools view
delivery can delay or omit updates; a recent sample is not a heartbeat.

The fake-DevTools integration test now sends actual injector-produced view JSON
through the production menu renderer in a bounded-memory Node/LinkeDOM child. It
asserts visible status, fault, child reaping, endpoint cleanup and feature status
before requesting diagnostics, then checks subsequent voice history and handled
teardown. This test requires `npm ci` before Cargo tests, as does the documented
client build. It uses no authenticated Discord process or valid BPF object and
does not validate PipeWire, real kernel collection or cross-sensor correlation.

Tests verify closed/finished subscription behavior, publication while tracking
continues, fake-DevTools visibility before detach, malformed and stale response
rejection, independent cleanup labels and state clearing. No valid BPF object or
authenticated Discord process is involved; live collection compatibility remains
unverified.

## Controlled Child Event Delivery

The next separately authorized kernel test observes only a disposable same-executable child, not Discord or an externally supplied process:

```sh
./target/debug/intellicord-observe verify-delivery \
	--object crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process \
	--acknowledge-bpf-delivery
```

This command requires appropriate BPF/perf permissions and a reviewed binary/object. A privileged invocation is an explicit operator decision; never run Cargo or the injector as root. The command accepts exactly four arguments, no PID, executable path, duration, event export or status-stream options. Neither client diagnostics nor CI invokes a valid-object probe.

After bounded offline object inspection, the command creates fresh unpinned maps and loads the tracepoint program. It starts the same observation executable in the fixed `delivery-child` mode with an empty environment and piped readiness/release channels. That helper emits one readiness byte and waits up to five seconds for release. Only after readiness does the parent set/read back its owned child's PID, check a pidfd lifetime guard, attach `sched/sched_process_exec`, recheck the guard and release the child. The helper uses exec, not another spawn, to become the same executable's harmless `delivery-child-exit` mode. No arbitrary command is accepted or shell used.

The parent polls at most 64 ring records per ten-millisecond iteration. Success requires exactly one valid native 24-byte leader-exec record for that child (`TGID == TID == child PID`) with a kernel timestamp between the parent's pre-release and receipt `CLOCK_MONOTONIC` readings, plus observed child exit. Wrong schema/length/identity/time or extra records fail closed. The child remains unreaped until after the target map is cleared, explicit detach is attempted, and owned BPF/map handles are dropped; keeping the exited child unreaped prevents its PID being recycled while the numeric filter is active. The parent then checks/reaps the child's exit and requires success. Error paths kill and await their own child. There is no external process selection or participant attribution.

Readiness and event delivery each have a two-second asynchronous deadline; release writing has 100 ms and direct-child cleanup a separate two seconds. The helper's five-second release deadline is a fallback if the controller disappears before release. Synchronous kernel setup, map operations and detach are outside these bounds; this is not a hard end-to-end deadline. Cancellation/task abortion retains kill-on-drop fallback, not guaranteed reaping or a terminal report. Privilege separation, crash cleanup, resource containment and time/PID namespace authentication remain unfinished. The clocks must share the kernel's monotonic interpretation; namespace offsets are not measured.

The final JSON is smaller than 1024 bytes with twelve fields: `schema_version: 1`, `source: explicit_kernel_delivery_probe`, `outcome`, `program_loaded`, `child_started`, `child_reaped`, `attachment_confirmed`, `detachment_confirmed`, `event_verified`, `target_cleared`, `owned_handles_dropped`, and `sensor_running: false`. Flags describe completed operations, not ongoing liveness or independently verified global kernel removal. In particular, `event_verified` alone does not mean the whole probe passed. No PID, executable/object path, absolute clock, record bytes or raw errors are serialized.

Only `outcome: verified` exits zero. Typed failures are `object_unreadable`, `object_rejected`, `map_setup_failed`, `program_load_failed`, `child_failed`, `target_setup_failed`, `attach_failed`, `trigger_failed`, `delivery_timeout`, `event_rejected`, `read_failed`, `ring_drops`, and `cleanup_failed`. A final post-detach ring check rejects extra records, and successful verification requires zero summed per-CPU reservation drops. Cleanup failures override the earlier outcome while preserving operation flags; detach and reaping are never confirmed without successful operations. A zero drop count is not a general no-loss proof. The operator subsequently reported `verified` on the reference host, with expected event, child reaping, target clearing, detach and owned-handle disposal confirmed. This is one controlled delivery result, not sustained collection, overload or crash-cleanup validation.

### Generated Delivery Checks

```sh
cargo run --quiet --bin intellicord -- replay-process-delivery
cargo test --lib observe::delivery::tests
cargo test --features observe --test observe-cli delivery
```

The fixed replay uses the same event predicate as the live probe with seven generated cases: matching exec, foreign process, worker thread, pre-release event, post-receipt event, malformed record and backwards bracket. It never loads BPF or spawns a child. Safety & Intelligence displays **Delivery record checks**, **Failed delivery checks** and **Live event delivery: Not tested by this diagnostic**. Failed checks remain visible; malformed/active sources, extra fields, wrong order/count/types and stale sessions are rejected. Optional absence supports older backends; rate-limit clearing, explicit requests and exclusion from participant evidence/export are preserved. Preview values remain fixtures.

Unprivileged executable tests exercise the real helper's readiness, release, EOF/wrong-token failure and child exit, plus distinct-consent/no-external-target gates and pre-load file rejection. A generated waiting-child test exercises the actual cleanup function's kill/wait behavior with no BPF handles. These tests do not fault-inject live map/attach/read/detach errors or prove the complete kernel path. Existing build-only CI includes the new unit/executable tests through its observation and platform targets.

## Explicit Attach-Only Verification

After building the client and the optional reader, a separately authorized operator can test the next kernel boundary with the already-built, reviewed observation binary:

```sh
./target/debug/intellicord-observe verify-attach \
	--object crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process \
	--acknowledge-bpf-attach
```

This is a **real kernel operation**, not an offline check. Appropriate BPF/perf permissions or a separately approved privileged invocation are required. Never run Cargo or the Discord injector as root. No permission changes, elevation, installation or probe execution is automatic. Only the exact four arguments are accepted; the load-only acknowledgement, target IDs, event export, duration and status-stream options are rejected.

The probe reads and inspects the object once, creates temporary unpinned maps, loads `process_exec`, and reads `TARGET[0]` back as zero before attaching to `sched/sched_process_exec`. It never writes a target. The reviewed shared kernel filter treats zero as disabled, so the attached program still executes on exec tracepoints but should produce no selected records. This depends on trusted reviewed bytecode: metadata inspection and a zero map value cannot prove an arbitrary object's behavior. The probe immediately requests explicit detachment, drops all owned handles, then reports; it never opens or polls the event ring, selects Discord, or starts a collection loop. No application or authentication data is inspected.

One JSON record smaller than 768 bytes has thirteen fields: `schema_version: 1`, `source: explicit_kernel_attach_probe`, `outcome`, `map_setup_attempted`, `program_load_attempted`, `program_loaded`, `target_disabled_confirmed`, `attachment_attempted`, `attachment_confirmed`, `detachment_attempted`, `detachment_confirmed`, `sensor_running: false`, and `owned_handles_dropped: true`. Confirmation flags describe past successful operations, not ongoing liveness. `sensor_running: false` means no collection session remains; it does not mean no brief attachment occurred.

Outcomes are `object_unreadable`, `object_rejected`, `map_setup_failed`, `program_load_failed` (including missing/wrong program type), `target_not_disabled` (including failed map read), `attach_failed`, `detach_failed`, and `verified`. Each failure stops later operations; successful attachment is followed by a detach attempt. A failed explicit detach never claims detachment, even after handle disposal. Only `verified` exits zero; typed failure JSON plus nonzero exit is intentional. Paths, PIDs, verifier logs and raw errors are omitted. Setup errors are not specific permission diagnoses.

Synchronous setup/attach/detach has no hard deadline or asynchronous cancellation guarantee. `owned_handles_dropped` describes userspace disposal, not independently observed global kernel removal or crash cleanup. Detachment success does not prove event delivery, ring loss accounting, target lifetime safety or live collection cleanup. The operator subsequently reported `verified`, with target-disabled, attach and explicit detach confirmed, on Linux 6.18.58-1-lts. That establishes this object's attach-only compatibility on that host, not event delivery or general compatibility.

### Sensor-Free Client Checks

```sh
cargo run --quiet --bin intellicord -- replay-process-attachment
cargo test --lib observe::attachment::tests
cargo test --features observe --test observe-cli attach_only_probe
```

The real adapter and generated fake operations use the same lifecycle controller. Six ordered checks cover setup failure, load failure, disabled-target gating, attach failure, visible detach failure and successful explicit cleanup. Each case checks operation order, absence of later operations after failure, report flags, and driver destruction before reporting. This is adapter control-flow coverage, not fault injection into Aya or the kernel.

The fixed replay takes no arguments and returns a bounded generated report, not any live probe result. Safety & Intelligence includes it as optional `process_replay.attachment` data, with **Attachment lifecycle checks**, **Failed attachment checks** and an explicit **Live tracepoint attachment: Not tested by this diagnostic** row. Malformed/active provenance, wrong check names/order/types and unknown fields reject the replay; real failed checks remain visible. Missing data supports older backends; session/request/rate-limit invalidation and exclusion from participant evidence/export are preserved. Preview results are labeled fixtures. The existing read-only build CI runs these unit and executable gates, never a real attach command.

## Safe Generated Tests

```sh
cargo test --test observe-contract
cargo test --features observe --test observe-cli
cargo test --features observe --test observe-object
cargo test --features observe --test observe-target
cargo run --quiet --features observe --bin intellicord-observe -- replay
```

Replay emits a bounded, deterministic JSON report with `source: generated_replay`, `sensor_running: false` and health counts: two accepted events, two invalid/foreign records, one backwards clock and three simulated ring drops. The three drops are supplied test data, not measured kernel loss. No process inspection, device access or BPF syscall is needed for replay.

Safety & Intelligence's existing explicit prerequisite check also runs this generated decoder replay. Its separate **Process-event decoder replay** section labels every count synthetic. Unsupported or live-source replay claims are omitted, and session changes invalidate the displayed result. It does not launch the optional reader; replay health is not participant evidence and is not included in the intelligence evidence export.

## Backend Execution Context

`intellicord diagnostics` and the client's existing explicit prerequisite request now return readiness schema 3. It retains all v2 fields and adds `no_new_privileges` (`true`, `false` or `null`) and `seccomp` (`disabled`, `strict`, `filter`, `unknown` or `unsupported`). The report has thirteen fixed fields and remains under the executable test's 2 KiB bound. Unsupported platforms return null no-new-privileges and unsupported seccomp without Linux metadata access. The client accepts v1 and v2 as before, showing the new facts unavailable even if an old-version report carries extra unversioned fields.

Linux reads `NoNewPrivs`, `Seccomp` and the existing capability mask from the same at-most-64-KiB `/proc/self/status` read. Each field is parsed independently and must occur exactly once with its documented integer spelling: no-new-privileges 0/1 and seccomp 0/1/2. Missing, duplicate, malformed or oversized data produces null/unknown, never an inferred disabled state. Status text, names, IDs and raw masks are not included in the report. The preview supplies labeled fixtures; it performs no host read.

These are main-thread status facts for the checking backend's thread-group leader at read time. Other backend threads can have different restrictions; a separately launched or privileged loader can have a different context. No-new-privileges prevents acquiring new privilege across exec, not use of existing capabilities. Seccomp mode identifies disabled, strict or filter operation, not a filter's allowed syscalls or its effectiveness. A filter may permit or deny BPF-related operations. Neither a disabled mode nor an unset flag proves that any operation is permitted. Values are not combined into a loadability verdict or participant finding.

Safety & Intelligence shows **Restriction context**, **No-new-privileges** and **Seccomp mode** beside existing kernel permissions. Schema-v3 fields are mandatory and strictly typed; missing/invalid values reject the readiness report and clear stale displayed facts. Request/session/rate-limit and disposal protections are unchanged. Tests cover each valid mode/flag, unknown and independent malformed/duplicate fields, oversized status input, old-version compatibility, invalid-response/session clearing and the executable schema/argument gates. This is diagnostic plumbing, not seccomp installation, policy inspection, cgroup containment, privilege separation or a BPF load/attach test. It changes no host setting and never starts a sensor.

## Shared Kernel Record Contract

```sh
cargo run --quiet --bin intellicord -- replay-process-record
```

The actual Aya ring writer now calls the shared inline `no_std` `ProcessEvent::exec` constructor after reservation succeeds. Generated timed/exported transport records use that same constructor and a safe fixed-array native-endian encoder. Compile-time assertions pin the 24-byte structure and all five field offsets (0, 4, 8, 12, 16); event/map schemas and ring capacity are unchanged. The constructor fills schema/kind constants but does not validate caller-supplied identity or time. Kernel identity filtering and consumer validation remain separate required boundaries.

Four generated checks cover exact native bytes against an independent fixture, full-width identity/timestamp preservation, unaligned decoding, and rejection of invalid schema/kind/length/target/zero identities. Host contract tests additionally compare fields and independent byte assembly. Existing exporter, timed receiver, joint collection and supervised subprocess replays now consume the shared constructor output. Independent fixtures remain to detect matching encoder/decoder mistakes.

The command accepts no arguments and emits under 512 bytes: generated source, inactive flags, schema and named boolean checks only. Full-width IDs, timestamps and raw bytes are not serialized into the diagnostic report. The explicit client check includes optional `record_layout` data with **Record contract source**, **Record contract checks** and **Failed record checks**. Invalid provenance, active-operation claims, schema or ordered result shapes clear replay diagnostics; actual failed checks remain visible. Older backends may omit the report. Session/request guards and exclusion from participant evidence/export are unchanged; preview data is static.

This remains a native-endian same-host ABI, not a portable interchange format. The encoder does not inspect kernel memory or authenticate records. Offset checks and host replays do not establish BPF-target code generation, verifier acceptance, actual helper values, ring delivery or live compatibility. No sensor is loaded or attached by this command or client check.

## Shared Kernel Producer Replay

```sh
cargo run --quiet --bin intellicord -- replay-process-producer
```

The kernel tracepoint and this sensor-free replay call the same inline `no_std` producer function. It filters identities before attempting submission, submits selected identities through a statically dispatched sink, and requests exactly one drop-counter update after each failed selected submission. The Aya adapter retains actual ring reservation, timestamp-helper access, slot write/submit and per-CPU counter updates; the shared submission routine below controls their ordering and event construction. The event and map ABI are unchanged.

The generated sink has two fixed slots, not a byte-accurate simulation of the kernel ring. Six checks cover filtering before reservation, preserved leader/worker identities, capacity rejection, filtering while full, recovery after draining, and agreement between submission attempts, successes and drop notifications. The deterministic report has five attempts, three submissions and two drop notifications, no process identities or clock values, and is smaller than 1536 bytes including the accounting and reservation checks below. Extra CLI arguments are rejected before any sensor work.

Explicit client diagnostics include this optional `producer` report in the existing generated decoder replay. **Producer checks**, **Failed producer checks** and synthetic submission counters expose both passing and failing results. Source, inactive flags, schema, capacity, bounded counters and ordered case names are checked; malformed reports clear the decoder diagnostics. Older backends may omit the report. Session and request guards still apply. The standalone preview uses a static fixture, and these facts never enter participant evidence or exports.

These host checks validate shared control flow, not actual kernel ring capacity, map access, timestamp helpers, counter atomicity, BPF code generation, verifier acceptance or attachment. A drop notification means the producer called its accounting adapter, not proof that a map update succeeded. Real BPF-target and live validation remain separate prerequisites. The older decoder replay's synthetic ring-drop count remains an independent supplied fixture, not this producer's measured loss.

### Loss Accounting Contract

The kernel adapter and generated sink use the same `no_std` saturating increment. Both periodic and terminal reader snapshots use a shared saturating sum of per-CPU counts. `ProcessMonitor` retains the maximum total seen, not the sum of snapshots: repeated snapshots cannot double-count losses, lower readings cannot reduce the reported count, and later increases remain visible. Saturation at `u64::MAX` means **at least** that many accounted drops; it is not an exact total after overflow.

The producer replay includes an additive `accounting` list with six fixed checks: counter saturation, ordinary/empty per-CPU sums, aggregate saturation, snapshot high-water behavior, filtering with a saturated counter, and successful submission after draining despite saturation. Host tests exercise the same arithmetic called by the kernel writer and reader, including full-width overflow boundaries. Only case names and booleans are serialized; full-width fixture values never pass through JavaScript numbers. The complete producer report remains below 1536 bytes.

Client diagnostics show **Loss accounting source**, **Loss accounting checks**, and **Failed accounting checks**. Missing accounting data preserves older-producer compatibility; malformed lengths, ordering, case names or result types reject the decoder report and clear stale results. Failed checks remain visible, including when every check fails. The existing session/request guards and inactive-source requirements still apply; no sensor or participant evidence is added.

Arithmetic checks do not prove successful map lookup/update or coherent kernel snapshots. Per-CPU values are read separately while the producer may run; they are not an atomic global snapshot. Existing non-atomic per-CPU updates, CPU availability, target lifetime, and source trust are not established by this replay. A high-water mark cannot recover unobserved losses or prove continuity across map replacement/reset. Successful submissions remain possible when accounting saturates. Zero reported drops do not prove complete delivery, and reservation failures are distinct from decoder rejections, transport failures and retention evictions.

### Reservation Submission Contract

The actual Aya writer now calls the same inline, allocation-free `submit_reserved_exec` routine as a generated byte-storing sink. An absent reservation returns failure without calling the clock or submit closures. A present reservation samples the supplied clock once, constructs the unchanged native event, and consumes the reservation in one submission. Identity validation remains the outer producer's responsibility. The kernel still uses `bpf_ktime_get_ns` and Aya's actual slot write/submit operations; the helper does not replace or simulate those facilities.

The producer report adds six ordered `reservation` checks: `filtered_reservation`, `reserve_before_clock`, `single_submission`, `native_record_delivery`, `full_ring_preservation`, and `fresh_recovery`. Generated slots retain at most two 24-byte records. Independent native-byte expectations and decoder checks cover complete emitted records; failed reservations preserve existing bytes and do not advance the generated clock. Direct integration tests additionally cover full-width fields, a zero timestamp, rejected-reservation callback nonexecution and recovery.

In **Safety & Intelligence**, explicitly check Linux sensor prerequisites to see **Reservation test source**, **Reservation checks**, and **Failed reservation checks**. Missing data remains compatible with older producers. Invalid lengths, order, names or boolean types clear the replay; failed checks remain visible. Session/request guards and inactive provenance still apply, and the standalone preview is only a static fixture. Results are not participant evidence or exports.

Run the host checks without kernel privileges:

```sh
cargo test --test observe-reservation
cargo test --lib observe::producer::tests
cargo check --release --manifest-path crates/intellicord-ebpf/Cargo.toml
```

The release profile is required for the kernel crate's host `no_std` check because its development profile does not select aborting panics. Host checks do not prove BPF closure inlining/code generation, verifier acceptance, actual reservation capacity, helper values, kernel delivery, counter atomicity or cleanup. The BPF build workflow includes the new tests before its real target build, but that workflow was not dispatched here. `scripts/build-ebpf.sh` remains the explicit build-and-inspect gate; live compatibility and privilege separation remain unfinished. No sensor is loaded or attached by these checks.

## Shared Kernel Target Selection

```sh
cargo run --quiet --bin intellicord -- replay-process-selection
```

The tracepoint handler now calls the same inline `no_std` `selected_identity` helper exercised by this generated replay. It splits the Linux helper's packed TGID/TID value, accepts only an exact nonzero target TGID with a nonzero TID, and returns both numeric fields. A matching TID under a foreign TGID is rejected; a different nonzero TID under the selected TGID is accepted. A zero target disables observation. Invalid zero-thread identities are now filtered before reservation instead of producing records the userspace decoder would reject. Filtered identities do not increment ring reservation-drop counters. Event/map sizes, schema, native byte layout and ring capacity are unchanged.

Seven ordered checks cover disabled targets, foreign TGIDs, matching leaders, matching worker threads, zero processes, zero threads and full-width identity splitting. The command accepts no arguments and emits bounded schema-versioned JSON containing only generated provenance, inactive-operation flags and case names/booleans. It does not read process identities, make BPF syscalls or open a target. Host contract tests additionally assert exact returned identities, including boundary values; the shared crate remains `no_std` for the kernel build.

The existing explicit client diagnostics attach an optional `selection` list to the generated decoder replay. **Target selection source**, **Target selection checks** and **Failed target selections** distinguish shared-filter tests from kernel capture. Failed checks remain visible; wrong names/order/count or non-boolean results reject that decoder report. Older backends may omit the list. Existing request/session/rate-limit clearing applies, and no diagnostic enters participant evidence or exports. The preview is a static fixture.

Calling shared code improves test coverage of the actual selection decision, not its execution environment. Host checks do not prove BPF code generation, verifier acceptance, the helper's runtime value, namespace compatibility, attachment, ring behavior or cleanup. Numeric TGID reuse and namespace/selection races remain; this is not executable identity or generation binding. No BPF load/attach test or privileged tool installation is performed by this command or client check. Real target compilation and live compatibility remain unverified.

## Machine-Readable Lifecycle

```sh
cargo run --quiet --features observe --bin intellicord-observe -- replay-lifecycle
```

This safe command emits six deterministic newline-delimited JSON records on stdout. It opens no process handle and performs no BPF operation. Records are version 1 with `record_type: process_status`, `source: generated_replay`, consecutive `sequence` values starting at one, and phases `validating`, `loading`, `attaching`, `observing`, `stopping`, `stopped`. Only the final record has a reason (`duration_limit`) and generated decoder health. Both operation-confirmation flags remain false throughout, including the simulated observing/stopped phases.

The client's explicit diagnostic check displays these records in **Process lifecycle replay**, separate from event evidence. It rejects changed provenance, order, schema, reasons, malformed health and operation confirmations. Rate-limited/invalid matching responses and session changes clear previous results. The preview labels its outer response as a fixture. Neither client path starts the reader, streams live statuses, chooses a target or includes diagnostic records in participant evidence exports.

For a separately reviewed and explicitly authorized live invocation, append `--status-json` to the existing complete capture command, after `--acknowledge-process-observation`. All object, PID, duration and acknowledgement gates remain mandatory. This optional flag replaces human stderr with NDJSON; the final successful health report on stdout retains its existing schema. Invalid JSON-mode capture requests return a typed `invalid_request` fault without loading or attaching. Without the flag, human diagnostics remain available.

Each status has exactly nine fields: `schema_version`, `record_type`, `source`, `sequence`, `phase`, `reason`, `attachment_confirmed`, `detachment_confirmed`, `health`. There are no IDs, paths, timestamps, diagnostic strings, participant data or event payloads. Capture records use `source: kernel_tracepoint`, including pre-load validation failures: source names the requested adapter, not proof it ran. `attachment_confirmed` becomes true only after successful attachment; `attaching` alone is not confirmation. `detachment_confirmed` becomes true only in the normal `stopped` record after successful explicit detach and final accounting. A fault never asserts detachment, even if owned unpinned Aya handles have already been dropped. The flags describe past operations, not current liveness or independently verified cleanup.

Nonterminal reasons are null. Normal terminal reasons are `duration_limit`, `interrupt`, `terminate`, `target_exited`; typed fault reasons are `invalid_request`, `invalid_object`, `target_unavailable`, `signal_setup_failed`, `load_failed`, `attach_failed`, `input_rejected`, `read_failed`, `delivery_failed`, `detach_failed`, `cancelled`, `internal_failure`. Reasons use typed error context, not string parsing. `delivery_failed` covers event-export writer creation, write, flush or acknowledgement failure; unclassified setup/status-output errors use `internal_failure`. Fault health can be null; do not treat it as zero loss or complete accounting. Observing/stopping/stopped records carry bounded counters, not participant evidence. There is no periodic heartbeat.

Every record is smaller than 2 KiB. A dedicated writer uses a one-slot queue and a 100 ms acknowledgement deadline per emission. A stalled/closed output fails capture closed; the writer never queues another record after its first delivery failure and is not joined during teardown. The explicit detach attempt after normal collection is not skipped if the stopping record cannot be delivered. Other error returns rely on owned Aya handles for teardown. No terminal record can be guaranteed after output failure, crash or forced termination. Consumers must drain stderr, check process exit and require a terminal record rather than infer success from EOF or an observing record. The duration limit starts after attachment, does not bound synchronous kernel setup, and is not a hard real-time cleanup guarantee.

Generated transition/confirmation tests, a stalled-writer unit test, real CLI replay/invalid-request/object-rejection tests and client validation cover this protocol without privileges. The status/cleanup paths after actual attachment, kernel load failure, runtime read failure and detach failure have not been exercised against a live BPF sensor. This is a protocol foundation, not external sensor supervision, IPC security or live compatibility proof.

## Offline Status Validation

The platform-neutral `StatusStream` receiver consumes arbitrary byte fragments without retaining a full transcript. It enforces records smaller than 2 KiB, a 16 KiB total input budget and at most seven records. Unknown/duplicate/missing fields, invalid JSON/UTF-8, wrong schema/source, replayed or skipped sequence numbers, illegal phases/reasons, forged operation flags and decreasing health counters are rejected. Nullable `reason` and `health` fields must still be present. Health is required for a normal stop and for the kernel adapter's observing/stopping records; fault health remains optional. Any rejection is permanent for that receiver, even after earlier valid records. EOF requires a newline-terminated terminal record, no pending partial bytes, and agreement with the caller-supplied process exit classification. Trailing records and empty extra lines are invalid.

The optional binary provides a read-only stdin consumer:

```sh
cargo run --quiet --features observe --bin intellicord-observe -- replay-lifecycle | \
	cargo run --quiet --features observe --bin intellicord-observe -- \
	inspect-status --source generated_replay --exit success
```

It prints one bounded summary only after successful validation. Malformed/truncated input or exit mismatch returns nonzero with no stdout. `--source kernel_tracepoint` can inspect a previously supplied capture status stream, but never loads, attaches or launches a collector. `--exit failure` accepts a coherent faulted run: the inspector's zero exit means the protocol is valid, not that collection succeeded. Stdin is blocking and must reach EOF; byte bounds do not impose an elapsed-time deadline. Use this offline tool only with a finite producer or an externally supervised pipe, not as the live supervisor itself.

The outer summary uses `source: status_stream_validation`, `sensor_running: false` and `load_test_performed: false`. Nested confirmation flags reproduce validated claims from the supplied stream, not authenticated kernel facts. Source and exit classification are caller assertions. There is no process/session identity binding, transport authentication, heartbeat, timeout, collector termination, crash supervision or cross-process clock normalization in this receiver.

The client's explicit diagnostic check also executes eight generated stream cases: normal completion, fault completion, truncated final record, sequence replay, source mismatch, confirmation forgery, health regression and process-exit mismatch. **Process protocol validation** shows passed counts and any failed cases without hiding failures or treating them as participant evidence. The preview uses labeled static fixture results, not host tests. Optional missing reports remain compatible with older backends; invalid/active reports and stale sessions are discarded. The build-only BPF CI job now runs receiver unit tests as well as executable contract tests before compiling the sensor. CI execution and real BPF compatibility remain unverified here.

## Supervised Generated Collector

```sh
cargo run --quiet --bin intellicord -- supervise-replay
```

This command launches the current Intellicord executable with the fixed arguments `process-replay --status-json`. That child only emits the six generated lifecycle records on stderr, then exits. It also runs a separate fixed event/lifecycle child as described under [joint collector transport](#joint-collector-transport). Neither child performs process observation or BPF operations. No executable, target, object path or capture argument can be selected through this command or through the renderer. The command prints a bounded report containing `source: generated_replay`, inactive-sensor flags, a `supervision` result and an additive `collection` result. A successful command invocation means a report was produced; inspect each outcome, not the command exit alone, for the generated results.

The reusable `supervise` API accepts a trusted backend-created Tokio command, expected status source, cancellation future and nonzero duration no longer than 65 seconds. It checks already-ready cancellation before spawning, overrides stdin/stdout to null, pipes stderr, enables kill-on-drop and feeds 1024-byte fragments into the bounded status receiver. Child stdout is discarded without buffering: the existing collector stdout summary is not validated by this API. A valid terminal record, stderr EOF and a collected process exit are all required. A coherent `faulted` status and failed exit produce `collector_fault`, never `completed`.

Cancellation, deadline, malformed stream and read/wait failures trigger a direct-child kill request and an explicit wait, with a separate two-second cleanup timeout. `collector_reaped` is true only after successful wait; `cleanup_failed` leaves it false. There is no signal grace period, restart or reconnect. The deadline covers asynchronous collection after spawn, not synchronous executable resolution/spawning or arbitrary runtime scheduling, and cleanup can add two seconds. Closing stderr or writing a terminal status without exiting cannot bypass the deadline. The report retains no stdout, raw stderr, PID, path or diagnostic text.

Safety & Intelligence's existing explicit check now runs this fixed generated child with a two-second collection limit. A single background task keeps voice-state processing responsive; requests remain session-bound and rate-limited, and an in-flight replay prevents another spawn. The client clears stale/invalid responses, accepts generated provenance only, and separates **Collector cleanup** from kernel-operation confirmation. Preview results are static labeled fixtures rather than an actual child test. Supervision results remain outside participant evidence and exports. Default startup still launches no collector; the explicit check launches a sensor-free replay even in builds without Aya.

This is direct-child supervision, not a sandbox, privileged loader, cgroup, process-tree killer, authenticated IPC channel or production sensor service. A child can fork descendants that outlive it; descendants retaining stderr can delay EOF until the deadline. Do not pass an untrusted executable. Dropping/aborting the supervisor future uses kill-on-drop only and cannot return reaping confirmation. The backend's task set aborts on disconnect/shutdown; it does not promise awaited cleanup after task abortion or process crash. No live sensor launch command is exposed here, and actual BPF program removal is not verified by waiting for a collector PID.

Linux subprocess tests use generated Node.js fixtures with bounded readiness notifications to check finite/fragmented/faulted output, ignored stdout, malformed/oversized/truncated streams, exit disagreement, active cancellation, stalled output, early EOF and a terminal-but-running child. Known direct child PIDs are absent after each awaited run. An observe-feature test supervises the actual reader's `/dev/null` object rejection before BPF access. Missing tools, cancellation before spawn and invalid limits have separate checks. Read/wait/cleanup OS failures and abort-time reaping are not fault-injected; real kernel loading/attachment and cleanup remain unverified. The build-only CI configuration includes these tests, but has not been run here.

## Supervised Offline Object Check

```sh
cargo run --quiet --features observe --bin intellicord -- check-object
```

This command launches the current executable in its fixed `object-preflight` role. That role calls the same offline metadata inspector as the optional reader, but is not the capture command and never loads or attaches BPF. Only the backend's `INTELLICORD_OBSERVE_OBJECT` environment setting selects the object; neither command accepts extra arguments, paths, PIDs or activation flags. Without `observe`, the helper reports `feature_disabled` on Linux without reading an object. Other platforms report `unsupported`.

The parent accepts at most 2048 stdout bytes, requires exactly one strict versioned object-check JSON reply, rejects unknown/missing/duplicate fields, active-sensor claims and contradictory inspector/status values, and requires EOF plus successful exit. Stdin and stderr are discarded. The inspection deadline is two seconds after spawn; failures request direct-child termination and wait up to two additional seconds for cleanup. `helper_reaped` is set only after successful wait, and failed/unfinished checks retain no object result, raw output, path or error text. `outcome: completed` means the helper protocol completed, not that the object matched: inspect the nested `object.status`, which can be `rejected`, `unreadable` or `not_configured`. Command exit success means a report was produced, not that inspection succeeded.

The client's explicit diagnostic check runs this helper alongside the generated lifecycle child in its single asynchronous diagnostic task. Object parsing no longer runs synchronously in the voice-event request handler. **Object inspection isolation** exposes helper completion/failure and reaping; the nested validated object result is authoritative rather than an independently supplied object field. Older backends without a preflight field retain their optional object-report behavior. Invalid reports and stale/rate-limited sessions clear the result, and the standalone preview uses labeled static fixtures. Diagnostics remain outside participant evidence and exports; normal startup still performs no object inspection or sensor launch.

Generated tests cover matching/mismatched ELF metadata through the actual helper, default-feature availability, activation-argument rejection, oversized/malformed replies, failed exits, stalled output and a valid reply with EOF from a still-running helper. This is process isolation for trusted local parsing, not a sandbox, memory/CPU limit, process-tree controller, authenticated IPC channel or privileged reader launch. Executable resolution/spawn are synchronous and not covered by the deadline; task abortion retains kill-on-drop without guaranteed awaited reaping. OS read/wait/cleanup failures are represented but not fault-injected. Real BPF-target compilation, verifier acceptance, attachment and kernel cleanup remain unverified.

## Kernel Clock Diagnostics

```sh
cargo run --quiet --features observe --bin intellicord -- probe-clock
```

This fixed-argument command reads Linux `CLOCK_MONOTONIC` once, bracketed by Rust `Instant` readings. No process is selected, no helper or sensor is started, and no BPF operation is performed. The versioned `local_kernel_clock_probe` report contains the elapsed bracket width, a one-second source-time horizon and an explicitly assumed 100 ppm relative rate-error bound, with false running/load-tested flags. It omits absolute clock values, IDs and paths. Default Linux builds report `feature_disabled`; other platforms report `unsupported`. Read/conversion failures report `unavailable` with no bracket. Additional arguments, including activation flags, are rejected.

The shared ABI now declares its timestamp domain as `linux_monotonic`, matching the kernel program's `bpf_ktime_get_ns` helper. Its 24-byte record layout and schema are unchanged. This domain excludes system suspend time and is not wall time, `CLOCK_MONOTONIC_RAW`, or suspend-inclusive `CLOCK_BOOTTIME`. A future reader must not silently substitute one of those clocks.

`observe::clock::anchor(backend_origin)` constructs a `KernelClockAnchor` using a caller-owned `Instant` epoch. Future origins and invalid/overflowing clock reads are rejected. Its `map(source_ns)` reuses the existing `SourceClockMapping` arithmetic: it returns an earliest/latest backend interval rather than an exact timestamp, expands the bracket outward by the assumed rate error, and rejects readings before the anchor, more than one second after it, or outside integer bounds. `from_bracket` supports deterministic offline tests and requires caller-asserted source/receipt readings. Neither method authenticates the source, measures drift, nor verifies host, boot or time-namespace identity. The rate bound is a provisional assumption, not a Linux scheduling, clock-resolution or synchronization guarantee.

The horizon limits elapsed source time only, not wall-clock age or liveness. A frozen/replayed reading can remain in range; callers still need independent receipt deadlines, source/sequence guards and fresh anchors. Only timestamps from the same host/boot and matching time namespace may use an anchor; container offsets, process restarts, suspension and clock resets require separate handling. Delayed pre-anchor ring records are rejected, not assigned fabricated receipt times. The probe creates and discards a private local epoch; it does not persist or publish a usable session mapping.

Safety & Intelligence's existing explicit check shows **Kernel clock diagnostics**, including the measured read bracket and the unmeasured rate assumption. It explicitly leaves cross-process synchronization unverified. Optional absent reports remain compatible with older backends; malformed/active claims and stale-session/rate-limited responses clear the result. Preview values are static labeled fixtures, not host measurements. Results never enter participant evidence, scoring or exports, and no live event mapping is wired into fusion.

Deterministic tests cover interval bounds, outward rounding, before-anchor rejection, horizon expiry, overflow and shared-ABI event timestamp composition. An observe-enabled Linux test checks a real local read against its receipt interval; executable tests check bounded inactive output and forbidden arguments. These are local clock/contract checks, not a cross-process handshake, BPF timestamp comparison, live sensor compatibility test or calibrated correlation system. The full supervision/IPC/cross-clock milestone remains unfinished.

## Bounded Clock Handshake

```sh
cargo run --quiet --features observe --bin intellicord -- probe-clock-handshake
```

This fixed-argument diagnostic starts only a same-executable, sensor-free clock helper on observe-enabled Linux builds. Default Linux builds report `feature_disabled` without spawning; other platforms report `unsupported`. It never loads BPF, selects a target or starts capture. CLI success means a report was produced, not that the exchange passed: inspect `status` and `result.outcome`.

The helper announces `clock_ready` before the parent starts its timing bracket. The parent writes one `clock_request` with a positive nonce, closes request stdin, and brackets reception of `clock_reply` around the helper's `CLOCK_MONOTONIC` read. Each JSON line is limited to 256 bytes with strict required fields, schema, domain and nonce checks. Replies with unknown/duplicate fields, missing newlines, trailing output, nonzero exit or a bracket wider than 25 ms fail closed. Readiness excludes helper startup from the measured exchange, but scheduling and IPC delay remain part of the uncertainty; a busy machine can legitimately report `unavailable`. There is no retry or relaxed threshold.

The whole exchange, stdout EOF and child exit have a two-second deadline. Failure or cancellation kills the direct child and waits up to another two seconds; reaping is confirmed only after a successful wait. An otherwise valid anchor is discarded if its one-second local receipt horizon expires before handoff. Kill-on-drop is a fallback, not an awaited cleanup guarantee after task abortion or process crash; descendants and resource containment remain outside this helper.

`observe::handshake::exchange` returns a private-field `ClockHandshake` only after successful validation and reaping. Its `event_stream(pid)` creates the existing bounded timed receiver using the same parent `Instant` origin. Callers must supply receipts relative to that origin and retain the existing one-second horizon, assumed 100 ppm rate bound and 250 ms freshness policy. The nonce binds one request/reply, not source authentication. Matching host, boot and time namespace, trustworthy helper code and compatible kernel clock interpretation remain caller assumptions; namespace identity, drift and suspend behavior are not measured. An anchor from this separate probe is not permission to normalize an arbitrary collector's timestamps.

The public report exposes only status, bounded outcome/cleanup fields, domain and bracket width, never the nonce or absolute readings. Safety & Intelligence requests it alongside the existing fixed diagnostic helpers, displays failures separately from cleanup, and marks namespace identity unverified and live collector handoff unconnected. Invalid or stale-session/rate-limited responses clear the result; absent optional reports support older backends. Preview values remain static labeled fixtures. No report enters participant evidence, scores or exports.

Unit and generated subprocess tests cover nonce/domain/framing/bounds, receipt mapping, stalls, cancellation, exit/EOF agreement, expired handoff and cleanup. Executable tests cover default-disabled and optional Linux helper paths and reject activation arguments. These checks do not compare a real BPF timestamp, establish authenticated IPC, connect the live collector to this exchange or implement anchor renewal for captures lasting up to 60 seconds. BPF-target compilation, verifier acceptance, live sensor compatibility and the full supervision/IPC/cross-clock milestone remain unverified or unfinished.

## Timed Process Replay

```sh
cargo run --quiet --bin intellicord -- replay-timed-process
cargo test --test observe-timed
```

This fixed-argument command exercises shared-ABI decoding, clock-interval mapping and receipt-freshness checks entirely with generated records and timestamps. Its bounded version 1 JSON report has `source: generated_replay`, the `linux_monotonic` clock domain, inactive-sensor flags, eight aggregate health counters and nine named checks. It opens no process, reads no clock, launches no helper and performs no BPF operation. Extra arguments, including paths, target IDs and activation flags, are rejected. The generated totals are two accepted records and one rejection in each of seven categories; these are not measured kernel loss or live telemetry.

The reusable `TimedProcessMonitor` owns a fixed clock anchor and accepted source/receipt watermarks, retaining no records, process details beyond the selected numeric PID, queue or transcript. Construction validates a nonzero PID, bracket ordering and a representable local deadline. `accept(bytes, receipt_ns)` first uses the shared strict decoder, rejects receipt times before anchoring or before the last accepted receipt, rejects receipts after the anchor's local one-second deadline, rejects source-clock regression, then maps the event timestamp. Source readings outside the mapping horizon or before the source anchor are mapping rejections. Events wholly after receipt are future rejections. For freshness, the earliest possible event time must be no more than 250 ms before receipt; an uncertainty interval that could include an older event is conservatively rejected. Accepted intervals are clipped at receipt, preserving uncertainty without assigning an exact event time. Deadline and age boundaries are inclusive.

Each accepted/rejected input increments one saturating health counter. Rejected input changes neither accepted watermark; rejection is not a permanent fault, and the caller chooses whether to stop collection. Equal source/receipt timestamps remain allowed, matching the existing decoder's ordering semantics; this is not event deduplication, a sequence guard or authentication. There is no auto-renewal or re-anchoring: create a fresh monitor when a trusted anchor changes. Ring reservation drops are separate and are not represented by these input-rejection counters.

The caller supplies trustworthy receipt times and matching source/backend clock epochs. The one-second receipt deadline prevents a frozen source timestamp from extending an anchor when receipts advance, but cannot detect a caller that replays or freezes both clocks, nor authenticate time namespaces, process generations, host/boot identity or data origin. The 250 ms age policy and 100 ppm mapping assumption are provisional testable limits, not calibrated production thresholds. The live reader still uses its existing health-only path; this adapter is not yet wired to actual ring collection, sensor lifecycle, cross-process IPC or participant fusion.

Safety & Intelligence's explicit diagnostics run the nine-case generated replay and show **Timed process replay**, all synthetic counters and any failed check. Reports must have generated provenance, inactive flags, the expected ordered check list and safe nonnegative counters. Unsupported/malformed reports and stale/rate-limited sessions clear the result; old backends can omit it. Preview results are static labeled fixtures. These diagnostics never enter participant evidence, exports, scoring or moderation. Tests cover invalid/foreign input, future/stale events, mapping/receipt/anchor limits, interval clipping, state isolation, counter saturation, repeated timestamps and executable argument/privacy boundaries. Build-only CI includes the timed integration target; real BPF compilation, verifier acceptance, live mapping and cleanup remain unverified.

## Offline Event Transport

`EventStream` incrementally feeds the timed reader from native-endian, fixed 24-byte ABI records. It retains one partial record, accepts at most 256 records (6,144 bytes), and permanently rejects the stream after any byte-budget, receipt-order, ABI or timing failure. A valid prefix followed by invalid input cannot produce a successful summary. Completion requires at least one complete record, no partial tail and a successful producer-exit classification. Each record uses the receipt timestamp of the chunk that completes it; even partial-chunk receipts must not regress.

The optional reader can inspect finite binary stdin without loading BPF:

```sh
cargo run --quiet --features observe --bin intellicord-observe -- inspect-events \
	--pid 42 --source-anchor 1000 --before 100 --after 120 --receipt 1300 \
	--exit success < generated-events.bin
cargo test --features observe --test observe-cli
```

This example expects caller-provided native ABI bytes for PID 42 with source timestamps compatible with the supplied anchor. All numeric clock inputs are nanoseconds. The inspector applies the same asserted receipt time to every stdin chunk; it does not measure delivery latency, validate the producer's actual exit, select a process or authenticate input. It emits only aggregate counters and inactive flags after full acceptance. Unknown or extra arguments fail. Blocking stdin must reach EOF; the byte limit is not a wall-clock deadline, and a stalled producer needs external supervision.

The generated timed replay additionally exposes eight ordered `transport` checks: byte-by-byte delivery, truncation, foreign PID, byte limit, partial-chunk receipt regression, failed exit, empty input and delayed-fragment freshness. The client shows totals and failed checks separately from the nine timed checks; older reports may omit this additive field. Preview data remains a static fixture. These checks do not enter participant evidence or exports.

EOF plus successful exit does not prove that whole records were not omitted. There is no sequence/terminal marker, source authentication, endian negotiation, live receipt clock, ring-drop accounting or lifecycle agreement in this byte adapter alone. The joint supervisor below adds receipt timing and terminal count agreement. The optional live reader can now export the same ABI bytes, but actual BPF compilation, verifier acceptance and live transport remain unverified.

## Joint Collector Transport

`Collection` composes the existing bounded event and status receivers with permanent rejection across both streams. Successful completion requires a complete stopped lifecycle, terminal health, whole event input, successful exit and exact agreement between terminal collector `accepted` and receiver accepted counts. A zero-byte event stream is permitted only through this joint completion path with zero terminal accepted records; standalone `EventStream::finish` and `inspect-events` still reject empty input. Terminal collector rejection counters and ring reservation drops remain separate from receiver timing/ABI rejection; matching received records does not imply zero kernel loss. A faulted collector produces no successful collection summary. Whole-record omissions are detected when the producer's accepted count still includes those records.

`supervise_collection` owns a trusted backend-created command and drains binary stdout and NDJSON stderr concurrently in separate 1024-byte buffers. The caller supplies the `Collection`, its trusted source-clock mapping and matching backend `Instant` origin. Each completed read receives checked elapsed nanoseconds from that origin; a future origin fails before spawn. The collection's status source is fixed at construction, not inferred from output. This does not establish a cross-process clock handshake or authenticate a source.

Both pipe EOFs and a successful child wait are required before final agreement validation. The existing supervisor's duration limit, cancellation, bounded direct-child kill/wait cleanup and kill-on-drop fallback apply. Invalid input in either pipe fails the whole run; no partial success summary survives. A child holding either pipe open, or closing both without exiting, reaches the deadline. Quiet completion does not prove that the tracepoint ran, the selected process executed, or the source was loss-free. With no events, no event timestamp is available to exercise freshness or mapping expiry; the supervisor's wall deadline still applies. No raw bytes, PID, path or individual timestamps are retained in the report. Status-only supervision still discards stdout and retains its prior fault-result behavior.

`intellicord supervise-replay` runs the status-only and joint generated children concurrently. The new child role `process-collection-replay` accepts no arguments and emits two native records plus six lifecycle records with matching health. Its generated source clock and local anchor both start at zero immediately before spawning; actual backend elapsed read receipts enforce the provisional 250 ms freshness limit. Slow scheduling can therefore yield `invalid_stream` instead of a passing diagnostic. There is no threshold relaxation, automatic retry or claim that this synthetic source is a real kernel clock. The two-second deadline bounds stalled collection independently of event freshness.

The client displays **Joint transport result**, **Joint collector cleanup**, **Matched event records** and **Joint synthetic ring drops**, requiring generated provenance, inactive flags, safe bounded counts and successful count agreement. Failures stay visible without successful event totals. Older reports can omit the additive field. Preview results are static fixtures; session/rate/invalid-response handling clears stale data, and these results never enter participant evidence, exports or moderation.

The additive `quiet_collection_check` boolean runs a generated in-process zero-event lifecycle through the same collection receiver, preserving synthetic rejection/drop counters. Safety & Intelligence shows **Quiet collection replay** as passed or failed, rather than treating a failed check as malformed input; non-boolean values invalidate the supervision report. Older backends may omit the field. This adds no process launch, live observation or sensor health claim. Subprocess fixtures separately cover quiet pre-anchored and negotiated streams, zero-count disagreement, partial records, failed exit and direct-child reaping. Zero matched records can coexist with reported drops and rejected producer input; they must not be presented as an absence of host activity.

Tests cover byte-fragment delivery, missing whole records, partial tails, foreign records, budget overflow, exit mismatch, one/both EOFs without exit, cancellation, pre-spawn guards and confirmed direct-child reaping. CI's existing observation unit/integration targets include them. Count agreement is not a sequence, integrity or authenticity guarantee: matching-count substitutions/duplicates, dishonest counters and source declarations remain possible. Pipe arrival order does not establish event/lifecycle temporal ordering. Process trees, privileged service separation, authenticated IPC, calibrated clocks, crash/abort cleanup guarantees and actual BPF loading remain unfinished. The raw live export mode below is not launched by these diagnostics.

## Negotiated Collector Transport

`intellicord supervise-replay` now also runs a fixed `process-negotiated-replay` child. That child announces readiness, accepts one bounded clock request, replies with a declared synthetic source anchor, then writes two native ABI events through the production exporter and six lifecycle records with matching health. Its command accepts no target, object or activation arguments. This works in default builds without real clock reads, Aya or privileges. The aggregate `negotiated_collection` field is additive; it retains no nonce, absolute readings or individual event data.

`supervise_negotiated_collection(command, source, pid, nonce, limit, cancel)` owns one child for the entire exchange and collection. It creates a private parent `Instant` origin before spawn, leaves stdin piped only for the request, and reuses the strict 256-byte ready/reply parser and 25 ms bracket policy. After the validated reply, it constructs `Collection` from that anchor without spawning a replacement child or consuming trailing event bytes. Reply and events may arrive in the same OS pipe write. The selected lifecycle source is a trusted caller argument, not inferred from child data. A nonce binds this exchange only; it is not authentication.

The requested 1 ns..65 second overall deadline starts after spawn and includes negotiation; negotiation also has a two-second cap. Cancellation is checked before spawn and during both stages. Invalid PID/nonce, malformed exchange, excessive uncertainty, stream rejection, deadline or cancellation yields no successful summary. Both EOFs, successful child wait and terminal count agreement still gate completion. Negotiation failure uses the same direct-child kill and separate two-second wait budget; kill-on-drop remains only a fallback after task abortion. Stderr is not parsed until negotiation completes: a child must deliver its stdout preamble before depending on status-pipe drainage; a stalled or backpressured preamble fails at the deadline rather than collecting indefinitely.

After negotiation the existing receiver still permits at most 256 records, a one-second receipt/source horizon and 250 ms conservative event age, with assumed 100 ppm mapping error. Negotiation does not extend those policies. Pre-anchor events are rejected, not silently discarded or relabeled. Scheduling delays can legitimately fail a generated replay; there is no retry or threshold relaxation. Count agreement is not an authenticity, sequence or deduplication guarantee. Matching host/boot/time namespace and trustworthy source declarations remain assumptions for future real kernel use.

Safety & Intelligence shows **Negotiated transport source**, **Negotiated transport result**, **Negotiated collector cleanup** and **Negotiated matched records**, explicitly naming synthetic clocks and events. Malformed, active, inconsistent-count and stale-session/rate-limited results are cleared; failure outcomes remain visible without successful counts. Older backends may omit the field. The preview uses a static labeled fixture, and these diagnostics never enter participant evidence, exports or moderation.

Generated subprocess tests cover delayed readiness, a coalesced reply/event write, wrong nonce, truncated data, failed exit, EOF without exit, stalls, cancellation and known-child reaping. Unit and executable tests cover pre-spawn guards, bounded aggregate output and forbidden arguments. CI already includes these test targets. The opt-in reader preamble described below now implements the producer side, but neither the renderer nor this replay launches live capture. Real clock/namespace validation, anchor renewal for long captures, privileged separation, resource containment, BPF-target compilation, verifier acceptance and live cleanup remain unfinished or unverified. Kernel event/map layout is unchanged.

## Explicit Event Export

The optional reader accepts trailing `--events --status-json` after all the normal `capture` object, PID, duration and acknowledgement arguments. Both flags and this order are required; missing consent, repeated flags or a malformed request fail before loading. Default capture retains its health-only behavior. Event mode writes only native 24-byte binary records to stdout, never the final JSON summary; stderr retains the lifecycle protocol. Binary output includes local numeric process/thread IDs and monotonic times, not remote participant evidence. Do not mix it with text, forward it remotely or persist it without deliberate consent.

`EventExporter` shares the strict `ProcessMonitor` ABI/PID/source-order validation and emits at most 256 records (6,144 bytes). A dedicated writer accepts one fixed-size record at a time; successful write plus flush must be acknowledged within 100 ms before another record is accepted. Acknowledgement is local I/O completion, not downstream validation. Invalid/foreign/backwards input, exceeding the record budget, write/flush failure or a missing acknowledgement permanently rejects that exporter. Record/budget errors fault capture with `input_rejected`; output errors use `delivery_failed`. The budget is a failure boundary, not a successful truncation policy or event sampling.

On error, no successful terminal lifecycle or JSON summary is produced. Existing owned unpinned BPF handles remain the error cleanup boundary; fault status does not confirm detachment. A prefix, partial record or in-flight record may already have reached the pipe, including after an acknowledgement timeout; only a complete validated lifecycle/exit can authorize a collection result. The writer is not joined during teardown, and a blocked write may remain until process exit. Memory bounds cover fixed queued/in-flight records and monitor state, not OS pipe buffers. The monitor counts validated records before the write, so fault-time accepted counts are not delivery confirmation. Normal terminal counts reflect acknowledged records; ring reservation drops remain independent.

Per-record acknowledgements can delay a batch; cancellation and duration checks occur between existing bounded collection batches, not inside synchronous writes or kernel setup. There is no hard real-time stop guarantee. The one-second timing anchor used by `EventStream` also cannot cover arbitrary 60-second captures: live supervisor integration still requires a trusted clock handshake, compatible collection duration or explicit re-anchoring policy. Neither the renderer nor `supervise-replay` launches live capture.

The fixed generated child now uses this exact exporter before emitting matching terminal health, so the existing client joint-transport diagnostic exercises the production producer/receiver path without Aya. Unit tests cover exact bytes, strict budgets, foreign/malformed input, stalled acknowledgements and write/flush errors. Executable tests verify flag/consent gates and invalid-object rejection before BPF access. These generated checks do not prove verifier acceptance, actual ring export, post-attach fault cleanup or live compatibility; those remain unverified, and kernel event/map layout is unchanged.

## Negotiated Reader Export

The optional `intellicord-observe` reader accepts the explicit form `capture --object PATH --pid PID --milliseconds 1..500 --acknowledge-process-observation --clock-handshake --events --status-json`. Flag order is strict. The handshake requires binary events and JSON lifecycle output; seconds-based duration, missing consent, repeated or reordered flags, and millisecond durations outside 1..500 fail before loading. Ordinary health capture and unnegotiated 1..60 second event export retain their existing formats. Do not start this command without deliberate target/object review and permission to load BPF.

The reader validates the object, opens/rechecks its pidfd and loads/configures its owned unpinned BPF handles before clock readiness. It negotiates immediately before attachment using its actual `CLOCK_MONOTONIC` reading. Successful completion is followed by signal and target-lifetime rechecks before `attach`. Readiness is not proof of attachment; existing `observing` lifecycle status remains the attachment confirmation. Invalid objects produce no stdout preamble. A supervisor must accommodate setup inside its overall deadline and apply strict source/domain/nonce/bracket checks; there is no renderer activation route or automatic privileged launcher.

The responder runs one stdio worker with a two-second acknowledgement deadline, independent of the parent's deadline. Pending cancellation is checked before starting the worker and SIGINT/SIGTERM are observed while awaiting it. No attachment follows responder failure or cancellation. Cancellation maps to `cancelled`, worker creation failure to `delivery_failed`, and rejected/timed-out requests to `input_rejected`; those codes do not distinguish every clock I/O failure. Owned BPF handles are dropped on error, but real cleanup remains unverified.

The worker's bounded JSON parser and fixed reply do not bound OS I/O latency. Timeout/cancellation does not interrupt or join a blocked stdio thread; the CLI's process exit is the final worker boundary, as with the event writer. A prefix or delayed clock reply can escape after failure, so consumers must require full stream/exit agreement. This one-shot responder is not a reusable daemon worker pool or a thread-reaping guarantee. Synchronous BPF setup still has no in-process hard deadline.

After the reply, stdout contains native binary events only and no final JSON report; stderr carries the existing kernel-source lifecycle. The short millisecond duration starts after attachment. The 500 ms maximum leaves nominal room in the receiver's one-second anchor horizon, but scheduling, attachment delay and synchronous output can still cause freshness or horizon rejection. No guarantee of acceptance or hard real-time stop is implied; there is no anchor renewal. The existing 256-record cap, PID/ABI guards, 250 ms freshness policy and terminal count agreement remain unchanged. Quiet collections require the full joint lifecycle/exit/zero-count agreement described above, not merely a normal reader exit; no event timing compatibility is proven when no events arrive.

`intellicord-observe replay-negotiated` exercises the same bounded responder/exporter with a fixed synthetic clock and events, without loading BPF or selecting a process. It expects one clock request on stdin, not an interactive invocation. Tests run this actual binary under the owning supervisor, check two matched records and reaping, reject activation arguments, and verify an open stalled request independently times out. The client's existing generated replay now uses this same responder and exposes optional **Clock request deadline** metadata (2000 ms); older reports may omit it. Preview values remain fixtures. None of these diagnostics enters participant evidence or exports.

These are generated/default/optional-feature transport and consent-gate checks, not a real BPF timestamp comparison, live attachment test or namespace/authentication proof. BPF-target compilation, verifier acceptance, live export/cleanup and least-privilege separation remain unverified or unfinished. Kernel program/map/event ABI is unchanged.

## Build Boundaries

The root `observe` feature adds Aya 0.13 and the `intellicord-observe` binary. Default builds do not compile Aya or the binary. The small `intellicord-observe-common` no-std crate is shared with the independently built kernel crate.

The kernel crate uses Aya eBPF 0.1, with its own lockfile and target directory. Host-only source checks are:

```sh
cargo check --release --locked --manifest-path crates/intellicord-ebpf/Cargo.toml
cargo clippy --release --locked --manifest-path crates/intellicord-ebpf/Cargo.toml -- -D warnings
```

Actual BPF compilation requires a nightly Rust toolchain, its `rust-src` component, and `bpf-linker` on PATH. The build workflow pins `nightly-2026-10-15` and uses the little-endian `bpfel-unknown-none` target. After deliberately installing rustup and a suitable linker, install the selected compiler sources explicitly:

```sh
rustup toolchain install nightly-2026-10-15 --profile minimal --component rust-src
```

The earlier BPF-target check failed with missing `core`, then the build preflight was blocked by missing tooling. With the pinned nightly, `rust-src` and linker now installed, the real build-and-inspect command succeeds locally. This establishes target code generation and offline layout/instruction inspection, not verifier or kernel compatibility. The local script installs no compiler, linker, kernel capability or privileged service.

## Build And Inspect

From a reviewed checkout, with the prerequisites above and the ordinary host Rust toolchain available:

```sh
npm ci
npm run build
bash scripts/build-ebpf.sh
```

The script accepts no arguments. It checks Cargo, rustup, `bpf-linker`, the pinned nightly and the exact installed `rust-src` component before compiling. It uses the kernel crate's lockfile, release profile, explicit linker, `-Z build-std=core`, and an explicit repository-local target directory rather than falling back to a host build. Compiler diagnostics go to stderr. On successful compilation, it invokes the ordinary host reader's `inspect --object` on exactly:

```text
crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process
```

Only a successful inspection emits the bounded metadata JSON on stdout; any missing prerequisite, compilation failure or inspection rejection exits nonzero. A previous object is never inspected as a fallback after a failed build. This command does not load BPF, attach a sensor, inspect another process or change Discord. It is intended for little-endian Linux hosts; the inspector intentionally rejects a cross-endian object. Source/compiler/linker behavior is trusted, not sandboxed. Arbitrary local build flags and tool substitutions are outside reproducibility guarantees.

The GitHub Actions workflow in `.github/workflows/ebpf-build.yml` configures this same command on Ubuntu 24.04 with host Rust 1.99.1, the dated nightly and a SHA-256-checked prebuilt `bpf-linker` 0.13.0 archive. It builds the frontend first, runs orchestration and sensor-contract tests, then compiles and inspects the actual BPF artifact. The job has read-only repository permissions, no persisted checkout credentials, no privileged steps and a 20-minute timeout. Successful jobs retain `process-sensor-build-only` artifacts for seven days, containing the object and `bpf-object-metadata.json`; they are not a signed release, authenticity guarantee or permission to load arbitrary downloaded code.

The workflow has been parsed and its command/pin/permission contract tested locally, but has not been dispatched or observed succeeding during this implementation. Do not count configuration or generated tests as a passing BPF build. Verifier acceptance, privileged loading, supported-kernel compatibility and live cleanup remain separate, unperformed checks even after a future successful CI build.

Generated build-tool tests run without any BPF toolchain:

```sh
node --max-old-space-size=256 --test tests/ebpf-build.test.js
```

These tests verify exact build/inspection arguments, working directory, output routing, missing tools/components, compiler/inspector failure propagation, argument rejection and parsed CI configuration. Fake tools are confined to temporary directories and emit an explicitly generated report; they do not compile kernel code.

A locally built and reviewed object at the path above can be supplied through the backend's existing `INTELLICORD_OBSERVE_OBJECT` setting. An explicitly requested client prerequisite check then validates that actual file rather than the standalone preview fixture. No new renderer activation or file-selection surface is added; inspection still does not start tracing. Rebuild the backend with `observe` before using this setting, and do not run the injector as root.

## Explicit Load-Only Verification

The optional Linux reader now has a separately acknowledged command for the next boundary after target compilation and offline inspection:

```sh
cargo run --quiet --features observe --bin intellicord-observe -- verify-load \
	--object crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process \
	--acknowledge-bpf-load
```

**This is not an offline check.** Only run it deliberately with a trusted, reviewed object and appropriate host permissions. It can create temporary kernel maps and load the tracepoint program for verifier processing. It never selects a PID, writes a target, attaches a tracepoint, records events, pins an object, changes resource limits or changes kernel security settings. Extra target/attach/capture flags and missing acknowledgement are rejected before object access. Never run Cargo or the Discord injector as root; if elevated permissions are required, build as an ordinary user and arrange approved permissions for only the reviewed reader binary.

The command reads the bounded regular file once and applies the same ELF/program/map/instruction inspection as capture before calling Aya. It then creates the object/maps and calls `TracePoint::load`, not `attach`. The owned Aya object is dropped before a result is emitted. Normal `capture`, default injector startup, client diagnostics and CI do not invoke this operation.

One bounded JSON report (less than 512 bytes, schema 1, source `explicit_kernel_load_probe`) is emitted on stdout for a validly acknowledged request. Exit zero means `verified`; every failure outcome exits nonzero. Invalid arguments emit no JSON. Fields distinguish map-setup attempt, program-load attempt, successful load, attachment attempt, sensor state and owned-handle disposal. They are stage facts, not permission diagnoses or proof that a particular syscall reached the verifier:

| Outcome | Map Setup Attempted | Program Load Attempted | Program Loaded |
| --- | --- | --- | --- |
| `object_unreadable` | No | No | No |
| `object_rejected` | No | No | No |
| `map_setup_failed` | Yes | No | No |
| `program_unavailable` | Yes | No | No |
| `program_load_failed` | Yes | Yes | No |
| `verified` | Yes | Yes | Yes |

Map setup can fail from loader, BTF, resource, kernel or policy problems; program loading can fail before or during verifier processing. The report intentionally omits raw verifier logs, object paths, identities and exception strings. It does not distinguish every kernel/policy failure. `owned_handles_dropped` describes userspace disposal, not observed kernel removal, global absence of references or tracing-detachment proof. Setup/loading is synchronous with no controller deadline or cancellation guarantee; interrupting or killing the process relies on OS descriptor cleanup, not this report. A successful load applies only to that object and execution context at that moment and is not retained as a trusted client verdict.

The client's explicit offline diagnostics show **Load-only probe build** and **Load-only probe activation**. Availability is optional build metadata, checked against inspector availability, not a loaded-program claim. It is included for observe-enabled Linux builds, absent/unknown for older helpers, and unavailable in default builds. Session/rate-limit guards still apply; malformed or contradictory fields clear stale results. **Object load and attach** remains **Not tested**. There is no renderer load command, imported load report, automatic retry or privileged helper activation. The preview remains a fixture; no participant evidence or evidence export is added.

Sensor-free tests cover consent, unsupported arguments, bounded outcomes and invalid objects stopping before any kernel loader callback. An explicitly selected real-artifact test uses the real preflight with injected loader outcomes, not BPF syscalls:

```sh
cargo test --features observe --test observe-cli load_only_probe
cargo test --features observe --lib observe::verification::tests
INTELLICORD_TEST_BPF_OBJECT="$PWD/crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process" \
	cargo test --features observe --lib observe::verification::tests::real_bpf_artifact_exercises_load_outcomes_without_syscalls -- --ignored --exact
```

CI now runs that injected-loader test after the real target build, without invoking `verify-load`. No acknowledged load of a valid object was performed during this implementation. Actual verifier acceptance, permission-failure behavior, attachment, live event delivery and cleanup still require separately authorized kernel testing. Full telemetry and privilege separation remain unfinished.

## Compiled Code Inventory

Successful offline inspection now includes a strict versioned `code` inventory derived from Aya's parsed functions and instructions, not disassembler text. It counts declared functions and eight-byte instruction slots (including continuation slots), and classifies direct helper calls to map lookup (1), monotonic clock (5), current identity (14), ring reservation (131) and ring submission (132). Other helpers, BPF local calls, and other call forms (`CALLX` or non-helper/non-local call encodings, including kfunc forms) have separate counts. Inventory is not reachability analysis, a helper allowlist, ABI proof or a verifier. Unused/unreachable code can contribute counts; helper IDs and instructions alone do not prove arguments or control flow are safe.

The actual local build currently has 100 instruction slots in four parsed functions, two map-lookup call sites, one each clock/identity/reserve/submit call site, and no other helper, local or indirect call forms. These are compiler-dependent observations, not pinned instruction-count guarantees. The normally ignored regression test requires an explicit artifact path and checks a broad 16..4096 slot budget, required helper presence and absence of unexpected helper/indirect calls:

```sh
bash scripts/build-ebpf.sh
INTELLICORD_TEST_BPF_OBJECT="$PWD/crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process" \
	cargo test --features observe --test observe-object real_bpf_artifact_has_expected_instruction_inventory -- --ignored --exact
```

The test fails if the supplied object is missing or rejected; it does not silently substitute generated input. CI now runs it after the real target build, before artifact upload. The updated CI job has not been dispatched here. Generated ELF fixtures independently encode helper/local/other calls, and an initial exit makes explicit that counting is not execution proof. Existing size/section/symbol limits apply; inventories require 1..4096 functions, at most 524288 slots and aggregate call counts no larger than slots. These are logical parsing/report bounds, not measured memory/CPU budgets.

The existing isolated object helper carries an optional inventory only for `layout_matched`; missing inventories preserve old-backend compatibility. Strict schema, field, integer, bound and count-consistency checks reject malformed reports. The client displays **Code inventory source**, **Declared code functions**, **BPF instruction slots** and call-site counts beneath **Process sensor object**, while load/attach stays **Not tested**. Session/request/rate-limit guards remain unchanged, and the standalone preview is explicitly a fixture. No bytecode, paths, function names or raw helper IDs enter the client report, participant evidence or evidence exports.

Exercise the real helper path without attaching Discord or loading a sensor:

```sh
INTELLICORD_OBSERVE_OBJECT="$PWD/crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process" \
	cargo run --quiet --features observe --bin intellicord -- check-object
```

For a separately authorized injector session, use the same backend environment setting and the **Check Linux sensor prerequisites** control. The renderer cannot select objects or start tracing. Default builds still report the inspector unavailable. BPF verifier acceptance, helper runtime values, actual ring delivery, pidfd/filter races, privilege separation and live cleanup remain unverified or unfinished; no real sensor was loaded during this work.

## Offline Object Inspection

After building a trusted object, inspect its declared layout without loading it:

```sh
cargo run --quiet --features observe --bin intellicord-observe -- inspect --object crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process
```

The inspector uses structured ELF and Aya metadata parsers only. It accepts a regular file of 1 byte through 4 MiB, a native-endian 64-bit relocatable BPF ELF, at most 64 sections and 4096 symbols, and in-range symbol extents. It requires exactly one `process_exec` tracepoint program and these unpinned, zero-flag maps:

| Name | Map Type | Key Bytes | Value Bytes | Maximum Entries |
| --- | --- | --- | --- | --- |
| `TARGET` | Array | 4 | 4 | 1 |
| `EVENTS` | Ring buffer | 0 | 0 | 65536 bytes |
| `DROPPED` | Per-CPU array | 4 | 8 | 1 per CPU |

Successful output has `source: object_metadata`, the matched layout, and false load-test/running flags. `expected_event_bytes` describes the reader's 24-byte expectation, not an instruction-level proof of emitted record layout. Wrong architecture, program declarations, map resources, pinning and malformed inputs are rejected without a success report. Generated ELF fixtures cover matching metadata and rejection through the real CLI; those fixtures are not a compiled sensor or evidence of live compatibility.

The same check runs on the exact bytes passed to Aya before live loading. This is a declared-layout guard, not a sandbox, trust/authenticity check, instruction verifier or permission test. Continue to accept only reviewed, locally built objects; no untrusted-object parsing service is provided.

For client checks, the backend may optionally be launched with `INTELLICORD_OBSERVE_OBJECT` set to the trusted object's path and the `observe` feature compiled in. The existing explicit prerequisite request then adds **Process sensor object** status. No configured path means no file access; default builds report that the optional inspector is not built. Read failures, rejected metadata and a layout match are distinct. Paths and parser diagnostics are not sent to the client or evidence export. Renderer requests cannot supply a path. Every check remains offline, rate-limited and session-bound; it never launches the reader, loads BPF or attaches. The preview's layout-match result is explicitly a fixture, not a host result. No previously inspected bytes are authorized for later use; capture always reads and checks its own bytes again.

## Process Lifetime Guard

Test this backend's pidfd access without selecting another process or loading BPF:

```sh
cargo run --quiet --features observe --bin intellicord-observe -- probe-target
```

The command opens a pidfd for itself, polls it without waiting, and closes it. A bounded schema-v1 report has `source: self_pidfd_probe`, a status of `available`, `unavailable` or `unsupported`, and false `target_selected`, `load_test_performed` and `sensor_running` flags. No PID, executable path, process metadata or error details are serialized. Extra arguments, including `--pid`, are rejected. `available` means only that this self-process open/poll succeeded; it does not establish permission to inspect another process, BPF permission, executable identity or kernel-filter identity.

The existing explicit client prerequisite request includes the same self-only probe when built with `observe` on Linux. Default builds report `feature_disabled` without pidfd access. **Process target safety** keeps the self-probe result separate from the object layout and generated replay, always showing that no observation target is selected and identity binding is not verified. The renderer cannot select a process. Malformed, active-sensor or selected-target claims are omitted; session changes and failed responses clear prior results. Preview results remain labeled fixtures, not a host probe. Probe results do not enter participant evidence or exports.

Capture now requires a pidfd for the explicitly selected numeric PID after object metadata validation but before BPF loading. It refuses to load if the handle cannot be opened, polling fails or exit is already visible. It checks again immediately before attachment. Collection checks liveness on each 20 ms tick before processing records. Visible exit causes detachment and a normal health-only report with `stop_reason: target_exited`; no final tail drain is performed. Other stop reasons drain only while liveness checks continue to pass. Unread records can be discarded, so counters remain partial: `ring_dropped` counts failed reservations, not tail records discarded on target exit.

The pidfd refers to the process selected when it was opened and does not become a handle to a later process reusing the numeric PID. It is close-on-exec and owned by the reader; closing it does not terminate the target. Tests create only disposable `cat` children with piped input, then exercise normal exit, kill, reaping and unrelated child activity. They do not force actual PID reuse, load BPF, attach to Discord or prove live reader teardown.

This is a userspace lifetime gate, not durable identity binding in the BPF program. Selection can already be stale before `pidfd_open`, exit can occur between a check and attachment/event processing, and scheduling or synchronous setup can delay detection beyond one tick. Events accepted before visible exit are not retracted. The kernel filter still compares numeric TGIDs; executable verification, matching PID namespaces and generation-bound in-kernel identity remain unverified. No fallback to unguarded capture is attempted if pidfd is unavailable. Use only a controlled disposable process for future live testing.

## Explicit Live Invocation

Only after building and reviewing the kernel object, a deliberate invocation has this form:

```sh
cargo run --features observe --bin intellicord-observe -- capture --object crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process --pid 12345 --seconds 10 --acknowledge-process-observation
```

Replace the numeric PID deliberately. The command accepts only an explicit object, positive PID, duration of 1..60 seconds and acknowledgement. It rejects nonregular, empty, oversized and metadata-mismatched objects before Aya loading, using the offline contract above. Only use the reviewed object built from this repository: metadata matching does not establish trustworthy behavior or sandbox a malicious ELF. Do not expose this command as an arbitrary privileged-object loader.

The reader does not elevate privileges, change sysctls/rlimits, grant capabilities or fall back to another tracing source. Linux permissions, kernel support and tracing policy must already permit operation; failure exits without a success report. This prototype is not a privilege-separated production service. Do not run the Discord injector as root to make the sensor work. An audited loader and IPC design remain future work.

## Collection And Bounds

- The only hook is `sched:sched_process_exec`. A map filters the current TGID to the explicitly selected numeric PID before reservation. No process tree, filenames, command lines, environment, sockets, payloads, Discord IDs or remote participants are collected.
- The native-endian, same-host record is exactly 24 bytes: schema, event kind, process ID, thread ID and `bpf_ktime_get_ns` timestamp. The decoder rejects wrong size/schema/kind, foreign PIDs and zero thread IDs without assuming buffer alignment.
- The supplied kernel program declares a 64 KiB ring, one target entry and one per-CPU drop-counter entry. Failed reservations increment a saturating per-CPU count. The reader sums those counters with saturation. These are logical bounds for the reviewed object, not measured total RAM or CPU budgets.
- The reader processes at most 64 records per 20 ms tick, retains only counters and the latest accepted source timestamp, and faults on malformed/backwards input. Invalid records do not advance the accepted clock. Equal timestamps are permitted; this is not an event-ID deduplicator.
- Duration, SIGINT, SIGTERM or visible target exit end the collection loop. Cancellation pending during setup and target liveness are checked before attachment. Detachment precedes bounded, liveness-gated final draining and success output; visible target exit skips or stops draining. Owned Aya handles provide teardown on ordinary errors; nothing is pinned for persistence. Live signal/error/crash cleanup remains unverified.
- Status is written before attachment and after detachment, not while collecting, so a blocked status consumer cannot hold the active loop in a synchronous print. Kernel loading is synchronous: duration bounds cover the collection loop, not a hard deadline on kernel setup syscalls.
- Successful output is health-only JSON, not an event dump. PIDs and source timestamps are not serialized into the report. Raw records exist temporarily in the kernel ring/userspace borrowed view and are not written to disk by this tool. Ordinary destruction is not secure erasure; OS swap/core dumps are outside this guarantee.

## Important Limitations

A numeric PID is not a durable process identity: the pidfd gate above detects the opened process's exit but does not bind the kernel filter to that lifetime or verify an executable. Reuse before selection or between checks can still affect filtering, and this prototype does not follow children. Select a controlled disposable process for future live testing, not an authenticated Discord process. No live BPF capture was performed during this implementation.

The source clock is Linux monotonic time since boot, not the injector's `Instant` origin. No cross-process normalization, calibrated uncertainty, suspend handling or fusion routing is implemented. Counts refer to accepted decoder records and failed ring reservations, not complete system activity or total pipeline loss. BTF/cgroup availability alone does not establish load/attach permission or verifier compatibility.

Live compatibility, permission-denied and signal fault injection with a real object, PID identity binding, least-privilege separation, IPC supervision, CPU/memory profiling, client live indicators and clock-normalized health transport remain unfinished. No process-specific findings, automatic mitigation, remote identity attribution or participant moderation is implemented.