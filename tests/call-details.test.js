import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { build } from "esbuild";
import { eventDetails, recapDetails, compareRecaps, mountCallDetails } from "../src/web/call-details.js";

const first = "9007199254740993";
const second = "9007199254740992";
const user = { id: first, username: "morgan", display_name: "<script>Morgan</script>", avatar_url: "https://cdn.discordapp.com/avatars/1/a.png" };
const channel = { id: "200", name: "Group <img>", kind: 3 };
const leave = { sequence: 8, timestamp: 2000, action: "left", user, channel };
const joined = { sequence: 9, timestamp: 2300, action: "joined", user, channel, reconnect_of: 8, reconnect_gap_ms: 300 };
const call = (id = 2, started_at = 1000, overrides = {}) => ({
  id, channel, started_at, ended_at: started_at + 4000,
  recap: { baseline_count: 2, participant_ids: [first, second], peak_participants: 3, joined_count: 4, left_count: 5, probable_reconnections: 1 }, ...overrides,
});

function fixture(tools = {}) {
  const { window, document } = parseHTML('<html><body><main id="menu"></main><button id="origin">Origin</button></body></html>');
  const originalDocument = globalThis.document;
  const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  globalThis.document = document;
  let focused = document.querySelector("#origin");
  Object.defineProperty(document, "activeElement", { get: () => focused });
  window.HTMLElement.prototype.focus = function () { focused = this; this.dispatchEvent(new window.Event("focusin", { bubbles: true })); };
  let selected = null;
  window.HTMLElement.prototype.select = function () { selected = this; };
  const copied = [];
  const clipboard = { writeText: async value => { copied.push(value); } };
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard } });
  const details = mountCallDetails(tools);
  return {
    window, document, details, clipboard, copied, get selected() { return selected; },
    row: field => [...document.querySelectorAll("[data-field]")].find(row => row.dataset.field === field),
    async click(label) {
      const button = document.querySelector(`button[aria-label="${label}"]`);
      assert.ok(button, label);
      button.click();
      await Promise.resolve();
      await Promise.resolve();
    },
    key(key, shiftKey = false) {
      const event = new window.Event("keydown", { bubbles: true, cancelable: true });
      Object.assign(event, { key, shiftKey });
      document.dispatchEvent(event);
      return event;
    },
    cleanup() {
      details.dispose();
      globalThis.document = originalDocument;
      if (originalNavigator) Object.defineProperty(globalThis, "navigator", originalNavigator);
      else delete globalThis.navigator;
    },
  };
}

test("event projection includes observed fields, dates, reconnect links and retained raw leave only", () => {
  const data = eventDetails(joined, call(), [leave, joined]);
  assert.equal(data.action, "joined");
  assert.equal(data.sequence, 9);
  assert.deepEqual(data.user, user);
  assert.deepEqual(data.channel, channel);
  assert.equal(data.timestamp.iso, new Date(2300).toISOString());
  assert.equal(data.timestamp.local, new Date(2300).toLocaleString());
  assert.equal(data.call.id, 2);
  assert.equal(data.call.started_at.iso, new Date(1000).toISOString());
  assert.equal(data.call.ended_at.iso, new Date(5000).toISOString());
  assert.equal(data.reconnect_of, 8);
  assert.equal(data.reconnect_gap_ms, 300);
  assert.equal(data.raw_leave_event.sequence, 8);
  assert.equal(data.raw_leave_status, "Retained");
  assert.equal(eventDetails(leave, call(), [joined]).linked_probable_reconnect_events[0].sequence, 9);
  assert.match(data.observation, /not the actual network cause.*No cause is inferred.*not join events/);
  assert.match(eventDetails(joined, call(), []).raw_leave_status, /no longer retained/);
  assert.equal(eventDetails(joined, call(), [{ ...leave, user: { id: second } }]).raw_leave_event, null);
  assert.equal(eventDetails(joined, call(), [{ ...leave, channel: { id: "201" } }]).raw_leave_event, null);
  assert.equal(eventDetails(leave).raw_leave_event, null);
  assert.match(eventDetails(leave).raw_leave_status, /Not applicable/);
  assert.match(eventDetails(leave).call_end_status, /Ongoing or end not observed/);
  assert.equal(eventDetails({}).timestamp.iso, null);
});

test("recaps gate strictly on ended group calls, preserve baseline counts and warn at retention cap", () => {
  for (const invalid of [null, call(1, 0, { recap: null }), call(1, 0, { ended_at: null }),
    ...[undefined, 1, 2, "3", 13].map(kind => call(1, 0, { channel: { ...channel, kind } }))]) {
    assert.equal(recapDetails(invalid), null);
    assert.equal(compareRecaps(call(), invalid), null);
  }
  const data = recapDetails(call());
  assert.equal(data.observed_duration_ms, 4000);
  assert.equal(data.baseline_count, 2);
  assert.equal(data.unique_retained_participants, 2);
  assert.equal(data.peak_participants, 3);
  assert.equal(data.joined_count, 4);
  assert.equal(data.left_count, 5);
  assert.equal(data.probable_reconnections, 1);
  assert.equal(data.participant_ids_truncated, false);
  assert.deepEqual(data.participant_ids, [second, first]);
  const capped = call();
  capped.recap.participant_ids = Array.from({ length: 10000 }, (_, index) => String(index + 1));
  assert.match(recapDetails(capped).retention, /may be truncated/);
  assert.match(compareRecaps(capped, call(1)).retention, /truncated/);
  const truncated = call();
  truncated.recap.participant_ids_truncated = true;
  assert.equal(recapDetails(truncated).participant_ids_truncated, true);
  assert.match(recapDetails(truncated).retention, /were truncated/);
  assert.equal(compareRecaps(truncated, call(1)).participant_ids_truncated, true);
  assert.equal(compareRecaps(call(1), truncated).other_participant_ids_truncated, true);
  assert.match(compareRecaps(call(1), truncated).retention, /were truncated/);
  assert.equal(recapDetails(call(1, 5000, { ended_at: 4000 })).observed_duration_ms, null);
  assert.equal(recapDetails(call(1, 0, { ended_at: Infinity })).observed_duration_ms, null);
});

test("comparison is same exact group only, in either temporal direction, with unique ID overlap and deltas", () => {
  const other = call(1, 0);
  other.ended_at = 2000;
  other.recap = { ...other.recap, participant_ids: [second, "3", "3"], joined_count: 1, baseline_count: 1 };
  const data = compareRecaps(call(), other);
  assert.equal(data.deltas.observed_duration_ms, 2000);
  assert.equal(data.deltas.joined_count, 3);
  assert.equal(data.deltas.baseline_count, 1);
  assert.equal(data.deltas.unique_retained_participants, 0);
  assert.deepEqual(data.shared_participant_ids, [second]);
  assert.deepEqual(data.only_this_participant_ids, [first]);
  assert.deepEqual(data.only_other_participant_ids, ["3"]);
  assert.equal(data.shared_participant_count, 1);
  assert.equal(data.only_this_participant_count, 1);
  assert.equal(data.only_other_participant_count, 1);
  assert.equal(compareRecaps(other, call()).deltas.observed_duration_ms, -2000);
  for (const invalid of [call(), call(1, 0, { channel: { ...channel, id: "0200" } }), call(1, 0, { ended_at: null }), call(1, 0, { channel: { ...channel, kind: 2 } })]) {
    assert.equal(compareRecaps(call(), invalid), null);
  }
});

test("event modal is an owned independent themed dialog; every explanation leaf is individually copyable and in text/JSON", async () => {
  const state = fixture();
  try {
    state.details.event(joined, call(), [leave, joined]);
    const dialog = state.document.querySelector('[role="dialog"]');
    assert.equal(dialog.getAttribute("aria-modal"), "true");
    assert.equal(dialog.parentElement.parentElement, state.document.body);
    assert.equal(dialog.hasAttribute("data-intellicord-owned"), true);
    assert.equal(dialog.parentElement.hasAttribute("data-intellicord-owned"), true);
    assert.match(state.document.querySelector("style").textContent, /width:640px;max-width:100%/);
    assert.match(state.document.querySelector("style").textContent, /max-height:calc\(100dvh - 32px\);overflow:auto/);
    assert.match(state.document.querySelector("style").textContent, /theme-light \.ic-user-tool/);
    assert.equal(dialog.querySelector("script,img,audio,video,a"), null);
    assert.equal(state.row("user.display_name").querySelector("dd").textContent, user.display_name);
    for (const row of dialog.querySelectorAll("[data-field]")) {
      const copy = row.querySelector("button");
      assert.equal(copy.title, `Copy ${row.dataset.field}`);
      assert.equal(copy.querySelector("svg").getAttribute("width"), "18");
      await state.click(copy.title);
      assert.equal(state.copied.at(-1), row.dataset.field === "reconnect_gap_ms" ? "300" : row.querySelector("dd").textContent);
    }
    await state.click("Copy all JSON");
    assert.deepEqual(JSON.parse(state.copied.at(-1)), eventDetails(joined, call(), [leave, joined]));
    await state.click("Copy all text");
    assert.ok(state.copied.at(-1).includes("reconnect_gap_ms: 300"));
    assert.ok(state.copied.at(-1).includes(`user.id: ${first}`));
    assert.equal(state.document.querySelector('[role="status"]').textContent, "Copied");
  } finally { state.cleanup(); }
});

test("visible labels explain nested fields and duration display leaves raw copies unchanged", async () => {
  const state = fixture();
  try {
    state.details.event(joined, call(), [leave]);
    for (const [field, label] of [["user.id", "User ID"], ["timestamp.iso", "Observed at (UTC)"],
      ["call.channel.kind", "Call channel type"], ["raw_leave_event.user.id", "Retained leave event user ID"],
      ["raw_leave_event.timestamp", "Retained leave event: observed at (unix ms)"]]) {
      assert.equal(state.row(field).querySelector("dt").textContent, label);
    }
    assert.equal(state.row("reconnect_gap_ms").querySelector("dd").textContent, "0.3 sec");
    await state.click("Copy reconnect_gap_ms");
    assert.equal(state.copied.at(-1), "300");
    for (const [milliseconds, formatted] of [[4000, "4 sec"], [125000, "2 min 5 sec"], [3661000, "1:01:01"], [3661125, "1:01:01.125"]]) {
      state.details.recap(call(2, 0, { ended_at: milliseconds }), [call(1, 0)]);
      const summary = state.document.querySelector('[aria-label="Call summary"]');
      assert.ok(summary.textContent.includes(channel.name));
      assert.ok(summary.textContent.includes(formatted));
      assert.match(summary.textContent, /2 participants already present.*not join events/);
      assert.match(summary.textContent, /4 observed joins; 5 observed leaves/);
      assert.match(summary.textContent, /Ended at \(local time\)/);
      assert.equal(summary.textContent.includes(first), false);
      assert.equal(state.document.querySelector("details").hasAttribute("open"), false);
      assert.equal(state.row("observed_duration_ms").querySelector("dd").textContent, formatted);
      await state.click("Copy observed_duration_ms");
      assert.equal(state.copied.at(-1), String(milliseconds));
      await state.click("Copy all JSON");
      const exported = JSON.parse(state.copied.at(-1));
      assert.deepEqual(exported.participant_ids, [second, first]);
      assert.equal(exported.comparison.shared_participant_count, 2);
      assert.equal(exported.comparison.participant_ids_truncated, false);
      await state.click("Copy all text");
      assert.ok(state.copied.at(-1).includes(`observed_duration_ms: ${milliseconds}`));
      assert.ok(state.copied.at(-1).includes(first));
    }
    state.details.recap(call(2, 0, { ended_at: 0 }), [call(1, 0)]);
    assert.equal(state.row("observed_duration_ms").querySelector("dd").textContent, "0 sec");
    assert.equal(state.row("comparison.deltas.observed_duration_ms").querySelector("dd").textContent, "-4 sec");
    await state.click("Copy comparison.deltas.observed_duration_ms");
    assert.equal(state.copied.at(-1), "-4000");
    const truncated = call(1);
    truncated.recap.participant_ids_truncated = true;
    state.details.recap(call(), [truncated]);
    assert.match(state.document.querySelector('[aria-label="Call summary"]').textContent, /were truncated/);
    await state.click("Copy all JSON");
    assert.equal(JSON.parse(state.copied.at(-1)).comparison.other_participant_ids_truncated, true);
  } finally { state.cleanup(); }
});

test("picker orders newest first, defaults to most recent earlier and switches comparison exports", async () => {
  const state = fixture();
  try {
    state.details.recap(call(4, 5000), [call(1, 0), call(8, 9000), call(3, 4000), call(2, 2000), call(3, 4000), call(4, 5000), call(7, 0, { ended_at: null }), call(9, 0, { channel: { ...channel, id: "201" } })]);
    const picker = state.document.querySelector("select");
    assert.deepEqual([...picker.options].map(option => option.value), ["8", "3", "2", "1"]);
    assert.equal(picker.value, "3");
    assert.equal(state.row("comparison.other_call.id").querySelector("dd").textContent, "3");
    [...picker.options].find(option => option.value === "8").selected = true;
    picker.dispatchEvent(new state.window.Event("change"));
    assert.equal(state.row("comparison.other_call.id").querySelector("dd").textContent, "8");
    await state.click("Copy all JSON");
    assert.equal(JSON.parse(state.copied.at(-1)).comparison.other_call.id, 8);
    state.details.recap(call(1, 0), [call(2, 5000)]);
    assert.equal(state.document.querySelector("select").value, "2");
    state.details.recap(call(), []);
    assert.equal(state.document.querySelector("select"), null);
    for (const invalid of [call(1, 0, { ended_at: null }), call(1, 0, { channel: { ...channel, kind: 2 } })]) {
      state.details.dispose();
      const details = mountCallDetails();
      details.recap(invalid, []);
      assert.equal(state.document.querySelector('[role="dialog"]'), null);
      details.dispose();
    }
  } finally { state.cleanup(); }
});

test("clipboard rejection or missing API produces selectable readonly fallback, guarded against stale async results", async () => {
  const state = fixture();
  try {
    state.details.event(joined, call(), []);
    state.clipboard.writeText = async () => { throw new Error("denied"); };
    await state.click("Copy user.id");
    const fallback = state.document.querySelector("textarea");
    assert.equal(fallback.value, first);
    assert.equal(fallback.readOnly, true);
    assert.equal(state.document.activeElement, fallback);
    assert.equal(state.selected, fallback);
    state.key("Tab");
    assert.equal(state.document.activeElement.title, "Copy all text");
    state.key("Tab", true);
    assert.equal(state.document.activeElement, fallback);
    state.clipboard.writeText = undefined;
    await state.click("Copy all JSON");
    assert.equal(state.document.querySelectorAll("textarea").length, 1);
    assert.deepEqual(JSON.parse(state.document.querySelector("textarea").value), eventDetails(joined, call(), []));
    let reject;
    state.clipboard.writeText = () => new Promise((resolve, rejectPromise) => { reject = rejectPromise; });
    await state.click("Copy user.id");
    state.details.recap(call(), []);
    reject(new Error("late"));
    await Promise.resolve();
    assert.equal(state.document.querySelector("textarea"), null);
    assert.equal(state.document.querySelector('[role="status"]').textContent, "");
  } finally { state.cleanup(); }
});

test("focus trap includes select; Escape/outside/reopening/disposal restore focus and remove listeners", () => {
  const state = fixture();
  const origin = state.document.activeElement;
  try {
    state.details.recap(call(), [call(1, 0)]);
    const close = state.document.activeElement;
    assert.equal(close.title, "Close Call recap");
    state.key("Tab");
    assert.equal(state.document.activeElement.tagName, "SELECT");
    state.key("Tab", true);
    assert.equal(state.document.activeElement, close);
    origin.focus();
    assert.equal(state.document.activeElement.title, "Copy all text");
    assert.equal(state.key("Escape").defaultPrevented, true);
    assert.equal(state.document.activeElement, origin);
    assert.equal(state.key("Tab").defaultPrevented, false);
    for (let index = 0; index < 6; index++) {
      state.details.event(joined, call(), []);
      state.details.recap(call(), []);
      assert.equal(state.document.querySelectorAll('[role="dialog"]').length, 1);
      state.document.querySelector('[role="dialog"]').click();
      assert.ok(state.document.querySelector('[role="dialog"]'));
      state.document.querySelector(".ic-details-root").click();
      assert.equal(state.document.activeElement, origin);
      assert.equal(state.document.querySelector("style"), null);
    }
    state.details.event(joined);
    state.details.dispose();
    state.details.dispose();
    state.details.event(joined);
    state.details.recap(call(), []);
    assert.equal(state.document.querySelector('[role="dialog"]'), null);
    assert.equal(state.key("Escape").defaultPrevented, false);
    assert.equal(state.document.activeElement, origin);
  } finally { state.cleanup(); }
});

test("collapsed recap details skip hidden copy controls and expanded details join the focus cycle", () => {
  const state = fixture();
  try {
    state.details.recap(call(), [call(1, 0)]);
    state.key("Tab");
    assert.equal(state.document.activeElement.tagName, "SELECT");
    state.key("Tab");
    const summary = state.document.activeElement;
    assert.equal(summary.tagName, "SUMMARY");
    state.key("Tab");
    assert.equal(state.document.activeElement.title, "Copy all text");
    state.key("Tab", true);
    assert.equal(state.document.activeElement, summary);
    summary.parentElement.setAttribute("open", "");
    state.key("Tab");
    assert.equal(state.document.activeElement.title, "Copy observation");
    state.key("Tab", true);
    assert.equal(state.document.activeElement, summary);
  } finally { state.cleanup(); }
});

test("reopening after a trigger is removed never restores focus to a removed dialog control", () => {
  const state = fixture();
  try {
    const origin = state.document.activeElement;
    state.details.event(joined, call(), [leave]);
    const removedControl = state.document.activeElement;
    origin.remove();
    state.details.recap(call(), []);
    assert.equal(removedControl.isConnected, false);
    assert.equal(state.document.activeElement.title, "Close Call recap");
    state.key("Escape");
    assert.equal(state.document.activeElement.isConnected, true);
    assert.notEqual(state.document.activeElement, removedControl);
    assert.notEqual(state.document.activeElement, origin);
  } finally { state.cleanup(); }
});

test("optional tools receive observed safe user only, after details focus trap is closed", async () => {
  const opened = [];
  let state;
  const tool = { open(id, fallback) {
    assert.equal(state.document.querySelector('[role="dialog"]'), null);
    assert.equal(state.key("Tab").defaultPrevented, false);
    opened.push([id, fallback]);
  } };
  state = fixture({ identity: tool, watch: tool });
  try {
    for (const label of ["Identity Lens", "Voice Watch"]) {
      state.details.event(joined);
      await state.click(label);
    }
    assert.deepEqual(opened, [[first, user], [first, user]]);
    state.details.recap(call(), []);
    assert.equal(state.document.querySelector('[aria-label="Identity Lens"]'), null);
  } finally { state.cleanup(); }
});

test("privacy allowlists never access secrets, arbitrary toJSON, stores, network, or synthesize baseline events", async () => {
  const poison = object => {
    for (const key of ["token", "email", "phone", "messages", "audio", "toJSON"]) Object.defineProperty(object, key, { get() { assert.fail(`Read private field ${key}`); } });
    return object;
  };
  const privateUser = poison({ ...user });
  const privateEvent = poison({ ...joined, user: privateUser });
  const privateCall = poison(call());
  privateCall.recap = poison({ ...privateCall.recap });
  const state = fixture();
  try {
    state.details.event(privateEvent, privateCall, [poison({ ...leave, user: privateUser })]);
    await state.click("Copy all JSON");
    assert.doesNotMatch(state.copied.at(-1), /token|email|phone|messages|audio|toJSON/);
    assert.equal(JSON.parse(state.copied.at(-1)).raw_leave_event.action, "left");
    state.details.recap(privateCall, [poison(call(1))]);
    await state.click("Copy all JSON");
    const exported = JSON.parse(state.copied.at(-1));
    assert.equal(exported.baseline_count, 2);
    assert.equal(exported.joined_count, 4);
    assert.equal(exported.events, undefined);
    assert.equal(exported.comparison.deltas.joined_count, 0);
    assert.equal(state.document.querySelector("img,audio,video,iframe"), null);
  } finally { state.cleanup(); }
});

test("details bundle with the existing browser toolchain", async () => {
  const result = await build({ entryPoints: [new URL("../src/web/call-details.js", import.meta.url).pathname], bundle: true, write: false, format: "iife", logLevel: "silent" });
  assert.ok(result.outputFiles[0].text.includes("ic-details-dialog"));
});