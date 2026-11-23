import { createElement, ShieldAlert, X } from "lucide";
import { userToolTheme } from "./user-tool-theme.js";

const keys = ["mute", "soundboard", "video", "ignore", "block", "moderation"];
let inFlight = null;
const panels = [];
const descriptions = {
  mute: "Mute this user locally for you.",
  soundboard: "Mute this user's soundboard locally for you.",
  video: "Disable this user's video locally for you only.",
  ignore: "Ignore this user account-wide.",
  block: "Block this user account-wide.",
  moderation: "Server moderation affects this user in the server.",
};
const moderationImpact = {
  ban: "Ban removes this user from the server and prevents rejoining. Its consequences may be irreversible.",
  kick: "Kick removes this user from the server but does not prevent rejoining. Its consequences may be irreversible.",
  timeout: "Timeout restricts this user's participation in the server.",
};
const styles = userToolTheme + `
.ic-obliterate-root{position:fixed;inset:0;z-index:1004;display:flex;align-items:center;justify-content:center;padding:16px;box-sizing:border-box;background:rgba(0,0,0,.65)}
.ic-obliterate-dialog{box-sizing:border-box;width:560px;max-width:100%;max-height:calc(100vh - 32px);max-height:calc(100dvh - 32px);overflow:auto;padding:20px;border-radius:8px;background:var(--ic-tool-bg);color:var(--ic-tool-text);font-family:var(--font-primary,Whitney,sans-serif);font-size:14px;letter-spacing:0;overflow-wrap:anywhere}
.ic-obliterate-dialog header,.ic-obliterate-dialog footer{display:flex;align-items:center;gap:12px}.ic-obliterate-dialog h2{flex:1;margin:0;font-size:20px}.ic-obliterate-dialog button{min-height:32px;padding:6px 12px;border:0;border-radius:4px;background:var(--ic-tool-secondary);color:inherit;cursor:pointer}.ic-obliterate-dialog button:disabled{opacity:.5;cursor:default}.ic-obliterate-dialog button[data-confirm]{background:var(--button-danger-background,#b42332);color:#fff}.ic-obliterate-dialog header button{display:grid;place-items:center;width:32px;padding:0;flex-shrink:0}.ic-obliterate-dialog svg{flex-shrink:0}.ic-obliterate-dialog :is(button,input):focus-visible{outline:2px solid var(--text-link,#00a8fc);outline-offset:2px}
.ic-obliterate-dialog ol{list-style:none;padding:0;margin:16px 0}.ic-obliterate-dialog li{padding:10px 0;border-bottom:1px solid var(--ic-tool-border)}.ic-obliterate-dialog li p{margin:4px 0}.ic-obliterate-dialog small{display:block;color:var(--ic-tool-muted)}.ic-obliterate-dialog label{display:flex;align-items:center;gap:8px;margin:16px 0}.ic-obliterate-dialog input{flex-shrink:0;width:18px;height:18px}.ic-obliterate-dialog footer{justify-content:flex-end;flex-wrap:wrap}.ic-obliterate-dialog [role=status]{white-space:pre-wrap}
`;

function snapshot(value, request) {
  if (!value || value.target?.id !== request.userId || typeof value.target.name !== "string"
    || value.context?.channelId !== request.channelId || !Array.isArray(value.actions)) {
    throw new Error("Plan does not match the requested user and channel.");
  }
  const context = value.context;
  if (![context.guildId, context.guildName].every(field => field == null || typeof field === "string")) {
    throw new Error("Invalid server context.");
  }
  const actions = keys.map(key => {
    const matches = value.actions.filter(action => action?.key === key);
    if (matches.length > 1) throw new Error(`Duplicate action: ${key}`);
    const action = matches[0];
    if (!action) return { key, label: key, status: "unavailable", reason: "Not provided by the adapter." };
    if (typeof action.label !== "string" || !["ready", "done", "unavailable"].includes(action.status)
      || (action.reason != null && typeof action.reason !== "string")
      || (action.severity != null && !["ban", "kick", "timeout"].includes(action.severity))
      || (key === "moderation" && action.status === "ready" && !action.severity)) {
      throw new Error(`Invalid action: ${key}`);
    }
    return { key, label: action.label, status: action.status,
      ...(action.reason ? { reason: action.reason } : {}),
      ...(action.severity ? { severity: action.severity } : {}) };
  });
  return { target: { id: value.target.id, name: value.target.name },
    context: { channelId: context.channelId, guildId: context.guildId ?? null, guildName: context.guildName ?? null }, actions };
}

export function mountObliteratePanel(adapter) {
  let root = null;
  let dialog, cancel, dismiss, confirm, checkbox, status, content;
  let previousFocus = null;
  let request = null;
  let plan = null;
  let busy = false;
  let executed = false;
  let disposed = false;
  let generation = 0;
  const results = new Map();
  const node = (tag, text) => {
    const element = document.createElement(tag);
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const controls = () => [...root.querySelectorAll("button,input")].filter(element => !element.disabled);
  function focusSafe() { if (root && panels.at(-1) === root) (cancel && !cancel.disabled ? cancel : dialog)?.focus(); }
  function focusInside(event) { if (root && panels.at(-1) === root && !root.contains(event.target)) focusSafe(); }
  function close(force = false) {
    if (busy && !force) return;
    generation++;
    document.removeEventListener("keydown", keyboard, true);
    document.removeEventListener("focusin", focusInside, true);
    const index = panels.indexOf(root);
    if (index >= 0) panels.splice(index, 1);
    root?.remove();
    root = null;
    const origin = previousFocus;
    previousFocus = null;
    if (origin?.isConnected) origin.focus();
  }
  function keyboard(event) {
    if (!root || panels.at(-1) !== root || !["Escape", "Tab"].includes(event.key)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.key === "Escape") close();
    else {
      const elements = controls();
      const index = elements.indexOf(document.activeElement);
      const next = index < 0 ? (event.shiftKey ? elements.length - 1 : 0)
        : (index + (event.shiftKey ? -1 : 1) + elements.length) % elements.length;
      if (elements.length) elements[next].focus();
      else dialog.focus();
    }
  }
  function updateControls() {
    cancel.disabled = dismiss.disabled = busy;
    if (checkbox) checkbox.disabled = busy || executed;
    confirm.disabled = busy || executed || !plan?.actions.some(action => action.status === "ready")
      || (plan.actions.some(action => action.status === "unavailable") && !checkbox?.checked);
    dialog.setAttribute("aria-busy", String(busy));
    if (busy) dialog.focus();
  }
  function renderPlan() {
    content.replaceChildren();
    results.clear();
    checkbox = null;
    content.append(node("p", `Target: ${plan.target.name} (ID: ${plan.target.id})`),
      node("p", `Channel ID: ${plan.context.channelId} | ${plan.context.guildId
        ? `Server: ${plan.context.guildName ?? "Unknown name"} (ID: ${plan.context.guildId})` : "No verified server context"}`));
    const list = node("ol");
    for (const action of plan.actions) {
      const row = node("li");
      row.dataset.action = action.key;
      const impact = action.key === "moderation" && action.severity
        ? `Selected server action: ${action.severity}. ${moderationImpact[action.severity]}` : descriptions[action.key];
      const result = node("p", `${action.status}${action.status === "unavailable"
        ? `: ${action.reason || "No availability reason was provided by the adapter."}` : action.reason ? `: ${action.reason}` : ""}`);
      result.dataset.result = action.key;
      result.setAttribute("role", "status");
      results.set(action.key, result);
      row.append(node("strong", action.label), node("small", impact), result);
      list.append(row);
    }
    content.append(list);
    if (plan.actions.some(action => action.status === "unavailable")) {
      const label = node("label");
      checkbox = node("input");
      checkbox.type = "checkbox";
      checkbox.checked = false;
      checkbox.addEventListener("change", updateControls);
      label.append(checkbox, node("span", "Proceed with available actions only"));
      content.append(label);
    }
    updateControls();
  }
  async function execute() {
    if (disposed || !root || busy || executed || confirm.disabled) return;
    if (inFlight) {
      status.textContent = "Another Obliterate operation is still in progress. No actions were executed here.";
      return;
    }
    const operation = {};
    inFlight = operation;
    const currentGeneration = generation;
    const active = () => !disposed && root && generation === currentGeneration;
    busy = true;
    updateControls();
    status.textContent = "Checking the plan again...";
    try {
      const revised = snapshot(await adapter.plan({ ...request }), request);
      if (!active()) return;
      if (JSON.stringify(revised) !== JSON.stringify(plan)) {
        plan = revised;
        busy = false;
        renderPlan();
        status.textContent = "The plan changed. Review the revised plan and explicitly confirm again. No actions were executed.";
        focusSafe();
        return;
      }
      executed = true;
      status.textContent = "Executing actions in order. Completed actions will not be rolled back.";
      let failures = 0;
      for (const action of plan.actions) {
        if (!active()) return;
        if (action.status !== "ready") continue;
        const resultNode = results.get(action.key);
        resultNode.textContent = "Running...";
        try {
          const result = await adapter.executeAction({ ...request }, action.key,
            action.key === "moderation" ? { ...action, guildId: plan.context.guildId } : { ...action });
          if (!active()) return;
          if (!result || !["done", "submitted", "unverified"].includes(result.status)) throw new Error("Invalid action result.");
          resultNode.textContent = `${result.status}${typeof result.detail === "string" && result.detail ? `: ${result.detail}` : ""}`;
        } catch (error) {
          if (!active()) return;
          failures++;
          resultNode.textContent = `failed: ${error instanceof Error ? error.message : String(error)}. Not retried.`;
        }
      }
      status.textContent = `${failures ? "Finished with failures; results may be partial." : "Finished; review each action's result."} Submitted means requested, not verified complete. Unverified means the outcome could not be verified. No automatic retries or rollback.`;
    } catch (error) {
      if (!active()) return;
      plan = null;
      status.textContent = `Unable to verify the plan: ${error instanceof Error ? error.message : String(error)}. No actions were executed. Close and reopen to review a new plan.`;
    } finally {
      if (inFlight === operation) inFlight = null;
      if (active()) {
        busy = false;
        updateControls();
        focusSafe();
      }
    }
  }
  async function open(context) {
    if (disposed || busy || inFlight) return;
    if (!context || ![context.userId, context.channelId].every(value => typeof value === "string" && /^\d{1,20}$/.test(value) && BigInt(value) > 0n)) {
      throw new Error("User and channel IDs must be positive numeric strings.");
    }
    if (root) close();
    request = { userId: context.userId, channelId: context.channelId };
    plan = null;
    executed = false;
    previousFocus = document.activeElement;
    root = node("div");
    root.className = "ic-obliterate-root ic-user-tool";
    root.dataset.intellicordOwned = "";
    dialog = node("section");
    dialog.className = "ic-obliterate-dialog";
    dialog.dataset.intellicordOwned = "";
    dialog.tabIndex = -1;
    dialog.setAttribute("role", "alertdialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.setAttribute("aria-label", "Obliterate");
    const header = node("header");
    const shield = createElement(ShieldAlert, { "aria-hidden": "true", width: 20, height: 20 });
    dismiss = node("button");
    dismiss.type = "button";
    dismiss.title = "Close Obliterate";
    dismiss.setAttribute("aria-label", "Close Obliterate");
    dismiss.append(createElement(X, { "aria-hidden": "true", width: 18, height: 18 }));
    dismiss.addEventListener("click", () => close());
    header.append(shield, node("h2", "Obliterate"), dismiss);
    content = node("div");
    status = node("p", "Loading read-only plan...");
    status.setAttribute("role", "status");
    const footer = node("footer");
    cancel = node("button", "Cancel");
    cancel.type = "button";
    cancel.addEventListener("click", () => close());
    confirm = node("button", "Obliterate");
    confirm.type = "button";
    confirm.dataset.confirm = "";
    confirm.disabled = true;
    confirm.addEventListener("click", execute);
    footer.append(cancel, confirm);
    dialog.append(header, node("p", "Obliterate may kick or ban this user when your server permissions allow it. No messages are deleted. Local controls affect only you; ignore and block affect your account. There is no automatic rollback."), content, status, footer);
    root.append(node("style", styles), dialog);
    root.addEventListener("click", event => { if (event.target === root) close(); });
    document.body.append(root);
    panels.push(root);
    document.addEventListener("keydown", keyboard, true);
    document.addEventListener("focusin", focusInside, true);
    cancel.focus();
    const currentGeneration = generation;
    const openingRequest = { ...request };
    try {
      const initial = snapshot(await adapter.plan({ ...openingRequest }), openingRequest);
      if (disposed || !root || generation !== currentGeneration) return;
      plan = initial;
      renderPlan();
      status.textContent = plan.actions.some(action => action.status === "ready")
        ? "Review all six actions before confirming." : "No actions are ready. Nothing will be executed.";
    } catch (error) {
      if (disposed || !root || generation !== currentGeneration) return;
      status.textContent = `Unable to load the plan: ${error instanceof Error ? error.message : String(error)}. No actions were executed.`;
    }
  }
  return { open, dispose() { if (disposed) return; disposed = true; close(true); } };
}