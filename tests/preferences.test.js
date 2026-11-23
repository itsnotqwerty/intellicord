import assert from "node:assert/strict";
import test from "node:test";
import { backendPreferences, CONTENT_PREFERENCE, SURFACE_PREFERENCES, readContentPreference, writeContentPreference } from "../src/web/preferences.js";

test("sponsored filtering is opt-in and stores only one boolean", () => {
  const values = new Map();
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
  assert.deepEqual(readContentPreference(storage), { disabled: false, error: null });
  writeContentPreference(storage, true);
  assert.deepEqual([...values], [[CONTENT_PREFERENCE, "true"]]);
  assert.equal(readContentPreference(storage).disabled, true);
  writeContentPreference(storage, false);
  assert.equal(readContentPreference(storage).disabled, false);
  assert.throws(() => writeContentPreference(storage, "false"), TypeError);
});

test("malformed or unavailable storage cannot silently enable filtering", () => {
  assert.equal(readContentPreference({ getItem: () => '{"enabled":true}' }).disabled, false);
  const storage = { getItem() { throw new Error("denied"); }, setItem() { throw new Error("denied"); } };
  assert.deepEqual(readContentPreference(storage), { disabled: false, error: "Preference storage unavailable" });
  assert.throws(() => writeContentPreference(storage, true), /denied/);
});

test("all surface storage keys are separate and opt-in", () => {
  const values = new Map();
  const storage = { getItem: key => values.get(key), setItem: (key, value) => values.set(key, value) };
  for (const key of Object.values(SURFACE_PREFERENCES)) {
    assert.equal(readContentPreference(storage, key).disabled, false);
    writeContentPreference(storage, true, key);
  }
  writeContentPreference(storage, false, SURFACE_PREFERENCES.shop);
  assert.equal(readContentPreference(storage, SURFACE_PREFERENCES.shop).disabled, false);
  assert.equal(readContentPreference(storage, SURFACE_PREFERENCES.promotions).disabled, true);
  assert.equal(new Set(Object.values(SURFACE_PREFERENCES)).size, 4);
  assert.throws(() => writeContentPreference(storage, true, "unknown"), /Unsupported/);
});

test("backend defaults old responses and serializes independently acknowledged surface saves", async () => {
  const sent = [];
  const storage = backendPreferences(payload => sent.push(JSON.parse(payload)));
  const notifications = [];
  storage.subscribe((state, request) => notifications.push({ ...request }));
  try {
    const loaded = storage.load();
    storage.update({ disabled: true, error: null });
    await loaded;
    for (const surface of ["promotions", "shop", "decorations"]) assert.equal(storage.getItem(SURFACE_PREFERENCES[surface]), "false");
    const promotions = storage.setItem(SURFACE_PREFERENCES.promotions, "true");
    const shop = storage.setItem(SURFACE_PREFERENCES.shop, "true");
    assert.deepEqual(sent.at(-1), { kind: "preferences_set", key: "promotions", disabled: true });
    assert.equal(sent.length, 2);
    storage.update({ disabled: true, promotions: true, shop: false, decorations: false, error: null });
    await promotions;
    assert.deepEqual(sent.at(-1), { kind: "preferences_set", key: "shop", disabled: true });
    const failed = assert.rejects(shop, /write failed/);
    storage.update({ disabled: true, promotions: true, shop: false, decorations: false, error: "write failed" });
    await failed;
    assert.equal(storage.getItem(SURFACE_PREFERENCES.promotions), "true");
    assert.equal(storage.getItem(SURFACE_PREFERENCES.shop), "false");
    assert.deepEqual(notifications.at(-1), { key: "shop", error: "write failed" });
    const sponsored = storage.setItem(CONTENT_PREFERENCE, "false");
    assert.deepEqual(sent.at(-1), { kind: "preferences_set", disabled: false });
    storage.update({ disabled: false, promotions: true, error: null });
    await sponsored;
  } finally { storage.dispose(); }
});

test("unsupported acknowledgements and malformed fields cannot enable a surface", async () => {
  const storage = backendPreferences(() => {});
  try {
    storage.update({ disabled: false, promotions: "true", error: null });
    assert.equal(storage.ready, false);
    const saved = storage.setItem(SURFACE_PREFERENCES.decorations, "true");
    const rejected = assert.rejects(saved, /did not acknowledge/);
    storage.update({ disabled: false, error: null });
    await rejected;
    assert.equal(storage.getItem(SURFACE_PREFERENCES.decorations), "false");
    assert.throws(() => storage.setItem("unknown", "true"), /Unsupported/);
  } finally { storage.dispose(); }
});

test("detachment rejects active and queued requests", async () => {
  const storage = backendPreferences(() => {});
  const active = assert.rejects(storage.setItem(SURFACE_PREFERENCES.promotions, "true"), /detached/);
  const queued = assert.rejects(storage.setItem(SURFACE_PREFERENCES.shop, "true"), /detached/);
  storage.dispose();
  await Promise.all([active, queued]);
  await assert.rejects(storage.load(), /detached/);
});