import assert from "node:assert/strict";
import test from "node:test";
import { callPlacement } from "../src/web/placement.js";

test("Callmon anchors inside the call, not the application corner", () => {
  assert.deepEqual(callPlacement({ left: 369, top: 32, bottom: 1080, width: 1551, height: 1048 }, { width: 1920, height: 1080 }),
    { left: 383, top: 1028, panelLeft: 0, panelHeight: 1008 });
});

test("call popup stays within narrow and short viewports", () => {
  const placement = callPlacement({ left: 200, top: 0, bottom: 320, width: 160, height: 320 }, { width: 360, height: 320 });
  assert.equal(placement.left + placement.panelLeft + 332, 346);
  assert.equal(placement.top, 268);
  assert.equal(placement.panelHeight, 248);
});

test("no visible call means no call launcher", () => {
  assert.equal(callPlacement(null, { width: 640, height: 320 }), null);
  assert.equal(callPlacement({ left: 0, top: 0, bottom: 0, width: 0, height: 0 }, { width: 640, height: 320 }), null);
});