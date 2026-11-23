import { createElement, Copy, ExternalLink, X } from "lucide";

const trackingParameter = /^(?:utm_.*|fbclid|gclid|msclkid)$/i;

export function inspectLink(input) {
  const url = new URL(input);
  const cleaned = new URL(url.href);
  const parameters = [...url.searchParams].map(([name, value]) => ({
    name, value, tracking: trackingParameter.test(name),
  }));
  for (const { name, tracking } of parameters) if (tracking) cleaned.searchParams.delete(name);
  const credentials = Boolean(url.username || url.password);
  const redacted = new URL(url.href);
  const redactedCleaned = new URL(cleaned.href);
  for (const visible of [redacted, redactedCleaned]) {
    visible.username = "";
    visible.password = "";
  }
  return {
    scheme: url.protocol.slice(0, -1), hostname: url.hostname,
    pathname: url.pathname, port: url.port, credentials, parameters,
    hash: url.hash, originalUrl: url.href, cleanedUrl: cleaned.href,
    displayUrl: redacted.href, displayCleanedUrl: redactedCleaned.href,
    canOpen: /^(https?:)$/.test(url.protocol) && !credentials,
  };
}

const css = `
[data-intellicord-links-overlay] { position:fixed; inset:0; z-index:1003; background:#0008; display:grid; place-items:center; padding:16px; }
.ic-link-dialog { box-sizing:border-box; position:relative; margin:auto; width:560px; max-width:100%; max-height:calc(100dvh - 32px); overflow:auto; border:1px solid #65666e; border-radius:8px; padding:20px; background:#232428; color:#f2f3f5; font:14px "gg sans",sans-serif; }
.ic-link-dialog::backdrop { background:#0008; }
.ic-link-dialog h2 { margin:0 0 16px; font-size:20px; }
.ic-link-dialog dt { color:#b5bac1; margin-top:12px; }
.ic-link-dialog dd { margin:4px 0; overflow-wrap:anywhere; white-space:pre-wrap; }
.ic-link-dialog ul { padding-left:20px; overflow-wrap:anywhere; }
.ic-link-actions { display:flex; flex-wrap:wrap; gap:8px; margin-top:16px; }
.ic-link-actions button,.ic-link-actions a { box-sizing:border-box; padding:8px 10px; border:1px solid #65666e; border-radius:4px; color:inherit; background:#35363c; text-decoration:none; font:inherit; }
.ic-link-actions button:focus-visible,.ic-link-actions a:focus-visible { outline:2px solid #62c9b8; outline-offset:2px; }
.ic-link-item { --ic-link-foreground:var(--interactive-text-default,var(--interactive-normal,var(--text-default,var(--text-normal,light-dark(#313338,#dbdee1))))); color-scheme:dark; color:var(--ic-link-foreground); cursor:pointer; }
.ic-link-item[data-link-fallback-style] { display:flex; align-items:center; padding:6px 8px; font:inherit; }
.theme-light .ic-link-item { color-scheme:light; }
.theme-dark .ic-link-item { color-scheme:dark; }
.ic-link-actions button,.ic-link-actions a { display:inline-flex; align-items:center; gap:6px; }
.ic-link-actions svg,.ic-link-item svg { flex-shrink:0; }
.ic-link-item:hover,.ic-link-item:focus { background:var(--menu-item-default-hover-bg,var(--background-modifier-hover,light-dark(#e3e5e8,#404249))); color:var(--menu-item-default-hover-text,var(--interactive-text-active,var(--ic-link-foreground))); }
`;

function defaultMenuAnchor(menu) {
  const navId = menu.getAttribute("data-nav-id") ?? menu.getAttribute("navId") ?? menu.id;
  if (navId === "user-context") return null;
  const standalone = menu.querySelector('[role="menuitem"][id="link-copy"]');
  if (!["link-context", "message"].includes(navId)) {
    return !navId && standalone ? { item: standalone,
      href: standalone.getAttribute("data-url") ?? standalone.getAttribute("href") } : null;
  }
  const item = [...menu.querySelectorAll('[role="menuitem"]')].find(node =>
    /^(?:(?:link-context|message)-)?(?:open-link|copy-link)$/.test(node.id)) ?? standalone;
  return { item, href: item?.getAttribute("data-url") ?? item?.getAttribute("href") };
}

export function mountLinkInspector(adapter = {}) {
  const doc = document;
  const view = doc.defaultView;
  const style = doc.createElement("style");
  style.dataset.intellicordLinksStyle = "";
  style.textContent = css;
  doc.head.append(style);
  let context = null;
  let activeMenu = null;
  let inserted = null;
  let overlay = null;
  let dialog = null;
  let disposed = false;
  let returnFocus = null;

  function close() {
    if (!overlay) return;
    overlay.remove();
    overlay = null;
    dialog = null;
    if (returnFocus?.isConnected) returnFocus.focus();
    returnFocus = null;
  }

  function clearContext() {
    context = null;
    activeMenu = null;
    inserted?.remove();
    inserted = null;
  }

  function show(href, anchor) {
    close();
    const result = inspectLink(href);
    returnFocus = anchor;
    overlay = doc.createElement("div");
    overlay.dataset.intellicordLinksOverlay = "";
    dialog = doc.createElement("dialog");
    dialog.className = "ic-link-dialog";
    dialog.setAttribute("aria-label", "Link Inspector");
    const heading = doc.createElement("h2");
    heading.textContent = "Link Inspector";
    const details = doc.createElement("dl");
    for (const [label, value] of [
      ["Original URL", result.displayUrl], ["Cleaned URL", result.displayCleanedUrl],
      ["Scheme", result.scheme], ["ASCII hostname", result.hostname],
      ["Path", result.pathname], ["Port", result.port || "Default"],
      ["Credentials", result.credentials ? "Present (redacted); external opening blocked" : "None"],
      ["Fragment", result.hash || "None"],
    ]) {
      const term = doc.createElement("dt");
      term.textContent = label;
      const definition = doc.createElement("dd");
      definition.textContent = value;
      details.append(term, definition);
    }
    const parameterHeading = doc.createElement("h3");
    parameterHeading.textContent = "Parameters";
    const parameters = doc.createElement("ul");
    for (const parameter of result.parameters) {
      const row = doc.createElement("li");
      row.textContent = `${parameter.name} = ${parameter.value}${parameter.tracking ? " (tracking)" : ""}`;
      parameters.append(row);
    }
    if (!result.parameters.length) {
      const row = doc.createElement("li");
      row.textContent = "None";
      parameters.append(row);
    }
    const actions = doc.createElement("div");
    actions.className = "ic-link-actions";
    const status = doc.createElement("p");
    status.setAttribute("role", "status");
    const closeButton = doc.createElement("button");
    closeButton.type = "button";
    closeButton.textContent = "Close";
    closeButton.prepend(createElement(X, { width: 16, height: 16, "aria-hidden": "true" }));
    closeButton.addEventListener("click", close);
    actions.append(closeButton);
    for (const [label, url] of [["original", result.originalUrl], ["cleaned", result.cleanedUrl]]) {
      const copy = doc.createElement("button");
      copy.type = "button";
      copy.textContent = `Copy ${label} URL`;
      copy.prepend(createElement(Copy, { width: 16, height: 16, "aria-hidden": "true" }));
      copy.addEventListener("click", async () => {
        try {
          await view.navigator.clipboard.writeText(url);
          status.textContent = "Copied";
        } catch { status.textContent = "Unable to copy URL"; }
      });
      actions.append(copy);
      const open = doc.createElement(result.canOpen ? "a" : "button");
      open.textContent = `Open ${label}`;
      open.prepend(createElement(ExternalLink, { width: 16, height: 16, "aria-hidden": "true" }));
      if (result.canOpen) {
        open.href = url;
        open.target = "_blank";
        open.rel = "noopener noreferrer";
        open.referrerPolicy = "no-referrer";
      } else {
        open.type = "button";
        open.disabled = true;
      }
      actions.append(open);
    }
    dialog.append(heading, details, parameterHeading, parameters, actions, status);
    overlay.append(dialog);
    doc.body.append(overlay);
    overlay.addEventListener("click", event => { if (event.target === overlay) close(); });
    dialog.addEventListener("cancel", event => { event.preventDefault(); close(); });
    if (typeof dialog.showModal === "function") dialog.showModal();
    else dialog.setAttribute("open", "");
    closeButton.focus();
  }

  function capture(event) {
    const previousMenus = new Set(doc.querySelectorAll('[role="menu"]'));
    clearContext();
    if (overlay) return;
    const anchor = event.target?.closest?.("a[href]");
    if (!anchor?.isConnected || anchor.ownerDocument !== doc) return;
    try {
      const href = new URL(anchor.getAttribute("href"), doc.baseURI ?? undefined).href;
      const result = inspectLink(href);
      if (!["http", "https"].includes(result.scheme)) return;
      context = { anchor, href, previousMenus };
    } catch { return; }
  }

  function refresh() {
    if (disposed || !context) return;
    if (!context.anchor.isConnected || (activeMenu && !activeMenu.isConnected)
      || (context.pendingMenu && !context.pendingMenu.isConnected)) {
      clearContext();
      return;
    }
    for (const menu of activeMenu ? [activeMenu] : doc.querySelectorAll('[role="menu"]')) {
      if (context.previousMenus.has(menu)) continue;
      let match;
      try { match = (adapter.menuAnchor ?? defaultMenuAnchor)(menu); }
      catch { clearContext(); return; }
      if (!match) { clearContext(); return; }
      const item = "item" in match ? match.item : match;
      context.pendingMenu = menu;
      if (!item) {
        if (activeMenu) clearContext();
        continue;
      }
      if (!menu.contains(item) || item.closest('[role="menu"]') !== menu) continue;
      if (match.href != null) {
        try {
          if (new URL(match.href, doc.baseURI ?? undefined).href !== context.href) { clearContext(); return; }
        } catch { clearContext(); return; }
      }
      if (activeMenu) {
        if (!inserted?.isConnected) clearContext();
        return;
      }
      const { href, anchor } = context;
      inserted = doc.createElement("div");
      inserted.className = `${item.className} ic-link-item`.trim();
      if (!item.className) inserted.dataset.linkFallbackStyle = "";
      inserted.dataset.intellicordLinkItem = "";
      inserted.setAttribute("role", "menuitem");
      inserted.tabIndex = -1;
      const label = doc.createElement("span");
      label.className = item.firstElementChild?.getAttribute("class") || "";
      label.textContent = "Inspect Link";
      label.style.cssText = "flex:1;min-width:0;color:inherit";
      inserted.append(label);
      inserted.addEventListener("click", event => {
        event.stopPropagation();
        show(href, anchor);
        clearContext();
      });
      activeMenu = menu;
      const open = [...menu.querySelectorAll('[role="menuitem"][id]')]
        .find(node => /(?:^|-)open-link$/.test(node.id) && node.closest('[role="menu"]') === menu);
      const copy = [...menu.querySelectorAll('[role="menuitem"][id]')]
        .find(node => /(?:^|-)(?:copy-link|copy-url|link-copy)$/.test(node.id) && node.closest('[role="menu"]') === menu);
      if (copy) copy.after(inserted);
      else if (open) open.before(inserted);
      else item.after(inserted);
      return;
    }
  }

  function keydown(event) {
    if (dialog) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        close();
      } else if (event.key === "Tab") {
        const controls = [...dialog.querySelectorAll("button:not([disabled]), a[href]")];
        const position = controls.indexOf(doc.activeElement);
        const next = event.shiftKey ? (position <= 0 ? controls.length - 1 : position - 1) : (position + 1) % controls.length;
        event.preventDefault();
        event.stopImmediatePropagation();
        controls[next]?.focus();
      }
      return;
    }
    if (event.key === "Escape") { clearContext(); return; }
    if (!activeMenu?.contains(event.target)) return;
    const items = [...activeMenu.querySelectorAll('[role^="menuitem"]')].filter(item =>
      item.closest('[role="menu"]') === activeMenu && !item.disabled &&
      item.getAttribute("aria-disabled") !== "true" && !item.closest('[hidden], [aria-hidden="true"]'));
    const position = items.indexOf(event.target.closest('[role^="menuitem"]'));
    let next;
    if (event.key === "ArrowDown") next = (position + 1) % items.length;
    if (event.key === "ArrowUp") next = (position <= 0 ? items.length : position) - 1;
    if (event.key === "Home") next = 0;
    if (event.key === "End") next = items.length - 1;
    if (next !== undefined) {
      event.preventDefault();
      event.stopImmediatePropagation();
      items[next]?.focus();
    } else if (event.target === inserted && ["Enter", " "].includes(event.key)) {
      event.preventDefault();
      event.stopImmediatePropagation();
      inserted.click();
    }
  }

  function pointerdown(event) {
    if (context && !activeMenu?.contains(event.target)) clearContext();
  }

  const observer = new view.MutationObserver(refresh);
  observer.observe(doc.body, { childList: true, subtree: true, attributes: true,
    attributeFilter: ["id", "data-nav-id", "navid", "href", "data-url"] });
  doc.addEventListener("contextmenu", capture, true);
  doc.addEventListener("keydown", keydown, true);
  doc.addEventListener("pointerdown", pointerdown, true);
  return {
    dispose() {
      if (disposed) return;
      disposed = true;
      observer.disconnect();
      doc.removeEventListener("contextmenu", capture, true);
      doc.removeEventListener("keydown", keydown, true);
      doc.removeEventListener("pointerdown", pointerdown, true);
      clearContext();
      close();
      style.remove();
    },
  };
}