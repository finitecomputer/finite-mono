import assert from "node:assert/strict";
import test from "node:test";

import {
  BILLING_SYNC_TIMEOUT_MS,
  billingSyncStampRedirectPath,
  parseBillingReturnParam,
  parseBillingSyncStartedAt,
  resolveBillingReturnState,
} from "./billing-return";

const RETURN_AT_MS = 1_750_000_000_000;

test("parseBillingReturnParam accepts only known checkout return values", () => {
  assert.equal(parseBillingReturnParam("success"), "success");
  assert.equal(parseBillingReturnParam("cancelled"), "cancelled");
  assert.equal(parseBillingReturnParam("SUCCESS"), null);
  assert.equal(parseBillingReturnParam("anything"), null);
  assert.equal(parseBillingReturnParam(""), null);
  assert.equal(parseBillingReturnParam(null), null);
  assert.equal(parseBillingReturnParam(undefined), null);
});

test("parseBillingSyncStartedAt accepts only positive epoch integers", () => {
  assert.equal(parseBillingSyncStartedAt(String(RETURN_AT_MS)), RETURN_AT_MS);
  assert.equal(parseBillingSyncStartedAt("0"), null);
  assert.equal(parseBillingSyncStartedAt("-5"), null);
  assert.equal(parseBillingSyncStartedAt("1.5"), null);
  assert.equal(parseBillingSyncStartedAt("soon"), null);
  assert.equal(parseBillingSyncStartedAt(""), null);
  assert.equal(parseBillingSyncStartedAt("   "), null);
  assert.equal(parseBillingSyncStartedAt(null), null);
  assert.equal(parseBillingSyncStartedAt(undefined), null);
});

test("billingSyncStampRedirectPath round-trips through the URL parsers", () => {
  const path = billingSyncStampRedirectPath(RETURN_AT_MS);
  assert.equal(path, `/dashboard?billing=success&billingSyncStartedAt=${RETURN_AT_MS}`);

  const params = new URL(path, "https://finite.computer").searchParams;
  assert.equal(parseBillingReturnParam(params.get("billing")), "success");
  assert.equal(
    parseBillingSyncStartedAt(params.get("billingSyncStartedAt")),
    RETURN_AT_MS
  );
});

test("billingSyncStampRedirectPath preserves the explicit new-agent flow", () => {
  assert.equal(
    billingSyncStampRedirectPath(RETURN_AT_MS, {
      newAgent: true,
      returnMachineId: "runtime_existing-agent",
    }),
    `/dashboard?new=1&machine=runtime_existing-agent&billing=success&billingSyncStartedAt=${RETURN_AT_MS}`
  );
});

test("no checkout return renders the dashboard as usual", () => {
  assert.deepEqual(
    resolveBillingReturnState({
      billingParam: null,
      billingLoaded: true,
      requiresBilling: true,
      syncStartedAtMs: null,
      nowMs: RETURN_AT_MS,
    }),
    { kind: "idle" }
  );
});

test("webhook arrives before the checkout return: advance immediately", () => {
  assert.deepEqual(
    resolveBillingReturnState({
      billingParam: "success",
      billingLoaded: true,
      requiresBilling: false,
      syncStartedAtMs: null,
      nowMs: RETURN_AT_MS,
    }),
    { kind: "synced" }
  );
});

test("webhook slow then arrives: stamp, confirm, then advance", () => {
  // First render after the return: no sync window stamped yet.
  assert.deepEqual(
    resolveBillingReturnState({
      billingParam: "success",
      billingLoaded: true,
      requiresBilling: true,
      syncStartedAtMs: null,
      nowMs: RETURN_AT_MS,
    }),
    { kind: "stamp-sync-start" }
  );

  // While the webhook is in flight the waiting panel keeps polling.
  assert.deepEqual(
    resolveBillingReturnState({
      billingParam: "success",
      billingLoaded: true,
      requiresBilling: true,
      syncStartedAtMs: RETURN_AT_MS,
      nowMs: RETURN_AT_MS + 12_000,
    }),
    { kind: "confirming", deadlineAtMs: RETURN_AT_MS + BILLING_SYNC_TIMEOUT_MS }
  );

  // A later refresh sees Core billing active and auto-advances.
  assert.deepEqual(
    resolveBillingReturnState({
      billingParam: "success",
      billingLoaded: true,
      requiresBilling: false,
      syncStartedAtMs: RETURN_AT_MS,
      nowMs: RETURN_AT_MS + 20_000,
    }),
    { kind: "synced" }
  );
});

test("webhook never arrives: confirm until the deadline, then fall back", () => {
  // One millisecond before the deadline we are still confirming.
  assert.deepEqual(
    resolveBillingReturnState({
      billingParam: "success",
      billingLoaded: true,
      requiresBilling: true,
      syncStartedAtMs: RETURN_AT_MS,
      nowMs: RETURN_AT_MS + BILLING_SYNC_TIMEOUT_MS - 1,
    }),
    { kind: "confirming", deadlineAtMs: RETURN_AT_MS + BILLING_SYNC_TIMEOUT_MS }
  );

  // At and after the deadline the bounded poll ends in the fallback state.
  for (const elapsedMs of [BILLING_SYNC_TIMEOUT_MS, BILLING_SYNC_TIMEOUT_MS + 60_000]) {
    assert.deepEqual(
      resolveBillingReturnState({
        billingParam: "success",
        billingLoaded: true,
        requiresBilling: true,
        syncStartedAtMs: RETURN_AT_MS,
        nowMs: RETURN_AT_MS + elapsedMs,
      }),
      { kind: "sync-timeout" },
      `${elapsedMs}ms`
    );
  }
});

test("custom timeout bounds the confirming window", () => {
  assert.deepEqual(
    resolveBillingReturnState({
      billingParam: "success",
      billingLoaded: true,
      requiresBilling: true,
      syncStartedAtMs: RETURN_AT_MS,
      nowMs: RETURN_AT_MS + 5_000,
      timeoutMs: 4_000,
    }),
    { kind: "sync-timeout" }
  );
});

test("checkout cancelled shows the note only while billing setup remains", () => {
  assert.deepEqual(
    resolveBillingReturnState({
      billingParam: "cancelled",
      billingLoaded: true,
      requiresBilling: true,
      syncStartedAtMs: null,
      nowMs: RETURN_AT_MS,
    }),
    { kind: "cancelled" }
  );

  // A stale cancelled param after billing became active is ignored.
  assert.deepEqual(
    resolveBillingReturnState({
      billingParam: "cancelled",
      billingLoaded: true,
      requiresBilling: false,
      syncStartedAtMs: null,
      nowMs: RETURN_AT_MS,
    }),
    { kind: "idle" }
  );
});

test("billing overview errors fall back to the normal dashboard states", () => {
  for (const billingParam of ["success", "cancelled"] as const) {
    assert.deepEqual(
      resolveBillingReturnState({
        billingParam,
        billingLoaded: false,
        requiresBilling: false,
        syncStartedAtMs: null,
        nowMs: RETURN_AT_MS,
      }),
      { kind: "idle" },
      billingParam
    );
  }
});
