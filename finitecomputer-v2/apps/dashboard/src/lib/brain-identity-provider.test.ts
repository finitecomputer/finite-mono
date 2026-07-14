import assert from "node:assert/strict";
import test from "node:test";

import {
  brainCapabilityMatchesCurrentAccount,
  brainIdentityRequestHash,
  issueBrainClientCapability,
  issueBrainSessionProof,
  officialBrainFrameNavigation,
  officialBrainFrameParentOrigin,
  parseBrainIdentityProviderRequest,
  verifyBrainClientCapability,
  verifyBrainSessionProof,
} from "@/lib/brain-identity-provider";

test("only a same-origin Brain iframe navigation can receive a client capability", () => {
  const frameHeaders = (
    referer: string | null,
    destination = "iframe",
    host = "finite.computer",
  ) =>
    new Headers({
      ...(referer ? { referer } : {}),
      host,
      "sec-fetch-dest": destination,
      "sec-fetch-mode": "navigate",
      "sec-fetch-site": "same-origin",
    });
  assert.equal(
    officialBrainFrameNavigation(
      "https://finite.computer/client",
      frameHeaders("https://finite.computer/dashboard/machines/machine-1/brain"),
    ),
    true
  );
  assert.equal(
    officialBrainFrameNavigation(
      "https://internal-proxy:3000/client",
      frameHeaders("https://finite.computer/dashboard/machines/machine-1/brain"),
    ),
    true,
  );
  assert.equal(
    officialBrainFrameParentOrigin(
      "http://localhost:13002/client",
      frameHeaders(
        "http://127.0.0.1:13002/dashboard/machines/machine-1/brain",
        "iframe",
        "127.0.0.1:13002",
      ),
    ),
    "http://127.0.0.1:13002",
  );
  assert.equal(
    officialBrainFrameNavigation(
      "https://finite.computer/client",
      frameHeaders("https://finite.computer/dashboard"),
    ),
    false
  );
  assert.equal(
    officialBrainFrameNavigation(
      "https://finite.computer/client",
      frameHeaders("https://sites.finite.computer/dashboard/machines/machine-1/brain"),
    ),
    false
  );
  assert.equal(
    officialBrainFrameNavigation(
      "https://finite.computer/client",
      frameHeaders("https://finite.computer/dashboard/machines/machine-1/brain", "empty"),
    ),
    false
  );
  assert.equal(
    officialBrainFrameNavigation(
      "https://finite.computer/client",
      frameHeaders(
        "https://finite.computer/dashboard/machines/machine-1/brain",
        "iframe",
        "other.finite.computer",
      ),
    ),
    false,
  );
});

test("Brain client capabilities are signed, account-and-origin-bound, and expiring", () => {
  const token = issueBrainClientCapability(
    "hosted-secret",
    "user_paul",
    "https://finite.computer",
    1_000,
    "nonce-1",
  );
  assert.deepEqual(verifyBrainClientCapability(token, "hosted-secret", 1_001), {
    workosUserId: "user_paul",
    emailVerified: true,
    brainPublicOrigin: "https://finite.computer",
  });
  assert.equal(verifyBrainClientCapability(token, "other-secret", 1_001), null);
  assert.equal(verifyBrainClientCapability(`${token}x`, "hosted-secret", 1_001), null);
  assert.equal(verifyBrainClientCapability(token, "hosted-secret", 1_000 + 8 * 60 * 60 + 1), null);
});

test("a Brain client capability remains usable only while its dashboard account is active", () => {
  const capability = verifyBrainClientCapability(
    issueBrainClientCapability(
      "hosted-secret",
      "user_paul",
      "https://finite.computer",
      1_000,
      "nonce-1",
    ),
    "hosted-secret",
    1_001,
  );
  assert.ok(capability);
  assert.equal(
    brainCapabilityMatchesCurrentAccount(capability, {
      workosUserId: "user_paul",
      emailVerified: true,
    }),
    true,
  );
  assert.equal(
    brainCapabilityMatchesCurrentAccount(capability, {
      workosUserId: null,
      emailVerified: false,
    }),
    false,
  );
  assert.equal(
    brainCapabilityMatchesCurrentAccount(capability, {
      workosUserId: "user_other",
      emailVerified: true,
    }),
    false,
  );
});

test("Brain session proofs are short-lived and bound to one exact provider request", () => {
  const requestHash = brainIdentityRequestHash('{"operation":"identifyMember"}');
  const proof = issueBrainSessionProof(
    "hosted-secret",
    "user_paul",
    requestHash,
    1_000,
    "proof-nonce",
  );
  assert.deepEqual(verifyBrainSessionProof(proof, "hosted-secret", requestHash, 1_001), {
    workosUserId: "user_paul",
    emailVerified: true,
  });
  assert.equal(
    verifyBrainSessionProof(
      proof,
      "hosted-secret",
      brainIdentityRequestHash('{"operation":"authorizeBrainEvent"}'),
      1_001,
    ),
    null,
  );
  assert.equal(verifyBrainSessionProof(proof, "hosted-secret", requestHash, 1_011), null);
});

test("the dashboard bridge forwards only the versioned bounded operation set", () => {
  assert.deepEqual(
    parseBrainIdentityProviderRequest({
      version: "finite-brain-identity-provider-v1",
      operation: "authorizeHttpRequest",
      input: { method: "GET" },
    }),
    {
      version: "finite-brain-identity-provider-v1",
      operation: "authorizeHttpRequest",
      input: { method: "GET" },
    }
  );
  for (const value of [
    null,
    { version: "v2", operation: "identifyMember", input: null },
    {
      version: "finite-brain-identity-provider-v1",
      operation: "signEvent",
      input: {},
    },
  ]) {
    assert.throws(() => parseBrainIdentityProviderRequest(value), /Brain identity request/u);
  }
});
