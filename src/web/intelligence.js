import { createElement, X, Download, Trash2, RefreshCw, Play, Square } from "lucide";
import { userToolTheme } from "./user-tool-theme.js";
import { formatIntelligenceMarkdown } from "./intelligence-export.js";
import { createProgressHandoff, readOwnerOutcome } from "./process-progress.js";
import { readAcousticOwner, readAcousticSummary } from "./acoustic-status.js";
import { readSessionEvidence } from "./session-evidence.js";
import { readSessionSensors } from "./session-sensors.js";
import { readSessionTiming } from "./session-timing.js";

const unavailable = "Unavailable";
const record = value => value !== null && typeof value === "object" && !Array.isArray(value);
const text = (value, limit = 4000) => typeof value === "string" && value.length > 0 && value.length <= limit ? value : unavailable;
const id = value => text(value, 512);
const number = value => typeof value === "number" && Number.isFinite(value) && Number.isInteger(value) && value >= 0 ? String(value) : unavailable;
const count = value => Number.isSafeInteger(value) && value >= 0 ? String(value) : unavailable;
const protocolCases = ["normal_completion", "fault_completion", "truncated_record", "sequence_replay",
  "source_mismatch", "confirmation_forgery", "health_regression", "exit_mismatch"];
const timedCases = ["interval_bounds", "foreign_record", "receipt_regression", "source_regression", "anchor_expiry",
  "mapping_expiry", "future_event", "stale_event", "rejection_isolation"];
const transportCases = ["fragmented_delivery", "truncated_record", "foreign_record", "stream_limit",
  "receipt_regression", "failed_exit", "empty_stream", "delayed_fragment"];
const timedCounters = [["accepted", "Timed accepted records"], ["rejected_record", "Timed invalid records"],
  ["rejected_receipt_clock", "Timed receipt regressions"], ["rejected_anchor_expired", "Timed expired anchors"],
  ["rejected_source_clock", "Timed source regressions"], ["rejected_mapping", "Timed mapping rejections"],
  ["rejected_future", "Timed future events"], ["rejected_stale", "Timed stale events"]];
const choice = (value, choices) => choices.includes(value) ? value : unavailable;
const time = value => {
  if (typeof value !== "number" || !Number.isFinite(value)) return unavailable;
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? `${date.toISOString()} (Unix ms: ${value})` : unavailable;
};
const latest = (values, limit) => values.map((value, index) => ({ value, index }))
  .sort((left, right) => {
    const stamp = item => typeof item.value?.wall_time_unix_ms === "number" && Number.isFinite(item.value.wall_time_unix_ms) ? item.value.wall_time_unix_ms : -Infinity;
    return (stamp(right) - stamp(left) || right.index - left.index);
  }).slice(0, limit).map(item => item.value);

const styles = userToolTheme + `
.ic-intelligence-root{position:fixed;inset:0;z-index:1004;display:flex;align-items:center;justify-content:center;box-sizing:border-box;padding:16px;background:rgba(0,0,0,.65)}
.ic-intelligence-dialog{box-sizing:border-box;width:640px;max-width:100%;max-height:calc(100vh - 32px);max-height:calc(100dvh - 32px);overflow:auto;border-radius:8px;padding:20px;background:var(--ic-tool-bg);color:var(--ic-tool-text);font-family:var(--font-primary,Whitney,sans-serif);font-size:14px;letter-spacing:0;overflow-wrap:anywhere;user-select:text}
.ic-intelligence-dialog header{display:flex;align-items:center;gap:8px}.ic-intelligence-dialog h2{flex:1;min-width:0;font-size:20px;margin:0}.ic-intelligence-dialog h3{font-size:16px;margin:20px 0 8px}.ic-intelligence-dialog p{color:var(--ic-tool-muted);margin:8px 0}.ic-intelligence-dialog button{display:grid;place-items:center;flex-shrink:0;width:32px;height:32px;border:0;border-radius:4px;background:transparent;color:inherit;cursor:pointer}.ic-intelligence-dialog button:hover{background:var(--ic-tool-hover)}.ic-intelligence-dialog :is(button,summary):focus-visible{outline:2px solid var(--text-link,#00a8fc);outline-offset:2px}
.ic-intelligence-dialog dl{margin:8px 0}.ic-intelligence-row{display:grid;grid-template-columns:minmax(0,160px) minmax(0,1fr);gap:8px;padding:5px 0}.ic-intelligence-row dt{color:var(--ic-tool-muted)}.ic-intelligence-row dd{margin:0;min-width:0;white-space:pre-wrap;overflow-wrap:anywhere}.ic-intelligence-dialog details{border-top:1px solid var(--ic-tool-border);padding:4px 0}.ic-intelligence-dialog summary{cursor:pointer;padding:8px 0;overflow-wrap:anywhere}.ic-intelligence-evidence{border-left:2px solid var(--ic-tool-border);padding-left:12px;margin:12px 0}@media(max-width:480px){.ic-intelligence-row{grid-template-columns:minmax(0,1fr);gap:2px}.ic-intelligence-dialog{padding:16px}}
@media(max-width:480px){.ic-intelligence-dialog header{flex-wrap:wrap}.ic-intelligence-dialog h2{flex-basis:100%}}
.ic-intelligence-dialog button[hidden]{display:none}.ic-intelligence-dialog button:disabled{opacity:.5;cursor:default}
`;

export function mountIntelligenceReview({ reset, checkDiagnostics, startAcoustic, stopAcoustic } = {}) {
  let value;
  let root = null;
  let content = null;
  let restoreFocus = null;
  let disposed = false;
  let exportButton;
  let resetButton;
  let confirmation;
  let status;
  let armedSession = null;
  let pending = null;
  let resetTimer;
  let diagnosticsButton;
  let diagnosticsTimer;
  let diagnosticsSequence = 0;
  let diagnosticsPending = null;
  let readiness = null;
  let processReplay = null;
  let processObject = null;
  let processTarget = null;
  let processLifecycle = null;
  let processValidation = null;
  let processSupervision = null;
  let processPreflight = null;
  let processClock = null;
  let processHandshake = null;
  let processTimed = null;
  let healthSnapshot = null;
  const progressHandoff = createProgressHandoff();
  const sessionProgressHandoff = createProgressHandoff();
  let ownerOutcome = null;
  let sessionOwner = null;
  let sessionEvidence = null;
  let sessionSensors = null;
  let sessionTiming = null;
  let acoustic = null;
  let acousticOwner = null;
  let acousticStart = null;
  let acousticStartTimer;
  let acousticStartButton;
  let acousticConfirmation;
  let acousticStop = null;
  let acousticStopTimer;
  let acousticStopButton;
  let progressTimer;
  let sessionProgressTimer;
  let diagnosticsIssuedAt = 0;
  const clearProgress = () => { clearTimeout(progressTimer); progressHandoff.clear(); ownerOutcome = null; };
  let diagnosticsMessage = "Not checked";
  const validSession = () => record(value) && value.schema_version === 1 && id(value.session_id) !== unavailable;
  const validHealthSnapshot = snapshot => {
    if (!record(snapshot) || Object.keys(snapshot).length !== 5 || snapshot.schema_version !== 1
      || snapshot.source !== "operator_supplied_snapshot" || snapshot.sensor_running !== false) return false;
    if (["not_configured", "unavailable", "rejected", "inspection_timed_out", "inspection_failed", "cleanup_unconfirmed"].includes(snapshot.status)) return snapshot.report === null;
    const report = snapshot.report, run = report?.run, status = run?.status, health = run?.terminal_health;
    return snapshot.status === "supplied" && record(report) && Object.keys(report).length === 4
      && report.schema_version === 1 && report.source === "supervised_kernel_capture" && report.sensor_running === false
      && record(run) && Object.keys(run).length === 6 && run.schema_version === 1 && run.outcome === "completed"
      && run.collector_started === true && run.collector_reaped === true
      && record(status) && Object.keys(status).length === 7 && status.schema_version === 1
      && status.source === "kernel_tracepoint" && status.records === 6 && status.terminal_phase === "stopped"
      && ["duration_limit", "interrupt", "terminate", "target_exited"].includes(status.reason)
      && status.attachment_confirmed === true && status.detachment_confirmed === true
      && record(health) && Object.keys(health).length === 5 && health.schema_version === 1
      && ["accepted", "rejected_record", "rejected_clock", "ring_dropped"]
        .every(key => Number.isSafeInteger(health[key]) && health[key] >= 0);
  };
  const selectionCases = ["disabled_target", "foreign_tgid", "matching_leader", "matching_thread", "zero_process", "zero_thread", "full_width_identity"];
  const producerCases = ["filter_before_reservation", "submitted_identity", "capacity_rejection", "full_ring_filter", "recovery_after_drain", "drop_agreement"];
  const accountingCases = ["counter_saturation", "per_cpu_sum", "aggregate_saturation", "snapshot_high_water", "saturated_filter", "saturated_recovery"];
  const reservationCases = ["filtered_reservation", "reserve_before_clock", "single_submission", "native_record_delivery", "full_ring_preservation", "fresh_recovery"];
  const recordCases = ["native_layout", "full_width_fields", "unaligned_decode", "invalid_record_rejection"];
  const attachmentCases = ["setup_failure_stops", "load_failure_stops", "disabled_target_required", "attach_failure_stops", "detach_failure_visible", "successful_cleanup"];
  const deliveryCases = ["matching_exec", "foreign_process", "worker_thread", "before_release", "after_receipt", "malformed_record", "backwards_bracket"];
  const progressCases = ["fragmented_delivery", "foreign_session", "replayed_sequence", "counter_regression", "before_observation", "freshness_bounds", "truncated_record", "exit_mismatch"];
  const validProgressReplay = report => record(report) && Object.keys(report).length === 5
    && report.schema_version === 1 && report.source === "generated_replay"
    && report.sensor_running === false && report.load_test_performed === false
    && Array.isArray(report.checks) && report.checks.length === progressCases.length
    && report.checks.every((item, index) => record(item) && Object.keys(item).length === 2
      && item.name === progressCases[index] && typeof item.passed === "boolean");
  const validDeliveryReplay = report => record(report) && Object.keys(report).length === 5
    && report.schema_version === 1 && report.source === "generated_replay"
    && report.sensor_running === false && report.load_test_performed === false
    && Array.isArray(report.checks) && report.checks.length === deliveryCases.length
    && report.checks.every((item, index) => record(item) && Object.keys(item).length === 2
      && item.name === deliveryCases[index] && typeof item.passed === "boolean");
  const validAttachmentReplay = report => record(report) && report.schema_version === 1
    && Object.keys(report).length === 6 && report.source === "generated_replay"
    && report.sensor_running === false && report.load_test_performed === false && report.attachment_attempted === false
    && Array.isArray(report.checks) && report.checks.length === attachmentCases.length
    && report.checks.every((item, index) => record(item) && Object.keys(item).length === 2
      && item.name === attachmentCases[index] && typeof item.passed === "boolean");
  const validRecordReplay = report => record(report) && report.schema_version === 1
    && report.source === "generated_replay" && report.sensor_running === false && report.load_test_performed === false
    && Array.isArray(report.checks) && report.checks.length === recordCases.length
    && report.checks.every((item, index) => record(item) && item.name === recordCases[index] && typeof item.passed === "boolean");
  const validProducerReplay = report => record(report) && report.schema_version === 1
    && report.source === "generated_replay" && report.sensor_running === false && report.load_test_performed === false
    && report.capacity === 2 && ["attempts", "submitted", "dropped"].every(key => Number.isSafeInteger(report[key]) && report[key] >= 0 && report[key] <= 9)
    && Array.isArray(report.cases) && report.cases.length === producerCases.length
    && report.cases.every((item, index) => record(item) && item.name === producerCases[index] && typeof item.passed === "boolean")
    && (report.reservation === undefined || (Array.isArray(report.reservation) && report.reservation.length === reservationCases.length
      && report.reservation.every((item, index) => record(item) && item.name === reservationCases[index] && typeof item.passed === "boolean")))
    && (report.accounting === undefined || (Array.isArray(report.accounting) && report.accounting.length === accountingCases.length
      && report.accounting.every((item, index) => record(item) && item.name === accountingCases[index] && typeof item.passed === "boolean")));
  const validProcessReplay = replay => record(replay) && replay.schema_version === 1
    && replay.source === "generated_replay" && replay.sensor_running === false
    && record(replay.health) && replay.health.schema_version === 1
    && ["accepted", "rejected_record", "rejected_clock", "ring_dropped"]
      .every(key => Number.isSafeInteger(replay.health[key]) && replay.health[key] >= 0)
    && (replay.selection === undefined || (Array.isArray(replay.selection) && replay.selection.length === selectionCases.length
      && replay.selection.every((item, index) => record(item) && item.name === selectionCases[index] && typeof item.passed === "boolean")))
    && (replay.producer === undefined || validProducerReplay(replay.producer))
    && (replay.attachment === undefined || validAttachmentReplay(replay.attachment))
    && (replay.delivery === undefined || validDeliveryReplay(replay.delivery))
    && (replay.progress === undefined || validProgressReplay(replay.progress))
    && (replay.record_layout === undefined || validRecordReplay(replay.record_layout));
  const validReadiness = report => record(report) && [1, 2, 3].includes(report.schema_version)
    && ["linux", "unsupported"].includes(report.platform)
    && [report.kernel_btf, report.cgroup_v2].every(state => ["present", "missing", "unreadable", "unsupported"].includes(state))
    && ["enabled", "disabled_locked", "disabled_configurable", "unknown", "unsupported"].includes(report.unprivileged_bpf)
    && report.sensor_running === false && report.load_test_performed === false
    && (report.schema_version === 1 || (
      (report.perf_event_paranoid === null || (Number.isInteger(report.perf_event_paranoid) && report.perf_event_paranoid >= -2147483648 && report.perf_event_paranoid <= 2147483647))
      && (report.effective_capabilities === null || (record(report.effective_capabilities)
        && ["bpf", "perfmon", "sys_admin"].every(key => typeof report.effective_capabilities[key] === "boolean")))
      && ["present", "missing", "unreadable", "unsupported"].includes(report.sched_process_exec)
      && ["none", "integrity", "confidentiality", "unknown", "unsupported"].includes(report.lockdown)
    )) && (report.schema_version !== 3 || (
      (report.no_new_privileges === null || typeof report.no_new_privileges === "boolean")
      && ["disabled", "strict", "filter", "unknown", "unsupported"].includes(report.seccomp)
    ));
  const codeCalls = ["map_lookup_calls", "clock_calls", "identity_calls", "ring_reserve_calls", "ring_submit_calls", "other_helper_calls", "local_calls", "indirect_calls"];
  const validCodeInventory = code => record(code) && code.schema_version === 1
    && Object.keys(code).length === codeCalls.length + 3
    && Number.isInteger(code.functions) && code.functions > 0 && code.functions <= 4096
    && Number.isInteger(code.instruction_slots) && code.instruction_slots >= code.functions && code.instruction_slots <= 524288
    && codeCalls.every(key => Number.isInteger(code[key]) && code[key] >= 0 && code[key] <= code.instruction_slots)
    && codeCalls.reduce((sum, key) => sum + code[key], 0) <= code.instruction_slots;
  const validProcessObject = report => record(report) && report.schema_version === 1
    && report.sensor_running === false && report.load_test_performed === false
    && typeof report.inspector_available === "boolean"
    && (report.load_probe_available === undefined || (typeof report.load_probe_available === "boolean"
      && report.load_probe_available === report.inspector_available))
    && (report.code === undefined || (report.status === "layout_matched" && validCodeInventory(report.code)))
    && (report.inspector_available
      ? ["not_configured", "layout_matched", "rejected", "unreadable"].includes(report.status)
      : ["feature_disabled", "unsupported"].includes(report.status));
  const validProcessPreflight = report => {
    if (!record(report) || report.schema_version !== 1 || report.source !== "offline_object_preflight"
      || report.sensor_running !== false || report.load_test_performed !== false
      || typeof report.helper_started !== "boolean" || typeof report.helper_reaped !== "boolean") return false;
    if (report.outcome === "completed") return report.helper_started && report.helper_reaped && validProcessObject(report.object);
    if (report.object !== null) return false;
    if (report.outcome === "spawn_failed") return !report.helper_started && !report.helper_reaped;
    if (report.outcome === "cleanup_failed") return report.helper_started && !report.helper_reaped;
    return ["deadline", "invalid_stream", "read_failed", "wait_failed", "collector_fault"].includes(report.outcome)
      && report.helper_started && report.helper_reaped;
  };
  function checkPrerequisites() {
    if (!validSession() || diagnosticsPending || typeof checkDiagnostics !== "function" || diagnosticsSequence >= Number.MAX_SAFE_INTEGER) return;
    const request = { session_id: value.session_id, request_id: ++diagnosticsSequence };
    clearProgress();
    diagnosticsIssuedAt = performance.now();
    diagnosticsPending = request;
    diagnosticsMessage = "Checking";
    diagnosticsTimer = setTimeout(() => {
      diagnosticsPending = null;
      diagnosticsMessage = "Check not acknowledged";
      render(); refreshControls();
    }, 5000);
    render(); refreshControls();
    try { checkDiagnostics(request); }
    catch {
      clearTimeout(diagnosticsTimer);
      diagnosticsPending = null;
      diagnosticsMessage = "Check request failed";
      render(); refreshControls();
    }
  }
  const validProcessTarget = report => record(report) && report.schema_version === 1
    && report.source === "self_pidfd_probe"
    && ["available", "unavailable", "feature_disabled", "unsupported"].includes(report.status)
    && report.target_selected === false && report.sensor_running === false && report.load_test_performed === false;
  const validProcessClock = report => record(report) && report.schema_version === 1
    && report.source === "local_kernel_clock_probe" && report.clock_domain === "linux_monotonic"
    && report.sensor_running === false && report.load_test_performed === false
    && report.horizon_ns === 1000000000 && report.assumed_rate_error_ppm === 100
    && (report.status === "available"
      ? Number.isSafeInteger(report.bracket_width_ns) && report.bracket_width_ns >= 0
      : ["unavailable", "feature_disabled", "unsupported"].includes(report.status) && report.bracket_width_ns === null);
  const validProcessHandshake = report => {
    if (!record(report) || Object.keys(report).length !== 8 || report.schema_version !== 1
      || report.source !== "local_clock_handshake" || report.clock_domain !== "linux_monotonic"
      || report.max_bracket_ns !== 25000000 || report.sensor_running !== false || report.load_test_performed !== false) return false;
    if (["feature_disabled", "unsupported"].includes(report.status)) return report.result === null;
    const run = report.result;
    if (!record(run) || Object.keys(run).length !== 5 || run.schema_version !== 1
      || typeof run.helper_started !== "boolean" || typeof run.helper_reaped !== "boolean") return false;
    if (report.status === "available") return run.outcome === "completed" && run.helper_started && run.helper_reaped
      && Number.isSafeInteger(run.bracket_width_ns) && run.bracket_width_ns >= 0 && run.bracket_width_ns <= report.max_bracket_ns;
    if (report.status !== "unavailable" || run.bracket_width_ns !== null) return false;
    if (run.outcome === "spawn_failed") return !run.helper_started && !run.helper_reaped;
    if (run.outcome === "cleanup_failed") return run.helper_started && !run.helper_reaped;
    if (["cancelled", "invalid_stream"].includes(run.outcome)) return run.helper_started === run.helper_reaped;
    return ["deadline", "read_failed", "wait_failed", "collector_fault"].includes(run.outcome)
      && run.helper_started && run.helper_reaped;
  };
  const validProcessTimed = report => record(report) && report.schema_version === 1
    && report.source === "generated_replay" && report.clock_domain === "linux_monotonic"
    && report.sensor_running === false && report.load_test_performed === false && record(report.health)
    && timedCounters.every(([key]) => Number.isSafeInteger(report.health[key]) && report.health[key] >= 0)
    && Array.isArray(report.cases) && report.cases.length === timedCases.length
    && report.cases.every((item, index) => record(item) && item.name === timedCases[index] && typeof item.passed === "boolean")
    && (report.transport === undefined || (Array.isArray(report.transport) && report.transport.length === transportCases.length
      && report.transport.every((item, index) => record(item) && item.name === transportCases[index] && typeof item.passed === "boolean")));
  const validProcessLifecycle = records => Array.isArray(records) && records.length === 6
    && ["validating", "loading", "attaching", "observing", "stopping", "stopped"].every((phase, index) => {
      const item = records[index];
      return record(item) && item.schema_version === 1 && item.record_type === "process_status"
        && item.source === "generated_replay" && item.sequence === index + 1 && item.phase === phase
        && item.attachment_confirmed === false && item.detachment_confirmed === false
        && item.reason === (index === 5 ? "duration_limit" : null)
        && (index === 5 ? validProcessReplay({ schema_version: 1, source: "generated_replay",
          sensor_running: false, health: item.health }) : item.health === null);
    });
  const validProcessValidation = report => record(report) && report.schema_version === 1
    && report.source === "generated_replay" && report.sensor_running === false && report.load_test_performed === false
    && Array.isArray(report.cases) && report.cases.length === protocolCases.length
    && report.cases.every((item, index) => record(item) && item.name === protocolCases[index] && typeof item.passed === "boolean");
  const validProcessSupervision = report => {
    if (!record(report) || report.schema_version !== 1 || report.source !== "generated_replay"
      || report.sensor_running !== false || report.load_test_performed !== false) return false;
    if (report.collection !== undefined && !validProcessCollection(report.collection)) return false;
    if (report.negotiated_collection !== undefined && !validProcessCollection(report.negotiated_collection)) return false;
    if (report.response_deadline_ms !== undefined && report.response_deadline_ms !== 2000) return false;
    if (report.quiet_collection_check !== undefined && typeof report.quiet_collection_check !== "boolean") return false;
    const run = report.supervision;
    if (!record(run) || run.schema_version !== 1 || typeof run.collector_started !== "boolean"
      || typeof run.collector_reaped !== "boolean" || (!run.collector_started && run.collector_reaped)) return false;
    if (["completed", "collector_fault"].includes(run.outcome)) {
      const status = run.status;
      return run.collector_started && run.collector_reaped && record(status) && status.schema_version === 1
        && status.source === "generated_replay" && status.attachment_confirmed === false && status.detachment_confirmed === false
        && (run.outcome === "completed"
          ? status.records === 6 && status.terminal_phase === "stopped" && status.reason === "duration_limit"
          : Number.isSafeInteger(status.records) && status.records >= 1 && status.records <= 7 && status.terminal_phase === "faulted"
            && ["invalid_object", "target_unavailable", "signal_setup_failed", "load_failed", "attach_failed", "input_rejected",
              "read_failed", "delivery_failed", "detach_failed", "cancelled", "invalid_request", "internal_failure"].includes(status.reason));
    }
    if (run.status !== null) return false;
    if (["invalid_duration", "spawn_failed"].includes(run.outcome)) return !run.collector_started && !run.collector_reaped;
    if (run.outcome === "cancelled") return run.collector_started === run.collector_reaped;
    if (run.outcome === "cleanup_failed") return run.collector_started && !run.collector_reaped;
    return ["deadline", "invalid_stream", "read_failed", "wait_failed"].includes(run.outcome)
      && run.collector_started && run.collector_reaped;
  };
  function validProcessCollection(run) {
    if (!record(run) || run.schema_version !== 1 || typeof run.collector_started !== "boolean"
      || typeof run.collector_reaped !== "boolean" || (!run.collector_started && run.collector_reaped)) return false;
    if (run.outcome !== "completed") {
      if (run.collection !== null) return false;
      if (["invalid_duration", "spawn_failed"].includes(run.outcome)) return !run.collector_started && !run.collector_reaped;
      if (run.outcome === "cancelled") return run.collector_started === run.collector_reaped;
      if (run.outcome === "cleanup_failed") return run.collector_started && !run.collector_reaped;
      return ["deadline", "invalid_stream", "read_failed", "wait_failed"].includes(run.outcome)
        && run.collector_started && run.collector_reaped;
    }
    const summary = run.collection, status = summary?.status, events = summary?.events, health = summary?.collector_health;
    return run.collector_started && run.collector_reaped && record(summary) && summary.schema_version === 1
      && record(status) && status.schema_version === 1 && status.source === "generated_replay" && status.records === 6
      && status.terminal_phase === "stopped" && status.reason === "duration_limit"
      && status.attachment_confirmed === false && status.detachment_confirmed === false
      && record(events) && events.schema_version === 1 && Number.isSafeInteger(events.records) && events.records >= 0 && events.records <= 256
      && record(events.health) && events.health.accepted === events.records
      && timedCounters.slice(1).every(([key]) => events.health[key] === 0)
      && record(health) && health.schema_version === 1 && health.accepted === events.records
      && ["accepted", "rejected_record", "rejected_clock", "ring_dropped"].every(key => Number.isSafeInteger(health[key]) && health[key] >= 0);
  }
  const downloads = new Map();
  const resetSupported = () => typeof reset === "function" && record(value) && value.schema_version === 1
    && id(value.session_id) !== unavailable && Number.isSafeInteger(value.reset_count) && value.reset_count >= 0;
  function cancelReset() {
    armedSession = null;
    if (confirmation) confirmation.hidden = true;
  }
  function refreshControls() {
    if (!root) return;
    exportButton.disabled = !record(value) || value.schema_version !== 1 || !Array.isArray(value.observations) || !Array.isArray(value.incidents);
    resetButton.disabled = !resetSupported() || Boolean(pending);
    diagnosticsButton.disabled = !validSession() || typeof checkDiagnostics !== "function" || Boolean(diagnosticsPending);
    acousticStartButton.hidden = !canStartAcoustic() && acousticStart?.state !== "pending";
    acousticStartButton.disabled = !canStartAcoustic() || acousticStart?.state === "pending";
    acousticStopButton.hidden = !canStopAcoustic() && acousticStop?.state !== "pending";
    acousticStopButton.disabled = !canStopAcoustic() || acousticStop?.state === "pending";
    if (acousticStopButton.hidden && document.activeElement === acousticStopButton) root.querySelector("button").focus();
    confirmation.querySelectorAll("button").forEach(button => { button.disabled = Boolean(pending); });
  }
  function canStartAcoustic() {
    return validSession() && typeof startAcoustic === "function" && value.acoustic_start_available === true
      && acoustic === null;
  }
  function cancelAcousticStart() {
    if (acousticConfirmation) acousticConfirmation.hidden = true;
  }
  function confirmAcousticStart() {
    if (!canStartAcoustic() || acousticStart?.state === "pending") return;
    cancelAcousticStart();
    clearTimeout(acousticStartTimer);
    const request = { session_id: value.session_id, state: "pending", message: "Start requested; awaiting capture status." };
    acousticStart = request;
    acousticStartTimer = setTimeout(() => {
      if (acousticStart !== request) return;
      request.state = "unconfirmed";
      request.message = "Start unconfirmed; no automatic retry.";
      render(); refreshControls();
    }, 5000);
    try { startAcoustic({ session_id: request.session_id }); }
    catch {
      clearTimeout(acousticStartTimer);
      request.state = "failed";
      request.message = "Start request failed; output analysis was not confirmed.";
    }
    render(); refreshControls();
  }
  function canStopAcoustic() {
    return validSession() && typeof stopAcoustic === "function" && value.acoustic_stop_available === true
      && acoustic?.source === "mixed_output" && ["starting", "receiving"].includes(acoustic.status);
  }
  function requestAcousticStop() {
    if (!canStopAcoustic() || acousticStop?.state === "pending") return;
    clearTimeout(acousticStopTimer);
    const request = { session_id: value.session_id, state: "pending", message: "Stop requested; awaiting capture cleanup." };
    acousticStop = request;
    acousticStopTimer = setTimeout(() => {
      if (acousticStop !== request) return;
      request.state = "unconfirmed";
      request.message = "Stop unconfirmed; no automatic retry.";
      render(); refreshControls();
    }, 5000);
    try { stopAcoustic({ session_id: request.session_id }); }
    catch {
      clearTimeout(acousticStopTimer);
      request.state = "failed";
      request.message = "Stop request failed; capture may still be running.";
    }
    render(); refreshControls();
  }
  function releaseDownload(url) {
    clearTimeout(downloads.get(url));
    downloads.delete(url);
    URL.revokeObjectURL(url);
  }
  function download() {
    let url;
    let anchor;
    try {
      const markdown = formatIntelligenceMarkdown(value);
      url = URL.createObjectURL(new Blob([markdown], { type: "text/markdown;charset=utf-8" }));
      anchor = node("a");
      anchor.href = url;
      anchor.download = "intellicord-evidence.md";
      document.body.append(anchor);
      anchor.click();
      downloads.set(url, setTimeout(() => releaseDownload(url), 30000));
      while (downloads.size > 4) releaseDownload(downloads.keys().next().value);
      status.textContent = "Evidence download requested. The file contains participant and channel IDs.";
    } catch {
      if (url) releaseDownload(url);
      status.textContent = "Evidence export failed: data is incomplete, unsupported, or the download is unavailable.";
    } finally { anchor?.remove(); }
  }
  function confirmReset() {
    if (!resetSupported() || pending) return;
    if (!armedSession || armedSession.session_id !== value.session_id || armedSession.reset_count !== value.reset_count) {
      cancelReset();
      status.textContent = "Review changed. Request reset again.";
      resetButton.focus();
      return;
    }
    pending = armedSession;
    refreshControls();
    status.textContent = "Waiting for backend reset acknowledgement.";
    resetTimer = setTimeout(() => {
      pending = null;
      cancelReset();
      if (status) status.textContent = "Reset not acknowledged. Review may be unchanged; no automatic retry was made.";
      refreshControls();
      if (root) resetButton.focus();
    }, 5000);
    try { reset({ ...pending }); }
    catch {
      clearTimeout(resetTimer);
      pending = null;
      cancelReset();
      status.textContent = "Reset request failed. Review was not cleared locally.";
      refreshControls();
      resetButton.focus();
    }
  }
  const entryKey = (prefix, value, index) => `${prefix}:${id(value) === unavailable ? `unavailable:${index}` : value}`;
  const node = (tag, value) => {
    const element = document.createElement(tag);
    if (value !== undefined) element.textContent = value;
    return element;
  };
  function rows(host, fields) {
    const list = node("dl");
    for (const [label, value] of fields) {
      const row = node("div");
      row.className = "ic-intelligence-row";
      row.append(node("dt", label), node("dd", value));
      list.append(row);
    }
    host.append(list);
  }
  function observation(host, item) {
    if (!record(item) || item.schema_version !== 1) {
      host.append(node("p", "Observation unavailable: missing or unsupported schema version."));
      return;
    }
    rows(host, [
      ["Observation ID", id(item.observation_id)], ["Schema version", "1"], ["Session ID", id(item.session_id)],
      ["Kind", choice(item.kind?.type, ["joined", "left", "gap", "baseline"])],
      ["Reason", text(item.kind?.reason)], ["Missing transitions", count(item.kind?.missing)],
      ["User ID", id(item.subject?.user_id)], ["Channel ID", id(item.subject?.channel_id)], ["Call ID", count(item.subject?.call_id)],
      ["Source", choice(item.source, ["store_notification", "reconciliation", "initial_snapshot", "unknown"])],
      ["Wall time (UTC)", time(item.wall_time_unix_ms)], ["Monotonic ns", number(item.monotonic_ns)],
      ["History sequence", count(item.history_sequence)],
      ["Evidence quality", choice(item.evidence_quality, ["snapshot_difference"])], ["Clock uncertainty", text(item.clock_uncertainty)],
    ]);
  }
  function expandable(host, key, title, render) {
    const details = node("details");
    details.dataset.reviewKey = key;
    const summary = node("summary", title);
    details.append(summary);
    render(details);
    host.append(details);
  }
  function render() {
    if (!content) return;
    const expanded = new Set([...content.querySelectorAll("details[open]")].map(item => item.dataset.reviewKey));
    const focused = document.activeElement?.tagName === "SUMMARY" && content.contains(document.activeElement)
      ? document.activeElement.parentElement.dataset.reviewKey : null;
    const scrollTop = root.querySelector("section").scrollTop;
    content.replaceChildren();
    content.append(node("h3", "Linux sensor prerequisites"));
    rows(content, [["Check status", diagnosticsMessage]]);
    if (readiness) {
      const stateLabels = { present: "Present", missing: "Missing", unreadable: "Unreadable", unsupported: "Unsupported" };
      rows(content, [["Kernel BTF metadata", stateLabels[readiness.kernel_btf]], ["cgroup v2 interface", stateLabels[readiness.cgroup_v2]],
        ["Unprivileged BPF policy", ({ enabled: "Enabled by policy", disabled_locked: "Disabled until reboot", disabled_configurable: "Disabled; administrator-controlled", unknown: "Unknown", unsupported: "Unsupported" })[readiness.unprivileged_bpf]],
        ["eBPF sensor", "Not started"], ["Program load and attach", "Not tested"]]);
      const capability = name => readiness.effective_capabilities == null ? unavailable
        : readiness.effective_capabilities[name] ? "Effective" : "Not effective";
      rows(content, [["Perf event policy", readiness.perf_event_paranoid == null ? unavailable : String(readiness.perf_event_paranoid)],
        ["CAP_BPF", capability("bpf")], ["CAP_PERFMON", capability("perfmon")], ["CAP_SYS_ADMIN", capability("sys_admin")],
        ["Process-exec tracepoint", stateLabels[readiness.sched_process_exec] || unavailable],
        ["Kernel lockdown", ({ none: "None", integrity: "Integrity", confidentiality: "Confidentiality", unknown: "Unknown", unsupported: "Unsupported" })[readiness.lockdown] || unavailable]]);
      rows(content, [["Restriction context", "Backend main-thread status; sensor permissions untested"],
        ["No-new-privileges", readiness.schema_version === 3 && readiness.no_new_privileges !== null
          ? readiness.no_new_privileges ? "Set" : "Not set" : unavailable],
        ["Seccomp mode", readiness.schema_version === 3
          ? ({ disabled: "Disabled", strict: "Strict", filter: "Filter installed", unknown: "Unknown", unsupported: "Unsupported" })[readiness.seccomp]
          : unavailable]]);
      content.append(node("p", "Prerequisite snapshot only. Program permissions, kernel features and attach support remain unverified."));
    }
    if (processClock) {
      content.append(node("h3", "Kernel clock diagnostics"));
      rows(content, [["Clock probe", ({ available: "Local clock read passed", unavailable: "Local clock read failed",
        feature_disabled: "Optional observe feature not built", unsupported: "Unsupported platform" })[processClock.status]],
        ["Kernel timestamp domain", "Linux CLOCK_MONOTONIC"],
        ["Clock read bracket", processClock.bracket_width_ns === null ? unavailable : `${processClock.bracket_width_ns} ns`],
        ["Mapping horizon", "1 second of source time"], ["Assumed relative rate bound", "100 ppm; not measured"],
        ["Cross-process synchronization", "Not verified by this local probe"]]);
    }
    if (processHandshake) {
      content.append(node("h3", "Clock handoff diagnostics"));
      const run = processHandshake.result;
      rows(content, [["Clock handshake", ({ available: "Bounded exchange passed", unavailable: "Exchange failed",
        feature_disabled: "Optional observe feature not built", unsupported: "Unsupported platform" })[processHandshake.status]],
        ["Handshake outcome", run ? run.outcome.replaceAll("_", " ") : unavailable],
        ["Handshake bracket", run?.bracket_width_ns == null ? unavailable : `${run.bracket_width_ns} ns`],
        ["Clock helper cleanup", !run?.helper_started ? "Not started" : run.helper_reaped ? "Reaped" : "Not confirmed"],
        ["Clock namespace identity", "Not verified"], ["Live collector clock handoff", "Not connected"]]);
    }
    if (processTimed) {
      content.append(node("h3", "Timed process replay"));
      const failures = processTimed.cases.filter(item => !item.passed);
      rows(content, [["Timed replay source", "Generated records and clocks; not kernel capture"],
        ["Timed replay checks", `${processTimed.cases.length - failures.length} / ${processTimed.cases.length} passed`],
        ["Failed timed checks", failures.length ? failures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"],
        ...timedCounters.map(([key, label]) => [label, count(processTimed.health[key])])]);
      if (processTimed.transport) {
        const failures = processTimed.transport.filter(item => !item.passed);
        rows(content, [["Event transport checks", `${processTimed.transport.length - failures.length} / ${processTimed.transport.length} passed`],
          ["Failed transport checks", failures.length ? failures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"]]);
      }
    }
    if (processTarget) {
      content.append(node("h3", "Process target safety"));
      rows(content, [["Process lifetime guard", ({ available: "Self-process pidfd probe passed", unavailable: "Self-process pidfd probe failed",
        feature_disabled: "Optional observe feature not built", unsupported: "Unsupported platform" })[processTarget.status]],
        ["Observation target", "None selected"], ["Target identity binding", "Not verified by this probe"]]);
    }
    if (processPreflight) {
      content.append(node("h3", "Object inspection isolation"));
      rows(content, [["Inspection process", "Separate offline helper; no BPF loading"],
        ["Inspection result", ({ completed: "Reply validated", spawn_failed: "Helper launch failed", deadline: "Deadline expired",
          invalid_stream: "Invalid helper reply", read_failed: "Reply read failed", wait_failed: "Process wait failed",
          collector_fault: "Helper exited unsuccessfully", cleanup_failed: "Cleanup unconfirmed" })[processPreflight.outcome]],
        ["Inspection cleanup", processPreflight.helper_reaped ? "Reaped" : "Not confirmed"]]);
    }
    if (processObject) {
      content.append(node("h3", "Process sensor object"));
      rows(content, [["Metadata inspector", processObject.inspector_available ? "Included in this build" : "Not available in this build"],
        ["Object metadata", ({ not_configured: "No object configured", layout_matched: "Expected layout matched; behavior unverified",
          rejected: "Object rejected", unreadable: "Object unavailable or outside file limits", feature_disabled: "Optional observe feature not built",
          unsupported: "Unsupported platform" })[processObject.status]],
        ["Object load and attach", "Not tested"],
        ["Load-only probe build", processObject.load_probe_available === true ? "Included; not requested"
          : processObject.load_probe_available === false ? "Not included in this build" : "Unknown (older report)"],
        ["Load-only probe activation", "Explicit CLI only; never client-driven"]]);
      if (processObject.code) {
        const code = processObject.code;
        rows(content, [["Code inventory source", "Offline object bytes; not verifier results"],
          ["Declared code functions", count(code.functions)], ["BPF instruction slots", count(code.instruction_slots)],
          ["Map lookup call sites", count(code.map_lookup_calls)], ["Clock helper call sites", count(code.clock_calls)],
          ["Identity helper call sites", count(code.identity_calls)], ["Ring reservation call sites", count(code.ring_reserve_calls)],
          ["Ring submission call sites", count(code.ring_submit_calls)], ["Other helper call sites", count(code.other_helper_calls)],
          ["Local call sites", count(code.local_calls)], ["Indirect call forms", count(code.indirect_calls)]]);
      }
    }
    if (processLifecycle) {
      content.append(node("h3", "Process lifecycle replay"));
      rows(content, [["Lifecycle source", "Generated input; not kernel capture"],
        ["Lifecycle sequence", processLifecycle.map(item => item.phase).join(" > ")],
        ["Lifecycle termination", "Duration limit (generated)"],
        ["Kernel operation confirmations", "None; replay only"]]);
    }
    if (processValidation) {
      content.append(node("h3", "Process protocol validation"));
      const failures = processValidation.cases.filter(item => !item.passed);
      rows(content, [["Protocol test source", "Generated streams; no kernel access"],
        ["Protocol checks", `${processValidation.cases.length - failures.length} / ${processValidation.cases.length} passed`],
        ["Failed protocol checks", failures.length ? failures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"]]);
    }
    if (processReplay?.progress) {
      const progress = processReplay.progress, failures = progress.checks.filter(item => !item.passed);
      content.append(node("h3", "Process progress protocol"));
      rows(content, [["Progress test source", "Generated session, records and receipt clocks"],
        ["Progress protocol checks", `${progress.checks.length - failures.length} / ${progress.checks.length} passed`],
        ["Failed progress checks", failures.length ? failures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"],
        ["Live health delivery", "Not connected; authentication and liveness unverified"]]);
    }
    if (sessionEvidence) {
      content.append(node("h3", "Combined session evidence"));
      rows(content, [["Evidence status", {
        sensor_free: "No optional evidence configured", awaiting_evidence: "Awaiting usable evidence",
        partial: "One evidence stream usable", independent_evidence: "Independent process and acoustic evidence",
        degraded: "Evidence integrity degraded",
      }[sessionEvidence.status]], ["Owner integrity", sessionEvidence.owner_integrity.replaceAll("_", " ")],
      ["Process evidence", {
        not_configured: "Not configured", awaiting_recent_sample: "Awaiting recent sample",
        recent_kernel_sample: "Recent kernel sample", rejected: "Rejected or invalid sample",
      }[sessionEvidence.process]], ["Acoustic evidence", {
        not_configured: "Not configured", awaiting_frame: "Awaiting frame", frame_available: "Frame available",
        processing_loss: "Processing loss detected", faulted: "Capture or processing fault",
      }[sessionEvidence.acoustic]], ["Evidence correlation", "Unavailable; streams remain independent"],
      ["Evidence attribution", "Not participant evidence"]]);
    }
    if (sessionSensors) {
      content.append(node("h3", "Combined sensor ownership"));
      rows(content, [["Sensor session status", {
        sensor_free: "No optional sensors configured", pending: "Owner work pending",
        settled: "All configured owners settled", attention: "Owner attention required",
      }[sessionSensors.status]], ["Configured sensor owners", count(sessionSensors.configured)],
      ["Pending sensor owners", count(sessionSensors.pending)],
      ["Finished sensor owners", count(sessionSensors.finished)],
      ["Combined cleanup", sessionSensors.cleanup.replaceAll("_", " ")],
      ["Combined sensor liveness", "Not established"]]);
    }
    if (sessionTiming) {
      content.append(node("h3", "Session timing boundaries"));
      rows(content, [["Voice receipt clock", "Backend session monotonic"],
        ["Acoustic receipt clock", sessionTiming.acoustic_receipt_clock === "backend_session_monotonic"
          ? "Backend session monotonic" : "Unavailable"],
        ["Acoustic frame clock", {
          backend_session_interval: "Bounded backend session interval",
          capture_relative: "Capture-relative; source clock uncalibrated",
          unavailable: "Unavailable",
        }[sessionTiming.acoustic_frame_clock]],
        ["Process timing", sessionTiming.process_timing === "receipt_freshness_only"
          ? "Receipt freshness only; no event clock" : "Unavailable"],
        ["Cross-sensor correlation", "Unavailable; no calibrated synchronization"]]);
    }
    if (acoustic || acousticOwner && acousticOwner.status !== "not_configured" || canStartAcoustic() || acousticStart) {
      content.append(node("h3", "Acoustic session"));
      if (!acoustic) rows(content, [["Acoustic source", "Prepared selected mixed output; all applications"],
        ["Acoustic status", "Not started"], ["Acoustic attribution", "Unattributed; not participant evidence"]]);
      if (acoustic) rows(content, [["Acoustic source", acoustic.source === "generated_pcm" ? "Generated PCM; not live capture" : "Selected mixed output; all applications"],
        ["Acoustic status", { starting: "Starting; awaiting frames", receiving: "Frames received; last update only", stopped: "Stopped", faulted: "Capture fault" }[acoustic.status]],
        ["Acoustic attribution", "Unattributed; not participant evidence"],
        ["Acoustic clock", acoustic.latest?.session_start_interval_ns
          ? "Backend session interval; recorder startup bounded, source clock uncalibrated"
          : acoustic.latest ? "Sample-relative; not synchronized to voice events" : "Unavailable; awaiting first frame"],
        ...(acoustic.latest?.session_start_interval_ns ? [["Latest frame session interval",
          `${(acoustic.latest.session_start_interval_ns.earliest_ns / 1e6).toFixed(1)}-${(acoustic.latest.session_start_interval_ns.latest_ns / 1e6).toFixed(1)} ms`]] : []),
        ["Acoustic accepted frames", count(acoustic.frames)], ["Acoustic discontinuities", count(acoustic.discontinuities)],
        ["Acoustic missing messages", count(acoustic.missing_messages)], ["Acoustic liveness", "Not established"]]);
      if (acousticOwner) rows(content, [["Acoustic owner status", {
        not_configured: "Not configured", prepared: "Prepared; awaiting confirmation",
        running: "Running; activity unverified", finished: "Finished",
      }[acousticOwner.status]], ["Acoustic owner outcome", acousticOwner.result
        ? acousticOwner.result.outcome === "completed" ? "Completed" : "Faulted"
        : "Not available"], ["Acoustic terminal reason", acousticOwner.result?.reason
        ? acousticOwner.result.reason.replaceAll("_", " ") : acousticOwner.result ? "Normal or owner-requested completion" : "Not available"],
      ["Acoustic cleanup", acousticOwner.result
        ? acousticOwner.result.cleanup_confirmed ? "Confirmed" : "Unconfirmed"
        : "Not yet reported"], ["Acoustic call binding", acousticOwner.call_bound
        ? acousticOwner.status === "prepared" && value.acoustic_start_available !== true
          ? "Waiting for an active voice call"
          : acousticOwner.status === "prepared"
            ? "Active voice call required; capture binds on start"
            : acousticOwner.status === "running"
              ? "Bound to originating voice call; stops on end or switch"
              : "Bound to originating voice call"
        : "Independent bounded CLI request"]]);
      if (acoustic?.latest) {
        rows(content, [["Last acoustic RMS", acoustic.latest.rms.toFixed(3)], ["Last acoustic peak", acoustic.latest.peak.toFixed(3)],
          ["Last near-full-scale ratio", acoustic.latest.clipped_sample_ratio.toFixed(3)]]);
        if (acoustic.latest.analysis) {
          const analysis = acoustic.latest.analysis;
          rows(content, [["Derived analysis scope", "Latest frame only; not a classifier or participant evidence"],
            ["Latest spectral centroid", analysis.centroid_hz === null ? "Unavailable for this frame" : `${analysis.centroid_hz.toFixed(1)} Hz`],
            ["Latest high-band power ratio", analysis.high_band_power_ratio === null ? "Unavailable for this frame" : analysis.high_band_power_ratio.toFixed(3)],
            ["Latest positive spectral flux", analysis.positive_power_flux === null ? "Unavailable for this frame" : analysis.positive_power_flux.toFixed(3)],
            ["Compared repetition lags", count(analysis.compared_lags)],
            ["Best coarse repetition lag", analysis.best_lag_ns === null ? "No comparison available" : `${(analysis.best_lag_ns / 1000000).toFixed(0)} ms`],
            ["Best coarse repetition similarity", analysis.best_similarity === null ? "No comparison available" : analysis.best_similarity.toFixed(3)]]);
        }
      }
      if (acoustic?.processing_health) {
        const health = acoustic.processing_health;
        rows(content, [["Processing health scope", "In-process queue accounting; not source or device liveness"],
          ["Processing state", health.status.state.replaceAll("_", " ")],
          ["Processed acoustic frames", count(health.processed.frames)],
          ["Delivered acoustic frames", count(health.delivered.frames)],
          ["Pending acoustic frames", count(health.pending.frames)],
          ["Discarded acoustic frames", count(health.discarded.frames)],
          ["Processing overflow events", count(health.overflow_events)],
          ["Processing queue high-water", `${count(health.high_water_batches)} / ${count(health.capacity_batches)} batches`]]);
      }
      if (acousticStart) rows(content, [["Acoustic start request", acousticStart.message]]);
      if (acousticStop) rows(content, [["Acoustic stop request", acousticStop.message]]);
    }
    if (sessionOwner) {
      content.append(node("h3", "Session process observation"));
      rows(content, [["Session owner status", { not_configured: "Not configured", awaiting_completion: "Awaiting completion; activity unverified",
        unavailable: "Outcome unavailable", finished: "Finished" }[sessionOwner.status]],
      ["Session sensor liveness", "Not established"]]);
      if (sessionOwner.result) {
        const result = sessionOwner.result;
        rows(content, [["Session owner outcome", result.outcome.replaceAll("_", " ")],
          ["Session child started", result.collector_started ? "Yes" : "No"],
          ["Session child reaped", result.collector_reaped ? "Confirmed" : "Not confirmed"],
          ["Session endpoint removed", result.endpoint_removed ? "Confirmed" : "Not confirmed"]]);
      }
    }
    const sessionProgress = sessionProgressHandoff.read();
    if (sessionProgress) {
      content.append(node("h3", "Automatic process telemetry"));
      rows(content, [["Session progress status", { not_connected: "Not connected", no_recent_sample: "No recent sample",
        generated_rejected: "Generated input rejected", invalid_sample: "Invalid sample rejected",
        recent_kernel_sample: "Recent kernel sample" }[sessionProgress.status]],
      ["Session progress liveness", "Not established"]]);
      if (sessionProgress.sample) {
        rows(content, [["Session accepted records", count(sessionProgress.sample.accepted)],
          ["Session rejected records", count(sessionProgress.sample.rejected_record)],
          ["Session clock rejections", count(sessionProgress.sample.rejected_clock)],
          ["Session ring drops", count(sessionProgress.sample.ring_dropped)]]);
      }
    }
    if (ownerOutcome) {
      content.append(node("h3", "Capture owner outcome"));
      rows(content, [["Owner result status", { not_configured: "Not configured", awaiting_completion: "Awaiting completion; activity unverified",
        unavailable: "Outcome unavailable", finished: "Finished; last explicit check" }[ownerOutcome.status]]]);
      if (ownerOutcome.result) {
        const result = ownerOutcome.result;
        rows(content, [["Owner outcome", result.outcome.replaceAll("_", " ")],
          ["Owner child started", result.collector_started ? "Yes" : "No"],
          ["Owner child reaped", result.collector_reaped ? "Confirmed" : "Not confirmed"],
          ["Owner endpoint removed", result.endpoint_removed ? "Confirmed" : "Not confirmed"]]);
      }
    }
    const progress = progressHandoff.read();
    if (progress) {
      content.append(node("h3", "Owner progress"));
      rows(content, [["Progress status", { not_connected: "Not connected", no_recent_sample: "No recent sample",
        generated_rejected: "Generated input rejected", invalid_sample: "Invalid sample rejected",
        recent_kernel_sample: "Recent kernel sample" }[progress.status]],
      ["Progress liveness", "Not established"]]);
      if (progress.sample) {
        rows(content, [["Recent accepted records", count(progress.sample.accepted)],
          ["Recent rejected records", count(progress.sample.rejected_record)],
          ["Recent clock rejections", count(progress.sample.rejected_clock)],
          ["Recent ring drops", count(progress.sample.ring_dropped)]]);
      }
    }
    if (healthSnapshot) {
      content.append(node("h3", "Supplied sensor health snapshot"));
      rows(content, [["Snapshot provenance", "Operator-supplied file; not authenticated or live"],
        ["Snapshot status", { supplied: "Completed capture report supplied", not_configured: "Not configured",
          unavailable: "Unavailable in this build", rejected: "Unreadable or invalid report",
          inspection_timed_out: "Report inspection timed out; helper reaped",
          inspection_failed: "Report inspection failed",
          cleanup_unconfirmed: "Report inspection cleanup unconfirmed" }[healthSnapshot.status]],
        ["Snapshot freshness", "Unknown; no automatic refresh or liveness claim"]]);
      if (healthSnapshot.report) {
        const run = healthSnapshot.report.run, health = run.terminal_health;
        rows(content, [["Reported collector cleanup", "Reaped; explicit detach reported"],
          ["Reported accepted records", count(health.accepted)],
          ["Reported rejected records", count(health.rejected_record)],
          ["Reported clock rejections", count(health.rejected_clock)],
          ["Reported ring drops", count(health.ring_dropped)]]);
      }
    }
    if (processSupervision) {
      content.append(node("h3", "Process supervision replay"));
      const run = processSupervision.supervision;
      const label = { completed: "Completed (generated)", collector_fault: "Collector fault (generated)",
        invalid_duration: "Invalid duration", spawn_failed: "Child launch failed", cancelled: "Cancelled", deadline: "Deadline expired",
        invalid_stream: "Status stream rejected", read_failed: "Status read failed", wait_failed: "Process wait failed", cleanup_failed: "Cleanup unconfirmed" };
      rows(content, [["Supervised collector", "Generated child process; not a kernel sensor"],
        ["Supervision result", label[run.outcome]], ["Collector process", run.collector_started ? "Started" : "Not started"],
        ["Collector cleanup", run.collector_reaped ? "Reaped" : "Not confirmed"]]);
      if (processSupervision.quiet_collection_check !== undefined) {
        rows(content, [["Quiet collection replay", processSupervision.quiet_collection_check ? "Passed (generated)" : "Failed (generated)"]]);
      }
      if (processSupervision.collection) {
        const collection = processSupervision.collection;
        rows(content, [["Joint transport source", "Generated event and lifecycle pipes; not kernel capture"],
          ["Joint transport result", collection.outcome === "invalid_stream" ? "Event or lifecycle stream rejected" : label[collection.outcome]],
          ["Joint collector cleanup", collection.collector_reaped ? "Reaped" : "Not confirmed"],
          ["Matched event records", collection.collection ? count(collection.collection.events.records) : unavailable],
          ["Joint synthetic ring drops", collection.collection ? count(collection.collection.collector_health.ring_dropped) : unavailable]]);
      }
      if (processSupervision.negotiated_collection) {
        const collection = processSupervision.negotiated_collection;
        rows(content, [["Negotiated transport source", "Same generated child; synthetic clock and events"],
          ["Negotiated transport result", collection.outcome === "invalid_stream" ? "Clock, event or lifecycle stream rejected" : label[collection.outcome]],
          ["Negotiated collector cleanup", collection.collector_reaped ? "Reaped" : "Not confirmed"],
          ["Negotiated matched records", collection.collection ? count(collection.collection.events.records) : unavailable]]);
        if (processSupervision.response_deadline_ms !== undefined) {
          rows(content, [["Clock request deadline", `${processSupervision.response_deadline_ms} ms`]]);
        }
      }
    }
    if (processReplay) {
      if (processReplay.delivery) {
        const checks = processReplay.delivery.checks;
        const failures = checks.filter(item => !item.passed);
        content.append(node("h3", "Controlled exec validation replay"));
        rows(content, [["Delivery test source", "Shared record validator; generated bytes and clocks"],
          ["Delivery record checks", `${checks.length - failures.length} / ${checks.length} passed`],
          ["Failed delivery checks", failures.length ? failures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"],
          ["Live event delivery", "Not tested by this diagnostic"]]);
      }
      if (processReplay.attachment) {
        const checks = processReplay.attachment.checks;
        const failures = checks.filter(item => !item.passed);
        content.append(node("h3", "Attach-only lifecycle replay"));
        rows(content, [["Attachment test source", "Shared probe controller; generated operations only"],
          ["Attachment lifecycle checks", `${checks.length - failures.length} / ${checks.length} passed`],
          ["Failed attachment checks", failures.length ? failures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"],
          ["Live tracepoint attachment", "Not tested by this diagnostic"]]);
      }
      content.append(node("h3", "Process-event decoder replay"));
      if (processReplay.record_layout) {
        const checks = processReplay.record_layout.checks;
        const failures = checks.filter(item => !item.passed);
        rows(content, [["Record contract source", "Shared kernel constructor; generated native bytes"],
          ["Record contract checks", `${checks.length - failures.length} / ${checks.length} passed`],
          ["Failed record checks", failures.length ? failures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"]]);
      }
      if (processReplay.producer) {
        const producer = processReplay.producer;
        const failures = producer.cases.filter(item => !item.passed);
        rows(content, [["Producer test source", "Shared kernel producer; generated two-slot sink"],
          ["Producer checks", `${producer.cases.length - failures.length} / ${producer.cases.length} passed`],
          ["Failed producer checks", failures.length ? failures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"],
          ["Synthetic submission attempts", count(producer.attempts)],
          ["Synthetic submitted records", count(producer.submitted)],
          ["Synthetic drop notifications", count(producer.dropped)]]);
        if (producer.reservation) {
          const reservationFailures = producer.reservation.filter(item => !item.passed);
          rows(content, [["Reservation test source", "Shared kernel submission; generated slots and clock"],
            ["Reservation checks", `${producer.reservation.length - reservationFailures.length} / ${producer.reservation.length} passed`],
            ["Failed reservation checks", reservationFailures.length ? reservationFailures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"]]);
        }
        if (producer.accounting) {
          const accountingFailures = producer.accounting.filter(item => !item.passed);
          rows(content, [["Loss accounting source", "Shared counter arithmetic; generated snapshots"],
            ["Loss accounting checks", `${producer.accounting.length - accountingFailures.length} / ${producer.accounting.length} passed`],
            ["Failed accounting checks", accountingFailures.length ? accountingFailures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"]]);
        }
      }
      rows(content, [["Replay source", "Generated input; not kernel capture"],
        ["Synthetic accepted events", count(processReplay.health.accepted)],
        ["Synthetic rejected records", count(processReplay.health.rejected_record)],
        ["Synthetic rejected clocks", count(processReplay.health.rejected_clock)],
        ["Synthetic ring drops", count(processReplay.health.ring_dropped)]]);
      if (processReplay.selection) {
        const failures = processReplay.selection.filter(item => !item.passed);
        rows(content, [["Target selection source", "Shared kernel filter; generated identities"],
          ["Target selection checks", `${processReplay.selection.length - failures.length} / ${processReplay.selection.length} passed`],
          ["Failed target selections", failures.length ? failures.map(item => item.name.replaceAll("_", " ")).join(", ") : "None"]]);
      }
    }
    if (!record(value) || value.schema_version !== 1) {
      content.append(node("p", value == null ? "Intelligence unavailable: no engine data received."
        : "Intelligence unavailable: missing or unsupported schema version (expected 1)."));
    } else {
      const observations = Array.isArray(value.observations) ? value.observations : null;
      const incidents = Array.isArray(value.incidents) ? value.incidents : null;
      rows(content, [["Schema version", "1"], ["Session ID", id(value.session_id)],
        ["Review resets", count(value.reset_count)],
        ["Retained observations", observations ? String(observations.length) : unavailable],
        ["Retained incidents", incidents ? String(incidents.length) : unavailable],
        ["Evicted observations", count(value.dropped_observations)], ["Evicted incidents", count(value.dropped_incidents)],
        ["Known missing voice transitions", count(value.missing_transitions)], ["Gap count", count(value.gap_count)]]);
      const ingestion = record(value.ingestion) ? value.ingestion : {};
      content.append(node("h3", "Ingestion health"), node("p", "Session-lifetime counters; evidence reset does not clear ingestion health."));
      rows(content, [
        ["Queue capacity", count(ingestion.queue_capacity)], ["Dropped queue messages", count(ingestion.dropped_full)],
        ["Accepted inputs", count(ingestion.accepted)], ["Rejected invalid inputs", count(ingestion.rejected_invalid)],
        ["Rejected closed inputs", count(ingestion.rejected_closed)], ["Rejected stale sessions", count(ingestion.rejected_session)],
        ["Rejected replayed inputs", count(ingestion.rejected_replay)], ["Rejected clock inputs", count(ingestion.rejected_clock)],
        ["Rejected state inputs", count(ingestion.rejected_state)],
        ["Diagnostics (future in-process reports)", ingestion.diagnostics === null ? "Not connected"
          : ({ available: "Available", degraded: "Degraded", unavailable: "Unavailable" })[choice(ingestion.diagnostics, ["available", "degraded", "unavailable"])] || unavailable],
      ]);
      content.append(node("p", "Retained data only. At most 50 newest incidents and 20 latest observations or gaps per section are shown. Wall-clock order may be uncertain. Baseline participants are not join events."));
      content.append(node("h3", "Newest incidents"));
      if (!incidents) content.append(node("p", "Incidents unavailable."));
      else if (!incidents.length) content.append(node("p", "No retained incidents."));
      for (const [index, item] of latest(incidents || [], 50).entries()) {
        expandable(content, entryKey("incident", item?.incident_id, index), `${choice(item?.rule, ["join_burst", "rapid_reentry"])} - ${id(item?.incident_id)}`, host => {
          rows(host, [["Incident ID", id(item?.incident_id)], ["Detector version", text(item?.detector_version)],
            ["Rule", choice(item?.rule, ["join_burst", "rapid_reentry"])], ["Explanation", text(item?.explanation)],
            ["Uncertainty", text(item?.uncertainty)], ["Confidence", choice(item?.confidence, ["uncalibrated"])],
            ["Recommended action (informational)", text(item?.recommended_action)], ["Call ID", count(item?.call_id)],
            ["Channel ID", id(item?.channel_id)], ["Wall time (UTC)", time(item?.wall_time_unix_ms)]]);
          host.append(node("h3", "Explicit evidence"));
          const evidence = Array.isArray(item?.evidence) ? item.evidence : null;
          host.append(node("p", !evidence ? "Evidence unavailable." : !evidence.length ? "No retained evidence supplied."
            : `${evidence.length} retained evidence observations${evidence.length > 100 ? "; first 100 shown" : ""}.`));
          for (const entry of (evidence || []).slice(0, 100)) {
            const block = node("div");
            block.className = "ic-intelligence-evidence";
            observation(block, entry);
            host.append(block);
          }
        });
      }
      for (const [label, entries] of [["Latest gaps", observations?.filter(item => item?.schema_version === 1 && item.kind?.type === "gap")], ["Latest observations", observations]]) {
        content.append(node("h3", label));
        if (!entries) content.append(node("p", `${label} unavailable.`));
        else if (!entries.length) content.append(node("p", `No retained ${label === "Latest gaps" ? "gaps" : "observations"}.`));
        for (const [index, item] of latest(entries || [], 20).entries()) {
          expandable(content, entryKey(label, item?.observation_id, index), `${choice(item?.kind?.type, ["joined", "left", "gap", "baseline"])} - ${id(item?.observation_id)}`, host => observation(host, item));
        }
      }
    }
    let target;
    for (const details of content.querySelectorAll("details")) {
      if (expanded.has(details.dataset.reviewKey)) details.setAttribute("open", "");
      if (focused === details.dataset.reviewKey) target = details.querySelector("summary");
    }
    if (focused) (target || root.querySelector("button")).focus();
    root.querySelector("section").scrollTop = scrollTop;
  }
  const controls = () => [...root.querySelectorAll("button,summary")].filter(element => !element.disabled && !element.closest("[hidden]") &&
    ![...root.querySelectorAll("details:not([open])")].some(details => details.contains(element) && element !== details.querySelector("summary")));
  function close() {
    clearTimeout(resetTimer);
    clearTimeout(diagnosticsTimer);
    clearProgress();
    if (diagnosticsPending) diagnosticsMessage = "Check not acknowledged";
    diagnosticsPending = null;
    pending = null;
    cancelReset();
    for (const url of downloads.keys()) releaseDownload(url);
    document.removeEventListener("keydown", keyboard, true);
    document.removeEventListener("focusin", focusInside, true);
    root?.remove();
    root = null;
    content = null;
    status = null;
    const previous = restoreFocus;
    restoreFocus = null;
    if (previous?.isConnected) previous.focus();
  }
  function keyboard(event) {
    if (!root || !["Escape", "Tab"].includes(event.key)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.key === "Escape") return close();
    const elements = controls();
    const index = elements.indexOf(document.activeElement);
    elements[(index + (event.shiftKey ? -1 : 1) + elements.length) % elements.length]?.focus();
  }
  function focusInside(event) {
    if (root && !root.contains(event.target)) root.querySelector("button").focus();
  }
  return {
    diagnostics: {
      update(response) {
        if (disposed || !diagnosticsPending || response?.session_id !== diagnosticsPending.session_id
          || response?.request_id !== diagnosticsPending.request_id || value?.session_id !== response.session_id) return;
        clearTimeout(diagnosticsTimer);
        diagnosticsPending = null;
        readiness = null;
        processReplay = null;
        processObject = null;
        processTarget = null;
        processLifecycle = null;
        processValidation = null;
        processSupervision = null;
        processPreflight = null;
        processClock = null;
        processHandshake = null;
        processTimed = null;
        healthSnapshot = null;
        clearProgress();
        if (response.schema_version !== 1 || !["host", "preview_fixture"].includes(response.source)) {
          diagnosticsMessage = "Unsupported diagnostic response";
        } else if (response.error === "rate_limited" && response.report === null) {
          diagnosticsMessage = "Check rate limited; try again shortly";
        } else if (response.error === null && validReadiness(response.report)) {
          readiness = response.report;
          if (validProcessReplay(response.process_replay)) processReplay = response.process_replay;
          if (response.process_preflight === undefined) {
            if (validProcessObject(response.process_object)) processObject = response.process_object;
          } else if (validProcessPreflight(response.process_preflight)) {
            processPreflight = response.process_preflight;
            processObject = processPreflight.object;
          }
          if (validProcessTarget(response.process_target)) processTarget = response.process_target;
          if (validProcessClock(response.process_clock)) processClock = response.process_clock;
          if (validProcessHandshake(response.process_clock_handshake)) processHandshake = response.process_clock_handshake;
          if (validProcessTimed(response.process_timed)) processTimed = response.process_timed;
          if (validProcessLifecycle(response.process_lifecycle)) processLifecycle = response.process_lifecycle;
          if (validProcessValidation(response.process_validation)) processValidation = response.process_validation;
          if (validProcessSupervision(response.process_supervision)) processSupervision = response.process_supervision;
          if (validHealthSnapshot(response.process_health_snapshot)) healthSnapshot = response.process_health_snapshot;
          if (response.source === "host") {
            ownerOutcome = readOwnerOutcome(response.process_owner);
            const progress = progressHandoff.accept(response.process_progress, diagnosticsIssuedAt);
            if (progress?.sample) progressTimer = setTimeout(() => { render(); refreshControls(); }, progress.valid_for_ms);
          }
          diagnosticsMessage = response.source === "preview_fixture" ? "Preview fixture; not a host check" : "Last explicit check; not live monitoring";
        } else diagnosticsMessage = "Invalid prerequisite report";
        render(); refreshControls();
      },
    },
    update(next) {
      if (disposed) return;
      if (next?.session_id !== value?.session_id) {
        clearTimeout(sessionProgressTimer);
        sessionProgressHandoff.clear();
        clearTimeout(acousticStartTimer);
        acousticStart = null;
        cancelAcousticStart();
        clearTimeout(acousticStopTimer);
        acousticStop = null;
        clearTimeout(diagnosticsTimer);
        diagnosticsPending = null;
        readiness = null;
        processReplay = null;
        processObject = null;
        processTarget = null;
        processLifecycle = null;
        processValidation = null;
        processSupervision = null;
        processPreflight = null;
        processClock = null;
        processHandshake = null;
        processTimed = null;
        healthSnapshot = null;
        diagnosticsMessage = "Not checked";
        clearProgress();
      }
      value = next;
      sessionOwner = validSession() ? readOwnerOutcome(value.process_owner) : null;
      sessionEvidence = validSession() ? readSessionEvidence(value.session_evidence) : null;
      sessionSensors = validSession() ? readSessionSensors(value.session_sensors) : null;
      sessionTiming = validSession() ? readSessionTiming(value.session_timing) : null;
      clearTimeout(sessionProgressTimer);
      const receivedAt = performance.now();
      const sessionProgress = sessionProgressHandoff.accept(
        validSession() ? value.process_progress : null,
        receivedAt,
      );
      if (sessionProgress?.sample) {
        sessionProgressTimer = setTimeout(() => { render(); refreshControls(); }, sessionProgress.valid_for_ms);
      }
      acoustic = validSession() ? readAcousticSummary(value.acoustic) : null;
      acousticOwner = validSession() ? readAcousticOwner(value.acoustic_owner) : null;
      if (acousticStart && acoustic) {
        clearTimeout(acousticStartTimer);
        acousticStart.state = "finished";
        acousticStart.message = "Capture start acknowledged by backend status.";
      }
      if (!acoustic) {
        clearTimeout(acousticStopTimer);
        acousticStop = null;
      } else if (acousticStop && ["stopped", "faulted"].includes(acoustic.status)) {
        clearTimeout(acousticStopTimer);
        acousticStop.state = "finished";
        acousticStop.message = acoustic.status === "stopped" ? "Capture stopped."
          : acousticOwner?.result?.cleanup_confirmed === true ? "Capture fault; cleanup confirmed."
          : acousticOwner?.result?.cleanup_confirmed === false ? "Capture fault; cleanup unconfirmed."
          : "Capture fault; cleanup may be unconfirmed.";
      }
      if (pending && value?.schema_version === 1 && value.session_id === pending.session_id && value.reset_count > pending.reset_count) {
        clearTimeout(resetTimer);
        pending = null;
        cancelReset();
        if (status) status.textContent = "Review reset acknowledged. Callmon history and session ingestion health were preserved.";
        refreshControls();
        if (root) resetButton.focus();
      } else if (armedSession && (value?.session_id !== armedSession.session_id || value?.reset_count !== armedSession.reset_count)) {
        clearTimeout(resetTimer);
        pending = null;
        cancelReset();
        if (status) status.textContent = "Review session changed. No reset acknowledgement received for the previous review.";
        if (root) root.querySelector("button").focus();
      }
      render();
      refreshControls();
    },
    open() {
      if (disposed) return;
      if (root) { root.querySelector("button").focus(); return; }
      restoreFocus = document.activeElement;
      root = node("div");
      root.className = "ic-intelligence-root ic-user-tool";
      root.dataset.intellicordOwned = "";
      const style = node("style", styles);
      const dialog = node("section");
      dialog.className = "ic-intelligence-dialog";
      dialog.dataset.intellicordOwned = "";
      dialog.setAttribute("role", "dialog");
      dialog.setAttribute("aria-modal", "true");
      dialog.setAttribute("aria-label", "Safety & Intelligence");
      const header = node("header");
      const dismiss = node("button");
      dismiss.type = "button";
      dismiss.title = "Close Safety & Intelligence";
      dismiss.setAttribute("aria-label", dismiss.title);
      dismiss.append(createElement(X, { width: 18, height: 18, "aria-hidden": "true" }));
      dismiss.addEventListener("click", close);
      const iconButton = (icon, label, action) => {
        const button = node("button");
        button.type = "button";
        button.title = label;
        button.setAttribute("aria-label", label);
        button.append(createElement(icon, { width: 18, height: 18, "aria-hidden": "true" }));
        button.addEventListener("click", action);
        return button;
      };
      exportButton = iconButton(Download, "Export retained evidence as Markdown", download);
      resetButton = iconButton(Trash2, "Reset intelligence review", () => {
        if (!resetSupported() || pending) return;
        armedSession = { session_id: value.session_id, reset_count: value.reset_count };
        confirmation.hidden = false;
        confirmation.querySelector("button").focus();
      });
      diagnosticsButton = iconButton(RefreshCw, "Check Linux sensor prerequisites", checkPrerequisites);
      acousticStartButton = iconButton(Play, "Start output analysis", () => {
        if (!canStartAcoustic() || acousticStart?.state === "pending") return;
        acousticConfirmation.hidden = false;
        acousticConfirmation.querySelector("button").focus();
      });
      acousticStopButton = iconButton(Square, "Stop output analysis", requestAcousticStop);
      header.append(node("h2", "Safety & Intelligence"), dismiss, acousticStartButton, acousticStopButton, diagnosticsButton, exportButton, resetButton);
      acousticConfirmation = node("div");
      acousticConfirmation.hidden = true;
      acousticConfirmation.setAttribute("role", "group");
      acousticConfirmation.setAttribute("aria-label", "Confirm output analysis start");
      acousticConfirmation.append(node("p", "Start bounded analysis of all applications routed to the preselected output for this voice call? No participant attribution or automatic action. Capture stops when this call ends or switches, or at the configured duration."));
      const cancelAcoustic = node("button", "Cancel");
      const confirmAcoustic = node("button", "Start analysis");
      for (const button of [cancelAcoustic, confirmAcoustic]) { button.type = "button"; button.style.width = "auto"; button.style.padding = "0 12px"; }
      cancelAcoustic.addEventListener("click", () => { cancelAcousticStart(); acousticStartButton.focus(); });
      confirmAcoustic.addEventListener("click", confirmAcousticStart);
      acousticConfirmation.append(cancelAcoustic, confirmAcoustic);
      confirmation = node("div");
      confirmation.hidden = true;
      confirmation.setAttribute("role", "group");
      confirmation.setAttribute("aria-label", "Confirm intelligence reset");
      confirmation.append(node("p", "Erase all retained intelligence observations, incidents, embedded evidence, and pending detector windows? Capture continues. Callmon history and session ingestion counters are kept. Exported files are not deleted."));
      const cancel = node("button", "Cancel");
      const confirm = node("button", "Reset review");
      for (const button of [cancel, confirm]) { button.type = "button"; button.style.width = "auto"; button.style.padding = "0 12px"; }
      cancel.addEventListener("click", () => { cancelReset(); resetButton.focus(); });
      confirm.addEventListener("click", confirmReset);
      confirmation.append(cancel, confirm);
      status = node("p");
      status.setAttribute("role", "status");
      content = node("div");
      dialog.append(header, node("p", "Informational review only. Snapshot differences are not proof of intent or network cause. Confidence is uncalibrated; no moderation actions are taken."), acousticConfirmation, confirmation, status, content);
      root.append(style, dialog);
      root.addEventListener("pointerdown", event => { if (event.target === root) close(); });
      document.body.append(root);
      render();
      refreshControls();
      document.addEventListener("keydown", keyboard, true);
      document.addEventListener("focusin", focusInside, true);
      dismiss.focus();
    },
    dispose() { disposed = true; clearTimeout(acousticStartTimer); acousticStart = null; clearTimeout(acousticStopTimer); acousticStop = null; clearTimeout(sessionProgressTimer); sessionProgressHandoff.clear(); close(); value = undefined; },
  };
}