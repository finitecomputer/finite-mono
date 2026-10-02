import assert from "node:assert/strict";
import test from "node:test";

import { parseConnectionsStatus } from "./hosted-agent-controls";
import {
  backupConfiguredFor,
  classifyLegacyProvider,
  inferenceView,
  pollDelayMs,
  routeLabel,
} from "./inference-status";

// Today's agentd status (`connections.rs` `ConnectionsStatus`): no `capabilities`, no v2 keys.
const legacyStatus = (provider: string, model: string) =>
  parseConnectionsStatus({
    inference: { profile: provider === "openrouter" ? "openrouter" : "finite_private", provider, model },
    telegram: { connected: false, home_channel: null, pending: [], approved: [] },
    google: { connected: false, email: null },
  });

const ALL_CAPABILITIES = [
  "inference.status.v2", "inference.select.v1", "inference.disconnect.v1",
  "openrouter.connect.v1", "openrouter.usage.v1", "codex.login.v1", "codex.models.v1",
];

function v2Status(overrides: { inference?: Record<string, unknown>; capabilities?: string[] } = {}) {
  return parseConnectionsStatus({
    inference: {
      profile: "openrouter",
      provider: "openrouter",
      model: "anthropic/claude-sonnet-4.6",
      saved: { route: "openrouter", provider: "openrouter", model: "anthropic/claude-sonnet-4.6" },
      routes: {
        finite_private: { state: "configured", reason: null },
        openrouter: {
          state: "key_saved", key_source: "agent", key_hash: "a".repeat(64),
          hermes_key: "saved_key", other_pool_keys: "none",
        },
        openai_codex: {
          state: "not_signed_in", quota_reset_at_ms: null, reported_quota_reset_at_ms: null, login: null,
        },
      },
      fallback: { state: "configured", reason: null, model: "glm-5-3-flash", extra_entries: 0 },
      operation: null,
      ...overrides.inference,
    },
    telegram: { connected: false, home_channel: null, pending: [], approved: [] },
    google: { connected: false, email: null },
    capabilities: overrides.capabilities ?? ALL_CAPABILITIES,
  });
}

test("legacy providers classify by raw provider, and Codex is never Finite Private", () => {
  const table: Array<[string, string]> = [
    ["openrouter", "openrouter"],
    ["openai-codex", "openai_codex"],
    ["codex", "openai_codex"],
    ["openai_codex", "openai_codex"],
    ["custom", "finite_private"],
    ["finite-private", "finite_private"],
    ["custom:finite-private", "finite_private"],
    ["anthropic", "other"],
    ["OpenAI-Codex", "other"],
    ["", "other"],
  ];
  for (const [provider, route] of table) {
    assert.equal(classifyLegacyProvider(provider), route, provider);
  }
  for (const provider of ["openai-codex", "codex", "openai_codex"]) {
    // Old agentd reports Codex with the legacy `finite_private` profile.
    const view = inferenceView(legacyStatus(provider, "gpt-5.5"));
    assert.equal(view.saved.route, "openai_codex");
    assert.notEqual(routeLabel(view.saved.route), routeLabel("finite_private"));
  }
});

test("a legacy agent's view uses legacy classification and claims nothing it didn't report", () => {
  const view = inferenceView(legacyStatus("custom", "glm-5-3-flash"));
  assert.deepEqual(view, {
    v2: false,
    saved: { route: "finite_private", provider: "custom", model: "glm-5-3-flash" },
    finitePrivate: { state: "unknown", reason: null },
    openrouter: { state: "unknown", keySource: null, keyHash: null, hermesKey: "unknown", otherPoolKeys: "unknown" },
    codex: null,
    fallback: { state: "unknown", reason: null, model: null, extraEntries: 0 },
    operation: null,
    capabilities: new Set(),
  });
  assert.equal(inferenceView(legacyStatus("openrouter", "openai/gpt-5")).saved.route, "openrouter");
  assert.equal(inferenceView(legacyStatus("anthropic", "claude")).saved.route, "other");
});

test("a v2 agent's view carries the stored facts it reported", () => {
  const view = inferenceView(v2Status());
  assert.equal(view.v2, true);
  assert.deepEqual(view.saved, { route: "openrouter", provider: "openrouter", model: "anthropic/claude-sonnet-4.6" });
  assert.deepEqual(view.finitePrivate, { state: "configured", reason: null });
  assert.deepEqual(view.openrouter, {
    state: "key_saved", keySource: "agent", keyHash: "a".repeat(64), hermesKey: "saved_key", otherPoolKeys: "none",
  });
  assert.deepEqual(view.codex, { state: "not_signed_in", quotaResetAtMs: null, reportedQuotaResetAtMs: null, login: null });
  assert.deepEqual(view.fallback, { state: "configured", reason: null, model: "glm-5-3-flash", extraEntries: 0 });
});

test("v2 fields are ignored without inference.status.v2, and Codex is null without codex.login.v1", () => {
  const withoutV2 = inferenceView(
    v2Status({ inference: { provider: "openai-codex", profile: "finite_private" }, capabilities: ["codex.login.v1"] })
  );
  assert.equal(withoutV2.v2, false);
  assert.equal(withoutV2.saved.route, "openai_codex");
  assert.equal(withoutV2.fallback.state, "unknown");
  assert.equal(withoutV2.openrouter.state, "unknown");
  assert.equal(withoutV2.codex?.state, "unknown");

  const noCodex = inferenceView(v2Status({ capabilities: ["inference.status.v2"] }));
  assert.equal(noCodex.codex, null);
});

test("a v2 agent whose saved block was invalid falls back to legacy classification", () => {
  const view = inferenceView(
    v2Status({ inference: { provider: "openai-codex", profile: "finite_private", saved: { route: "finite_private!" } } })
  );
  assert.equal(view.v2, true);
  assert.equal(view.saved.route, "openai_codex");
});

test("a Codex login and an operation map to camel-case views", () => {
  const view = inferenceView(
    v2Status({
      inference: {
        routes: {
          openai_codex: {
            state: "not_signed_in", quota_reset_at_ms: null, reported_quota_reset_at_ms: 1_790_000_000_000,
            login: {
              attempt_id: `cxl_${"b".repeat(32)}`, state: "pending", user_code: "ABCD-1234",
              verification_uri: "https://auth.openai.com/codex/device", expires_at_ms: 1_790_000_900_000,
              poll_interval_s: 5, error_code: null, retry_after_s: null,
            },
          },
        },
        operation: {
          id: `op_${"c".repeat(32)}`, kind: "select", route: "openrouter", model: "openai/gpt-5",
          state: "running", phase: "restarting", error_code: null, attempts: 1, updated_at_ms: 1_790_000_000_000,
        },
      },
    })
  );
  assert.deepEqual(view.codex, {
    state: "not_signed_in",
    quotaResetAtMs: null,
    reportedQuotaResetAtMs: 1_790_000_000_000,
    login: {
      attemptId: `cxl_${"b".repeat(32)}`, state: "pending", userCode: "ABCD-1234",
      verificationUri: "https://auth.openai.com/codex/device", expiresAtMs: 1_790_000_900_000,
      pollIntervalS: 5, errorCode: null, retryAfterS: null,
    },
  });
  assert.deepEqual(view.operation, {
    id: `op_${"c".repeat(32)}`, kind: "select", route: "openrouter", model: "openai/gpt-5",
    state: "running", phase: "restarting", errorCode: null, attempts: 1, updatedAtMs: 1_790_000_000_000,
  });
});

test("backupConfiguredFor needs a configured backup and that saved route", () => {
  const configured = inferenceView(v2Status());
  assert.equal(backupConfiguredFor(configured, "openrouter"), true);
  assert.equal(backupConfiguredFor(configured, "openai_codex"), false);
  for (const state of ["unavailable", "not_configured", "off", "custom", "unknown"]) {
    const view = inferenceView(v2Status({ inference: { fallback: { state, reason: null, model: null, extra_entries: 0 } } }));
    assert.equal(backupConfiguredFor(view, "openrouter"), false, state);
  }
  assert.equal(backupConfiguredFor(inferenceView(legacyStatus("openrouter", "openai/gpt-5")), "openrouter"), false);
});

test("pollDelayMs polls every 3 s only while visible and something is in flight", () => {
  const operation = (state: string) => ({
    id: `op_${"c".repeat(32)}`, kind: "disconnect", route: "openrouter", model: null,
    state, phase: "cleanup", error_code: null, attempts: 0, updated_at_ms: 1_790_000_000_000,
  });
  const login = (state: string) => ({
    routes: {
      openai_codex: {
        state: "not_signed_in", quota_reset_at_ms: null, reported_quota_reset_at_ms: null,
        login: {
          attempt_id: `cxl_${"b".repeat(32)}`, state, user_code: null, verification_uri: null,
          expires_at_ms: 1_790_000_900_000, poll_interval_s: 5, error_code: null, retry_after_s: null,
        },
      },
    },
  });
  const running = inferenceView(v2Status({ inference: { operation: operation("running") } }));
  assert.equal(pollDelayMs(running, true), 3000);
  assert.equal(pollDelayMs(running, false), null);
  for (const state of ["succeeded", "failed"]) {
    assert.equal(pollDelayMs(inferenceView(v2Status({ inference: { operation: operation(state) } })), true), null);
  }
  for (const state of ["pending", "committing"]) {
    assert.equal(pollDelayMs(inferenceView(v2Status({ inference: login(state) })), true), 3000, state);
  }
  for (const state of ["approved", "canceled", "expired", "failed", "interrupted"]) {
    assert.equal(pollDelayMs(inferenceView(v2Status({ inference: login(state) })), true), null, state);
  }
  assert.equal(pollDelayMs(inferenceView(legacyStatus("custom", "glm-5-3-flash")), true), null);
});

test("routeLabel names each route", () => {
  assert.equal(routeLabel("finite_private"), "Finite Private");
  assert.equal(routeLabel("openrouter"), "OpenRouter");
  assert.equal(routeLabel("openai_codex"), "ChatGPT");
  assert.equal(routeLabel("other"), "Custom model");
});
