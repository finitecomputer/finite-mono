import assert from "node:assert/strict";
import test from "node:test";

import type { BillingReturnState } from "./billing-return";
import {
  resolveDashboardHomeView,
  type DashboardHomeViewInput,
} from "./dashboard-home-view";

const IDLE: BillingReturnState = { kind: "idle" };
const CONFIRMING: BillingReturnState = {
  kind: "confirming",
  deadlineAtMs: 1_750_000_090_000,
};
const SYNC_TIMEOUT: BillingReturnState = { kind: "sync-timeout" };
const READY_AGENT = {
  chatHref: "/dashboard/machines/runtime_oslo-bot/chat",
  name: "Oslo Bot",
};

function input(
  overrides: Partial<DashboardHomeViewInput> = {}
): DashboardHomeViewInput {
  return {
    coreConfigured: true,
    hasAccountEmail: true,
    isNewAgentFlow: false,
    hasProjects: false,
    hasPendingAgentCreation: false,
    hasFailedAgentCreation: false,
    creationAuthorizationRetry: false,
    readyAgent: null,
    billingReturn: IDLE,
    ...overrides,
  };
}

test("a ready tracked request resolves to the agent-ready view", () => {
  assert.deepEqual(
    resolveDashboardHomeView(
      input({ isNewAgentFlow: true, readyAgent: READY_AGENT })
    ),
    { kind: "agent-ready", ...READY_AGENT }
  );
});

test("a confirming checkout return resolves to the billing-sync view", () => {
  assert.deepEqual(
    resolveDashboardHomeView(input({ billingReturn: CONFIRMING })),
    { kind: "billing-sync", returnState: CONFIRMING }
  );
});

test("an elapsed sync window resolves to the billing-sync timeout view", () => {
  assert.deepEqual(
    resolveDashboardHomeView(input({ billingReturn: SYNC_TIMEOUT })),
    { kind: "billing-sync", returnState: SYNC_TIMEOUT }
  );
});

test("a fresh configured account resolves to the create-agent view", () => {
  assert.deepEqual(resolveDashboardHomeView(input()), { kind: "create-agent" });
});

test("the new-agent flow with existing projects still shows the creation form", () => {
  assert.deepEqual(
    resolveDashboardHomeView(input({ isNewAgentFlow: true, hasProjects: true })),
    { kind: "create-agent" }
  );
});

test("existing projects outside the new-agent flow resolve to the projects view", () => {
  assert.deepEqual(
    resolveDashboardHomeView(input({ hasProjects: true })),
    { kind: "projects" }
  );
});

test("an in-flight creation request resolves to the pending view", () => {
  assert.deepEqual(
    resolveDashboardHomeView(input({ hasPendingAgentCreation: true })),
    { kind: "pending" }
  );
  assert.deepEqual(
    resolveDashboardHomeView(
      input({ isNewAgentFlow: true, hasPendingAgentCreation: true })
    ),
    { kind: "pending" }
  );
});

test("a failed creation request resolves to the failed view", () => {
  assert.deepEqual(
    resolveDashboardHomeView(input({ hasFailedAgentCreation: true })),
    { kind: "failed" }
  );
});

test("an unconfigured Core resolves to the empty-account view", () => {
  assert.deepEqual(
    resolveDashboardHomeView(input({ coreConfigured: false })),
    { kind: "empty-account" }
  );
  assert.deepEqual(
    resolveDashboardHomeView(input({ hasAccountEmail: false })),
    { kind: "empty-account" }
  );
});

test("precedence: agent-ready beats billing-sync (the double-panel bug)", () => {
  // A tracked request flipping to "running" while the checkout return is
  // still confirming previously rendered both the Ready panel and the
  // billing-sync panel. The union makes that overlap unreachable.
  assert.deepEqual(
    resolveDashboardHomeView(
      input({
        isNewAgentFlow: true,
        readyAgent: READY_AGENT,
        billingReturn: CONFIRMING,
      })
    ),
    { kind: "agent-ready", ...READY_AGENT }
  );
});

test("precedence: agent-ready beats the creation form", () => {
  assert.deepEqual(
    resolveDashboardHomeView(input({ readyAgent: READY_AGENT })),
    { kind: "agent-ready", ...READY_AGENT }
  );
});

test("precedence: billing-sync beats the creation form", () => {
  // Without the sync window this exact state is the create-agent view.
  assert.deepEqual(
    resolveDashboardHomeView(input({ billingReturn: CONFIRMING })),
    { kind: "billing-sync", returnState: CONFIRMING }
  );
});

test("precedence: an authorization retry keeps the creation form over pending", () => {
  // The pending status panel stays hidden while the retry form is visible.
  assert.deepEqual(
    resolveDashboardHomeView(
      input({
        creationAuthorizationRetry: true,
        hasPendingAgentCreation: true,
      })
    ),
    { kind: "create-agent" }
  );
});

test("precedence: without a retry, pending beats the creation form", () => {
  assert.deepEqual(
    resolveDashboardHomeView(input({ hasPendingAgentCreation: true })),
    { kind: "pending" }
  );
});

test("precedence: failed beats empty-account", () => {
  assert.deepEqual(
    resolveDashboardHomeView(
      input({ coreConfigured: false, hasFailedAgentCreation: true })
    ),
    { kind: "failed" }
  );
});

test("precedence: projects stay primary while a creation is pending", () => {
  // Outside the new-agent flow the status panel renders alongside the
  // project list, so the list remains the primary view.
  assert.deepEqual(
    resolveDashboardHomeView(
      input({ hasProjects: true, hasPendingAgentCreation: true })
    ),
    { kind: "projects" }
  );
});

test("precedence: pending beats failed", () => {
  // The failed panel still renders alongside; the union only fixes the
  // primary view.
  assert.deepEqual(
    resolveDashboardHomeView(
      input({
        isNewAgentFlow: true,
        hasPendingAgentCreation: true,
        hasFailedAgentCreation: true,
      })
    ),
    { kind: "pending" }
  );
});

test("billing-sync stays hidden while a creation request is in flight", () => {
  assert.deepEqual(
    resolveDashboardHomeView(
      input({ billingReturn: CONFIRMING, hasPendingAgentCreation: true })
    ),
    { kind: "pending" }
  );
});

test("billing-sync stays hidden off the onboarding surface", () => {
  assert.deepEqual(
    resolveDashboardHomeView(
      input({ billingReturn: CONFIRMING, coreConfigured: false })
    ),
    { kind: "empty-account" }
  );
  // Existing projects outside the new-agent flow keep the project list.
  assert.deepEqual(
    resolveDashboardHomeView(
      input({ billingReturn: CONFIRMING, hasProjects: true })
    ),
    { kind: "projects" }
  );
  // The new-agent flow is part of the onboarding surface.
  assert.deepEqual(
    resolveDashboardHomeView(
      input({
        billingReturn: CONFIRMING,
        hasProjects: true,
        isNewAgentFlow: true,
      })
    ),
    { kind: "billing-sync", returnState: CONFIRMING }
  );
});

test("idle, cancelled, and synced checkout returns never yield billing-sync", () => {
  for (const billingReturn of [
    IDLE,
    { kind: "cancelled" },
    { kind: "synced" },
  ] as BillingReturnState[]) {
    assert.deepEqual(
      resolveDashboardHomeView(input({ billingReturn })),
      { kind: "create-agent" }
    );
  }
});
