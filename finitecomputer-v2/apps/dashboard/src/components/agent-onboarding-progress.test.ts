import assert from "node:assert/strict";
import { test } from "node:test";

import { agentOnboardingStageFromSearchParams } from "./agent-onboarding-progress";

test("new agent onboarding starts on Profile", () => {
  assert.equal(
    agentOnboardingStageFromSearchParams(new URLSearchParams({ new: "1" })),
    "profile"
  );
});

test("billing return resumes on Access", () => {
  assert.equal(
    agentOnboardingStageFromSearchParams(
      new URLSearchParams({ new: "1", billing: "success" })
    ),
    "access"
  );
});

test("a tracked creation request resumes on Launch", () => {
  assert.equal(
    agentOnboardingStageFromSearchParams(
      new URLSearchParams({ new: "1", creation: "request_1" })
    ),
    "launch"
  );
});
