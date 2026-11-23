const fields = ["schema_version", "source", "status", "sensor_running", "valid_for_ms", "sample"];
const counters = ["accepted", "rejected_record", "rejected_clock", "ring_dropped"];
const inactive = new Set(["not_connected", "no_recent_sample", "generated_rejected", "invalid_sample"]);
const exact = (value, keys) => value !== null && typeof value === "object" && !Array.isArray(value)
  && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));

export function readOwnerOutcome(value) {
  if (!exact(value, ["schema_version", "source", "status", "result"]) || value.schema_version !== 1
    || value.source !== "capture_owner_outcome") return null;
  if (["not_configured", "awaiting_completion", "unavailable"].includes(value.status)) {
    return value.result === null ? { ...value } : null;
  }
  const result = value.result;
  if (value.status !== "finished" || !exact(result, ["outcome", "collector_started", "collector_reaped", "endpoint_removed"])
    || !["completed", "collector_fault", "invalid_duration", "spawn_failed", "cancelled", "deadline",
      "invalid_stream", "read_failed", "wait_failed", "cleanup_failed"].includes(result.outcome)
    || ![result.collector_started, result.collector_reaped, result.endpoint_removed].every(flag => typeof flag === "boolean")
    || (result.collector_reaped && !result.collector_started)
    || (["completed", "collector_fault"].includes(result.outcome) && (!result.collector_started || !result.collector_reaped))
    || (["invalid_duration", "spawn_failed"].includes(result.outcome) && result.collector_started)) return null;
  return { ...value, result: { ...result } };
}

export function ownerFeature(value) {
  const owner = readOwnerOutcome(value);
  if (!owner || owner.status === "unavailable") return { state: "error", detail: "Owner outcome unavailable; activity unverified" };
  if (owner.status === "not_configured") return { state: "ready", detail: "Not configured; no process collector requested" };
  if (owner.status === "awaiting_completion") return { state: "waiting", detail: "Awaiting owner completion; activity unverified" };
  const result = owner.result;
  const clean = result.endpoint_removed && (!result.collector_started || result.collector_reaped);
  return { state: clean && ["completed", "cancelled"].includes(result.outcome) ? "ready" : "error",
    detail: `Owner finished: ${result.outcome.replaceAll("_", " ")}; cleanup ${clean ? "confirmed" : "unconfirmed"}` };
}

export function progressFeature(value) {
  if (!value) return { state: "unsupported", detail: "Automatic process telemetry unavailable" };
  if (value.status === "not_connected") return { state: "ready", detail: "Not connected; no process collector requested" };
  if (value.status === "no_recent_sample") return { state: "waiting", detail: "No recent kernel counters; liveness not established" };
  if (value.status === "recent_kernel_sample" && value.sample) {
    return { state: "ready", detail: "Recent kernel counters received; liveness not established" };
  }
  if (["generated_rejected", "invalid_sample"].includes(value.status)) {
    return { state: "error", detail: "Process telemetry sample rejected" };
  }
  return { state: "unsupported", detail: "Automatic process telemetry unavailable" };
}

export function createProgressHandoff(now = () => performance.now()) {
  let current = null;
  let deadline = 0;
  let previous = 0;
  const clear = () => { current = null; deadline = 0; };
  function clock() {
    const stamp = now();
    if (!Number.isFinite(stamp) || stamp < previous) { clear(); return null; }
    previous = stamp;
    return stamp;
  }
  function copy(stamp) {
    if (!current) return null;
    const remaining = Math.floor(deadline - stamp);
    if (current.status === "recent_kernel_sample" && remaining <= 0) {
      current = { ...current, status: "no_recent_sample", valid_for_ms: 0, sample: null };
    }
    return { ...current, valid_for_ms: current.sample ? remaining : 0,
      sample: current.sample ? { ...current.sample } : null };
  }
  return {
    clear,
    accept(value, requestedAt) {
      clear();
      const stamp = clock();
      if (stamp === null || !Number.isFinite(requestedAt) || requestedAt < 0 || requestedAt > stamp
        || !exact(value, fields) || value.schema_version !== 1
        || value.source !== "owner_progress_subscription" || value.sensor_running !== false) return null;
      if (inactive.has(value.status)) {
        if (value.valid_for_ms !== 0 || value.sample !== null) return null;
      } else if (value.status === "recent_kernel_sample") {
        if (!Number.isSafeInteger(value.valid_for_ms) || value.valid_for_ms < 1 || value.valid_for_ms > 2000
          || !exact(value.sample, ["schema_version", ...counters]) || value.sample.schema_version !== 1
          || !counters.every(key => Number.isSafeInteger(value.sample[key]) && value.sample[key] >= 0)) return null;
      } else return null;
      deadline = requestedAt + value.valid_for_ms;
      current = { schema_version: 1, source: "owner_progress_subscription", status: value.status,
        sensor_running: false, valid_for_ms: value.valid_for_ms,
        sample: value.sample ? { schema_version: 1, ...Object.fromEntries(counters.map(key => [key, value.sample[key]])) } : null };
      return copy(stamp);
    },
    read() { const stamp = clock(); return stamp === null ? null : copy(stamp); },
  };
}