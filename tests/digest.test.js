import assert from "node:assert/strict";
import test from "node:test";
import { createCallDigest, summarizeChanges } from "../src/web/digest.js";

const event = (sequence, action, reconnect_of) => ({ sequence, action, reconnect_of, user: { id: "1" } });
const state = (id, latest_sequence, events = []) => ({ current_call: { id }, latest_sequence, events });

test("digest separates probable returns from paired departures and arrivals without mutating raw events", () => {
  const events = [event(1, "left"), event(2, "joined", 1), event(3, "joined"), event(4, "left")];
  const digest = summarizeChanges(events, 0, 4);
  assert.deepEqual(digest.arrivals, [events[2]]);
  assert.deepEqual(digest.departures, [events[3]]);
  assert.deepEqual(digest.returns, [events[1]]);
  assert.equal(digest.unavailable, 0);
  assert.equal(events.length, 4);
  assert.equal(summarizeChanges(events, 1, 4).returns.length, 1);
});

test("opening reads once, visible changes are checked, and switching calls resets the baseline", () => {
  const digest = createCallDigest();
  digest.update(state(1, 0));
  digest.update(state(1, 1, [event(1, "joined")]));
  assert.equal(digest.read().arrivals.length, 1);
  assert.equal(digest.read().arrivals.length, 0);
  digest.update(state(1, 2, [event(1, "joined"), event(2, "left")]), true);
  assert.equal(digest.read().departures.length, 0);
  digest.update(state(2, 2));
  assert.equal(digest.read().unavailable, 0);
  digest.update(state(2, 3, [event(3, "joined")]));
  assert.equal(digest.read().arrivals.length, 1);
});

test("clear and retention gaps are explicit and attachment does not claim old events as unseen", () => {
  const digest = createCallDigest();
  digest.update(state(1, 20, [event(20, "left")]));
  assert.equal(digest.read().unavailable, 0);
  digest.update(state(1, 23, [event(23, "joined")]));
  assert.equal(digest.read().unavailable, 2);
  digest.update(state(1, 25));
  assert.equal(digest.read().unavailable, 2);
});