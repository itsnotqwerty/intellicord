import { createConnection } from "node:net";
import { closeSync, writeSync } from "node:fs";
import { endianness } from "node:os";

const [mode, socket] = process.argv.slice(2);
const health = { schema_version: 1, accepted: 2, rejected_record: 2, rejected_clock: 1, ring_dropped: 3 };
if (mode.includes("quiet") && !mode.endsWith("missing")) health.accepted = 0;
const records = ["validating", "loading", "attaching", "observing", "stopping", "stopped"].map((phase, index) => ({
  schema_version: 1, record_type: "process_status", source: "generated_replay", sequence: index + 1, phase,
  reason: index === 5 ? "duration_limit" : null, attachment_confirmed: false, detachment_confirmed: false,
  health: index === 5 ? health : null,
}));
const connection = createConnection(socket, () => {
  connection.end(JSON.stringify({ pid: process.pid }));
  const hold = () => setInterval(() => {}, 1000);
  const encoded = records.map(record => `${JSON.stringify(record)}\n`).join("");
  if (mode.startsWith("negotiated_")) {
    setTimeout(() => {
      writeSync(1, JSON.stringify({ schema_version: 1, record_type: "clock_ready", clock_domain: "linux_monotonic" }) + "\n");
      let input = "";
      process.stdin.on("data", chunk => {
        input += chunk;
        if (!input.endsWith("\n")) return;
        const request = JSON.parse(input);
        if (mode === "negotiated_stall") return hold();
        const event = Buffer.alloc(24);
        const write = endianness() === "LE" ? "writeUInt32LE" : "writeUInt32BE";
        [1, 1, 42, 42].forEach((value, index) => event[write](value, index * 4));
        event[endianness() === "LE" ? "writeBigUInt64LE" : "writeBigUInt64BE"](1000n, 16);
        const reply = Buffer.from(JSON.stringify({ schema_version: 1, record_type: "clock_reply", clock_domain: "linux_monotonic",
          nonce: request.nonce + (mode === "negotiated_nonce" ? 1 : 0), source_ns: 1000 }) + "\n");
        let bytes = Buffer.concat([event, event]);
        if (mode.includes("quiet")) bytes = mode.endsWith("partial") ? event.subarray(0, 1) : Buffer.alloc(0);
        writeSync(1, Buffer.concat([reply, mode === "negotiated_partial" ? bytes.subarray(0, 47) : bytes]));
        writeSync(2, encoded);
        if (mode === "negotiated_eof") { closeSync(1); closeSync(2); hold(); }
        if (mode === "negotiated_failure" || mode === "negotiated_quiet_failure") process.exitCode = 1;
      });
    }, 100);
    return;
  }
  if (mode.startsWith("events_")) {
    const event = Buffer.alloc(24);
    const write = endianness() === "LE" ? "writeUInt32LE" : "writeUInt32BE";
    [1, 1, mode === "events_foreign" ? 43 : 42, 42].forEach((value, index) => event[write](value, index * 4));
    let bytes = Buffer.concat([event, event]);
    if (mode.includes("quiet")) bytes = mode.endsWith("partial") ? event.subarray(0, 1) : Buffer.alloc(0);
    if (mode === "events_missing") bytes = event;
    if (mode === "events_partial") bytes = bytes.subarray(0, 47);
    if (mode === "events_overflow") bytes = Buffer.alloc(6145);
    writeSync(2, encoded);
    for (const byte of bytes) writeSync(1, Buffer.from([byte]));
    if (mode === "events_status_eof") { closeSync(2); return hold(); }
    if (mode === "events_data_eof") { closeSync(1); return hold(); }
    if (mode === "events_both_eof") { closeSync(1); closeSync(2); return hold(); }
    if (mode === "events_hold") return hold();
    if (mode === "events_failure") process.exitCode = 1;
    return;
  }
  if (mode === "stall") return hold();
  if (mode === "eof_stall") { closeSync(2); return hold(); }
  if (mode === "invalid") { writeSync(2, "{}\n"); return hold(); }
  if (mode === "oversize") { writeSync(2, " ".repeat(4096)); return hold(); }
  if (mode === "truncated") { writeSync(2, encoded.slice(0, -1)); return; }
  if (mode === "fault") {
    writeSync(2, `${JSON.stringify(records[0])}\n${JSON.stringify({ ...records[0], sequence: 2, phase: "faulted", reason: "invalid_object" })}\n`);
    process.exitCode = 1;
    return;
  }
  if (mode === "stdout_flood") writeSync(1, "untrusted output".repeat(8192));
  if (mode === "fragmented") {
    for (const byte of Buffer.from(encoded)) writeSync(2, Buffer.from([byte]));
  } else writeSync(2, encoded);
  if (mode === "terminal_stall") return hold();
  if (mode === "exit_mismatch") process.exitCode = 1;
});
connection.on("error", () => { process.exitCode = 2; });