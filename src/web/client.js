import { createUserContextMenu, discoverClientStores, observeCall, observeCallView, observeClientDom, sponsoredContentNodes, sponsoredContentSection } from "./bridge.js";
import { mountMenu } from "./menu.js";
import { mountSponsoredContent } from "./sponsored.js";
import { backendPreferences } from "./preferences.js";
import { mountDiscordLookup } from "./lookup.js";

if (globalThis.__intellicord || globalThis.__callmon) throw new Error("Intellicord is already attached. Stop the existing injector before attaching again.");
if (typeof globalThis.__callmon_send !== "function") throw new Error("Intellicord's Rust binding is missing.");
const stores = discoverClientStores(globalThis.webpackChunkdiscord_app);
const send = payload => globalThis.__callmon_send(payload);
const menu = mountMenu(send, createUserContextMenu(stores), observeCallView);
let stop;
let sponsoredContent;
let discordLookup;
const storage = backendPreferences(send);
try {
  stop = observeCall(stores, send, message => menu.error(message));
  sponsoredContent = mountSponsoredContent({ section: sponsoredContentSection, nodes: sponsoredContentNodes, observe: observeClientDom }, storage);
  discordLookup = mountDiscordLookup();
} catch (error) { stop?.(); discordLookup?.dispose(); sponsoredContent?.dispose(); storage.dispose(); menu.dispose(); throw error; }
let lastHeartbeat = Date.now();
menu.connection(true);
const timer = setInterval(() => menu.connection(Date.now() - lastHeartbeat < 15000), 2000);
globalThis.__intellicord = {
  callmon: menu,
  sponsoredContent,
  discordLookup,
  preferences: storage,
  update: value => menu.update(value),
  heartbeat() { lastHeartbeat = Date.now(); menu.connection(true); },
  dispose() {
    stop();
    clearInterval(timer);
    menu.dispose();
    sponsoredContent.dispose();
    discordLookup.dispose();
    storage.dispose();
    delete globalThis.__intellicord;
    delete globalThis.__callmon;
  },
};
globalThis.__callmon = globalThis.__intellicord;
storage.load().catch(() => {});