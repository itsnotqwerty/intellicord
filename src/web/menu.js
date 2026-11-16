import { createElement, History, X, Copy, Check, Trash2, ArrowDownLeft, ArrowUpRight, Search, Volume2, Users, Download } from "lucide";
import { activityUsers, filterActivity, formatTextLog } from "./activity.js";
import { callPlacement } from "./placement.js";
import { profilePicture } from "./avatars.js";
import css from "./menu.css";

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function iconButton(icon, label) {
  const button = element("button", "cm-icon");
  button.type = "button";
  button.title = label;
  button.setAttribute("aria-label", label);
  button.append(createElement(icon, { width: 18, height: 18, "aria-hidden": "true" }));
  return button;
}

export function mountMenu(send, userContextMenu = null, watchCallView = null) {
  const previousFocus = () => document.activeElement;
  const root = element("div", "cm-root");
  root.id = "callmon-root";
  const style = element("style");
  style.textContent = css;
  const launcher = iconButton(History, "Voice history");
  launcher.classList.add("cm-launcher");
  launcher.setAttribute("aria-expanded", "false");
  launcher.setAttribute("aria-controls", "callmon-menu");
  const panel = element("section", "cm-panel");
  panel.id = "callmon-menu";
  panel.hidden = true;
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-label", "Voice history");
  const header = element("header", "cm-header");
  const heading = element("h2", "", "Voice history");
  const download = iconButton(Download, "Export matching activity to text log");
  const clear = iconButton(Trash2, "Clear history");
  const close = iconButton(X, "Close history");
  header.append(heading, download, clear, close);
  const channelRow = element("div", "cm-channel");
  const channelName = element("strong", "", "Not in a voice call");
  const count = element("span", "cm-count", "0");
  channelRow.append(createElement(Volume2, { width: 18, height: 18 }), channelName,
    createElement(Users, { width: 15, height: 15 }), count);
  const session = element("div", "cm-session", "This session · 0 events");
  const searchWrap = element("div", "cm-search");
  const search = element("input");
  search.type = "search";
  search.placeholder = "Search usernames or IDs";
  search.setAttribute("aria-label", "Search history");
  const userFilter = iconButton(Users, "Filter by user");
  userFilter.setAttribute("aria-expanded", "false");
  userFilter.setAttribute("aria-controls", "callmon-user-picker");
  const userMenu = element("div", "cm-user-menu");
  userMenu.id = "callmon-user-picker";
  userMenu.hidden = true;
  const userSearch = element("input", "cm-user-search");
  userSearch.type = "search";
  userSearch.placeholder = "Find users";
  userSearch.setAttribute("aria-label", "Find users");
  const allLabel = element("label", "cm-user-all");
  const allUsers = element("input");
  allUsers.type = "checkbox";
  allUsers.setAttribute("aria-label", "All users");
  allLabel.append(allUsers, element("span", "", "All users"));
  const userOptions = element("div", "cm-user-options");
  userOptions.setAttribute("role", "group");
  userOptions.setAttribute("aria-label", "Activity users");
  userMenu.append(userSearch, allLabel, userOptions);
  searchWrap.append(createElement(Search, { width: 16, height: 16 }), search, userFilter, userMenu);
  const filters = element("div", "cm-filters");
  filters.setAttribute("role", "group");
  filters.setAttribute("aria-label", "Event type");
  const list = element("div", "cm-list");
  list.setAttribute("role", "list");
  const status = element("div", "cm-status", "Connecting");
  status.setAttribute("role", "status");
  panel.append(header, channelRow, session, searchWrap, filters, list, status);
  root.append(style, panel, launcher);
  document.body.append(root);
  let state = { channel: null, participants: {}, events: [] };
  let mode = "all";
  let selectedUsers = null;
  let restoreFocus = null;
  let confirmClear = false;
  let live = false;
  let error = null;
  let userContextActive = false;
  let userContextPending = null;
  const filterButtons = new Map();
  const copyTimers = new Set();
  const downloads = new Map();

  function matchingEvents() {
    return filterActivity(state.events, { query: search.value, action: mode, userIds: selectedUsers });
  }

  function renderUsers() {
    const focusedId = userOptions.contains(document.activeElement) ? document.activeElement.dataset.userId : null;
    const scrollTop = userOptions.scrollTop;
    const users = activityUsers(state.events, state.participants);
    const selectedCount = users.filter(user => selectedUsers === null || selectedUsers.has(user.id)).length;
    allUsers.checked = users.length > 0 && selectedCount === users.length;
    allUsers.indeterminate = selectedCount > 0 && selectedCount < users.length;
    userFilter.disabled = users.length === 0;
    userFilter.classList.toggle("cm-filter-active", selectedUsers !== null);
    userFilter.title = selectedUsers === null ? "Filter by user" : `Filter by user (${selectedCount} selected)`;
    userFilter.setAttribute("aria-label", userFilter.title);
    const query = userSearch.value.trim().toLowerCase();
    userOptions.replaceChildren();
    for (const user of users.filter(user => `${user.display_name} ${user.username} ${user.id}`.toLowerCase().includes(query))) {
      const label = element("div", "cm-user-option");
      const choice = element("label", "cm-user-choice");
      const checkbox = element("input");
      checkbox.type = "checkbox";
      checkbox.dataset.userId = user.id;
      checkbox.checked = selectedUsers === null || selectedUsers.has(user.id);
      checkbox.setAttribute("aria-label", `Activity for ${user.username} (${user.id})`);
      checkbox.addEventListener("change", () => {
        if (selectedUsers === null) selectedUsers = new Set(users.map(user => user.id));
        if (checkbox.checked) selectedUsers.add(user.id);
        else selectedUsers.delete(user.id);
        render();
      });
      const details = element("div", "cm-user-details");
      details.append(element("strong", "", user.display_name), element("span", "", `@${user.username}`),
        element("span", "cm-user-id", user.id));
      label.title = `${user.display_name} · @${user.username} · ${user.id}`;
      choice.append(checkbox, details);
      label.append(choice);
      bindUserContextMenu(label, user);
      userOptions.append(label);
    }
    if (!userOptions.children.length) userOptions.append(element("div", "cm-user-none", "No matching users"));
    userOptions.scrollTop = scrollTop;
    if (focusedId) userOptions.querySelector(`[data-user-id="${focusedId}"]`)?.focus();
  }

  function bindUserContextMenu(node, user, channelId) {
    node.tabIndex = 0;
    node.setAttribute("aria-haspopup", "menu");
    function show(event) {
      event.preventDefault();
      event.stopPropagation();
      function failed(cause) {
        userContextActive = false;
        userContextPending = null;
        status.textContent = cause.message || "Unable to open Discord user context menu";
      }
      try {
        if (!userContextMenu?.open) throw new Error("Discord user context menu unavailable");
        userContextActive = true;
        const opened = userContextMenu.open(event, user.id, channelId);
        status.textContent = "";
        if (opened?.then) {
          userContextPending = opened;
          opened.then(() => {
            if (userContextPending === opened) userContextPending = null;
          }, cause => {
            if (userContextPending === opened) failed(cause);
          });
        }
      } catch (cause) { failed(cause); }
    }
    node.addEventListener("contextmenu", show);
    node.addEventListener("keydown", event => {
      if (event.key === "ContextMenu" || event.key === "F10" && event.shiftKey) show(event);
    });
  }

  function render() {
    const focusedCopyId = list.contains(document.activeElement) ? document.activeElement.dataset.copyId : null;
    const scrollTop = list.scrollTop;
    channelName.textContent = state.channel?.name || "Not in a voice call";
    count.textContent = String(Object.keys(state.participants).length);
    count.title = `${count.textContent} participants`;
    const matches = matchingEvents();
    session.textContent = `This session · ${state.events.length} events${matches.length === state.events.length ? "" : ` · ${matches.length} shown`}`;
    clear.disabled = state.events.length === 0;
    download.disabled = matches.length === 0;
    for (const [key, button] of filterButtons) button.setAttribute("aria-pressed", String(mode === key));
    renderUsers();
    const events = [...matches].reverse();
    list.replaceChildren();
    if (!events.length) {
      const empty = element("div", "cm-empty");
      empty.append(createElement(History, { width: 28, height: 28 }),
        element("strong", "", state.events.length ? "No matching events" : "No activity yet"));
      list.append(empty);
    }
    for (const event of events) {
      const row = element("div", `cm-event cm-${event.action}`);
      row.setAttribute("role", "listitem");
      const avatar = profilePicture(event.user);
      const details = element("div", "cm-details");
      const name = element("strong", "cm-name", event.user.display_name);
      const username = element("span", "cm-username", `@${event.user.username}`);
      const action = element("div", "cm-action");
      action.append(createElement(event.action === "joined" ? ArrowDownLeft : ArrowUpRight, { width: 13, height: 13 }),
        element("span", "", `${event.action === "joined" ? "Joined" : "Left"} · ${event.channel.name}`));
      details.append(name, username, action);
      const metadata = element("div", "cm-meta");
      const time = element("time", "", new Date(event.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
      time.dateTime = new Date(event.timestamp).toISOString();
      time.title = new Date(event.timestamp).toLocaleString();
      const copy = iconButton(Copy, `Copy user ID for ${event.user.username}`);
      copy.dataset.copyId = String(event.sequence);
      copy.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText(event.user.id);
          copy.replaceChildren(createElement(Check, { width: 18, height: 18 }));
          copy.title = "Copied";
          copy.setAttribute("aria-label", "User ID copied");
          status.textContent = "User ID copied";
          const timer = setTimeout(() => {
            copyTimers.delete(timer);
            copy.replaceChildren(createElement(Copy, { width: 18, height: 18 }));
            copy.title = `Copy user ID for ${event.user.username}`;
            copy.setAttribute("aria-label", copy.title);
            refreshStatus();
          }, 1800);
          copyTimers.add(timer);
        } catch {
          status.textContent = "Clipboard unavailable";
          const fallback = element("input", "cm-copy-fallback");
          fallback.value = event.user.id;
          fallback.readOnly = true;
          fallback.setAttribute("aria-label", "User ID");
          metadata.append(fallback);
          fallback.focus();
          fallback.select();
        }
      });
      const tools = element("div", "cm-event-tools");
      tools.append(copy);
      metadata.append(time, tools);
      row.append(avatar, details, metadata);
      bindUserContextMenu(row, event.user, event.channel.id);
      list.append(row);
    }
    list.scrollTop = scrollTop;
    if (focusedCopyId) list.querySelector(`[data-copy-id="${focusedCopyId}"]`)?.focus();
  }

  function refreshStatus() {
    status.textContent = error ? `Capture paused · ${error}` : live
      ? state.channel ? "Listening" : "Not connected to voice" : "Backend disconnected";
    status.classList.toggle("cm-warning", Boolean(error) || !live);
  }

  function resetClear() {
    confirmClear = false;
    clear.title = "Clear history";
    clear.setAttribute("aria-label", "Clear history");
    clear.classList.remove("cm-confirm");
  }

  function open(value) {
    if (value) restoreFocus = previousFocus();
    panel.hidden = !value;
    launcher.setAttribute("aria-expanded", String(value));
    resetClear();
    openUserPicker(false);
    if (value) search.focus();
    else if (restoreFocus?.isConnected) restoreFocus.focus();
  }

  function openUserPicker(value) {
    userMenu.hidden = !value;
    userFilter.setAttribute("aria-expanded", String(value));
    fitUserPicker();
    if (value) userSearch.focus();
  }

  function fitUserPicker() {
    if (userMenu.hidden) return;
    const available = panel.getBoundingClientRect().bottom - searchWrap.getBoundingClientRect().bottom - 6;
    userMenu.style.maxHeight = `${Math.max(60, available)}px`;
  }

  userFilter.addEventListener("click", () => openUserPicker(userMenu.hidden));
  userSearch.addEventListener("input", renderUsers);
  allUsers.addEventListener("change", () => {
    selectedUsers = allUsers.checked ? null : new Set();
    render();
  });
  download.addEventListener("click", () => {
    const events = matchingEvents();
    if (!events.length) return;
    try {
      const now = Date.now();
      const url = URL.createObjectURL(new Blob([formatTextLog(events, now)], { type: "text/plain;charset=utf-8" }));
      const link = element("a");
      link.href = url;
      link.download = `callmon-voice-activity-${new Date(now).toISOString().replace(/[:.]/g, "-")}.txt`;
      root.append(link);
      link.click();
      link.remove();
      const timer = setTimeout(() => { URL.revokeObjectURL(url); downloads.delete(url); }, 60000);
      downloads.set(url, timer);
      status.textContent = `Text log prepared · ${events.length} events`;
    } catch { status.textContent = "Unable to prepare text log"; }
  });

  for (const [key, label] of [["all", "All"], ["joined", "Joined"], ["left", "Left"]]) {
    const button = element("button", "", label);
    button.type = "button";
    button.addEventListener("click", () => { mode = key; render(); });
    filterButtons.set(key, button);
    filters.append(button);
  }
  launcher.addEventListener("click", () => open(panel.hidden));
  close.addEventListener("click", () => open(false));
  search.addEventListener("input", render);
  clear.addEventListener("click", () => {
    if (confirmClear) { send(JSON.stringify({ kind: "clear" })); resetClear(); }
    else {
      confirmClear = true;
      clear.title = "Confirm clear history";
      clear.setAttribute("aria-label", "Confirm clear history");
      clear.classList.add("cm-confirm");
    }
  });
  function keyboard(event) {
    if (userContextActive) {
      if (userContextPending || document.querySelector('[role="menu"]')) return;
      if (event.key === "Escape") { userContextActive = false; return; }
    }
    if (event.key === "Escape" && !panel.hidden) {
      event.stopPropagation();
      if (!userMenu.hidden) { openUserPicker(false); userFilter.focus(); }
      else open(false);
    }
  }
  function outside(event) {
    if (userContextActive) {
      if (userContextPending) return;
      const nativeMenu = document.querySelector('[role="menu"]');
      if (nativeMenu) {
        if (!nativeMenu.contains(event.target) && !root.contains(event.target)) userContextActive = false;
        return;
      }
      userContextActive = false;
      if (!root.contains(event.target)) return;
    }
    if (!panel.hidden && !root.contains(event.target)) open(false);
    else if (!userMenu.hidden && !searchWrap.contains(event.target)) openUserPicker(false);
  }
  document.addEventListener("keydown", keyboard, true);
  document.addEventListener("pointerdown", outside, true);
  window.addEventListener("resize", fitUserPicker);
  const stopPlacement = watchCallView?.(rect => {
    const placement = callPlacement(rect, { width: window.innerWidth, height: window.innerHeight });
    root.hidden = !placement;
    if (!placement) { open(false); return; }
    root.style.left = `${placement.left}px`;
    root.style.top = `${placement.top}px`;
    root.style.bottom = "auto";
    panel.style.left = `${placement.panelLeft}px`;
    panel.style.maxHeight = `${placement.panelHeight}px`;
    fitUserPicker();
  });
  render();
  return {
    update(value) {
      state = value;
      if (!state.events.length) { selectedUsers = null; openUserPicker(false); }
      render(); refreshStatus();
    },
    connection(value) { live = value; refreshStatus(); },
    error(value) { error = value; refreshStatus(); },
    dispose() {
      document.removeEventListener("keydown", keyboard, true);
      document.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("resize", fitUserPicker);
      for (const timer of copyTimers) clearTimeout(timer);
      for (const [url, timer] of downloads) { clearTimeout(timer); URL.revokeObjectURL(url); }
      stopPlacement?.();
      root.remove();
    },
  };
}