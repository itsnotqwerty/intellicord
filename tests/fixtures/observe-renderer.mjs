import { build } from "esbuild";
import { parseHTML } from "linkedom";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { createProgressHandoff, ownerFeature, progressFeature } from "../../src/web/process-progress.js";
import { acousticFeature } from "../../src/web/acoustic-status.js";
import { sessionEvidenceFeature } from "../../src/web/session-evidence.js";
import { sessionSensorsFeature } from "../../src/web/session-sensors.js";
import { sessionTimingFeature } from "../../src/web/session-timing.js";

const bundle = await build({
  entryPoints: [fileURLToPath(new URL("../../src/web/menu.js", import.meta.url))],
  bundle: true, write: false, format: "esm", loader: { ".css": "text" },
});
const { mountMenu } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);
const { window, document } = parseHTML("<html><head></head><body></body></html>");
globalThis.window = window;
globalThis.document = document;
let focused = document.body;
Object.defineProperty(document, "activeElement", { get: () => focused });
window.HTMLElement.prototype.focus = function () { focused = this; };
const sent = [];
const processProgress = createProgressHandoff();
const menu = mountMenu(payload => sent.push(JSON.parse(payload)));
const field = label => [...document.querySelectorAll(".ic-intelligence-root dt")]
  .find(item => item.textContent === label)?.nextElementSibling.textContent ?? null;
try {
  for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
    const input = JSON.parse(line);
    const view = input.view ?? input;
    processProgress.accept(view.intelligence?.process_progress, performance.now());
    menu.update(view);
    document.querySelector('[aria-label="Safety & Intelligence"]').click();
    const start = document.querySelector('[aria-label="Start output analysis"]');
    const stop = document.querySelector('[aria-label="Stop output analysis"]');
    if (input.action === "start_acoustic") {
      start.click();
      [...document.querySelectorAll('[aria-label="Confirm output analysis start"] button')]
        .find(button => button.textContent === "Start analysis").click();
    }
    if (input.action === "stop_acoustic") stop.click();
    process.stdout.write(`${JSON.stringify({
      status: field("Session owner status"), outcome: field("Session owner outcome"),
      reaped: field("Session child reaped"), removed: field("Session endpoint removed"),
      liveness: field("Session sensor liveness"), feature: ownerFeature(view.intelligence?.process_owner),
      session_evidence: { feature: sessionEvidenceFeature(view.intelligence?.session_evidence),
        status: field("Evidence status"), process: field("Process evidence"), acoustic: field("Acoustic evidence"),
        correlation: field("Evidence correlation"), attribution: field("Evidence attribution") },
      sensor_session: sessionSensorsFeature(view.intelligence?.session_sensors),
      session_timing: { feature: sessionTimingFeature(view.intelligence?.session_timing),
        voice: field("Voice receipt clock"), acoustic: field("Acoustic frame clock"),
        process: field("Process timing"), correlation: field("Cross-sensor correlation") },
      process_telemetry: { status: field("Session progress status"), liveness: field("Session progress liveness"),
        feature: progressFeature(processProgress.read()) },
      acoustic: { status: field("Acoustic status"), rms: field("Last acoustic RMS"),
        centroid: field("Latest spectral centroid"), repetition: field("Best coarse repetition similarity"),
        source: field("Acoustic source"), attribution: field("Acoustic attribution"),
        clock: field("Acoustic clock"), interval: field("Latest frame session interval"),
        processing_scope: field("Processing health scope"), processed: field("Processed acoustic frames"),
        delivered: field("Delivered acoustic frames"), pending: field("Pending acoustic frames"),
        discarded: field("Discarded acoustic frames"), overflows: field("Processing overflow events"),
        queue_high_water: field("Processing queue high-water"),
        liveness: field("Acoustic liveness"), feature: acousticFeature(view.intelligence?.acoustic, view.intelligence?.acoustic_owner),
        owner_status: field("Acoustic owner status"), owner_outcome: field("Acoustic owner outcome"),
        owner_reason: field("Acoustic terminal reason"), cleanup: field("Acoustic cleanup"), call_binding: field("Acoustic call binding"),
        start_visible: !start.hidden, start_disabled: start.disabled, start_request: field("Acoustic start request"),
        stop_visible: !stop.hidden, stop_disabled: stop.disabled, stop_request: field("Acoustic stop request") },
      commands: sent.splice(0),
    })}\n`);
  }
} catch (cause) {
  process.stderr.write(`renderer failure: ${cause?.message || String(cause)}\n`);
  process.exitCode = 1;
} finally {
  menu.dispose();
}