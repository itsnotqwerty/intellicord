import { createUserContextMenu, createObliterateAdapter, obliterateMenuContext, discoverClientStores, observeCall, observeCallView, observeClientDom, sponsoredContentNodes, sponsoredContentSection, quietSurfaceNodes, settingsSidebar, resolveUserCall, voiceStatusLabels, nativeLinkMenuAnchor } from "./bridge.js";
import { mountMenu } from "./menu.js";
import { mountSponsoredContent } from "./sponsored.js";
import { backendPreferences } from "./preferences.js";
import { mountDiscordLookup } from "./lookup.js";
import { mountIdentityLens } from "./identity.js";
import { createVoiceWatch } from "./watch.js";
import { mountVoiceWatch } from "./watch-panel.js";
import { mountObliteratePanel } from "./obliterate-panel.js";
import { mountLinkInspector } from "./links.js";
import { mountSessionHealth } from "./health.js";
import { mountVoiceStatus } from "./voice-status.js";
import { userToolTheme } from "./user-tool-theme.js";
import { createProgressHandoff, ownerFeature, progressFeature } from "./process-progress.js";
import { acousticFeature } from "./acoustic-status.js";
import { sessionSensorsFeature } from "./session-sensors.js";
import { sessionTimingFeature } from "./session-timing.js";

if (globalThis.__intellicord || globalThis.__callmon) throw new Error("Intellicord is already attached. Stop the existing injector before attaching again.");
if (typeof globalThis.__callmon_send !== "function") throw new Error("Intellicord's Rust binding is missing.");
const send = payload => globalThis.__callmon_send(payload);
const startedAt = Date.now();
let lastHeartbeat = startedAt;
let lastSnapshot = null;
const features = {};
features["Sensor session"] = { state: "waiting", detail: "Waiting for combined owner status" };
features["Session timing"] = { state: "waiting", detail: "Waiting for backend clock inventory" };
features["Process observation"] = { state: "waiting", detail: "Waiting for backend owner status" };
features["Process telemetry"] = { state: "waiting", detail: "Waiting for automatic process telemetry" };
features["Output analysis"] = { state: "waiting", detail: "Waiting for backend acoustic status" };
const processProgress = createProgressHandoff();
const mounted = [];
let disposed = false;
function mount(name, factory) {
  try {
    const value = factory();
    features[name] = { state: "ready", detail: "Mounted; private client APIs may vary by build" };
    mounted.push(value);
    return value;
  } catch (cause) {
    features[name] = { state: "error", detail: cause.message || "Failed to initialize" };
    return null;
  }
}
const stores = discoverClientStores(globalThis.webpackChunkdiscord_app);
const storage = backendPreferences(send);
storage.subscribe(value => {
  features["Preferences"] = {state: value.error ? "error" : "ready", detail: value.error || "Backend preferences loaded"};
});
const identity = stores && mount("Identity Lens", () => mountIdentityLens(stores));
const watchNotice = document.createElement("div");
watchNotice.className = "ic-user-tool";
const watchStyle = document.createElement("style");
watchStyle.textContent = userToolTheme;
document.head.append(watchStyle);
mounted.push({dispose: () => watchStyle.remove()});
watchNotice.dataset.intellicordOwned = "";
watchNotice.setAttribute("role", "status");
watchNotice.hidden = true;
watchNotice.style.cssText = "position:fixed;right:16px;bottom:16px;z-index:1001;max-width:min(340px,calc(100vw - 32px));padding:12px;background:var(--ic-tool-secondary);color:var(--ic-tool-text);border:1px solid var(--ic-tool-border);border-radius:4px;overflow-wrap:anywhere;font:14px var(--font-primary,sans-serif)";
document.body.append(watchNotice);
let noticeTimer;
const watch = stores && mount("Voice Watch", () => createVoiceWatch(stores, event => {
  watchNotice.textContent = `${event.user.display_name || event.user.id} ${event.action} ${stores.channels.getChannel(event.channel.id)?.name || "the current call"}`;
  watchNotice.hidden = false;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => { watchNotice.hidden = true; }, 6000);
}, cause => { features["Voice Watch"] = { state: "error", detail: cause.message }; }));
const watchPanel = watch && mount("Voice Watch controls", () => mountVoiceWatch(watch, stores));
const menu = stores && mount("Callmon", () => mountMenu(send, createUserContextMenu(stores), observeCallView, {identity, watch: watchPanel}));
const observation = stores && menu && mount("Voice capture", () => {
  const stop = observeCall(stores, payload => { lastSnapshot = Date.now(); send(payload); }, message => {
    menu.error(message);
    features["Voice capture"] = { state: message ? "error" : "ready", detail: message || "Current-call snapshots available" };
  });
  return {dispose: stop};
});
const sponsoredContent = mount("Quiet Surfaces", () => mountSponsoredContent({ section: sponsoredContentSection, nodes: sponsoredContentNodes, surfaceNodes: quietSurfaceNodes, observe: observeClientDom }, storage));
const obliterateAdapter = mount("Obliterate actions", () => createObliterateAdapter(stores));
const obliterate = obliterateAdapter && mount("Obliterate confirmation", () => mountObliteratePanel(obliterateAdapter));
const discordLookup = mount("Native user menus", () => mountDiscordLookup({identity, watch: watchPanel,
  obliterate: obliterate && { context: obliterateMenuContext, open: context => obliterate.open(context) }}));
const links = mount("Link Inspector", () => mountLinkInspector({menuAnchor: nativeLinkMenuAnchor}));
const voiceStatus = stores && mount("Resolved voice status", () => mountVoiceStatus({
  labels: voiceStatusLabels,
  resolve: id => resolveUserCall(stores, id),
  observe: observeClientDom,
  subscribe(refresh) {
    stores.voice.addChangeListener(refresh);
    stores.selected.addChangeListener(refresh);
    return () => { stores.voice.removeChangeListener(refresh); stores.selected.removeChangeListener(refresh); };
  },
}));
const health = mount("Session Health", () => mountSessionHealth({sidebar: settingsSidebar, observe: observeClientDom}, () => {
  const snapshot = {...features};
  snapshot["Process telemetry"] = progressFeature(processProgress.read());
  snapshot["Preferences"] = features["Preferences"] || {state: "waiting", detail: "Waiting for backend acknowledgement"};
  for (const [key, value] of Object.entries(sponsoredContent?.states || {})) {
    snapshot[`Quiet Surfaces: ${key}`] = {state: value.error ? "error" : !value.available ? "unsupported" : value.loading || value.saving ? "waiting" : "ready", detail: value.error || (!value.available ? "Surface selectors unavailable" : value.disabled ? "Suppression enabled" : "Suppression off")};
  }
  return {startedAt,lastHeartbeat,lastSnapshot,features:snapshot};
}));
menu?.connection(true);
const timer = setInterval(() => menu?.connection(Date.now() - lastHeartbeat < 15000), 2000);
globalThis.__intellicord = {
  callmon: menu,
  diagnostics: menu?.diagnostics,
  sponsoredContent,
  discordLookup,
  identity,
  watch,
  watchPanel,
  obliterate,
  links,
  voiceStatus,
  health,
  preferences: storage,
  update(value) {
    const session = value?.intelligence;
    const current = session?.schema_version === 1 && typeof session.session_id === "string"
      && session.session_id.length > 0 && session.session_id.length <= 512;
    features["Sensor session"] = sessionSensorsFeature(current ? session.session_sensors : null);
    features["Session timing"] = sessionTimingFeature(current ? session.session_timing : null);
    features["Process observation"] = ownerFeature(current ? session.process_owner : null);
    processProgress.accept(current ? session.process_progress : null, performance.now());
    features["Output analysis"] = acousticFeature(current ? session.acoustic : undefined,
      current ? session.acoustic_owner : undefined);
    menu?.update(value);
  },
  heartbeat() { lastHeartbeat = Date.now(); menu?.connection(true); },
  dispose() {
    if (disposed) return;
    disposed = true;
    clearInterval(timer);
    clearTimeout(noticeTimer);
    processProgress.clear();
    watchNotice.remove();
    for (const feature of mounted.reverse()) feature?.dispose?.();
    storage.dispose();
    delete globalThis.__intellicord;
    delete globalThis.__callmon;
  },
};
globalThis.__callmon = globalThis.__intellicord;
storage.load().catch(cause => { features["Preferences"] = {state: "error", detail: cause.message}; });