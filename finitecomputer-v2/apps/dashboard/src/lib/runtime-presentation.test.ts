import assert from "node:assert/strict";
import test from "node:test";

import {
  formatAgeSeconds,
  runtimeCanPresentActivity,
  runtimeHealthAgeLabel,
  runtimeHealthAgeSeconds,
  runtimeHealthAnnotation,
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

const now = Date.parse("2026-08-24T12:00:00Z");

test("health age prefers the runner's observation time and clamps skew", () => {
  assert.equal(
    runtimeHealthAgeSeconds(
      { observed_at: "2026-08-24T11:59:15Z", reported_at: "2026-08-24T11:59:20Z" },
      now
    ),
    45
  );
  assert.equal(runtimeHealthAgeSeconds({ reported_at: "2026-08-24T11:48:00Z" }, now), 720);
  assert.equal(runtimeHealthAgeSeconds({ observed_at: "2026-08-24T12:00:05Z" }, now), 0);
  assert.equal(runtimeHealthAgeSeconds({ observed_at: "not a time" }, now), null);
  assert.equal(runtimeHealthAgeSeconds(null, now), null);
  assert.equal(runtimeHealthAgeSeconds(undefined, now), null);
});

test("ages read as seconds, minutes, hours, then days", () => {
  assert.equal(formatAgeSeconds(45), "45s");
  assert.equal(formatAgeSeconds(720), "12m");
  assert.equal(formatAgeSeconds(7_200), "2h");
  assert.equal(formatAgeSeconds(172_800), "2d");
});

test("health annotations carry the check age and stay silent without Core evidence", () => {
  const fresh = { observed_at: "2026-08-24T11:59:15Z" };
  assert.equal(runtimeHealthAgeLabel(fresh, now), "last checked 45s ago");
  assert.equal(runtimeHealthAgeLabel({}, now), "not yet checked");
  assert.equal(runtimeHealthAnnotation(fresh, now), "Last checked 45s ago.");
  assert.equal(
    runtimeHealthAnnotation({ reported_at: "2026-08-24T11:48:00Z" }, now),
    "Last checked 12m ago."
  );
  // Core sent the health field but no report has landed yet.
  assert.equal(runtimeHealthAnnotation({ observed_at: null, reported_at: null }, now), "Not yet checked.");
  // Core derives the status; without its health field there is nothing to
  // annotate and the status wording must stand exactly as sent.
  assert.equal(runtimeHealthAnnotation(null, now), "");
  assert.equal(runtimeHealthAnnotation(undefined, now), "");
});
