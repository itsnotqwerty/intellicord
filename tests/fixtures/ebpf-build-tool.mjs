import { appendFileSync } from "node:fs";
import { basename } from "node:path";

const tool = basename(process.argv[1]);
const args = process.argv.slice(2);
appendFileSync(process.env.EBPF_TEST_LOG, `${JSON.stringify({ tool, args, cwd: process.cwd() })}\n`);
const stage = tool === "rustup" ? (args.includes("build") ? "build" : args[0] === "run" ? "toolchain" : "components")
  : args.includes("build") ? "build" : "inspect";
if (process.env.EBPF_TEST_FAIL === stage) {
  console.error(`Generated ${stage} failure`);
  process.exit(1);
}
if (stage === "components") {
  console.log(process.env.EBPF_TEST_NO_SOURCES ? "not-rust-src" : "rust-src");
} else if (stage === "inspect") {
  console.log(JSON.stringify({ source: "generated_build_fixture", sensor_running: false }));
}