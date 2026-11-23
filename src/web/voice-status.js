export function mountVoiceStatus(adapter) {
  const owned = new Map();
  const disposers = [];
  let disposed = false;
  let refreshing = false;

  function restore(node, state) {
    owned.delete(node);
    if (node.textContent === state.rendered) node.textContent = state.original;
  }

  function refresh() {
    if (disposed || refreshing) return;
    refreshing = true;
    try {
      const labels = new Map();
      for (const { node, userId } of adapter.labels()) labels.set(node, userId);
      for (const [node, state] of owned) {
        if (!labels.has(node) || labels.get(node) !== state.userId) restore(node, state);
        else if (node.textContent !== state.rendered) owned.delete(node);
      }
      for (const [node, userId] of labels) {
        if (disposed) break;
        const state = owned.get(node);
        if (!state && node.textContent !== "In voice") continue;
        let name = null;
        try { name = adapter.resolve(userId); } catch {}
        if (disposed) break;
        if (typeof name !== "string" || !name.trim() || name.length > 200) {
          if (state) restore(node, state);
          continue;
        }
        const rendered = `In ${name}`;
        owned.set(node, { userId, original: state?.original ?? "In voice", rendered });
        if (node.textContent !== rendered) node.textContent = rendered;
      }
    } finally { refreshing = false; }
  }

  function dispose() {
    if (disposed) return;
    disposed = true;
    let failure;
    for (const stop of disposers.splice(0)) {
      try { stop(); } catch (error) { failure ??= error; }
    }
    for (const [node, state] of owned) restore(node, state);
    if (failure) throw failure;
  }

  try {
    disposers.push(adapter.observe(refresh));
    if (adapter.subscribe) disposers.push(adapter.subscribe(refresh));
    refresh();
  } catch (error) {
    dispose();
    throw error;
  }
  return { refresh, dispose };
}