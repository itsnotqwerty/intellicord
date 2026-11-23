import test from "node:test";
import assert from "node:assert/strict";
import { readSessionTiming, sessionTimingFeature } from "../src/web/session-timing.js";

const timing = (change = {}) => ({ schema_version: 1, source: "backend_session_clock_inventory",
  voice_receipt_clock: "backend_session_monotonic", acoustic_receipt_clock: "unavailable",
  acoustic_frame_clock: "unavailable", process_timing: "unavailable",
  cross_sensor_correlation: "unavailable", ...change });

test("session timing distinguishes unavailable, relative and bounded clocks", () => {
  assert.match(sessionTimingFeature(timing()).detail, /optional sensor timing unavailable/);
  assert.match(sessionTimingFeature(timing({ process_timing: "receipt_freshness_only" })).detail, /freshness-only/);
  assert.match(sessionTimingFeature(timing({ acoustic_receipt_clock: "backend_session_monotonic",
    acoustic_frame_clock: "capture_relative" })).detail, /capture-relative/);
  assert.match(sessionTimingFeature(timing({ acoustic_receipt_clock: "backend_session_monotonic",
    acoustic_frame_clock: "backend_session_interval" })).detail, /calibrated correlation unavailable/);
});

test("session timing is exact and rejects contradictory clock claims", () => {
  const value = timing({ acoustic_receipt_clock: "backend_session_monotonic",
    acoustic_frame_clock: "backend_session_interval", process_timing: "receipt_freshness_only" });
  const copy = readSessionTiming(value);
  assert.deepEqual(copy, value);
  copy.process_timing = "unavailable";
  assert.equal(value.process_timing, "receipt_freshness_only");
  for (const invalid of [null, {}, timing({ schema_version: 2 }), timing({ source: "renderer_guess" }),
    timing({ voice_receipt_clock: "wall_time" }), timing({ acoustic_receipt_clock: "unavailable",
      acoustic_frame_clock: "capture_relative" }), timing({ acoustic_receipt_clock: "source_clock",
      acoustic_frame_clock: "backend_session_interval" }), timing({ process_timing: "event_clock" }),
    timing({ cross_sensor_correlation: "available" }), timing({ extra: true })]) {
    assert.equal(readSessionTiming(invalid), null);
    assert.equal(sessionTimingFeature(invalid).state, "unsupported");
  }
});
