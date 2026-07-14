import assert from "node:assert/strict";
import test from "node:test";

import {
  officialBrainClientRequest,
  parseBrainIdentityProviderRequest,
} from "@/lib/brain-identity-provider";

test("only the official same-origin Brain client path reaches the hosted adapter", () => {
  assert.equal(
    officialBrainClientRequest(
      "https://finite.computer/api/brain/identity-provider",
      "https://finite.computer/client"
    ),
    true
  );
  assert.equal(
    officialBrainClientRequest(
      "https://finite.computer/api/brain/identity-provider",
      "https://finite.computer/dashboard"
    ),
    false
  );
  assert.equal(
    officialBrainClientRequest(
      "https://finite.computer/api/brain/identity-provider",
      "https://sites.finite.computer/client"
    ),
    false
  );
  assert.equal(
    officialBrainClientRequest(
      "https://finite.computer/api/brain/identity-provider",
      null
    ),
    false
  );
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
