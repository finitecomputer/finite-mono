import assert from "node:assert/strict";
import { test } from "node:test";

import { agentOnboardingStageFromSearchParams } from "./agent-onboarding-progress";

test("new agent onboarding starts on Launch code", () => {
  assert.equal(
    agentOnboardingStageFromSearchParams(new URLSearchParams({ new: "1" })),
    "code"
  );
});

test("billing return resumes on Plan", () => {
  assert.equal(
    agentOnboardingStageFromSearchParams(
      new URLSearchParams({ new: "1", billing: "success" })
    ),
    "billing"
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
