import { createElement, Copy, X } from "lucide";

const states = new Set(["ready", "waiting", "error", "unsupported"]);
const heartbeatTimeout = 6000;
const css = `
.ic-health-tab { box-sizing: border-box; display: block; width: 100%; border: 0; border-radius: 4px; padding: 6px 10px; margin: 2px 0; background: transparent; color: var(--interactive-normal, #b5bac1); font: inherit; text-align: left; cursor: pointer; }
.ic-health-tab:hover { background: var(--background-modifier-hover, #35373c); }
.ic-health-tab[aria-selected="true"] { background: var(--background-modifier-selected, #404249); color: var(--interactive-active, #fff); }
.ic-health-tab:focus-visible, .ic-health-panel button:focus-visible { outline: 2px solid var(--text-link, #00a8fc); outline-offset: 2px; }
.ic-health-panel { position: fixed; z-index: 1001; box-sizing: border-box; overflow: auto; padding: 32px 24px; background: var(--background-primary, #313338); color: var(--text-normal, #dbdee1); font-family: inherit; font-size: 14px; letter-spacing: 0; }
.ic-health-panel[hidden], .ic-health-panel [hidden] { display: none; }
.ic-health-panel header { display: flex; align-items: center; gap: 12px; }
.ic-health-panel h1 { flex: 1; font-size: 20px; margin: 0; overflow-wrap: anywhere; }
.ic-health-panel button { display: inline-flex; align-items: center; justify-content: center; flex: 0 0 32px; width: 32px; height: 32px; border: 0; border-radius: 4px; background: transparent; color: inherit; cursor: pointer; }
.ic-health-panel button:hover { background: var(--background-modifier-hover, #35373c); }
.ic-health-panel dl { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 2fr); gap: 8px 16px; margin: 24px 0; }
.ic-health-panel dd { margin: 0; overflow-wrap: anywhere; }
.ic-health-panel dt { color: var(--text-muted, #949ba4); }
.ic-health-panel .ic-health-table { overflow-x: auto; }
.ic-health-panel table { border-collapse: collapse; width: 100%; text-align: left; table-layout: fixed; min-width: 380px; }
.ic-health-panel th, .ic-health-panel td { padding: 10px 8px; border-bottom: 1px solid var(--background-modifier-accent, #45474c); vertical-align: top; overflow-wrap: anywhere; }
.ic-health-panel th { font-weight: 600; }
.ic-health-panel td[data-state="ready"] { color: var(--text-positive, #72dba3); }
.ic-health-panel td[data-state="error"] { color: var(--text-danger, #ff858b); }
.ic-health-panel textarea { box-sizing: border-box; width: 100%; min-height: 180px; margin-top: 16px; background: var(--background-secondary, #2b2d31); color: inherit; border: 1px solid var(--background-modifier-accent, #45474c); border-radius: 4px; padding: 12px; }
`;

function node(tag, text) {
  const result = document.createElement(tag);
  if (text !== undefined) result.textContent = text;
  return result;
}

function iconButton(icon, label) {
  const button = node("button");
  button.type = "button";
  button.title = label;
  button.setAttribute("aria-label", label);
  button.append(createElement(icon, { width: 18, height: 18, "aria-hidden": "true" }));
  return button;
}

function timestamp(value) {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 8640000000000000 ? value : null;
}

function age(value, now) {
  return value === null ? "Never" : `${Math.floor(Math.max(0, now - value) / 1000)}s ago`;
}

function visible(sidebar) {
  if (!sidebar?.isConnected) return false;
  for (let current = sidebar; current; current = current.parentElement) {
    if (current.hidden) return false;
    const style = window.getComputedStyle?.(current);
    if (style?.display === "none" || style?.visibility === "hidden") return false;
  }
  return !sidebar.getClientRects || sidebar.getClientRects().length > 0;
}

function defaultSidebar() {
  for (const anchor of document.querySelectorAll("[data-settings-sidebar-item]")) {
    const sidebar = anchor.closest('[role="tablist"]') || anchor.parentElement;
    if (visible(sidebar)) return sidebar;
  }
  return null;
}

export function mountSessionHealth(adapter, getSnapshot) {
  if (typeof adapter === "function" && getSnapshot === undefined) {
    getSnapshot = adapter;
    adapter = {};
  }
  adapter ??= {};
  if (typeof getSnapshot !== "function") throw new TypeError("A snapshot getter is required");
  const findSidebar = adapter.sidebar ? () => adapter.sidebar() : defaultSidebar;
  const style = node("style", css);
  const entry = node("button", "Intellicord");
  entry.className = "ic-health-tab";
  entry.type = "button";
  entry.setAttribute("role", "tab");
  entry.setAttribute("aria-selected", "false");
  const panel = node("section");
  panel.className = "ic-health-panel";
  panel.hidden = true;
  panel.setAttribute("role", "tabpanel");
  panel.setAttribute("aria-label", "Intellicord diagnostics");
  let suffix = 1;
  while (document.getElementById(`ic-health-${suffix}`) || document.getElementById(`ic-health-tab-${suffix}`)) suffix += 1;
  panel.id = `ic-health-${suffix}`;
  entry.id = `ic-health-tab-${suffix}`;
  entry.setAttribute("aria-controls", panel.id);
  panel.setAttribute("aria-labelledby", entry.id);
  const header = node("header");
  const copy = iconButton(Copy, "Copy diagnostics");
  const closeButton = iconButton(X, "Close Intellicord diagnostics");
  header.append(node("h1", "Intellicord"), copy, closeButton);
  const timing = node("dl");
  const tableWrap = node("div");
  tableWrap.className = "ic-health-table";
  const table = node("table");
  table.setAttribute("aria-label", "Feature health");
  const head = node("thead");
  const headingRow = node("tr");
  for (const label of ["Feature", "Status", "Reason", "Last update"]) {
    const cell = node("th", label);
    cell.scope = "col";
    headingRow.append(cell);
  }
  head.append(headingRow);
  const body = node("tbody");
  table.append(head, body);
  tableWrap.append(table);
  const copyStatus = node("p");
  copyStatus.setAttribute("role", "status");
  const fallback = node("textarea");
  fallback.readOnly = true;
  fallback.hidden = true;
  fallback.setAttribute("aria-label", "Sanitized diagnostics");
  panel.append(header, timing, tableWrap, copyStatus, fallback);
  document.body.append(style, panel);
  let sidebar = null;
  let disposed = false;
  let snapshot = {};
  let features = new Map();
  let selection = [];
  let previousFocus = null;
  let getterFailed = false;
  let observer = null;
  const observerOptions = { childList: true, subtree: true, attributes: true, attributeFilter: ["hidden", "style", "class"] };

  function close(restoreFocus = false) {
    if (panel.hidden) return;
    panel.hidden = true;
    entry.setAttribute("aria-selected", "false");
    for (const saved of selection) {
      if (saved.tab.getAttribute("aria-selected") === "false") {
        if (saved.selected === null) saved.tab.removeAttribute("aria-selected");
        else saved.tab.setAttribute("aria-selected", saved.selected);
      }
    }
    selection = [];
    fallback.hidden = true;
    copyStatus.textContent = "";
    if (restoreFocus) (previousFocus?.isConnected ? previousFocus : entry).focus();
  }

  function position() {
    const rect = sidebar?.getBoundingClientRect?.();
    if (!rect) return;
    const beside = window.innerWidth - rect.right >= 420;
    panel.style.left = `${beside ? Math.max(0, rect.right) : 0}px`;
    panel.style.right = "0px";
    panel.style.top = `${beside ? 0 : Math.max(0, Math.min(rect.bottom, window.innerHeight - 240))}px`;
    panel.style.bottom = "0px";
  }

  function open() {
    if (disposed || !visible(sidebar) || !panel.hidden) return;
    previousFocus = document.activeElement;
    selection = [...sidebar.querySelectorAll('[aria-selected="true"]')]
      .filter(tab => tab !== entry)
      .map(tab => ({ tab, selected: tab.getAttribute("aria-selected") }));
    for (const saved of selection) saved.tab.setAttribute("aria-selected", "false");
    entry.setAttribute("aria-selected", "true");
    panel.hidden = false;
    entry.focus();
    refresh();
  }

  function nativeNavigation(event) {
    if (!entry.contains(event.target)) close();
  }

  function keyboard(event) {
    if (event.key === "Escape") close(true);
    if (event.target !== entry) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      open();
    } else if (["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) {
      const tabs = [...sidebar.querySelectorAll('[role="tab"], [data-settings-sidebar-item], a')]
        .filter(tab => tab !== entry && !tab.hasAttribute("disabled") && tab.getAttribute("aria-disabled") !== "true");
      const target = ["ArrowUp", "End"].includes(event.key) ? tabs.at(-1) : tabs[0];
      if (target) {
        event.preventDefault();
        close();
        target.focus();
      }
    }
  }

  function refresh() {
    if (disposed) return;
    observer?.disconnect();
    try {
      refreshContent();
    } finally {
      if (!disposed) observer?.observe(document.body, observerOptions);
    }
  }

  function refreshContent() {
    const candidate = findSidebar();
    const nextSidebar = visible(candidate) ? candidate : null;
    if (nextSidebar !== sidebar) {
      close();
      sidebar?.removeEventListener("click", nativeNavigation, true);
      entry.remove();
      sidebar = nextSidebar;
      if (sidebar) {
        sidebar.append(entry);
        sidebar.addEventListener("click", nativeNavigation, true);
      }
    } else if (sidebar && entry.parentElement !== sidebar) sidebar.append(entry);
    const now = Date.now();
    try {
      snapshot = getSnapshot() || {};
      getterFailed = false;
    } catch {
      getterFailed = true;
    }
    const nextFeatures = new Map();
    for (const [name, feature] of Object.entries(snapshot.features || {})) {
      const state = states.has(feature?.state) ? feature.state : "unsupported";
      const detail = typeof feature?.detail === "string" ? feature.detail : "";
      const previous = features.get(name);
      nextFeatures.set(name, { state, detail, updatedAt: previous?.state === state && previous.detail === detail ? previous.updatedAt : now });
    }
    features = nextFeatures;
    if (panel.hidden) return;
    position();
    timing.replaceChildren();
    const heartbeat = timestamp(snapshot.lastHeartbeat);
    const started = timestamp(snapshot.startedAt);
    const connected = !getterFailed && heartbeat !== null && now - heartbeat <= heartbeatTimeout;
    const values = [
      ["Backend heartbeat", `${connected ? "Connected" : "Disconnected"} · ${age(heartbeat, now)}`],
      ["Last successful snapshot", age(timestamp(snapshot.lastSnapshot), now)],
      ["Attachment uptime", started === null ? "Unknown" : `${Math.floor(Math.max(0, now - started) / 1000)}s`],
    ];
    if (typeof snapshot.build === "string" || typeof snapshot.build === "number") values.push(["Current build", String(snapshot.build)]);
    if (getterFailed) values.push(["Diagnostics", "Snapshot unavailable"]);
    for (const [label, value] of values) timing.append(node("dt", label), node("dd", value));
    body.replaceChildren();
    for (const [name, feature] of features) {
      const row = node("tr");
      const state = node("td", feature.state);
      state.dataset.state = feature.state;
      row.append(node("td", name), state, node("td", feature.detail), node("td", age(feature.updatedAt, now)));
      body.append(row);
    }
  }

  async function copyDiagnostics() {
    const text = JSON.stringify({
      startedAt: timestamp(snapshot.startedAt),
      lastHeartbeat: timestamp(snapshot.lastHeartbeat),
      lastSnapshot: timestamp(snapshot.lastSnapshot),
      features: [...features.values()].map(feature => ({ state: feature.state, lastUpdate: feature.updatedAt })),
    }, null, 2);
    try {
      if (!window.navigator?.clipboard?.writeText) throw new Error("Clipboard unavailable");
      await window.navigator.clipboard.writeText(text);
      if (!disposed && !panel.hidden) copyStatus.textContent = "Diagnostics copied";
    } catch {
      if (disposed || panel.hidden) return;
      fallback.value = text;
      fallback.hidden = false;
      copyStatus.textContent = "Clipboard unavailable";
      fallback.focus();
      fallback.select?.();
      fallback.setSelectionRange?.(0, text.length);
    }
  }

  entry.addEventListener("click", open);
  copy.addEventListener("click", copyDiagnostics);
  const closeFromButton = () => close(true);
  closeButton.addEventListener("click", closeFromButton);
  document.addEventListener("keydown", keyboard);
  window.addEventListener("resize", refresh);
  document.addEventListener("scroll", position, true);
  refresh();
  let stopObserving;
  if (adapter.observe) stopObserving = adapter.observe(refresh);
  else {
    observer = new window.MutationObserver(records => {
      if (records.some(record => !panel.contains(record.target) && !entry.contains(record.target) && record.target !== style)) refresh();
    });
    observer.observe(document.body, observerOptions);
    stopObserving = () => observer.disconnect();
  }
  const interval = window.setInterval(refresh, 2000);

  return {
    refresh,
    dispose() {
      if (disposed) return;
      close();
      disposed = true;
      window.clearInterval(interval);
      stopObserving?.();
      sidebar?.removeEventListener("click", nativeNavigation, true);
      document.removeEventListener("keydown", keyboard);
      window.removeEventListener("resize", refresh);
      document.removeEventListener("scroll", position, true);
      entry.removeEventListener("click", open);
      copy.removeEventListener("click", copyDiagnostics);
      closeButton.removeEventListener("click", closeFromButton);
      entry.remove();
      panel.remove();
      style.remove();
    },
  };
}