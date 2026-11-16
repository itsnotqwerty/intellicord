export function activityUsers(events, participants = {}) {
  const users = new Map();
  for (const event of events) users.set(event.user.id, event.user);
  for (const user of Object.values(participants)) users.set(user.id, user);
  return [...users.values()].sort((left, right) =>
    left.display_name.localeCompare(right.display_name) || left.id.localeCompare(right.id));
}

export function filterActivity(events, { query = "", action = "all", userIds = null } = {}) {
  const search = query.trim().toLowerCase();
  return events.filter(event => {
    const text = `${event.user.display_name} ${event.user.username} ${event.user.id} ${event.channel.name}`.toLowerCase();
    return (userIds === null || userIds.has(event.user.id))
      && (action === "all" || event.action === action) && text.includes(search);
  });
}

export function formatTextLog(events, exportedAt = Date.now()) {
  const sorted = [...events].sort((left, right) => left.timestamp - right.timestamp || left.sequence - right.sequence);
  const lines = ["Callmon voice activity", `Exported: ${new Date(exportedAt).toISOString()}`,
    `Events: ${sorted.length}`, "Order: oldest first", ""];
  for (const event of sorted) {
    lines.push(`${new Date(event.timestamp).toISOString()} ${event.action.toUpperCase()} `
      + `user=${JSON.stringify(event.user.display_name)} username=${JSON.stringify(event.user.username)} `
      + `user_id=${JSON.stringify(event.user.id)} channel=${JSON.stringify(event.channel.name)} `
      + `channel_id=${JSON.stringify(event.channel.id)}`);
  }
  return `${lines.join("\n")}\n`;
}