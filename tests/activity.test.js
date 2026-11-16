import test from "node:test";
import assert from "node:assert/strict";
import { activityUsers, filterActivity, formatTextLog } from "../src/web/activity.js";

const user = (id, displayName = "Morgan") => ({ id, username: `user_${id}`, display_name: displayName });
const event = (sequence, id, action = "joined", timestamp = sequence * 1000) => ({
  sequence, timestamp, action, user: user(id), channel: { id: "100", name: "General" },
});

test("user choices group by exact ID and use the latest recorded name", () => {
  const renamed = event(3, "9007199254740993");
  renamed.user.display_name = "Renamed";
  const users = activityUsers([event(1, "9007199254740993"), event(2, "9007199254740992"), renamed]);
  assert.equal(users.length, 2);
  assert.equal(users.find(user => user.id === "9007199254740993").display_name, "Renamed");
});

test("current callers remain selectable before any activity is recorded", () => {
  const current = user("9007199254740993");
  assert.deepEqual(activityUsers([], { [current.id]: current }), [current]);
  assert.equal(filterActivity([], { userIds: new Set([current.id]) }).length, 0);
});

test("user selection composes with action and search filters without mutating history", () => {
  const events = [event(1, "9007199254740993"), event(2, "2"), event(3, "9007199254740993", "left")];
  assert.deepEqual(filterActivity(events, { userIds: new Set(["9007199254740993"]) }), [events[0], events[2]]);
  assert.deepEqual(filterActivity(events, { userIds: new Set(["9007199254740993"]), action: "left", query: "GENERAL" }), [events[2]]);
  assert.deepEqual(filterActivity(events, { userIds: new Set() }), []);
  assert.equal(filterActivity(events).length, 3);
  assert.equal(events.length, 3);
});

test("text log includes exact IDs and chronological event details", () => {
  const events = [event(3, "9007199254740993", "left"), event(1, "2"), event(2, "9007199254740993")];
  const log = formatTextLog(events, 0);
  assert.match(log, /Exported: 1970-01-01T00:00:00.000Z/);
  assert.match(log, /Events: 3/);
  assert.match(log, /LEFT user="Morgan" username="user_9007199254740993" user_id="9007199254740993" channel="General" channel_id="100"/);
  assert.ok(log.indexOf("00:00:01.000Z") < log.indexOf("00:00:03.000Z"));
  assert.deepEqual(events.map(event => event.sequence), [3, 1, 2]);
});

test("log escapes names that could forge entries or insert terminal controls", () => {
  const entry = event(1, "1");
  entry.user.display_name = "Morgan\nFORGED\t\u001b[31m";
  entry.channel.name = "General\r\nFORGED";
  const log = formatTextLog([entry], 0);
  assert.match(log, /Morgan\\nFORGED\\t\\u001b\[31m/);
  assert.match(log, /General\\r\\nFORGED/);
  assert.equal(log.split("\n").filter(line => line.includes("FORGED")).length, 1);
  assert.ok(!log.includes("\u001b"));
});