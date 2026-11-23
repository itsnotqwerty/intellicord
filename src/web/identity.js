import { createElement, Copy, ExternalLink, Save, X } from "lucide";
import { userToolTheme } from "./user-tool-theme.js";

const fields = ["id", "username", "globalName", "display_name", "discriminator", "bot", "system", "avatar", "banner", "accentColor", "publicFlags"];
const validId = value => typeof value === "string" && /^\d{1,20}$/.test(value) && BigInt(value) > 0n && BigInt(value) <= 18446744073709551615n;

function mediaUrl(value, userId, kind) {
  if (typeof value !== "string") return null;
  if (/^(?:a_)?[a-f0-9]+$/i.test(value)) {
    return `https://cdn.discordapp.com/${kind}/${userId}/${value}.${value.startsWith("a_") ? "gif" : "png"}?size=128`;
  }
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || !["cdn.discordapp.com", "media.discordapp.net"].includes(url.hostname)
      || url.username || url.password || url.port || url.hash) return null;
    if (!new RegExp(`^/${kind}/${userId}/(?:a_)?[a-f0-9]+\\.(?:png|gif|webp|jpe?g|avif)$`, "i").test(url.pathname)) return null;
    if (!value.startsWith(`${url.origin}${url.pathname}`)) return null;
    return url.href;
  } catch { return null; }
}

export function collectIdentity(user, channel = null) {
  const result = {};
  for (const field of fields) {
    const value = user?.[field];
    result[field] = field === "id" ? (validId(value) ? value : null)
      : ["bot", "system"].includes(field) ? (typeof value === "boolean" ? value : null)
        : ["accentColor", "publicFlags"].includes(field) ? (Number.isSafeInteger(value) && value >= 0 ? value : null)
          : (typeof value === "string" && value.length ? value : null);
  }
  result.createdAt = result.id === null ? null
    : new Date(Number((BigInt(result.id) >> 22n) + 1420070400000n)).toISOString();
  return result;
}

export function mountIdentityLens(stores) {
  let root = null;
  let restoreFocus = null;
  let disposed = false;
  let generation = 0;
  let copyRequest = 0;
  const node = (tag, text) => {
    const element = document.createElement(tag);
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const button = (icon, label) => {
    const element = node("button");
    element.type = "button";
    element.title = label;
    element.setAttribute("aria-label", label);
    element.append(createElement(icon, { width: 18, height: 18, "aria-hidden": "true" }));
    return element;
  };
  function close() {
    generation++;
    root?.remove();
    root = null;
    document.removeEventListener("keydown", keyboard, true);
    document.removeEventListener("focusin", containFocus, true);
    if (restoreFocus?.isConnected) restoreFocus.focus();
    restoreFocus = null;
  }
  function controls() {
    return [...root.querySelectorAll("button:not(:disabled), a[href], textarea")];
  }
  function containFocus(event) {
    if (root && !root.contains(event.target)) controls()[0]?.focus();
  }
  function keyboard(event) {
    if (!root) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopImmediatePropagation();
      close();
    } else if (event.key === "Tab") {
      const available = controls();
      const index = available.indexOf(document.activeElement);
      event.preventDefault();
      event.stopImmediatePropagation();
      available[(index + (event.shiftKey ? -1 : 1) + available.length) % available.length]?.focus();
    }
  }
  function open(userId, fallbackUser = null) {
    if (disposed) return;
    if (!validId(userId)) throw new Error("Invalid Discord user ID");
    let user = null;
    try { user = stores.users?.getUser(userId); } catch {}
    if (user?.id !== userId) user = fallbackUser?.id === userId ? fallbackUser : null;
    const identity = collectIdentity(user);
    identity.id = userId;
    identity.createdAt = collectIdentity({ id: userId }).createdAt;
    if (root) close();
    restoreFocus = document.activeElement;
    const currentGeneration = ++generation;
    root = node("div");
    root.className = "ic-identity-root ic-user-tool";
    root.dataset.intellicordOwned = "";
    root.style.cssText = "position:fixed;inset:0;z-index:1003;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,.65);padding:16px";
    const style = node("style", userToolTheme + `.ic-identity-dialog{box-sizing:border-box;width:440px;max-width:100%;max-height:calc(100vh - 32px);overflow:auto;border-radius:8px;padding:20px;background:var(--ic-tool-bg);color:var(--ic-tool-text);font-family:var(--font-primary,Whitney,sans-serif);font-size:14px}.ic-identity-dialog header{display:flex;align-items:center;gap:8px}.ic-identity-dialog h2{font-size:20px;margin:0;flex:1}.ic-identity-dialog button{display:grid;place-items:center;width:32px;height:32px;flex-shrink:0;border:0;border-radius:4px;background:transparent;color:inherit;cursor:pointer}.ic-identity-dialog button:hover{background:var(--ic-tool-hover)}.ic-identity-dialog button:focus-visible{outline:2px solid var(--text-link,#00a8fc)}.ic-identity-dialog dl{margin:16px 0}.ic-identity-dialog dl div{display:grid;grid-template-columns:120px minmax(0,1fr);gap:12px;padding:7px 0;border-bottom:1px solid var(--ic-tool-border);user-select:text}.ic-identity-dialog dt{color:var(--ic-tool-muted)}.ic-identity-dialog dd{margin:0;overflow-wrap:anywhere}.ic-identity-dialog p{color:var(--ic-tool-muted);line-height:1.4}.ic-identity-dialog img{width:64px;height:64px;border-radius:50%;object-fit:cover}`);
    style.textContent += `.ic-identity-root{box-sizing:border-box}.ic-identity-dialog header{flex-wrap:wrap}.ic-identity-dialog h2{min-width:0;overflow-wrap:anywhere}.ic-identity-dialog dl div{grid-template-columns:minmax(0,1fr) 68px;gap:4px 8px}.ic-identity-dialog dt{grid-column:1}.ic-identity-dialog dd{grid-column:1;min-width:0}.ic-identity-dialog .ic-identity-tools{grid-column:2;grid-row:1 / span 2;display:flex;align-items:center;width:68px;gap:4px}.ic-identity-dialog a{display:grid;place-items:center;width:32px;height:32px;flex-shrink:0;border-radius:4px;color:inherit}.ic-identity-dialog a:hover{background:var(--ic-tool-hover)}.ic-identity-dialog a:focus-visible,.ic-identity-dialog textarea:focus-visible{outline:2px solid var(--text-link,#00a8fc)}.ic-identity-dialog textarea{display:block;box-sizing:border-box;width:100%;max-width:100%;min-height:80px;background:var(--ic-tool-bg);color:var(--ic-tool-text);border:1px solid var(--ic-tool-border);resize:vertical}`;
    const dialog = node("section");
    dialog.className = "ic-identity-dialog";
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.setAttribute("aria-label", "Identity Lens");
    const header = node("header");
    const copyAll = button(Save, "Copy all JSON");
    const dismiss = button(X, "Close Identity Lens");
    const status = node("p", "");
    status.setAttribute("role", "status");
    let fallback = null;
    async function copyValue(value, message, label) {
      const request = ++copyRequest;
      fallback?.remove();
      fallback = null;
      try {
        if (!globalThis.navigator?.clipboard?.writeText) throw new Error("Clipboard unavailable");
        await globalThis.navigator.clipboard.writeText(value);
        if (generation === currentGeneration && request === copyRequest) status.textContent = message;
      } catch {
        if (generation !== currentGeneration || request !== copyRequest) return;
        status.textContent = "Clipboard unavailable";
        fallback = node("textarea");
        fallback.readOnly = true;
        fallback.setAttribute("readonly", "");
        fallback.setAttribute("aria-label", label);
        fallback.value = value;
        dialog.append(fallback);
        fallback.focus();
        fallback.select?.();
      }
    }
    copyAll.addEventListener("click", () => copyValue(JSON.stringify(identity, null, 2), "Identity JSON copied", "Identity JSON to copy"));
    dismiss.addEventListener("click", close);
    header.append(node("h2", "Identity Lens"), copyAll, dismiss);
    dialog.append(header, node("p", "Observed local profile only. Unavailable fields have not been observed locally."));
    const avatarUrl = mediaUrl(identity.avatar, userId, "avatars");
    const bannerUrl = mediaUrl(identity.banner, userId, "banners");
    if (avatarUrl) {
      const image = node("img");
      image.alt = "User avatar";
      image.referrerPolicy = "no-referrer";
      image.addEventListener("error", () => image.remove(), { once: true });
      image.src = avatarUrl;
      dialog.append(image);
    }
    const rows = node("dl");
    const labels = { id: "User ID", username: "Username", globalName: "Global name", display_name: "Display name", discriminator: "Discriminator", bot: "Bot", system: "System", avatar: "Avatar", banner: "Banner", accentColor: "Accent color", publicFlags: "Public badges (raw flags)", createdAt: "Created (UTC)" };
    const values = { ...identity, ...(avatarUrl ? { avatarUrl } : {}), ...(bannerUrl ? { bannerUrl } : {}) };
    labels.avatarUrl = "Avatar URL";
    labels.bannerUrl = "Banner URL";
    for (const [field, value] of Object.entries(values)) {
      const row = node("div");
      row.dataset.field = field;
      row.append(node("dt", labels[field]), node("dd", value === null ? "Unavailable" : String(value)));
      const tools = node("span");
      tools.className = "ic-identity-tools";
      if (value !== null) {
        const fieldCopy = button(Copy, `Copy ${labels[field]}`);
        fieldCopy.addEventListener("click", () => copyValue(String(value), `${labels[field]} copied`, `${labels[field]} to copy`));
        tools.append(fieldCopy);
      }
      if (field === "avatarUrl" || field === "bannerUrl") {
        const link = node("a");
        link.href = value;
        link.target = "_blank";
        link.rel = "noopener noreferrer";
        link.setAttribute("referrerpolicy", "no-referrer");
        link.title = `Open ${labels[field]} externally`;
        link.setAttribute("aria-label", link.title);
        link.append(createElement(ExternalLink, { width: 18, height: 18, "aria-hidden": "true" }));
        tools.append(link);
      }
      row.append(tools);
      rows.append(row);
    }
    dialog.append(rows, status);
    root.append(style, dialog);
    root.addEventListener("click", event => { if (event.target === root) close(); });
    document.body.append(root);
    document.addEventListener("keydown", keyboard, true);
    document.addEventListener("focusin", containFocus, true);
    dismiss.focus();
  }
  return { open, dispose() { if (disposed) return; close(); disposed = true; } };
}