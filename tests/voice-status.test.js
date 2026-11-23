import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { mountVoiceStatus } from "../src/web/voice-status.js";

const first = "9007199254740993";
const second = "9007199254740992";

function fixture({ text = "In voice", name = null, subscribed = true } = {}) {
  const { document, window } = parseHTML("<html><body><span></span></body></html>");
  const node = document.querySelector("span");
  node.textContent = text;
  const names = new Map([[first, name]]);
  const observers = new Set();
  const subscribers = new Set();
  const reads = [];
  let labels = [{ node, userId: first }];
  const adapter = {
    resolve(userId) { reads.push(userId); return names.get(userId) ?? null; },
    labels: () => labels,
    observe(refresh) { observers.add(refresh); return () => observers.delete(refresh); },
  };
  if (subscribed) adapter.subscribe = refresh => {
    subscribers.add(refresh);
    return () => subscribers.delete(refresh);
  };
  const feature = mountVoiceStatus(adapter);
  return { document, window, node, names, observers, subscribers, reads, adapter, feature,
    labels(value) { labels = value; },
    emit() { for (const refresh of observers) refresh(); },
    notify() { for (const refresh of subscribers) refresh(); },
  };
}

test("unknown calls remain unchanged and only exact native text is eligible", () => {
  for (const text of ["In voice", " In voice", "In voice ", "In Voice", "In voice with friends", "Playing", ""]) {
    const context = fixture({ text });
    assert.equal(context.node.textContent, text);
    context.names.set(first, "Lounge");
    context.emit();
    assert.equal(context.node.textContent, text === "In voice" ? "In Lounge" : text);
    assert.deepEqual(context.reads, text === "In voice" ? [first, first] : []);
    context.feature.dispose();
  }
});

test("resolved calls rename on subscription and preserve exact string user IDs", () => {
  const context = fixture({ name: "Lounge" });
  assert.equal(context.node.textContent, "In Lounge");
  context.names.set(first, "Study room");
  context.notify();
  assert.equal(context.node.textContent, "In Study room");
  assert.deepEqual(context.reads, [first, first]);
  context.feature.dispose();
});

test("resolution loss restores the original and later resolution can replace it again", () => {
  const context = fixture({ name: "Lounge" });
  context.names.delete(first);
  context.notify();
  assert.equal(context.node.textContent, "In voice");
  context.names.set(first, "Study");
  context.feature.refresh();
  assert.equal(context.node.textContent, "In Study");
  context.feature.dispose();
  assert.equal(context.node.textContent, "In voice");
});

test("blank, non-string and overlong names never replace native labels", () => {
  const context = fixture();
  for (const name of [null, undefined, "", " \t\n", 42, {}, "x".repeat(201)]) {
    context.names.set(first, "Valid");
    context.emit();
    assert.equal(context.node.textContent, "In Valid");
    context.names.set(first, name);
    context.emit();
    assert.equal(context.node.textContent, "In voice");
  }
  context.names.set(first, "x".repeat(200));
  context.emit();
  assert.equal(context.node.textContent, `In ${"x".repeat(200)}`);
  context.names.set(first, "<img src=remote>");
  context.emit();
  assert.equal(context.node.textContent, "In <img src=remote>");
  assert.equal(context.node.children.length, 0);
  context.feature.dispose();
});

test("reused nodes cannot retain another user's resolved name", () => {
  const context = fixture({ name: "First room" });
  context.labels([{ node: context.node, userId: second }]);
  context.emit();
  assert.equal(context.node.textContent, "In voice");
  context.names.set(second, "Second room");
  context.emit();
  assert.equal(context.node.textContent, "In Second room");
  context.labels([{ node: context.node, userId: first }]);
  context.emit();
  assert.equal(context.node.textContent, "In First room");
  assert.deepEqual(context.reads, [first, second, second, first]);
  context.feature.dispose();
});

test("React text changes relinquish ownership and native rerenders are decorated again", () => {
  const context = fixture({ name: "Lounge" });
  context.node.textContent = "Playing chess";
  context.emit();
  assert.equal(context.node.textContent, "Playing chess");
  context.node.textContent = "In voice";
  context.emit();
  assert.equal(context.node.textContent, "In Lounge");
  context.node.textContent = "Offline";
  context.feature.dispose();
  assert.equal(context.node.textContent, "Offline");
});

test("reused nodes with React-owned text are not overwritten or restored", () => {
  const context = fixture({ name: "Lounge" });
  context.node.textContent = "Online";
  context.names.set(second, "Other room");
  context.labels([{ node: context.node, userId: second }]);
  context.emit();
  assert.equal(context.node.textContent, "Online");
  context.feature.dispose();
  assert.equal(context.node.textContent, "Online");
});

test("labels no longer supplied by the adapter are restored and forgotten", () => {
  const context = fixture({ name: "Lounge" });
  context.labels([]);
  context.node.remove();
  context.emit();
  assert.equal(context.node.textContent, "In voice");
  context.node.textContent = "New owner";
  context.feature.dispose();
  assert.equal(context.node.textContent, "New owner");
});

test("disposal restores all owned labels, cleans observers and subscriptions, and is idempotent", () => {
  const context = fixture({ name: "Lounge" });
  const other = context.document.createElement("span");
  other.textContent = "In voice";
  context.names.set(second, "Study");
  context.labels([{ node: context.node, userId: first }, { node: other, userId: second }]);
  context.emit();
  const savedObserver = [...context.observers][0];
  const savedSubscriber = [...context.subscribers][0];
  context.feature.dispose();
  context.feature.dispose();
  assert.equal(context.node.textContent, "In voice");
  assert.equal(other.textContent, "In voice");
  assert.equal(context.observers.size, 0);
  assert.equal(context.subscribers.size, 0);
  const readCount = context.reads.length;
  savedObserver();
  savedSubscriber();
  context.feature.refresh();
  assert.equal(context.reads.length, readCount);
  assert.equal(context.node.textContent, "In voice");
});

test("subscription is optional and resolver failures restore rather than guess", () => {
  const context = fixture({ name: "Lounge", subscribed: false });
  context.adapter.resolve = () => { throw new Error("Unavailable"); };
  context.emit();
  assert.equal(context.node.textContent, "In voice");
  context.feature.dispose();
  assert.equal(context.observers.size, 0);
});

test("own mutations settle without observer loops or redundant text writes", async () => {
  const context = fixture();
  let notifications = 0;
  const observer = new context.window.MutationObserver(() => {
    notifications++;
    assert.ok(notifications < 5, "mutation feedback must settle");
    context.emit();
  });
  observer.observe(context.node, { childList: true, characterData: true, subtree: true });
  try {
    context.names.set(first, "Lounge");
    context.emit();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(context.node.textContent, "In Lounge");
    assert.equal(notifications, 1);
    context.emit();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(notifications, 1);
    context.names.delete(first);
    context.emit();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(context.node.textContent, "In voice");
    assert.equal(notifications, 2);
  } finally {
    observer.disconnect();
    context.feature.dispose();
  }
});

test("synchronous observer feedback cannot recurse during text replacement", () => {
  let text = "In voice";
  let callback;
  let writes = 0;
  const node = {
    get textContent() { return text; },
    set textContent(value) { text = value; writes++; callback(); },
  };
  const feature = mountVoiceStatus({
    labels: () => [{ node, userId: first }],
    resolve: () => "Lounge",
    observe(refresh) { callback = refresh; return () => {}; },
  });
  assert.equal(text, "In Lounge");
  assert.equal(writes, 1);
  feature.refresh();
  assert.equal(writes, 1);
  feature.dispose();
  assert.equal(text, "In voice");
  assert.equal(writes, 2);
});

test("failed subscription setup rolls back initial observer-driven replacements", () => {
  const context = fixture();
  context.feature.dispose();
  context.names.set(first, "Lounge");
  assert.throws(() => mountVoiceStatus({
    ...context.adapter,
    observe(refresh) { context.observers.add(refresh); refresh(); return () => context.observers.delete(refresh); },
    subscribe() { throw new Error("Subscription unavailable"); },
  }), /Subscription unavailable/);
  assert.equal(context.node.textContent, "In voice");
  assert.equal(context.observers.size, 0);
});