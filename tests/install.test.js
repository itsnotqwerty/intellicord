import test from "node:test";
import assert from "node:assert/strict";
import { access, chmod, mkdtemp, mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const project = resolve(".");
const installScript = join(project, "scripts/install-user.sh");
const uninstallScript = join(project, "scripts/uninstall-user.sh");
const helperInstallScript = join(project, "scripts/install-bpf-helper.sh");
const helperUninstallScript = join(project, "scripts/uninstall-bpf-helper.sh");
const fixture = await readFile(join(project, "tests/fixtures/install-tool.mjs"), "utf8");

async function exists(path) {
  try { await access(path, constants.F_OK); return true; } catch { return false; }
}

async function fixtureDirectory(root) {
  const tools = join(root, "tools");
  await mkdir(tools);
  for (const tool of ["npm", "cargo", "rustup", "bpf-linker", "desktop-file-install",
    "desktop-file-validate", "update-desktop-database", "kbuildsycoca6"]) {
    const path = join(tools, tool);
    await writeFile(path, `#!${process.execPath}\n${fixture}`, { mode: 0o700 });
  }
  await symlink("/usr/bin/install", join(tools, "install"));
  return tools;
}

function run(script, args, environment) {
  return spawnSync("/bin/bash", [script, ...args], {
    cwd: project,
    env: environment,
    encoding: "utf8",
    timeout: 10000,
  });
}

test("user install keeps sensors optional and packages a complete explicitly requested live set", async () => {
  const root = await mkdtemp(join(tmpdir(), "intellicord-install-"));
  try {
    const home = join(root, "home");
    const data = join(root, "data");
    const state = join(root, "state");
    const target = join(root, "target");
    const ebpfTarget = join(root, "ebpf-target");
    const log = join(root, "commands.jsonl");
    await mkdir(home);
    await writeFile(log, "");
    const tools = await fixtureDirectory(root);
    const environment = {
      ...process.env,
      HOME: home,
      XDG_DATA_HOME: data,
      XDG_STATE_HOME: state,
      CARGO_TARGET_DIR: target,
      INTELLICORD_EBPF_TARGET_DIR: ebpfTarget,
      INTELLICORD_INSTALL_TEST_LOG: log,
      PATH: `${tools}:${process.env.PATH}`,
    };

    let result = run(installScript, [], environment);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(await exists(join(home, ".local/bin/intellicord")), true);
    assert.equal(await exists(join(home, ".local/bin/intellicord-observe")), false);
    assert.equal(await exists(join(data, "intellicord/intellicord-process")), false);
    let commands = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
    assert.deepEqual(commands.filter(command => command.tool === "cargo").at(-1).args,
      ["build", "--release", "--locked"]);

    await writeFile(log, "");
    result = run(installScript, ["--with-live-sensors"], environment);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /no sensor was activated/i);
    assert.equal(await exists(join(home, ".local/bin/intellicord-observe")), true);
    assert.equal(await exists(join(data, "intellicord/intellicord-process")), true);
    assert.equal(await exists(join(state, "intellicord/live-sensors.managed")), true);
    commands = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
    assert.deepEqual(commands.filter(command => command.tool === "cargo").at(-1).args,
      ["build", "--release", "--locked", "--features", "observe"]);
    assert.ok(commands.some(command => command.tool === "rustup" && command.args.includes("build")));

    result = run(uninstallScript, [], environment);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(await exists(join(home, ".local/bin/intellicord")), false);
    assert.equal(await exists(join(home, ".local/bin/intellicord-observe")), false);
    assert.equal(await exists(join(data, "intellicord/intellicord-process")), false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("user install rejects unknown modes before invoking build tools", async () => {
  const result = run(installScript, ["--live"], { ...process.env, PATH: "" });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /Usage:/);
});

test("system helper provisioning is explicit and rejects arguments before privilege checks", () => {
  const result = run(helperInstallScript, ["--automatic"], { ...process.env, PATH: "" });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /Usage: sudo/);
  const source = spawnSync("/bin/cat", [helperInstallScript], { encoding: "utf8" }).stdout;
  assert.match(source, /setcap cap_bpf,cap_perfmon,cap_dac_read_search=ep/);
  assert.match(source, /\/usr\/libexec\/intellicord\/intellicord-bpf-helper/);
  assert.doesNotMatch(source, /setcap[^\n]*intellicord-observe/);
  const removal = run(helperUninstallScript, ["--automatic"], { ...process.env, PATH: "" });
  assert.equal(removal.status, 2);
  assert.match(removal.stderr, /Usage: sudo/);
});