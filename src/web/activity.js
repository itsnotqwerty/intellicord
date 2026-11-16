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

export function formatMarkdownLog(events, exportedAt = Date.now(), call = null) {
  const sorted = [...events].sort((left, right) => left.timestamp - right.timestamp || left.sequence - right.sequence);
  const cell = value => JSON.stringify(String(value)).slice(1, -1)
    .replace(/[\\`*_{}\[\]<>()#+.!|&"'~=-]/g, character => `&#${character.charCodeAt(0)};`);
  const lines = ["# Callmon Voice Activity", "", `Exported: ${new Date(exportedAt).toISOString()}`, ""];
  if (call) {
    lines.push(`Call: ${cell(call.channel.name)} (${cell(call.channel.id)})`, "",
      `Started: ${new Date(call.started_at).toISOString()}`, "",
      `Ended: ${call.ended_at == null ? "Ongoing" : new Date(call.ended_at).toISOString()}`, "");
  }
  lines.push(`Events: ${sorted.length}`, "", "Order: oldest first", "",
    "| Timestamp (UTC) | Action | Display Name | Username | User ID | Channel | Channel ID |",
    "| --- | --- | --- | --- | --- | --- | --- |");
  for (const event of sorted) {
    lines.push(`| ${[new Date(event.timestamp).toISOString(), event.action.toUpperCase(),
      event.user.display_name, event.user.username, event.user.id, event.channel.name, event.channel.id]
      .map(cell).join(" | ")} |`);
  }
  return `${lines.join("\n")}\n`;
}