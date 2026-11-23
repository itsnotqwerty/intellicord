import { createElement, Copy, FileJson, Radio, Users, X } from "lucide";
import { userToolTheme } from "./user-tool-theme.js";

const unavailable = "Unavailable in retained observations";
const observation = "Local observations, not the actual network cause. No cause is inferred. Baseline participants were already present when observation began; they are not join events.";
const participantLimit = 10000;
const scalar = value => ["string", "number", "boolean"].includes(typeof value) ? value : null;
const channelData = channel => ({ id: scalar(channel?.id), name: scalar(channel?.name), kind: scalar(channel?.kind) });
const userData = user => ({ id: scalar(user?.id), username: scalar(user?.username), display_name: scalar(user?.display_name), avatar_url: scalar(user?.avatar_url) });
const eventData = event => ({
  action: scalar(event?.action), timestamp: scalar(event?.timestamp), sequence: scalar(event?.sequence),
  user: userData(event?.user), channel: channelData(event?.channel),
  reconnect_of: scalar(event?.reconnect_of), reconnect_gap_ms: scalar(event?.reconnect_gap_ms),
});
const dateFields = value => {
  const date = typeof value === "number" && Number.isFinite(value) ? new Date(value) : null;
  return date && Number.isFinite(date.getTime())
    ? { timestamp: value, iso: date.toISOString(), local: date.toLocaleString() }
    : { timestamp: scalar(value), iso: null, local: null };
};
const callData = call => ({ id: scalar(call?.id), channel: channelData(call?.channel), started_at: dateFields(call?.started_at), ended_at: dateFields(call?.ended_at) });
const endedGroup = call => call?.channel?.kind === 3 && typeof call.channel.id === "string" && call.id != null && call.recap != null && call.ended_at != null;
const participantIds = call => [...new Set((Array.isArray(call.recap.participant_ids) ? call.recap.participant_ids : []).filter(id => typeof id === "string"))].sort();
const duration = call => typeof call.started_at === "number" && typeof call.ended_at === "number"
  && Number.isFinite(call.ended_at - call.started_at) && call.ended_at >= call.started_at ? call.ended_at - call.started_at : null;
const counts = call => ({
  baseline_count: scalar(call.recap.baseline_count), unique_retained_participants: participantIds(call).length,
  peak_participants: scalar(call.recap.peak_participants), joined_count: scalar(call.recap.joined_count),
  left_count: scalar(call.recap.left_count), probable_reconnections: scalar(call.recap.probable_reconnections),
});
const truncated = call => call.recap.participant_ids_truncated === true;
const retentionNote = (wasTruncated, atLimit) => wasTruncated
  ? "Participant IDs were truncated; unique counts and participant comparisons use retained IDs only."
  : atLimit
    ? "Participant IDs are at the 10000 retention limit and may be truncated; comparisons use retained IDs only."
    : "Unique counts and participant comparisons use retained participant IDs only; event history may have been trimmed.";

export function eventDetails(event, call = null, events = []) {
  const observed = eventData(event);
  const linkedLeave = observed.reconnect_of == null ? null : events.find(candidate =>
    candidate.sequence === observed.reconnect_of && candidate.action === "left"
    && candidate.user?.id === observed.user.id && candidate.channel?.id === observed.channel.id);
  const reconnects = events.filter(candidate => candidate.reconnect_of === observed.sequence
    && candidate.action === "joined" && observed.action === "left"
    && candidate.user?.id === observed.user.id && candidate.channel?.id === observed.channel.id).map(eventData);
  return {
    observation, ...observed, timestamp: dateFields(observed.timestamp), call: callData(call),
    call_end_status: call?.ended_at == null ? "Ongoing or end not observed" : "Ended observation",
    probable_reconnect: observed.reconnect_of == null ? "No probable reconnect link recorded" : "Probable reconnect only; not a network diagnosis",
    raw_leave_status: observed.reconnect_of == null ? "Not applicable: no reconnect link" : linkedLeave ? "Retained" : "Unavailable: leave event is no longer retained",
    raw_leave_event: linkedLeave ? eventData(linkedLeave) : null,
    linked_probable_reconnect_events: reconnects,
  };
}

export function recapDetails(call) {
  if (!endedGroup(call)) return null;
  const ids = participantIds(call);
  return {
    observation, call: callData(call), observed_duration_ms: duration(call), ...counts(call), participant_ids: ids,
    participant_ids_truncated: truncated(call),
    retention: retentionNote(truncated(call), ids.length >= participantLimit),
  };
}

export function compareRecaps(call, other) {
  if (!endedGroup(call) || !endedGroup(other) || call.channel.id !== other.channel.id || call.id === other.id) return null;
  const currentIds = participantIds(call);
  const otherIds = participantIds(other);
  const currentSet = new Set(currentIds);
  const otherSet = new Set(otherIds);
  const currentCounts = { observed_duration_ms: duration(call), ...counts(call) };
  const otherCounts = { observed_duration_ms: duration(other), ...counts(other) };
  return {
    other_call: callData(other),
    delta_direction: "This call minus comparison call; participant overlap uses retained IDs only",
    deltas: Object.fromEntries(Object.keys(currentCounts).map(key => [key,
      typeof currentCounts[key] === "number" && typeof otherCounts[key] === "number" ? currentCounts[key] - otherCounts[key] : null])),
    shared_participant_ids: currentIds.filter(id => otherSet.has(id)),
    only_this_participant_ids: currentIds.filter(id => !otherSet.has(id)),
    only_other_participant_ids: otherIds.filter(id => !currentSet.has(id)),
    shared_participant_count: currentIds.filter(id => otherSet.has(id)).length,
    only_this_participant_count: currentIds.filter(id => !otherSet.has(id)).length,
    only_other_participant_count: otherIds.filter(id => !currentSet.has(id)).length,
    participant_ids_truncated: truncated(call), other_participant_ids_truncated: truncated(other),
    retention: retentionNote(truncated(call) || truncated(other), currentIds.length >= participantLimit || otherIds.length >= participantLimit),
  };
}

const styles = userToolTheme + `
.ic-details-root{position:fixed;inset:0;z-index:1004;display:flex;align-items:center;justify-content:center;box-sizing:border-box;padding:16px;background:rgba(0,0,0,.65)}
.ic-details-dialog{box-sizing:border-box;width:640px;max-width:100%;max-height:calc(100vh - 32px);max-height:calc(100dvh - 32px);overflow:auto;border-radius:8px;padding:20px;background:var(--ic-tool-bg);color:var(--ic-tool-text);font-family:var(--font-primary,Whitney,sans-serif);font-size:14px;letter-spacing:0;overflow-wrap:anywhere}
.ic-details-dialog header{display:flex;flex-wrap:wrap;align-items:center;gap:8px}.ic-details-dialog h2{flex:1;min-width:120px;font-size:20px;margin:0}.ic-details-dialog button{display:grid;place-items:center;flex-shrink:0;width:32px;height:32px;border:0;border-radius:4px;background:transparent;color:inherit;cursor:pointer}.ic-details-dialog button:hover{background:var(--ic-tool-hover)}.ic-details-dialog :is(button,select,textarea):focus-visible{outline:2px solid var(--text-link,#00a8fc);outline-offset:2px}
.ic-details-dialog dl{margin:16px 0}.ic-details-row{display:grid;grid-template-columns:minmax(0,150px) minmax(0,1fr) 32px;align-items:start;gap:8px;padding:8px 0;border-bottom:1px solid var(--ic-tool-border);user-select:text}.ic-details-row dt{color:var(--ic-tool-muted)}.ic-details-row dd{margin:0;white-space:pre-wrap;overflow-wrap:anywhere;min-width:0}.ic-details-dialog select,.ic-details-dialog textarea{box-sizing:border-box;max-width:100%;width:100%;background:var(--ic-tool-secondary);color:var(--ic-tool-text);border:1px solid var(--ic-tool-border);border-radius:4px;padding:8px;font:inherit}.ic-details-dialog textarea{min-height:120px;resize:vertical;user-select:text}.ic-details-dialog [role=status]{color:var(--ic-tool-muted)}@media(max-width:480px){.ic-details-row{grid-template-columns:minmax(0,1fr) 32px}.ic-details-row dt{grid-column:1}.ic-details-row dd{grid-column:1}.ic-details-row button{grid-column:2;grid-row:1 / span 2}}
.ic-details-dialog h3{font-size:16px;margin:20px 0 8px}.ic-details-dialog summary{cursor:pointer;padding:12px 0}.ic-details-summary p{margin:8px 0}.ic-details-summary{margin:16px 0}.ic-details-dialog details dl{margin-top:0}
.ic-details-dialog summary:focus-visible{outline:2px solid var(--text-link,#00a8fc);outline-offset:2px}
`;

const textValue = value => value == null ? unavailable : typeof value === "object" ? JSON.stringify(value, null, 2) : String(value);
const rowsOf = (data, prefix = "") => Object.entries(data).flatMap(([key, value]) => {
  const field = prefix ? `${prefix}.${key}` : key;
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? rowsOf(value, field) : [[field, textValue(value)]];
});
const labels = {
  observation: "Observation context", action: "Observed action", sequence: "Event sequence",
  id: "ID", username: "Username", display_name: "Display name", avatar_url: "Avatar URL",
  name: "Name", kind: "Type", timestamp: "Observed at (Unix ms)", iso: "Observed at (UTC)", local: "Observed at (local time)",
  reconnect_of: "Linked leave event sequence", reconnect_gap_ms: "Observed reconnect gap",
  call_end_status: "Call observation status", probable_reconnect: "Reconnect assessment", raw_leave_status: "Linked leave availability",
  raw_leave_event: "Retained leave event", linked_probable_reconnect_events: "Linked probable reconnect events",
  observed_duration_ms: "Observed duration", baseline_count: "Participants already present",
  unique_retained_participants: "Unique retained participants", peak_participants: "Peak participants",
  joined_count: "Observed joins", left_count: "Observed leaves", probable_reconnections: "Probable reconnects",
  participant_ids: "Retained participant IDs", participant_ids_truncated: "This call's participant IDs truncated",
  other_participant_ids_truncated: "Comparison call's participant IDs truncated", retention: "Retention context",
  delta_direction: "Comparison direction", shared_participant_ids: "Shared retained participant IDs",
  only_this_participant_ids: "Retained participant IDs only in this call", only_other_participant_ids: "Retained participant IDs only in comparison call",
  shared_participant_count: "Shared retained participants", only_this_participant_count: "Retained participants only in this call",
  only_other_participant_count: "Retained participants only in comparison call",
};
const contexts = { user: "User", channel: "Channel", call: "Call", other_call: "Comparison call", raw_leave_event: "Retained leave event" };
const fieldLabel = field => {
  const parts = field.split(".");
  const leaf = parts.pop();
  const parent = parts.at(-1);
  const date = { started_at: "Started at", ended_at: "Ended at" }[parent];
  if (date) return `${parts.includes("other_call") ? "Comparison call" : "Call"} ${date.toLowerCase()} (${({ iso: "UTC", local: "local time", timestamp: "Unix ms" })[leaf]})`;
  const context = parts.filter(part => contexts[part]).map((part, index) => index ? contexts[part].toLowerCase() : contexts[part]).join(" ");
  const label = labels[leaf] || leaf.replaceAll("_", " ");
  if (parts.includes("deltas")) return `Change in ${label.toLowerCase()}`;
  if (["id", "name", "kind", "username", "display_name", "avatar_url"].includes(leaf)) return `${context} ${label === "ID" ? label : label.toLowerCase()}`.trim();
  return parent === "raw_leave_event" ? `Retained leave event: ${label.toLowerCase()}` : label;
};
const formatDuration = value => {
  if (typeof value !== "number" || !Number.isFinite(value)) return unavailable;
  const sign = value < 0 ? "-" : "";
  const seconds = Math.round(Math.abs(value)) / 1000;
  if (seconds < 60) return `${sign}${Number(seconds.toFixed(3))} sec`;
  if (seconds < 3600) return `${sign}${Math.floor(seconds / 60)} min${seconds % 60 ? ` ${Number((seconds % 60).toFixed(3))} sec` : ""}`;
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor(seconds % 3600 / 60);
  const remainder = Number((seconds % 60).toFixed(3));
  return `${sign}${hours}:${String(minutes).padStart(2, "0")}:${remainder < 10 ? "0" : ""}${remainder}`;
};
const visibleValue = (field, value) => field.endsWith("_ms") && value !== unavailable ? formatDuration(Number(value)) : value;

export function mountCallDetails({ identity, watch } = {}) {
  let root = null;
  let restoreFocus = null;
  let disposed = false;
  let generation = 0;
  const node = (tag, text) => {
    const element = document.createElement(tag);
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const button = (icon, label, action) => {
    const element = node("button");
    element.type = "button";
    element.title = label;
    element.setAttribute("aria-label", label);
    const image = createElement(icon, { "aria-hidden": "true" });
    image.setAttribute("width", "18");
    image.setAttribute("height", "18");
    element.append(image);
    element.addEventListener("click", action);
    return element;
  };
  const controls = () => [...(root?.querySelectorAll("button,select,textarea,summary") || [])].filter(element =>
    !element.disabled && ![...root.querySelectorAll("details")].some(details => !details.hasAttribute("open") && details.contains(element) && element !== details.querySelector("summary")));
  function close() {
    generation++;
    document.removeEventListener("keydown", keyboard, true);
    document.removeEventListener("focusin", focusInside, true);
    root?.remove();
    root = null;
    const previous = restoreFocus;
    restoreFocus = null;
    if (previous?.isConnected) previous.focus();
  }
  function focusInside(event) {
    if (root && !root.contains(event.target)) controls()[0]?.focus();
  }
  function keyboard(event) {
    if (!root || !["Escape", "Tab"].includes(event.key)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.key === "Escape") return close();
    const elements = controls();
    const index = elements.indexOf(document.activeElement);
    const next = index < 0 ? (event.shiftKey ? elements.length - 1 : 0)
      : (index + (event.shiftKey ? -1 : 1) + elements.length) % elements.length;
    elements[next]?.focus();
  }
  function open(title, initialData, user = null, comparisons = []) {
    if (disposed) return;
    const previousFocus = root ? restoreFocus : document.activeElement;
    close();
    restoreFocus = previousFocus?.isConnected ? previousFocus
      : document.activeElement?.isConnected ? document.activeElement : document.body;
    const currentGeneration = generation;
    let data = initialData;
    root = node("div");
    root.className = "ic-details-root ic-user-tool";
    root.dataset.intellicordOwned = "";
    const dialog = node("section");
    dialog.className = "ic-details-dialog";
    dialog.dataset.intellicordOwned = "";
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.setAttribute("aria-label", title);
    const header = node("header");
    const status = node("p", "");
    status.setAttribute("role", "status");
    const fallbackHost = node("div");
    let copyRequest = 0;
    async function copy(value) {
      const request = ++copyRequest;
      fallbackHost.replaceChildren();
      try {
        if (!globalThis.navigator?.clipboard?.writeText) throw new Error("Clipboard unavailable");
        await globalThis.navigator.clipboard.writeText(value);
        if (generation === currentGeneration && request === copyRequest) status.textContent = "Copied";
      } catch {
        if (generation !== currentGeneration || request !== copyRequest) return;
        status.textContent = "Clipboard unavailable";
        const fallback = node("textarea");
        fallback.readOnly = true;
        fallback.setAttribute("aria-label", "Copy fallback");
        fallback.value = value;
        fallbackHost.replaceChildren(fallback);
        fallback.focus();
        fallback.select?.();
      }
    }
    const dismiss = button(X, `Close ${title}`, close);
    header.append(node("h2", title),
      button(Copy, "Copy all text", () => copy(rowsOf(data).map(([field, value]) => `${field}: ${value}`).join("\n"))),
      button(FileJson, "Copy all JSON", () => copy(JSON.stringify(data, null, 2))));
    for (const [tool, icon, label] of [[identity, Users, "Identity Lens"], [watch, Radio, "Voice Watch"]]) {
      if (user?.id && typeof tool?.open === "function") header.append(button(icon, label, () => { close(); tool.open(user.id, user); }));
    }
    header.append(dismiss);
    const rows = node("div");
    function render() {
      rows.replaceChildren();
      let host = rows;
      if ("observed_duration_ms" in data) {
        const summary = node("section");
        summary.className = "ic-details-summary";
        summary.setAttribute("aria-label", "Call summary");
        summary.append(node("h3", textValue(data.call.channel.name)),
          node("p", `Observed duration: ${formatDuration(data.observed_duration_ms)}. Ended at (local time): ${textValue(data.call.ended_at.local)}.`),
          node("p", `${textValue(data.unique_retained_participants)} unique retained participants; peak ${textValue(data.peak_participants)}.`),
          node("p", `${textValue(data.baseline_count)} participants already present when observation began (not join events).`),
          node("p", `${textValue(data.joined_count)} observed joins; ${textValue(data.left_count)} observed leaves; ${textValue(data.probable_reconnections)} probable reconnects.`),
          node("p", data.retention));
        if (data.comparison) summary.append(node("h3", "Comparison"),
          node("p", `Call ${data.comparison.other_call.id}: ${data.comparison.shared_participant_count} shared retained participants; ${data.comparison.only_this_participant_count} only in this call; ${data.comparison.only_other_participant_count} only in comparison call.`),
          node("p", `Observed duration change: ${formatDuration(data.comparison.deltas.observed_duration_ms)} (this call minus comparison call).`),
          node("p", data.comparison.retention));
        const details = node("details");
        details.append(node("summary", "All retained details"));
        rows.append(summary, details);
        host = details;
      }
      let section = null;
      let list = null;
      for (const [field, value] of rowsOf(data)) {
        const group = field.startsWith("comparison.") ? "Comparison details" : field.startsWith("call.") ? "Call details"
          : field.startsWith("raw_leave") || field.startsWith("linked_probable") ? "Linked observations" : "Observation details";
        if (section !== group) {
          section = group;
          list = node("dl");
          host.append(node("h3", group), list);
        }
        const row = node("div");
        row.className = "ic-details-row";
        row.dataset.field = field;
        row.append(node("dt", fieldLabel(field)), node("dd", visibleValue(field, value)), button(Copy, `Copy ${field}`, () => copy(value)));
        list.append(row);
      }
    }
    dialog.append(header);
    if (comparisons.length) {
      const label = node("label", "Comparison call");
      const picker = node("select");
      picker.setAttribute("aria-label", "Comparison call");
      for (const comparison of comparisons) {
        const option = node("option", `Call ${comparison.other_call.id} / ${comparison.other_call.started_at.iso || unavailable}`);
        option.value = String(comparison.other_call.id);
        picker.append(option);
      }
      const selected = comparisons.find(comparison => comparison.other_call.started_at.timestamp < initialData.call.started_at.timestamp) || comparisons[0];
      [...picker.options].find(option => option.value === String(selected.other_call.id)).selected = true;
      data = { ...initialData, comparison: selected };
      picker.addEventListener("change", () => {
        copyRequest++;
        fallbackHost.replaceChildren();
        status.textContent = "";
        data = { ...initialData, comparison: comparisons.find(comparison => String(comparison.other_call.id) === picker.value) };
        render();
      });
      label.append(picker);
      dialog.append(label);
    }
    render();
    dialog.append(rows, status, fallbackHost);
    root.append(node("style", styles), dialog);
    root.addEventListener("click", event => { if (event.target === root) close(); });
    document.body.append(root);
    document.addEventListener("keydown", keyboard, true);
    document.addEventListener("focusin", focusInside, true);
    dismiss.focus();
  }
  return {
    event(event, call = null, events = []) { if (!disposed) open("Event details", eventDetails(event, call, events), userData(event?.user)); },
    recap(call, archives = []) {
      if (disposed) return;
      const data = recapDetails(call);
      if (!data) return;
      const seen = new Set();
      const comparisons = archives.map(other => compareRecaps(call, other)).filter(comparison => {
        if (!comparison || seen.has(comparison.other_call.id)) return false;
        seen.add(comparison.other_call.id);
        return true;
      }).sort((left, right) => right.other_call.started_at.timestamp - left.other_call.started_at.timestamp);
      open("Call recap", data, null, comparisons);
    },
    dispose() { if (disposed) return; disposed = true; close(); },
  };
}