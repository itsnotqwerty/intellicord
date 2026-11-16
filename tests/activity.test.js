import test from "node:test";
import assert from "node:assert/strict";
import { activityUsers, filterActivity, formatMarkdownLog } from "../src/web/activity.js";

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

test("Markdown log includes exact IDs and chronological event details", () => {
  const events = [event(3, "9007199254740993", "left"), event(1, "2"), event(2, "9007199254740993")];
  const log = formatMarkdownLog(events, 0);
  assert.ok(log.startsWith("# Callmon Voice Activity\n"));
  assert.match(log, /Exported: 1970-01-01T00:00:00.000Z/);
  assert.match(log, /Events: 3/);
  assert.match(log, /\| LEFT \| Morgan \| user&#95;9007199254740993 \| 9007199254740993 \| General \| 100 \|/);
  assert.ok(log.indexOf("00:00:01&#46;000Z") < log.indexOf("00:00:03&#46;000Z"));
  assert.deepEqual(events.map(event => event.sequence), [3, 1, 2]);
});

test("log escapes names that could forge entries or insert terminal controls", () => {
  const entry = event(1, "1");
  entry.user.display_name = "Morgan\nFORGED\t\u001b[31m";
  entry.channel.name = "General\r\nFORGED";
  const log = formatMarkdownLog([entry], 0);
  assert.ok(log.includes("Morgan&#92;nFORGED&#92;t&#92;u001b&#91;31m"));
  assert.ok(log.includes("General&#92;r&#92;nFORGED"));
  assert.equal(log.split("\n").filter(line => line.includes("FORGED")).length, 1);
  assert.ok(!log.includes("\u001b"));
});

test("Markdown fields cannot inject table columns, links, HTML, or formatting", () => {
  const entry = event(1, "9007199254740993");
  entry.user.display_name = '| [name](https://example.com) <img src=x> **bold** `code` &amp;';
  const log = formatMarkdownLog([entry], 0);
  const row = log.trimEnd().split("\n").at(-1);
  assert.equal(row.split("|").length, 9);
  assert.ok(row.includes("&#124; &#91;name&#93;&#40;https://example&#46;com&#41;"));
  assert.ok(row.includes("&#60;img src&#61;x&#62;"));
  assert.ok(row.includes("&#42;&#42;bold&#42;&#42; &#96;code&#96; &#38;amp;"));
  assert.ok(row.includes("9007199254740993"));
});

test("Markdown call metadata preserves identity and safely labels ongoing or completed calls", () => {
  const call = { channel: { id: "9007199254740993", name: "Call | <script>\nFORGED" }, started_at: 1000, ended_at: null };
  const ongoing = formatMarkdownLog([], 0, call);
  assert.ok(ongoing.includes("Call: Call &#124; &#60;script&#62;&#92;nFORGED (9007199254740993)"));
  assert.ok(ongoing.includes("Started: 1970-01-01T00:00:01.000Z"));
  assert.ok(ongoing.includes("Ended: Ongoing"));
  assert.ok(formatMarkdownLog([], 0, { ...call, ended_at: 2000 }).includes("Ended: 1970-01-01T00:00:02.000Z"));
});