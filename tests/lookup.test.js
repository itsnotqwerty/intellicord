import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { discordLookupUrl, mountDiscordLookup } from "../src/web/lookup.js";

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