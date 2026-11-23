import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createElement, Save } from "lucide";
import { collectIdentity, mountIdentityLens } from "../src/web/identity.js";

const id = "9007199254740993";

test("identity copies only whitelisted scalar public fields without inspecting private data or channel", () => {
  const user = { id, username: "morgan", globalName: "Morgan", display_name: "Local name", discriminator: "0",
    bot: false, system: true, avatar: "abc", banner: "def", accentColor: 123, publicFlags: 4194304 };
  for (const field of ["email", "phone", "token", "tokens", "relationships", "flags", "privateFlags", "avatar_url", "getAvatarURL"]) {
    Object.defineProperty(user, field, { get() { throw new Error(`Read private field ${field}`); } });
  }
  const channel = new Proxy({}, { get() { throw new Error("Read channel data"); } });
  const identity = collectIdentity(user, channel);
  assert.deepEqual(identity, { id, username: "morgan", globalName: "Morgan", display_name: "Local name", discriminator: "0",
    bot: false, system: true, avatar: "abc", banner: "def", accentColor: 123, publicFlags: 4194304,
    createdAt: new Date(Number((BigInt(id) >> 22n) + 1420070400000n)).toISOString() });
  identity.username = "Changed";
  assert.equal(user.username, "morgan");
});

test("creation UTC uses BigInt before conversion, preserving timestamp boundary and exact IDs", () => {
  const timestamp = 1700000000123n;
  const snowflake = (((timestamp - 1420070400000n) << 22n) - 1n).toString();
  assert.equal(collectIdentity({ id: snowflake }).createdAt, "2023-11-14T22:13:20.122Z");
  assert.equal(collectIdentity({ id }).id, id);
  assert.notEqual(collectIdentity({ id }).id, collectIdentity({ id: "9007199254740992" }).id);
  assert.equal(collectIdentity({ id: "18446744073709551615" }).createdAt,
    new Date(Number((18446744073709551615n >> 22n) + 1420070400000n)).toISOString());
});

test("missing and malformed fields remain unavailable, not coerced or derived into a completeness claim", () => {
  const empty = collectIdentity(null);
  assert.ok(Object.values(empty).every(value => value === null));
  for (const invalid of [9007199254740993, "", "0", "-1", "1e9", "18446744073709551616", null]) {
    assert.equal(collectIdentity({ id: invalid }).createdAt, null);
  }
  const identity = collectIdentity({ id, username: {}, bot: 1, system: "false", publicFlags: {}, accentColor: NaN });
  assert.equal(identity.username, null);
  assert.equal(identity.bot, null);
  assert.equal(identity.system, null);
  assert.equal(identity.publicFlags, null);
  assert.equal(identity.accentColor, null);
  assert.equal(collectIdentity({ id, publicFlags: 0, accentColor: 0 }).publicFlags, 0);
});

function dom() {
  const { document, window } = parseHTML("<html><body><button id='origin'>Origin</button></body></html>");
  const originalDocument = globalThis.document;
  const originalWindow = globalThis.window;
  globalThis.document = document;
  globalThis.window = window;
  let focused = document.querySelector("#origin");
  Object.defineProperty(document, "activeElement", { get: () => focused });
  window.HTMLElement.prototype.focus = function () { focused = this; };
  return { document, window, restore() { globalThis.document = originalDocument; globalThis.window = originalWindow; } };
}

test("modal uses native variables, safe avatar, selectable public rows, Escape and restored focus", () => {
  const fixture = dom();
  const { document, window } = fixture;
  const origin = document.activeElement;
  const lens = mountIdentityLens({ users: { getUser: requested => {
    assert.equal(requested, id);
    return { id, username: "<script>not markup</script>", avatar: "a_abcdef", publicFlags: 0 };
  } } });
  try {
    lens.open(id);
    const dialog = document.querySelector('[role="dialog"]');
    assert.equal(dialog.getAttribute("aria-modal"), "true");
    assert.equal(dialog.getAttribute("aria-label"), "Identity Lens");
    assert.equal(dialog.parentElement.style.zIndex, "1003");
    assert.match(document.querySelector("style").textContent, /var\(--background-base-low,var\(--background-primary/);
    assert.match(document.querySelector("style").textContent, /\.theme-light \.ic-user-tool\{color-scheme:light\}/);
    assert.match(document.querySelector("style").textContent, /\.theme-dark \.ic-user-tool\{color-scheme:dark\}/);
    assert.match(document.querySelector("style").textContent, /color:var\(--ic-tool-text\)/);
    assert.match(document.querySelector("style").textContent, /user-select:text/);
    assert.equal(document.querySelector('[data-field="username"] dd').textContent, "<script>not markup</script>");
    assert.equal(document.querySelector("script"), null);
    assert.equal(document.querySelector('[data-field="display_name"] dd').textContent, "Unavailable");
    assert.equal(document.querySelector('[data-field="publicFlags"] dd').textContent, "0");
    assert.match(dialog.textContent, /Observed local profile only/);
    assert.equal(document.querySelector("img").src, `https://cdn.discordapp.com/avatars/${id}/a_abcdef.gif?size=128`);
    assert.equal(document.querySelector("img").referrerPolicy, "no-referrer");
    const tab = new window.Event("keydown", { cancelable: true });
    tab.key = "Tab";
    document.dispatchEvent(tab);
    assert.equal(document.activeElement.getAttribute("aria-label"), "Copy User ID");
    const escape = new window.Event("keydown", { cancelable: true });
    escape.key = "Escape";
    document.dispatchEvent(escape);
    assert.equal(escape.defaultPrevented, true);
    assert.equal(document.querySelector('[role="dialog"]'), null);
    assert.equal(document.activeElement, origin);
    lens.open(id);
    lens.open(id);
    assert.equal(document.querySelectorAll('[role="dialog"]').length, 1);
    lens.dispose();
    assert.equal(document.activeElement, origin);
    lens.open(id);
    assert.equal(document.querySelector('[role="dialog"]'), null);
  } finally { lens.dispose(); fixture.restore(); }
});

test("fallback must match exact ID; unsafe avatars never render; unavailable user still has exact ID", () => {
  const fixture = dom();
  const lens = mountIdentityLens({ users: { getUser() { return null; } } });
  try {
    for (const avatar of ["javascript:alert(1)", "https://evil.test/avatars/1/abc.png", "https://cdn.discordapp.com/banners/1/abc.png"]) {
      lens.open(id, { id, username: "Fallback", avatar });
      assert.equal(fixture.document.querySelector("img"), null);
      assert.equal(fixture.document.querySelector('[data-field="username"] dd').textContent, "Fallback");
    }
    lens.open(id, { id: "9007199254740992", username: "Wrong user" });
    assert.equal(fixture.document.querySelector('[data-field="id"] dd').textContent, id);
    assert.equal(fixture.document.querySelector('[data-field="username"] dd').textContent, "Unavailable");
    assert.throws(() => lens.open(Number(id)), /Invalid/);
  } finally { lens.dispose(); fixture.restore(); }
});

test("copy icon copies only exact ID and reports clipboard errors locally", async () => {
  const fixture = dom();
  const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  const copied = [];
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard: { writeText: async value => copied.push(value) } } });
  const lens = mountIdentityLens({});
  try {
    lens.open(id);
    assert.equal(fixture.document.querySelector("header").querySelectorAll("button").length, 2);
    assert.equal(fixture.document.querySelector('header [aria-label="Copy user ID"]'), null);
    assert.equal(fixture.document.querySelector('[aria-label="Copy all JSON"] svg').outerHTML,
      createElement(Save, { width: 18, height: 18, "aria-hidden": "true" }).outerHTML);
    const copy = fixture.document.querySelector('[data-field="id"] [aria-label="Copy User ID"]');
    assert.ok(copy.querySelector("svg"));
    copy.click();
    await Promise.resolve();
    assert.deepEqual(copied, [id]);
    assert.equal(fixture.document.querySelector('[role="status"]').textContent, "User ID copied");
    globalThis.navigator.clipboard.writeText = async () => { throw new Error("denied"); };
    copy.click();
    await Promise.resolve();
    assert.equal(fixture.document.querySelector('[role="status"]').textContent, "Clipboard unavailable");
  } finally {
    lens.dispose(); fixture.restore();
    if (originalNavigator) Object.defineProperty(globalThis, "navigator", originalNavigator);
    else delete globalThis.navigator;
  }
});

function clipboard(writeText) {
  const original = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard: { writeText } } });
  return () => {
    if (original) Object.defineProperty(globalThis, "navigator", original);
    else delete globalThis.navigator;
  };
}

test("each available whitelist value has an exact scalar copy, including false, zero, hashes and UTC dates", async () => {
  const fixture = dom();
  const copied = [];
  const restoreClipboard = clipboard(async value => { copied.push(value); });
  const user = { id, username: "morgan", globalName: "Morgan", display_name: "Local name", discriminator: "0",
    bot: false, system: false, avatar: "abc", banner: "a_def", accentColor: 0, publicFlags: 0 };
  for (const field of ["avatar_url", "banner_url", "getAvatarURL", "email", "token", "files", "messages"]) {
    Object.defineProperty(user, field, { get() { throw new Error(`Unexpected read: ${field}`); } });
  }
  const lens = mountIdentityLens({ users: { getUser: () => user } });
  try {
    lens.open(id);
    for (const [field, value] of Object.entries(collectIdentity(user))) {
      const row = fixture.document.querySelector(`[data-field="${field}"]`);
      const copy = row.querySelector("button");
      assert.ok(copy.querySelector("svg"), field);
      assert.equal(copy.title, copy.getAttribute("aria-label"));
      copy.click();
      await Promise.resolve();
      assert.equal(copied.at(-1), String(value), field);
    }
    fixture.document.querySelector('[aria-label="Copy all JSON"]').click();
    await Promise.resolve();
    assert.deepEqual(JSON.parse(copied.at(-1)), collectIdentity(user));
    assert.equal(fixture.document.querySelector('[role="status"]').textContent, "Identity JSON copied");
    for (const field of ["avatarUrl", "bannerUrl"]) {
      const row = fixture.document.querySelector(`[data-field="${field}"]`);
      row.querySelector("button").click();
      await Promise.resolve();
      assert.equal(copied.at(-1), row.querySelector("a").href);
    }
    lens.open(id, null);
    assert.equal(fixture.document.querySelector("textarea"), null);
  } finally { lens.dispose(); restoreClipboard(); fixture.restore(); }
});

test("unavailable fields have no active copy control or external links", () => {
  const fixture = dom();
  const lens = mountIdentityLens({});
  try {
    lens.open(id);
    for (const row of fixture.document.querySelectorAll("dl > div")) {
      if (["id", "createdAt"].includes(row.dataset.field)) continue;
      assert.equal(row.querySelector("dd").textContent, "Unavailable");
      assert.equal(row.querySelector("button"), null);
    }
    assert.equal(fixture.document.querySelector("a"), null);
    assert.equal(fixture.document.querySelector("img"), null);
  } finally { lens.dispose(); fixture.restore(); }
});

test("observed avatar and banner files have explicit safe external links without automatic navigation", () => {
  const fixture = dom();
  const lens = mountIdentityLens({});
  try {
    for (const host of ["cdn.discordapp.com", "media.discordapp.net"]) {
      const avatar = `https://${host}/avatars/${id}/abc.webp?size=256`;
      const banner = `https://${host}/banners/${id}/a_def.gif?size=512`;
      lens.open(id, { id, avatar, banner });
      const links = [...fixture.document.querySelectorAll("a")];
      assert.deepEqual(links.map(link => link.href), [avatar, banner]);
      assert.equal(fixture.document.querySelectorAll("img").length, 1);
      assert.equal(fixture.document.querySelector("img").src, avatar);
      assert.equal(fixture.document.querySelector("iframe"), null);
      for (const link of links) {
        assert.equal(link.target, "_blank");
        assert.equal(link.rel, "noopener noreferrer");
        assert.equal(link.getAttribute("referrerpolicy"), "no-referrer");
        assert.match(link.getAttribute("aria-label"), /^Open (Avatar|Banner) URL externally$/);
        assert.equal(link.title, link.getAttribute("aria-label"));
        assert.ok(link.querySelector("svg"));
      }
    }
    lens.open(id, { id, avatar: "abc", banner: "a_def" });
    assert.equal(fixture.document.querySelector('[data-field="avatarUrl"] a').href,
      `https://cdn.discordapp.com/avatars/${id}/abc.png?size=128`);
    assert.equal(fixture.document.querySelector('[data-field="bannerUrl"] a').href,
      `https://cdn.discordapp.com/banners/${id}/a_def.gif?size=128`);
  } finally { lens.dispose(); fixture.restore(); }
});

test("unsafe, mismatched and traversal media URLs never become links or avatar previews", () => {
  const fixture = dom();
  const lens = mountIdentityLens({});
  try {
    for (const [field, kind, wrongKind] of [["avatar", "avatars", "banners"], ["banner", "banners", "avatars"]]) {
      for (const value of [
        "javascript:alert(1)", "data:image/png;base64,abc", "//cdn.discordapp.com/avatars/1/abc.png",
        `http://cdn.discordapp.com/${kind}/${id}/abc.png`,
        `https://evil.test/${kind}/${id}/abc.png`,
        `https://cdn.discordapp.com.evil.test/${kind}/${id}/abc.png`,
        `https://user:pass@cdn.discordapp.com/${kind}/${id}/abc.png`,
        `https://cdn.discordapp.com:8443/${kind}/${id}/abc.png`,
        `https://cdn.discordapp.com/${kind}/1/abc.png`,
        `https://cdn.discordapp.com/${wrongKind}/${id}/abc.png`,
        `https://cdn.discordapp.com/${kind}/${id}/abc.svg`,
        `https://cdn.discordapp.com/${kind}/${id}/not-a-hash.png`,
        `https://cdn.discordapp.com/${kind}/${id}/abc.png/extra`,
        `https://cdn.discordapp.com/${kind}/${id}/../${id}/abc.png`,
        `https://cdn.discordapp.com/${kind}/${id}/%2e%2e/${id}/abc.png`,
        `https://cdn.discordapp.com/${kind}/${id}/%61bc.png`,
        `https://cdn.discordapp.com/${kind}/${id}\\abc.png`,
        `https://cdn.discordapp.com/${kind}/${id}/abc.png#fragment`,
      ]) {
        lens.open(id, { id, [field]: value });
        assert.equal(fixture.document.querySelector("a"), null, value);
        assert.equal(fixture.document.querySelector("img"), null, value);
        assert.equal(fixture.document.querySelector(`[data-field="${field}"] dd`).textContent, value);
        assert.ok(fixture.document.querySelector(`[data-field="${field}"] button`));
      }
    }
  } finally { lens.dispose(); fixture.restore(); }
});

test("clipboard failure exposes a selected readonly fallback and traps links and textarea until cleanup", async () => {
  const fixture = dom();
  const { document, window } = fixture;
  const restoreClipboard = clipboard(undefined);
  const lens = mountIdentityLens({});
  const key = (name, shiftKey = false) => {
    const event = new window.Event("keydown", { cancelable: true });
    event.key = name;
    event.shiftKey = shiftKey;
    document.dispatchEvent(event);
    return event;
  };
  const originalSelect = window.HTMLTextAreaElement.prototype.select;
  let selected = null;
  window.HTMLTextAreaElement.prototype.select = function () { selected = this; };
  try {
    lens.open(id, { id, avatar: "abc", banner: "def" });
    document.querySelector('[aria-label="Copy User ID"]').click();
    await Promise.resolve();
    const textarea = document.querySelector("textarea");
    assert.equal(textarea.value, id);
    assert.ok(textarea.hasAttribute("readonly"));
    assert.equal(textarea.getAttribute("aria-label"), "User ID to copy");
    assert.equal(selected, textarea);
    assert.equal(document.activeElement, textarea);
    key("Tab");
    assert.equal(document.activeElement.getAttribute("aria-label"), "Copy all JSON");
    key("Tab", true);
    assert.equal(document.activeElement, textarea);
    const controls = [...document.querySelector('[role="dialog"]').querySelectorAll("button, a[href], textarea")];
    controls[0].focus();
    for (let index = 1; index <= controls.length; index++) {
      key("Tab");
      assert.equal(document.activeElement, controls[index % controls.length]);
    }
    document.querySelector("#origin").dispatchEvent(new window.Event("focusin", { bubbles: true }));
    assert.equal(document.activeElement, controls[0]);
    assert.equal(key("Escape").defaultPrevented, true);
    assert.equal(document.querySelector("textarea"), null);
    assert.equal(document.activeElement.id, "origin");
    assert.equal(key("Tab").defaultPrevented, false);
    assert.equal(key("Escape").defaultPrevented, false);
    lens.open(id);
    lens.dispose();
    const outside = document.createElement("button");
    document.body.append(outside);
    outside.focus();
    outside.dispatchEvent(new window.Event("focusin", { bubbles: true }));
    assert.equal(document.activeElement, outside);
    assert.equal(key("Tab").defaultPrevented, false);
  } finally {
    window.HTMLTextAreaElement.prototype.select = originalSelect;
    lens.dispose(); restoreClipboard(); fixture.restore();
  }
});

test("clipboard requests cannot overwrite newer feedback or add fallback to reopened or disposed dialogs", async () => {
  const fixture = dom();
  const pending = [];
  const restoreClipboard = clipboard(() => new Promise((resolve, reject) => pending.push({ resolve, reject })));
  const lens = mountIdentityLens({});
  const copy = label => fixture.document.querySelector(`[aria-label="${label}"]`).click();
  try {
    lens.open(id);
    copy("Copy User ID");
    copy("Copy all JSON");
    pending[1].resolve();
    await Promise.resolve();
    pending[0].reject(new Error("Old failure"));
    await Promise.resolve();
    assert.equal(fixture.document.querySelector('[role="status"]').textContent, "Identity JSON copied");
    assert.equal(fixture.document.querySelector("textarea"), null);
    copy("Copy User ID");
    copy("Copy all JSON");
    pending[3].reject(new Error("New failure"));
    await Promise.resolve();
    const fallback = fixture.document.querySelector("textarea");
    assert.deepEqual(JSON.parse(fallback.value), collectIdentity({ id }));
    pending[2].resolve();
    await Promise.resolve();
    assert.equal(fixture.document.querySelector('[role="status"]').textContent, "Clipboard unavailable");
    assert.equal(fixture.document.querySelector("textarea"), fallback);
    copy("Copy User ID");
    assert.equal(fixture.document.querySelector("textarea"), null);
    lens.open("123");
    pending[4].reject(new Error("Closed request"));
    await Promise.resolve();
    assert.equal(fixture.document.querySelector('[role="status"]').textContent, "");
    assert.equal(fixture.document.querySelector("textarea"), null);
    copy("Copy User ID");
    lens.dispose();
    pending[5].reject(new Error("Disposed request"));
    await Promise.resolve();
    assert.equal(fixture.document.querySelector('[role="dialog"]'), null);
    assert.equal(fixture.document.querySelector("textarea"), null);
  } finally { lens.dispose(); restoreClipboard(); fixture.restore(); }
});