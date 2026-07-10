import assert from "node:assert/strict";
import test from "node:test";

import {
  configuredRunnerClasses,
  defaultRunnerClass,
  normalizeAgentDisplayName,
  sealAgentOnboardingDraft,
  unsealAgentOnboardingDraft,
} from "@/lib/agent-onboarding";

const env = {
  WORKOS_COOKIE_PASSWORD: "agent-onboarding-test-password-32-characters",
  NODE_ENV: "production",
};

test("production defaults to Kata while local development defaults to Apple Container", () => {
  assert.equal(defaultRunnerClass(env), "kata");
  assert.equal(defaultRunnerClass({ NODE_ENV: "development" }), "apple_container");
  assert.deepEqual(configuredRunnerClasses(env), ["kata"]);
});

test("runner availability is explicit and bounded", () => {
  assert.deepEqual(
    configuredRunnerClasses({
      NODE_ENV: "production",
      FC_DASHBOARD_DEFAULT_RUNNER_CLASS: "kata",
      FC_DASHBOARD_RUNNER_CLASSES: "kata, phala,unknown,kata",
    }),
    ["kata", "phala"]
  );
});

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
      runnerClass: "kata",
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
