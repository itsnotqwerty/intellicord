const keys = ["schema_version", "source", "status", "owner_integrity", "process", "acoustic",
  "cross_sensor_correlation"];
const exact = value => value !== null && typeof value === "object" && !Array.isArray(value)
  && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));

export function readSessionEvidence(value) {
  if (!exact(value) || value.schema_version !== 1 || value.source !== "backend_session_evidence_aggregate"
    || !["sensor_free", "awaiting_evidence", "partial", "independent_evidence", "degraded"].includes(value.status)
    || !["sensor_free", "pending", "settled", "attention"].includes(value.owner_integrity)
    || !["not_configured", "awaiting_recent_sample", "recent_kernel_sample", "rejected"].includes(value.process)
    || !["not_configured", "awaiting_frame", "frame_available", "processing_loss", "faulted"].includes(value.acoustic)
    || value.cross_sensor_correlation !== "unavailable") return null;
  const degraded = value.owner_integrity === "attention" || value.process === "rejected"
    || ["processing_loss", "faulted"].includes(value.acoustic);
  const available = Number(value.process === "recent_kernel_sample") + Number(value.acoustic === "frame_available");
  const expected = degraded ? "degraded"
    : value.process === "not_configured" && value.acoustic === "not_configured" ? "sensor_free"
      : available === 2 ? "independent_evidence" : available === 1 ? "partial" : "awaiting_evidence";
  if (value.status !== expected || value.status === "sensor_free" && value.owner_integrity !== "sensor_free") return null;
  return { ...value };
}

export function sessionEvidenceFeature(value) {
  const evidence = readSessionEvidence(value);
  if (!evidence) return { state: "unsupported", detail: "Combined session evidence unavailable" };
  if (evidence.status === "degraded") return { state: "error", detail: "Session evidence integrity degraded" };
  if (evidence.status === "awaiting_evidence") return { state: "waiting", detail: "Configured sensors have no usable evidence yet" };
  if (evidence.status === "partial") return { state: "waiting", detail: "One independent evidence stream is currently usable" };
  if (evidence.status === "independent_evidence") return {
    state: "ready", detail: "Process and acoustic evidence available independently; correlation unavailable",
  };
  return { state: "ready", detail: "No optional sensor evidence configured" };
}