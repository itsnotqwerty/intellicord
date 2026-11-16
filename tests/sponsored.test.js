import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { build } from "esbuild";
import { backendPreferences, CONTENT_PREFERENCE } from "../src/web/preferences.js";
import { sponsoredContentNodes, sponsoredContentSection } from "../src/web/bridge.js";

const bundle = await build({ entryPoints: ["src/web/sponsored.js"], bundle: true, write: false, format: "esm", loader: { ".css": "text" } });
const { mountSponsoredContent } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);

function fixture(storageValues = new Map()) {
  const { window, document } = parseHTML('<html><head></head><body><div data-nav-anchor-key="sponsored_content_category"><h2>Sponsored Content</h2><div class="sponsoredSetting_native"><input type="checkbox"></div></div><a href="/quests">Quests</a><div class="questCard_demo">Quest</div><div class="sponsoredCard_demo">Sponsor</div><div class="requestCard_demo">Friend request</div></body></html>');
  globalThis.window = window;
  globalThis.document = document;
  let refresh;
  let stopped = false;
  const storage = { getItem: key => storageValues.get(key) ?? null, setItem: (key, value) => storageValues.set(key, value) };
  const feature = mountSponsoredContent({ section: sponsoredContentSection, nodes: sponsoredContentNodes, observe(callback) { refresh = callback; callback(); return () => { stopped = true; }; } }, storage);
  const toggle = document.querySelector('[aria-label="Disable Quests and Sponsored Content"]');
  function set(value) { toggle.checked = value; toggle.dispatchEvent(new window.Event("change")); }
  return { feature, document, toggle, set, refresh: () => refresh(), stopped: () => stopped, storageValues, storage };
}

test("master switch mounts at the section head and excludes native privacy controls", () => {
  const context = fixture();
  assert.equal(sponsoredContentSection().firstElementChild.className, "ic-content-setting");
  assert.equal(context.feature.disabled, false);
  assert.equal(context.document.querySelectorAll('[data-intellicord-content-hidden]').length, 0);
  context.set(true);
  assert.equal(context.document.querySelectorAll('[data-intellicord-content-hidden]').length, 3);
  assert.equal(context.document.querySelector('.sponsoredSetting_native').hasAttribute('data-intellicord-content-hidden'), false);
  assert.equal(context.document.querySelector('.requestCard_demo').hasAttribute('data-intellicord-content-hidden'), false);
  assert.equal(context.storageValues.get(CONTENT_PREFERENCE), "true");
  context.set(false);
  assert.equal(context.document.querySelectorAll('[data-intellicord-content-hidden]').length, 0);
  context.feature.dispose();
});

test("saved filtering covers new elements, survives settings navigation, and reverses on detach", () => {
  const context = fixture(new Map([[CONTENT_PREFERENCE, "true"]]));
  const oldSection = sponsoredContentSection();
  oldSection.remove();
  context.refresh();
  assert.equal(context.document.querySelector('.ic-content-setting'), null);
  const section = context.document.createElement("div");
  section.setAttribute("data-nav-anchor-key", "sponsored_content_category");
  context.document.body.append(section);
  const newQuest = context.document.createElement("div");
  newQuest.className = "questTile_demo";
  context.document.body.append(newQuest);
  context.refresh();
  assert.equal(section.firstElementChild.className, "ic-content-setting");
  assert.equal(newQuest.hasAttribute('data-intellicord-content-hidden'), true);
  context.feature.dispose();
  assert.equal(context.stopped(), true);
  assert.equal(context.document.querySelectorAll('[data-intellicord-content-hidden], [data-intellicord-owned]').length, 0);
  assert.equal(context.storageValues.get(CONTENT_PREFERENCE), "true");
});

test("failed preference writes do not change suppression state", () => {
  const context = fixture();
  context.storage.setItem = () => { throw new Error("denied"); };
  context.set(true);
  assert.equal(context.feature.disabled, false);
  assert.equal(context.toggle.checked, false);
  assert.match(context.document.querySelector('[role="status"]').textContent, /Unable to save/);
  context.feature.dispose();
});

test("initialization failure restores every filtered element", () => {
  const context = fixture();
  context.feature.dispose();
  context.storageValues.set(CONTENT_PREFERENCE, "true");
  assert.throws(() => mountSponsoredContent({ section: sponsoredContentSection, nodes: sponsoredContentNodes, observe(refresh) {
    refresh();
    throw new Error("observer unavailable");
  } }, context.storage), /observer unavailable/);
  assert.equal(context.document.querySelectorAll('[data-intellicord-content-hidden], [data-intellicord-owned]').length, 0);
});

test("backend saves wait for acknowledgement and restore prior filtering on failure", async () => {
  const context = fixture();
  context.feature.dispose();
  const requests = [];
  const preferences = backendPreferences(payload => requests.push(JSON.parse(payload)));
  const feature = mountSponsoredContent({ section: sponsoredContentSection, nodes: sponsoredContentNodes, observe: refresh => { refresh(); return () => {}; } }, preferences);
  const toggle = context.document.querySelector('[role="switch"]');
  try {
    assert.equal(toggle.disabled, true);
    const loaded = preferences.load();
    preferences.update({ disabled: false, error: null });
    await loaded;
    assert.equal(toggle.disabled, false);
    toggle.checked = true;
    toggle.dispatchEvent(new window.Event("change"));
    assert.deepEqual(requests.at(-1), { kind: "preferences_set", disabled: true });
    assert.equal(toggle.disabled, true);
    assert.equal(feature.disabled, false);
    preferences.update({ disabled: true, error: null });
    await Promise.resolve();
    assert.equal(feature.disabled, true);
    assert.equal(toggle.disabled, false);
    toggle.checked = false;
    toggle.dispatchEvent(new window.Event("change"));
    preferences.update({ disabled: true, error: "Unable to save Intellicord preferences; setting unchanged" });
    await Promise.resolve();
    assert.equal(feature.disabled, true);
    assert.equal(toggle.checked, true);
    assert.equal(toggle.disabled, false);
    assert.match(context.document.querySelector('[role="status"]').textContent, /setting unchanged/);
  } finally { feature.dispose(); preferences.dispose(); }
});