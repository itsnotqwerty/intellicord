import test from "node:test";
import assert from "node:assert/strict";
import { acousticFeature, readAcousticOwner, readAcousticSummary } from "../src/web/acoustic-status.js";

const summary = () => ({ schema_version: 1, source: "generated_pcm", status: "receiving", frames: 1,
  discontinuities: 0, missing_messages: 0, last_receipt_ns: 20000000,
  latest: { source: "generated_pcm", frame_start_ns: 0, rms: 0.5, peak: 0.5, clipped_sample_ratio: 0, discontinuity: false } });

test("acoustic session summaries are bounded copied measurements, not participant evidence", () => {
  const value = summary(), copy = readAcousticSummary(value);
  assert.deepEqual(copy, value);
  value.latest.rms = 0.1;
  assert.equal(copy.latest.rms, 0.5);
  for (const change of [{ source: "discord_user" }, { status: "live" },
    { frames: -1 }, { discontinuities: 2 }, { latest: null }, { user_id: "123" },
    { latest: { ...summary().latest, rms: NaN } }, { latest: { ...summary().latest, source: "mixed_output" } },
    { latest: { ...summary().latest, samples: [] } }, { last_receipt_ns: Number.MAX_SAFE_INTEGER + 1 }]) {
    assert.equal(readAcousticSummary({ ...summary(), ...change }), null);
  }
  assert.equal(acousticFeature(null).state, "ready");
  assert.equal(acousticFeature(undefined).state, "unsupported");
  assert.equal(acousticFeature({ ...summary(), status: "faulted" }).state, "error");
  assert.equal(acousticFeature({ ...summary(), status: "starting", frames: 0, latest: null }).state, "waiting");
  assert.match(acousticFeature(summary()).detail, /liveness unverified/);
});

test("version 2 acoustic summaries strictly reduce and copy latest derived analysis", () => {
  const analysis = { centroid_hz: 1200.5, high_band_power_ratio: 0.2, positive_power_flux: 0.3,
    compared_lags: 4, best_lag_ns: 160000000, best_similarity: 0.8 };
  const value = { ...summary(), schema_version: 2, latest: { ...summary().latest, analysis } };
  const copy = readAcousticSummary(value);
  assert.deepEqual(copy, value);
  analysis.best_similarity = 0.1;
  assert.equal(copy.latest.analysis.best_similarity, 0.8);
  assert.deepEqual(readAcousticSummary({ ...value, latest: { ...value.latest, analysis: null } }).latest.analysis, null);
  for (const invalid of [{ ...value, latest: { ...value.latest, analysis: { ...analysis, unknown: 1 } } },
    { ...value, latest: { ...value.latest, analysis: { ...analysis, centroid_hz: Infinity } } },
    { ...value, latest: { ...value.latest, analysis: { ...analysis, high_band_power_ratio: 1.1 } } },
    { ...value, latest: { ...value.latest, analysis: { ...analysis, compared_lags: 47 } } },
    { ...value, latest: { ...value.latest, analysis: { ...analysis, best_lag_ns: 150000001 } } },
    { ...value, latest: { ...value.latest, analysis: { ...analysis, best_lag_ns: null } } }]) {
    assert.equal(readAcousticSummary(invalid), null);
  }
});

test("version 3 acoustic summaries preserve a bounded backend session interval", () => {
  const interval = { earliest_ns: 10000000, latest_ns: 10001000 };
  const value = { ...summary(), schema_version: 3,
    latest: { ...summary().latest, session_start_interval_ns: interval, analysis: null } };
  const copy = readAcousticSummary(value);
  assert.deepEqual(copy, value);
  interval.earliest_ns = 0;
  assert.equal(copy.latest.session_start_interval_ns.earliest_ns, 10000000);
  for (const invalid of [
    { ...value, latest: { ...value.latest, session_start_interval_ns: null } },
    { ...value, latest: { ...value.latest, session_start_interval_ns: { earliest_ns: 2, latest_ns: 1 } } },
    { ...value, latest: { ...value.latest, session_start_interval_ns: { earliest_ns: 1, latest_ns: 2, clock: "source" } } },
  ]) assert.equal(readAcousticSummary(invalid), null);
});

test("version 4 acoustic summaries strictly retain bounded processing health", () => {
  const processing_health = { schema_version: 1, status: { state: "active" }, capacity_batches: 8,
    pending: { batches: 0, frames: 0 }, processed: { batches: 2, frames: 2 },
    delivered: { batches: 2, frames: 2 }, discarded: { batches: 0, frames: 0 },
    overflow_events: 0, high_water_batches: 1 };
  const value = { ...summary(), schema_version: 4, processing_health,
    latest: { ...summary().latest, session_start_interval_ns: { earliest_ns: 1, latest_ns: 2 }, analysis: null } };
  const copy = readAcousticSummary(value);
  assert.deepEqual(copy, value);
  processing_health.delivered.frames = 0;
  assert.equal(copy.processing_health.delivered.frames, 2);
  for (const invalid of [
    { ...value, processing_health: { ...value.processing_health, capacity_batches: 9 } },
    { ...value, processing_health: { ...value.processing_health, processed: { batches: 3, frames: 2 } } },
    { ...value, processing_health: { ...value.processing_health, pending: { batches: 9, frames: 0 } } },
    { ...value, processing_health: { ...value.processing_health, status: { state: "faulted" } } },
    { ...value, processing_health: { ...value.processing_health, status: { state: "faulted", error: "unknown" } } },
  ]) assert.equal(readAcousticSummary(invalid), null);
  const degraded = { ...value, processing_health: { ...value.processing_health,
    discarded: { batches: 1, frames: 1 }, delivered: { batches: 1, frames: 1 } } };
  assert.equal(acousticFeature(degraded).state, "error");
  assert.match(acousticFeature(degraded).detail, /discarded data/);
  const completedOwner = { schema_version: 1, status: "finished", call_bound: true,
    result: { outcome: "completed", cleanup_confirmed: true } };
  assert.equal(acousticFeature(degraded, completedOwner).state, "error");
  assert.match(acousticFeature(degraded, completedOwner).detail, /cleanup confirmed/);
});

test("acoustic owner lifecycle distinguishes preparation, capture outcome, and cleanup", () => {
  const owner = (status, result = null, call_bound = status !== "not_configured", schema_version = 1) => ({ schema_version, status, call_bound, result });
  assert.deepEqual(readAcousticOwner(owner("prepared")), owner("prepared"));
  assert.equal(acousticFeature(null, owner("prepared")).state, "ready");
  assert.equal(acousticFeature(null, owner("running")).state, "waiting");
  assert.match(acousticFeature(summary(), owner("finished", { outcome: "completed", cleanup_confirmed: true })).detail,
    /cleanup confirmed/);
  assert.match(acousticFeature({ ...summary(), status: "faulted" },
    owner("finished", { outcome: "faulted", cleanup_confirmed: false })).detail, /cleanup unconfirmed/);
  const classified = owner("finished", { outcome: "faulted", cleanup_confirmed: true,
    reason: "stream_ended" }, true, 2);
  assert.deepEqual(readAcousticOwner(classified), classified);
  for (const invalid of [owner("finished"), owner("running", { outcome: "faulted", cleanup_confirmed: true }),
    owner("finished", { outcome: "completed", cleanup_confirmed: false }),
    owner("finished", { outcome: "faulted", cleanup_confirmed: true, reason: null }, true, 2),
    owner("finished", { outcome: "completed", cleanup_confirmed: true, reason: "internal_failure" }, true, 2),
    owner("not_configured", null, true),
    { ...owner("prepared"), extra: true }]) assert.equal(readAcousticOwner(invalid), null);
  assert.equal(acousticFeature(null, owner("not_configured")).state, "ready");
  assert.equal(acousticFeature(summary(), owner("not_configured")).state, "unsupported");
});