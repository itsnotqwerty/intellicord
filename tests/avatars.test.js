import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { profilePicture, safeAvatarUrl } from "../src/web/avatars.js";

test("avatar URLs accept only Discord profile image paths", () => {
  const valid = "https://cdn.discordapp.com/avatars/9007199254740993/a_abcdef.gif?size=64";
  assert.equal(safeAvatarUrl(valid), valid);
  assert.ok(safeAvatarUrl("https://cdn.discordapp.com/embed/avatars/0.png"));
  for (const url of ["https://evil.test/avatars/1/a.png", "javascript:alert(1)", "https://cdn.discordapp.com.evil.test/avatars/1/a.png", "https://cdn.discordapp.com/attachments/1/a.png", "https://user@cdn.discordapp.com/avatars/1/a.png"]) assert.equal(safeAvatarUrl(url), null);
});

test("profile pictures retain initials until loaded and fall back on failure", () => {
  const { document, window } = parseHTML("<html><body></body></html>");
  globalThis.document = document;
  const user = { display_name: "Morgan", avatar_url: "https://cdn.discordapp.com/avatars/1/abcdef.png" };
  const avatar = profilePicture(user);
  const image = avatar.querySelector("img");
  assert.equal(avatar.textContent, "MO");
  assert.equal(image.getAttribute("referrerpolicy") || image.referrerPolicy, "no-referrer");
  image.dispatchEvent(new window.Event("load"));
  assert.equal(avatar.textContent, "");
  assert.equal(avatar.firstChild, image);
  const failed = profilePicture(user);
  failed.querySelector("img").dispatchEvent(new window.Event("error"));
  assert.equal(failed.textContent, "MO");
  assert.equal(failed.querySelector("img"), null);
});