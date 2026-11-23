import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { parseHTML } from "linkedom";

const bundle = await build({ entryPoints: ["src/web/obliterate-panel.js"], bundle: true, write: false, format: "esm", platform: "browser" });
const { mountObliteratePanel } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);
const context = { userId: "9007199254740993", channelId: "200" };
const keys = ["mute", "soundboard", "video", "ignore", "block", "moderation"];
const makePlan = () => ({ target: { id: context.userId, name: "<img src=x> Exact Name" },
  context: { channelId: "200", guildId: "300", guildName: "Exact Guild" },
  actions: keys.map(key => ({ key, label: key === "moderation" ? "Ban member" : key, status: "ready", ...(key === "moderation" ? { severity: "ban" } : {}) })) });
const flush = async () => { for (let index = 0; index < 30; index++) await Promise.resolve(); };
function deferred() { let resolve, reject; const promise = new Promise((accept, fail) => { resolve = accept; reject = fail; }); return { promise, resolve, reject }; }
function fixture(overrides = {}) {
  const { window, document } = parseHTML('<html><body><button id="origin">Origin</button></body></html>');
  const original = globalThis.document;
  globalThis.document = document;
  let focused = document.querySelector("#origin");
  const origin = focused;
  Object.defineProperty(document, "activeElement", { get: () => focused });
  window.HTMLElement.prototype.focus = function () { focused = this; this.dispatchEvent(new window.Event("focusin", { bubbles: true })); };
  const calls = [];
  let plans = 0;
  const adapter = { plan: () => { plans++; return makePlan(); }, executeAction: async (...args) => { calls.push(args); return { status: "done" }; }, ...overrides };
  const panel = mountObliteratePanel(adapter);
  return { window, document, origin, panel, adapter, calls, get plans() { return plans; },
    confirm: () => document.querySelector("[data-confirm]"),
    key(key, shiftKey = false) { const event = new window.Event("keydown", { bubbles: true, cancelable: true }); Object.assign(event, { key, shiftKey }); document.dispatchEvent(event); return event; },
    cleanup() { panel.dispose(); globalThis.document = original; } };
}

test("read-only opening shows six exact safe rows, danger context, theme and Cancel default", async () => {
  const state = fixture();
  try {
    await state.panel.open(context);
    const dialog = state.document.querySelector('[role="alertdialog"]');
    assert.equal(dialog.getAttribute("aria-modal"), "true");
    assert.ok(dialog.hasAttribute("data-intellicord-owned"));
    assert.ok(dialog.parentElement.hasAttribute("data-intellicord-owned"));
    assert.equal(state.document.activeElement.textContent, "Cancel");
    assert.equal(dialog.querySelectorAll("li").length, 6);
    for (const text of [context.userId, "<img src=x> Exact Name", "200", "300", "Exact Guild", "Selected server action: ban", "may be irreversible", "No messages are deleted", "locally for you only", "account-wide"]) assert.ok(dialog.textContent.includes(text), text);
    assert.equal(dialog.querySelector("img"), null);
    assert.match(state.document.querySelector("style").textContent, /z-index:1004/);
    assert.match(state.document.querySelector("style").textContent, /width:560px;max-width:100%/);
    assert.match(state.document.querySelector("style").textContent, /theme-light/);
    assert.equal(state.plans, 1);
    assert.deepEqual(state.calls, []);
    state.document.activeElement.click();
    assert.equal(state.document.querySelector('[role="alertdialog"]'), null);
    assert.equal(state.document.activeElement, state.origin);
  } finally { state.cleanup(); }
});

test("one confirmation replans then executes sequentially with expected snapshots, moderation last, no rerun", async () => {
  const pending = deferred();
  const state = fixture();
  try {
    state.adapter.executeAction = async (...args) => { state.calls.push(args); if (args[1] === "mute") return pending.promise; return { status: "done" }; };
    await state.panel.open(context);
    state.confirm().click();
    await flush();
    assert.equal(state.plans, 2);
    assert.deepEqual(state.calls.map(call => call[1]), ["mute"]);
    state.confirm().click();
    await state.panel.open({ userId: "400", channelId: "500" });
    state.key("Escape");
    state.document.querySelector(".ic-obliterate-root").click();
    assert.ok(state.document.querySelector('[role="alertdialog"]'));
    assert.ok([...state.document.querySelectorAll('[role="alertdialog"] button,input')].every(element => element.disabled));
    pending.resolve({ status: "done" });
    await flush();
    assert.deepEqual(state.calls.map(call => call[1]), keys);
    for (const [request, key, expected] of state.calls) {
      assert.deepEqual(request, context);
      const action = makePlan().actions.find(action => action.key === key);
      assert.deepEqual(expected, key === "moderation" ? { ...action, guildId: makePlan().context.guildId } : action);
    }
    assert.equal(state.confirm().disabled, true);
    state.confirm().click();
    await flush();
    assert.equal(state.calls.length, 6);
    assert.equal(state.document.querySelectorAll('[data-result="moderation"]')[0].textContent, "done");
    assert.equal(state.document.activeElement.textContent, "Cancel");
  } finally { state.cleanup(); }
});

test("Cancel, Escape, close icon and backdrop never execute; Tab and focusin stay inside and cleanup restores focus", async () => {
  const state = fixture();
  try {
    for (const dismiss of [
      () => state.document.activeElement.click(),
      () => state.key("Escape"),
      () => state.document.querySelector('[aria-label="Close Obliterate"]').click(),
      () => state.document.querySelector(".ic-obliterate-root").click(),
    ]) {
      await state.panel.open(context);
      const cancel = state.document.activeElement;
      state.key("Tab");
      assert.equal(state.document.activeElement, state.confirm());
      state.key("Tab");
      assert.equal(state.document.activeElement.title, "Close Obliterate");
      state.key("Tab", true);
      assert.equal(state.document.activeElement, state.confirm());
      state.origin.focus();
      assert.equal(state.document.activeElement, cancel);
      state.document.querySelector('[role="alertdialog"]').dispatchEvent(new state.window.Event("click", { bubbles: true }));
      assert.ok(state.confirm());
      dismiss();
      assert.equal(state.confirm(), null);
      assert.equal(state.document.activeElement, state.origin);
      assert.equal(state.key("Tab").defaultPrevented, false);
    }
    assert.deepEqual(state.calls, []);
    assert.equal(state.plans, 4);
  } finally { state.cleanup(); }
});

test("fingerprint detects target, context, readiness and severity changes, with fresh explicit consent", async () => {
  for (const revise of [
    plan => { plan.target.name = "Revised name"; },
    plan => { plan.context.guildId = "999"; },
    plan => { plan.context.guildName = "Revised server"; },
    plan => { plan.actions[0].status = "done"; },
    plan => { plan.actions[5].severity = "kick"; plan.actions[5].label = "Kick member"; },
    plan => { plan.actions[5].severity = "timeout"; },
  ]) {
    let current = makePlan();
    const state = fixture({ plan: () => current });
    try {
      await state.panel.open(context);
      revise(current);
      state.confirm().click();
      await flush();
      assert.deepEqual(state.calls, []);
      assert.match(state.document.body.textContent, /plan changed.*explicitly confirm again/);
      assert.equal(state.confirm().disabled, false);
      state.confirm().click();
      await flush();
      assert.equal(state.calls.length, current.actions.filter(action => action.status === "ready").length);
      assert.equal(state.calls.at(-1)[2].severity, current.actions[5].severity);
    } finally { state.cleanup(); }
  }
});

test("every unavailable action including DM moderation requires unchecked partial consent; revision resets it", async () => {
  let current = makePlan();
  current.context.guildId = current.context.guildName = null;
  Object.assign(current.actions[5], { status: "unavailable", reason: "No server moderation in a direct message." });
  const state = fixture({ plan: () => current });
  const check = () => { const checkbox = state.document.querySelector("input"); checkbox.checked = true; checkbox.dispatchEvent(new state.window.Event("change")); };
  try {
    await state.panel.open(context);
    assert.match(state.document.body.textContent, /No verified server context/);
    assert.match(state.document.body.textContent, /No server moderation in a direct message/);
    assert.match(state.document.querySelector("label").textContent, /^Proceed with available actions only$/);
    assert.equal(state.document.querySelector("input").checked, false);
    assert.equal(state.confirm().disabled, true);
    state.confirm().click();
    await flush();
    assert.deepEqual(state.calls, []);
    check();
    assert.equal(state.confirm().disabled, false);
    current.actions[0].status = "unavailable";
    current.actions[0].reason = "Mute permission changed.";
    state.confirm().click();
    await flush();
    assert.deepEqual(state.calls, []);
    assert.equal(state.document.querySelector("input").checked, false);
    assert.equal(state.confirm().disabled, true);
    check();
    state.confirm().click();
    await flush();
    assert.deepEqual(state.calls.map(call => call[1]), ["soundboard", "video", "ignore", "block"]);
  } finally { state.cleanup(); }
});

test("missing actions become unavailable rows; empty, all done and all unavailable plans cannot execute", async () => {
  for (const mode of ["empty", "done", "unavailable", "missing"]) {
    const plan = makePlan();
    if (mode === "empty") plan.actions = [];
    else if (mode === "missing") plan.actions.pop();
    else for (const action of plan.actions) { action.status = mode; delete action.reason; }
    const state = fixture({ plan: () => plan });
    try {
      await state.panel.open(context);
      assert.equal(state.document.querySelectorAll("li").length, 6);
      assert.equal(state.confirm().disabled, true);
      if (mode === "missing" || mode === "empty") assert.match(state.document.body.textContent, /Not provided by the adapter/);
      if (mode === "unavailable") assert.match(state.document.body.textContent, /No availability reason was provided/);
      if (mode !== "missing") {
        const checkbox = state.document.querySelector("input");
        if (checkbox) { checkbox.checked = true; checkbox.dispatchEvent(new state.window.Event("change")); }
        assert.equal(state.confirm().disabled, true);
        assert.match(state.document.body.textContent, /No actions are ready/);
      }
      state.confirm().click();
      await flush();
      assert.deepEqual(state.calls, []);
    } finally { state.cleanup(); }
  }
});

test("results distinguish done, submitted, unverified and failures, continue in order and never retry", async () => {
  const state = fixture();
  try {
    state.adapter.executeAction = async (...args) => {
      state.calls.push(args);
      if (args[1] === "ignore") throw new Error("Relationship changed; skipped by adapter");
      if (args[1] === "block") return { status: "unexpected" };
      return { status: { mute: "done", soundboard: "submitted", video: "unverified", moderation: "submitted" }[args[1]], detail: "<img> exact detail" };
    };
    await state.panel.open(context);
    state.confirm().click();
    await flush();
    assert.deepEqual(state.calls.map(call => call[1]), keys);
    const text = key => state.document.querySelector(`[data-result="${key}"]`).textContent;
    assert.match(text("mute"), /^done:/);
    assert.match(text("soundboard"), /^submitted:/);
    assert.match(text("video"), /^unverified:/);
    assert.match(text("ignore"), /^failed: Relationship changed; skipped by adapter.*Not retried/);
    assert.match(text("block"), /^failed: Invalid action result/);
    assert.equal(state.document.querySelector("img"), null);
    assert.match(state.document.body.textContent, /Finished with failures; results may be partial/);
    assert.match(state.document.body.textContent, /Submitted means requested, not verified complete/);
    assert.equal(state.confirm().disabled, true);
    state.confirm().click();
    await flush();
    assert.equal(state.calls.length, 6);
  } finally { state.cleanup(); }
});

test("disposal during an action suppresses detached UI updates and stops subsequent scheduling, including rejection", async () => {
  for (const reject of [false, true]) {
    const pending = deferred();
    const state = fixture();
    try {
      state.adapter.executeAction = (...args) => { state.calls.push(args); return pending.promise; };
      await state.panel.open(context);
      state.confirm().click();
      await flush();
      const row = state.document.querySelector('[data-result="mute"]');
      assert.equal(row.textContent, "Running...");
      state.panel.dispose();
      state.panel.dispose();
      assert.equal(state.document.querySelector("style"), null);
      assert.equal(state.document.activeElement, state.origin);
      if (reject) pending.reject(new Error("Too late"));
      else pending.resolve({ status: "done" });
      await flush();
      assert.equal(row.textContent, "Running...");
      assert.equal(state.calls.length, 1);
      assert.equal(state.key("Tab").defaultPrevented, false);
      await state.panel.open(context);
      assert.equal(state.confirm(), null);
    } finally { state.cleanup(); }
  }
});

test("pending initial plans can be cancelled or superseded without stale updates or writes", async () => {
  const pending = deferred();
  const state = fixture({ plan: () => pending.promise });
  try {
    const opening = state.panel.open(context);
    assert.equal(state.document.activeElement.textContent, "Cancel");
    state.key("Escape");
    state.adapter.plan = () => makePlan();
    await state.panel.open(context);
    pending.resolve({ ...makePlan(), target: { id: context.userId, name: "Stale user" } });
    await opening;
    assert.doesNotMatch(state.document.body.textContent, /Stale user/);
    assert.equal(state.document.querySelectorAll('[role="alertdialog"]').length, 1);
    assert.deepEqual(state.calls, []);
    assert.equal(state.document.activeElement.textContent, "Cancel");
  } finally { state.cleanup(); }
});

test("disposal while replanning prevents all execution and releases the shared lock after settlement", async () => {
  const state = fixture();
  const pending = deferred();
  try {
    await state.panel.open(context);
    state.adapter.plan = () => pending.promise;
    state.confirm().click();
    const detached = state.document.querySelector('[role="alertdialog"]');
    const text = detached.textContent;
    state.panel.dispose();
    pending.resolve(makePlan());
    await flush();
    assert.equal(detached.textContent, text);
    assert.deepEqual(state.calls, []);
    const next = mountObliteratePanel({ plan: makePlan, executeAction: async () => ({ status: "done" }) });
    try { await next.open(context); assert.ok(state.confirm()); }
    finally { next.dispose(); }
  } finally { state.cleanup(); }
});

test("shared module lock prevents concurrent execution across mounted panels", async () => {
  const state = fixture();
  const pending = deferred();
  const otherCalls = [];
  const other = mountObliteratePanel({ plan: makePlan, executeAction: async (...args) => { otherCalls.push(args); return { status: "done" }; } });
  try {
    await state.panel.open(context);
    const firstConfirm = state.confirm();
    state.adapter.executeAction = (...args) => { state.calls.push(args); return pending.promise; };
    await other.open(context);
    const confirms = state.document.querySelectorAll("[data-confirm]");
    firstConfirm.click();
    await flush();
    confirms[1].click();
    await flush();
    assert.deepEqual(otherCalls, []);
    assert.match(state.document.body.textContent, /Another Obliterate operation/);
    state.panel.dispose();
    pending.resolve({ status: "done" });
    await flush();
    confirms[1].click();
    await flush();
    assert.equal(otherCalls.length, 6);
  } finally { other.dispose(); state.cleanup(); }
});

test("plan failures and mismatched identity fail closed without executing", async () => {
  for (const invalid of [
    () => { throw new Error("Store unavailable"); },
    () => ({ ...makePlan(), target: { id: "999", name: "Wrong target" } }),
    () => ({ ...makePlan(), context: { channelId: "999" } }),
    () => { const plan = makePlan(); delete plan.actions[5].severity; return plan; },
    () => { const plan = makePlan(); plan.actions.push(plan.actions[0]); return plan; },
  ]) {
    const state = fixture({ plan: invalid });
    try {
      await state.panel.open(context);
      assert.equal(state.confirm().disabled, true);
      assert.match(state.document.body.textContent, /Unable to load the plan/);
      assert.deepEqual(state.calls, []);
    } finally { state.cleanup(); }
  }
  const state = fixture();
  try {
    await state.panel.open(context);
    state.adapter.plan = async () => { throw new Error("Recheck failed"); };
    state.confirm().click();
    await flush();
    assert.equal(state.confirm().disabled, true);
    assert.match(state.document.body.textContent, /Unable to verify the plan: Recheck failed/);
    assert.deepEqual(state.calls, []);
  } finally { state.cleanup(); }
});

test("invalid IDs reject without replacing an existing plan", async () => {
  const state = fixture();
  try {
    await state.panel.open(context);
    const dialog = state.document.querySelector('[role="alertdialog"]');
    for (const invalid of [null, {}, { userId: 123, channelId: "200" }, { userId: "0", channelId: "200" }, { ...context, channelId: "bad" }]) {
      await assert.rejects(state.panel.open(invalid), /IDs must be positive numeric strings/);
      assert.equal(state.document.querySelector('[role="alertdialog"]'), dialog);
    }
    assert.deepEqual(state.calls, []);
  } finally { state.cleanup(); }
});