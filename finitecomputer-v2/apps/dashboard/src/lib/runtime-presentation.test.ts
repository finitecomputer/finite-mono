import assert from "node:assert/strict";
import test from "node:test";

import {
  runtimeCanPresentActivity,
  runtimePrismState,
} from "@/lib/runtime-presentation";

test("runtime prism state distinguishes stopped agents from unhealthy agents", () => {
  assert.equal(runtimePrismState("online"), "happy");
  assert.equal(runtimePrismState("stale"), "working");
  assert.equal(runtimePrismState("offline"), "off");
  assert.equal(runtimePrismState("unknown"), "stuck");
});

test("only an online runtime can present live agent activity", () => {
  assert.equal(runtimeCanPresentActivity("online"), true);
  assert.equal(runtimeCanPresentActivity("stale"), false);
  assert.equal(runtimeCanPresentActivity("offline"), false);
  assert.equal(runtimeCanPresentActivity("unknown"), false);
});
