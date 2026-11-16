import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { observeCallView } from "../src/web/bridge.js";

test("call launcher follows element resizing and replacement, hides in settings, and cleans up", () => {
  const { window, document } = parseHTML('<html><body><div class="callContainer_fixture"></div></body></html>');
  globalThis.window = window;
  globalThis.document = document;
  let mutation;
  let resize;
  let observed;
  let mutationStopped = false;
  let resizeStopped = false;
  const frames = [];
  globalThis.requestAnimationFrame = callback => { frames.push(callback); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  globalThis.MutationObserver = class {
    constructor(callback) { mutation = callback; }
    observe() {}
    disconnect() { mutationStopped = true; }
  };
  globalThis.ResizeObserver = class {
    constructor(callback) { resize = callback; }
    observe(node) { observed = node; resizeStopped = false; }
    disconnect() { observed = null; resizeStopped = true; }
  };
  let rect = { left: 312, top: 0, width: 968, height: 800, bottom: 800 };
  function makeVisible(node) {
    node.getClientRects = () => [rect];
    node.getBoundingClientRect = () => rect;
  }
  const call = document.querySelector('.callContainer_fixture');
  makeVisible(call);
  const updates = [];
  const stop = observeCallView(value => updates.push(value));
  assert.equal(observed, call);
  assert.equal(updates.at(-1), rect);
  rect = { ...rect, left: 56, width: 584, height: 320, bottom: 320 };
  resize();
  assert.equal(updates.at(-1).bottom, 320);
  const replacement = document.createElement("div");
  replacement.className = "callContainer_replacement";
  makeVisible(replacement);
  call.replaceWith(replacement);
  mutation([{ target: document.body }]);
  frames.shift()();
  assert.equal(observed, replacement);
  const app = document.createElement("div");
  replacement.replaceWith(app);
  app.append(replacement);
  app.setAttribute("aria-hidden", "true");
  mutation([{ target: document.body }]);
  frames.shift()();
  assert.equal(observed, replacement);
  assert.equal(updates.at(-1), rect);
  app.hidden = true;
  mutation([{ target: document.body }]);
  frames.shift()();
  assert.equal(observed, null);
  assert.equal(updates.at(-1), null);
  app.hidden = false;
  app.removeAttribute("aria-hidden");
  mutation([{ target: document.body }]);
  frames.shift()();
  assert.equal(observed, replacement);
  const settings = document.createElement("li");
  settings.dataset.settingsSidebarItem = "data_and_privacy_panel";
  makeVisible(settings);
  document.body.append(settings);
  mutation([{ target: document.body }]);
  frames.shift()();
  assert.equal(updates.at(-1), null);
  assert.equal(observed, null);
  settings.remove();
  mutation([{ target: document.body }]);
  frames.shift()();
  assert.equal(observed, replacement);
  stop();
  assert.equal(mutationStopped, true);
  assert.equal(resizeStopped, true);
  delete globalThis.MutationObserver;
  delete globalThis.ResizeObserver;
  delete globalThis.requestAnimationFrame;
  delete globalThis.cancelAnimationFrame;
});