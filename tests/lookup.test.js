import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { discordLookupUrl, mountDiscordLookup } from "../src/web/lookup.js";

function obliterateMenu(testContext, { resolve = () => ({ userId: "1", channelId: "100" }), open = () => {}, copyId, muteId = "user-context-mute" } = {}) {
  const { window, document } = parseHTML(`<html><head></head><body><div role="menu" id="user-context"><button role="menuitemcheckbox" id="${muteId}">Mute</button>${copyId ? `<button role="menuitem" id="user-context-devmode-copy-id-${copyId}">Copy User ID</button>` : ""}</div></body></html>`);
  const originals = { document: globalThis.document, MutationObserver: globalThis.MutationObserver, KeyboardEvent: globalThis.KeyboardEvent };
  globalThis.document = document;
  let refresh;
  globalThis.MutationObserver = class { constructor(callback) { refresh = callback; } observe() {} disconnect() {} };
  globalThis.KeyboardEvent = class extends window.Event { constructor(type, options) { super(type, options); Object.assign(this, options); } };
  const lookup = mountDiscordLookup({ obliterate: { context: resolve, open } });
  testContext.after(() => { lookup.dispose(); Object.assign(globalThis, originals); });
  const menu = document.querySelector('[role="menu"]');
  return { window, document, menu, refresh, lookup, action: menu.querySelector('[data-obliterate-action]') };
}

for (const [name, resolve] of [
  ["missing context", () => null],
  ["incomplete context", () => ({ userId: "1" })],
  ["throwing resolver", () => { throw new Error("Unavailable store"); }],
  ["conflicting Copy User ID", () => ({ userId: "2", channelId: "100" })],
]) {
  test(`Obliterate fails closed on insertion with ${name}`, testContext => {
    let opened = 0;
    const { action, refresh } = obliterateMenu(testContext, { resolve, copyId: "1", open: () => { opened++; } });
    assert.equal(action.getAttribute("aria-disabled"), "true");
    assert.match(action.title, /could not be verified/);
    action.click();
    refresh();
    action.click();
    assert.equal(opened, 0);
  });
}

test("Obliterate only recognizes the exact native user mute ID", testContext => {
  const { action, document } = obliterateMenu(testContext, { muteId: "custom-user-context-mute" });
  assert.equal(action, null);
  assert.equal(document.querySelector('[data-intellicord-owned]'), null);
});

for (const drift of ["user", "channel", "copy", "new copy", "throw", "missing"]) {
  test(`Obliterate permanently disables after ${drift} target drift until reopen`, testContext => {
    const context = { userId: "1", channelId: "100" };
    let broken = false;
    let opened = 0;
    const { action, menu, document, refresh } = obliterateMenu(testContext, {
      copyId: drift === "new copy" ? undefined : "1",
      resolve: () => {
        if (broken && drift === "throw") throw new Error("Store failed");
        return broken && drift === "missing" ? null : context;
      },
      open: () => { opened++; },
    });
    assert.equal(action.getAttribute("aria-disabled"), "false");
    broken = true;
    if (drift === "user") context.userId = "2";
    if (drift === "channel") context.channelId = "200";
    let copy = menu.querySelector('[id*="devmode-copy-id-"]');
    if (drift === "new copy") {
      copy = document.createElement("button");
      copy.setAttribute("role", "menuitem");
      menu.append(copy);
    }
    if (drift === "copy" || drift === "new copy") copy.id = "user-context-devmode-copy-id-2";
    action.click();
    assert.equal(opened, 0);
    assert.equal(action.getAttribute("aria-disabled"), "true");
    assert.match(action.title, /reopen the menu/);
    broken = false;
    context.userId = "1";
    context.channelId = "100";
    if (copy) copy.id = "user-context-devmode-copy-id-1";
    refresh();
    action.click();
    assert.equal(opened, 0);
    const reopened = menu.cloneNode(true);
    reopened.querySelectorAll('[data-intellicord-owned]').forEach(node => node.remove());
    menu.replaceWith(reopened);
    refresh();
    reopened.querySelector('[data-obliterate-action]').click();
    assert.equal(opened, 1);
  });
}

for (const failure of ["rejection", "throw"]) {
  test(`Obliterate open ${failure} is visible and disables further activation`, async testContext => {
    let opened = 0;
    const { action } = obliterateMenu(testContext, { open: () => {
      opened++;
      if (failure === "throw") throw new Error("Panel unavailable");
      return Promise.reject(new Error("Panel unavailable"));
    } });
    action.click();
    await new Promise(resolve => setImmediate(resolve));
    assert.match(action.title, /Could not open Obliterate: Panel unavailable/);
    assert.equal(action.getAttribute("aria-disabled"), "true");
    action.click();
    assert.equal(opened, 1);
  });
}

test("keyboard without Developer Mode preserves native activation and skips disabled navigation", testContext => {
  let opened = 0;
  const { window, menu, action, document } = obliterateMenu(testContext, { open: () => { opened++; } });
  const mute = menu.querySelector('#user-context-mute');
  const disabled = document.createElement("button");
  disabled.setAttribute("role", "menuitem");
  disabled.className = "disabled";
  menu.append(disabled);
  const keydown = (target, key) => {
    const event = new window.Event("keydown", { bubbles: true, cancelable: true });
    event.key = key;
    target.dispatchEvent(event);
    return event;
  };
  const nativeKeys = [];
  menu.addEventListener("keydown", event => { if (event.target === mute && ["Enter", " "].includes(event.key)) nativeKeys.push(event.key); });
  for (const key of ["Enter", " "]) assert.equal(keydown(mute, key).defaultPrevented, false);
  assert.deepEqual(nativeKeys, ["Enter", " "]);
  let focused;
  action.focus = () => { focused = action; };
  disabled.focus = () => { focused = disabled; };
  assert.equal(keydown(mute, "ArrowDown").defaultPrevented, true);
  assert.equal(focused, action);
  for (const key of ["Enter", " "]) assert.equal(keydown(action, key).defaultPrevented, true);
  assert.equal(opened, 2);
  assert.deepEqual(nativeKeys, ["Enter", " "]);
});

test("keyboard activates the owned external lookup link with Enter and Space", testContext => {
  const { window, menu } = obliterateMenu(testContext, { copyId: "1" });
  const link = menu.querySelector('[data-intellicord-lookup]');
  let clicked = 0;
  link.click = () => { clicked++; };
  for (const key of ["Enter", " "]) {
    const event = new window.Event("keydown", { bubbles: true, cancelable: true });
    event.key = key;
    link.dispatchEvent(event);
    assert.equal(event.defaultPrevented, true);
  }
  assert.equal(clicked, 2);
});

test("Obliterate refresh avoids duplicates and disposal removes only owned elements", testContext => {
  const { menu, document, refresh, lookup } = obliterateMenu(testContext, { copyId: "1" });
  refresh();
  refresh();
  assert.equal(menu.querySelectorAll('[data-obliterate-action]').length, 1);
  assert.equal(menu.querySelectorAll('[data-intellicord-lookup]').length, 1);
  lookup.dispose();
  lookup.dispose();
  assert.equal(document.querySelectorAll('[data-intellicord-owned]').length, 0);
  assert.equal(document.querySelectorAll("style").length, 0);
  assert.equal(menu.querySelectorAll('[role^="menuitem"]').length, 2);
});

test("lookup URLs preserve exact IDs and encode request parameters", () => {
  const url = new URL(discordLookupUrl("9007199254740993", "request-token"));
  assert.equal(url.origin, "https://leakfun.com");
  assert.equal(url.pathname, "/dashboard/searches/discord");
  assert.deepEqual(Object.fromEntries(url.searchParams), {
    source: "universal", autorun: "1", q: "9007199254740993", mode: "lookup", autorunToken: "request-token",
  });
  assert.notEqual(discordLookupUrl("1"), discordLookupUrl("1"));
  for (const id of [1, "", "1&other=2", "123456789012345678901"]) assert.throws(() => discordLookupUrl(id), /Invalid/);
});

test("Obliterate sits above Mute without Developer Mode, binds exact context, and never executes on insertion", () => {
  const { window, document } = parseHTML('<html><head></head><body><div role="menu" id="user-context"><button role="menuitemcheckbox" id="user-context-mute">Mute</button></div><div role="menu" id="message-context"><button role="menuitem" id="message-mute">Mute</button></div></body></html>');
  const originals = { document: globalThis.document, MutationObserver: globalThis.MutationObserver, KeyboardEvent: globalThis.KeyboardEvent };
  globalThis.document = document;
  let refresh;
  globalThis.MutationObserver = class { constructor(callback) { refresh = callback; } observe() {} disconnect() {} };
  globalThis.KeyboardEvent = class extends window.Event { constructor(type, options) { super(type, options); Object.assign(this, options); } };
  let context = { userId: "1", channelId: "100" };
  const opened = [];
  const lookup = mountDiscordLookup({ obliterate: { context: () => context, open: value => opened.push(value) } });
  try {
    refresh();
    assert.deepEqual(opened, []);
    assert.equal(document.querySelectorAll('[data-obliterate-action]').length, 1);
    const action = document.querySelector('[data-obliterate-action]');
    assert.equal(action.nextElementSibling.id, "user-context-mute");
    action.click();
    assert.deepEqual(opened, [{ userId: "1", channelId: "100" }]);
    context = { userId: "2", channelId: "200" };
    action.click();
    assert.equal(opened.length, 1);
    assert.equal(action.getAttribute("aria-disabled"), "true");
  } finally { lookup.dispose(); Object.assign(globalThis, originals); }
  assert.equal(document.querySelector('[data-obliterate-action]'), null);
});

test("native user menus get one external link after Copy ID and clean up on detach", () => {
  const { window, document } = parseHTML('<html><body><div role="menu" id="user-context"><div role="menuitem" id="user-context-devmode-copy-id-9007199254740993" class="native-item">Copy User ID</div></div><div role="menu" id="message"><div role="menuitem" id="message-devmode-copy-id-2">Copy Message ID</div></div></body></html>');
  globalThis.document = document;
  let refresh;
  let disconnected = false;
  const originalObserver = globalThis.MutationObserver;
  globalThis.MutationObserver = class {
    constructor(callback) { refresh = callback; }
    observe() {}
    disconnect() { disconnected = true; }
  };
  const lookup = mountDiscordLookup();
  try {
    refresh();
    const link = document.querySelector('[data-intellicord-lookup]');
    assert.equal(document.querySelectorAll('[data-intellicord-lookup]').length, 1);
    assert.equal(link.previousElementSibling.textContent, "Copy User ID");
    assert.equal(link.className, "native-item");
    assert.equal(new URL(link.href).searchParams.get("q"), "9007199254740993");
    assert.equal(link.target, "_blank");
    assert.equal(link.rel, "noopener noreferrer");
    assert.equal(link.referrerPolicy, "no-referrer");
    assert.equal(document.querySelector('#message [data-intellicord-lookup]'), null);
    let focused = false;
    link.focus = () => { focused = true; };
    const key = new window.Event("keydown", { bubbles: true, cancelable: true });
    key.key = "ArrowDown";
    link.previousElementSibling.dispatchEvent(key);
    assert.equal(focused, true);
    assert.equal(key.defaultPrevented, true);
    document.querySelector('#user-context').remove();
    refresh();
  } finally {
    lookup.dispose();
    globalThis.MutationObserver = originalObserver;
  }
  assert.equal(disconnected, true);
  assert.equal(document.querySelectorAll('[data-intellicord-lookup]').length, 0);
});

test("native user tools open panels with exact IDs and restore Copy ID on disposal", () => {
  const { window, document } = parseHTML('<html><body><div role="menu" id="user-context"><div role="menuitem" id="user-context-devmode-copy-id-9007199254740993" class="native-item"><span class="native-label">Copy User ID</span></div></div><div role="menu" id="user-context"><div role="menuitem" id="user-context-devmode-copy-id-9007199254740995" class="native-item" hidden><span class="native-label">Copy User ID</span></div></div></body></html>');
  const originalDocument = globalThis.document;
  const originalObserver = globalThis.MutationObserver;
  const originalKeyboardEvent = globalThis.KeyboardEvent;
  globalThis.document = document;
  let refresh;
  let disconnected = false;
  globalThis.MutationObserver = class {
    constructor(callback) { refresh = callback; }
    observe() {}
    disconnect() { disconnected = true; }
  };
  globalThis.KeyboardEvent = class extends window.Event {
    constructor(type, options) { super(type, options); Object.assign(this, options); }
  };
  const userIds = ["9007199254740993", "9007199254740995"];
  const opened = [];
  const watchOpened = [];
  const lookup = mountDiscordLookup({
    identity: { open: (...args) => opened.push(args) },
    watch: {
      open(id) { watchOpened.push(id); },
    },
  });
  const copies = [...document.querySelectorAll('[id*="devmode-copy-id-"]')];
  try {
    refresh();
    refresh();
    assert.equal(document.querySelectorAll('[data-intellicord-owned]').length, 6);
    for (const [index, menu] of [...document.querySelectorAll('[role="menu"]')].entries()) {
      const userId = userIds[index];
      const copy = copies[index];
      const identity = menu.querySelector('button[role="menuitem"]');
      const watch = menu.querySelector('[data-voice-watch-id]');
      const link = menu.querySelector('[data-intellicord-lookup]');
      assert.equal(copy.hidden, true);
      assert.equal(identity.textContent, "Identity Lens");
      assert.equal(identity.className, copy.className);
      assert.equal(watch.className, copy.className);
      for (const action of [identity, watch]) {
        assert.equal(action.style.background, "transparent");
        assert.match(action.style.color, /var\(--interactive-text-default/);
        assert.equal(action.style.font, "inherit");
        action.dispatchEvent(new window.Event("focus"));
        action.dispatchEvent(new window.Event("pointerleave"));
        assert.match(action.style.background, /var\(--menu-item-default-hover-bg/);
        assert.match(action.style.color, /var\(--menu-item-default-hover-text/);
        action.dispatchEvent(new window.Event("blur"));
        assert.equal(action.style.background, "transparent");
      }
      assert.equal(watch.querySelector("input"), null);
      assert.equal(watch.firstElementChild.textContent, "Voice Watch");
      assert.equal(watch.firstElementChild.className, "native-label");
      assert.equal(identity.firstElementChild.className, "native-label");
      assert.equal(watch.style.justifyContent, "space-between");
      assert.match(document.querySelector("style").textContent, /\[data-intellicord-user-action\]:focus\{/);
      assert.equal(watch.getAttribute("role"), "menuitem");
      assert.equal(watch.querySelector('[data-voice-watch-indicator]'), null);
      assert.equal(watch.querySelector("svg"), null);
      assert.equal(watch.hasAttribute("aria-checked"), false);
      assert.match(document.querySelector("style").textContent, /\.theme-light \[data-intellicord-user-action\]\{color-scheme:light\}/);
      assert.match(document.querySelector("style").textContent, /\.theme-dark \[data-intellicord-user-action\]\{color-scheme:dark\}/);
      assert.equal(watch.getAttribute("aria-label"), "Voice Watch");
      assert.equal(watch.dataset.voiceWatchId, userId);
      assert.equal(link.id, `user-context-intellicord-lookup-${userId}`);
      assert.equal(link.dataset.intellicordLookup, userId);
      assert.equal(new URL(link.href).searchParams.get("q"), userId);
      let dismissed = false;
      menu.addEventListener("keydown", event => { if (event.key === "Escape") dismissed = true; });
      if (index === 0) watch.click();
      else {
        const enter = new window.Event("keydown", { bubbles: true, cancelable: true });
        enter.key = "Enter";
        watch.dispatchEvent(enter);
        assert.equal(enter.defaultPrevented, true);
      }
      assert.equal(dismissed, true);
      assert.equal(watchOpened[index], userId);
      assert.equal(watch.hasAttribute("aria-checked"), false);
      dismissed = false;
      identity.click();
      assert.equal(dismissed, true);
      assert.deepEqual(opened[index], [userId]);
    }
    refresh();
    assert.equal(document.querySelectorAll('[data-intellicord-owned]').length, 6);
  } finally {
    lookup.dispose();
    globalThis.document = originalDocument;
    globalThis.MutationObserver = originalObserver;
    globalThis.KeyboardEvent = originalKeyboardEvent;
  }
  assert.equal(disconnected, true);
  assert.equal(copies[0].hidden, false);
  assert.equal(copies[1].hidden, true);
  assert.ok(copies.every(copy => copy.isConnected));
  assert.equal(document.querySelectorAll('[data-intellicord-owned]').length, 0);
  assert.equal(document.querySelector("style"), null);
});