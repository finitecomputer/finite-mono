import assert from "node:assert/strict";
import test from "node:test";

import {
  agentCreationErrorMessage,
  draftStartedStripeCheckout,
  normalizeAgentDisplayName,
  resolveAgentCreationAccessPath,
  sealAgentOnboardingDraft,
  unsealAgentOnboardingDraft,
} from "@/lib/agent-onboarding";

test("agent access follows the explicitly submitted path", () => {
  assert.equal(resolveAgentCreationAccessPath("launch-code", true), "launch-code");
  assert.equal(resolveAgentCreationAccessPath("stripe", true), "stripe");
  assert.equal(resolveAgentCreationAccessPath("entitled", true), "entitlement");
  assert.equal(resolveAgentCreationAccessPath("entitled", true, false), "denied");
  assert.equal(resolveAgentCreationAccessPath("entitled", false), "denied");
  assert.equal(resolveAgentCreationAccessPath(null, true), "denied");
});

test("only a signed draft that initiated Stripe is eligible for checkout completion", () => {
  const draft = {
    version: 1 as const,
    workosUserId: "user-a",
    displayName: "Moss",
    profilePictureUrl: null,
    idempotencyKey: "idem-checkout",
    issuedAtMs: 1_000,
  };

  assert.equal(draftStartedStripeCheckout(draft), false);
  assert.equal(
    draftStartedStripeCheckout({ ...draft, stripeCheckoutStartedAtMs: 1_001 }),
    true
  );
});

test("agent creation exhaustion is explained in customer language", () => {
  const expected =
    "This account already has an agent. Open it from your dashboard, or ask an operator to remove it before creating another.";
  assert.equal(
    agentCreationErrorMessage(new Error("agent creation entitlement is exhausted")),
    expected
  );
  assert.equal(
    agentCreationErrorMessage(new Error("Agent creation entitlement is exhausted (409)")),
    expected
  );
});

test("other agent creation errors remain useful", () => {
  assert.equal(agentCreationErrorMessage(new Error("Enter your Launch Code.")), "Enter your Launch Code.");
  assert.equal(agentCreationErrorMessage(null), "Could not create agent.");
});

const env = {
  WORKOS_COOKIE_PASSWORD: "agent-onboarding-test-password-32-characters",
  NODE_ENV: "production",
};

test("agent names are compact user-facing values", () => {
  assert.equal(normalizeAgentDisplayName("  Moss   Agent  "), "Moss Agent");
  assert.throws(() => normalizeAgentDisplayName(""), /between 1 and 80/u);
});

test("onboarding draft is sealed, user-bound, and expiring", async () => {
  const issuedAtMs = Date.now();
  const sealed = await sealAgentOnboardingDraft(
    {
      version: 1,
      workosUserId: "user-a",
      displayName: "Moss",
      profilePictureUrl: "https://chat.example/profile.png",
      idempotencyKey: "request-a",
      issuedAtMs,
    },
    env
  );
  assert.equal(
    (await unsealAgentOnboardingDraft(sealed, "user-a", env, issuedAtMs + 1000))
      ?.displayName,
    "Moss"
  );
  assert.equal(await unsealAgentOnboardingDraft(sealed, "user-b", env), null);
  assert.equal(
    await unsealAgentOnboardingDraft(sealed, "user-a", env, issuedAtMs + 25 * 60 * 60 * 1000),
    null
  );
});

test("onboarding draft strips stale provider placement state", async () => {
  const issuedAtMs = Date.now();
  const staleDraft = {
    version: 1 as const,
    workosUserId: "user-a",
    displayName: "Moss",
    profilePictureUrl: null,
    idempotencyKey: "request-stale-provider",
    issuedAtMs,
    runnerClass: "phala",
  };
  const sealed = await sealAgentOnboardingDraft(staleDraft, env);
  const draft = await unsealAgentOnboardingDraft(sealed, "user-a", env, issuedAtMs + 1000);
  assert(draft);
  assert.equal("runnerClass" in draft, false);
});
