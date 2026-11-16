import { safeAvatarUrl } from "./avatars.js";
import { parse } from "acorn";
import { simple } from "acorn-walk";

export function sponsoredContentSection() {
  return document.querySelector('[data-nav-anchor-key="sponsored_content_category"]');
}

export function sponsoredContentNodes() {
  const candidates = document.querySelectorAll('[class*="quest" i], [class*="sponsor" i], [aria-label*="quest" i], [aria-label*="sponsored" i], [data-list-item-id*="quest" i], a[href*="/quests"], a[href*="/quest-home"]');
  return [...candidates].filter(node => {
    if (node.closest('[data-intellicord-owned], [data-nav-anchor-key="sponsored_content_category"], [data-settings-sidebar-item]')) return false;
    return [...node.classList].some(name => /^(?:[Qq]uests?|[Ss]ponsored)(?:[A-Z_])/.test(name)) ||
      /^(?:Quests?|Sponsored Content)(?:$|\s)/i.test(node.getAttribute("aria-label") || "") ||
      /(?:^|[_-])quests?(?:$|[_-])/i.test(node.getAttribute("data-list-item-id") || "") ||
      /\/(?:quests|quest-home)(?:\/|$|\?)/.test(node.getAttribute("href") || "");
  });
}

export function observeClientDom(refresh) {
  let frame = null;
  const schedule = () => {
    if (frame === null) frame = requestAnimationFrame(() => { frame = null; refresh(); });
  };
  const observer = new MutationObserver(records => {
    if (records.some(record => !record.target.closest?.('[data-intellicord-owned], #callmon-root'))) schedule();
  });
  observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["class", "style", "hidden", "aria-selected"] });
  window.addEventListener("resize", schedule);
  document.addEventListener("scroll", schedule, true);
  refresh();
  return () => {
    observer.disconnect();
    if (frame !== null) cancelAnimationFrame(frame);
    window.removeEventListener("resize", schedule);
    document.removeEventListener("scroll", schedule, true);
  };
}

export function observeCallView(update) {
  let observedView = null;
  const sizes = typeof ResizeObserver === "function" ? new ResizeObserver(() => refresh()) : null;
  function refresh() {
    const settingsOpen = [...document.querySelectorAll('[data-settings-sidebar-item]')].some(node => node.getClientRects().length);
    const view = settingsOpen ? null : [...document.querySelectorAll('[class*="callContainer_"], [data-intellicord-call-view]')]
      .find(node => node.getClientRects().length && node.getBoundingClientRect().width > 0 && !node.closest('[hidden]'));
    if (view !== observedView) {
      sizes?.disconnect();
      if (view) sizes?.observe(view);
      observedView = view;
    }
    update(view?.getBoundingClientRect() ?? null);
  }
  const stop = observeClientDom(refresh);
  return () => { sizes?.disconnect(); stop(); };
}

export function discoverStores(require) {
  if (!require?.c) throw new Error("Callmon: Discord's module cache is unavailable. This client build is not supported.");
  const candidates = [];
  for (const module of Object.values(require.c)) {
    try {
      let exports = module?.exports;
      if (!exports) continue;
      const asyncExports = Object.getOwnPropertySymbols(exports).find(symbol =>
        symbol.description === "rspack exports" || symbol.description === "webpack exports");
      if (asyncExports) exports = exports[asyncExports];
      if (!exports) continue;
      candidates.push(exports, ...Object.values(exports));
    } catch {}
  }
  const find = (storeName, ...methods) => {
    const matches = candidates.filter(candidate => {
      try {
        return candidate && methods.every(method => method in Object(candidate) && typeof candidate[method] === "function");
      } catch { return false; }
    });
    return matches.find(candidate => {
      try { return typeof candidate.getName === "function" && candidate.getName() === storeName; }
      catch { return false; }
    }) || matches[0];
  };
  const voice = find("VoiceStateStore", "getVoiceStatesForChannel", "addChangeListener", "removeChangeListener");
  const selected = find("SelectedChannelStore", "getVoiceChannelId", "addChangeListener", "removeChangeListener");
  const users = find("UserStore", "getUser");
  const channels = find("ChannelStore", "getChannel");
  const media = find("MediaEngineStore", "isLocalMute", "addChangeListener", "removeChangeListener");
  const muteActions = find("MediaEngineActions", "toggleLocalMute");
  if (!voice || !selected || !users || !channels) {
    throw new Error("Callmon: required voice/user/channel stores were not found. No listeners were installed.");
  }
  return { voice, selected, users, channels, ...(media && muteActions ? { media, muteActions } : {}) };
}

export function createLocalMute(stores) {
  if (!stores.media || !stores.muteActions) return null;
  function unavailableReason(userId) {
    try {
      const currentUser = stores.users.getCurrentUser?.();
      if (!currentUser?.id) return "Current user unavailable";
      if (userId === currentUser.id) return "You cannot locally mute yourself";
      const channelId = stores.selected.getVoiceChannelId();
      const states = channelId ? stores.voice.getVoiceStatesForChannel(channelId) : {};
      if (!channelId || !Object.values(states || {}).some(state => state.userId === userId)) return "User is not in the current call";
      return null;
    } catch { return "Local mute temporarily unavailable"; }
  }
  return {
    unavailableReason,
    isMuted(userId) {
      try { return Boolean(stores.media.isLocalMute(userId)); }
      catch { return false; }
    },
    toggle(userId) {
      const reason = unavailableReason(userId);
      if (reason) throw new Error(reason);
      return stores.muteActions.toggleLocalMute(userId);
    },
    subscribe(listener) {
      stores.media.addChangeListener(listener);
      return () => stores.media.removeChangeListener(listener);
    },
  };
}

export function discoverClientStores(chunks) {
  if (!Array.isArray(chunks)) throw new Error("Discord's module loader is unavailable. Wait for the client to finish loading.");
  const loaders = new Set();
  const chunkId = `callmon_${Date.now()}`;
  chunks.push([[chunkId], {}, loader => { loaders.add(loader); }]);
  if (chunks.at(-1)?.[0]?.includes(chunkId)) chunks.pop();
  const errors = [];
  for (const loader of loaders) {
    try {
      const stores = discoverStores(loader);
      Object.defineProperty(stores, "runtime", { value: loader });
      return stores;
    }
    catch (error) { errors.push(error.message); }
  }
  throw new Error(errors.at(-1) || "Callmon: no Discord runtime was captured.");
}

function discoverLazyUserMenu(loader, interaction) {
  for (const factory of Object.values(loader?.m || {})) {
    const source = String(factory);
    if (!source.includes(interaction)) continue;
    try {
      let tree;
      try { tree = parse(`(${source})`, { ecmaVersion: "latest" }); }
      catch { tree = parse(`({${source}})`, { ecmaVersion: "latest" }); }
      let match;
      simple(tree, {
        ArrowFunctionExpression(node) {
          if (!node.async) return;
          let named = false;
          let loading;
          simple(node.body, {
            Literal(value) { if (value.value === interaction) named = true; },
            CallExpression(call) {
              if (call.callee.type !== "MemberExpression" || call.callee.property.name !== "then") return;
              const all = call.callee.object;
              const binding = call.arguments[0];
              if (all.type !== "CallExpression" || all.callee.type !== "MemberExpression" ||
                  all.callee.object.name !== "Promise" || all.callee.property.name !== "all" ||
                  all.arguments[0]?.type !== "ArrayExpression" || binding?.type !== "CallExpression" ||
                  binding.callee.type !== "MemberExpression" || binding.callee.property.name !== "bind" ||
                  binding.callee.object.type !== "Identifier" || binding.arguments[0]?.name !== binding.callee.object.name ||
                  binding.arguments[1]?.type !== "Literal") return;
              const moduleId = binding.arguments[1].value;
              const runtimeName = binding.callee.object.name;
              const chunks = all.arguments[0].elements;
              if (!chunks.length || !chunks.every(chunk => chunk?.type === "CallExpression" &&
                  chunk.callee.type === "MemberExpression" && chunk.callee.object.name === runtimeName &&
                  chunk.callee.property.name === "e" && chunk.arguments.length === 1 &&
                  chunk.arguments[0].type === "Literal" && Number.isSafeInteger(chunk.arguments[0].value))) return;
              if (!Number.isSafeInteger(moduleId)) return;
              loading = { moduleId, chunks: chunks.map(chunk => chunk.arguments[0].value) };
            },
          });
          if (named && loading) match = loading;
        },
      });
      if (match) return match;
    } catch {}
  }
  return null;
}

export function createUserContextMenu(stores) {
  function exportsOf(module) {
    let exports = module?.exports;
    if (!exports) return null;
    const symbol = Object.getOwnPropertySymbols(exports).find(symbol =>
      symbol.description === "rspack exports" || symbol.description === "webpack exports");
    return symbol ? exports[symbol] : exports;
  }
  return {
    open(event, userId, channelId) {
      const loader = stores.runtime;
      const user = stores.users.getUser(userId);
      const channel = stores.channels.getChannel(channelId || stores.selected.getVoiceChannelId());
      if (!user || !channel) throw new Error("Discord user or channel is no longer available");
      const guildId = channel.getGuildId?.() ?? channel.guild_id;
      const interaction = guildId ? "GuildChannelUserContextMenu" : "UserGenericContextMenu";
      const menuId = Object.keys(loader?.m || {}).find(id => {
        const source = String(loader.m[id]);
        return source.includes('navId:"user-context"') && source.includes(interaction);
      });
      let opener;
      let jsx;
      for (const module of Object.values(loader?.c || {})) {
        try {
          const exports = exportsOf(module);
          if (typeof exports?.jsx === "function") jsx = exports.jsx;
          for (const value of Object.values(exports || {})) {
            if (typeof value !== "function") continue;
            const source = String(value);
            if (source.includes("currentTarget.contains") && source.includes("renderLazy:") && source.includes("preventDefault")) opener = value;
          }
        } catch {}
      }
      if (!opener || !jsx || typeof loader !== "function") throw new Error("Discord user context menu is unavailable on this build");
      const render = (id, menuEvent) => {
        const component = exportsOf({ exports: loader(id) })?.default;
        if (typeof component !== "function") throw new Error("Discord user context menu could not be loaded");
        opener(menuEvent, props => jsx(component, {
          ...props, user, channel, guildId,
          showMediaItems: true, showChannelCallItems: true,
        }));
      };
      if (menuId) return render(menuId, event);
      const lazy = discoverLazyUserMenu(loader, interaction);
      if (!lazy || typeof loader.e !== "function") throw new Error("Discord user context menu is unavailable on this build");
      const menuEvent = {
        target: event.target, currentTarget: event.currentTarget,
        pageX: event.pageX ?? 0, pageY: event.pageY ?? 0,
        preventDefault: () => event.preventDefault(), stopPropagation: () => event.stopPropagation(),
      };
      return Promise.all(lazy.chunks.map(id => loader.e(id))).then(() => render(lazy.moduleId, menuEvent));
    },
  };
}

export function observeCall(stores, send, onError) {
  let last = null;
  let disposed = false;
  function snapshot() {
    if (disposed) return;
    try {
      const channelId = stores.selected.getVoiceChannelId();
      const channel = channelId ? stores.channels.getChannel(channelId) : null;
      if (channelId && !channel) throw new Error("The active voice channel is unavailable.");
      const states = channelId ? stores.voice.getVoiceStatesForChannel(channelId) : {};
      if (channelId && (!states || typeof states !== "object")) throw new Error("Unsupported voice state format.");
      const users = Object.values(states).map(state => {
        if (typeof state?.userId !== "string") throw new Error("Unsupported voice user ID format.");
        const user = stores.users.getUser(state.userId);
        if (!user) throw new Error("A voice participant is not yet available in the user store.");
        let avatarUrl = null;
        try {
          if (typeof user.getAvatarURL === "function") avatarUrl = safeAvatarUrl(user.getAvatarURL(null, 64, true));
          if (!avatarUrl && /^[a-f0-9_]+$/i.test(user.avatar || "")) {
            avatarUrl = safeAvatarUrl(`https://cdn.discordapp.com/avatars/${state.userId}/${user.avatar}.${user.avatar.startsWith("a_") ? "gif" : "png"}?size=64`);
          }
        } catch {}
        return {
          id: state.userId,
          username: user.username ?? "Unknown user",
          display_name: user.globalName || user.username || "Unknown user",
          avatar_url: avatarUrl,
        };
      }).sort((left, right) => left.id.localeCompare(right.id));
      const payload = JSON.stringify({ kind: "snapshot", channel: channelId ? {
        id: channelId, name: channel.name || "Voice call",
      } : null, users });
      if (payload !== last) {
        send(payload);
        last = payload;
      }
      onError(null);
    } catch (error) { onError(error.message); }
  }
  const listeners = [stores.voice, stores.selected];
  try {
    for (const store of listeners) store.addChangeListener(snapshot);
  } catch (error) {
    for (const store of listeners) store.removeChangeListener(snapshot);
    throw error;
  }
  const timer = setInterval(snapshot, 1000);
  snapshot();
  return () => {
    disposed = true;
    clearInterval(timer);
    for (const store of listeners) store.removeChangeListener(snapshot);
  };
}