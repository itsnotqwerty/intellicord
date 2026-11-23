import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { build } from "esbuild";
import { backendPreferences, CONTENT_PREFERENCE, SURFACE_PREFERENCES } from "../src/web/preferences.js";
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

test("promoted content popups are suppressed by the sponsored preference and restored", () => {
  const context = fixture(new Map([[CONTENT_PREFERENCE, "true"]]));
  try {
    const host = context.document.createElement("div");
    host.innerHTML = '<div role="dialog" aria-label="Promoted Content" id="promoted"><div class="promotedContent_demo">Promotion</div></div><div class="promotedPopup_demo" id="popup">Promotion</div><div role="dialog" aria-label="Account settings" id="ordinary">Settings</div><div role="alertdialog" class="promotedContent_demo" id="alert">Important notice</div><div class="promotedContent_demo" id="protected"><input aria-label="Permission"></div><div class="promotedUser_demo" id="member">Member</div>';
    context.document.body.append(host);
    context.refresh();
    const hidden = id => context.document.getElementById(id).hasAttribute("data-intellicord-content-hidden");
    assert.equal(hidden("promoted"), true);
    assert.equal(hidden("popup"), true);
    for (const id of ["ordinary", "alert", "protected", "member"]) assert.equal(hidden(id), false);
    context.set(false);
    assert.equal(hidden("promoted"), false);
    assert.equal(hidden("popup"), false);
    context.set(true);
    context.feature.dispose();
    assert.equal(hidden("promoted"), false);
    assert.equal(hidden("popup"), false);
  } finally { context.feature.dispose(); }
});

test("quest filtering hides complete surfaces without stripping controls from unknown layouts", () => {
  const context = fixture(new Map([[CONTENT_PREFERENCE, "true"]]));
  try {
    const host = context.document.createElement("div");
    host.innerHTML = '<div class="questBar_demo" id="bar"><div class="questTitle_demo">Quest</div><button class="questDismiss_demo">Dismiss</button></div><div role="dialog" id="quest-dialog"><div class="questContent_demo">Quest</div><button>Dismiss</button></div><div id="unknown"><div class="questTitle_demo" id="unknown-title">Quest</div><button class="questDismiss_demo" id="unknown-dismiss">Dismiss</button></div><div role="dialog" id="ordinary-dialog"><a href="/quests" id="quest-link">Quests</a><button>Close settings</button></div><div role="dialog" id="protected-dialog"><div class="questContent_demo" id="protected-content">Quest</div><input aria-label="Security"></div>';
    context.document.body.append(host);
    context.refresh();
    const hidden = id => context.document.getElementById(id).hasAttribute("data-intellicord-content-hidden");
    assert.equal(hidden("bar"), true);
    assert.equal(hidden("quest-dialog"), true);
    for (const id of ["unknown", "unknown-title", "unknown-dismiss", "ordinary-dialog", "protected-dialog", "protected-content"]) {
      assert.equal(hidden(id), false, id);
    }
    assert.equal(hidden("quest-link"), true);
    context.set(false);
    assert.equal(hidden("bar"), false);
    assert.equal(hidden("quest-dialog"), false);
  } finally { context.feature.dispose(); }
});

test("saved quest filtering hides the complete live sidebar banner without hiding voice controls", () => {
  const context = fixture(new Map([[CONTENT_PREFERENCE, "true"]]));
  try {
    const host = context.document.createElement("main");
    host.innerHTML = '<div class="questBannerExpanded_demo" id="expanded"><span class="questTitle_demo">Quest</span></div><div class="questPromotionCard_demo" id="promotion">Quest</div><section class="panels_demo" id="panels"><div class="mask_banner" id="banner"><div class="wrapper_banner wrapperVisible_banner"><div class="contentWrapper_banner"><div class="contentCollapsed_demo"></div><div class="contentExpanded_demo"><div><div><div class="questPromoContent_demo" id="promo"><button>Accept</button></div></div></div><div class="rewardHighlightWrapper_demo"><div class="questRewardTileInteractive_demo questRewardTile_demo" id="reward">Reward</div></div><button aria-label="Actions">Actions</button></div><div class="overlay_banner" id="overlay"></div></div></div></div><div class="wrapper_voice" id="voice"><button>Disconnect</button></div><div class="container_demo containerQuestBarVisible_demo" id="status"><button>Mute</button></div></section><div class="wrapper_demo" id="ordinary"><a href="/quests">Quests</a><button aria-label="Close">Close</button></div>';
    context.document.body.append(host);
    context.refresh();
    const hidden = id => context.document.getElementById(id).hasAttribute("data-intellicord-content-hidden");
    for (const id of ["expanded", "promotion", "banner"]) assert.equal(hidden(id), true, id);
    for (const id of ["panels", "promo", "reward", "overlay", "voice", "status", "ordinary"]) assert.equal(hidden(id), false, id);
    assert.equal(host.hasAttribute("data-intellicord-content-hidden"), false);
    context.set(false);
    for (const id of ["expanded", "promotion", "banner"]) assert.equal(hidden(id), false, id);
  } finally { context.feature.dispose(); }
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

function quietFixture(storage) {
  const context = fixture();
  context.feature.dispose();
  const elements = {};
  for (const name of ["promotions", "shop", "decorations", "shared", "security", "essential"]) {
    const node = context.document.createElement("div");
    node.dataset.testSurface = name;
    context.document.body.append(node);
    elements[name] = node;
  }
  const candidates = {
    promotions: [elements.promotions, elements.shared, elements.security],
    shop: [elements.shop, elements.shared, elements.essential],
    decorations: [elements.decorations],
  };
  let refresh;
  const adapter = {
    section: sponsoredContentSection,
    nodes: () => [...sponsoredContentNodes(), elements.shared],
    surfaceNodes: key => candidates[key],
    isProtected: node => node === elements.security || node === elements.essential,
    observe(callback) { refresh = callback; return () => {}; },
  };
  const feature = mountSponsoredContent(adapter, storage ?? context.storage);
  const row = key => context.document.querySelector(`[data-surface="${key}"]`);
  const toggle = key => row(key).querySelector('[role="switch"]');
  function set(key, disabled) {
    toggle(key).checked = disabled;
    toggle(key).dispatchEvent(new window.Event("change"));
  }
  return { ...context, feature, elements, candidates, adapter, row, toggle, set, refresh: () => refresh() };
}

test("surface switches opt in independently and suppress a union without hiding protected notices", () => {
  const context = quietFixture();
  try {
    assert.equal(context.document.querySelectorAll('[role="switch"]').length, 4);
    for (const state of Object.values(context.feature.states)) assert.equal(state.disabled, false);
    const hidden = node => node.hasAttribute("data-intellicord-content-hidden");
    context.set("promotions", true);
    assert.equal(hidden(context.elements.promotions), true);
    assert.equal(hidden(context.elements.shared), true);
    assert.equal(hidden(context.elements.shop), false);
    context.set("shop", true);
    context.set("sponsored", true);
    context.set("promotions", false);
    assert.equal(hidden(context.elements.promotions), false);
    assert.equal(hidden(context.elements.shared), true);
    context.set("shop", false);
    assert.equal(hidden(context.elements.shared), true);
    context.set("sponsored", false);
    assert.equal(hidden(context.elements.shared), false);
    context.set("decorations", true);
    assert.equal(hidden(context.elements.decorations), true);
    assert.equal(hidden(context.elements.security), false);
    assert.equal(hidden(context.elements.essential), false);
    assert.equal(context.storageValues.get(SURFACE_PREFERENCES.decorations), "true");
    assert.equal(context.storageValues.get(SURFACE_PREFERENCES.promotions), "false");
  } finally { context.feature.dispose(); }
});

test("availability is per control, fails open, and can recover after navigation", () => {
  const context = quietFixture();
  try {
    context.set("promotions", true);
    context.candidates.promotions = null;
    context.refresh();
    assert.equal(context.feature.states.promotions.disabled, true);
    assert.equal(context.feature.states.promotions.available, false);
    assert.equal(context.toggle("promotions").disabled, true);
    assert.match(context.row("promotions").textContent, /Unavailable/);
    assert.equal(context.elements.promotions.hasAttribute("data-intellicord-content-hidden"), false);
    assert.equal(context.toggle("shop").disabled, false);
    context.candidates.promotions = [context.elements.promotions];
    context.refresh();
    assert.equal(context.toggle("promotions").disabled, false);
    assert.equal(context.elements.promotions.hasAttribute("data-intellicord-content-hidden"), true);
    context.adapter.surfaceNodes = key => {
      if (key === "shop") throw new Error("shop selector unavailable");
      return context.candidates[key];
    };
    context.refresh();
    assert.match(context.feature.states.shop.error, /shop selector unavailable/);
    assert.equal(context.feature.states.decorations.available, true);
    context.adapter.surfaceNodes = undefined;
    context.refresh();
    assert.equal(context.feature.states.sponsored.available, true);
    assert.equal(context.feature.states.decorations.available, false);
  } finally { context.feature.dispose(); }
});

test("attribute restoration preserves originals on selector changes, removal, and detach", () => {
  const context = quietFixture();
  const attribute = "data-intellicord-content-hidden";
  try {
    context.elements.promotions.setAttribute(attribute, "original");
    context.elements.shared.setAttribute(attribute, "shared-original");
    context.set("promotions", true);
    context.set("shop", true);
    assert.equal(context.elements.promotions.getAttribute(attribute), "");
    context.candidates.promotions = [context.elements.shared];
    context.refresh();
    assert.equal(context.elements.promotions.getAttribute(attribute), "original");
    context.elements.shared.remove();
    context.refresh();
    assert.equal(context.elements.shared.getAttribute(attribute), "shared-original");
    context.elements.decorations.setAttribute(attribute, "decoration-original");
    context.set("decorations", true);
    context.feature.dispose();
    assert.equal(context.elements.decorations.getAttribute(attribute), "decoration-original");
    assert.equal(context.elements.shop.hasAttribute(attribute), false);
  } finally { context.feature.dispose(); }
});

test("backend controls load and save independently and scope errors to the failed switch", async () => {
  const requests = [];
  const storage = backendPreferences(payload => requests.push(JSON.parse(payload)));
  const context = quietFixture(storage);
  const response = { disabled: false, promotions: false, shop: false, decorations: false, error: null };
  try {
    for (const key of Object.keys(SURFACE_PREFERENCES)) {
      assert.equal(context.feature.states[key].loading, true);
      assert.equal(context.toggle(key).disabled, true);
      assert.match(context.row(key).textContent, /Loading/);
    }
    const loaded = storage.load();
    storage.update(response);
    await loaded;
    context.set("promotions", true);
    context.set("shop", true);
    assert.equal(context.feature.states.promotions.saving, true);
    assert.equal(context.feature.states.shop.saving, true);
    assert.equal(context.toggle("decorations").disabled, false);
    assert.equal(context.elements.promotions.hasAttribute("data-intellicord-content-hidden"), false);
    storage.update({ ...response, promotions: true });
    await Promise.resolve();
    assert.equal(context.feature.states.promotions.saving, false);
    assert.equal(context.feature.states.shop.saving, true);
    assert.equal(context.elements.promotions.hasAttribute("data-intellicord-content-hidden"), true);
    assert.deepEqual(requests.at(-1), { kind: "preferences_set", key: "shop", disabled: true });
    storage.update({ ...response, promotions: true, error: "shop save failed; setting unchanged" });
    await Promise.resolve();
    assert.equal(context.feature.states.shop.saving, false);
    assert.equal(context.toggle("shop").checked, false);
    assert.match(context.feature.states.shop.error, /shop save failed/);
    assert.equal(context.feature.states.promotions.error, null);
    context.set("decorations", true);
    storage.update({ ...response, promotions: true, decorations: true });
    await Promise.resolve();
    assert.equal(context.feature.states.decorations.disabled, true);
    assert.match(context.feature.states.shop.error, /shop save failed/);
    context.set("shop", true);
    storage.update({ ...response, promotions: true, decorations: true, shop: true });
    await Promise.resolve();
    assert.equal(context.feature.states.shop.error, null);
  } finally { context.feature.dispose(); storage.dispose(); }
});

test("storage events synchronize individual surfaces and clear events reset all switches", () => {
  const context = quietFixture();
  function sync(key) {
    const event = new window.Event("storage");
    event.key = key;
    window.dispatchEvent(event);
  }
  try {
    context.storageValues.set(SURFACE_PREFERENCES.promotions, "true");
    sync(SURFACE_PREFERENCES.promotions);
    assert.equal(context.feature.states.promotions.disabled, true);
    assert.equal(context.feature.states.shop.disabled, false);
    context.storageValues.set(SURFACE_PREFERENCES.shop, "true");
    sync("unrelated");
    assert.equal(context.feature.states.shop.disabled, false);
    sync(SURFACE_PREFERENCES.shop);
    assert.equal(context.feature.states.shop.disabled, true);
    context.storageValues.clear();
    sync(null);
    for (const state of Object.values(context.feature.states)) assert.equal(state.disabled, false);
    assert.equal(context.document.querySelectorAll("[data-intellicord-content-hidden]").length, 0);
  } finally { context.feature.dispose(); }
});