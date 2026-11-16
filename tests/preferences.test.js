import assert from "node:assert/strict";
import test from "node:test";
import { CONTENT_PREFERENCE, readContentPreference, writeContentPreference } from "../src/web/preferences.js";

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