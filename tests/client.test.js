import assert from "node:assert/strict";
import test from "node:test";
import { build } from "esbuild";
import { runInNewContext } from "node:vm";

test("startup store discovery failures reach the launcher before mounting features", async () => {
  const bundle = await build({
    entryPoints: ["src/web/client.js"],
    bundle: true,
    write: false,
    format: "iife",
    loader: { ".css": "text" },
  });
  let sent = false;
  const context = {
    __callmon_send() { sent = true; },
  };
  assert.throws(() => runInNewContext(bundle.outputFiles[0].text, context), /module loader is unavailable/);
  assert.equal(sent, false);
  assert.equal(context.__intellicord, undefined);
  assert.equal(context.__callmon, undefined);
});