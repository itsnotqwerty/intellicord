import { createElement, ExternalLink } from "lucide";

export function discordLookupUrl(userId, token = crypto.randomUUID()) {
  if (typeof userId !== "string" || !/^\d{1,20}$/.test(userId)) throw new Error("Invalid Discord user ID");
  const url = new URL("https://leakfun.com/dashboard/searches/discord");
  url.search = new URLSearchParams({ source: "universal", autorun: "1", q: userId, mode: "lookup", autorunToken: token });
  return url.href;
}

export function mountDiscordLookup(tools = {}) {
  const foreground = "var(--interactive-text-default,var(--interactive-normal,var(--text-default,var(--text-normal,light-dark(#313338,#dbdee1)))))";
  const style = document.createElement("style");
  style.textContent = `[data-intellicord-user-action]{color-scheme:dark}.theme-light [data-intellicord-user-action]{color-scheme:light}.theme-dark [data-intellicord-user-action]{color-scheme:dark}[data-intellicord-user-action]:focus{background:var(--menu-item-default-hover-bg,var(--background-modifier-hover,light-dark(#e3e5e8,#404249))) !important;color:var(--menu-item-default-hover-text,var(--interactive-text-active,${foreground})) !important}`;
  document.head.append(style);
  const links = new Set();
  const replaced = new Map();
  const actions = new Set();
  function styleAction(action) {
    action.dataset.intellicordUserAction = "";
    action.style.cssText = `appearance:none;background:transparent;color:${foreground};border:0;font:inherit;text-align:left;cursor:pointer;display:flex;align-items:center;justify-content:space-between;gap:8px;width:100%`;
    let hovered = false;
    let focused = false;
    const highlight = () => {
      const active = hovered || focused;
      action.style.background = active ? "var(--menu-item-default-hover-bg,var(--background-modifier-hover,light-dark(#e3e5e8,#404249)))" : "transparent";
      action.style.color = active ? `var(--menu-item-default-hover-text,var(--interactive-text-active,${foreground}))` : foreground;
    };
    action.addEventListener("pointerenter", () => { hovered = true; highlight(); });
    action.addEventListener("pointerleave", () => { hovered = false; highlight(); });
    action.addEventListener("focus", () => { focused = true; highlight(); });
    action.addEventListener("blur", () => { focused = false; highlight(); });
  }
  function menuLabel(copy, text) {
    const element = document.createElement("span");
    element.className = copy.firstElementChild?.getAttribute("class") || "";
    element.textContent = text;
    element.style.cssText = "flex:1;min-width:0;color:inherit";
    return element;
  }
  function dismiss(menu) {
    menu.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", keyCode: 27, which: 27, bubbles: true }));
  }
  function nativeCopy(menu) {
    return [...menu.querySelectorAll('[role="menuitem"][id]')]
      .find(node => !node.hasAttribute("data-intellicord-owned") && /(?:^|-)devmode-copy-id-\d{1,20}$/.test(node.id));
  }
  function obliterateContext(menu) {
    try {
      const context = tools.obliterate.context(menu);
      const copy = nativeCopy(menu);
      if (!context || typeof context.userId !== "string" || !context.userId ||
          typeof context.channelId !== "string" || !context.channelId ||
          (copy && copy.id.match(/devmode-copy-id-(\d{1,20})$/)[1] !== context.userId)) return null;
      return { ...context };
    } catch {
      return null;
    }
  }
  function refresh() {
    for (const link of links) if (!link.isConnected) links.delete(link);
    for (const action of actions) if (!action.isConnected) actions.delete(action);
    for (const copy of replaced.keys()) if (!copy.isConnected) replaced.delete(copy);
    for (const menu of document.querySelectorAll('[role="menu"][id="user-context"]')) {
      const mute = [...menu.querySelectorAll('[role^="menuitem"][id]')]
        .find(node => node.id === "user-context-mute");
      if (tools.obliterate && mute && !menu.querySelector('[data-obliterate-action]')) {
        const action = document.createElement("button");
        action.type = "button";
        action.className = mute.className;
        styleAction(action);
        action.setAttribute("role", "menuitem");
        action.setAttribute("aria-label", "Obliterate");
        action.tabIndex = -1;
        action.dataset.intellicordOwned = "";
        action.dataset.obliterateAction = "";
        action.append(menuLabel(mute, "Obliterate"));
        const context = obliterateContext(menu);
        let disabled = !context;
        const disable = title => {
          disabled = true;
          action.setAttribute("aria-disabled", "true");
          action.title = title;
        };
        action.setAttribute("aria-disabled", String(disabled));
        if (disabled) disable("User and channel context could not be verified");
        action.addEventListener("click", event => {
          event.stopPropagation();
          if (disabled) return;
          const current = obliterateContext(menu);
          if (!current || current.userId !== context.userId || current.channelId !== context.channelId) {
            disable("Menu target changed or could not be verified; reopen the menu");
            return;
          }
          const failed = error => disable(`Could not open Obliterate: ${error?.message || "Unknown error"}`);
          try {
            dismiss(menu);
            Promise.resolve(tools.obliterate.open(context)).catch(failed);
          } catch (error) {
            failed(error);
          }
        });
        mute.before(action);
        actions.add(action);
      }
      const copy = nativeCopy(menu);
      if (!copy || menu.querySelector('[data-intellicord-lookup]')) continue;
      const userId = copy.id.match(/devmode-copy-id-(\d{1,20})$/)[1];
      if (tools.identity) {
        replaced.set(copy, copy.hidden);
        copy.hidden = true;
        const identity = document.createElement("button");
        identity.type = "button";
        identity.className = copy.className;
        styleAction(identity);
        identity.append(menuLabel(copy, "Identity Lens"));
        identity.setAttribute("role", "menuitem");
        identity.tabIndex = -1;
        identity.dataset.intellicordOwned = "";
        identity.addEventListener("click", event => { event.stopPropagation(); dismiss(menu); tools.identity.open(userId); });
        copy.before(identity);
        actions.add(identity);
      }
      if (tools.watch) {
        const watch = document.createElement("button");
        watch.type = "button";
        watch.className = copy.className;
        styleAction(watch);
        watch.setAttribute("role", "menuitem");
        watch.setAttribute("aria-label", "Voice Watch");
        watch.tabIndex = -1;
        watch.dataset.intellicordOwned = "";
        watch.dataset.voiceWatchId = userId;
        watch.append(menuLabel(copy, "Voice Watch"));
        watch.addEventListener("click", event => {
          event.stopPropagation();
          dismiss(menu);
          tools.watch.open(userId);
        });
        copy.before(watch);
        actions.add(watch);
      }
      const link = document.createElement("a");
      link.dataset.intellicordLookup = userId;
      link.dataset.intellicordOwned = "";
      link.id = `user-context-intellicord-lookup-${userId}`;
      link.className = copy.className;
      link.setAttribute("role", "menuitem");
      link.tabIndex = -1;
      link.href = discordLookupUrl(userId);
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      link.referrerPolicy = "no-referrer";
      link.title = "Look up public profile on Leakfun (external site)";
      link.style.cssText = "display:flex;align-items:center;justify-content:space-between;gap:8px;text-decoration:none;cursor:pointer";
      const label = document.createElement("span");
      label.textContent = "Look Up Public Profile";
      link.append(label, createElement(ExternalLink, { width: 18, height: 18, "aria-hidden": "true" }));
      const highlight = value => { link.style.background = value ? "var(--background-modifier-hover, #404249)" : ""; };
      link.addEventListener("pointerenter", () => highlight(true));
      link.addEventListener("pointerleave", () => highlight(false));
      link.addEventListener("focus", () => highlight(true));
      link.addEventListener("blur", () => highlight(false));
      link.addEventListener("click", event => {
        event.stopPropagation();
        menu.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", keyCode: 27, which: 27, bubbles: true }));
      });
      copy.after(link);
      links.add(link);
    }
  }
  function keyboard(event) {
    const menu = event.target.closest?.('[role="menu"][id="user-context"]');
    const link = menu?.querySelector('[data-intellicord-lookup]');
    if (!link && !menu?.querySelector('[data-obliterate-action]')) return;
    const items = [...menu.querySelectorAll('[role^="menuitem"]')].filter(item => !item.hidden && !item.disabled && !item.classList.contains("disabled") && item.getAttribute("aria-disabled") !== "true");
    const active = event.target.closest?.('[role^="menuitem"]');
    const index = items.indexOf(active);
    if (index >= 0) {
      if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        event.preventDefault();
        event.stopPropagation();
        const next = event.key === "Home" ? items[0] : event.key === "End" ? items.at(-1)
          : items[(index + (event.key === "ArrowUp" ? -1 : 1) + items.length) % items.length];
        next?.focus();
      } else if ((event.key === "Enter" || event.key === " ") && (active === link || actions.has(active))) {
        event.preventDefault();
        event.stopPropagation();
        active.click();
      }
    }
  }
  const observer = new MutationObserver(refresh);
  observer.observe(document.body, { childList: true, subtree: true });
  document.addEventListener("keydown", keyboard, true);
  refresh();
  return { dispose() {
    style.remove();
    observer.disconnect();
    document.removeEventListener("keydown", keyboard, true);
    for (const [copy, hidden] of replaced) copy.hidden = hidden;
    replaced.clear();
    for (const action of actions) action.remove();
    actions.clear();
    for (const link of links) link.remove();
    links.clear();
  } };
}