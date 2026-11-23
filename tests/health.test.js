import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { build } from "esbuild";

const bundle = await build({ entryPoints: ["src/web/health.js"], bundle: true, write: false, format: "esm" });
const { mountSessionHealth } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);

function fixture() {
  const { window, document } = parseHTML('<html><body><div id="settings"><nav role="tablist"><a role="tab" data-settings-sidebar-item="account" aria-selected="true" tabindex="0">Account</a><a role="tab" data-settings-sidebar-item="privacy" aria-selected="false">Privacy</a></nav><main id="native">Native settings</main></div></body></html>');
  globalThis.window = window;
  globalThis.document = document;
  const sidebar = document.querySelector("nav");
  sidebar.getBoundingClientRect = () => ({ right: 280, top: 20, bottom: 780 });
  window.innerHeight = 800;
  window.innerWidth = 1280;
  let timer;
  let cleared = 0;
  window.setInterval = (callback, delay) => { assert.equal(delay, 2000); timer = callback; return 42; };
  window.clearInterval = id => { assert.equal(id, 42); cleared += 1; };
  let observed;
  let stopped = 0;
  const adapter = { sidebar: () => sidebar, observe: callback => { observed = callback; return () => { stopped += 1; }; } };
  const snapshot = { startedAt: Date.now() - 20000, lastHeartbeat: Date.now(), lastSnapshot: Date.now() - 2000,
    build: "673869", features: { voice: { state: "ready", detail: "Available" } } };
  const health = mountSessionHealth(adapter, () => snapshot);
  const entry = document.querySelector(".ic-health-tab");
  const panel = document.querySelector(".ic-health-panel");
  return { window, document, sidebar, snapshot, health, entry, panel, observed: () => observed(), tick: () => timer(), stopped: () => stopped, cleared: () => cleared };
}

function key(window, target, value) {
  const event = new window.Event("keydown", { bubbles: true, cancelable: true });
  event.key = value;
  target.dispatchEvent(event);
  return event;
}

test("adapter mounts a tab, shows scoped content, and restores native selection on Escape", () => {
  const context = fixture();
  const { health, entry, panel, sidebar, document, window } = context;
  const native = document.querySelector("#native");
  const account = sidebar.firstElementChild;
  try {
    assert.equal(entry.parentElement, sidebar);
    assert.equal(entry.getAttribute("role"), "tab");
    assert.equal(panel.hidden, true);
    assert.equal(entry.getAttribute("aria-controls"), panel.id);
    entry.click();
    assert.equal(panel.hidden, false);
    assert.equal(entry.getAttribute("aria-selected"), "true");
    assert.equal(account.getAttribute("aria-selected"), "false");
    assert.equal(panel.style.left, "280px");
    assert.equal(panel.style.top, "0px");
    assert.equal(panel.style.bottom, "0px");
    assert.equal(native.textContent, "Native settings");
    assert.equal(native.hasAttribute("hidden"), false);
    assert.equal(native.hasAttribute("aria-hidden"), false);
    assert.equal(document.querySelector("tbody td").textContent, "voice");
    context.tick();
    assert.equal(sidebar.querySelectorAll(".ic-health-tab").length, 1);
    const escape = key(window, entry, "Escape");
    assert.equal(escape.defaultPrevented, false);
    assert.equal(panel.hidden, true);
    assert.equal(account.getAttribute("aria-selected"), "true");
    assert.equal(account.getAttribute("tabindex"), "0");
    assert.equal(entry.getAttribute("aria-selected"), "false");
  } finally { health.dispose(); }
});

test("narrow settings use viewport-constrained diagnostics", () => {
  const {health,entry,panel,window,sidebar} = fixture();
  try {
    window.innerWidth = 390;
    window.innerHeight = 700;
    sidebar.getBoundingClientRect = () => ({right:420,top:20,bottom:780});
    entry.click();
    health.refresh();
    assert.equal(panel.style.left,"0px");
    assert.equal(panel.style.right,"0px");
    assert.equal(panel.style.top,"460px");
    assert.equal(panel.style.bottom,"0px");
  } finally {health.dispose();}
});

test("native navigation is delivered unchanged and its new selection survives closure", () => {
  const { health, entry, panel, sidebar, window } = fixture();
  const account = sidebar.firstElementChild;
  const privacy = account.nextElementSibling;
  let delivered = 0;
  sidebar.addEventListener("click", event => {
    if (event.target !== privacy) return;
    assert.equal(event.defaultPrevented, false);
    assert.equal(panel.hidden, true);
    account.setAttribute("aria-selected", "false");
    privacy.setAttribute("aria-selected", "true");
    delivered += 1;
  });
  try {
    entry.click();
    privacy.dispatchEvent(new window.Event("click", { bubbles: true, cancelable: true }));
    assert.equal(delivered, 1);
    assert.equal(privacy.getAttribute("aria-selected"), "true");
    health.refresh();
    assert.equal(account.getAttribute("aria-selected"), "false");
  } finally { health.dispose(); }
});

test("keyboard activation and navigation work without consuming native keyboard events", () => {
  const { health, entry, panel, sidebar, window } = fixture();
  let focused = null;
  sidebar.firstElementChild.focus = () => { focused = "first"; };
  sidebar.children[1].focus = () => { focused = "last"; };
  try {
    assert.equal(key(window, entry, "Enter").defaultPrevented, true);
    assert.equal(panel.hidden, false);
    key(window, entry, "ArrowDown");
    assert.equal(focused, "first");
    assert.equal(panel.hidden, true);
    key(window, entry, " ");
    key(window, entry, "End");
    assert.equal(focused, "last");
    assert.equal(key(window, sidebar.firstElementChild, "Enter").defaultPrevented, false);
  } finally { health.dispose(); }
});

test("heartbeat expires and feature last update tracks changes rather than polling", () => {
  const originalNow = Date.now;
  let now = 100000;
  Date.now = () => now;
  const context = fixture();
  try {
    context.entry.click();
    assert.match(context.panel.textContent, /Connected · 0s ago/);
    assert.match(context.panel.textContent, /Attachment uptime20s/);
    assert.match(context.panel.textContent, /Current build673869/);
    now += 8000;
    context.tick();
    assert.match(context.panel.textContent, /Disconnected · 8s ago/);
    assert.equal(context.document.querySelector("tbody tr").lastElementChild.textContent, "8s ago");
    context.snapshot.features.voice = { state: "waiting", detail: "Reconnecting" };
    context.snapshot.lastHeartbeat = now;
    context.tick();
    assert.equal(context.document.querySelector("tbody tr").lastElementChild.textContent, "0s ago");
    assert.match(context.panel.textContent, /Connected · 0s ago/);
    context.snapshot.lastHeartbeat = null;
    context.snapshot.lastSnapshot = NaN;
    context.tick();
    assert.match(context.panel.textContent, /Disconnected · Never/);
    assert.match(context.panel.textContent, /Last successful snapshotNever/);
  } finally { context.health.dispose(); Date.now = originalNow; }
});

test("copy exports only numeric timing and normalized states, with selectable fallback", async () => {
  const { health, entry, document, snapshot, window } = fixture();
  snapshot.features = { "Morgan 9007199254740993": { state: "ready", detail: "user Morgan token secret", userId: "9007199254740993" },
    unsafe: { state: "secret", detail: "<img src=x>" } };
  snapshot.watchEvents = [{ username: "Morgan", userId: "9007199254740993" }];
  snapshot.preferences = { token: "secret" };
  let copied;
  const clipboard = { writeText: async text => { copied = text; } };
  globalThis.window = new Proxy(window, { get: (target, property) => property === "navigator" ? { clipboard } : target[property] });
  try {
    entry.click();
    assert.equal(document.querySelector("tbody img"), null);
    document.querySelector('[aria-label="Copy diagnostics"]').click();
    await Promise.resolve();
    const exported = JSON.parse(copied);
    assert.deepEqual(Object.keys(exported), ["startedAt", "lastHeartbeat", "lastSnapshot", "features"]);
    assert.deepEqual(exported.features.map(feature => feature.state), ["ready", "unsupported"]);
    assert.ok(exported.features.every(feature => Number.isFinite(feature.lastUpdate)));
    assert.doesNotMatch(copied, /Morgan|9007199254740993|secret|watchEvents|preferences|673869|detail/);
    clipboard.writeText = async () => { throw new Error("Denied"); };
    const fallback = document.querySelector("textarea");
    let selected = false;
    fallback.select = () => { selected = true; };
    document.querySelector('[aria-label="Copy diagnostics"]').click();
    await Promise.resolve();
    assert.equal(fallback.hidden, false);
    assert.equal(fallback.readOnly, true);
    assert.equal(selected, true);
    assert.deepEqual(JSON.parse(fallback.value), exported);
  } finally { health.dispose(); }
});

test("adapter observer handles settings disappearance and disposal cleans resources and selection", () => {
  const context = fixture();
  const { health, entry, panel, sidebar, document } = context;
  entry.click();
  document.querySelector("#settings").hidden = true;
  context.observed();
  assert.equal(panel.hidden, true);
  assert.equal(entry.isConnected, false);
  assert.equal(sidebar.firstElementChild.getAttribute("aria-selected"), "true");
  document.querySelector("#settings").hidden = false;
  context.observed();
  entry.click();
  health.dispose();
  health.dispose();
  assert.equal(context.stopped(), 1);
  assert.equal(context.cleared(), 1);
  assert.equal(sidebar.firstElementChild.getAttribute("aria-selected"), "true");
  assert.equal(document.querySelector(".ic-health-panel"), null);
  assert.equal(document.querySelector(".ic-health-tab"), null);
  assert.equal(document.querySelector("style"), null);
  health.refresh();
  context.observed();
  context.tick();
  assert.equal(document.querySelector(".ic-health-tab"), null);
});

test("default MutationObserver mounts when settings appear and prefers the nearest tablist", async () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  window.setInterval = () => 42;
  window.clearInterval = () => {};
  let snapshots = 0;
  const health = mountSessionHealth(() => { snapshots += 1; return {}; });
  try {
    assert.equal(document.querySelector(".ic-health-tab"), null);
    const sidebar = document.createElement("nav");
    sidebar.setAttribute("role", "tablist");
    sidebar.innerHTML = '<div><a data-settings-sidebar-item="account">Account</a></div>';
    document.body.append(sidebar);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(document.querySelector(".ic-health-tab").parentElement, sidebar);
    document.querySelector(".ic-health-tab").click();
    const before = snapshots;
    await new Promise(resolve => setImmediate(resolve));
    assert.ok(snapshots <= before + 1);
    sidebar.remove();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(document.querySelector(".ic-health-tab"), null);
  } finally { health.dispose(); }
});

test("adapter owns sidebar discovery and getter errors stay inside diagnostics", () => {
  const context = fixture();
  context.health.dispose();
  const { document, sidebar } = context;
  for (const anchor of sidebar.querySelectorAll("a")) anchor.removeAttribute("data-settings-sidebar-item");
  let currentSidebar = sidebar;
  let fail = false;
  let observeRefresh;
  const health = mountSessionHealth({ sidebar: () => currentSidebar, observe: callback => { observeRefresh = callback; return () => {}; } }, () => {
    if (fail) throw new Error("private native error user 123");
    return context.snapshot;
  });
  try {
    document.querySelector(".ic-health-tab").click();
    fail = true;
    health.refresh();
    assert.match(document.querySelector(".ic-health-panel").textContent, /Disconnected/);
    assert.match(document.querySelector(".ic-health-panel").textContent, /Snapshot unavailable/);
    assert.doesNotMatch(document.querySelector(".ic-health-panel").textContent, /private native error/);
    assert.equal(document.querySelector("#native").textContent, "Native settings");
    const replacement = document.createElement("nav");
    document.querySelector("#settings").append(replacement);
    currentSidebar = replacement;
    observeRefresh();
    assert.equal(document.querySelector(".ic-health-tab").parentElement, replacement);
    assert.equal(document.querySelector(".ic-health-panel").hidden, true);
    assert.equal(sidebar.firstElementChild.getAttribute("aria-selected"), "true");
  } finally { health.dispose(); }
});