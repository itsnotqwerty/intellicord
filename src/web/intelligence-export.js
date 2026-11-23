import { readAcousticOwner, readAcousticSummary } from "./acoustic-status.js";
import { readSessionEvidence } from "./session-evidence.js";
import { readSessionSensors } from "./session-sensors.js";
import { readSessionTiming } from "./session-timing.js";

const record = value => value !== null && typeof value === "object" && !Array.isArray(value);
const scalar = value => typeof value === "string" && value.length > 0 && value.length <= 4000 ? value
  : Number.isSafeInteger(value) && value >= 0 ? String(value) : "Unavailable";
const escape = value => JSON.stringify(scalar(value)).slice(1, -1)
  .replace(/[\\`*_{}\[\]<>()#+.!|&"'~=-]/g, character => `&#${character.charCodeAt(0)};`);
const utc = value => Number.isSafeInteger(value) && value >= 0 && Number.isFinite(new Date(value).getTime())
  ? new Date(value).toISOString() : "Unavailable";
const words = value => typeof value === "string" ? value.replaceAll("_", " ") : value;

export function formatIntelligenceMarkdown(value, exportedAt = Date.now()) {
  if (!record(value) || value.schema_version !== 1 || !Array.isArray(value.observations)
    || !Array.isArray(value.incidents) || value.observations.length > 512 || value.incidents.length > 50) {
    throw new Error("Evidence export unavailable: unsupported or incomplete review data.");
  }
  const lines = ["# Intellicord Safety & Intelligence", "", `Exported (UTC): ${utc(exportedAt)}`, "",
    "Retained evidence only. Findings are uncalibrated hypotheses, not proof of abuse, intent, coordination, or network cause.", "",
    "Sampling gaps can hide intermediate activity. Baselines are not arrivals. No automatic moderation is performed.", "",
    "This file contains exact participant and channel IDs. Share only with intended recipients.", ""];
  const fields = entries => {
    lines.push("| Field | Value |", "| --- | --- |");
    for (const [label, field] of entries) lines.push(`| ${label} | ${escape(field)} |`);
    lines.push("");
  };
  const observation = item => {
    if (!record(item) || item.schema_version !== 1) throw new Error("Evidence export unavailable: unsupported observation.");
    fields([["Schema version", item.schema_version], ["Observation ID", item.observation_id], ["Session ID", item.session_id],
      ["Kind", item.kind?.type], ["Gap reason", item.kind?.reason], ["Known missing transitions", item.kind?.missing],
      ["User ID", item.subject?.user_id], ["Channel ID", item.subject?.channel_id], ["Call ID", item.subject?.call_id],
      ["Source", item.source], ["Wall time (UTC)", utc(item.wall_time_unix_ms)], ["Wall time Unix ms", item.wall_time_unix_ms],
      ["Monotonic ns", item.monotonic_ns], ["History sequence", item.history_sequence],
      ["Evidence quality", item.evidence_quality], ["Clock uncertainty", item.clock_uncertainty]]);
  };
  fields([["Schema version", value.schema_version], ["Session ID", value.session_id], ["Review resets", value.reset_count],
    ["Retained observations", value.observations.length], ["Retained incidents", value.incidents.length],
    ["Evicted observations", value.dropped_observations], ["Evicted incidents", value.dropped_incidents],
    ["Known missing voice transitions", value.missing_transitions], ["Gap count", value.gap_count]]);
  lines.push("## Session Ingestion Health", "", "Lifetime counters, not reset with evidence. Diagnostic state is the last report, not a liveness guarantee.", "");
  const ingestion = record(value.ingestion) ? value.ingestion : {};
  fields([["Queue capacity", ingestion.queue_capacity], ["Dropped queue messages", ingestion.dropped_full],
    ["Accepted inputs", ingestion.accepted], ["Rejected invalid inputs", ingestion.rejected_invalid],
    ["Rejected closed inputs", ingestion.rejected_closed], ["Rejected stale sessions", ingestion.rejected_session],
    ["Rejected replayed inputs", ingestion.rejected_replay], ["Rejected clock inputs", ingestion.rejected_clock],
    ["Rejected state inputs", ingestion.rejected_state], ["Diagnostics", ingestion.diagnostics === null ? "Not connected" : ingestion.diagnostics]]);
  const integrationKeys = ["session_sensors", "session_timing", "session_evidence", "acoustic_owner", "acoustic"];
  if (integrationKeys.some(key => Object.hasOwn(value, key))) {
    const sensors = readSessionSensors(value.session_sensors);
    const timing = readSessionTiming(value.session_timing);
    const evidence = readSessionEvidence(value.session_evidence);
    const owner = readAcousticOwner(value.acoustic_owner);
    const acoustic = readAcousticSummary(value.acoustic);
    const health = acoustic?.processing_health;
    lines.push("## Session Integration Boundaries", "",
      "Backend publication snapshot. Process freshness may expire after publication. Sensor liveness, participant attribution, and calibrated cross-sensor correlation are not established.", "");
    fields([["Combined owner status", words(sensors?.status)], ["Configured owners", sensors?.configured],
      ["Finished owners", sensors?.finished], ["Combined cleanup", words(sensors?.cleanup)],
      ["Evidence status", words(evidence?.status)], ["Process evidence", words(evidence?.process)],
      ["Acoustic evidence", words(evidence?.acoustic)],
      ["Cross-sensor correlation", words(evidence?.cross_sensor_correlation)],
      ["Acoustic frame clock", words(timing?.acoustic_frame_clock)], ["Process timing", words(timing?.process_timing)],
      ["Acoustic owner status", words(owner?.status)], ["Acoustic owner outcome", words(owner?.result?.outcome)],
      ["Acoustic terminal reason", owner?.result?.reason ? words(owner.result.reason)
        : owner?.result?.outcome === "completed" ? "Normal or owner-requested completion" : undefined],
      ["Acoustic cleanup", owner?.result ? owner.result.cleanup_confirmed ? "Confirmed" : "Unconfirmed" : undefined],
      ["Acoustic source", words(acoustic?.source)], ["Acoustic status", words(acoustic?.status)],
      ["Accepted acoustic frames", acoustic?.frames], ["Acoustic discontinuities", acoustic?.discontinuities],
      ["Missing acoustic messages", acoustic?.missing_messages], ["Processor status", words(health?.status?.state)],
      ["Processed acoustic frames", health?.processed?.frames], ["Delivered acoustic frames", health?.delivered?.frames],
      ["Pending acoustic frames", health?.pending?.frames], ["Discarded acoustic frames", health?.discarded?.frames],
      ["Processing overflow events", health?.overflow_events]]);
  }
  lines.push("## Retained Incidents", "");
  for (const item of value.incidents) {
    if (!record(item) || !Array.isArray(item.evidence) || item.evidence.length > 20) {
      throw new Error("Evidence export unavailable: incomplete or oversized incident evidence.");
    }
    lines.push("### Incident", "");
    fields([["Incident ID", item.incident_id], ["Detector version", item.detector_version], ["Rule", item.rule],
      ["Explanation", item.explanation], ["Uncertainty", item.uncertainty], ["Confidence", item.confidence],
      ["Recommended action (informational)", item.recommended_action], ["Call ID", item.call_id],
      ["Channel ID", item.channel_id], ["Wall time (UTC)", utc(item.wall_time_unix_ms)], ["Evidence observations", item.evidence.length]]);
    for (const entry of item.evidence) observation(entry);
  }
  lines.push("## Retained Observations", "", "Backend retention order. Wall-clock order may be uncertain.", "");
  for (const item of value.observations) observation(item);
  return `${lines.join("\n")}\n`;
}