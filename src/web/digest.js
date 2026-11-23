export function summarizeChanges(events, after, latest) {
  const unseen = events.filter(event => event.sequence > after && event.sequence <= latest);
  const returnedLeaves = new Set(unseen.filter(event => event.reconnect_of != null).map(event => event.reconnect_of));
  return {
    arrivals: unseen.filter(event => event.action === "joined" && event.reconnect_of == null),
    departures: unseen.filter(event => event.action === "left" && !returnedLeaves.has(event.sequence)),
    returns: unseen.filter(event => event.action === "joined" && event.reconnect_of != null),
    unavailable: Math.max(0, latest - after - unseen.length),
  };
}

export function createCallDigest() {
  let callId;
  let checkpoint = 0;
  let latest = 0;
  let events = [];
  return {
    update(state, viewing = false) {
      events = state.events || [];
      latest = state.latest_sequence ?? Math.max(checkpoint, ...events.map(event => event.sequence));
      if (state.current_call?.id !== callId) {
        callId = state.current_call?.id;
        checkpoint = latest;
      }
      if (viewing) checkpoint = latest;
    },
    read() {
      const digest = summarizeChanges(events, checkpoint, latest);
      checkpoint = latest;
      return digest;
    },
  };
}