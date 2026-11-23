import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createVoiceWatch } from "../src/web/watch.js";
import { mountVoiceWatch } from "../src/web/watch-panel.js";

const first = "9007199254740993";
const second = "9007199254740992";

function fixture() {
  const { window, document } = parseHTML('<html><body><button id="origin">Origin</button></body></html>');
  const originalDocument = globalThis.document;
  globalThis.document = document;
  let focused = document.querySelector("#origin");
  Object.defineProperty(document, "activeElement", { get: () => focused });
  window.HTMLElement.prototype.focus = function () {
    focused = this;
    this.dispatchEvent(new window.Event("focusin", { bubbles: true }));
  };
  let states = [];
  const listeners = new Set();
  const errors = [];
  const store = { addChangeListener: callback => listeners.add(callback), removeChangeListener: callback => listeners.delete(callback) };
  const user = { id: first, globalName: "<script>Morgan</script>", username: "morgan" };
  for (const field of ["token", "email", "phone", "messages"]) {
    Object.defineProperty(user, field, { get() { assert.fail(`Read private field ${field}`); } });
  }
  const stores = {
    voice: { ...store, getVoiceStatesForChannel: () => states },
    selected: { ...store, getVoiceChannelId: () => "200" },
    users: { getUser: id => id === first ? user : { id: "wrong", globalName: "Wrong name" } },
    onError: error => errors.push(error),
  };
  const watch = createVoiceWatch(stores);
  let subscriptions = 0;
  const originalSubscribe = watch.subscribe;
  watch.subscribe = callback => {
    subscriptions++;
    const unsubscribe = originalSubscribe(callback);
    let active = true;
    return () => { if (active) { active = false; subscriptions--; unsubscribe(); } };
  };
  const panel = mountVoiceWatch(watch, stores);
  return {
    document, window, watch, panel, errors, listeners,
    get subscriptions() { return subscriptions; },
    emit(ids) { states = ids.map(userId => ({ userId, channelId: "200" })); for (const callback of listeners) callback(); },
    key(key, shiftKey = false) {
      const event = new window.Event("keydown", { bubbles: true, cancelable: true });
      Object.assign(event, { key, shiftKey });
      document.dispatchEvent(event);
      return event;
    },
    cleanup() { panel.dispose(); watch.dispose(); globalThis.document = originalDocument; },
  };
}

test("selected user modal uses shared theme, safe local names, checkbox and live removable watch list", () => {
  const state = fixture();
  const { panel, document, window, watch } = state;
  try {
    panel.open(first);
    const dialog = document.querySelector('[role="dialog"]');
    assert.equal(dialog.getAttribute("aria-modal"), "true");
    assert.equal(dialog.getAttribute("aria-label"), "Voice Watch");
    assert.equal(dialog.hasAttribute("data-intellicord-owned"), true);
    assert.equal(dialog.parentElement.hasAttribute("data-intellicord-owned"), true);
    assert.match(document.querySelector("style").textContent, /\.theme-light \.ic-user-tool/);
    assert.match(document.querySelector("style").textContent, /max-height:calc\(100dvh - 32px\);overflow:auto/);
    assert.match(document.querySelector("style").textContent, /max-width:100%/);
    assert.match(dialog.textContent, /<script>Morgan<\/script>/);
    assert.match(dialog.textContent, new RegExp(first));
    assert.equal(dialog.querySelector("script,img,audio,video"), null);
    const checkbox = dialog.querySelector('input[type="checkbox"]');
    assert.equal(Boolean(checkbox.checked), false);
    checkbox.checked = true;
    checkbox.dispatchEvent(new window.Event("change"));
    assert.equal(watch.isWatched(first), true);
    assert.equal(panel.isWatched(first), true);
    assert.equal(document.querySelector('[aria-label="Watched users"]').children.length, 1);
    watch.setWatched(second, true);
    assert.equal(document.querySelector('[aria-label="Watched users"]').children.length, 2);
    assert.doesNotMatch(document.querySelector('[aria-label="Watched users"]').textContent, /Wrong name/);
    const remove = document.querySelector(`[data-remove-watch="${first}"]`);
    assert.equal(remove.title, "Stop watching <script>Morgan</script>");
    assert.equal(remove.querySelector("svg").getAttribute("width"), "18");
    remove.focus();
    remove.click();
    assert.equal(watch.isWatched(first), false);
    assert.equal(checkbox.checked, false);
    assert.equal(document.activeElement, checkbox);
    assert.equal(state.subscriptions, 1);
  } finally { state.cleanup(); }
});

test("management without selected user shows newest bounded events and can remove all watches", () => {
  const state = fixture();
  try {
    state.watch.setWatched(first, true);
    for (let index = 0; index < 106; index++) state.emit(index % 2 ? [] : [first]);
    state.panel.open();
    assert.equal(state.document.querySelector("input"), null);
    const events = state.document.querySelector('[aria-label="Recent voice events"]');
    assert.equal(events.children.length, 100);
    assert.match(events.firstElementChild.textContent, /left/);
    assert.match(events.firstElementChild.textContent, /Channel 200/);
    assert.match(events.firstElementChild.querySelector("time").dateTime, /^\d{4}-/);
    state.emit([first]);
    assert.equal(events.children.length, 100);
    assert.match(events.firstElementChild.textContent, /joined/);
    state.document.querySelector("[data-remove-watch]").click();
    assert.deepEqual(state.watch.listWatched(), []);
    assert.match(state.document.querySelector('[aria-label="Watched users"]').textContent, /No watched users/);
    assert.equal(state.document.activeElement.getAttribute("aria-label"), "Close Voice Watch");
  } finally { state.cleanup(); }
});

test("keyboard trap, outside close, repeated open and disposal restore focus and release subscriptions", () => {
  const state = fixture();
  const { panel, document, window } = state;
  const origin = document.activeElement;
  try {
    panel.open(first);
    const close = document.activeElement;
    const checkbox = document.querySelector("input");
    assert.equal(close.title, "Close Voice Watch");
    assert.equal(state.key("Tab").defaultPrevented, true);
    assert.equal(document.activeElement, checkbox);
    state.key("Tab");
    assert.equal(document.activeElement, close);
    state.key("Tab", true);
    assert.equal(document.activeElement, checkbox);
    origin.focus();
    assert.equal(document.activeElement, close);
    assert.equal(state.key("Escape").defaultPrevented, true);
    assert.equal(document.querySelector('[role="dialog"]'), null);
    assert.equal(document.activeElement, origin);
    assert.equal(state.subscriptions, 0);
    assert.equal(state.key("Tab").defaultPrevented, false);
    panel.open();
    panel.open(second);
    assert.equal(document.querySelectorAll('[role="dialog"]').length, 1);
    assert.equal(state.subscriptions, 1);
    document.querySelector('[role="dialog"]').dispatchEvent(new window.Event("click", { bubbles: true }));
    assert.ok(document.querySelector('[role="dialog"]'));
    document.querySelector(".ic-watch-root").click();
    assert.equal(document.querySelector('[role="dialog"]'), null);
    assert.equal(document.activeElement, origin);
    panel.open();
    document.querySelector('[aria-label="Close Voice Watch"]').click();
    assert.equal(state.subscriptions, 0);
    panel.open();
    panel.dispose();
    panel.dispose();
    assert.equal(state.subscriptions, 0);
    assert.equal(document.querySelector("style"), null);
    assert.equal(document.activeElement, origin);
    origin.focus();
    assert.equal(document.activeElement, origin);
    panel.open(first);
    assert.equal(document.querySelector('[role="dialog"]'), null);
    state.watch.setWatched(first, true);
    assert.equal(state.watch.isWatched(first), true);
  } finally { state.cleanup(); }
});

test("capacity rejection resets checkbox, invalid IDs do not close modal, and delegates remain usable", () => {
  const state = fixture();
  try {
    for (let index = 1; index <= 100; index++) state.watch.setWatched(String(index), true);
    let notifications = 0;
    const unsubscribe = state.panel.subscribe(() => notifications++);
    state.panel.open(first);
    const checkbox = state.document.querySelector("input");
    checkbox.checked = true;
    checkbox.dispatchEvent(new state.window.Event("change"));
    assert.equal(checkbox.checked, false);
    assert.match(state.document.querySelector('[role="status"]').textContent, /limit reached/);
    assert.match(state.errors.at(-1).message, /limit/);
    assert.equal(state.watch.listWatched().length, 100);
    for (const id of [0, "0", "18446744073709551616", "x"]) assert.throws(() => state.panel.open(id), /Invalid/);
    assert.equal(state.document.querySelectorAll('[role="dialog"]').length, 1);
    state.watch.setWatched("1", false);
    assert.equal(notifications, 1);
    unsubscribe();
    state.watch.setWatched("2", false);
    assert.equal(notifications, 1);
  } finally { state.cleanup(); }
});

test("subscription failure leaves no modal or keyboard trap and restores origin", () => {
  const state = fixture();
  const origin = state.document.activeElement;
  try {
    state.watch.subscribe = () => { throw new Error("listener limit"); };
    assert.throws(() => state.panel.open(first), /listener limit/);
    assert.equal(state.document.querySelector('[role="dialog"]'), null);
    assert.equal(state.document.activeElement, origin);
    assert.equal(state.key("Tab").defaultPrevented, false);
  } finally { state.cleanup(); }
});