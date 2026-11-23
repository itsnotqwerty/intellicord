import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { parseDocument } from "yaml";

const project = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const script = join(project, "scripts/build-ebpf.sh");
const fixture = readFileSync(join(project, "tests/fixtures/ebpf-build-tool.mjs"), "utf8");

function run(options = {}) {
  const directory = mkdtempSync(join(tmpdir(), "intellicord-bpf-build-"));
  try {
    for (const tool of ["cargo", "rustup", "bpf-linker"]) {
      if (tool === options.omit) continue;
      writeFileSync(join(directory, tool), `#!${process.execPath}\n${fixture}`, { mode: 0o700 });
    }
    symlinkSync("/usr/bin/dirname", join(directory, "dirname"));
    const log = join(directory, "commands.jsonl");
    writeFileSync(log, "");
    const result = spawnSync("/bin/bash", [script, ...(options.args || [])], {
      cwd: directory,
      env: { ...process.env, PATH: directory, EBPF_TEST_LOG: log,
        EBPF_TEST_FAIL: options.fail || "", EBPF_TEST_NO_SOURCES: options.noSources ? "1" : "" },
      encoding: "utf8", timeout: 10000, maxBuffer: 64 * 1024,
    });
    assert.equal(result.error, undefined);
    const commands = readFileSync(log, "utf8").trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
    return { ...result, commands };
  } finally { rmSync(directory, { recursive: true, force: true }); }
}

test("BPF build uses locked target/linker and inspects its exact artifact without capture", () => {
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.commands.map(command => command.tool), ["rustup", "rustup", "rustup", "cargo"]);
  assert.deepEqual(result.commands[0].args, ["run", "nightly-2026-10-15", "rustc", "--version"]);
  const build = result.commands[2];
  const targetDir = join(project, "crates/intellicord-ebpf/target");
  assert.equal(build.cwd, project);
  assert.deepEqual(build.args, ["run", "nightly-2026-10-15", "cargo", "build", "--locked", "--release",
    "--manifest-path", "crates/intellicord-ebpf/Cargo.toml", "--target", "bpfel-unknown-none",
    "--target-dir", targetDir, "--config", 'target.bpfel-unknown-none.linker="bpf-linker"', "-Z", "build-std=core"]);
  assert.deepEqual(result.commands[3].args, ["run", "--quiet", "--locked", "--features", "observe",
    "--bin", "intellicord-observe", "--", "inspect", "--object", join(targetDir, "bpfel-unknown-none/release/intellicord-process")]);
  assert.equal(JSON.parse(result.stdout).source, "generated_build_fixture");
  assert.equal(result.commands.some(command => command.args.includes("capture")), false);
});

test("BPF workflow stops on missing sources, build failure and inspection failure", () => {
  for (const [options, count] of [[{ fail: "toolchain" }, 1], [{ fail: "components" }, 2],
    [{ noSources: true }, 2], [{ fail: "build" }, 3], [{ fail: "inspect" }, 4]]) {
    const result = run(options);
    assert.notEqual(result.status, 0);
    assert.equal(result.stdout, "");
    assert.equal(result.commands.length, count);
  }
});

test("BPF workflow rejects arguments before executing any tools", () => {
  const result = run({ args: ["--capture"] });
  assert.equal(result.status, 2);
  assert.equal(result.stdout, "");
  assert.equal(result.commands.length, 0);
});

test("BPF workflow names missing prerequisites without invoking tools", () => {
  for (const omit of ["cargo", "rustup", "bpf-linker"]) {
    const result = run({ omit });
    assert.equal(result.status, 1);
    assert.equal(result.stdout, "");
    assert.ok(result.stderr.includes(`Required tool unavailable: ${omit}`));
    assert.equal(result.commands.length, 0);
  }
});

test("CI compiles the real object with pinned tools and read-only permissions", () => {
  const document = parseDocument(readFileSync(join(project, ".github/workflows/ebpf-build.yml"), "utf8"));
  assert.deepEqual(document.errors.map(error => error.message), []);
  const workflow = document.toJS();
  assert.deepEqual(Object.keys(workflow.on).sort(), ["pull_request", "push", "workflow_dispatch"]);
  assert.deepEqual(workflow.permissions, { contents: "read" });
  const job = workflow.jobs["compile-and-inspect"];
  assert.equal(job["runs-on"], "ubuntu-24.04");
  assert.equal(job["timeout-minutes"], 20);
  assert.equal(job.steps[0].with["persist-credentials"], false);
  const steps = job.steps.filter(step => step.run).map(step => step.run);
  assert.ok(steps.some(command => command.includes("rustup toolchain install nightly-2026-10-15 --profile minimal --component rust-src")));
  const linker = steps.find(command => command.includes("bpf-linker/releases/download/"));
  assert.ok(linker.includes("/v0.13.0/bpf-linker-x86_64-unknown-linux-musl.tar.zst"));
  assert.ok(linker.includes("0e9b2b30f68898177f1cb6f0ecbfa909a82479473a49a0ef66529fd088e3e410"));
  assert.ok(linker.includes("sha256sum --check --strict"));
  assert.ok(steps.includes("bash scripts/build-ebpf.sh > bpf-object-metadata.json"));
  const inventory = job.steps.find(step => step.run?.includes("real_bpf_artifact_has_expected_instruction_inventory"));
  assert.ok(inventory.run.includes("--ignored --exact"));
  assert.ok(inventory.run.includes("observe::verification::tests::real_bpf_artifact_exercises_load_outcomes_without_syscalls"));
  assert.equal(inventory.env.INTELLICORD_TEST_BPF_OBJECT, "crates/intellicord-ebpf/target/bpfel-unknown-none/release/intellicord-process");
  assert.ok(job.steps.indexOf(inventory) > job.steps.findIndex(step => step.run === "bash scripts/build-ebpf.sh > bpf-object-metadata.json"));
  assert.ok(steps.some(command => command.includes("cargo test --locked --features observe --lib observe::")));
  assert.ok(steps.some(command => command.includes("--test observe-supervisor")));
  assert.ok(steps.some(command => command.includes("--test observe-timed")));
  assert.ok(steps.some(command => command.includes("--test observe-reservation")));
  assert.ok(steps.every(command => !/\bsudo\b|\bcapture\b|\binject\b/.test(command)));
  const artifact = job.steps.find(step => step.uses?.startsWith("actions/upload-artifact@"));
  assert.equal(artifact.with["if-no-files-found"], "error");
  assert.equal(artifact.with["retention-days"], 7);
});