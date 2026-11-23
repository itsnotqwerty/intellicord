const keys = ["schema_version", "source", "status", "configured", "pending", "finished", "cleanup"];
const exact = value => value !== null && typeof value === "object" && !Array.isArray(value)
  && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
const bounded = value => Number.isSafeInteger(value) && value >= 0 && value <= 2;

export function readSessionSensors(value) {
  if (!exact(value) || value.schema_version !== 1 || value.source !== "owner_lifecycle_aggregate"
    || !["sensor_free", "pending", "settled", "attention"].includes(value.status)
    || ![value.configured, value.pending, value.finished].every(bounded)
    || value.pending + value.finished > value.configured
    || !["not_applicable", "pending", "confirmed", "unconfirmed"].includes(value.cleanup)) return null;
  if (value.status === "sensor_free"
    && (value.configured !== 0 || value.pending !== 0 || value.finished !== 0 || value.cleanup !== "not_applicable")) return null;
  if (value.status === "pending"
    && (value.configured === 0 || value.pending === 0 || value.cleanup !== "pending")) return null;
  if (value.status === "settled"
    && (value.configured === 0 || value.pending !== 0 || value.finished !== value.configured
      || value.cleanup !== "confirmed")) return null;
  if (value.status === "attention"
    && (value.configured === 0 || value.cleanup === "not_applicable")) return null;
  return { ...value };
}

export function sessionSensorsFeature(value) {
  const session = readSessionSensors(value);
  if (!session) return { state: "unsupported", detail: "Combined sensor owner status unavailable" };
  if (session.status === "sensor_free") {
    return { state: "ready", detail: "No optional sensors configured" };
  }
  const owners = `${session.finished}/${session.configured} owners finished`;
  if (session.status === "pending") {
    return { state: "waiting", detail: `${owners}; ${session.pending} pending; cleanup pending` };
  }
  if (session.status === "settled") {
    return { state: "ready", detail: `${owners}; cleanup confirmed` };
  }
  return { state: "error", detail: `${owners}; owner attention required; cleanup ${session.cleanup.replace("_", " ")}` };
}