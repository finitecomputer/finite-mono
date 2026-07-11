import assert from "node:assert/strict";
import test from "node:test";

import type {
  CoreAgentCreationRequestSummary,
  CoreVisibleProject,
} from "./core-client";
import { coreProjectHasRunningKataCreationRequest } from "./dashboard-machine-access";

const project = {
  project: { id: "project-1" },
  runtime: { id: "runtime-1" },
} as CoreVisibleProject;

function request(
  overrides: Partial<CoreAgentCreationRequestSummary> = {}
): CoreAgentCreationRequestSummary {
  return {
    id: "request-1",
    project_id: "project-1",
    display_name: "Test agent",
    runner_class: "kata",
    status: "running",
    agent_runtime_id: "runtime-1",
    created_at: "2026-07-10T00:00:00Z",
    updated_at: "2026-07-10T00:00:00Z",
    ...overrides,
  };
}

test("allows removal only for a running Kata creation request for the project", () => {
  assert.equal(coreProjectHasRunningKataCreationRequest(project, [request()]), true);
  assert.equal(
    coreProjectHasRunningKataCreationRequest(project, [request({ runner_class: "phala" })]),
    false
  );
  assert.equal(
    coreProjectHasRunningKataCreationRequest(project, [request({ status: "failed" })]),
    false
  );
  assert.equal(
    coreProjectHasRunningKataCreationRequest(project, [request({ project_id: "project-2" })]),
    false
  );
  assert.equal(
    coreProjectHasRunningKataCreationRequest(project, [request({ agent_runtime_id: "runtime-2" })]),
    false
  );
  assert.equal(coreProjectHasRunningKataCreationRequest(project, []), false);
});
