import { createElement, Radio, Trash2, X } from "lucide";
import { userToolTheme } from "./user-tool-theme.js";

const validId = value => typeof value === "string" && /^\d{1,20}$/.test(value) && BigInt(value) > 0n && BigInt(value) <= 18446744073709551615n;
const styles = userToolTheme + `
.ic-watch-root{position:fixed;inset:0;z-index:1003;display:flex;align-items:center;justify-content:center;box-sizing:border-box;padding:16px;background:rgba(0,0,0,.65)}
.ic-watch-dialog{box-sizing:border-box;width:480px;max-width:100%;max-height:calc(100vh - 32px);max-height:calc(100dvh - 32px);overflow:auto;border-radius:8px;padding:20px;background:var(--ic-tool-bg);color:var(--ic-tool-text);font-family:var(--font-primary,Whitney,sans-serif);font-size:14px;letter-spacing:0;overflow-wrap:anywhere}
.ic-watch-dialog header,.ic-watch-row{display:flex;align-items:center;gap:12px}.ic-watch-dialog header h2{flex:1;margin:0;font-size:20px}.ic-watch-dialog h3{margin:20px 0 8px;font-size:14px}.ic-watch-dialog button{display:grid;place-items:center;flex-shrink:0;width:32px;height:32px;border:0;border-radius:4px;background:transparent;color:inherit;cursor:pointer}.ic-watch-dialog button:hover{background:var(--ic-tool-hover)}.ic-watch-dialog :is(button,input):focus-visible{outline:2px solid var(--text-link,#00a8fc);outline-offset:2px}
.ic-watch-dialog ul,.ic-watch-dialog ol{list-style:none;padding:0;margin:0}.ic-watch-row{padding:10px 0;border-bottom:1px solid var(--ic-tool-border)}.ic-watch-person{flex:1;min-width:0;user-select:text}.ic-watch-person small,.ic-watch-dialog time{display:block;color:var(--ic-tool-muted);font-size:12px}.ic-watch-selected{margin-top:16px;padding:12px 0;border-block:1px solid var(--ic-tool-border)}.ic-watch-selected label{display:flex;align-items:center;gap:8px;margin-top:12px;cursor:pointer}.ic-watch-selected input{width:18px;height:18px;margin:0;accent-color:var(--text-link,#00a8fc)}.ic-watch-empty{color:var(--ic-tool-muted);margin:10px 0}.ic-watch-dialog [role=status]{color:var(--ic-tool-muted);margin:0}.ic-watch-dialog [role=status]:not(:empty){margin-top:12px}
`;

export function mountVoiceWatch(watch, stores = {}) {
  let root = null;
  let selectedId = null;
  let restoreFocus = null;
  let unsubscribe = null;
  let disposed = false;
  let checkbox = null;
  let watchedList = null;
  let recentList = null;
  let heading = null;
  let status = null;
  const node = (tag, text) => {
    const element = document.createElement(tag);
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const iconButton = (icon, label) => {
    const button = node("button");
    button.type = "button";
    button.title = label;
    button.setAttribute("aria-label", label);
    const image = createElement(icon, { "aria-hidden": "true" });
    image.setAttribute("width", "18");
    image.setAttribute("height", "18");
    button.append(image);
    return button;
  };
  function name(id) {
    try {
      const user = stores.users?.getUser(id);
      if (user?.id === id) {
        if (typeof user.globalName === "string" && user.globalName) return user.globalName;
        if (typeof user.username === "string" && user.username) return user.username;
      }
    } catch {}
    return id;
  }
  function person(id) {
    const element = node("div");
    element.className = "ic-watch-person";
    const displayName = name(id);
    element.append(node("strong", displayName));
    if (displayName !== id) element.append(node("small", id));
    return element;
  }
  function controls() { return [...root.querySelectorAll("button,input")].filter(element => !element.disabled); }
  function close() {
    const previousFocus = restoreFocus;
    restoreFocus = null;
    unsubscribe?.();
    unsubscribe = null;
    document.removeEventListener("keydown", keyboard, true);
    document.removeEventListener("focusin", focusInside, true);
    root?.remove();
    root = checkbox = watchedList = recentList = heading = status = null;
    selectedId = null;
    if (previousFocus?.isConnected) previousFocus.focus();
  }
  function focusInside(event) {
    if (root && !root.contains(event.target)) controls()[0]?.focus();
  }
  function keyboard(event) {
    if (!root) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopImmediatePropagation();
      close();
    } else if (event.key === "Tab") {
      const elements = controls();
      const index = elements.indexOf(document.activeElement);
      const next = index < 0 ? (event.shiftKey ? elements.length - 1 : 0)
        : (index + (event.shiftKey ? -1 : 1) + elements.length) % elements.length;
      event.preventDefault();
      event.stopImmediatePropagation();
      elements[next]?.focus();
    }
  }
  function updateWatch(id, enabled) {
    try {
      watch.setWatched(id, enabled);
      status.textContent = watch.isWatched(id) === enabled ? "" : "Voice watch limit reached (100)";
    } catch (error) {
      status.textContent = "Unable to update Voice Watch";
      try { stores.onError?.(error); } catch {}
    }
    render();
  }
  function render() {
    if (!root) return;
    const focused = document.activeElement;
    const focusedId = focused?.dataset?.removeWatch;
    const ids = watch.listWatched().slice(0, 100);
    if (checkbox) checkbox.checked = watch.isWatched(selectedId);
    heading.textContent = `Watched users (${ids.length}/100)`;
    watchedList.replaceChildren();
    for (const id of ids) {
      const row = node("li");
      row.className = "ic-watch-row";
      const remove = iconButton(Trash2, `Stop watching ${name(id)}`);
      remove.dataset.removeWatch = id;
      remove.addEventListener("click", () => updateWatch(id, false));
      row.append(person(id), remove);
      watchedList.append(row);
    }
    if (!ids.length) watchedList.append(node("li", "No watched users"));
    recentList.replaceChildren();
    for (const event of watch.events.slice(-100).reverse()) {
      const row = node("li");
      row.className = "ic-watch-row";
      const detail = node("div");
      detail.className = "ic-watch-person";
      detail.append(node("span", `${event.user.display_name || event.user.username || event.user.id} ${event.action}`),
        node("small", `User ${event.user.id} / Channel ${event.channel.id}`));
      const time = node("time", new Date(event.timestamp).toLocaleTimeString());
      time.dateTime = new Date(event.timestamp).toISOString();
      detail.append(time);
      row.append(detail);
      recentList.append(row);
    }
    if (!recentList.children.length) recentList.append(node("li", "No recent events"));
    if (focusedId) {
      const replacement = [...watchedList.querySelectorAll("button")].find(button => button.dataset.removeWatch === focusedId);
      (replacement || checkbox || controls()[0]).focus();
    }
  }
  function open(userId = null) {
    if (disposed) return;
    if (userId !== null && !validId(userId)) throw new Error("Invalid Discord user ID");
    if (root) close();
    selectedId = userId;
    restoreFocus = document.activeElement;
    root = node("div");
    root.className = "ic-watch-root ic-user-tool";
    root.dataset.intellicordOwned = "";
    const dialog = node("section");
    dialog.className = "ic-watch-dialog";
    dialog.dataset.intellicordOwned = "";
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.setAttribute("aria-label", "Voice Watch");
    const header = node("header");
    const icon = createElement(Radio, { "aria-hidden": "true" });
    icon.setAttribute("width", "20");
    icon.setAttribute("height", "20");
    const dismiss = iconButton(X, "Close Voice Watch");
    dismiss.addEventListener("click", close);
    header.append(icon, node("h2", "Voice Watch"), dismiss);
    dialog.append(header);
    if (selectedId !== null) {
      const selected = node("section");
      selected.className = "ic-watch-selected";
      selected.setAttribute("aria-label", "Selected user");
      const label = node("label");
      checkbox = node("input");
      checkbox.type = "checkbox";
      checkbox.setAttribute("aria-label", "Watch voice activity");
      checkbox.addEventListener("change", () => updateWatch(selectedId, checkbox.checked));
      label.append(checkbox, node("span", "Watch voice activity"));
      selected.append(person(selectedId), label);
      dialog.append(selected);
    }
    heading = node("h3");
    watchedList = node("ul");
    watchedList.setAttribute("aria-label", "Watched users");
    recentList = node("ol");
    recentList.setAttribute("aria-label", "Recent voice events");
    status = node("p");
    status.setAttribute("role", "status");
    dialog.append(heading, watchedList, node("h3", "Recent events"), recentList, status);
    root.append(node("style", styles), dialog);
    root.addEventListener("click", event => { if (event.target === root) close(); });
    try {
      unsubscribe = watch.subscribe(render);
      render();
      document.body.append(root);
      document.addEventListener("keydown", keyboard, true);
      document.addEventListener("focusin", focusInside, true);
      dismiss.focus();
    } catch (error) { close(); throw error; }
  }
  return {
    open,
    isWatched: id => watch.isWatched(id),
    subscribe: listener => watch.subscribe(listener),
    dispose() { if (disposed) return; disposed = true; close(); },
  };
}