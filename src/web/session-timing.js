const exact = (value, keys) => value !== null && typeof value === "object" && !Array.isArray(value)
  && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));

export function readSessionTiming(value) {
  const keys = ["schema_version", "source", "voice_receipt_clock", "acoustic_receipt_clock",
    "acoustic_frame_clock", "process_timing", "cross_sensor_correlation"];
  if (!exact(value, keys) || value.schema_version !== 1 || value.source !== "backend_session_clock_inventory"
    || value.voice_receipt_clock !== "backend_session_monotonic"
    || !["backend_session_monotonic", "unavailable"].includes(value.acoustic_receipt_clock)
    || !["backend_session_interval", "capture_relative", "unavailable"].includes(value.acoustic_frame_clock)
    || !["receipt_freshness_only", "unavailable"].includes(value.process_timing)
    || value.cross_sensor_correlation !== "unavailable"
    || value.acoustic_receipt_clock === "unavailable" && value.acoustic_frame_clock !== "unavailable"
    || value.acoustic_frame_clock !== "unavailable" && value.acoustic_receipt_clock !== "backend_session_monotonic") return null;
  return { ...value };
}

export function sessionTimingFeature(value) {
  const timing = readSessionTiming(value);
  if (!timing) return { state: "unsupported", detail: "Session timing inventory unavailable" };
  if (timing.acoustic_frame_clock === "backend_session_interval") return {
    state: "ready", detail: "Acoustic frames bounded in backend session time; calibrated correlation unavailable",
  };
  if (timing.acoustic_frame_clock === "capture_relative") return {
    state: "ready", detail: "Receipt clocks shared; acoustic frames remain capture-relative",
  };
  return { state: "ready", detail: timing.process_timing === "receipt_freshness_only"
    ? "Voice receipt clock available; process timing is freshness-only"
    : "Voice receipt clock available; optional sensor timing unavailable" };
}
