import assert from "node:assert/strict";
import test from "node:test";

import {
  BRAIN_FRAME_SANDBOX,
  BRAIN_PERSONAL_AGENT_CONFIRMATION_REQUEST,
  brainClientPath,
  brainMachinePath,
  BRAIN_SESSION_PROOF_REQUEST,
  parseBrainPersonalAgentConfirmationRequest,
  parseBrainSessionProofRequest,
} from "./brain-session-bridge";

test("the Brain frame permits its bounded confirmation dialogs without broader navigation powers", () => {
  assert.deepEqual(BRAIN_FRAME_SANDBOX.split(" "), [
    "allow-downloads",
    "allow-forms",
    "allow-modals",
    "allow-scripts",
  ]);
});

test("the dashboard confirms only its selected Personal Agent identity", () => {
  const request = {
    type: BRAIN_PERSONAL_AGENT_CONFIRMATION_REQUEST,
    requestId: "a".repeat(32),
    identity: "Cheater@finite.vip",
  };
  assert.deepEqual(
    parseBrainPersonalAgentConfirmationRequest(request, "cheater@finite.vip"),
    { ...request, identity: "cheater@finite.vip" },
  );
  assert.equal(
    parseBrainPersonalAgentConfirmationRequest(request, "someone-else@finite.vip"),
    null,
  );
  assert.equal(
    brainClientPath({
      email: "cheater-a1b2c3d4e5f60708@finite.vip",
      name: "cheater",
      brainId: "org-acme",
    }),
    "/client?agentEmail=cheater-a1b2c3d4e5f60708%40finite.vip&agentName=cheater&brainId=org-acme"
  );
  assert.equal(brainClientPath({ brainId: "../../personal" }), "/client");
  assert.equal(
    brainMachinePath("runtime_reconciled", "org-acme"),
    "/dashboard/machines/runtime_reconciled/brain?brainId=org-acme",
  );
  assert.equal(
    brainMachinePath("runtime_reconciled", "../../personal"),
    "/dashboard/machines/runtime_reconciled/brain",
  );
  assert.equal(
    parseBrainPersonalAgentConfirmationRequest({ ...request, requestId: "short" }, request.identity),
    null,
  );
});

test("the parent dashboard accepts only bounded Brain session-proof messages", () => {
  const request = {
    type: BRAIN_SESSION_PROOF_REQUEST,
    requestId: "1".repeat(32),
    requestHash: "2".repeat(64),
  };
  assert.deepEqual(parseBrainSessionProofRequest(request), request);
  for (const invalid of [
    null,
    { ...request, type: "sign-anything" },
    { ...request, requestId: "short" },
    { ...request, requestHash: "not-a-hash" },
  ]) {
    assert.equal(parseBrainSessionProofRequest(invalid), null);
  }
});

test("the selected runtime Agent Principal is only a bounded Brain input hint", () => {
  assert.equal(
    brainClientPath({
      email: "cheater-a1b2c3d4e5f60708@finite.vip",
      name: "cheater",
      npub: "npub1agentexamplekey",
    }),
    "/client?agentEmail=cheater-a1b2c3d4e5f60708%40finite.vip&agentName=cheater"
  );
  assert.equal(
    brainClientPath({ email: null, name: "cheater", npub: "npub1agentexamplekey" }),
    "/client?agentName=cheater&agentNpub=npub1agentexamplekey"
  );
  assert.equal(
    brainClientPath({ email: "not-an-email", name: "x", npub: "not-an-npub" }),
    "/client"
  );
  assert.equal(brainClientPath(null), "/client");
});
