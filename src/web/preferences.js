export const CONTENT_PREFERENCE = "intellicord.disableSponsoredContent";

export function readContentPreference(storage) {
  try { return { disabled: storage.getItem(CONTENT_PREFERENCE) === "true", error: null }; }
  catch { return { disabled: false, error: "Preference storage unavailable" }; }
}

export function writeContentPreference(storage, disabled) {
  if (typeof disabled !== "boolean") throw new TypeError("Content preference must be boolean");
  return storage.setItem(CONTENT_PREFERENCE, String(disabled));
}

export function backendPreferences(send) {
  let state = { disabled: false, error: null };
  let ready = false;
  let pending = null;
  const listeners = new Set();
  function notify() { for (const listener of listeners) listener(state); }
  function request(kind, disabled) {
    if (pending) return Promise.reject(new Error("A preference request is already pending"));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending = null;
        ready = true;
        state = { ...state, error: "Intellicord preference backend did not respond" };
        notify();
        reject(new Error(state.error));
      }, 10000);
      pending = { resolve, reject, timer };
      try { send(JSON.stringify({ kind, ...(disabled === undefined ? {} : { disabled }) })); }
      catch (error) { clearTimeout(timer); pending = null; reject(error); }
    });
  }
  return {
    get ready() { return ready; },
    getItem(key) { return key === CONTENT_PREFERENCE ? String(state.disabled) : null; },
    setItem(key, value) {
      if (key !== CONTENT_PREFERENCE || !["true", "false"].includes(value)) throw new Error("Unsupported preference");
      return request("preferences_set", value === "true");
    },
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    load() { return request("preferences_get"); },
    update(value) {
      if (typeof value?.disabled !== "boolean" || !(value.error === null || typeof value.error === "string")) return;
      state = value;
      ready = true;
      const request = pending;
      pending = null;
      if (request) {
        clearTimeout(request.timer);
        if (value.error) request.reject(new Error(value.error));
        else request.resolve();
      }
      notify();
    },
    dispose() {
      listeners.clear();
      if (pending) {
        clearTimeout(pending.timer);
        pending.reject(new Error("Intellicord detached"));
        pending = null;
      }
    },
  };
}