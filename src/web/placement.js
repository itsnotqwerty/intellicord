export function callPlacement(rect, viewport) {
  if (!rect || rect.width <= 0 || rect.height <= 0 || rect.bottom <= 0 || rect.top >= viewport.height) return null;
  const left = Math.max(14, Math.min(rect.left + 14, viewport.width - 52));
  const top = Math.max(10, Math.min(rect.bottom - 52, viewport.height - 52));
  return {
    left,
    top,
    panelLeft: Math.min(0, viewport.width - 14 - left - Math.min(400, viewport.width - 28)),
    panelHeight: Math.max(0, top - 20),
  };
}