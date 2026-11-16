import { CONTENT_PREFERENCE, readContentPreference, writeContentPreference } from "./preferences.js";
import css from "./sponsored.css";

export function mountSponsoredContent(adapter, storage) {
  const preference = readContentPreference(storage);
  let disabled = preference.disabled;
  let error = preference.error;
  let disposed = false;
  let saving = false;
  const style = document.createElement("style");
  style.dataset.intellicordOwned = "";
  style.textContent = css;
  document.head.append(style);
  const control = document.createElement("div");
  control.dataset.intellicordOwned = "";
  control.className = "ic-content-setting";
  const label = document.createElement("label");
  label.className = "ic-content-label";
  const text = document.createElement("span");
  const title = document.createElement("strong");
  title.textContent = "Disable Quests and Sponsored Content";
  const scope = document.createElement("span");
  scope.className = "ic-content-scope";
  scope.textContent = "Intellicord · This client only";
  const input = document.createElement("input");
  input.type = "checkbox";
  input.setAttribute("role", "switch");
  input.setAttribute("aria-label", title.textContent);
  text.append(title, scope);
  label.append(text, input);
  const status = document.createElement("div");
  status.className = "ic-content-status";
  status.setAttribute("role", "status");
  control.append(label, status);
  const hidden = new Set();

  function refresh() {
    if (disposed) return;
    const section = adapter.section();
    if (section && control.parentElement !== section) section.prepend(control);
    if (!section) control.remove();
    input.checked = disabled;
    input.disabled = saving || storage?.ready === false;
    status.textContent = error || (storage?.ready === false ? "Loading Intellicord preferences" : saving ? "Saving preference" : "");
    status.hidden = !status.textContent;
    if (!disabled) {
      for (const node of hidden) node.removeAttribute("data-intellicord-content-hidden");
      hidden.clear();
      return;
    }
    const candidates = new Set(adapter.nodes());
    for (const node of hidden) {
      if (!node.isConnected || !candidates.has(node)) {
        node.removeAttribute("data-intellicord-content-hidden");
        hidden.delete(node);
      }
    }
    for (const node of candidates) {
      if (!hidden.has(node)) {
        node.setAttribute("data-intellicord-content-hidden", "");
        hidden.add(node);
      }
    }
  }

  input.addEventListener("change", () => {
    const nextValue = input.checked;
    const success = () => {
      if (disposed) return;
      disabled = nextValue;
      saving = false;
      error = null;
      refresh();
    };
    const failure = cause => {
      if (disposed) return;
      saving = false;
      error = cause?.message || "Unable to save preference; setting unchanged";
      refresh();
    };
    try {
      const result = writeContentPreference(storage, nextValue);
      if (result?.then) {
        saving = true;
        error = null;
        refresh();
        result.then(success, failure);
      } else success();
    } catch { failure(new Error("Unable to save preference; setting unchanged")); }
  });
  const unsubscribe = storage?.subscribe?.(value => {
    if (disposed) return;
    disabled = value.disabled;
    error = value.error;
    refresh();
  });
  function sync(event) {
    if (event.key !== CONTENT_PREFERENCE) return;
    const value = readContentPreference(storage);
    disabled = value.disabled;
    error = value.error;
    refresh();
  }
  window.addEventListener("storage", sync);
  let stop;
  try { stop = adapter.observe(refresh); }
  catch (cause) {
    unsubscribe?.();
    window.removeEventListener("storage", sync);
    for (const node of hidden) node.removeAttribute("data-intellicord-content-hidden");
    control.remove();
    style.remove();
    throw cause;
  }
  return {
    get disabled() { return disabled; },
    dispose() {
      disposed = true;
      unsubscribe?.();
      stop();
      window.removeEventListener("storage", sync);
      for (const node of hidden) node.removeAttribute("data-intellicord-content-hidden");
      hidden.clear();
      control.remove();
      style.remove();
    },
  };
}