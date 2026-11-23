import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { inspectLink, mountLinkInspector } from "../src/web/links.js";

test("tracking removal preserves unknown parameters, duplicates, fragments and URL components", () => {
  const result = inspectLink("https://example.com:8443/a%20b?utm_source=a&keep=1&fbclid=x&keep=2&gclid=y&msclkid=z&UTM_medium=b#section");
  assert.equal(result.scheme, "https");
  assert.equal(result.hostname, "example.com");
  assert.equal(result.port, "8443");
  assert.equal(result.pathname, "/a%20b");
  assert.equal(result.cleanedUrl, "https://example.com:8443/a%20b?keep=1&keep=2#section");
  assert.equal(result.parameters.length, 7);
  assert.deepEqual(result.parameters[1], { name: "keep", value: "1", tracking: false });
  assert.equal(inspectLink("https://example.com/?utmish=1&fbclid_extra=2#utm_source=x").cleanedUrl,
    "https://example.com/?utmish=1&fbclid_extra=2#utm_source=x");
});

test("credentials are redacted for display, preserved for copying and never opened", () => {
  const result = inspectLink("https://name:s%65cret@example.com/path?utm_source=x&keep=1#hash");
  assert.equal(result.credentials, true);
  assert.equal(result.canOpen, false);
  assert.equal(result.displayUrl, "https://example.com/path?utm_source=x&keep=1#hash");
  assert.equal(result.displayCleanedUrl, "https://example.com/path?keep=1#hash");
  assert.equal(result.cleanedUrl, "https://name:s%65cret@example.com/path?keep=1#hash");
  assert.equal(inspectLink("https://name@example.com").canOpen, false);
  assert.equal(inspectLink("https://:secret@example.com").canOpen, false);
});

test("URL parsing exposes the actual ASCII host, not apparent hosts or encoded input", () => {
  assert.equal(inspectLink("https://%65xample.com").hostname, "example.com");
  assert.equal(inspectLink("https://b\u00fccher.example").hostname, "xn--bcher-kva.example");
  assert.equal(inspectLink("https://example.com@evil.example").hostname, "evil.example");
  assert.equal(inspectLink("http://[::1]:8080/").hostname, "[::1]");
  for (const input of ["not a URL", "/relative", "https://%zz/"]) assert.throws(() => inspectLink(input), TypeError);
  for (const input of ["javascript:alert(1)", "data:text/plain,test", "file:///tmp/test", "ftp://example.com"]) {
    assert.equal(inspectLink(input).canOpen, false);
  }
});

test("without tracking parameters the exact parsed query encoding is retained", () => {
  const url = "https://example.com/?q=a%20b&empty=&q=%7e#fragment";
  assert.equal(inspectLink(url).cleanedUrl, url);
  assert.equal(inspectLink("https://example.com/?utm_source=1&utm_source=2#hash").cleanedUrl, "https://example.com/#hash");
});

function fixture(adapter) {
  const { document, window } = parseHTML('<html><head></head><body><a id="source" href="https://example.com/path?utm_source=x&keep=1#hash"><span>Link</span></a><div id="other">Other</div></body></html>');
  globalThis.document = document;
  const copied = [];
  const previousFetch = globalThis.fetch;
  const previousOpen = window.open;
  let requests = 0;
  globalThis.fetch = () => { requests++; throw new Error("Unexpected request"); };
  window.open = () => { requests++; throw new Error("Unexpected external open"); };
  const navigator = { clipboard: { writeText: async value => { copied.push(value); } } };
  Object.defineProperty(document, "defaultView", { configurable: true,
    value: new Proxy(window, { get: (target, key) => key === "navigator" ? navigator : target[key] }) });
  let focused = null;
  Object.defineProperty(document, "activeElement", { configurable: true, get: () => focused });
  window.HTMLElement.prototype.focus = function () { focused = this; };
  const feature = mountLinkInspector(adapter);
  const source = document.querySelector("#source");
  function capture(node = source.firstElementChild) {
    const event = new window.Event("contextmenu", { bubbles: true, cancelable: true });
    node.dispatchEvent(event);
    assert.equal(event.defaultPrevented, false);
  }
  async function flush() { await new Promise(resolve => setImmediate(resolve)); }
  async function menu(nav = "link-context", href, itemId = `${nav}-copy-link`) {
    const node = document.createElement("div");
    node.setAttribute("role", "menu");
    node.setAttribute("data-nav-id", nav);
    const copy = document.createElement("div");
    copy.setAttribute("role", "menuitem");
    copy.id = itemId;
    copy.textContent = "Copy Link";
    if (href !== undefined) copy.setAttribute("data-url", href);
    node.append(copy);
    document.body.append(node);
    await flush();
    return node;
  }
  function key(node, value, shiftKey = false) {
    const event = new window.Event("keydown", { bubbles: true, cancelable: true });
    event.key = value;
    event.shiftKey = shiftKey;
    node.dispatchEvent(event);
    return event;
  }
  return { document, window, feature, source, capture, menu, flush, key, copied,
    requests: () => requests,
    dispose() { feature.dispose(); globalThis.fetch = previousFetch; window.open = previousOpen; },
  };
}

test("captured nested anchors attach only to matching link menus and inspect without requests", async () => {
  const context = fixture();
  try {
    context.capture();
    const menu = await context.menu("link-context", context.source.getAttribute("href"));
    const button = menu.querySelector(".ic-link-item");
    assert.ok(button);
    const css = context.document.querySelector('[data-intellicord-links-style]').textContent;
    assert.match(css, /--ic-link-foreground:var\(--interactive-text-default/);
    assert.match(css, /color:var\(--ic-link-foreground\)/);
    assert.match(css, /\.theme-light \.ic-link-item \{ color-scheme:light;/);
    assert.match(css, /\.theme-dark \.ic-link-item \{ color-scheme:dark;/);
    assert.match(css, /color:var\(--menu-item-default-hover-text/);
    button.click();
    const dialog = context.document.querySelector("dialog[open]");
    assert.ok(dialog);
    assert.match(dialog.textContent, /ASCII hostnameexample.com/);
    assert.match(dialog.textContent, /utm_source = x \(tracking\)/);
    assert.equal(context.requests(), 0);
    const links = [...dialog.querySelectorAll("a")];
    assert.equal(links.length, 2);
    assert.equal(links[1].href, "https://example.com/path?keep=1#hash");
    for (const link of links) {
      assert.equal(link.target, "_blank");
      assert.equal(link.rel, "noopener noreferrer");
    }
    const copies = [...dialog.querySelectorAll("button")].filter(node => node.textContent.startsWith("Copy"));
    for (const copy of copies) copy.click();
    await context.flush();
    assert.deepEqual(context.copied, [context.source.getAttribute("href"), links[1].href]);
    context.key(dialog, "Escape");
    assert.equal(context.document.querySelector("dialog"), null);
    assert.equal(context.document.activeElement, context.source);
  } finally { context.dispose(); }
});

test("Inspect Link uses native row styling between Copy Link and Open Link", async () => {
  for (const openOnly of [false, true]) {
    const context = fixture();
    try {
      context.capture();
      const menu = context.document.createElement("div");
      menu.setAttribute("role", "menu");
      menu.setAttribute("data-nav-id", "link-context");
      if (!openOnly) {
        const copy = context.document.createElement("div");
        copy.setAttribute("role", "menuitem");
        copy.id = "link-context-copy-link";
        copy.className = "native-item";
        copy.innerHTML = '<span class="native-label">Copy Link</span>';
        menu.append(copy);
      }
      const open = context.document.createElement("div");
      open.setAttribute("role", "menuitem");
      open.id = "link-context-open-link";
      open.className = "native-item";
      open.innerHTML = '<span class="native-label">Open Link</span>';
      menu.append(open);
      context.document.body.append(menu);
      await context.flush();
      const inspect = menu.querySelector(".ic-link-item");
      if (!openOnly) {
        const copy = menu.querySelector('#link-context-copy-link');
        assert.equal(inspect.previousElementSibling, copy);
      }
      assert.ok(inspect.classList.contains("native-item"));
      assert.equal(inspect.firstElementChild.className, "native-label");
      assert.equal(inspect.hasAttribute("data-link-fallback-style"), false);
      assert.equal(inspect.nextElementSibling, open);
      assert.equal(inspect.textContent, "Inspect Link");
      assert.equal(inspect.getAttribute("role"), "menuitem");
      assert.equal(inspect.querySelector("svg"), null);
      assert.equal(context.requests(), 0);
    } finally { context.dispose(); }
  }
});

test("credential dialog never displays credentials or offers external links", async () => {
  const context = fixture();
  try {
    context.source.href = "https://username:password@example.com/?utm_source=x";
    context.capture();
    const menu = await context.menu("message");
    menu.querySelector(".ic-link-item").click();
    const dialog = context.document.querySelector("dialog");
    assert.doesNotMatch(dialog.textContent, /username|password/);
    assert.equal(dialog.querySelectorAll("a").length, 0);
    assert.equal(dialog.querySelectorAll("button[disabled]").length, 2);
    assert.equal(context.requests(), 0);
  } finally { context.dispose(); }
});

test("unrelated, mismatched, absent and stale anchor contexts cannot inject", async () => {
  for (const scenario of ["absent", "unrelated", "mismatch", "user", "removed", "unsupported", "existing"]) {
    const context = fixture();
    try {
      if (scenario === "existing") await context.menu();
      if (scenario === "unsupported") context.source.href = "javascript:alert(1)";
      if (scenario !== "absent") context.capture();
      if (scenario === "unrelated") context.capture(context.document.querySelector("#other"));
      if (scenario === "removed") context.source.remove();
      if (scenario !== "existing") await context.menu(scenario === "user" ? "user-context" : "link-context",
        scenario === "mismatch" ? "https://different.example/" : undefined);
      await context.flush();
      assert.equal(context.document.querySelector(".ic-link-item"), null, scenario);
    } finally { context.dispose(); }
  }
});

test("menu dismissal, outside click and detach remove injection and pending context", async () => {
  const context = fixture();
  try {
    context.capture();
    const menu = await context.menu();
    menu.remove();
    await context.flush();
    await context.menu();
    assert.equal(context.document.querySelector(".ic-link-item"), null);
    context.capture();
    context.document.querySelector("#other").dispatchEvent(new context.window.Event("pointerdown", { bubbles: true }));
    await context.menu();
    assert.equal(context.document.querySelector(".ic-link-item"), null);
    context.capture();
    const last = await context.menu();
    assert.ok(last.querySelector(".ic-link-item"));
    context.feature.dispose();
    context.feature.dispose();
    assert.equal(context.document.querySelector('[data-intellicord-link-item], [data-intellicord-links-style], [data-intellicord-links-overlay]'), null);
    context.capture();
    await context.menu();
    assert.equal(context.document.querySelector(".ic-link-item"), null);
  } finally { context.dispose(); }
});

test("keyboard navigation includes inserted item and traps dialog focus", async () => {
  const context = fixture();
  try {
    context.capture();
    const menu = await context.menu();
    const native = menu.firstElementChild;
    const inserted = menu.querySelector(".ic-link-item");
    native.focus();
    assert.equal(context.key(native, "ArrowDown").defaultPrevented, true);
    assert.equal(context.document.activeElement, inserted);
    context.key(inserted, "ArrowDown");
    assert.equal(context.document.activeElement, native);
    context.key(native, "End");
    assert.equal(context.document.activeElement, inserted);
    context.key(inserted, "Home");
    assert.equal(context.document.activeElement, native);
    context.key(native, "ArrowUp");
    assert.equal(context.document.activeElement, inserted);
    context.key(inserted, "Enter");
    const dialog = context.document.querySelector("dialog");
    const controls = [...dialog.querySelectorAll("button, a")];
    context.key(controls[0], "Tab", true);
    assert.equal(context.document.activeElement, controls.at(-1));
    context.key(controls.at(-1), "Tab");
    assert.equal(context.document.activeElement, controls[0]);
    context.key(dialog, "Escape");
    assert.equal(context.document.querySelector("dialog"), null);
  } finally { context.dispose(); }
});

test("adapter can identify a native menu copy item with a matching URL", async () => {
  const context = fixture({ menuAnchor: menu => ({ item: menu.firstElementChild, href: "https://example.com/path?utm_source=x&keep=1#hash" }) });
  try {
    context.capture();
    const menu = await context.menu("custom-link", undefined, "custom-copy");
    assert.ok(menu.querySelector(".ic-link-item"));
  } finally { context.dispose(); }
});

test("recognized menus can populate their copy item in a later mutation", async () => {
  const context = fixture();
  try {
    context.capture();
    const menu = context.document.createElement("div");
    menu.setAttribute("role", "menu");
    menu.setAttribute("data-nav-id", "link-context");
    context.document.body.append(menu);
    await context.flush();
    const item = context.document.createElement("div");
    item.id = "link-context-copy-link";
    item.setAttribute("role", "menuitem");
    menu.append(item);
    await context.flush();
    assert.ok(menu.querySelector(".ic-link-item"));
    assert.equal(menu.querySelectorAll(".ic-link-item").length, 1);
  } finally { context.dispose(); }
});

test("native menu reuse and changed copy URLs invalidate injection", async () => {
  for (const change of ["nav", "url", "item"]) {
    const context = fixture();
    try {
      context.capture();
      const menu = await context.menu();
      assert.ok(menu.querySelector(".ic-link-item"));
      if (change === "nav") menu.setAttribute("data-nav-id", "user-context");
      if (change === "url") menu.firstElementChild.setAttribute("data-url", "https://unrelated.example/");
      if (change === "item") menu.firstElementChild.remove();
      await context.flush();
      assert.equal(menu.querySelector(".ic-link-item"), null, change);
    } finally { context.dispose(); }
  }
});

test("removing an unpopulated native menu clears pending context", async () => {
  const context = fixture();
  try {
    context.capture();
    const menu = context.document.createElement("div");
    menu.setAttribute("role", "menu");
    menu.setAttribute("data-nav-id", "message");
    context.document.body.append(menu);
    await context.flush();
    menu.remove();
    await context.flush();
    await context.menu();
    assert.equal(context.document.querySelector(".ic-link-item"), null);
  } finally { context.dispose(); }
});

test("standalone link-copy items identify link menus but never user menus", async () => {
  for (const nav of ["", "user-context"]) {
    const context = fixture();
    try {
      context.capture();
      const menu = await context.menu(nav, undefined, "link-copy");
      assert.equal(Boolean(menu.querySelector(".ic-link-item")), nav === "");
    } finally { context.dispose(); }
  }
});