import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

test("desktop launcher resolves HOME at runtime and preserves Discord URL arguments", async () => {
  const home = await mkdtemp(join(tmpdir(), "intellicord home "));
  try {
    await mkdir(join(home, ".local/bin"), { recursive: true });
    await writeFile(join(home, ".local/bin/intellicord"), '#!/bin/sh\nprintf "%s\\n" "$@" > "$HOME/arguments"\n', { mode: 0o755 });
    const desktop = resolve("packaging/discord.desktop");
    assert.equal(spawnSync("desktop-file-validate", [desktop]).status, 0);
    const url = "discord://-/channels/@me?example=hello%20world&other=1";
    const result = spawnSync("gio", ["launch", desktop, url], { env: { ...process.env, HOME: home }, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(await readFile(join(home, "arguments"), "utf8"), `launch\n${url}\n`);
  } finally { await rm(home, { recursive: true, force: true }); }
});