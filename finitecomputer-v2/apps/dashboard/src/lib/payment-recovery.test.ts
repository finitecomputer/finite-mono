import assert from "node:assert/strict";
import { test } from "node:test";
import { paymentRecoveryPresentation } from "./payment-recovery";

test("payment recovery waits for fresh readiness and exposes actionable failure", () => {
  assert.equal(paymentRecoveryPresentation("restarting")?.description, "Restarting your agent. Your home, data, and history are retained.");
  assert.equal(paymentRecoveryPresentation("restarting")?.state, "working");
  assert.equal(paymentRecoveryPresentation("failed")?.failed, true);
  // An online agent without recovery keeps the ordinary overview and health annotation.
  assert.equal(paymentRecoveryPresentation(null), null);
  assert.equal(paymentRecoveryPresentation(undefined), null);
});

test("ordinary restart wording does not claim payment or automatic recovery", () => {
  assert.equal(paymentRecoveryPresentation("restart_pending")?.description, "Waiting for your agent to be ready. Your home, data, and history are retained.");
  const failed = paymentRecoveryPresentation("restart_failed");
  assert.equal(failed?.failed, true);
  assert.doesNotMatch(failed!.description, /payment|automatic/i);
});
