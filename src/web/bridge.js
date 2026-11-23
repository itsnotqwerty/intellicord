import { safeAvatarUrl } from "./avatars.js";
import { parse } from "acorn";
import { simple } from "acorn-walk";

export function sponsoredContentSection() {
  return document.querySelector('[data-nav-anchor-key="sponsored_content_category"]');
}

export function settingsSidebar() {
  const anchor = [...document.querySelectorAll('[data-settings-sidebar-item]')]
    .find(node => node.getClientRects().length && !node.closest('[data-intellicord-owned]'));
  return anchor?.closest('[role="tablist"]') || anchor?.parentElement || null;
}

export function quietSurfaceNodes(key) {
  const selectors = {
    promotions: '[class*="premiumPromo_"], [class*="nitroUpsell_"], [class*="marketingBanner_"]',
    shop: 'a[href="/shop"], a[href="/store"], [data-list-item-id$="-shop"]',
    decorations: '[class*="avatarDecoration_"], [class*="profileEffect_"]',
  };
  if (!selectors[key]) return null;
  return [...document.querySelectorAll(selectors[key])].filter(node =>
    !node.closest('[data-intellicord-owned], [role="alert"], [role="dialog"], [data-settings-sidebar-item]') &&
    !node.querySelector('[role="alert"], [data-settings-sidebar-item], input, [aria-label*="security" i], [aria-label*="permission" i]'));
}

export function resolveUserCall(stores, userId) {
  const channelId = stores.selected.getVoiceChannelId();
  const states = channelId ? stores.voice.getVoiceStatesForChannel(channelId) : {};
  if (!Object.values(states || {}).some(state => state.userId === userId)) return null;
  const channel = stores.channels.getChannel(channelId);
  return typeof channel?.name === "string" && channel.name.trim() ? channel.name : null;
}

export function voiceStatusLabels() {
  const labels = [];
  for (const owner of document.querySelectorAll('[data-user-id], [data-list-item-id^="members-"]')) {
    const userId = owner.getAttribute("data-user-id") || owner.getAttribute("data-list-item-id")?.match(/-(\d{1,20})$/)?.[1];
    if (!/^\d{1,20}$/.test(userId || "")) continue;
    for (const node of owner.querySelectorAll('[class*="activity_"], [class*="activityText_"], [data-intellicord-voice-status]')) {
      if (!node.children.length && (node.textContent === "In voice" || node.textContent.startsWith("In "))) labels.push({node, userId});
    }
  }
  return labels;
}

export function nativeLinkMenuAnchor(menu) {
  if (menu.id === "user-context") return null;
  return [...menu.querySelectorAll('[role="menuitem"][id]')]
    .find(node => /(?:open-link|copy-link|copy-url)/.test(node.id)) || null;
}

export function sponsoredContentNodes() {
  const candidates = document.querySelectorAll('[class*="quest" i], [class*="sponsor" i], [class*="promoted" i], [aria-label*="quest" i], [aria-label*="sponsored" i], [aria-label*="promoted content" i], [data-list-item-id*="quest" i], a[href*="/quests"], a[href*="/quest-home"]');
  const protectedAncestors = '[data-intellicord-owned], [data-nav-anchor-key="sponsored_content_category"], [data-settings-sidebar-item], [role="alert"], [role="alertdialog"]';
  const protectedDescendants = '[role="alert"], [role="alertdialog"], input, [data-settings-sidebar-item], [aria-label*="security" i], [aria-label*="permission" i]';
  const isQuestSurface = node => [...node.classList].some(name =>
    /^quests?(?:[a-z]*(?:Card|Tile|Panel|Bar|Banner|Container|Wrapper|Popup|Modal|Content|Overlay|Offer)[a-z]*)?(?:_|$)/i.test(name));
  const questBannerSurface = node => {
    const content = node.closest('[class*="contentWrapper_"]');
    const moduleId = content && [...content.classList].map(name => name.match(/^contentWrapper_(.+)$/)?.[1]).find(Boolean);
    const wrapper = content?.parentElement;
    const mask = wrapper?.parentElement;
    return moduleId && wrapper.classList.contains(`wrapper_${moduleId}`) && mask?.classList.contains(`mask_${moduleId}`)
      && wrapper.children.length === 1 && mask.children.length === 1 ? mask : null;
  };
  const surfaces = new Set();
  for (const node of candidates) {
    const questSurface = isQuestSurface(node);
    const questOwner = questSurface ? questBannerSurface(node) || node : null;
    const matches = questOwner ||
      [...node.classList].some(name => /^(?:[Ss]ponsored|[Pp]romotedContent|[Pp]romotedPopup)(?:[A-Z_])/.test(name)) ||
      /^(?:Quests?|Sponsored Content|Promoted Content)(?:$|\s)/i.test(node.getAttribute("aria-label") || "") ||
      /(?:^|[_-])quests?(?:$|[_-])/i.test(node.getAttribute("data-list-item-id") || "") ||
      /\/(?:quests|quest-home)(?:\/|$|\?)/.test(node.getAttribute("href") || "");
    if (!matches) continue;
    const target = questOwner ? questOwner.closest('[role="dialog"]') || questOwner : node;
    if (target.closest(protectedAncestors) || target.querySelector(protectedDescendants)) continue;
    surfaces.add(target);
  }
  return [...surfaces].filter(node => ![...surfaces].some(parent => parent !== node && parent.contains(node)));
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

export function obliterateMenuContext(menu) {
  if (menu?.id !== "user-context") return null;
  const pairs = new Map();
  const visited = new Set();
  for (const owner of [menu, menu.parentElement]) {
    if (!owner) continue;
    const key = Object.keys(owner).find(value => value.startsWith("__reactFiber$"));
    let fiber = key ? owner[key] : null;
    for (let depth = 0; fiber && depth < 80 && !visited.has(fiber); depth++, fiber = fiber.return) {
      visited.add(fiber);
      try {
        const props = fiber.memoizedProps;
        const userId = props?.user?.id ?? props?.userId;
        const channelId = props?.channel?.id ?? props?.channelId;
        if (typeof userId === "string" && /^\d{1,20}$/.test(userId) &&
            typeof channelId === "string" && /^\d{1,20}$/.test(channelId)) {
          pairs.set(`${userId}:${channelId}`, { userId, channelId });
        }
      } catch { return null; }
    }
  }
  return pairs.size === 1 ? pairs.values().next().value : null;
}

export function createObliterateAdapter(stores) {
  let disposed = false;
  const validId = value => typeof value === "string" && /^[1-9]\d{0,19}$/.test(value);
  const method = (owner, key) => {
    try { return owner != null && key in Object(owner) && typeof owner[key] === "function"; }
    catch { return false; }
  };
  const read = (owner, key, ...args) => method(owner, key) ? owner[key](...args) : undefined;
  const boolean = value => value === true || value === false;
  function candidates() {
    const result = [];
    for (const entry of [...Object.values(stores), ...Object.values(stores.runtime?.c || {}).map(module => module?.exports)]) {
      try {
        if (!entry) continue;
        const symbol = Object.getOwnPropertySymbols(entry).find(value =>
          value.description === "rspack exports" || value.description === "webpack exports");
        const exports = symbol ? entry[symbol] : entry;
        if (exports) result.push(exports, ...Object.values(exports));
      } catch {}
    }
    return result;
  }
  function snapshot(input = {}) {
    const { userId, channelId } = input || {};
    const plan = {
      target: { id: userId, name: userId },
      context: { channelId: channelId ?? null, guildId: null, guildName: null },
      actions: [
        { key: "mute", label: "Mute locally", status: "unavailable" },
        { key: "soundboard", label: "Mute soundboard locally", status: "unavailable" },
        { key: "video", label: "Disable local video", status: "unavailable" },
        { key: "ignore", label: "Ignore", status: "unavailable" },
        { key: "block", label: "Block", status: "unavailable" },
        { key: "moderation", label: "Moderation", status: "unavailable" },
      ],
    };
    const operations = {};
    const unavailable = reason => {
      for (const action of plan.actions) {
        action.status = "unavailable";
        action.reason = reason;
        delete action.severity;
      }
      return { plan, operations: {} };
    };
    if (disposed) return unavailable("Adapter disposed");
    if (!validId(userId) || (channelId != null && !validId(channelId))) return unavailable("Invalid target or channel ID");
    let stage = "target lookup";
    let moderationStarted = false;
    try {
      const current = read(stores.users, "getCurrentUser");
      const target = read(stores.users, "getUser", userId);
      if (!validId(current?.id) || target?.id !== userId) return unavailable("User unavailable");
      plan.target.name = target.globalName || target.username || userId;
      if (current.id === userId) return unavailable("Self actions are unavailable");
      stage = "native API discovery";
      const exports = candidates();
      const named = name => exports.find(owner => {
        try { return read(owner, "getName") === name; }
        catch { return false; }
      });
      const find = key => {
        const matches = exports.filter(owner => method(owner, key));
        return new Set(matches.map(owner => owner[key])).size === 1 ? matches[0] : undefined;
      };
      const permissionValue = key => {
        const values = new Set();
        for (const owner of exports) {
          try {
            const descriptor = owner != null ? Object.getOwnPropertyDescriptor(Object(owner), key) : null;
            if (descriptor && typeof descriptor.value === "bigint") values.add(descriptor.value);
          } catch {}
        }
        return values.size === 1 ? values.values().next().value : undefined;
      };
      const configure = (key, state, owner, actionKey, args, verify, reason) => {
        const action = plan.actions.find(value => value.key === key);
        if (!boolean(state) || !method(owner, actionKey)) {
          action.reason = reason || "Verified native API or state unavailable";
          return;
        }
        action.status = state ? "done" : "ready";
        operations[key] = { run: () => owner[actionKey](...args), verify };
      };
      stage = "local media state";
      const connected = channelId && read(stores.selected, "getVoiceChannelId") === channelId;
      const states = connected ? read(stores.voice, "getVoiceStatesForChannel", channelId) : null;
      const inCall = id => Object.values(states || {}).some(state => state?.userId === id);
      const local = connected && inCall(userId) && inCall(current.id);
      const media = named("MediaEngineStore");
      const soundboard = named("SoundboardStore");
      let modes;
      for (const owner of exports) {
        try {
          modes = Object.values(owner || {}).find(value => value &&
            ["DISABLED", "MANUAL_ENABLED", "AUTO_PROBING", "AUTO_ENABLED", "NONE"].every(key => Object.hasOwn(value, key)));
          if (modes) break;
        } catch {}
      }
      const locals = [
        ["mute", media, "isLocalMute", "toggleLocalMute"],
        ["soundboard", soundboard, "isLocalSoundboardMuted", "toggleLocalSoundboardMute"],
        ["video", media, "isLocalVideoDisabled", "setDisableLocalVideo"],
      ];
      for (const [key, owner, getter, actionKey] of locals) {
        const verify = () => read(owner, getter, ...("soundboard" === key ? [userId] : [userId, "default"]));
        const args = key === "video" ? [userId, modes?.DISABLED, "default", true, false] : [userId, "default"];
        configure(key, local && (key !== "video" || modes?.DISABLED != null) ? verify() : undefined,
          find(actionKey), actionKey, args, verify, local ? undefined : "Target and current user must be in the explicit connected call");
      }
      stage = "relationship state";
      const relationships = named("RelationshipStore");
      const blocked = read(relationships, "isBlocked", userId);
      const ignored = read(relationships, "isIgnored", userId);
      configure("ignore", blocked === false ? ignored : undefined, find("ignoreUser"), "ignoreUser",
        [userId, undefined, undefined], () => read(relationships, "isIgnored", userId),
        blocked === true ? "Blocked users mask ignored state; ignore cannot be verified" : undefined);
      configure("block", boolean(ignored) ? blocked : undefined, find("blockUser"), "blockUser",
        [userId, undefined, undefined], () => read(relationships, "isBlocked", userId));
      const block = plan.actions.find(action => action.key === "block");
      if (block.status === "ready") block.reason = "Blocking masks ignored state; verify ignore before blocking";
      const moderation = plan.actions.find(action => action.key === "moderation");
      moderation.reason = "No eligible native moderation action available";
      moderationStarted = true;
      stage = "server context";
      const channel = channelId ? read(stores.channels, "getChannel", channelId) : null;
      const guildId = read(channel, "getGuildId") ?? channel?.guild_id;
      if (!validId(guildId)) return { plan, operations };
      plan.context.guildId = guildId;
      const guild = read(named("GuildStore"), "getGuild", guildId);
      if (!guild || guild.id !== guildId) return { plan, operations };
      plan.context.guildName = guild.name ?? null;
      const members = named("GuildMemberStore");
      if (!read(members, "getMember", guildId, userId) || !read(members, "getMember", guildId, current.id) ||
          read(target, "isNonUserBot") !== false || target.isProvisional) return { plan, operations };
      stage = "moderation eligibility";
      const permissions = named("PermissionStore");
      for (const [severity, constant, actionKey, label] of [
        ["ban", "BAN_MEMBERS", "banUser", "Ban from server"],
        ["kick", "KICK_MEMBERS", "kickUser", "Kick from server"],
      ]) {
        const permission = permissionValue(constant);
        const owner = find(actionKey);
        if ((severity === "ban" && target.bot) || permission === undefined || !method(owner, actionKey) ||
            read(permissions, "canManageUser", permission, target, guild) !== true) continue;
        Object.assign(moderation, { severity, label, status: "ready", reason: undefined });
        operations.moderation = {
          run: () => severity === "ban" ? owner.banUser(guildId, userId, 0, undefined, undefined) :
            owner.kickUser(guildId, userId, undefined, undefined),
        };
        break;
      }
      if (moderation.status !== "ready") {
        const predicates = [...new Set(exports.filter(candidate => {
          if (typeof candidate !== "function" || candidate.length !== 2) return false;
          const source = Function.prototype.toString.call(candidate);
          return /^function\s*\w*\s*\(/.test(source) &&
            ["canManageUser", "MODERATE_MEMBERS", "ADMINISTRATOR", "isNonUserBot", "getGuild", "getUser"].every(token => source.includes(token)) &&
            /arguments\s*\[\s*2\s*\]/.test(source) && !/\buse[A-Z]\w*\s*\(|\.bG\s*\(/.test(source);
        }))];
        const setters = [...new Set(exports.filter(owner => {
          if (!method(owner, "setCommunicationDisabledUntil")) return false;
          const source = Function.prototype.toString.call(owner.setCommunicationDisabledUntil);
          return ["guildId", "userId", "communicationDisabledUntilTimestamp", "duration", "location"].every(token => source.includes(token));
        }))];
        if (predicates.length !== 1 || setters.length !== 1) {
          moderation.reason = "Timeout unsupported: verified native eligibility predicate or setter missing or ambiguous; ban/kick unavailable";
          return { plan, operations };
        }
        const permission = permissionValue("MODERATE_MEMBERS");
        if (permission === undefined || read(permissions, "canManageUser", permission, target, guild) !== true ||
            predicates[0](guildId, userId, [stores.users, named("GuildStore"), permissions]) !== true) return { plan, operations };
        const duration = 28 * 24 * 60 * 60;
        Object.assign(moderation, { severity: "timeout", label: "Timeout for 28 days", status: "ready", reason: undefined });
        operations.moderation = {
          run: () => setters[0].setCommunicationDisabledUntil({
            guildId, userId, communicationDisabledUntilTimestamp: new Date(Date.now() + duration * 1000).toISOString(),
            duration, reason: undefined, location: undefined, moderatorReportId: undefined,
          }),
        };
      }
      return { plan, operations };
    } catch {
      if (moderationStarted) {
        const moderation = plan.actions.find(action => action.key === "moderation");
        Object.assign(moderation, { status: "unavailable", label: "Moderation",
          reason: `Native state unavailable during ${stage}; moderation disabled` });
        delete moderation.severity;
        delete operations.moderation;
        return { plan, operations };
      }
      return unavailable(`Native state unavailable during ${stage}; no actions are safe`);
    }
  }
  return {
    plan: input => snapshot(input).plan,
    async executeAction(targetContext, key, expectedAction) {
      const { plan, operations } = snapshot(targetContext);
      const action = plan.actions.find(value => value.key === key);
      if (!action) throw new Error("Unknown action");
      if (key === "moderation" && (expectedAction?.key !== key || !expectedAction.severity || expectedAction.severity !== action.severity)) {
        throw new Error("Moderation severity changed or was not confirmed");
      }
      if (key === "moderation" && (!validId(expectedAction.guildId) || expectedAction.guildId !== plan.context.guildId)) {
        throw new Error("Moderation guild changed or was not confirmed");
      }
      if (action.status === "done") return { status: "done", detail: "Already applied" };
      if (action.status !== "ready" || disposed) throw new Error(action.reason || "Action unavailable");
      const operation = operations[key];
      if (typeof operation?.run !== "function") throw new Error("Native operation unavailable; reopen to review a new plan");
      await operation.run();
      if (key === "moderation") return { status: "submitted", detail: `${action.severity} submitted; completion is not verified` };
      let verified = false;
      try { verified = !disposed && operation.verify() === true; } catch {}
      return verified ? { status: "done", detail: "Native state verified" } :
        { status: "unverified", detail: "Native state has not confirmed the change" };
    },
    dispose() { disposed = true; },
  };
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
  let lastSentAt = -Infinity;
  let captureGap = false;
  let disposed = false;
  function snapshot(source = "store_notification") {
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
        kind: Number.isInteger(channel.type) && channel.type >= 0 && channel.type <= 255 ? channel.type : null,
      } : null, users });
      const now = performance.now();
      if (payload !== last || now - lastSentAt >= 2000 || captureGap) {
        send(JSON.stringify({ ...JSON.parse(payload), observation_source: source, capture_gap: captureGap }));
        last = payload;
        lastSentAt = now;
        captureGap = false;
      }
      onError(null);
    } catch (error) { captureGap = true; onError(error.message); }
  }
  const notification = () => snapshot("store_notification");
  const listeners = [stores.voice, stores.selected];
  try {
    for (const store of listeners) store.addChangeListener(notification);
  } catch (error) {
    for (const store of listeners) store.removeChangeListener(notification);
    throw error;
  }
  const timer = setInterval(() => snapshot("reconciliation"), 1000);
  snapshot("initial_snapshot");
  return () => {
    disposed = true;
    clearInterval(timer);
    for (const store of listeners) store.removeChangeListener(notification);
  };
}