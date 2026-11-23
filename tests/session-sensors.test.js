import test from "node:test";
import assert from "node:assert/strict";
import { readSessionSensors, sessionSensorsFeature } from "../src/web/session-sensors.js";

const session = (change = {}) => ({ schema_version: 1, source: "owner_lifecycle_aggregate",
  status: "sensor_free", configured: 0, pending: 0, finished: 0, cleanup: "not_applicable", ...change });

test("combined sensor lifecycle distinguishes sensor-free, pending, settled and attention states", () => {
  assert.equal(sessionSensorsFeature(session()).state, "ready");
  assert.match(sessionSensorsFeature(session()).detail, /No optional sensors/);
  const pending = session({ status: "pending", configured: 2, pending: 2, cleanup: "pending" });
  assert.equal(sessionSensorsFeature(pending).state, "waiting");
  assert.match(sessionSensorsFeature(pending).detail, /2 pending/);
  const settled = session({ status: "settled", configured: 2, finished: 2, cleanup: "confirmed" });
  assert.equal(sessionSensorsFeature(settled).state, "ready");
  assert.match(sessionSensorsFeature(settled).detail, /cleanup confirmed/);
  const attention = session({ status: "attention", configured: 2, pending: 1, finished: 1, cleanup: "pending" });
  assert.equal(sessionSensorsFeature(attention).state, "error");
  assert.match(sessionSensorsFeature(attention).detail, /attention required/);
});

test("combined sensor lifecycle is exact, bounded and internally consistent", () => {
  const value = session({ status: "settled", configured: 1, finished: 1, cleanup: "confirmed" });
  const copy = readSessionSensors(value);
  assert.deepEqual(copy, value);
  copy.status = "attention";
  assert.equal(value.status, "settled");
  for (const invalid of [null, {}, session({ schema_version: 2 }), session({ source: "renderer_guess" }),
    session({ status: "active" }), session({ configured: 3 }), session({ pending: 1 }),
    session({ status: "pending", configured: 1, pending: 0, cleanup: "pending" }),
    session({ status: "settled", configured: 2, finished: 1, cleanup: "confirmed" }),
    session({ status: "attention", configured: 0, cleanup: "unconfirmed" }),
    session({ sensor_running: true })]) {
    assert.equal(readSessionSensors(invalid), null);
    assert.equal(sessionSensorsFeature(invalid).state, "unsupported");
  }
});