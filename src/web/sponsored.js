import { SURFACE_PREFERENCES, readContentPreference, writeContentPreference } from "./preferences.js";
import css from "./sponsored.css";

export function mountSponsoredContent(adapter, storage) {
  let disposed = false;
  const style = document.createElement("style");
  style.dataset.intellicordOwned = "";
  style.textContent = css;
  document.head.append(style);
  const control = document.createElement("div");
  control.dataset.intellicordOwned = "";
  control.className = "ic-content-setting";
  const titles = {
    sponsored: "Disable Quests and Sponsored Content",
    promotions: "Disable Promotional Banners",
    shop: "Disable Shop Entry Points",
    decorations: "Disable Decorative Prompts",
  };
  const surfaces = Object.entries(SURFACE_PREFERENCES).map(([key, storageKey]) => {
    const preference = readContentPreference(storage, storageKey);
    const row = document.createElement("div");
    row.className = "ic-content-row";
    row.dataset.surface = key;
    const label = document.createElement("label");
    label.className = "ic-content-label";
    const text = document.createElement("span");
    const title = document.createElement("strong");
    title.textContent = titles[key];
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
    row.append(label, status);
    control.append(row);
    return { key, storageKey, input, status, disabled: preference.disabled, error: preference.error, saving: false, available: false, availabilityError: null };
  });
  const attribute = "data-intellicord-content-hidden";
  const hidden = new Map();

  function restore(node, original) {
    if (original === null) node.removeAttribute(attribute);
    else node.setAttribute(attribute, original);
  }

  function cleanup() {
    for (const [node, original] of hidden) restore(node, original);
    hidden.clear();
    control.remove();
    style.remove();
  }

  function refresh() {
    if (disposed) return;
    const section = adapter.section();
    if (section && control.parentElement !== section) section.prepend(control);
    if (!section) control.remove();
    const candidates = new Set();
    for (const surface of surfaces) {
      let nodes = [];
      try {
        const result = surface.key === "sponsored" ? adapter.nodes() : adapter.surfaceNodes?.(surface.key);
        surface.available = result != null;
        surface.availabilityError = surface.available ? null : "Unavailable on this client build";
        nodes = Array.from(result ?? []).filter(node => node?.isConnected && typeof node.setAttribute === "function"
          && !node.contains(control) && !control.contains(node) && node !== style && !adapter.isProtected?.(node));
      } catch (cause) {
        surface.available = false;
        surface.availabilityError = cause?.message || "Surface detection unavailable";
      }
      surface.input.checked = surface.disabled;
      surface.input.disabled = !surface.available || surface.saving || storage?.ready === false;
      surface.status.textContent = surface.error || surface.availabilityError || (storage?.ready === false ? "Loading Intellicord preferences" : surface.saving ? "Saving preference" : "");
      surface.status.hidden = !surface.status.textContent;
      if (surface.available && surface.disabled) for (const node of nodes) candidates.add(node);
    }
    for (const [node, original] of hidden) {
      if (!node.isConnected || !candidates.has(node)) {
        restore(node, original);
        hidden.delete(node);
      }
    }
    for (const node of candidates) {
      if (!hidden.has(node)) {
        hidden.set(node, node.getAttribute(attribute));
        node.setAttribute(attribute, "");
      }
    }
  }

  for (const surface of surfaces) {
    surface.input.addEventListener("change", () => {
      if (disposed || surface.saving || !surface.available || storage?.ready === false) { refresh(); return; }
      const nextValue = surface.input.checked;
      const success = () => {
        if (disposed) return;
        surface.disabled = nextValue;
        surface.saving = false;
        surface.error = null;
        refresh();
      };
      const failure = cause => {
        if (disposed) return;
        surface.saving = false;
        surface.error = cause?.message || "Unable to save preference; setting unchanged";
        refresh();
      };
      try {
        const result = writeContentPreference(storage, nextValue, surface.storageKey);
        if (result?.then) {
          surface.saving = true;
          surface.error = null;
          refresh();
          result.then(success, failure);
        } else success();
      } catch { failure(new Error("Unable to save preference; setting unchanged")); }
    });
  }
  const unsubscribe = storage?.subscribe?.((value, request) => {
    if (disposed) return;
    for (const surface of surfaces) {
      surface.disabled = value[surface.key === "sponsored" ? "disabled" : surface.key] === true;
      if (!request?.key || request.key === surface.key) surface.error = request?.error ?? value.error;
    }
    refresh();
  });
  function sync(event) {
    const affected = surfaces.filter(surface => event.key === null || event.key === surface.storageKey);
    if (!affected.length) return;
    for (const surface of affected) {
      const value = readContentPreference(storage, surface.storageKey);
      surface.disabled = value.disabled;
      surface.error = value.error;
    }
    refresh();
  }
  window.addEventListener("storage", sync);
  let stop;
  try { refresh(); stop = adapter.observe(refresh); }
  catch (cause) {
    unsubscribe?.();
    window.removeEventListener("storage", sync);
    cleanup();
    throw cause;
  }
  return {
    get disabled() { return surfaces[0].disabled; },
    get states() {
      return Object.fromEntries(surfaces.map(surface => [surface.key, {
        disabled: surface.disabled, available: surface.available, loading: storage?.ready === false,
        saving: surface.saving, error: surface.error || surface.availabilityError,
      }]));
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      unsubscribe?.();
      stop();
      window.removeEventListener("storage", sync);
      cleanup();
    },
  };
}