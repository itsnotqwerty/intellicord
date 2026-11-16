import { createElement, ExternalLink } from "lucide";

export function discordLookupUrl(userId, token = crypto.randomUUID()) {
  if (typeof userId !== "string" || !/^\d{1,20}$/.test(userId)) throw new Error("Invalid Discord user ID");
  const url = new URL("https://leakfun.com/dashboard/searches/discord");
  url.search = new URLSearchParams({ source: "universal", autorun: "1", q: userId, mode: "lookup", autorunToken: token });
  return url.href;
}

export function mountDiscordLookup() {
  const links = new Set();
  function refresh() {
    for (const link of links) if (!link.isConnected) links.delete(link);
    for (const menu of document.querySelectorAll('[role="menu"][id="user-context"]')) {
      const copy = [...menu.querySelectorAll('[role="menuitem"][id]')]
        .find(node => /(?:^|-)devmode-copy-id-\d{1,20}$/.test(node.id));
      if (!copy || menu.querySelector('[data-intellicord-lookup]')) continue;
      const userId = copy.id.match(/devmode-copy-id-(\d{1,20})$/)[1];
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
    if (!link) return;
    const items = [...menu.querySelectorAll('[role^="menuitem"]')].filter(item => item.getAttribute("aria-disabled") !== "true");
    const index = items.indexOf(link);
    if (event.target === items[index - 1] && event.key === "ArrowDown") {
      event.preventDefault();
      event.stopPropagation();
      link.focus();
    } else if (event.target === link) {
      if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        event.preventDefault();
        event.stopPropagation();
        const next = event.key === "Home" ? items[0] : event.key === "End" ? items.at(-1)
          : items[(index + (event.key === "ArrowUp" ? -1 : 1) + items.length) % items.length];
        next?.focus();
      } else if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        event.stopPropagation();
        link.click();
      }
    }
  }
  const observer = new MutationObserver(refresh);
  observer.observe(document.body, { childList: true, subtree: true });
  document.addEventListener("keydown", keyboard, true);
  refresh();
  return { dispose() {
    observer.disconnect();
    document.removeEventListener("keydown", keyboard, true);
    for (const link of links) link.remove();
    links.clear();
  } };
}