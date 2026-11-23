import test from "node:test";
import assert from "node:assert/strict";
import { createProgressHandoff, ownerFeature, progressFeature, readOwnerOutcome } from "../src/web/process-progress.js";

test("session health distinguishes optional configuration, pending work, faults and cleanup", () => {
  const owner = { schema_version: 1, source: "capture_owner_outcome", status: "not_configured", result: null };
  assert.equal(ownerFeature(owner).state, "ready");
  assert.match(ownerFeature(owner).detail, /no process collector requested/);
  assert.equal(ownerFeature({ ...owner, status: "awaiting_completion" }).state, "waiting");
  for (const invalid of [null, undefined, { ...owner, status: "unavailable" }, { ...owner, sensor_running: true }]) {
    assert.equal(ownerFeature(invalid).state, "error");
  }
  for (const outcome of ["completed", "cancelled", "collector_fault", "deadline", "cleanup_failed"]) {
    const result = { outcome, collector_started: true, collector_reaped: true, endpoint_removed: true };
    const finished = { ...owner, status: "finished", result };
    assert.equal(ownerFeature(finished).state, ["completed", "cancelled"].includes(outcome) ? "ready" : "error");
    assert.equal(ownerFeature({ ...finished, result: { ...result, endpoint_removed: false } }).state, "error");
  }
});

test("automatic telemetry describes freshness without claiming liveness", () => {
  assert.equal(progressFeature(null).state, "unsupported");
  assert.equal(progressFeature({ status: "not_connected" }).state, "ready");
  assert.equal(progressFeature({ status: "no_recent_sample" }).state, "waiting");
  assert.match(progressFeature({ status: "no_recent_sample" }).detail, /liveness not established/);
  assert.equal(progressFeature({ status: "recent_kernel_sample", sample: {} }).state, "ready");
  assert.match(progressFeature({ status: "recent_kernel_sample", sample: {} }).detail, /liveness not established/);
  assert.equal(progressFeature({ status: "invalid_sample" }).state, "error");
  assert.equal(progressFeature({ status: "generated_rejected" }).state, "error");
});

test("owner outcomes are bounded cleanup facts, never liveness or health", () => {
  const value = { schema_version: 1, source: "capture_owner_outcome", status: "finished",
    result: { outcome: "collector_fault", collector_started: true, collector_reaped: true, endpoint_removed: false } };
  const copy = readOwnerOutcome(value);
  assert.deepEqual(copy, value);
  copy.result.endpoint_removed = true;
  assert.equal(value.result.endpoint_removed, false);
  for (const status of ["not_configured", "awaiting_completion", "unavailable"]) {
    assert.equal(readOwnerOutcome({ ...value, status, result: null }).status, status);
    assert.equal(readOwnerOutcome({ ...value, status }), null);
  }
  for (const change of [{ schema_version: 2 }, { source: "generated_replay" }, { status: "running" },
    { sensor_running: true }, { result: null }, { path: "/private" },
    { result: { ...value.result, outcome: "unknown" } },
    { result: { ...value.result, collector_reaped: 1 } },
    { result: { ...value.result, collector_started: false } },
    { result: { ...value.result, collector_reaped: false } },
    { result: { ...value.result, outcome: "spawn_failed" } },
    { result: { ...value.result, accepted: 1 } }]) {
    assert.equal(readOwnerOutcome({ ...value, ...change }), null);
  }
  for (const outcome of ["completed", "collector_fault", "cancelled", "deadline", "invalid_stream", "read_failed", "wait_failed", "cleanup_failed"]) {
    assert.equal(readOwnerOutcome({ ...value, result: { ...value.result, outcome } }).result.outcome, outcome);
  }
  for (const outcome of ["spawn_failed", "invalid_duration", "cancelled"]) {
    assert.ok(readOwnerOutcome({ ...value, result: { ...value.result, outcome, collector_started: false, collector_reaped: false } }));
  }
});

const snapshot = () => ({ schema_version: 1, source: "owner_progress_subscription", status: "recent_kernel_sample",
  sensor_running: false, valid_for_ms: 2000,
  sample: { schema_version: 1, accepted: 2, rejected_record: 0, rejected_clock: 0, ring_dropped: 3 } });

test("progress freshness starts at request time and expires without extending receipt age", () => {
  let now = 1200;
  const view = createProgressHandoff(() => now);
  assert.equal(view.read(), null);
  assert.equal(view.accept(snapshot(), 1000).valid_for_ms, 1800);
  now = 2999;
  assert.equal(view.read().valid_for_ms, 1);
  now = 3000;
  assert.equal(view.read().status, "no_recent_sample");
  assert.equal(view.read().sample, null);
  assert.equal(view.read().valid_for_ms, 0);
  now = 4000;
  assert.equal(view.accept(snapshot(), 1000).sample, null);
});

test("handoff rejects malformed, historical, generated, active and unsafe samples", () => {
  const view = createProgressHandoff(() => 1000);
  for (const change of [{ schema_version: 2 }, { source: "generated_replay" }, { source: "operator_supplied_snapshot" },
    { sensor_running: true }, { valid_for_ms: 2001 }, { valid_for_ms: 0 }, { valid_for_ms: 1.1 },
    { status: "observing" }, { path: "private" }, { sample: null },
    { sample: { ...snapshot().sample, accepted: Number.MAX_SAFE_INTEGER + 1 } },
    { sample: { ...snapshot().sample, ring_dropped: -1 } },
    { sample: { ...snapshot().sample, schema_version: 0 } },
    { sample: { ...snapshot().sample, user_id: "private" } }]) {
    assert.equal(view.accept(snapshot(), 1000).status, "recent_kernel_sample");
    assert.equal(view.accept({ ...snapshot(), ...change }, 1000), null);
    assert.equal(view.read(), null);
  }
  for (const requestedAt of [-1, NaN, Infinity, 1001, undefined]) {
    assert.equal(view.accept(snapshot(), requestedAt), null);
  }
});

test("inactive states clear counters and require zero freshness with no sample", () => {
  const view = createProgressHandoff(() => 1000);
  for (const status of ["not_connected", "no_recent_sample", "generated_rejected", "invalid_sample"]) {
    const inactive = { ...snapshot(), status, valid_for_ms: 0, sample: null };
    view.accept(snapshot(), 1000);
    assert.equal(view.accept(inactive, 1000).sample, null);
    assert.equal(view.read().status, status);
    assert.equal(view.accept({ ...inactive, sample: snapshot().sample }, 1000), null);
    assert.equal(view.accept({ ...inactive, valid_for_ms: 1 }, 1000), null);
  }
});

test("handoff copies counters and fails closed on clock regression or explicit clear", () => {
  let now = 1000;
  const view = createProgressHandoff(() => now);
  const input = snapshot();
  const output = view.accept(input, 1000);
  input.sample.accepted = 999;
  output.sample.accepted = 888;
  assert.equal(view.read().sample.accepted, 2);
  now = 999;
  assert.equal(view.read(), null);
  now = 1100;
  assert.equal(view.read(), null);
  view.accept(snapshot(), 1100);
  view.clear();
  assert.equal(view.read(), null);
});