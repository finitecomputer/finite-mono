import assert from "node:assert/strict";
import { test } from "node:test";
import { paymentRecoveryPresentation } from "./payment-recovery";

test("payment recovery waits for fresh readiness and exposes actionable failure", () => {
  assert.equal(paymentRecoveryPresentation("restarting", "offline")?.description, "Restarting your agent. Your home, data, and history are retained.");
  assert.equal(paymentRecoveryPresentation("restarting", "online")?.state, "working");
  assert.equal(paymentRecoveryPresentation("failed", "offline")?.failed, true);
  assert.equal(paymentRecoveryPresentation(null, "online")?.description, "Your agent is ready.");
  assert.equal(paymentRecoveryPresentation(undefined, "offline"), null);
});

test("ordinary restart wording does not claim payment or automatic recovery", () => {
  assert.equal(paymentRecoveryPresentation("restart_pending", "offline")?.description, "Waiting for your agent to be ready. Your home, data, and history are retained.");
  const failed = paymentRecoveryPresentation("restart_failed", "offline");
  assert.equal(failed?.failed, true);
  assert.doesNotMatch(failed!.description, /payment|automatic/i);
});
