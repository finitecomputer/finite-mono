import assert from "node:assert/strict";
import test from "node:test";

import {
  BRAIN_SESSION_PROOF_REQUEST,
  parseBrainSessionProofRequest,
} from "./brain-session-bridge";

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
