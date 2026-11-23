export const CONTENT_PREFERENCE = "intellicord.disableSponsoredContent";
export const PROMOTIONS_PREFERENCE = "intellicord.disablePromotions";
export const SHOP_PREFERENCE = "intellicord.disableShop";
export const DECORATIONS_PREFERENCE = "intellicord.disableDecorations";
export const SURFACE_PREFERENCES = Object.freeze({
  sponsored: CONTENT_PREFERENCE,
  promotions: PROMOTIONS_PREFERENCE,
  shop: SHOP_PREFERENCE,
  decorations: DECORATIONS_PREFERENCE,
});

export function readContentPreference(storage, key = CONTENT_PREFERENCE) {
  try { return { disabled: storage.getItem(key) === "true", error: null }; }
  catch { return { disabled: false, error: "Preference storage unavailable" }; }
}

export function writeContentPreference(storage, disabled, key = CONTENT_PREFERENCE) {
  if (typeof disabled !== "boolean") throw new TypeError("Content preference must be boolean");
  if (!Object.values(SURFACE_PREFERENCES).includes(key)) throw new Error("Unsupported preference");
  return storage.setItem(key, String(disabled));
}

export function backendPreferences(send) {
  let state = { disabled: false, promotions: false, shop: false, decorations: false, error: null };
  let ready = false;
  let pending = null;
  let disposed = false;
  const queue = [];
  const listeners = new Set();
  function notify(key, error = state.error) {
    for (const listener of listeners) listener(state, { key, error });
  }
  function pump() {
    if (pending || disposed || !queue.length) return;
    const current = queue.shift();
    pending = current;
    function fail(error) {
      clearTimeout(current.timer);
      pending = null;
      ready = true;
      state = { ...state, error: error.message };
      notify(current.key);
      current.reject(error);
    }
    current.timer = setTimeout(() => {
      fail(new Error("Intellicord preference backend did not respond"));
      for (const queued of queue.splice(0)) {
        notify(queued.key);
        queued.reject(new Error(state.error));
      }
    }, 10000);
    try {
      send(JSON.stringify({
        kind: current.kind,
        ...(current.disabled === undefined ? {} : { disabled: current.disabled }),
        ...(current.key && current.key !== "sponsored" ? { key: current.key } : {}),
      }));
    } catch (error) {
      fail(error instanceof Error ? error : new Error(String(error)));
      pump();
    }
  }
  function request(kind, disabled, key) {
    if (disposed) return Promise.reject(new Error("Intellicord detached"));
    return new Promise((resolve, reject) => {
      queue.push({ kind, disabled, key, resolve, reject });
      pump();
    });
  }
  return {
    get ready() { return ready; },
    getItem(key) {
      const surface = Object.keys(SURFACE_PREFERENCES).find(surface => SURFACE_PREFERENCES[surface] === key);
      return surface ? String(state[surface === "sponsored" ? "disabled" : surface]) : null;
    },
    setItem(key, value) {
      const surface = Object.keys(SURFACE_PREFERENCES).find(surface => SURFACE_PREFERENCES[surface] === key);
      if (!surface || !["true", "false"].includes(value)) throw new Error("Unsupported preference");
      return request("preferences_set", value === "true", surface);
    },
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    load() { return request("preferences_get"); },
    update(value) {
      if (disposed) return;
      if (typeof value?.disabled !== "boolean" || !(value.error === null || typeof value.error === "string")) return;
      if (["promotions", "shop", "decorations"].some(key => value[key] !== undefined && typeof value[key] !== "boolean")) return;
      state = { disabled: value.disabled, promotions: value.promotions ?? false, shop: value.shop ?? false, decorations: value.decorations ?? false, error: value.error };
      ready = true;
      const current = pending;
      pending = null;
      if (current) {
        clearTimeout(current.timer);
        const field = current.key === "sponsored" ? "disabled" : current.key;
        if (!state.error && current.kind === "preferences_set" && state[field] !== current.disabled) {
          state.error = "Intellicord preference backend did not acknowledge the requested setting";
        }
        if (state.error) current.reject(new Error(state.error));
        else current.resolve();
      }
      notify(current?.key);
      pump();
    },
    dispose() {
      disposed = true;
      listeners.clear();
      if (pending) {
        clearTimeout(pending.timer);
        pending.reject(new Error("Intellicord detached"));
        pending = null;
      }
      for (const queued of queue.splice(0)) queued.reject(new Error("Intellicord detached"));
    },
  };
}