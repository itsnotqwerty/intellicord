export function safeAvatarUrl(value) {
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || !["cdn.discordapp.com", "media.discordapp.net"].includes(url.hostname) || url.username || url.password || url.port) return null;
    if (!/^\/(?:avatars\/\d+\/|embed\/avatars\/|guilds\/\d+\/users\/\d+\/avatars\/)/.test(url.pathname)) return null;
    return url.href;
  } catch { return null; }
}

export function profilePicture(user) {
  const avatar = document.createElement("div");
  avatar.className = "cm-avatar";
  avatar.setAttribute("aria-hidden", "true");
  avatar.textContent = user.display_name.slice(0, 2).toUpperCase();
  const url = safeAvatarUrl(user.avatar_url);
  if (url) {
    const image = document.createElement("img");
    image.alt = "";
    image.width = 32;
    image.height = 32;
    image.loading = "lazy";
    image.referrerPolicy = "no-referrer";
    image.addEventListener("load", () => { avatar.replaceChildren(image); }, { once: true });
    image.addEventListener("error", () => { image.remove(); }, { once: true });
    image.src = url;
    avatar.append(image);
  }
  return avatar;
}