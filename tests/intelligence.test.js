import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { build } from "esbuild";
import { formatIntelligenceMarkdown } from "../src/web/intelligence-export.js";

const bundle = await build({ entryPoints: ["src/web/intelligence.js"], bundle: true, write: false, format: "esm" });
const { mountIntelligenceReview } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);
const menuBundle = await build({ entryPoints: ["src/web/menu.js"], bundle: true, write: false, format: "esm", loader: { ".css": "text" } });
const { mountMenu } = await import(`data:text/javascript;base64,${Buffer.from(menuBundle.outputFiles[0].text).toString("base64")}`);
const userId = "9007199254740993";
const hostile = '<img src=x onerror="alert(1)"><script>bad()</script>';
const observation = (overrides = {}) => ({ schema_version: 1, observation_id: "obs:exact/0001", source: "store_notification",
  monotonic_ns: 123000000, wall_time_unix_ms: 1700000000000, session_id: "session:demo",
  subject: { user_id: userId, channel_id: "9007199254740995", call_id: 7 }, kind: { type: "joined" },
  evidence_quality: "snapshot_difference", clock_uncertainty: "Wall clock may drift", history_sequence: 42, ...overrides });
const incident = (overrides = {}) => ({ incident_id: "incident:exact/0001", detector_version: "phase1.v1", rule: "join_burst",
  explanation: hostile, uncertainty: "Snapshot order is uncertain", confidence: "uncalibrated", recommended_action: "Review retained evidence only",
  call_id: 7, channel_id: "9007199254740995", wall_time_unix_ms: 1700000000000, evidence: [observation()], ...overrides });
const data = (overrides = {}) => ({ schema_version: 1, session_id: "session:demo", observations: [observation()], incidents: [incident()],
  reset_count: 0, dropped_observations: 3, dropped_incidents: 2, missing_transitions: 5, gap_count: 1, ...overrides });

function fixture(options) {
  const original = globalThis.document;
  const originalWindow = globalThis.window;
  const { window, document } = parseHTML('<html><body><button id="origin">Origin</button></body></html>');
  globalThis.document = document;
  globalThis.window = window;
  let focused = document.querySelector("#origin");
  Object.defineProperty(document, "activeElement", { get: () => focused });
  window.HTMLElement.prototype.focus = function () {
    if (this.disabled || this.closest("[hidden]") || !this.isConnected) return;
    focused = this; this.dispatchEvent(new window.Event("focusin", { bubbles: true }));
  };
  const review = mountIntelligenceReview(options);
  return { document, window, review,
    get dialog() { return document.querySelector('[role="dialog"][aria-label="Safety & Intelligence"]'); },
    key(key, shiftKey = false) {
      const event = new window.Event("keydown", { bubbles: true, cancelable: true });
      Object.assign(event, { key, shiftKey });
      (document.activeElement || document).dispatchEvent(event);
      return event;
    },
    cleanup() { review.dispose(); globalThis.document = original; globalThis.window = originalWindow; },
  };
}
const field = (host, label) => [...host.querySelectorAll("dt")].find(node => node.textContent === label)?.nextElementSibling.textContent;

const readinessReport = () => ({ schema_version: 1, platform: "linux", kernel_btf: "present", cgroup_v2: "missing",
  unprivileged_bpf: "disabled_configurable", sensor_running: false, load_test_performed: false });

test("timed process replay keeps generated checks, counters and failures separate from host telemetry", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const report = { schema_version: 1, source: "generated_replay", clock_domain: "linux_monotonic", sensor_running: false, load_test_performed: false,
    health: { accepted: 2, rejected_record: 1, rejected_receipt_clock: 1, rejected_anchor_expired: 1,
      rejected_source_clock: 1, rejected_mapping: 1, rejected_future: 1, rejected_stale: 1 },
    cases: ["interval_bounds", "foreign_record", "receipt_regression", "source_regression", "anchor_expiry", "mapping_expiry",
      "future_event", "stale_event", "rejection_isolation"].map(name => ({ name, passed: true })) };
  const submit = (process_timed, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_timed, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(report);
    assert.equal(field(state.dialog, "Timed replay source"), "Generated records and clocks; not kernel capture");
    assert.equal(field(state.dialog, "Timed replay checks"), "9 / 9 passed");
    assert.equal(field(state.dialog, "Timed accepted records"), "2");
    assert.equal(field(state.dialog, "Timed expired anchors"), "1");
    assert.equal(field(state.dialog, "Timed stale events"), "1");
    assert.equal(field(state.dialog, "Event transport checks"), undefined);
    const transport = ["fragmented_delivery", "truncated_record", "foreign_record", "stream_limit", "receipt_regression",
      "failed_exit", "empty_stream", "delayed_fragment"].map(name => ({ name, passed: true }));
    submit({ ...report, transport });
    assert.equal(field(state.dialog, "Event transport checks"), "8 / 8 passed");
    const failedTransport = structuredClone(transport); failedTransport[7].passed = false;
    submit({ ...report, transport: failedTransport });
    assert.equal(field(state.dialog, "Event transport checks"), "7 / 8 passed");
    assert.equal(field(state.dialog, "Failed transport checks"), "delayed fragment");
    for (const invalid of [null, [], transport.slice(1), [...transport].reverse(),
      transport.map(item => ({ ...item, passed: 1 })), transport.map(item => ({ ...item, name: "unknown" }))]) {
      submit({ ...report, transport: invalid });
      assert.equal(field(state.dialog, "Timed replay checks"), undefined);
      assert.equal(field(state.dialog, "Event transport checks"), undefined);
    }
    const failed = structuredClone(report); failed.cases[4].passed = false;
    submit(failed);
    assert.equal(field(state.dialog, "Timed replay checks"), "8 / 9 passed");
    assert.equal(field(state.dialog, "Failed timed checks"), "anchor expiry");
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { clock_domain: "boottime" },
      { sensor_running: true }, { load_test_performed: true }, { cases: report.cases.slice(1) },
      { cases: [...report.cases].reverse() }, { cases: report.cases.map(item => ({ ...item, passed: 1 })) },
      { health: { ...report.health, accepted: Number.MAX_SAFE_INTEGER + 1 } }, { health: { ...report.health, rejected_stale: -1 } }]) {
      submit({ ...report, ...change });
      assert.equal(field(state.dialog, "Timed replay checks"), undefined);
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Timed replay checks"), undefined);
    submit(report, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(report, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Timed replay checks"), undefined);
    submit(report); state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Timed replay checks"), undefined);
  } finally { state.cleanup(); }
});

test("clock handoff diagnostics reject contradictory or active claims and clear stale results", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const report = { schema_version: 1, source: "local_clock_handshake", status: "available", clock_domain: "linux_monotonic",
    max_bracket_ns: 25000000, sensor_running: false, load_test_performed: false,
    result: { schema_version: 1, outcome: "completed", helper_started: true, helper_reaped: true, bracket_width_ns: 180000 } };
  const submit = (process_clock_handshake, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_clock_handshake, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(report);
    assert.equal(field(state.dialog, "Clock handshake"), "Bounded exchange passed");
    assert.equal(field(state.dialog, "Handshake bracket"), "180000 ns");
    assert.equal(field(state.dialog, "Clock helper cleanup"), "Reaped");
    assert.equal(field(state.dialog, "Clock namespace identity"), "Not verified");
    assert.equal(field(state.dialog, "Live collector clock handoff"), "Not connected");
    for (const status of ["feature_disabled", "unsupported"]) {
      submit({ ...report, status, result: null });
      assert.equal(field(state.dialog, "Clock helper cleanup"), "Not started");
    }
    for (const outcome of ["deadline", "invalid_stream", "read_failed", "wait_failed", "collector_fault", "cancelled", "spawn_failed", "cleanup_failed"]) {
      submit({ ...report, status: "unavailable", result: { ...report.result, outcome,
        helper_started: outcome !== "spawn_failed", helper_reaped: !["spawn_failed", "cleanup_failed"].includes(outcome), bracket_width_ns: null } });
      assert.equal(field(state.dialog, "Clock handshake"), "Exchange failed");
      assert.equal(field(state.dialog, "Handshake outcome"), outcome.replaceAll("_", " "));
    }
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { clock_domain: "boottime" },
      { sensor_running: true }, { load_test_performed: true }, { max_bracket_ns: 26000000 }, { status: "unavailable" },
      { source_ns: 123 }, { result: null }, ...[{ helper_reaped: false }, { helper_started: false }, { schema_version: 2 },
        { outcome: "deadline" }, { bracket_width_ns: -1 }, { bracket_width_ns: 25000001 }, { bracket_width_ns: null },
        { bracket_width_ns: 0.1 }, { bracket_width_ns: Number.MAX_SAFE_INTEGER + 1 }, { nonce: 42 }]
        .map(change => ({ result: { ...report.result, ...change } }))]) {
      submit({ ...report, ...change });
      assert.equal(field(state.dialog, "Clock handshake"), undefined);
    }
    for (const bracket_width_ns of [0, 25000000]) {
      submit({ ...report, result: { ...report.result, bracket_width_ns } });
      assert.equal(field(state.dialog, "Handshake bracket"), `${bracket_width_ns} ns`);
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Clock handshake"), undefined);
    submit(report, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(report, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Clock handshake"), undefined);
    submit(report); state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Clock handshake"), undefined);
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_clock_handshake: report });
    assert.equal(field(state.dialog, "Clock handshake"), undefined);
  } finally { state.cleanup(); }
});

test("kernel clock diagnostics distinguish local measurements from synchronization assumptions", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const report = { schema_version: 1, source: "local_kernel_clock_probe", status: "available", clock_domain: "linux_monotonic",
    bracket_width_ns: 2400, horizon_ns: 1000000000, assumed_rate_error_ppm: 100, sensor_running: false, load_test_performed: false };
  const submit = (process_clock, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_clock, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(report);
    assert.equal(field(state.dialog, "Clock probe"), "Local clock read passed");
    assert.equal(field(state.dialog, "Clock read bracket"), "2400 ns");
    assert.equal(field(state.dialog, "Assumed relative rate bound"), "100 ppm; not measured");
    assert.equal(field(state.dialog, "Cross-process synchronization"), "Not verified by this local probe");
    for (const status of ["unavailable", "feature_disabled", "unsupported"]) {
      submit({ ...report, status, bracket_width_ns: null });
      assert.equal(field(state.dialog, "Clock read bracket"), "Unavailable");
    }
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { clock_domain: "boottime" },
      { sensor_running: true }, { load_test_performed: true }, { horizon_ns: 0 }, { assumed_rate_error_ppm: 0 },
      { bracket_width_ns: -1 }, { bracket_width_ns: null }, { bracket_width_ns: 0.1 }, { bracket_width_ns: Number.MAX_SAFE_INTEGER + 1 },
      { status: "unsupported" }, { status: "synchronized" }]) {
      submit({ ...report, ...change });
      assert.equal(field(state.dialog, "Clock probe"), undefined);
    }
    submit({ ...report, bracket_width_ns: 0 });
    assert.equal(field(state.dialog, "Clock read bracket"), "0 ns");
    submit(undefined);
    assert.equal(field(state.dialog, "Clock probe"), undefined);
    submit(report, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(report, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Clock probe"), undefined);
    submit(report); state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Clock probe"), undefined);
  } finally { state.cleanup(); }
});

test("offline object helper status is authoritative and never implies BPF loading", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const object = { schema_version: 1, inspector_available: true, status: "layout_matched", load_test_performed: false, sensor_running: false };
  const report = { schema_version: 1, source: "offline_object_preflight", sensor_running: false, load_test_performed: false,
    outcome: "completed", helper_started: true, helper_reaped: true, object };
  const submit = (process_preflight, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_object: object, process_preflight, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(report);
    assert.equal(field(state.dialog, "Inspection result"), "Reply validated");
    assert.equal(field(state.dialog, "Inspection cleanup"), "Reaped");
    assert.equal(field(state.dialog, "Inspection process"), "Separate offline helper; no BPF loading");
    assert.equal(field(state.dialog, "Object metadata"), "Expected layout matched; behavior unverified");
    for (const [outcome, label, started, reaped] of [["spawn_failed", "Helper launch failed", false, false],
      ["deadline", "Deadline expired", true, true], ["invalid_stream", "Invalid helper reply", true, true],
      ["collector_fault", "Helper exited unsuccessfully", true, true], ["cleanup_failed", "Cleanup unconfirmed", true, false]]) {
      submit({ ...report, outcome, helper_started: started, helper_reaped: reaped, object: null });
      assert.equal(field(state.dialog, "Inspection result"), label);
      assert.equal(field(state.dialog, "Object metadata"), undefined);
    }
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { sensor_running: true },
      { load_test_performed: true }, { helper_reaped: false }, { helper_started: false }, { outcome: "unknown" },
      { object: { ...object, sensor_running: true } }, { object: null }]) {
      submit({ ...report, ...change });
      assert.equal(field(state.dialog, "Inspection result"), undefined);
      assert.equal(field(state.dialog, "Object metadata"), undefined);
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Object metadata"), "Expected layout matched; behavior unverified");
    submit(report, { source: "preview_fixture" });
    assert.equal(state.dialog.textContent.includes("Preview fixture; not a host check"), true);
    submit(report, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Inspection result"), undefined);
    submit(report); state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Inspection result"), undefined);
  } finally { state.cleanup(); }
});

test("load-only probe availability never implies a load request or result", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const submit = (available, changes = {}, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null, report: readinessReport(),
      process_preflight: { schema_version: 1, source: "offline_object_preflight", sensor_running: false, load_test_performed: false,
        outcome: "completed", helper_started: true, helper_reaped: true,
        object: { schema_version: 1, inspector_available: true, status: "not_configured", load_test_performed: false,
          sensor_running: false, load_probe_available: available, ...changes } }, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(true);
    assert.equal(field(state.dialog, "Load-only probe build"), "Included; not requested");
    assert.equal(field(state.dialog, "Load-only probe activation"), "Explicit CLI only; never client-driven");
    assert.equal(field(state.dialog, "Object load and attach"), "Not tested");
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    submit(false, { inspector_available: false, status: "feature_disabled" });
    assert.equal(field(state.dialog, "Load-only probe build"), "Not included in this build");
    submit(undefined);
    assert.equal(field(state.dialog, "Load-only probe build"), "Unknown (older report)");
    for (const invalid of [null, "true", 1, {}, [], false]) {
      submit(invalid);
      assert.equal(field(state.dialog, "Load-only probe build"), undefined);
      assert.equal(field(state.dialog, "Object metadata"), undefined);
    }
    submit(true, { inspector_available: false, status: "feature_disabled" });
    assert.equal(field(state.dialog, "Load-only probe build"), undefined);
    submit(true, { load_test_performed: true });
    assert.equal(field(state.dialog, "Load-only probe build"), undefined);
    submit(true, {}, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(true, {}, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Load-only probe build"), undefined);
    submit(true); state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Load-only probe build"), undefined);
  } finally { state.cleanup(); }
});

test("compiled object inventory is bounded, offline, optional and session scoped", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const code = { schema_version: 1, functions: 4, instruction_slots: 100, map_lookup_calls: 2, clock_calls: 1, identity_calls: 1,
    ring_reserve_calls: 1, ring_submit_calls: 1, other_helper_calls: 0, local_calls: 0, indirect_calls: 0 };
  const submit = (inventory, changes = {}, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null, report: readinessReport(),
      process_preflight: { schema_version: 1, source: "offline_object_preflight", sensor_running: false, load_test_performed: false,
        outcome: "completed", helper_started: true, helper_reaped: true,
        object: { schema_version: 1, inspector_available: true, status: "layout_matched", load_test_performed: false,
          sensor_running: false, code: inventory, ...changes } }, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(code);
    for (const [label, expected] of [["Code inventory source", "Offline object bytes; not verifier results"],
      ["Declared code functions", "4"], ["BPF instruction slots", "100"], ["Map lookup call sites", "2"],
      ["Clock helper call sites", "1"], ["Identity helper call sites", "1"], ["Ring reservation call sites", "1"],
      ["Ring submission call sites", "1"], ["Other helper call sites", "0"], ["Local call sites", "0"],
      ["Indirect call forms", "0"], ["Object load and attach", "Not tested"]]) assert.equal(field(state.dialog, label), expected);
    submit({ ...code, other_helper_calls: 2, local_calls: 3, indirect_calls: 1 });
    assert.equal(field(state.dialog, "Other helper call sites"), "2");
    assert.equal(field(state.dialog, "Local call sites"), "3");
    assert.equal(field(state.dialog, "Indirect call forms"), "1");
    for (const invalid of [null, {}, [], { ...code, schema_version: 2 }, { ...code, functions: 0 },
      { ...code, functions: 4097 }, { ...code, instruction_slots: 3 }, { ...code, instruction_slots: 524289 },
      { ...code, clock_calls: -1 }, { ...code, clock_calls: 0.1 }, { ...code, clock_calls: "1" },
      { ...code, clock_calls: Number.MAX_SAFE_INTEGER + 1 }, { ...code, clock_calls: 99 }, { ...code, unknown: 0 },
      ...Object.keys(code).map(key => { const missing = { ...code }; delete missing[key]; return missing; })]) {
      submit(invalid);
      assert.equal(field(state.dialog, "BPF instruction slots"), undefined);
      assert.equal(field(state.dialog, "Object metadata"), undefined);
    }
    for (const status of ["rejected", "unreadable", "not_configured"]) {
      submit(code, { status });
      assert.equal(field(state.dialog, "BPF instruction slots"), undefined);
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Object metadata"), "Expected layout matched; behavior unverified");
    assert.equal(field(state.dialog, "BPF instruction slots"), undefined);
    submit(code, {}, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(code, {}, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "BPF instruction slots"), undefined);
    submit(code); state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "BPF instruction slots"), undefined);
  } finally { state.cleanup(); }
});

test("supervision replay separates generated process cleanup from kernel claims", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const report = { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
    supervision: { schema_version: 1, outcome: "completed", collector_started: true, collector_reaped: true,
      status: { schema_version: 1, source: "generated_replay", records: 6, terminal_phase: "stopped", reason: "duration_limit",
        attachment_confirmed: false, detachment_confirmed: false } } };
  const submit = (process_supervision, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_supervision, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(report);
    assert.equal(field(state.dialog, "Supervision result"), "Completed (generated)");
    assert.equal(field(state.dialog, "Collector cleanup"), "Reaped");
    assert.equal(field(state.dialog, "Supervised collector"), "Generated child process; not a kernel sensor");
    for (const [outcome, label, started, reaped] of [["deadline", "Deadline expired", true, true],
      ["invalid_stream", "Status stream rejected", true, true], ["cancelled", "Cancelled", true, true],
      ["cancelled", "Cancelled", false, false], ["spawn_failed", "Child launch failed", false, false],
      ["cleanup_failed", "Cleanup unconfirmed", true, false]]) {
      submit({ ...report, supervision: { ...report.supervision, outcome, status: null, collector_started: started, collector_reaped: reaped } });
      assert.equal(field(state.dialog, "Supervision result"), label);
      assert.equal(field(state.dialog, "Collector cleanup"), reaped ? "Reaped" : "Not confirmed");
    }
    submit({ ...report, supervision: { ...report.supervision, outcome: "collector_fault", status: {
      ...report.supervision.status, records: 2, terminal_phase: "faulted", reason: "invalid_object" } } });
    assert.equal(field(state.dialog, "Supervision result"), "Collector fault (generated)");
    submit({ ...report, supervision: { ...report.supervision, outcome: "collector_fault", status: {
      ...report.supervision.status, records: 2, terminal_phase: "faulted", reason: "delivery_failed" } } });
    assert.equal(field(state.dialog, "Supervision result"), "Collector fault (generated)");
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { sensor_running: true }, { load_test_performed: true },
      { supervision: { ...report.supervision, collector_reaped: false } },
      { supervision: { ...report.supervision, collector_started: false } },
      { supervision: { ...report.supervision, outcome: "unknown" } },
      { supervision: { ...report.supervision, status: { ...report.supervision.status, attachment_confirmed: true } } },
      { supervision: { ...report.supervision, status: { ...report.supervision.status, source: "kernel_tracepoint" } } }]) {
      submit({ ...report, ...change });
      assert.equal(field(state.dialog, "Supervision result"), undefined);
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Supervision result"), undefined);
    submit(report, { source: "preview_fixture" });
    assert.equal(state.dialog.textContent.includes("Preview fixture; not a host check"), true);
    submit(report, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Supervision result"), undefined);
    submit(report); state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Supervision result"), undefined);
  } finally { state.cleanup(); }
});

test("joint transport requires generated provenance, reaping and event-count agreement", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const status = { schema_version: 1, source: "generated_replay", records: 6, terminal_phase: "stopped", reason: "duration_limit",
    attachment_confirmed: false, detachment_confirmed: false };
  const collection = { schema_version: 1, outcome: "completed", collector_started: true, collector_reaped: true,
    collection: { schema_version: 1, status,
      events: { schema_version: 1, records: 2, health: { accepted: 2, rejected_record: 0, rejected_receipt_clock: 0,
        rejected_anchor_expired: 0, rejected_source_clock: 0, rejected_mapping: 0, rejected_future: 0, rejected_stale: 0 } },
      collector_health: { schema_version: 1, accepted: 2, rejected_record: 0, rejected_clock: 0, ring_dropped: 3 } } };
  const submit = (result, extra = {}, negotiated_collection = undefined, response_deadline_ms = undefined, quiet_collection_check = undefined) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null, report: readinessReport(),
      process_supervision: { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
        supervision: { schema_version: 1, outcome: "completed", collector_started: true, collector_reaped: true, status },
        collection: result, negotiated_collection, response_deadline_ms, quiet_collection_check }, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(collection);
    assert.equal(field(state.dialog, "Joint transport result"), "Completed (generated)");
    assert.equal(field(state.dialog, "Joint collector cleanup"), "Reaped");
    assert.equal(field(state.dialog, "Matched event records"), "2");
    assert.equal(field(state.dialog, "Joint synthetic ring drops"), "3");
    assert.equal(field(state.dialog, "Joint transport source"), "Generated event and lifecycle pipes; not kernel capture");
    assert.equal(field(state.dialog, "Negotiated transport result"), undefined);
    assert.equal(field(state.dialog, "Quiet collection replay"), undefined);
    for (const passed of [true, false]) {
      submit(collection, {}, undefined, undefined, passed);
      assert.equal(field(state.dialog, "Quiet collection replay"), passed ? "Passed (generated)" : "Failed (generated)");
    }
    for (const invalid of [null, 0, "true", {}, []]) {
      submit(collection, {}, undefined, undefined, invalid);
      assert.equal(field(state.dialog, "Quiet collection replay"), undefined);
      assert.equal(field(state.dialog, "Supervision result"), undefined);
    }
    submit(collection, {}, collection);
    assert.equal(field(state.dialog, "Negotiated transport result"), "Completed (generated)");
    assert.equal(field(state.dialog, "Negotiated collector cleanup"), "Reaped");
    assert.equal(field(state.dialog, "Negotiated matched records"), "2");
    assert.equal(field(state.dialog, "Negotiated transport source"), "Same generated child; synthetic clock and events");
    assert.equal(field(state.dialog, "Clock request deadline"), undefined);
    submit(collection, {}, collection, 2000);
    assert.equal(field(state.dialog, "Clock request deadline"), "2000 ms");
    const quiet = structuredClone(collection);
    quiet.collection.events.records = 0;
    quiet.collection.events.health.accepted = 0;
    quiet.collection.collector_health.accepted = 0;
    submit(quiet, {}, quiet);
    assert.equal(field(state.dialog, "Joint transport result"), "Completed (generated)");
    assert.equal(field(state.dialog, "Matched event records"), "0");
    assert.equal(field(state.dialog, "Negotiated matched records"), "0");
    assert.equal(field(state.dialog, "Joint synthetic ring drops"), "3");
    for (const corrupt of [value => { value.collection.collector_health.accepted = 1; },
      value => { value.collection.events.health.accepted = 1; },
      value => { value.collection.events.records = -1; },
      value => { value.collection.status.terminal_phase = "observing"; }]) {
      const invalid = structuredClone(quiet); corrupt(invalid);
      submit(invalid, {}, invalid);
      assert.equal(field(state.dialog, "Joint transport result"), undefined);
      assert.equal(field(state.dialog, "Negotiated transport result"), undefined);
    }
    for (const invalid of [null, 0, 1999, 2001, "2000", Number.MAX_SAFE_INTEGER + 1]) {
      submit(collection, {}, collection, invalid);
      assert.equal(field(state.dialog, "Negotiated transport result"), undefined);
      assert.equal(field(state.dialog, "Clock request deadline"), undefined);
    }
    for (const [outcome, started, reaped] of [["deadline", true, true], ["invalid_stream", true, true],
      ["cancelled", false, false], ["cleanup_failed", true, false], ["spawn_failed", false, false]]) {
      submit({ ...collection, outcome, collector_started: started, collector_reaped: reaped, collection: null });
      assert.notEqual(field(state.dialog, "Joint transport result"), undefined);
      assert.equal(field(state.dialog, "Joint collector cleanup"), reaped ? "Reaped" : "Not confirmed");
      assert.notEqual(field(state.dialog, "Matched event records"), "2");
      submit(collection, {}, { ...collection, outcome, collector_started: started, collector_reaped: reaped, collection: null });
      assert.notEqual(field(state.dialog, "Negotiated transport result"), undefined);
      assert.notEqual(field(state.dialog, "Negotiated matched records"), "2");
    }
    const corruptions = [value => { value.schema_version = 2; }, value => { value.collector_reaped = false; },
      value => { value.outcome = "unknown"; }, value => { value.collection.status.source = "kernel_tracepoint"; },
      value => { value.collection.status.attachment_confirmed = true; }, value => { value.collection.collector_health.accepted = 1; },
      value => { value.collection.events.records = 257; }, value => { value.collection.events.health.accepted = 1; },
      value => { value.collection.events.health.rejected_record = 1; }, value => { value.collection.collector_health.ring_dropped = -1; },
      value => { value.collection.collector_health.ring_dropped = Number.MAX_SAFE_INTEGER + 1; },
      value => { delete value.collection.events.health.rejected_future; }];
    for (const corrupt of corruptions) {
      const invalid = structuredClone(collection); corrupt(invalid); submit(invalid);
      assert.equal(field(state.dialog, "Joint transport result"), undefined);
      submit(collection, {}, invalid);
      assert.equal(field(state.dialog, "Negotiated transport result"), undefined);
    }
    submit(null); assert.equal(field(state.dialog, "Joint transport result"), undefined);
    submit(undefined); assert.equal(field(state.dialog, "Supervision result"), "Completed (generated)");
    submit(collection, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(collection, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Joint transport result"), undefined);
    submit(collection, {}, collection); state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Joint transport result"), undefined);
    assert.equal(field(state.dialog, "Negotiated transport result"), undefined);
  } finally { state.cleanup(); }
});

test("protocol validation exposes failed checks and rejects live or malformed claims", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const report = { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
    cases: ["normal_completion", "fault_completion", "truncated_record", "sequence_replay", "source_mismatch",
      "confirmation_forgery", "health_regression", "exit_mismatch"].map(name => ({ name, passed: true })) };
  const submit = (process_validation, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_validation, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(report);
    assert.equal(field(state.dialog, "Protocol checks"), "8 / 8 passed");
    assert.equal(field(state.dialog, "Protocol test source"), "Generated streams; no kernel access");
    submit({ ...report, cases: report.cases.map((item, index) => ({ ...item, passed: index !== 2 })) });
    assert.equal(field(state.dialog, "Protocol checks"), "7 / 8 passed");
    assert.equal(field(state.dialog, "Failed protocol checks"), "truncated record");
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { sensor_running: true },
      { load_test_performed: true }, { cases: [] }, { cases: report.cases.slice(1) },
      { cases: report.cases.map(item => ({ ...item, passed: "true" })) },
      { cases: report.cases.map(item => ({ ...item, name: "unknown" })) }]) {
      submit({ ...report, ...change });
      assert.equal(field(state.dialog, "Protocol checks"), undefined);
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Protocol checks"), undefined);
    submit(report, { source: "preview_fixture" });
    assert.equal(state.dialog.textContent.includes("Preview fixture; not a host check"), true);
    submit(report, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Protocol checks"), undefined);
    submit(report);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Protocol checks"), undefined);
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_validation: report });
    assert.equal(field(state.dialog, "Protocol checks"), undefined);
  } finally { state.cleanup(); }
});

test("lifecycle replay validates ordering and provenance and clears stale records", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const records = ["validating", "loading", "attaching", "observing", "stopping", "stopped"].map((phase, index) => ({
    schema_version: 1, record_type: "process_status", source: "generated_replay", sequence: index + 1, phase,
    reason: index === 5 ? "duration_limit" : null, attachment_confirmed: false, detachment_confirmed: false,
    health: index === 5 ? { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 } : null,
  }));
  const submit = (process_lifecycle, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_replay: { schema_version: 1, source: "generated_replay", sensor_running: false,
        health: records[5].health }, process_lifecycle, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(records);
    assert.equal(field(state.dialog, "Lifecycle source"), "Generated input; not kernel capture");
    assert.equal(field(state.dialog, "Kernel operation confirmations"), "None; replay only");
    submit(records, { process_replay: undefined });
    assert.equal(field(state.dialog, "Lifecycle source"), "Generated input; not kernel capture");
    for (const change of [{ sequence: 99 }, { phase: "faulted" }, { source: "kernel_tracepoint" },
      { attachment_confirmed: true }, { detachment_confirmed: true }, { reason: "duration_limit" },
      { schema_version: 2 }, { record_type: "capture_status" }, { health: {} }]) {
      submit(records.map((item, index) => index === 0 ? { ...item, ...change } : item));
      assert.equal(field(state.dialog, "Lifecycle source"), undefined);
    }
    submit(records.slice(1));
    assert.equal(field(state.dialog, "Lifecycle source"), undefined);
    submit(records.map((item, index) => index === 5 ? { ...item, health: { ...item.health, accepted: -1 } } : item));
    assert.equal(field(state.dialog, "Lifecycle source"), undefined);
    submit(records, { source: "preview_fixture" });
    assert.equal(state.dialog.textContent.includes("Preview fixture; not a host check"), true);
    submit(records, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Lifecycle source"), undefined);
    submit(records);
    state.review.update(data({ session_id: "new-session" }));
    assert.equal(field(state.dialog, "Lifecycle source"), undefined);
  } finally { state.cleanup(); }
});

test("target safety probes never imply selected targets or live attachment", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const probe = { schema_version: 1, source: "self_pidfd_probe", status: "available",
    target_selected: false, sensor_running: false, load_test_performed: false };
  const submit = (process_target, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_target, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    assert.equal(field(state.dialog, "Process lifetime guard"), undefined);
    for (const [status, label] of [
      ["available", "Self-process pidfd probe passed"], ["unavailable", "Self-process pidfd probe failed"],
      ["feature_disabled", "Optional observe feature not built"], ["unsupported", "Unsupported platform"],
    ]) {
      submit({ ...probe, status });
      assert.equal(field(state.dialog, "Process lifetime guard"), label);
      assert.equal(field(state.dialog, "Observation target"), "None selected");
      assert.equal(field(state.dialog, "Target identity binding"), "Not verified by this probe");
    }
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { status: "live" },
      { target_selected: true }, { target_selected: "false" }, { sensor_running: true },
      { load_test_performed: true }, { target_selected: undefined }]) {
      submit(probe);
      submit({ ...probe, ...change });
      assert.equal(field(state.dialog, "Process lifetime guard"), undefined);
      assert.equal(field(state.dialog, "Kernel BTF metadata"), "Present");
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Process lifetime guard"), undefined);
    submit(probe, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    state.review.update(data({ session_id: "next-target-session" }));
    assert.equal(field(state.dialog, "Process lifetime guard"), undefined);
    submit(probe);
    submit(null, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Process lifetime guard"), undefined);
  } finally { state.cleanup(); }
});

test("object metadata remains offline, optional and session scoped", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const object = { schema_version: 1, inspector_available: true, status: "layout_matched",
    load_test_performed: false, sensor_running: false };
  const submit = process_object => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_object });
  };
  try {
    state.review.update(data()); state.review.open();
    for (const [status, inspector_available, label] of [
      ["not_configured", true, "No object configured"],
      ["layout_matched", true, "Expected layout matched; behavior unverified"],
      ["rejected", true, "Object rejected"],
      ["unreadable", true, "Object unavailable or outside file limits"],
      ["feature_disabled", false, "Optional observe feature not built"],
      ["unsupported", false, "Unsupported platform"],
    ]) {
      submit({ ...object, status, inspector_available });
      assert.equal(field(state.dialog, "Object metadata"), label);
      assert.equal(field(state.dialog, "Object load and attach"), "Not tested");
    }
    for (const change of [{ schema_version: 2 }, { status: "live" }, { sensor_running: true },
      { load_test_performed: true }, { inspector_available: false }, { inspector_available: "true" }]) {
      submit(object);
      submit({ ...object, ...change });
      assert.equal(field(state.dialog, "Object metadata"), undefined);
      assert.equal(field(state.dialog, "Kernel BTF metadata"), "Present");
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Object metadata"), undefined);
    submit(object);
    state.review.update(data({ session_id: "next-session" }));
    assert.equal(field(state.dialog, "Object metadata"), undefined);
  } finally { state.cleanup(); }
});

test("prerequisite checks are explicit, session matched and distinguish preview from host", () => {
  const requests = [];
  const state = fixture({ checkDiagnostics: request => requests.push(request) });
  try {
    state.review.update(data()); state.review.open();
    assert.equal(requests.length, 0);
    const button = state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]');
    button.click();
    assert.deepEqual(requests, [{ session_id: "session:demo", request_id: 1 }]);
    assert.equal(button.disabled, true);
    const response = { schema_version: 1, ...requests[0], source: "preview_fixture", error: null, report: readinessReport() };
    state.review.diagnostics.update({ ...response, session_id: "stale" });
    state.review.diagnostics.update({ ...response, request_id: 2 });
    assert.equal(field(state.dialog, "Check status"), "Checking");
    state.review.diagnostics.update(response);
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    assert.equal(button.disabled, false);
    button.click();
    state.review.diagnostics.update({ ...response, ...requests[1], source: "host" });
    assert.equal(field(state.dialog, "Check status"), "Last explicit check; not live monitoring");
    state.review.update(data({ session_id: "new-session" }));
    assert.equal(field(state.dialog, "Check status"), "Not checked");
    assert.equal(field(state.dialog, "Kernel BTF metadata"), undefined);
  } finally { state.cleanup(); }
});

test("prerequisite checks reject positive sensor claims and expose request failure", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  try {
    state.review.update(data()); state.review.open();
    const button = state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]');
    for (const change of [{ schema_version: 3 }, { schema_version: 2 }, { sensor_running: true }, { load_test_performed: true }, { kernel_btf: "<script>" }]) {
      button.click();
      state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null, report: { ...readinessReport(), ...change } });
      assert.equal(field(state.dialog, "Check status"), "Invalid prerequisite report");
      assert.equal(field(state.dialog, "Kernel BTF metadata"), undefined);
    }
    button.click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: "rate_limited", report: null });
    assert.match(field(state.dialog, "Check status"), /rate limited/);
  } finally { state.cleanup(); }
  const failed = fixture({ checkDiagnostics: () => { throw new Error("offline"); } });
  try {
    failed.review.update(data()); failed.review.open();
    failed.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    assert.equal(field(failed.dialog, "Check status"), "Check request failed");
  } finally { failed.cleanup(); }
});

test("kernel execution restrictions preserve old reports and reject malformed context", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const report = { ...readinessReport(), schema_version: 3, perf_event_paranoid: 2,
    effective_capabilities: null, sched_process_exec: "unreadable", lockdown: "unknown",
    no_new_privileges: true, seccomp: "filter" };
  const submit = value => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null, report: value });
  };
  try {
    state.review.update(data()); state.review.open();
    for (const [mode, expected] of [["disabled", "Disabled"], ["strict", "Strict"], ["filter", "Filter installed"],
      ["unknown", "Unknown"], ["unsupported", "Unsupported"]]) {
      submit({ ...report, seccomp: mode });
      assert.equal(field(state.dialog, "Seccomp mode"), expected);
      assert.equal(field(state.dialog, "No-new-privileges"), "Set");
      assert.equal(field(state.dialog, "Program load and attach"), "Not tested");
    }
    submit({ ...report, no_new_privileges: false });
    assert.equal(field(state.dialog, "No-new-privileges"), "Not set");
    submit({ ...report, no_new_privileges: null });
    assert.equal(field(state.dialog, "No-new-privileges"), "Unavailable");
    assert.equal(field(state.dialog, "Restriction context"), "Backend main-thread status; sensor permissions untested");
    for (const change of [{ no_new_privileges: 0 }, { no_new_privileges: "false" }, { no_new_privileges: undefined },
      { seccomp: 2 }, { seccomp: null }, { seccomp: "allowed" }, { seccomp: undefined }, { schema_version: 4 }]) {
      submit(report);
      submit({ ...report, ...change });
      assert.equal(field(state.dialog, "Check status"), "Invalid prerequisite report");
      assert.equal(field(state.dialog, "Seccomp mode"), undefined);
      assert.equal(field(state.dialog, "No-new-privileges"), undefined);
    }
    submit({ ...report, schema_version: 2 });
    assert.equal(field(state.dialog, "Seccomp mode"), "Unavailable");
    assert.equal(field(state.dialog, "No-new-privileges"), "Unavailable");
    submit(readinessReport());
    assert.equal(field(state.dialog, "Seccomp mode"), "Unavailable");
    submit(report);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Seccomp mode"), undefined);
  } finally { state.cleanup(); }
});

test("kernel permission reports render bounded facts and preserve v1 compatibility", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  try {
    state.review.update(data()); state.review.open();
    const report = { ...readinessReport(), schema_version: 2, perf_event_paranoid: -1,
      effective_capabilities: { bpf: true, perfmon: false, sys_admin: false }, sched_process_exec: "unreadable", lockdown: "integrity" };
    const submit = value => {
      state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
      state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null, report: value });
    };
    submit(report);
    for (const [label, expected] of [["Perf event policy", "-1"], ["CAP_BPF", "Effective"], ["CAP_PERFMON", "Not effective"],
      ["CAP_SYS_ADMIN", "Not effective"], ["Process-exec tracepoint", "Unreadable"], ["Kernel lockdown", "Integrity"],
      ["eBPF sensor", "Not started"], ["Program load and attach", "Not tested"]]) assert.equal(field(state.dialog, label), expected);
    for (const change of [{ perf_event_paranoid: "-1" }, { perf_event_paranoid: 2147483648 }, { perf_event_paranoid: 0.5 },
      { effective_capabilities: { bpf: "true", perfmon: false, sys_admin: false } }, { effective_capabilities: [] },
      { sched_process_exec: "ready" }, { lockdown: "<script>" }, { sensor_running: true }]) {
      submit({ ...report, ...change });
      assert.equal(field(state.dialog, "Check status"), "Invalid prerequisite report");
      assert.equal(field(state.dialog, "CAP_BPF"), undefined);
    }
    submit({ ...report, perf_event_paranoid: null, effective_capabilities: null, lockdown: "unknown" });
    assert.equal(field(state.dialog, "CAP_BPF"), "Unavailable");
    assert.equal(field(state.dialog, "Perf event policy"), "Unavailable");
    submit(readinessReport());
    assert.equal(field(state.dialog, "CAP_BPF"), "Unavailable");
    assert.equal(field(state.dialog, "Process-exec tracepoint"), "Unavailable");
    assert.equal(field(state.dialog, "Kernel BTF metadata"), "Present");
  } finally { state.cleanup(); }
});

test("shared producer replay keeps failures visible and rejects invalid or active claims", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const producer = { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
    capacity: 2, attempts: 5, submitted: 3, dropped: 2,
    cases: ["filter_before_reservation", "submitted_identity", "capacity_rejection", "full_ring_filter", "recovery_after_drain", "drop_agreement"]
      .map(name => ({ name, passed: true })) };
  const replay = { schema_version: 1, source: "generated_replay", sensor_running: false,
    health: { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 } };
  const submit = value => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_replay: { ...replay, producer: value } });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(producer);
    for (const [label, expected] of [["Producer test source", "Shared kernel producer; generated two-slot sink"],
      ["Producer checks", "6 / 6 passed"], ["Failed producer checks", "None"], ["Synthetic submission attempts", "5"],
      ["Synthetic submitted records", "3"], ["Synthetic drop notifications", "2"], ["Synthetic ring drops", "3"]]) {
      assert.equal(field(state.dialog, label), expected);
    }
    submit({ ...producer, dropped: 1, cases: producer.cases.map(item => ({ ...item, passed: item.name !== "drop_agreement" })) });
    assert.equal(field(state.dialog, "Producer checks"), "5 / 6 passed");
    assert.equal(field(state.dialog, "Failed producer checks"), "drop agreement");
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { sensor_running: true },
      { load_test_performed: true }, { capacity: 3 }, { attempts: -1 }, { submitted: 10 }, { dropped: "2" },
      { cases: producer.cases.slice(1) }, { cases: [...producer.cases].reverse() },
      { cases: producer.cases.map(item => ({ ...item, passed: 1 })) }]) {
      submit({ ...producer, ...change });
      assert.equal(field(state.dialog, "Producer checks"), undefined);
      assert.equal(field(state.dialog, "Replay source"), undefined);
    }
    submit(null);
    assert.equal(field(state.dialog, "Producer checks"), undefined);
    submit(undefined);
    assert.equal(field(state.dialog, "Replay source"), "Generated input; not kernel capture");
    assert.equal(field(state.dialog, "Producer checks"), undefined);
    submit(producer);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Producer checks"), undefined);
  } finally { state.cleanup(); }
});

const runningAcoustic = () => ({ schema_version: 1, source: "mixed_output", status: "receiving", frames: 2,
  discontinuities: 0, missing_messages: 0, last_receipt_ns: 40000000,
  latest: { source: "mixed_output", frame_start_ns: 20000000, rms: 0.5, peak: 0.8, clipped_sample_ratio: 0, discontinuity: false } });

test("deferred acoustic start requires confirmation and waits for backend capture status", () => {
  const sent = [];
  const state = fixture({ startAcoustic: request => sent.push(request) });
  const update = (extra = {}) => state.review.update(data({ acoustic: null, acoustic_start_available: true, ...extra }));
  try {
    update(); state.review.open();
    const button = () => state.dialog.querySelector('[aria-label="Start output analysis"]');
    assert.equal(field(state.dialog, "Acoustic status"), "Not started");
    assert.match(field(state.dialog, "Acoustic source"), /all applications/);
    for (const acoustic_start_available of [undefined, false, 1, "true"]) {
      update({ acoustic_start_available });
      assert.equal(button().hidden, true);
    }
    update(); button().click();
    assert.equal(sent.length, 0);
    const confirmation = state.dialog.querySelector('[aria-label="Confirm output analysis start"]');
    assert.equal(confirmation.hidden, false);
    assert.match(confirmation.textContent, /all applications/);
    confirmation.querySelector("button").click();
    assert.equal(confirmation.hidden, true);
    button().click(); confirmation.querySelector("button:last-child").click();
    assert.deepEqual(sent, [{ session_id: "session:demo" }]);
    assert.equal(button().disabled, true);
    assert.equal(field(state.dialog, "Acoustic start request"), "Start requested; awaiting capture status.");
    state.dialog.querySelector('[aria-label="Close Safety & Intelligence"]').click(); state.review.open();
    update({ acoustic_start_available: false, acoustic_stop_available: true });
    assert.equal(field(state.dialog, "Acoustic start request"), "Start requested; awaiting capture status.");
    update({ acoustic: { ...runningAcoustic(), status: "starting", frames: 0, latest: null },
      acoustic_start_available: false, acoustic_stop_available: true });
    assert.equal(field(state.dialog, "Acoustic start request"), "Capture start acknowledged by backend status.");
    assert.equal(button().hidden, true);
    update({ session_id: "replacement", acoustic: null, acoustic_start_available: false });
    assert.equal(field(state.dialog, "Acoustic start request"), undefined);
  } finally { state.cleanup(); }
});

test("deferred acoustic start failures and timeout never imply capture or retry automatically", context => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  let requests = 0;
  const state = fixture({ startAcoustic: () => { requests += 1; } });
  try {
    state.review.update(data({ acoustic: null, acoustic_start_available: true })); state.review.open();
    state.dialog.querySelector('[aria-label="Start output analysis"]').click();
    state.dialog.querySelector('[aria-label="Confirm output analysis start"] button:last-child').click();
    context.mock.timers.tick(5000);
    assert.equal(field(state.dialog, "Acoustic start request"), "Start unconfirmed; no automatic retry.");
    assert.equal(field(state.dialog, "Acoustic status"), "Not started");
    assert.equal(requests, 1);
  } finally { state.cleanup(); }
  const failed = fixture({ startAcoustic: () => { throw new Error("offline"); } });
  try {
    failed.review.update(data({ acoustic: null, acoustic_start_available: true })); failed.review.open();
    failed.dialog.querySelector('[aria-label="Start output analysis"]').click();
    failed.dialog.querySelector('[aria-label="Confirm output analysis start"] button:last-child').click();
    assert.equal(field(failed.dialog, "Acoustic start request"), "Start request failed; output analysis was not confirmed.");
    context.mock.timers.tick(5000);
    assert.equal(field(failed.dialog, "Acoustic status"), "Not started");
  } finally { failed.cleanup(); }
});

test("acoustic stop is capability gated and waits for terminal backend state without detaching", () => {
  const sent = [];
  const state = fixture({ stopAcoustic: request => sent.push(request) });
  const update = (extra = {}) => state.review.update(data({ acoustic: runningAcoustic(), acoustic_stop_available: true, ...extra }));
  try {
    update(); state.review.open();
    const button = () => state.dialog.querySelector('[aria-label="Stop output analysis"]');
    assert.ok(button().querySelector("svg"));
    for (const acoustic_stop_available of [undefined, false, 1, "true"]) {
      update({ acoustic_stop_available });
      assert.equal(button().hidden, true);
      button().click();
    }
    update({ acoustic: { ...runningAcoustic(), source: "generated_pcm", latest: { ...runningAcoustic().latest, source: "generated_pcm" } } });
    assert.equal(button().hidden, true);
    update(); button().click(); button().click();
    assert.deepEqual(sent, [{ session_id: "session:demo" }]);
    assert.equal(button().disabled, true);
    assert.equal(field(state.dialog, "Acoustic stop request"), "Stop requested; awaiting capture cleanup.");
    state.dialog.querySelector('[aria-label="Close Safety & Intelligence"]').click(); state.review.open();
    assert.equal(button().disabled, true);
    update({ acoustic_stop_available: false });
    assert.equal(field(state.dialog, "Acoustic status"), "Frames received; last update only");
    assert.equal(field(state.dialog, "Acoustic stop request"), "Stop requested; awaiting capture cleanup.");
    update({ acoustic: { ...runningAcoustic(), status: "stopped" }, acoustic_stop_available: false });
    assert.equal(field(state.dialog, "Acoustic stop request"), "Capture stopped.");
    assert.equal(button().hidden, true);
    update({ session_id: "replacement", acoustic_stop_available: false });
    assert.equal(field(state.dialog, "Acoustic stop request"), undefined);
    assert.equal(sent.length, 1);
  } finally { state.cleanup(); }
});

test("acoustic stop timeout and send failure never imply cleanup or retry automatically", context => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  let requests = 0;
  const state = fixture({ stopAcoustic: () => { requests += 1; } });
  try {
    state.review.update(data({ acoustic: runningAcoustic(), acoustic_stop_available: true })); state.review.open();
    state.dialog.querySelector('[aria-label="Stop output analysis"]').click();
    context.mock.timers.tick(5000);
    assert.equal(field(state.dialog, "Acoustic stop request"), "Stop unconfirmed; no automatic retry.");
    assert.equal(field(state.dialog, "Acoustic status"), "Frames received; last update only");
    assert.equal(requests, 1);
    state.review.update(data({ acoustic: { ...runningAcoustic(), status: "faulted" }, acoustic_stop_available: false }));
    assert.equal(field(state.dialog, "Acoustic stop request"), "Capture fault; cleanup may be unconfirmed.");
  } finally { state.cleanup(); }
  const failed = fixture({ stopAcoustic: () => { throw new Error("offline"); } });
  try {
    failed.review.update(data({ acoustic: runningAcoustic(), acoustic_stop_available: true })); failed.review.open();
    failed.dialog.querySelector('[aria-label="Stop output analysis"]').click();
    assert.equal(field(failed.dialog, "Acoustic stop request"), "Stop request failed; capture may still be running.");
    context.mock.timers.tick(5000);
    assert.equal(field(failed.dialog, "Acoustic status"), "Frames received; last update only");
  } finally { failed.cleanup(); }
});

test("acoustic session levels stay unattributed and clear when session data is absent or invalid", () => {
  const state = fixture();
  const acoustic = { schema_version: 1, source: "generated_pcm", status: "receiving", frames: 2,
    discontinuities: 0, missing_messages: 1, last_receipt_ns: 40000000,
    latest: { source: "generated_pcm", frame_start_ns: 20000000, rms: 0.5, peak: 0.8, clipped_sample_ratio: 0.1, discontinuity: false } };
  try {
    state.review.update(data({ acoustic })); state.review.open();
    assert.equal(field(state.dialog, "Acoustic source"), "Generated PCM; not live capture");
    assert.equal(field(state.dialog, "Last acoustic RMS"), "0.500");
    assert.equal(field(state.dialog, "Acoustic missing messages"), "1");
    assert.equal(field(state.dialog, "Acoustic liveness"), "Not established");
    assert.equal(field(state.dialog, "Acoustic attribution"), "Unattributed; not participant evidence");
    acoustic.latest.rms = 0.1;
    state.dialog.querySelector('[aria-label="Close Safety & Intelligence"]').click(); state.review.open();
    assert.equal(field(state.dialog, "Last acoustic RMS"), "0.500");
    state.review.update(data({ acoustic: { ...acoustic, status: "stopped" }, reset_count: 1 }));
    assert.equal(field(state.dialog, "Acoustic status"), "Stopped");
    for (const invalid of [null, undefined, { ...acoustic, user_id: userId }, { ...acoustic, status: "live" }]) {
      state.review.update(data({ acoustic: invalid }));
      assert.equal(field(state.dialog, "Acoustic status"), undefined);
    }
    state.review.update(data({ acoustic }));
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Last acoustic RMS"), undefined);
  } finally { state.cleanup(); }
});

test("latest derived acoustic analysis remains summary-only and unclassified", () => {
  const state = fixture();
  const acoustic = { ...runningAcoustic(), schema_version: 2,
    latest: { ...runningAcoustic().latest, analysis: { centroid_hz: 1200.5, high_band_power_ratio: 0.2,
      positive_power_flux: 0.3, compared_lags: 4, best_lag_ns: 160000000, best_similarity: 0.8 } } };
  try {
    state.review.update(data({ acoustic })); state.review.open();
    assert.equal(field(state.dialog, "Derived analysis scope"), "Latest frame only; not a classifier or participant evidence");
    assert.equal(field(state.dialog, "Latest spectral centroid"), "1200.5 Hz");
    assert.equal(field(state.dialog, "Latest high-band power ratio"), "0.200");
    assert.equal(field(state.dialog, "Latest positive spectral flux"), "0.300");
    assert.equal(field(state.dialog, "Compared repetition lags"), "4");
    assert.equal(field(state.dialog, "Best coarse repetition lag"), "160 ms");
    assert.equal(field(state.dialog, "Best coarse repetition similarity"), "0.800");
    acoustic.latest.analysis.best_similarity = 0.1;
    state.dialog.querySelector('[aria-label="Close Safety & Intelligence"]').click(); state.review.open();
    assert.equal(field(state.dialog, "Best coarse repetition similarity"), "0.800");
  } finally { state.cleanup(); }
});

test("session owner updates arrive without diagnostics and stay independent of explicit checks", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const pending = { schema_version: 1, source: "capture_owner_outcome", status: "awaiting_completion", result: null };
  const finished = { ...pending, status: "finished",
    result: { outcome: "collector_fault", collector_started: true, collector_reaped: true, endpoint_removed: false } };
  try {
    state.review.update(data({ process_owner: pending })); state.review.open();
    assert.equal(request, undefined);
    assert.equal(field(state.dialog, "Session owner status"), "Awaiting completion; activity unverified");
    assert.equal(field(state.dialog, "Owner result status"), undefined);
    state.review.update(data({ process_owner: finished }));
    finished.result.endpoint_removed = true;
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_owner: { ...pending, status: "not_configured" } });
    assert.equal(field(state.dialog, "Session owner status"), "Finished");
    assert.equal(field(state.dialog, "Session owner outcome"), "collector fault");
    assert.equal(field(state.dialog, "Session child reaped"), "Confirmed");
    assert.equal(field(state.dialog, "Session endpoint removed"), "Not confirmed");
    assert.equal(field(state.dialog, "Session sensor liveness"), "Not established");
    assert.equal(field(state.dialog, "Owner result status"), "Not configured");
    assert.equal(field(state.dialog, "Recent accepted records"), undefined);
    state.dialog.querySelector('[aria-label="Close Safety & Intelligence"]').click(); state.review.open();
    assert.equal(field(state.dialog, "Session owner status"), "Finished");
    for (const invalid of [undefined, null, { ...pending, sensor_running: true }, { ...pending, source: "generated_replay" }]) {
      state.review.update(data({ process_owner: invalid }));
      assert.equal(field(state.dialog, "Session owner status"), undefined);
    }
    state.review.update(data({ process_owner: pending }));
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Session owner status"), undefined);
    state.review.update(data({ schema_version: 99, process_owner: pending }));
    assert.equal(field(state.dialog, "Session owner status"), undefined);
  } finally { state.cleanup(); }
});

test("owner outcomes show cleanup independently and clear outside matched host checks", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const outcome = { schema_version: 1, source: "capture_owner_outcome", status: "finished",
    result: { outcome: "collector_fault", collector_started: true, collector_reaped: true, endpoint_removed: false } };
  const submit = (process_owner, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_owner, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(outcome);
    assert.equal(field(state.dialog, "Owner outcome"), "collector fault");
    assert.equal(field(state.dialog, "Owner child reaped"), "Confirmed");
    assert.equal(field(state.dialog, "Owner endpoint removed"), "Not confirmed");
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    assert.equal(field(state.dialog, "Recent accepted records"), undefined);
    for (const status of ["not_configured", "awaiting_completion", "unavailable"]) {
      submit({ ...outcome, status, result: null });
      assert.equal(field(state.dialog, "Owner outcome"), undefined);
      assert.ok(field(state.dialog, "Owner result status"));
    }
    for (const invalid of [undefined, null, { ...outcome, sensor_running: true },
      { ...outcome, result: { ...outcome.result, collector_reaped: false } }]) {
      submit(outcome); submit(invalid);
      assert.equal(field(state.dialog, "Owner result status"), undefined);
    }
    submit(outcome, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Owner result status"), undefined);
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, request_id: request.request_id + 1,
      source: "host", error: null, report: readinessReport(), process_owner: outcome });
    assert.equal(field(state.dialog, "Owner result status"), undefined);
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_owner: outcome });
    assert.equal(field(state.dialog, "Owner outcome"), "collector fault");
    state.dialog.querySelector('[aria-label="Close Safety & Intelligence"]').click(); state.review.open();
    assert.equal(field(state.dialog, "Owner result status"), undefined);
    submit(outcome);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Owner result status"), undefined);
  } finally { state.cleanup(); }
});

const progressSnapshot = () => ({ schema_version: 1, source: "owner_progress_subscription", status: "recent_kernel_sample",
  sensor_running: false, valid_for_ms: 2000,
  sample: { schema_version: 1, accepted: 2, rejected_record: 0, rejected_clock: 0, ring_dropped: 3 } });

test("automatic session progress arrives without diagnostics and expires on its remaining budget", context => {
  let now = 1000;
  context.mock.method(performance, "now", () => now);
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const state = fixture();
  try {
    state.review.update(data({ process_progress: { ...progressSnapshot(), valid_for_ms: 100 } }));
    state.review.open();
    assert.equal(field(state.dialog, "Session progress status"), "Recent kernel sample");
    assert.equal(field(state.dialog, "Session progress liveness"), "Not established");
    assert.equal(field(state.dialog, "Session accepted records"), "2");
    state.dialog.querySelector('[aria-label="Close Safety & Intelligence"]').click();
    state.review.open();
    assert.equal(field(state.dialog, "Session accepted records"), "2");
    now = 1100;
    context.mock.timers.tick(100);
    assert.equal(field(state.dialog, "Session progress status"), "No recent sample");
    assert.equal(field(state.dialog, "Session accepted records"), undefined);
    state.review.update(data({ process_progress: { ...progressSnapshot(), source: "generated_replay" } }));
    assert.equal(field(state.dialog, "Session progress status"), undefined);
    state.review.update(data({ process_progress: progressSnapshot() }));
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Session progress status"), undefined);
  } finally { state.cleanup(); }
});

test("owner progress expires locally and never changes historical or sensor liveness claims", context => {
  let now = 1000, request;
  context.mock.method(performance, "now", () => now);
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  try {
    state.review.update(data()); state.review.open();
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    now = 1050;
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_progress: progressSnapshot(),
      process_health_snapshot: { schema_version: 1, source: "operator_supplied_snapshot", status: "not_configured", sensor_running: false, report: null } });
    assert.equal(field(state.dialog, "Progress status"), "Recent kernel sample");
    assert.equal(field(state.dialog, "Progress liveness"), "Not established");
    assert.equal(field(state.dialog, "Recent accepted records"), "2");
    assert.equal(field(state.dialog, "Snapshot status"), "Not configured");
    assert.equal(field(state.dialog, "Reported accepted records"), undefined);
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    now = 3000;
    context.mock.timers.tick(1950);
    assert.equal(field(state.dialog, "Progress status"), "No recent sample");
    assert.equal(field(state.dialog, "Recent accepted records"), undefined);
    assert.equal(field(state.dialog, "Snapshot status"), "Not configured");
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
  } finally { state.cleanup(); }
});

test("owner progress requires matched host responses and clears on malformed input, close and session replacement", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const submit = (snapshot, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_progress: snapshot, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    for (const change of [{ source: "generated_replay" }, { sensor_running: true }, { valid_for_ms: 2001 },
      { sample: { ...progressSnapshot().sample, accepted: -1 } }, { extra: "private" }]) {
      submit(progressSnapshot());
      assert.equal(field(state.dialog, "Recent accepted records"), "2");
      submit({ ...progressSnapshot(), ...change });
      assert.equal(field(state.dialog, "Progress status"), undefined);
      assert.equal(field(state.dialog, "Recent accepted records"), undefined);
    }
    for (const [status, label] of [["not_connected", "Not connected"], ["no_recent_sample", "No recent sample"],
      ["generated_rejected", "Generated input rejected"], ["invalid_sample", "Invalid sample rejected"]]) {
      submit({ ...progressSnapshot(), status, valid_for_ms: 0, sample: null });
      assert.equal(field(state.dialog, "Progress status"), label);
      assert.equal(field(state.dialog, "Recent accepted records"), undefined);
    }
    submit(progressSnapshot(), { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Progress status"), undefined);
    submit(progressSnapshot(), { request_id: "foreign" });
    assert.equal(field(state.dialog, "Progress status"), undefined);
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_progress: progressSnapshot() });
    assert.equal(field(state.dialog, "Recent accepted records"), "2");
    state.key("Escape"); state.review.open();
    assert.equal(field(state.dialog, "Recent accepted records"), undefined);
    submit(progressSnapshot());
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Recent accepted records"), undefined);
  } finally { state.cleanup(); }
});

test("supplied health snapshots remain historical and reject malformed or active reports", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const snapshot = { schema_version: 1, source: "operator_supplied_snapshot", status: "supplied", sensor_running: false,
    report: { schema_version: 1, source: "supervised_kernel_capture", sensor_running: false,
      run: { schema_version: 1, outcome: "completed", collector_started: true, collector_reaped: true,
        status: { schema_version: 1, source: "kernel_tracepoint", records: 6, terminal_phase: "stopped", reason: "duration_limit",
          attachment_confirmed: true, detachment_confirmed: true },
        terminal_health: { schema_version: 1, accepted: 2, rejected_record: 0, rejected_clock: 0, ring_dropped: 3 } } } };
  const submit = (value, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_health_snapshot: value, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(snapshot);
    assert.equal(field(state.dialog, "Snapshot provenance"), "Operator-supplied file; not authenticated or live");
    assert.equal(field(state.dialog, "Snapshot freshness"), "Unknown; no automatic refresh or liveness claim");
    assert.equal(field(state.dialog, "Reported accepted records"), "2");
    assert.equal(field(state.dialog, "Reported ring drops"), "3");
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    for (const [keys, value] of [
      [["sensor_running"], true], [["source"], "host"], [["report", "sensor_running"], true],
      [["report", "run", "collector_reaped"], false], [["report", "run", "outcome"], "deadline"],
      [["report", "run", "status", "source"], "generated_replay"],
      [["report", "run", "status", "detachment_confirmed"], false],
      [["report", "run", "terminal_health"], null],
      [["report", "run", "terminal_health", "accepted"], Number.MAX_SAFE_INTEGER + 1],
      [["report", "run", "terminal_health", "ring_dropped"], -1],
      [["report", "run", "terminal_health", "path"], "private"],
    ]) {
      const invalid = structuredClone(snapshot);
      let target = invalid;
      for (const key of keys.slice(0, -1)) target = target[key];
      target[keys.at(-1)] = value;
      submit(invalid);
      assert.equal(field(state.dialog, "Reported accepted records"), undefined);
      assert.equal(field(state.dialog, "Snapshot provenance"), undefined);
    }
    for (const [status, label] of [
      ["not_configured", "Not configured"], ["rejected", "Unreadable or invalid report"],
      ["unavailable", "Unavailable in this build"],
      ["inspection_timed_out", "Report inspection timed out; helper reaped"],
      ["inspection_failed", "Report inspection failed"],
      ["cleanup_unconfirmed", "Report inspection cleanup unconfirmed"],
    ]) {
      submit(snapshot);
      submit({ ...snapshot, status, report: null });
      assert.equal(field(state.dialog, "Snapshot status"), label);
      assert.equal(field(state.dialog, "Reported accepted records"), undefined);
      assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
      submit({ ...snapshot, status });
      assert.equal(field(state.dialog, "Snapshot status"), undefined);
      submit({ ...snapshot, status, report: null, sensor_running: true });
      assert.equal(field(state.dialog, "Snapshot status"), undefined);
    }
    submit({ ...snapshot, status: "unknown_failure", report: null });
    assert.equal(field(state.dialog, "Snapshot status"), undefined);
    submit(undefined);
    assert.equal(field(state.dialog, "Snapshot status"), undefined);
    submit(snapshot, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(snapshot, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Reported accepted records"), undefined);
    submit(snapshot);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Reported accepted records"), undefined);
  } finally { state.cleanup(); }
});

test("progress protocol diagnostics are generated, optional and session scoped", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const progress = { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
    checks: ["fragmented_delivery", "foreign_session", "replayed_sequence", "counter_regression", "before_observation", "freshness_bounds", "truncated_record", "exit_mismatch"]
      .map(name => ({ name, passed: true })) };
  const submit = (value, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_replay: { schema_version: 1, source: "generated_replay", sensor_running: false,
        health: { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 }, progress: value }, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(progress);
    assert.equal(field(state.dialog, "Progress protocol checks"), "8 / 8 passed");
    assert.equal(field(state.dialog, "Progress test source"), "Generated session, records and receipt clocks");
    assert.equal(field(state.dialog, "Live health delivery"), "Not connected; authentication and liveness unverified");
    submit({ ...progress, checks: progress.checks.map(item => ({ ...item, passed: item.name !== "foreign_session" })) });
    assert.equal(field(state.dialog, "Progress protocol checks"), "7 / 8 passed");
    assert.equal(field(state.dialog, "Failed progress checks"), "foreign session");
    submit({ ...progress, checks: progress.checks.map(item => ({ ...item, passed: false })) });
    assert.equal(field(state.dialog, "Progress protocol checks"), "0 / 8 passed");
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { sensor_running: true },
      { load_test_performed: true }, { session: "private" }, { checks: null }, { checks: progress.checks.slice(1) },
      { checks: [...progress.checks].reverse() }, { checks: progress.checks.map(item => ({ ...item, passed: 1 })) },
      { checks: progress.checks.map(item => ({ ...item, active: false })) }]) {
      submit({ ...progress, ...change });
      assert.equal(field(state.dialog, "Progress protocol checks"), undefined);
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Replay source"), "Generated input; not kernel capture");
    submit(progress, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(progress, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Progress protocol checks"), undefined);
    submit(progress);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Progress protocol checks"), undefined);
  } finally { state.cleanup(); }
});

test("controlled delivery diagnostics reject live claims and retain failed checks", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const delivery = { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
    checks: ["matching_exec", "foreign_process", "worker_thread", "before_release", "after_receipt", "malformed_record", "backwards_bracket"]
      .map(name => ({ name, passed: true })) };
  const submit = (value, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_replay: { schema_version: 1, source: "generated_replay", sensor_running: false,
        health: { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 }, delivery: value }, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(delivery);
    assert.equal(field(state.dialog, "Delivery record checks"), "7 / 7 passed");
    assert.equal(field(state.dialog, "Delivery test source"), "Shared record validator; generated bytes and clocks");
    assert.equal(field(state.dialog, "Live event delivery"), "Not tested by this diagnostic");
    submit({ ...delivery, checks: delivery.checks.map(item => ({ ...item, passed: item.name !== "foreign_process" })) });
    assert.equal(field(state.dialog, "Delivery record checks"), "6 / 7 passed");
    assert.equal(field(state.dialog, "Failed delivery checks"), "foreign process");
    submit({ ...delivery, checks: delivery.checks.map(item => ({ ...item, passed: false })) });
    assert.equal(field(state.dialog, "Delivery record checks"), "0 / 7 passed");
    for (const change of [{ schema_version: 2 }, { source: "explicit_kernel_delivery_probe" }, { sensor_running: true },
      { load_test_performed: true }, { event_verified: true }, { checks: null }, { checks: delivery.checks.slice(1) },
      { checks: [...delivery.checks].reverse() }, { checks: delivery.checks.map(item => ({ ...item, passed: 1 })) },
      { checks: delivery.checks.map(item => ({ ...item, event_verified: true })) }]) {
      submit({ ...delivery, ...change });
      assert.equal(field(state.dialog, "Delivery record checks"), undefined);
    }
    submit(null);
    assert.equal(field(state.dialog, "Replay source"), undefined);
    submit(undefined);
    assert.equal(field(state.dialog, "Replay source"), "Generated input; not kernel capture");
    submit(delivery, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(delivery, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Delivery record checks"), undefined);
    submit(delivery);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Delivery record checks"), undefined);
  } finally { state.cleanup(); }
});

test("attach-only lifecycle diagnostics stay generated and preserve failures", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const attachment = { schema_version: 1, source: "generated_replay", sensor_running: false,
    load_test_performed: false, attachment_attempted: false,
    checks: ["setup_failure_stops", "load_failure_stops", "disabled_target_required", "attach_failure_stops", "detach_failure_visible", "successful_cleanup"]
      .map(name => ({ name, passed: true })) };
  const submit = (value, extra = {}) => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_replay: { schema_version: 1, source: "generated_replay", sensor_running: false,
        health: { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 }, attachment: value }, ...extra });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(attachment);
    assert.equal(field(state.dialog, "Attachment lifecycle checks"), "6 / 6 passed");
    assert.equal(field(state.dialog, "Attachment test source"), "Shared probe controller; generated operations only");
    assert.equal(field(state.dialog, "Live tracepoint attachment"), "Not tested by this diagnostic");
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    submit({ ...attachment, checks: attachment.checks.map(item => ({ ...item, passed: item.name !== "detach_failure_visible" })) });
    assert.equal(field(state.dialog, "Attachment lifecycle checks"), "5 / 6 passed");
    assert.equal(field(state.dialog, "Failed attachment checks"), "detach failure visible");
    submit({ ...attachment, checks: attachment.checks.map(item => ({ ...item, passed: false })) });
    assert.equal(field(state.dialog, "Attachment lifecycle checks"), "0 / 6 passed");
    for (const change of [{ schema_version: 2 }, { source: "explicit_kernel_attach_probe" }, { sensor_running: true },
      { load_test_performed: true }, { attachment_attempted: true }, { program_loaded: true },
      { checks: null }, { checks: attachment.checks.slice(1) }, { checks: [...attachment.checks].reverse() },
      { checks: attachment.checks.map(item => ({ ...item, passed: 1 })) },
      { checks: attachment.checks.map(item => ({ ...item, attachment_confirmed: true })) }]) {
      submit({ ...attachment, ...change });
      assert.equal(field(state.dialog, "Attachment lifecycle checks"), undefined);
      assert.equal(field(state.dialog, "Replay source"), undefined);
    }
    submit(null);
    assert.equal(field(state.dialog, "Attachment lifecycle checks"), undefined);
    submit(undefined);
    assert.equal(field(state.dialog, "Replay source"), "Generated input; not kernel capture");
    submit(attachment, { source: "preview_fixture" });
    assert.equal(field(state.dialog, "Check status"), "Preview fixture; not a host check");
    submit(attachment, { error: "rate_limited", report: null });
    assert.equal(field(state.dialog, "Attachment lifecycle checks"), undefined);
    submit(attachment);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Attachment lifecycle checks"), undefined);
  } finally { state.cleanup(); }
});

test("record contract diagnostics reject malformed provenance and preserve failed checks", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const layout = { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
    checks: ["native_layout", "full_width_fields", "unaligned_decode", "invalid_record_rejection"].map(name => ({ name, passed: true })) };
  const submit = record_layout => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_replay: { schema_version: 1, source: "generated_replay", sensor_running: false,
        health: { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 }, record_layout } });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(layout);
    assert.equal(field(state.dialog, "Record contract source"), "Shared kernel constructor; generated native bytes");
    assert.equal(field(state.dialog, "Record contract checks"), "4 / 4 passed");
    assert.equal(field(state.dialog, "Failed record checks"), "None");
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    submit({ ...layout, checks: layout.checks.map(item => ({ ...item, passed: item.name !== "native_layout" })) });
    assert.equal(field(state.dialog, "Record contract checks"), "3 / 4 passed");
    assert.equal(field(state.dialog, "Failed record checks"), "native layout");
    submit({ ...layout, checks: layout.checks.map(item => ({ ...item, passed: false })) });
    assert.equal(field(state.dialog, "Record contract checks"), "0 / 4 passed");
    assert.equal(field(state.dialog, "Failed record checks"), "native layout, full width fields, unaligned decode, invalid record rejection");
    for (const change of [{ schema_version: 2 }, { source: "kernel_tracepoint" }, { sensor_running: true },
      { load_test_performed: true }, { checks: null }, { checks: layout.checks.slice(1) },
      { checks: [...layout.checks].reverse() }, { checks: [...layout.checks, layout.checks[0]] },
      { checks: layout.checks.map(item => ({ ...item, passed: 1 })) },
      { checks: layout.checks.map(item => ({ ...item, name: "unknown" })) }]) {
      submit({ ...layout, ...change });
      assert.equal(field(state.dialog, "Record contract checks"), undefined);
      assert.equal(field(state.dialog, "Replay source"), undefined);
    }
    submit(null);
    assert.equal(field(state.dialog, "Replay source"), undefined);
    submit(undefined);
    assert.equal(field(state.dialog, "Replay source"), "Generated input; not kernel capture");
    assert.equal(field(state.dialog, "Record contract checks"), undefined);
    submit(layout);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Record contract checks"), undefined);
  } finally { state.cleanup(); }
});

test("reservation diagnostics show failures, reject malformed checks and accept older producers", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const reservation = ["filtered_reservation", "reserve_before_clock", "single_submission", "native_record_delivery", "full_ring_preservation", "fresh_recovery"]
    .map(name => ({ name, passed: true }));
  const producer = { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
    capacity: 2, attempts: 5, submitted: 3, dropped: 2,
    cases: ["filter_before_reservation", "submitted_identity", "capacity_rejection", "full_ring_filter", "recovery_after_drain", "drop_agreement"]
      .map(name => ({ name, passed: true })) };
  const submit = checks => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_replay: { schema_version: 1, source: "generated_replay", sensor_running: false,
        health: { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 },
        producer: { ...producer, reservation: checks } } });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(reservation);
    assert.equal(field(state.dialog, "Reservation test source"), "Shared kernel submission; generated slots and clock");
    assert.equal(field(state.dialog, "Reservation checks"), "6 / 6 passed");
    assert.equal(field(state.dialog, "Failed reservation checks"), "None");
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    submit(reservation.map(item => ({ ...item, passed: item.name !== "reserve_before_clock" })));
    assert.equal(field(state.dialog, "Reservation checks"), "5 / 6 passed");
    assert.equal(field(state.dialog, "Failed reservation checks"), "reserve before clock");
    submit(reservation.map(item => ({ ...item, passed: false })));
    assert.equal(field(state.dialog, "Reservation checks"), "0 / 6 passed");
    for (const invalid of [null, {}, [], reservation.slice(1), [...reservation].reverse(), [...reservation, reservation[0]],
      reservation.map((item, index) => index ? item : { name: "unknown", passed: true }),
      reservation.map((item, index) => index ? item : { ...item, passed: 1 })]) {
      submit(invalid);
      assert.equal(field(state.dialog, "Reservation checks"), undefined);
      assert.equal(field(state.dialog, "Producer checks"), undefined);
      assert.equal(field(state.dialog, "Replay source"), undefined);
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Producer checks"), "6 / 6 passed");
    assert.equal(field(state.dialog, "Reservation checks"), undefined);
    submit(reservation);
    state.key("Escape"); state.review.open();
    assert.equal(field(state.dialog, "Reservation checks"), "6 / 6 passed");
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Reservation checks"), undefined);
  } finally { state.cleanup(); }
});

test("loss accounting diagnostics bound check names, show failures and accept older producers", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const accounting = ["counter_saturation", "per_cpu_sum", "aggregate_saturation", "snapshot_high_water", "saturated_filter", "saturated_recovery"]
    .map(name => ({ name, passed: true }));
  const producer = { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
    capacity: 2, attempts: 5, submitted: 3, dropped: 2,
    cases: ["filter_before_reservation", "submitted_identity", "capacity_rejection", "full_ring_filter", "recovery_after_drain", "drop_agreement"]
      .map(name => ({ name, passed: true })) };
  const submit = checks => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_replay: { schema_version: 1, source: "generated_replay", sensor_running: false,
        health: { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 },
        producer: { ...producer, accounting: checks } } });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(accounting);
    assert.equal(field(state.dialog, "Loss accounting source"), "Shared counter arithmetic; generated snapshots");
    assert.equal(field(state.dialog, "Loss accounting checks"), "6 / 6 passed");
    assert.equal(field(state.dialog, "Failed accounting checks"), "None");
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    submit(accounting.map(item => ({ ...item, passed: item.name !== "aggregate_saturation" })));
    assert.equal(field(state.dialog, "Loss accounting checks"), "5 / 6 passed");
    assert.equal(field(state.dialog, "Failed accounting checks"), "aggregate saturation");
    submit(accounting.map(item => ({ ...item, passed: false })));
    assert.equal(field(state.dialog, "Loss accounting checks"), "0 / 6 passed");
    assert.equal(field(state.dialog, "Failed accounting checks"), accounting.map(item => item.name.replaceAll("_", " ")).join(", "));
    for (const invalid of [null, {}, [], accounting.slice(1), [...accounting].reverse(), [...accounting, accounting[0]],
      accounting.map((item, index) => index ? item : { name: "unknown", passed: true }),
      accounting.map((item, index) => index ? item : { ...item, passed: 1 })]) {
      submit(invalid);
      assert.equal(field(state.dialog, "Loss accounting checks"), undefined);
      assert.equal(field(state.dialog, "Producer checks"), undefined);
      assert.equal(field(state.dialog, "Replay source"), undefined);
    }
    submit(undefined);
    assert.equal(field(state.dialog, "Producer checks"), "6 / 6 passed");
    assert.equal(field(state.dialog, "Loss accounting checks"), undefined);
    submit(accounting);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Loss accounting checks"), undefined);
  } finally { state.cleanup(); }
});

test("process decoder replay is explicitly synthetic and rejects live or malformed claims", () => {
  let request;
  const state = fixture({ checkDiagnostics: value => { request = value; } });
  const replay = { schema_version: 1, source: "generated_replay", sensor_running: false,
    health: { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 } };
  const submit = process_replay => {
    state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]').click();
    state.review.diagnostics.update({ schema_version: 1, ...request, source: "host", error: null,
      report: readinessReport(), process_replay });
  };
  try {
    state.review.update(data()); state.review.open();
    submit(replay);
    assert.equal(field(state.dialog, "Replay source"), "Generated input; not kernel capture");
    assert.equal(field(state.dialog, "Synthetic accepted events"), "2");
    assert.equal(field(state.dialog, "Synthetic ring drops"), "3");
    assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    assert.equal(field(state.dialog, "Target selection checks"), undefined);
    const selection = ["disabled_target", "foreign_tgid", "matching_leader", "matching_thread", "zero_process", "zero_thread", "full_width_identity"]
      .map(name => ({ name, passed: true }));
    submit({ ...replay, selection });
    assert.equal(field(state.dialog, "Target selection source"), "Shared kernel filter; generated identities");
    assert.equal(field(state.dialog, "Target selection checks"), "7 / 7 passed");
    assert.equal(field(state.dialog, "Failed target selections"), "None");
    submit({ ...replay, selection: selection.map((item, index) => ({ ...item, passed: index !== 3 })) });
    assert.equal(field(state.dialog, "Target selection checks"), "6 / 7 passed");
    assert.equal(field(state.dialog, "Failed target selections"), "matching thread");
    for (const invalid of [null, {}, selection.slice(1), [...selection].reverse(),
      selection.map((item, index) => index ? item : { ...item, passed: 1 })]) {
      submit({ ...replay, selection: invalid });
      assert.equal(field(state.dialog, "Replay source"), undefined);
      assert.equal(field(state.dialog, "Target selection checks"), undefined);
    }
    for (const invalid of [undefined, { ...replay, source: "kernel_tracepoint" },
      { ...replay, sensor_running: true }, { ...replay, schema_version: 2 },
      { ...replay, health: { ...replay.health, accepted: -1 } }]) {
      submit(invalid);
      assert.equal(field(state.dialog, "Replay source"), undefined);
      assert.equal(field(state.dialog, "eBPF sensor"), "Not started");
    }
    submit(replay);
    state.review.update(data({ session_id: "replacement" }));
    assert.equal(field(state.dialog, "Replay source"), undefined);
  } finally { state.cleanup(); }
});

test("prerequisite timeout, close and disposal reject delayed responses without retrying", context => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const requests = [];
  const state = fixture({ checkDiagnostics: request => requests.push(request) });
  const respond = request => state.review.diagnostics.update({ schema_version: 1, ...request,
    source: "host", error: null, report: readinessReport() });
  const check = () => state.dialog.querySelector('[aria-label="Check Linux sensor prerequisites"]');
  try {
    state.review.update(data()); state.review.open();
    check().click();
    context.mock.timers.tick(5000);
    assert.equal(requests.length, 1);
    assert.equal(field(state.dialog, "Check status"), "Check not acknowledged");
    assert.equal(check().disabled, false);
    respond(requests[0]);
    assert.equal(field(state.dialog, "Kernel BTF metadata"), undefined);
    check().click();
    respond(requests[0]);
    assert.equal(field(state.dialog, "Check status"), "Checking");
    state.key("Escape");
    respond(requests[1]);
    state.review.open();
    assert.equal(field(state.dialog, "Check status"), "Check not acknowledged");
    check().click();
    respond(requests[2]);
    assert.equal(field(state.dialog, "Kernel BTF metadata"), "Present");
    check().click();
    state.review.dispose();
    respond(requests[3]);
    context.mock.timers.tick(10000);
    assert.equal(state.dialog === null, true);
    assert.equal(requests.length, 4);
  } finally { state.cleanup(); }
});

test("missing data and schema mismatches are unavailable, never valid zero counts", () => {
  const state = fixture();
  try {
    state.review.open();
    assert.match(state.dialog.textContent, /unavailable: no engine data/);
    assert.equal(field(state.dialog, "Retained observations"), undefined);
    assert.equal(field(state.dialog, "Check status"), "Not checked");
    for (const value of [{}, { schema_version: 2, observations: [], incidents: [] }, { schema_version: "1" }, []]) {
      state.review.update(value);
      assert.match(state.dialog.textContent, /unavailable: missing or unsupported schema/);
      assert.equal(field(state.dialog, "Retained observations"), undefined);
      assert.equal(field(state.dialog, "Check status"), "Not checked");
    }
    state.review.update({ schema_version: 1 });
    assert.equal(field(state.dialog, "Retained observations"), "Unavailable");
    assert.equal(field(state.dialog, "Evicted incidents"), "Unavailable");
    state.review.update(data({ observations: [], incidents: [], dropped_incidents: 0 }));
    assert.equal(field(state.dialog, "Retained observations"), "0");
    assert.equal(field(state.dialog, "Evicted incidents"), "0");
    assert.match(state.dialog.textContent, /No retained incidents/);
  } finally { state.cleanup(); }
});

test("ingestion health distinguishes queue loss, input rejections, evictions and missing voice transitions", () => {
  const state = fixture();
  try {
    state.review.update(data({ ingestion: { queue_capacity: 64, dropped_full: 1, rejected_invalid: 2,
      rejected_closed: 3, accepted: 4, rejected_session: 5, rejected_replay: 6, rejected_clock: 7,
      rejected_state: 8, diagnostics: null } }));
    state.review.open();
    assert.ok([...state.dialog.querySelectorAll("h3")].some(node => node.textContent === "Ingestion health"));
    for (const [label, value] of [["Queue capacity", "64"], ["Dropped queue messages", "1"],
      ["Rejected invalid inputs", "2"], ["Rejected closed inputs", "3"], ["Accepted inputs", "4"],
      ["Rejected stale sessions", "5"], ["Rejected replayed inputs", "6"], ["Rejected clock inputs", "7"],
      ["Rejected state inputs", "8"], ["Evicted observations", "3"], ["Evicted incidents", "2"],
      ["Known missing voice transitions", "5"], ["Diagnostics (future in-process reports)", "Not connected"]]) {
      assert.equal(field(state.dialog, label), value);
    }
    assert.equal(field(state.dialog, "eBPF sensor"), undefined);
    for (const [diagnostics, expected] of [["available", "Available"], ["degraded", "Degraded"], ["unavailable", "Unavailable"]]) {
      state.review.update(data({ ingestion: { diagnostics } }));
      assert.equal(field(state.dialog, "Diagnostics (future in-process reports)"), expected);
      assert.equal(field(state.dialog, "eBPF sensor"), undefined);
    }
  } finally { state.cleanup(); }
});

test("missing and malformed ingestion stays unavailable while explicit zero counts remain valid", () => {
  const state = fixture();
  const metrics = [["queue_capacity", "Queue capacity"], ["dropped_full", "Dropped queue messages"],
    ["rejected_invalid", "Rejected invalid inputs"], ["rejected_closed", "Rejected closed inputs"],
    ["accepted", "Accepted inputs"], ["rejected_session", "Rejected stale sessions"],
    ["rejected_replay", "Rejected replayed inputs"], ["rejected_clock", "Rejected clock inputs"],
    ["rejected_state", "Rejected state inputs"]];
  try {
    state.review.open();
    for (const ingestion of [undefined, null, [], "bad", 1, {}, ...[-1, 0.5, NaN, Infinity, "0", true, null, Number.MAX_SAFE_INTEGER + 1]
      .map(value => Object.fromEntries([...metrics.map(([key]) => [key, value]), ["diagnostics", value]]))]) {
      assert.doesNotThrow(() => state.review.update(data({ ingestion })));
      for (const [, label] of metrics) assert.equal(field(state.dialog, label), "Unavailable");
      assert.equal(field(state.dialog, "Diagnostics (future in-process reports)"), ingestion?.diagnostics === null ? "Not connected" : "Unavailable");
      assert.equal(field(state.dialog, "Evicted observations"), "3");
      assert.ok(state.dialog.querySelector("details"));
    }
    for (const diagnostics of ["running", "connected", "toString", {}, [], false]) {
      state.review.update(data({ ingestion: { diagnostics } }));
      assert.equal(field(state.dialog, "Diagnostics (future in-process reports)"), "Unavailable");
    }
    state.review.update(data({ ingestion: Object.fromEntries(metrics.map(([key]) => [key, 0])) }));
    for (const [, label] of metrics) assert.equal(field(state.dialog, label), "0");
  } finally { state.cleanup(); }
});

test("explicit evidence preserves exact IDs, source, times, history sequence and safe text", () => {
  const state = fixture();
  try {
    state.review.update(data());
    state.review.open();
    const details = state.dialog.querySelector("details");
    assert.equal(details.hasAttribute("open"), false);
    assert.equal(field(details, "Incident ID"), "incident:exact/0001");
    assert.equal(field(details, "Explanation"), hostile);
    assert.equal(field(details, "Detector version"), "phase1.v1");
    assert.equal(field(details, "Confidence"), "uncalibrated");
    const evidence = details.querySelector(".ic-intelligence-evidence");
    for (const [label, value] of [["Observation ID", "obs:exact/0001"], ["User ID", userId], ["Channel ID", "9007199254740995"],
      ["Source", "store_notification"], ["Monotonic ns", "123000000"], ["History sequence", "42"], ["Clock uncertainty", "Wall clock may drift"]]) {
      assert.equal(field(evidence, label), value);
    }
    assert.match(field(evidence, "Wall time (UTC)"), /2023-11-14T22:13:20.000Z.*1700000000000/);
    assert.equal(state.dialog.querySelector("img,script,a,input,textarea"), null);
    assert.equal(details.querySelectorAll("button").length, 0);
    assert.match(state.document.querySelector("style").textContent, /theme-light \.ic-user-tool/);
    assert.equal(state.dialog.parentElement.hasAttribute("data-intellicord-owned"), true);
  } finally { state.cleanup(); }
});

test("open review updates counts, gaps, losses and evidence without losing expanded focus", () => {
  const state = fixture();
  try {
    state.review.update(data());
    state.review.open();
    const details = state.dialog.querySelector("details");
    details.setAttribute("open", "");
    details.querySelector("summary").focus();
    const gap = observation({ observation_id: "gap:1", subject: null, source: "reconciliation", history_sequence: null,
      kind: { type: "gap", reason: "Simulated missing transitions", missing: 5 }, wall_time_unix_ms: 1700000001000 });
    state.review.update(data({ observations: [observation(), gap], dropped_observations: 8,
      incidents: [incident({ uncertainty: "Updated uncertainty", evidence: [gap] })] }));
    assert.equal(field(state.dialog, "Retained observations"), "2");
    for (const [label, value] of [["Evicted observations", "8"], ["Evicted incidents", "2"], ["Known missing voice transitions", "5"], ["Gap count", "1"]]) assert.equal(field(state.dialog, label), value);
    assert.match(state.dialog.textContent, /Latest gaps.*gap:1.*Simulated missing transitions/s);
    assert.equal(state.dialog.querySelector("details").hasAttribute("open"), true);
    assert.equal(state.document.activeElement, state.dialog.querySelector("summary"));
    assert.equal(field(state.dialog.querySelector("details"), "Uncertainty"), "Updated uncertainty");
    state.review.update(data({ incidents: [incident(), incident({ incident_id: "newer", wall_time_unix_ms: 1700000002000 })] }));
    const retained = [...state.dialog.querySelectorAll("details")].find(item => field(item, "Incident ID") === "incident:exact/0001");
    assert.equal(retained.hasAttribute("open"), true);
    assert.equal(state.document.activeElement, retained.querySelector("summary"));
    state.review.update({ schema_version: 99 });
    assert.match(state.document.activeElement.getAttribute("aria-label"), /Close Safety/);
    assert.equal(state.dialog.querySelector("details"), null);
  } finally { state.cleanup(); }
});

test("malformed records and evidence are guarded; unsupported observation versions stay unavailable", () => {
  const state = fixture();
  try {
    state.review.open();
    for (const value of [data({ incidents: [null, 3, "bad", incident({ evidence: [null, {}, observation({ schema_version: 9 })] })], observations: [null, 1, {}] }),
      data({ incidents: [incident({ evidence: {} })], observations: {}, dropped_incidents: NaN }),
      data({ observations: [observation({ observation_id: "x".repeat(513), subject: { user_id: 9007199254740992 }, wall_time_unix_ms: Infinity, history_sequence: -1 })] })]) {
      assert.doesNotThrow(() => state.review.update(value));
      assert.match(state.dialog.textContent, /Unavailable|unavailable/);
    }
    assert.equal(state.dialog.textContent.includes("x".repeat(513)), false);
    assert.equal(field([...state.dialog.querySelectorAll("details")].at(-1), "User ID"), "Unavailable");
  } finally { state.cleanup(); }
});

test("lists show at most 50 newest incidents, 20 observations and 20 gaps; evidence is bounded", () => {
  const state = fixture();
  try {
    const incidents = Array.from({ length: 60 }, (_, index) => incident({ incident_id: `incident:${index}`, wall_time_unix_ms: index }));
    const observations = Array.from({ length: 30 }, (_, index) => observation({ observation_id: `gap:${index}`, wall_time_unix_ms: index, kind: { type: "gap" } }));
    state.review.update(data({ incidents, observations }));
    state.review.open();
    const details = [...state.dialog.querySelectorAll("details")];
    assert.equal(details.filter(item => item.dataset.reviewKey.startsWith("incident:")).length, 50);
    assert.equal(details.filter(item => item.dataset.reviewKey.startsWith("Latest observations:")).length, 20);
    assert.equal(details.filter(item => item.dataset.reviewKey.startsWith("Latest gaps:")).length, 20);
    assert.match(details[0].querySelector("summary").textContent, /incident:59$/);
    assert.match(details[49].querySelector("summary").textContent, /incident:10$/);
    state.review.update(data({ incidents: [incident({ evidence: Array(101).fill(observation()) })] }));
    assert.equal(state.dialog.querySelectorAll(".ic-intelligence-evidence").length, 100);
    assert.match(state.dialog.textContent, /101 retained evidence observations; first 100 shown/);
  } finally { state.cleanup(); }
});

test("Escape, backdrop, focus trap, restoration and disposal clean up the independent modal", () => {
  const state = fixture();
  try {
    const origin = state.document.querySelector("#origin");
    state.review.update(data());
    state.review.open();
    const dismiss = state.dialog.querySelector("button");
    assert.equal(state.document.activeElement, dismiss);
    state.key("Tab", true);
    assert.equal(state.document.activeElement, [...state.dialog.querySelectorAll("summary")].at(-1));
    state.key("Tab");
    assert.equal(state.document.activeElement, dismiss);
    origin.focus();
    assert.equal(state.document.activeElement, dismiss);
    assert.equal(state.key("Escape").defaultPrevented, true);
    assert.equal(state.dialog, null);
    assert.equal(state.document.activeElement, origin);
    state.review.open();
    state.dialog.dispatchEvent(new state.window.Event("pointerdown", { bubbles: true }));
    assert.ok(state.dialog);
    state.dialog.parentElement.dispatchEvent(new state.window.Event("pointerdown", { bubbles: true }));
    assert.equal(state.dialog, null);
    state.review.open();
    state.review.dispose();
    assert.equal(state.document.activeElement, origin);
    assert.equal(state.document.querySelector(".ic-intelligence-root"), null);
    assert.equal(state.key("Escape").defaultPrevented, false);
    origin.focus();
    state.review.open();
    state.review.update(data());
    assert.equal(state.dialog, null);
  } finally { state.cleanup(); }
});

test("menu owns review lifecycle, forwards intelligence updates and leaves history open on review Escape", () => {
  const state = fixture();
  const sent = [];
  const menu = mountMenu(value => sent.push(value));
  try {
    const history = { channel: null, participants: {}, events: [] };
    menu.update(history);
    state.document.querySelector('button[aria-label="Voice history"]').click();
    const safety = state.document.querySelector('button[aria-label="Safety & Intelligence"]');
    assert.ok(safety.querySelector("svg"));
    safety.focus();
    safety.click();
    assert.match(state.dialog.textContent, /no engine data/);
    menu.update({ ...history, intelligence: data({ process_owner: {
      schema_version: 1, source: "capture_owner_outcome", status: "unavailable", result: null,
    }, session_evidence: { schema_version: 1, source: "backend_session_evidence_aggregate",
      status: "degraded", owner_integrity: "attention", process: "awaiting_recent_sample",
      acoustic: "not_configured", cross_sensor_correlation: "unavailable",
    }, session_sensors: { schema_version: 1, source: "owner_lifecycle_aggregate",
      status: "attention", configured: 1, pending: 0, finished: 0, cleanup: "unconfirmed" } }) });
    assert.equal(field(state.dialog, "Session ID"), "session:demo");
    assert.equal(field(state.dialog, "Sensor session status"), "Owner attention required");
    assert.equal(field(state.dialog, "Configured sensor owners"), "1");
    assert.equal(field(state.dialog, "Combined cleanup"), "unconfirmed");
    assert.equal(field(state.dialog, "Combined sensor liveness"), "Not established");
    assert.equal(field(state.dialog, "Evidence status"), "Evidence integrity degraded");
    assert.equal(field(state.dialog, "Process evidence"), "Awaiting recent sample");
    assert.equal(field(state.dialog, "Evidence correlation"), "Unavailable; streams remain independent");
    assert.equal(field(state.dialog, "Evidence attribution"), "Not participant evidence");
    assert.equal(field(state.dialog, "Session owner status"), "Outcome unavailable");
    assert.equal(state.document.querySelectorAll(".ic-intelligence-root").length, 1);
    state.key("Escape");
    assert.equal(state.dialog, null);
    assert.equal(state.document.activeElement, safety);
    assert.equal(state.document.querySelector(".cm-panel").hidden, false);
    safety.click();
    menu.update(history);
    assert.match(state.dialog.textContent, /no engine data/);
    menu.dispose();
    assert.equal(state.dialog, null);
    assert.equal(state.document.querySelector("#callmon-root"), null);
    assert.equal(state.key("Escape").defaultPrevented, false);
    assert.deepEqual(sent, []);
  } finally { menu.dispose(); state.cleanup(); }
});

test("evidence export includes all retained facts and loss metadata, escaping untrusted fields and omitting unknown keys", () => {
  const observations = Array.from({ length: 512 }, (_, index) => observation({ observation_id: `obs:${index}`, extra: "SECRET_UNKNOWN_FIELD" }));
  const value = data({ session_id: "session|\n# forged", observations,
    incidents: [incident({ explanation: `${hostile}|\n# forged`, evidence: [observation({ kind: { type: "gap", missing: 2, reason: "capture gap" } })] })],
    ingestion: { diagnostics: null, dropped_full: 7 }, extra: "SECRET_UNKNOWN_FIELD" });
  const markdown = formatIntelligenceMarkdown(value, 1700000000000);
  assert.match(markdown, /2023-11-14T22:13:20.000Z/);
  assert.match(markdown, /obs:511/);
  assert.match(markdown, new RegExp(userId));
  assert.match(markdown, /\| Known missing transitions \| 2 \|/);
  assert.match(markdown, /\| Dropped queue messages \| 7 \|/);
  assert.match(markdown, /\| Evicted observations \| 3 \|/);
  assert.match(markdown, /\| Accepted inputs \| Unavailable \|/);
  assert.match(markdown, /Not connected/);
  assert.match(markdown, /uncalibrated hypotheses/);
  assert.match(markdown, /&#60;img/);
  assert.doesNotMatch(markdown, /<img|<script|SECRET_UNKNOWN_FIELD|\n# forged/);
  assert.match(markdown, /&#124;&#92;n&#35; forged/);
  assert.match(markdown, /store&#95;notification/);
  for (const invalid of [null, {}, data({ schema_version: 2 }), data({ observations: Array(513).fill(observation()) }),
    data({ incidents: Array(51).fill(incident()) }), data({ incidents: [incident({ evidence: Array(21).fill(observation()) })] }),
    data({ observations: [observation({ schema_version: 2 })] }), data({ incidents: [incident({ evidence: null })] })]) {
    assert.throws(() => formatIntelligenceMarkdown(invalid), /export unavailable/);
  }
});

test("evidence export includes only strictly parsed session integration boundaries", () => {
  const integrated = data({
    session_sensors: { schema_version: 1, source: "owner_lifecycle_aggregate", status: "settled",
      configured: 1, pending: 0, finished: 1, cleanup: "confirmed" },
    session_timing: { schema_version: 1, source: "backend_session_clock_inventory",
      voice_receipt_clock: "backend_session_monotonic", acoustic_receipt_clock: "backend_session_monotonic",
      acoustic_frame_clock: "backend_session_interval", process_timing: "unavailable",
      cross_sensor_correlation: "unavailable" },
    session_evidence: { schema_version: 1, source: "backend_session_evidence_aggregate", status: "partial",
      owner_integrity: "settled", process: "not_configured", acoustic: "frame_available",
      cross_sensor_correlation: "unavailable" },
    acoustic_owner: { schema_version: 2, status: "finished", call_bound: false,
      result: { outcome: "completed", cleanup_confirmed: true, reason: null } },
    acoustic: { schema_version: 4, source: "mixed_output", status: "stopped", frames: 1,
      discontinuities: 0, missing_messages: 0, last_receipt_ns: 30,
      latest: { source: "mixed_output", frame_start_ns: 0,
        session_start_interval_ns: { earliest_ns: 10, latest_ns: 20 }, rms: 0.1, peak: 0.2,
        clipped_sample_ratio: 0, discontinuity: false, analysis: null },
      processing_health: { schema_version: 1, status: { state: "stopped" }, capacity_batches: 8,
        pending: { batches: 0, frames: 0 }, processed: { batches: 1, frames: 1 },
        delivered: { batches: 1, frames: 1 }, discarded: { batches: 0, frames: 0 },
        overflow_events: 0, high_water_batches: 1 } },
  });
  const markdown = formatIntelligenceMarkdown(integrated, 1700000000000);
  assert.match(markdown, /## Session Integration Boundaries/);
  assert.match(markdown, /Backend publication snapshot/);
  assert.match(markdown, /\| Evidence status \| partial \|/);
  assert.match(markdown, /\| Cross-sensor correlation \| unavailable \|/);
  assert.match(markdown, /\| Acoustic frame clock \| backend session interval \|/);
  assert.match(markdown, /\| Acoustic terminal reason \| Normal or owner&#45;requested completion \|/);
  assert.match(markdown, /\| Acoustic cleanup \| Confirmed \|/);
  assert.match(markdown, /\| Processed acoustic frames \| 1 \|/);

  const malformed = formatIntelligenceMarkdown(data({
    session_sensors: { secret: "SECRET_SENSOR_PATH" },
    session_timing: { secret: "SECRET_CLOCK" },
    session_evidence: { secret: "SECRET_EVIDENCE" },
    acoustic_owner: { secret: "SECRET_OWNER" },
    acoustic: { secret: "SECRET_PCM" },
  }));
  assert.match(malformed, /## Session Integration Boundaries/);
  assert.match(malformed, /\| Evidence status \| Unavailable \|/);
  assert.doesNotMatch(malformed, /SECRET_SENSOR_PATH|SECRET_CLOCK|SECRET_EVIDENCE|SECRET_OWNER|SECRET_PCM/);
});

test("reset requires confirmation and backend acknowledgement, preserving evidence on unrelated updates", () => {
  const sent = [];
  const state = fixture({ reset: context => sent.push(context) });
  try {
    state.review.update(data()); state.review.open();
    const reset = state.dialog.querySelector('[aria-label="Reset intelligence review"]');
    const confirmation = state.dialog.querySelector('[aria-label="Confirm intelligence reset"]');
    reset.click();
    assert.equal(confirmation.hidden, false);
    assert.equal(state.document.activeElement.textContent, "Cancel");
    confirmation.querySelector("button").click();
    assert.equal(confirmation.hidden, true);
    assert.deepEqual(sent, []);
    reset.click();
    confirmation.querySelectorAll("button")[1].click();
    assert.deepEqual(sent, [{ session_id: "session:demo", reset_count: 0 }]);
    assert.equal(reset.disabled, true);
    assert.equal(field(state.dialog, "Retained observations"), "1");
    state.review.update(data());
    assert.equal(reset.disabled, true);
    assert.match(state.dialog.textContent, /Waiting for backend/);
    state.review.update(data({ reset_count: 1, observations: [], incidents: [] }));
    assert.match(state.dialog.textContent, /reset acknowledged/);
    assert.equal(field(state.dialog, "Retained observations"), "0");
    assert.equal(reset.disabled, false);
    assert.equal(confirmation.hidden, true);
    assert.equal(state.document.activeElement, reset);
    reset.click();
    state.review.update(data({ session_id: "new-session", reset_count: 0 }));
    assert.equal(confirmation.hidden, true);
    assert.equal(sent.length, 1);
  } finally { state.cleanup(); }
});

test("missing reset support stays disabled, request errors and timeouts never clear evidence or retry", context => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const state = fixture({ reset: () => { throw Error("Disconnected"); } });
  try {
    state.review.update(data()); state.review.open();
    const arm = () => state.dialog.querySelector('[aria-label="Reset intelligence review"]').click();
    const confirm = () => state.dialog.querySelector('[aria-label="Confirm intelligence reset"]').querySelectorAll("button")[1].click();
    arm(); confirm();
    assert.match(state.dialog.textContent, /request failed/);
    assert.equal(field(state.dialog, "Retained observations"), "1");
    state.review.update(data({ reset_count: undefined }));
    assert.equal(state.dialog.querySelector('[aria-label="Reset intelligence review"]').disabled, true);
  } finally { state.cleanup(); }
  let requests = 0;
  const timeout = fixture({ reset: () => { requests++; } });
  try {
    timeout.review.update(data()); timeout.review.open();
    const reset = timeout.dialog.querySelector('[aria-label="Reset intelligence review"]');
    reset.click();
    timeout.dialog.querySelector('[aria-label="Confirm intelligence reset"]').querySelectorAll("button")[1].click();
    context.mock.timers.tick(5000);
    assert.equal(requests, 1);
    assert.match(timeout.dialog.textContent, /not acknowledged/);
    assert.equal(field(timeout.dialog, "Retained observations"), "1");
    assert.equal(reset.disabled, false);
    assert.equal(timeout.document.activeElement, reset);
  } finally { timeout.cleanup(); }
});

test("download is explicit, produces Markdown and revokes owned URLs on close or failure", async () => {
  const state = fixture();
  const create = URL.createObjectURL;
  const revoke = URL.revokeObjectURL;
  const blobs = [];
  const revoked = [];
  const clicked = [];
  URL.createObjectURL = blob => { blobs.push(blob); return `blob:export-${blobs.length}`; };
  URL.revokeObjectURL = url => revoked.push(url);
  state.document.addEventListener("click", event => {
    if (event.target.tagName === "A") clicked.push({ href: event.target.href, name: event.target.download });
  });
  try {
    state.review.update(data()); state.review.open();
    assert.equal(blobs.length, 0);
    state.dialog.querySelector('[aria-label="Export retained evidence as Markdown"]').click();
    assert.equal(blobs.length, 1);
    assert.equal(blobs[0].type, "text/markdown;charset=utf-8");
    assert.match(await blobs[0].text(), new RegExp(userId));
    assert.deepEqual(clicked, [{ href: "blob:export-1", name: "intellicord-evidence.md" }]);
    assert.equal(state.document.querySelector("a"), null);
    for (let index = 0; index < 4; index++) state.dialog.querySelector('[aria-label="Export retained evidence as Markdown"]').click();
    assert.deepEqual(revoked, ["blob:export-1"]);
    state.key("Escape");
    assert.deepEqual(revoked, ["blob:export-1", "blob:export-2", "blob:export-3", "blob:export-4", "blob:export-5"]);
    state.review.open();
    URL.createObjectURL = () => { throw Error("Denied"); };
    state.dialog.querySelector('[aria-label="Export retained evidence as Markdown"]').click();
    assert.match(state.dialog.textContent, /export failed/);
    assert.equal(state.document.querySelector("a"), null);
  } finally { state.cleanup(); URL.createObjectURL = create; URL.revokeObjectURL = revoke; }
});

test("menu sends session-bound review reset only after confirmation", () => {
  const state = fixture();
  const sent = [];
  const menu = mountMenu(payload => sent.push(JSON.parse(payload)));
  try {
    menu.update({ channel: null, participants: {}, events: [], intelligence: data() });
    state.document.querySelector('[aria-label="Safety & Intelligence"]').click();
    state.dialog.querySelector('[aria-label="Reset intelligence review"]').click();
    assert.deepEqual(sent, []);
    state.dialog.querySelector('[aria-label="Confirm intelligence reset"]').querySelectorAll("button")[1].click();
    assert.deepEqual(sent, [{ kind: "reset_intelligence", session_id: "session:demo", reset_count: 0 }]);
  } finally { menu.dispose(); state.cleanup(); }
});