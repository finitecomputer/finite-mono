import test from "node:test";
import assert from "node:assert/strict";

import {
  ONE_TIME_KEY_WARNING,
  canAccessAdminOps,
  isCoreAdminEmail,
  parseAdminEmailAllowlist,
  heartbeatAgeLabel,
  oneTimeKeyDisplay,
  oneTimeKeyError,
  type OneTimeKeyActionState,
} from "./admin-ops";

test("canAccessAdminOps only allows dashboard admins", () => {
  assert.equal(canAccessAdminOps({ isAdmin: true }), true);
  assert.equal(canAccessAdminOps({ isAdmin: false }), false);
  assert.equal(canAccessAdminOps(null), false);
  assert.equal(canAccessAdminOps(undefined), false);
});

test("heartbeatAgeLabel formats ages and degrades safely", () => {
  const now = Date.parse("2026-07-02T12:00:00Z");
  assert.equal(heartbeatAgeLabel(null, now), "never");
  assert.equal(heartbeatAgeLabel(undefined, now), "never");
  assert.equal(heartbeatAgeLabel("not-a-date", now), "unknown");
  assert.equal(heartbeatAgeLabel("2026-07-02T12:00:05Z", now), "just now");
  assert.equal(heartbeatAgeLabel("2026-07-02T11:59:30Z", now), "30s ago");
  assert.equal(heartbeatAgeLabel("2026-07-02T11:45:00Z", now), "15m ago");
  assert.equal(heartbeatAgeLabel("2026-07-02T02:00:00Z", now), "10h ago");
  assert.equal(heartbeatAgeLabel("2026-06-28T12:00:00Z", now), "4d ago");
});

test("oneTimeKeyDisplay only renders for a real issued key", () => {
  assert.equal(oneTimeKeyDisplay(null), null);
  assert.equal(oneTimeKeyDisplay(undefined), null);
  assert.equal(oneTimeKeyDisplay({ status: "idle" }), null);
  assert.equal(oneTimeKeyDisplay({ status: "error", error: "nope" }), null);
  assert.equal(
    oneTimeKeyDisplay({
      status: "issued",
      keyId: "fp_key_1",
      grantId: "fp_grant_1",
      rawKey: "   ",
      note: "",
    }),
    null,
  );

  const display = oneTimeKeyDisplay({
    status: "issued",
    keyId: "fp_key_1",
    grantId: "fp_grant_1",
    rawKey: " fpk_live_abc123 ",
    note: "",
  });
  assert.ok(display);
  assert.equal(display.keyId, "fp_key_1");
  assert.equal(display.grantId, "fp_grant_1");
  assert.equal(display.rawKey, "fpk_live_abc123");
  assert.equal(display.warning, ONE_TIME_KEY_WARNING);
});

test("oneTimeKeyDisplay keeps a Core-provided one-time note", () => {
  const display = oneTimeKeyDisplay({
    status: "issued",
    keyId: "fp_key_2",
    grantId: null,
    rawKey: "fpk_live_next",
    note: "This raw key is shown once.",
  });
  assert.ok(display);
  assert.equal(display.warning, "This raw key is shown once.");
});

test("oneTimeKeyError surfaces only error states", () => {
  assert.equal(oneTimeKeyError(null), null);
  assert.equal(oneTimeKeyError({ status: "idle" }), null);
  const issued: OneTimeKeyActionState = {
    status: "issued",
    keyId: "fp_key_1",
    grantId: null,
    rawKey: "fpk_live_x",
    note: "",
  };
  assert.equal(oneTimeKeyError(issued), null);
  assert.equal(oneTimeKeyError({ status: "error", error: "denied" }), "denied");
  assert.equal(
    oneTimeKeyError({ status: "error", error: "  " }),
    "The admin action failed.",
  );
});

test("parseAdminEmailAllowlist trims, lowercases, and drops blanks", () => {
  const allowlist = parseAdminEmailAllowlist(" Paul@finite.vip ,, austin@finite.vip ,");
  assert.deepEqual([...allowlist], ["paul@finite.vip", "austin@finite.vip"]);
  assert.equal(parseAdminEmailAllowlist(undefined).size, 0);
  assert.equal(parseAdminEmailAllowlist("  ").size, 0);
});

test("isCoreAdminEmail matches the env allowlist and fails closed", () => {
  const env = { FC_CORE_ADMIN_EMAILS: "paul@finite.vip,austin@finite.vip" };
  assert.equal(isCoreAdminEmail("paul@finite.vip", env), true);
  assert.equal(isCoreAdminEmail("someone@else.com", env), false);
  assert.equal(isCoreAdminEmail("paul@finite.vip", {}), false);
  assert.equal(isCoreAdminEmail(null, env), false);
});
