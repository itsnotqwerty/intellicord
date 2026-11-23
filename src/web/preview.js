import { mountMenu } from "./menu.js";
import { observeCallView, observeClientDom, sponsoredContentNodes, sponsoredContentSection, quietSurfaceNodes } from "./bridge.js";
import { mountSponsoredContent } from "./sponsored.js";
import { createElement, Settings, ArrowLeft } from "lucide";
import {mountIdentityLens} from "./identity.js";
import {mountDiscordLookup} from "./lookup.js";
import {createVoiceWatch} from "./watch.js";
import {mountVoiceWatch} from "./watch-panel.js";
import {mountLinkInspector} from "./links.js";
import {mountSessionHealth} from "./health.js";
import {mountObliteratePanel} from "./obliterate-panel.js";

const now = Date.now();
const channel = { id: "111111111111111111", name: "Weekend group", kind: 3 };
const people = [
  { id: "9007199254740993", username: "morgan", display_name: "Morgan" },
  { id: "222222222222222222", username: "jules", display_name: "Jules" },
  { id: "333333333333333333", username: "alex.chen", display_name: "Alex Chen" },
  { id: "444444444444444444", username: "riley", display_name: "Riley" },
].map((user, index) => ({ ...user, avatar_url: `https://cdn.discordapp.com/embed/avatars/${index}.png` }));
const demoObservation = (observationId, user, timestamp, sequence, kind = { type: "joined" }) => ({
  schema_version: 1, observation_id: observationId, session_id: "SIMULATED-preview-session", source: "store_notification",
  monotonic_ns: (timestamp - (now - 7 * 60000)) * 1000000, wall_time_unix_ms: timestamp,
  subject: user ? { user_id: user.id, channel_id: channel.id, call_id: 3 } : null, kind,
  evidence_quality: "snapshot_difference", clock_uncertainty: "SIMULATED receipt times; sampling and transport latency are unmeasured",
  history_sequence: sequence,
});
const demoEvidence = [demoObservation("SIMULATED-observation:1", people[2], now - 6 * 60000, 1),
  demoObservation("SIMULATED-observation:2", people[3], now - 6 * 60000 + 1000, 2)];
const state = {
  channel,
  intelligence: {
    schema_version: 1, reset_count: 0, session_id: "SIMULATED-preview-session", observations: [...demoEvidence,
      { ...demoObservation("SIMULATED-gap:3", null, now - 5 * 60000, null,
        { type: "gap", reason: "SIMULATED reconciliation gap; intermediate transitions were not observed", missing: 2 }), source: "reconciliation" }],
    incidents: [{ incident_id: "SIMULATED-incident:1", detector_version: "SIMULATED-voice-patterns/0.1", rule: "join_burst",
      explanation: "SIMULATED incident: two retained join differences close together. This is demo data, not a live detector result.",
      uncertainty: "SIMULATED snapshot differences do not establish intent; a later gap leaves transitions unknown.",
      confidence: "uncalibrated", recommended_action: "Review simulated evidence only. No action is taken.",
      call_id: 3, channel_id: channel.id, wall_time_unix_ms: now - 6 * 60000 + 1000, evidence: demoEvidence }],
    dropped_observations: 4, dropped_incidents: 1, missing_transitions: 2, gap_count: 1,
    ingestion: { queue_capacity: 64, dropped_full: 1, rejected_invalid: 0, rejected_closed: 0, accepted: 3,
      rejected_session: 0, rejected_replay: 0, rejected_clock: 0, rejected_state: 0, diagnostics: null },
    session_sensors: { schema_version: 1, source: "owner_lifecycle_aggregate", status: "sensor_free",
      configured: 0, pending: 0, finished: 0, cleanup: "not_applicable" },
    session_timing: { schema_version: 1, source: "backend_session_clock_inventory",
      voice_receipt_clock: "backend_session_monotonic", acoustic_receipt_clock: "unavailable",
      acoustic_frame_clock: "unavailable", process_timing: "unavailable", cross_sensor_correlation: "unavailable" },
    process_owner: { schema_version: 1, source: "capture_owner_outcome", status: "not_configured", result: null },
    process_progress: { schema_version: 1, source: "owner_progress_subscription", status: "not_connected",
      sensor_running: false, valid_for_ms: 0, sample: null },
    acoustic: null,
    acoustic_owner: { schema_version: 1, status: "not_configured", call_bound: false, result: null },
    acoustic_start_available: false,
    acoustic_stop_available: false,
  },
  current_call: { id: 3, channel, started_at: now - 7 * 60000, ended_at: null,
    recap: { baseline_count: 2, participant_ids: people.map(user => user.id), peak_participants: 4, joined_count: 3, left_count: 3, probable_reconnections: 1 } },
  archives: [1, 2].map((id, index) => ({ id, channel, started_at: now - (index ? 50 : 100) * 60000,
    ended_at: now - (index ? 30 : 80) * 60000, events: [],
    recap: { baseline_count: 2, participant_ids: people.slice(0, index ? 3 : 2).map(user => user.id), peak_participants: index ? 3 : 2,
      joined_count: index ? 1 : 0, left_count: 0, probable_reconnections: 0 } })),
  latest_sequence: 6,
  participants: Object.fromEntries([people[0], people[2]].map(user => [user.id, user])),
  events: [
    { sequence: 1, timestamp: now - 6 * 60000, action: "joined", user: people[2], channel },
    { sequence: 2, timestamp: now - 5 * 60000, action: "joined", user: people[3], channel },
    { sequence: 3, timestamp: now - 4 * 60000, action: "left", user: people[3], channel },
    { sequence: 4, timestamp: now - 4 * 60000 + 10000, action: "joined", user: people[3], channel, reconnect_of: 3, reconnect_gap_ms: 10000 },
    { sequence: 5, timestamp: now - 3 * 60000, action: "left", user: people[3], channel },
    { sequence: 6, timestamp: now - 60000, action: "left", user: people[1], channel },
  ],
};
const mutedUsers = new Set();
const listeners = new Set();
const store = methods => ({...methods,addChangeListener:callback=>listeners.add(callback),removeChangeListener:callback=>listeners.delete(callback)});
const stores = {users:{getUser:id=>people.find(user=>user.id===id)},selected:store({getVoiceChannelId:()=>state.channel?.id}),voice:store({getVoiceStatesForChannel:()=>Object.values(state.participants).map(user=>({userId:user.id}))})};
const identity = mountIdentityLens(stores);
const watch = createVoiceWatch(stores, event => { document.querySelector('#demo-watch-status').textContent=`${event.user.display_name} ${event.action}`; });
const watchPanel = mountVoiceWatch(watch, stores);
const demoActions = [
  ["mute", "Mute user"], ["soundboard", "Mute soundboard"], ["video", "Disable video"],
  ["ignore", "Ignore user"], ["block", "Block user"], ["moderation", "Ban member"],
];
const obliterate = mountObliteratePanel({
  plan: ({userId, channelId}) => ({ target: {id:userId, name:people.find(user=>user.id===userId)?.display_name ?? userId},
    context: {channelId,guildId:"555555555555555555",guildName:"Simulated server"},
    actions:demoActions.map(([key,label])=>({key,label,status:"ready",...(key==="moderation"?{severity:"ban"}:{})})) }),
  executeAction: async () => ({status:"done",detail:"Simulated only; no Discord action was taken."}),
});
const lookup = mountDiscordLookup({identity,watch:watchPanel,obliterate:{
  context:menu=>({userId:menu.dataset.demoUserId,channelId:channel.id}),open:context=>obliterate.open(context),
}});
const linkInspector = mountLinkInspector();
const health = mountSessionHealth({sidebar:()=>document.querySelector('#preview-privacy:not([hidden]) .demo-settings-tabs'),observe:observeClientDom},()=>({startedAt:now,lastHeartbeat:Date.now(),lastSnapshot:Date.now(),features:{'Identity Lens':{state:'ready',detail:'Demo profile'},'Voice Watch':{state:'ready',detail:'Demo current call'},'Link Inspector':{state:'ready',detail:'Local URL parser'}}}));
let contextMenu = null;
function closeContextMenu() { contextMenu?.remove(); contextMenu = null; }
document.addEventListener("pointerdown", event => {
  if (contextMenu && !contextMenu.contains(event.target)) closeContextMenu();
});
document.addEventListener("keydown", event => {
  if (event.key === "Escape") closeContextMenu();
});
const menu = mountMenu(payload => {
  const command = JSON.parse(payload);
  if (command.kind === "diagnostics_check" && command.session_id === state.intelligence.session_id) {
    menu.diagnostics.update({ schema_version: 1, session_id: command.session_id, request_id: command.request_id,
      source: "preview_fixture", error: null, report: { schema_version: 3, platform: "linux", kernel_btf: "present",
        cgroup_v2: "present", unprivileged_bpf: "disabled_configurable", perf_event_paranoid: 2,
        effective_capabilities: { bpf: false, perfmon: false, sys_admin: false }, sched_process_exec: "unreadable",
        lockdown: "unknown", no_new_privileges: true, seccomp: "filter", sensor_running: false, load_test_performed: false },
      process_object: { schema_version: 1, inspector_available: true, status: "layout_matched",
        load_test_performed: false, sensor_running: false },
      process_clock: { schema_version: 1, source: "local_kernel_clock_probe", status: "available", clock_domain: "linux_monotonic",
        bracket_width_ns: 2400, horizon_ns: 1000000000, assumed_rate_error_ppm: 100, sensor_running: false, load_test_performed: false },
      process_clock_handshake: { schema_version: 1, source: "local_clock_handshake", status: "available", clock_domain: "linux_monotonic",
        max_bracket_ns: 25000000, sensor_running: false, load_test_performed: false,
        result: { schema_version: 1, outcome: "completed", helper_started: true, helper_reaped: true, bracket_width_ns: 180000 } },
      process_timed: { schema_version: 1, source: "generated_replay", clock_domain: "linux_monotonic", sensor_running: false, load_test_performed: false,
        health: { accepted: 2, rejected_record: 1, rejected_receipt_clock: 1, rejected_anchor_expired: 1,
          rejected_source_clock: 1, rejected_mapping: 1, rejected_future: 1, rejected_stale: 1 },
        cases: ["interval_bounds", "foreign_record", "receipt_regression", "source_regression", "anchor_expiry", "mapping_expiry",
          "future_event", "stale_event", "rejection_isolation"].map(name => ({ name, passed: true })),
        transport: ["fragmented_delivery", "truncated_record", "foreign_record", "stream_limit", "receipt_regression",
          "failed_exit", "empty_stream", "delayed_fragment"].map(name => ({ name, passed: true })) },
      process_preflight: { schema_version: 1, source: "offline_object_preflight", sensor_running: false, load_test_performed: false,
        outcome: "completed", helper_started: true, helper_reaped: true,
        object: { schema_version: 1, inspector_available: true, status: "layout_matched", load_test_performed: false, sensor_running: false,
          load_probe_available: true,
          code: { schema_version: 1, functions: 4, instruction_slots: 100, map_lookup_calls: 2, clock_calls: 1, identity_calls: 1,
            ring_reserve_calls: 1, ring_submit_calls: 1, other_helper_calls: 0, local_calls: 0, indirect_calls: 0 } } },
      process_target: { schema_version: 1, source: "self_pidfd_probe", status: "available",
        target_selected: false, load_test_performed: false, sensor_running: false },
      process_lifecycle: ["validating", "loading", "attaching", "observing", "stopping", "stopped"].map((phase, index) => ({
        schema_version: 1, record_type: "process_status", source: "generated_replay", sequence: index + 1, phase,
        reason: index === 5 ? "duration_limit" : null, attachment_confirmed: false, detachment_confirmed: false,
        health: index === 5 ? { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 } : null })),
      process_validation: { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
        cases: ["normal_completion", "fault_completion", "truncated_record", "sequence_replay", "source_mismatch",
          "confirmation_forgery", "health_regression", "exit_mismatch"].map(name => ({ name, passed: true })) },
      process_health_snapshot: { schema_version: 1, source: "operator_supplied_snapshot", status: "supplied", sensor_running: false,
        report: { schema_version: 1, source: "supervised_kernel_capture", sensor_running: false,
          run: { schema_version: 1, outcome: "completed", collector_started: true, collector_reaped: true,
            status: { schema_version: 1, source: "kernel_tracepoint", records: 6, terminal_phase: "stopped", reason: "duration_limit",
              attachment_confirmed: true, detachment_confirmed: true },
            terminal_health: { schema_version: 1, accepted: 2, rejected_record: 0, rejected_clock: 0, ring_dropped: 0 } } } },
      process_supervision: { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
        response_deadline_ms: 2000,
        quiet_collection_check: true,
        negotiated_collection: { schema_version: 1, outcome: "completed", collector_started: true, collector_reaped: true,
          collection: { schema_version: 1,
            status: { schema_version: 1, source: "generated_replay", records: 6, terminal_phase: "stopped", reason: "duration_limit",
              attachment_confirmed: false, detachment_confirmed: false },
            events: { schema_version: 1, records: 2, health: { accepted: 2, rejected_record: 0, rejected_receipt_clock: 0,
              rejected_anchor_expired: 0, rejected_source_clock: 0, rejected_mapping: 0, rejected_future: 0, rejected_stale: 0 } },
            collector_health: { schema_version: 1, accepted: 2, rejected_record: 0, rejected_clock: 0, ring_dropped: 0 } } },
        collection: { schema_version: 1, outcome: "completed", collector_started: true, collector_reaped: true,
          collection: { schema_version: 1,
            status: { schema_version: 1, source: "generated_replay", records: 6, terminal_phase: "stopped", reason: "duration_limit",
              attachment_confirmed: false, detachment_confirmed: false },
            events: { schema_version: 1, records: 2, health: { accepted: 2, rejected_record: 0, rejected_receipt_clock: 0,
              rejected_anchor_expired: 0, rejected_source_clock: 0, rejected_mapping: 0, rejected_future: 0, rejected_stale: 0 } },
            collector_health: { schema_version: 1, accepted: 2, rejected_record: 0, rejected_clock: 0, ring_dropped: 0 } } },
        supervision: { schema_version: 1, outcome: "completed", collector_started: true, collector_reaped: true,
          status: { schema_version: 1, source: "generated_replay", records: 6, terminal_phase: "stopped", reason: "duration_limit",
            attachment_confirmed: false, detachment_confirmed: false } } },
      process_replay: { schema_version: 1, source: "generated_replay", sensor_running: false,
        progress: { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
          checks: ["fragmented_delivery", "foreign_session", "replayed_sequence", "counter_regression", "before_observation", "freshness_bounds", "truncated_record", "exit_mismatch"]
            .map(name => ({ name, passed: true })) },
        delivery: { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
          checks: ["matching_exec", "foreign_process", "worker_thread", "before_release", "after_receipt", "malformed_record", "backwards_bracket"]
            .map(name => ({ name, passed: true })) },
        attachment: { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false, attachment_attempted: false,
          checks: ["setup_failure_stops", "load_failure_stops", "disabled_target_required", "attach_failure_stops", "detach_failure_visible", "successful_cleanup"]
            .map(name => ({ name, passed: true })) },
        record_layout: { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
          checks: ["native_layout", "full_width_fields", "unaligned_decode", "invalid_record_rejection"].map(name => ({ name, passed: true })) },
        producer: { schema_version: 1, source: "generated_replay", sensor_running: false, load_test_performed: false,
          capacity: 2, attempts: 5, submitted: 3, dropped: 2,
          reservation: ["filtered_reservation", "reserve_before_clock", "single_submission", "native_record_delivery", "full_ring_preservation", "fresh_recovery"]
            .map(name => ({ name, passed: true })),
          accounting: ["counter_saturation", "per_cpu_sum", "aggregate_saturation", "snapshot_high_water", "saturated_filter", "saturated_recovery"]
            .map(name => ({ name, passed: true })),
          cases: ["filter_before_reservation", "submitted_identity", "capacity_rejection", "full_ring_filter", "recovery_after_drain", "drop_agreement"]
            .map(name => ({ name, passed: true })) },
        selection: ["disabled_target", "foreign_tgid", "matching_leader", "matching_thread", "zero_process", "zero_thread", "full_width_identity"]
          .map(name => ({ name, passed: true })),
        health: { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 } } });
  }
  if (command.kind === "clear") { state.events = []; menu.update(state); }
  if (command.kind === "reset_intelligence" && command.session_id === state.intelligence.session_id && command.reset_count === state.intelligence.reset_count) {
    Object.assign(state.intelligence, { observations: [], incidents: [], dropped_observations: 0, dropped_incidents: 0,
      missing_transitions: 0, gap_count: 0, reset_count: state.intelligence.reset_count + 1 });
    menu.update(state);
  }
}, {
  open(event, id) {
    closeContextMenu();
    contextMenu = document.createElement("div");
    contextMenu.className = "preview-context-menu";
    contextMenu.id = "user-context";
    contextMenu.dataset.demoUserId = id;
    contextMenu.setAttribute("role", "menu");
    contextMenu.setAttribute("aria-label", "User actions");
    for (const label of ["Profile", "Message"]) {
      const item = document.createElement("button");
      item.type = "button";
      item.textContent = label;
      item.disabled = true;
      item.setAttribute("role", "menuitem");
      contextMenu.append(item);
    }
    const mute = document.createElement("button");
    mute.type = "button";
    mute.id = "user-context-mute";
    mute.textContent = "Mute";
    mute.setAttribute("role", "menuitemcheckbox");
    mute.setAttribute("aria-checked", String(mutedUsers.has(id)));
    mute.disabled = id === people[0].id || !state.participants[id];
    mute.addEventListener("click", () => {
      if (mutedUsers.has(id)) mutedUsers.delete(id);
      else mutedUsers.add(id);
      closeContextMenu();
    });
    const copy = document.createElement("button");
    copy.type = "button";
    copy.id = `user-context-devmode-copy-id-${id}`;
    copy.textContent = "Copy User ID";
    copy.setAttribute("role", "menuitem");
    copy.addEventListener("click", async () => {
      try { await navigator.clipboard.writeText(id); closeContextMenu(); }
      catch { copy.textContent = id; }
    });
    contextMenu.append(mute, copy);
    document.body.append(contextMenu);
    const rect = event.currentTarget.getBoundingClientRect();
    contextMenu.style.left = `${Math.max(8, Math.min(event.pageX || rect.left, window.innerWidth - 208))}px`;
    contextMenu.style.top = `${Math.max(8, Math.min(event.pageY || rect.top, window.innerHeight - contextMenu.offsetHeight - 8))}px`;
    mute.focus();
  },
}, observeCallView, {identity, watch:watchPanel});
menu.update(state);
menu.connection(true);
document.querySelector(".cm-launcher").click();
function simulate(action, user, extra = {}) {
  if (!state.channel) return;
  state.events.push({ sequence: ++state.latest_sequence, timestamp: Date.now(), action, user, channel, ...extra });
  if (action === "joined") state.participants[user.id] = user;
  else delete state.participants[user.id];
  const recap = state.current_call.recap;
  recap[action === "joined" ? "joined_count" : "left_count"]++;
  recap.peak_participants = Math.max(recap.peak_participants, Object.keys(state.participants).length);
  if (extra.reconnect_of != null) recap.probable_reconnections++;
  menu.update(state);
  for (const listener of listeners) listener();
}
document.querySelector("#simulate-join").addEventListener("click", () => {
  simulate(state.participants[people[3].id] ? "left" : "joined", people[3]);
});
document.querySelector("#simulate-reconnect").addEventListener("click", () => {
  simulate("left", people[0]);
  simulate("joined", people[0], { reconnect_of: state.latest_sequence, reconnect_gap_ms: 0 });
});
document.querySelector("#simulate-end").addEventListener("click", () => {
  if (!state.channel) return;
  state.current_call.ended_at = Date.now();
  state.channel = null;
  state.participants = {};
  menu.update(state);
  for (const listener of listeners) listener();
});
let storage;
try { storage = localStorage; } catch {}
const sponsoredContent = mountSponsoredContent({ section: sponsoredContentSection, nodes: sponsoredContentNodes, surfaceNodes:quietSurfaceNodes, observe: observeClientDom }, storage);
const settingsButton = document.querySelector("#preview-settings");
settingsButton.append(createElement(Settings, { width: 18, height: 18, "aria-hidden": "true" }));
const backButton = document.querySelector("#preview-back");
backButton.append(createElement(ArrowLeft, { width: 18, height: 18, "aria-hidden": "true" }));
function showSettings(value) {
  document.querySelector("#preview-call").hidden = value;
  document.querySelector("#preview-privacy").hidden = !value;
  document.querySelector("main").toggleAttribute("data-intellicord-call-view", !value);
}
settingsButton.addEventListener("click", () => showSettings(true));
backButton.addEventListener("click", () => showSettings(false));
globalThis.callmonPreview = { menu, state, mutedUsers, sponsoredContent, identity, watch, watchPanel, lookup, linkInspector, health, obliterate };