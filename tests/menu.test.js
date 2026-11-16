import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { build } from "esbuild";

const bundle = await build({ entryPoints: ["src/web/menu.js"], bundle: true, write: false, format: "esm", loader: { ".css": "text" } });
const { mountMenu } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);

test("history rows and user choices open context menus without mute buttons or changing filters", () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const calls = [];
  const user = { id: "9007199254740993", username: "morgan", display_name: "Morgan" };
  const channel = { id: "100", name: "General" };
  const menu = mountMenu(() => {}, { open: (event, ...args) => calls.push({ event, args }) });
  try {
    menu.update({ channel, participants: { [user.id]: user }, events: [{ sequence: 1, user, channel, timestamp: 1000, action: "joined" }] });
    assert.equal(document.querySelector('[data-mute-id]'), null);
    const row = document.querySelector(".cm-event");
    assert.equal(row.getAttribute("aria-haspopup"), "menu");
    const rightClick = new window.Event("contextmenu", { bubbles: true, cancelable: true });
    row.dispatchEvent(rightClick);
    assert.equal(rightClick.defaultPrevented, true);
    assert.deepEqual(calls[0].args, [user.id, channel.id]);
    const keyboard = new window.Event("keydown", { bubbles: true, cancelable: true });
    keyboard.key = "F10";
    keyboard.shiftKey = true;
    row.dispatchEvent(keyboard);
    assert.deepEqual(calls[1].args, [user.id, channel.id]);
    document.querySelector(".cm-user-option").dispatchEvent(new window.Event("contextmenu", { bubbles: true, cancelable: true }));
    assert.deepEqual(calls[2].args, [user.id, undefined]);
    assert.equal(document.querySelector('[data-user-id]').checked, true);
    assert.equal(document.querySelectorAll(".cm-event").length, 1);
  } finally { menu.dispose(); }
});

test("unsupported native context menus fail visibly while leaving history usable", () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const user = { id: "1", username: "morgan", display_name: "Morgan" };
  const channel = { id: "100", name: "General" };
  const menu = mountMenu(() => {});
  try {
    menu.update({ channel, participants: {}, events: [{ sequence: 1, user, channel, timestamp: 1000, action: "left" }] });
    document.querySelector(".cm-event").dispatchEvent(new window.Event("contextmenu", { cancelable: true }));
    assert.match(document.querySelector(".cm-status").textContent, /context menu unavailable/);
    assert.equal(document.querySelectorAll(".cm-event").length, 1);
  } finally { menu.dispose(); }
});

test("native user menu interactions and dismissal keep Callmon open", () => {
  const { window, document } = parseHTML('<html><body><button id="outside">Outside</button></body></html>');
  globalThis.window = window;
  globalThis.document = document;
  const user = { id: "1", username: "morgan", display_name: "Morgan" };
  const channel = { id: "100", name: "General" };
  let nativeMenu;
  const menu = mountMenu(() => {}, { open() {
    nativeMenu = document.createElement("div");
    nativeMenu.setAttribute("role", "menu");
    const action = document.createElement("button");
    action.textContent = "Mute";
    nativeMenu.append(action);
    document.body.append(nativeMenu);
  } });
  try {
    menu.update({ channel, participants: {}, events: [{ sequence: 1, user, channel, timestamp: 1000, action: "joined" }] });
    document.querySelector(".cm-launcher").click();
    const panel = document.querySelector(".cm-panel");
    document.querySelector(".cm-event").dispatchEvent(new window.Event("contextmenu", { bubbles: true, cancelable: true }));
    assert.equal(panel.hidden, false);
    nativeMenu.firstElementChild.dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    assert.equal(panel.hidden, false);
    const escape = new window.Event("keydown", { bubbles: true });
    escape.key = "Escape";
    nativeMenu.dispatchEvent(escape);
    assert.equal(panel.hidden, false);
    document.querySelector("#outside").dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    nativeMenu.remove();
    assert.equal(panel.hidden, false);
    document.querySelector("#outside").dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    assert.equal(panel.hidden, true);
  } finally { nativeMenu?.remove(); menu.dispose(); }
});