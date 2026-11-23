const exact = (value, keys) => value !== null && typeof value === "object" && !Array.isArray(value)
  && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
const count = value => Number.isSafeInteger(value) && value >= 0;
const level = value => Number.isFinite(value) && value >= 0 && value <= 1;
const counts = value => exact(value, ["batches", "frames"]) && count(value.batches) && count(value.frames);
const fault = value => ["unsupported_format", "invalid_length", "invalid_sample", "invalid_time", "output_overflow", "format_changed"]
  .includes(value) || exact(value, ["producer"]) && ["source_lost", "format_changed", "input_overflow"].includes(value.producer)
  || exact(value, ["clock"]) && ["invalid_anchor", "before_anchor", "expired", "overflow"].includes(value.clock);

const processingHealth = value => {
  if (!exact(value, ["schema_version", "status", "capacity_batches", "pending", "processed", "delivered",
    "discarded", "overflow_events", "high_water_batches"]) || value.schema_version !== 1
    || value.capacity_batches !== 8 || !counts(value.pending) || !counts(value.processed)
    || !counts(value.delivered) || !counts(value.discarded) || !count(value.overflow_events)
    || !count(value.high_water_batches) || value.pending.batches > value.capacity_batches
    || value.high_water_batches > value.capacity_batches
    || value.processed.batches !== value.delivered.batches + value.discarded.batches + value.pending.batches
    || value.processed.frames !== value.delivered.frames + value.discarded.frames + value.pending.frames) return null;
  const status = value.status;
  if (!exact(status, status?.state === "faulted" ? ["state", "error"] : ["state"])
    || !["stopped", "active", "faulted"].includes(status.state)
    || status.state === "faulted" && !fault(status.error)) return null;
  return { ...value, status: { ...status }, pending: { ...value.pending }, processed: { ...value.processed },
    delivered: { ...value.delivered }, discarded: { ...value.discarded } };
};

export function readAcousticSummary(value) {
  const summaryKeys = ["schema_version", "source", "status", "frames", "discontinuities", "missing_messages", "last_receipt_ns", "latest"];
  if (!exact(value, value?.schema_version === 4 ? [...summaryKeys, "processing_health"] : summaryKeys)
    || ![1, 2, 3, 4].includes(value.schema_version) || !["generated_pcm", "mixed_output"].includes(value.source)
    || !["starting", "receiving", "stopped", "faulted"].includes(value.status)
    || ![value.frames, value.discontinuities, value.missing_messages, value.last_receipt_ns].every(count)
    || value.discontinuities > value.frames) return null;
  const health = value.schema_version === 4 ? processingHealth(value.processing_health) : null;
  if (value.schema_version === 4 && !health) return null;
  const frame = value.latest;
  if (frame === null) return value.frames === 0 ? { ...value, ...(health && { processing_health: health }) } : null;
  const frameKeys = ["source", "frame_start_ns", "rms", "peak", "clipped_sample_ratio", "discontinuity"];
  const expectedFrameKeys = value.schema_version === 1 ? frameKeys
    : value.schema_version === 2 ? [...frameKeys, "analysis"]
      : [...frameKeys, "session_start_interval_ns", "analysis"];
  if (value.frames === 0 || !exact(frame, expectedFrameKeys)
    || frame.source !== value.source || !count(frame.frame_start_ns)
    || ![frame.rms, frame.peak, frame.clipped_sample_ratio].every(level)
    || typeof frame.discontinuity !== "boolean") return null;
  if (value.schema_version === 1) return { ...value, latest: { ...frame } };
  let interval = null;
  if (value.schema_version >= 3) {
    interval = frame.session_start_interval_ns;
    if (!exact(interval, ["earliest_ns", "latest_ns"]) || !count(interval.earliest_ns)
      || !count(interval.latest_ns) || interval.earliest_ns > interval.latest_ns) return null;
  }
  const analysis = frame.analysis;
  if (analysis === null) return { ...value, ...(health && { processing_health: health }), latest: { ...frame,
    ...(interval && { session_start_interval_ns: { ...interval } }) } };
  if (!exact(analysis, ["centroid_hz", "high_band_power_ratio", "positive_power_flux", "compared_lags", "best_lag_ns", "best_similarity"])
    || ![analysis.centroid_hz].every(item => item === null || Number.isFinite(item) && item >= 0 && item <= 48000)
    || ![analysis.high_band_power_ratio, analysis.positive_power_flux, analysis.best_similarity]
      .every(item => item === null || level(item))
    || !count(analysis.compared_lags) || analysis.compared_lags > 46
    || !(analysis.best_lag_ns === null || count(analysis.best_lag_ns) && analysis.best_lag_ns >= 100000000
      && analysis.best_lag_ns <= 1000000000 && analysis.best_lag_ns % 20000000 === 0)
    || (analysis.best_lag_ns === null) !== (analysis.best_similarity === null)
    || (analysis.compared_lags === 0 && analysis.best_lag_ns !== null)) return null;
  return { ...value, ...(health && { processing_health: health }), latest: { ...frame, ...(interval && { session_start_interval_ns: { ...interval } }),
    analysis: { ...analysis } } };
}

export function readAcousticOwner(value) {
  if (!exact(value, ["schema_version", "status", "call_bound", "result"]) || ![1, 2].includes(value.schema_version)
    || !["not_configured", "prepared", "running", "finished"].includes(value.status)
    || typeof value.call_bound !== "boolean" || value.status === "not_configured" && value.call_bound) return null;
  if (value.status !== "finished") return value.result === null ? { ...value } : null;
  const resultKeys = ["outcome", "cleanup_confirmed"];
  if (!exact(value.result, value.schema_version === 2 ? [...resultKeys, "reason"] : resultKeys)
    || !["completed", "faulted"].includes(value.result.outcome)
    || typeof value.result.cleanup_confirmed !== "boolean"
    || value.result.outcome === "completed" && !value.result.cleanup_confirmed
    || value.schema_version === 2 && (value.result.outcome === "completed" ? value.result.reason !== null
      : !["discovery_failed", "signal_setup_failed", "source_rejected", "recorder_start_failed", "input_stalled",
        "stream_ended", "stream_read_failed", "processing_failed", "delivery_failed", "recorder_exited",
        "cleanup_failed", "internal_failure"].includes(value.result.reason))) return null;
  return { ...value, result: { ...value.result } };
}

export function acousticFeature(value, owner = undefined) {
  if (owner !== undefined) {
    const lifecycle = readAcousticOwner(owner);
    if (!lifecycle) return { state: "unsupported", detail: "Acoustic owner status unavailable" };
    if (lifecycle.status === "not_configured") return value === null
      ? { state: "ready", detail: "Not configured; no output capture requested" }
      : { state: "unsupported", detail: "Contradictory acoustic owner status" };
    if (lifecycle.status === "prepared") return value === null
      ? { state: "ready", detail: "Prepared; awaiting client confirmation" }
      : { state: "unsupported", detail: "Contradictory acoustic owner status" };
    if (lifecycle.status === "running" && value === null) {
      return { state: "waiting", detail: "Output capture owner running; awaiting status" };
    }
    if (lifecycle.status === "finished") {
      if (lifecycle.result.outcome === "completed") {
        const summary = value === null ? null : readAcousticSummary(value);
        if (value !== null && !summary) return { state: "unsupported", detail: "Acoustic status unavailable" };
        if (summary?.processing_health?.overflow_events > 0 || summary?.processing_health?.discarded.frames > 0) {
          return { state: "error", detail: "Capture ended with discarded processing data; cleanup confirmed" };
        }
        return { state: "ready", detail: "Capture ended; cleanup confirmed" };
      }
      return { state: "error", detail: lifecycle.result.cleanup_confirmed
        ? "Output capture failed; cleanup confirmed; no automatic retry"
        : "Output capture failed; cleanup unconfirmed; no automatic retry" };
    }
  }
  if (value === null) return { state: "ready", detail: "Not configured; no output capture requested" };
  const summary = readAcousticSummary(value);
  if (!summary) return { state: "unsupported", detail: "Acoustic status unavailable" };
  if (summary.status === "faulted") return { state: "error", detail: "Output capture failed; no automatic retry" };
  if (summary.status === "starting") return { state: "waiting", detail: "Output capture requested; awaiting frames" };
  if (summary.processing_health?.overflow_events > 0 || summary.processing_health?.discarded.frames > 0) {
    return { state: "error", detail: "Output processing discarded data; capture liveness unverified" };
  }
  return { state: "ready", detail: summary.status === "stopped" ? "Capture ended; last status only"
    : "Output frames received; liveness unverified" };
}