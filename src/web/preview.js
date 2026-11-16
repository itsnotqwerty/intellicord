import { mountMenu } from "./menu.js";
import { observeCallView, observeClientDom, sponsoredContentNodes, sponsoredContentSection } from "./bridge.js";
import { mountSponsoredContent } from "./sponsored.js";
import { createElement, Settings, ArrowLeft } from "lucide";

const now = Date.now();
const channel = { id: "111111111111111111", name: "General" };
const people = [
  { id: "9007199254740993", username: "morgan", display_name: "Morgan" },
  { id: "222222222222222222", username: "jules", display_name: "Jules" },
  { id: "333333333333333333", username: "alex.chen", display_name: "Alex Chen" },
  { id: "444444444444444444", username: "riley", display_name: "Riley" },
].map((user, index) => ({ ...user, avatar_url: `https://cdn.discordapp.com/embed/avatars/${index}.png` }));
const state = {
  channel,
  participants: Object.fromEntries(people.slice(0, 3).map(user => [user.id, user])),
  events: [0, 1, 2, 3, 3, 1].map((person, index) => ({
    sequence: index + 1, timestamp: now - (6 - index) * 60000,
    action: index === 4 || index === 5 ? "left" : "joined", user: people[person], channel,
  })),
};
const mutedUsers = new Set();
let contextMenu = null;
function closeContextMenu() { contextMenu?.remove(); contextMenu = null; }
document.addEventListener("pointerdown", event => {
  if (contextMenu && !contextMenu.contains(event.target)) closeContextMenu();
});
document.addEventListener("keydown", event => {
  if (event.key === "Escape") closeContextMenu();
});
const menu = mountMenu(payload => {
  if (JSON.parse(payload).kind === "clear") { state.events = []; menu.update(state); }
}, {
  open(event, id) {
    closeContextMenu();
    contextMenu = document.createElement("div");
    contextMenu.className = "preview-context-menu";
    contextMenu.setAttribute("role", "menu");
    contextMenu.setAttribute("aria-label", "User actions");
    for (const label of ["Profile", "Message"]) {
      const item = document.createElement("button");
      item.type = "button";
      item.textContent = label;
      item.disabled = true;
      item.setAttribute("role", "menuitem");
      contextMenu.append(item);
    }
    const mute = document.createElement("button");
    mute.type = "button";
    mute.textContent = "Mute";
    mute.setAttribute("role", "menuitemcheckbox");
    mute.setAttribute("aria-checked", String(mutedUsers.has(id)));
    mute.disabled = id === people[0].id || !state.participants[id];
    mute.addEventListener("click", () => {
      if (mutedUsers.has(id)) mutedUsers.delete(id);
      else mutedUsers.add(id);
      closeContextMenu();
    });
    const copy = document.createElement("button");
    copy.type = "button";
    copy.textContent = "Copy User ID";
    copy.setAttribute("role", "menuitem");
    copy.addEventListener("click", async () => {
      try { await navigator.clipboard.writeText(id); closeContextMenu(); }
      catch { copy.textContent = id; }
    });
    contextMenu.append(mute, copy);
    document.body.append(contextMenu);
    const rect = event.currentTarget.getBoundingClientRect();
    contextMenu.style.left = `${Math.max(8, Math.min(event.pageX || rect.left, window.innerWidth - 208))}px`;
    contextMenu.style.top = `${Math.max(8, Math.min(event.pageY || rect.top, window.innerHeight - contextMenu.offsetHeight - 8))}px`;
    mute.focus();
  },
}, observeCallView);
menu.update(state);
menu.connection(true);
document.querySelector(".cm-launcher").click();
document.querySelector("#simulate-join").addEventListener("click", () => {
  state.events.push({ sequence: (state.events.at(-1)?.sequence ?? 0) + 1,
    timestamp: Date.now(), action: "joined", user: people[0], channel });
  menu.update(state);
});
let storage;
try { storage = localStorage; } catch {}
const sponsoredContent = mountSponsoredContent({ section: sponsoredContentSection, nodes: sponsoredContentNodes, observe: observeClientDom }, storage);
const settingsButton = document.querySelector("#preview-settings");
settingsButton.append(createElement(Settings, { width: 18, height: 18, "aria-hidden": "true" }));
const backButton = document.querySelector("#preview-back");
backButton.append(createElement(ArrowLeft, { width: 18, height: 18, "aria-hidden": "true" }));
function showSettings(value) {
  document.querySelector("#preview-call").hidden = value;
  document.querySelector("#preview-privacy").hidden = !value;
  document.querySelector("main").toggleAttribute("data-intellicord-call-view", !value);
}
settingsButton.addEventListener("click", () => showSettings(true));
backButton.addEventListener("click", () => showSettings(false));
globalThis.callmonPreview = { menu, state, mutedUsers, sponsoredContent };