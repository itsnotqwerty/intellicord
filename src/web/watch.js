const validId = value => typeof value === "string" && /^\d{1,20}$/.test(value) && BigInt(value) > 0n && BigInt(value) <= 18446744073709551615n;

export function createVoiceWatch(stores, onEvent, onError = stores.onError) {
  const watched = new Set();
  const listeners = new Set();
  const history = [];
  const registered = [];
  let channelId = null;
  let previous = null;
  let disposed = false;
  let updating = false;
  let notifying = false;
  let reporting = false;
  let sequence = 0;
  function report(error) {
    if (reporting) return;
    reporting = true;
    try { onError?.(error); } catch {}
    finally { reporting = false; }
  }
  function call(callback, ...args) {
    try { callback?.(...args); } catch (error) { report(error); }
  }
  function notify() {
    if (notifying) return;
    notifying = true;
    try {
      for (const listener of [...listeners]) {
        if (disposed) break;
        if (listeners.has(listener)) call(listener);
      }
    } finally { notifying = false; }
  }
  function snapshot() {
    if (disposed || updating) return;
    updating = true;
    try {
      const selectedId = stores.selected.getVoiceChannelId() || null;
      if (selectedId !== null && !validId(selectedId)) throw new Error("Unsupported selected voice channel ID");
      const states = selectedId === null ? {} : stores.voice.getVoiceStatesForChannel(selectedId);
      if (!states || typeof states !== "object") throw new Error("Unsupported voice state format");
      const current = new Set();
      for (const state of Object.values(states)) {
        if (!validId(state?.userId)) throw new Error("Unsupported voice user ID format");
        if (state.channelId != null && state.channelId !== selectedId) continue;
        current.add(state.userId);
      }
      const baseline = previous === null || channelId !== selectedId || selectedId === null;
      const before = previous;
      channelId = selectedId;
      previous = current;
      if (baseline) return;
      const changes = [...watched].filter(id => before.has(id) !== current.has(id));
      for (const id of changes) {
        if (disposed) break;
        if (!watched.has(id)) continue;
        let user = null;
        try { user = stores.users.getUser(id); } catch (error) { report(error); }
        const username = user?.id === id && typeof user.username === "string" ? user.username : null;
        const displayName = user?.id === id && typeof user.globalName === "string" && user.globalName ? user.globalName : username;
        const event = Object.freeze({ sequence: ++sequence, timestamp: Date.now(), action: current.has(id) ? "joined" : "left",
          user: Object.freeze({ id, username, display_name: displayName }), channel: Object.freeze({ id: selectedId }) });
        history.push(event);
        if (history.length > 100) history.shift();
        call(onEvent, event);
      }
      if (changes.length && !disposed) notify();
    } catch (error) {
      previous = null;
      channelId = null;
      report(error);
    } finally { updating = false; }
  }
  function dispose() {
    if (disposed) return;
    disposed = true;
    for (const store of registered) {
      try { store.removeChangeListener(snapshot); } catch (error) { report(error); }
    }
    registered.length = 0;
    watched.clear();
    listeners.clear();
    history.length = 0;
    previous = null;
  }
  try {
    for (const store of new Set([stores.voice, stores.selected])) {
      registered.push(store);
      store.addChangeListener(snapshot);
    }
  } catch (error) { dispose(); throw error; }
  snapshot();
  function setWatched(id, enabled) {
    if (disposed) return false;
    if (!validId(id)) throw new Error("Invalid Discord user ID");
    if (typeof enabled !== "boolean") throw new TypeError("Expected a boolean watch state");
    if (watched.has(id) === enabled) return enabled;
    if (!enabled) watched.delete(id);
    else {
      if (watched.size >= 100) { report(new Error("Voice watch limit reached (100)")); return false; }
      watched.add(id);
    }
    notify();
    return watched.has(id);
  }
  return {
    setWatched,
    toggle(id) { return setWatched(id, !watched.has(id)); },
    listWatched() { return [...watched]; },
    isWatched(id) { return watched.has(id); },
    subscribe(listener) {
      if (typeof listener !== "function") throw new TypeError("Expected a listener");
      if (disposed) return () => {};
      if (!listeners.has(listener) && listeners.size >= 100) throw new Error("Voice watch listener limit reached (100)");
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    get events() { return history.slice(); },
    dispose,
  };
}