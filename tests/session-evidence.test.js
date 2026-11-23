import assert from "node:assert/strict";
import test from "node:test";
import { readSessionEvidence, sessionEvidenceFeature } from "../src/web/session-evidence.js";

const evidence = overrides => ({
  schema_version: 1,
  source: "backend_session_evidence_aggregate",
  status: "independent_evidence",
  owner_integrity: "pending",
  process: "recent_kernel_sample",
  acoustic: "frame_available",
  cross_sensor_correlation: "unavailable",
  ...overrides,
});

test("session evidence accepts independent streams without implying correlation", () => {
  const value = evidence();
  assert.deepEqual(readSessionEvidence(value), value);
  assert.deepEqual(sessionEvidenceFeature(value), {
    state: "ready",
    detail: "Process and acoustic evidence available independently; correlation unavailable",
  });
});

test("session evidence rejects contradictory aggregate states and unknown fields", () => {
  assert.equal(readSessionEvidence(evidence({ status: "partial" })), null);
  assert.equal(readSessionEvidence(evidence({ cross_sensor_correlation: "available" })), null);
  assert.equal(readSessionEvidence(evidence({ extra: true })), null);
  assert.equal(readSessionEvidence(evidence({
    status: "sensor_free", owner_integrity: "settled", process: "not_configured", acoustic: "not_configured",
  })), null);
});

test("session evidence surfaces degradation and partial availability", () => {
  assert.equal(sessionEvidenceFeature(evidence({ status: "degraded", acoustic: "processing_loss" })).state, "error");
  assert.equal(sessionEvidenceFeature(evidence({ status: "partial", acoustic: "awaiting_frame" })).state, "waiting");
  assert.equal(sessionEvidenceFeature(null).state, "unsupported");
});