import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test, { type TestContext } from "node:test";

import {
  HostedAgentControlError,
  agentOwnerClaimCommand,
  checkOpenRouterModel,
  parseAgentConnectionAction,
  parseCodexModels,
  parseConnectionsStatus,
  parseOpenRouterUsage,
  parseSimplexStatus,
  savedOpenRouterModel,
} from "@/lib/hosted-agent-controls";

test("Connections reuses the durable successful owner claim", () => {
  assert.deepEqual(agentOwnerClaimCommand("room-1", "agent-account-1"), {
    room_id: "room-1",
    target_account_id: "agent-account-1",
    command: "agent.owner.claim",
    resource_key: "agent.connections",
    schema: "finite.agent.empty.request.v1",
    body: {},
    reuse_succeeded_owner_claim: true,
    wait_millis: 45_000,
  });
});

test("connection actions expose only the product-scoped command surface", () => {
  assert.deepEqual(parseAgentConnectionAction({ action: "status" }), { action: "status" });
  assert.deepEqual(
    parseAgentConnectionAction({
      action: "inference",
      profile: "openrouter",
      apiKey: "key-value",
      model: "anthropic/claude-sonnet-4.6",
    }),
    {
      action: "inference",
      profile: "openrouter",
      apiKey: "key-value",
      model: "anthropic/claude-sonnet-4.6",
    }
  );
  assert.deepEqual(
    parseAgentConnectionAction({ action: "telegram_approve", code: "ABCD2345" }),
    { action: "telegram_approve", code: "ABCD2345" }
  );
  assert.throws(
    () => parseAgentConnectionAction({ action: "run", command: "rm", args: ["-rf"] }),
    HostedAgentControlError
  );
});

test("connection actions reject unknown inference and oversized secrets", () => {
  assert.throws(
    () => parseAgentConnectionAction({ action: "inference", profile: "anything" }),
    /Choose Finite Private or OpenRouter/u
  );
  assert.throws(
    () =>
      parseAgentConnectionAction({
        action: "telegram_connect",
        token: "x".repeat(257),
      }),
    HostedAgentControlError
  );
});

test("SimpleX exposes pairing actions without arbitrary daemon commands", () => {
  for (const action of ["simplex_connect", "simplex_reset"]) {
    assert.deepEqual(parseAgentConnectionAction({ action }), { action });
  }
  assert.throws(() => parseAgentConnectionAction({ action: "simplex_approve", code: "ABCD2345" }));
  assert.throws(() => parseAgentConnectionAction({ action: "simplex_disconnect" }));
  assert.throws(() => parseAgentConnectionAction({ action: "simplex_command", command: "/sql" }));
});

test("SimpleX rejects executable links and malformed QR matrices", () => {
  const status = { enabled: true, ready: true, address: "https://smp.example/a#synthetic", qr: ["10", "01"], approved: [] };
  assert.equal(parseSimplexStatus(status).address, status.address);
  assert.throws(() => parseSimplexStatus({ ...status, address: "javascript:alert(1)" }));
  assert.throws(() => parseSimplexStatus({ ...status, qr: ["10", "1"] }));
  assert.throws(() => parseSimplexStatus({ ...status, qr: ["<svg>"] }));
});


test("SimpleX pending requests retain exact IDs and reject invalid metadata", () => {
  const request = { request_id: "abcdef0123456789", user_id: "3", name: "Owner", age_minutes: 2 };
  const status = { enabled: true, ready: true, qr: [], approved: [], pending: [request] };
  assert.deepEqual(parseSimplexStatus(status).pending, [request]);
  assert.equal(parseSimplexStatus({ ...status, pending: undefined }).pending, undefined);
  assert.deepEqual(parseAgentConnectionAction({ action: "simplex_approve_request", request_id: request.request_id }), { action: "simplex_approve_request", request_id: request.request_id });
  assert.throws(() => parseAgentConnectionAction({ action: "simplex_approve_request", request_id: "Owner" }));
  assert.throws(() => parseSimplexStatus({ ...status, pending: [{ ...request, request_id: "Owner" }] }));
  assert.throws(() => parseSimplexStatus({ ...status, pending: [{ ...request, age_minutes: -1 }] }));
});

// Golden copies of today's agentd `agent.connections.status` bodies (`connections.rs` `ConnectionsStatus`,
// `simplex.rs` `SimplexStatus`): no `capabilities` and none of the v2 inference keys. A new dashboard
// must parse them exactly as it did before v2.
const LEGACY_FINITE_PRIVATE = {
  inference: { profile: "finite_private", provider: "custom", model: "glm-5-3-flash" },
  telegram: { connected: false, home_channel: null, pending: [], approved: [] },
  google: { connected: false, email: null },
};
const LEGACY_OPENROUTER = {
  inference: { profile: "openrouter", provider: "openrouter", model: "anthropic/claude-sonnet-4.6" },
  telegram: {
    connected: true,
    home_channel: "Owner",
    pending: [{ user_id: "42", name: "Pending person" }],
    approved: [{ user_id: "7", name: "Owner" }],
  },
  google: { connected: true, email: "owner@example.test" },
  simplex: {
    reset_pending: false, enabled: true, ready: true, address: "https://smp.example/a#synthetic",
    qr: ["10", "01"], approved: [],
  },
};
// Old agentd after `/model ... --global` to Codex: it still says `finite_private`.
const LEGACY_CODEX = {
  ...LEGACY_FINITE_PRIVATE,
  inference: { profile: "finite_private", provider: "openai-codex", model: "gpt-5.5" },
};
// Old agentd with no `model` block: its own defaults.
const LEGACY_NO_MODEL = {
  ...LEGACY_FINITE_PRIVATE,
  inference: { profile: "finite_private", provider: "custom", model: "Unknown model" },
};

const ALL_CAPABILITIES = [
  "inference.status.v2", "inference.select.v1", "inference.disconnect.v1",
  "openrouter.connect.v1", "openrouter.usage.v1", "codex.login.v1", "codex.models.v1",
];
const KEY_HASH = "0123456789abcdef".repeat(4);
const OPERATION_ID = `op_${"0a".repeat(16)}`;
const ATTEMPT_ID = `cxl_${"1b".repeat(16)}`;
const V2_INFERENCE = {
  profile: "openrouter",
  provider: "openrouter",
  model: "anthropic/claude-sonnet-4.6",
  saved: { route: "openrouter", provider: "openrouter", model: "anthropic/claude-sonnet-4.6" },
  routes: {
    finite_private: { state: "configured", reason: null },
    openrouter: {
      state: "key_saved", key_source: "agent", key_hash: KEY_HASH, hermes_key: "saved_key", other_pool_keys: "none",
    },
    openai_codex: {
      state: "signed_in", quota_reset_at_ms: null, reported_quota_reset_at_ms: 1_790_000_000_000,
      login: {
        attempt_id: ATTEMPT_ID, state: "approved", user_code: "ABCD-1234",
        verification_uri: "https://auth.openai.com/codex/device", expires_at_ms: 1_790_000_900_000,
        poll_interval_s: 5, error_code: null, retry_after_s: null,
      },
    },
  },
  fallback: { state: "configured", reason: null, model: "glm-5-3-flash", extra_entries: 1 },
  operation: {
    id: OPERATION_ID, kind: "activate", route: "openrouter", model: "anthropic/claude-sonnet-4.6",
    state: "running", phase: "config_written", error_code: null, attempts: 0, updated_at_ms: 1_790_000_000_000,
  },
};
const V2_STATUS = { ...LEGACY_FINITE_PRIVATE, inference: V2_INFERENCE, capabilities: ALL_CAPABILITIES };

test("today's status payloads parse exactly as before v2", () => {
  assert.deepStrictEqual(parseConnectionsStatus(LEGACY_FINITE_PRIVATE), {
    inference: { profile: "finite_private", provider: "custom", model: "glm-5-3-flash" },
    simplex: undefined,
    telegram: { connected: false, home_channel: undefined, pending: [], approved: [] },
    google: { connected: false, email: undefined },
  });
  assert.deepStrictEqual(parseConnectionsStatus(LEGACY_OPENROUTER), {
    inference: { profile: "openrouter", provider: "openrouter", model: "anthropic/claude-sonnet-4.6" },
    simplex: {
      reset_pending: false, enabled: true, ready: true, address: "https://smp.example/a#synthetic",
      qr: ["10", "01"], approved: [], pending: undefined,
    },
    telegram: {
      connected: true,
      home_channel: "Owner",
      pending: [{ user_id: "42", name: "Pending person" }],
      approved: [{ user_id: "7", name: "Owner" }],
    },
    google: { connected: true, email: "owner@example.test" },
  });
  assert.deepStrictEqual(parseConnectionsStatus(LEGACY_CODEX).inference, LEGACY_CODEX.inference);
  assert.deepStrictEqual(parseConnectionsStatus(LEGACY_NO_MODEL).inference, LEGACY_NO_MODEL.inference);
});

test("an unknown legacy profile no longer throws, and legacy bounds still apply", () => {
  const status = { ...LEGACY_FINITE_PRIVATE, inference: { profile: "openai_codex", provider: "openai-codex", model: "gpt-5.5" } };
  assert.equal(parseConnectionsStatus(status).inference.profile, "openai_codex");
  for (const inference of [
    { profile: "finite_private", provider: "x".repeat(129), model: "m" },
    { profile: "finite_private", provider: "custom", model: "x".repeat(257) },
    { profile: "", provider: "custom", model: "m" },
    { profile: "finite_private", provider: 7, model: "m" },
    { profile: "finite_private", provider: "custom" },
  ]) {
    assert.throws(() => parseConnectionsStatus({ ...LEGACY_FINITE_PRIVATE, inference }), HostedAgentControlError);
  }
});

test("a full v2 status parses into the allowlisted fields", () => {
  const status = parseConnectionsStatus({ ...V2_STATUS, extra: "ignored", inference: { ...V2_INFERENCE, extra: "ignored" } });
  assert.deepStrictEqual(status.capabilities, ALL_CAPABILITIES);
  assert.deepStrictEqual(status.inference, V2_INFERENCE);
  assert.equal("extra" in status, false);
});

test("an invalid new field is dropped or degraded, never thrown", () => {
  const cases: Array<[string, unknown, (status: ReturnType<typeof parseConnectionsStatus>) => void]> = [
    ["capabilities not a list", { capabilities: "inference.status.v2" }, (s) => assert.equal(s.capabilities, undefined)],
    ["too many capabilities", { capabilities: Array.from({ length: 33 }, (_, i) => `cap.${i}`) }, (s) => assert.equal(s.capabilities, undefined)],
    ["capability charset", { capabilities: ["inference.status.v2", "<script>"] }, (s) => assert.equal(s.capabilities, undefined)],
    ["saved route", { inference: { ...V2_INFERENCE, saved: { route: "anthropic", provider: "a", model: "b" } } }, (s) => assert.equal(s.inference.saved, undefined)],
    ["saved model too long", { inference: { ...V2_INFERENCE, saved: { route: "openrouter", provider: "openrouter", model: "m".repeat(257) } } }, (s) => assert.deepStrictEqual(s.inference.saved, { route: "openrouter", provider: "openrouter", model: null })],
    ["routes not an object", { inference: { ...V2_INFERENCE, routes: [] } }, (s) => assert.equal(s.inference.routes, undefined)],
    ["tri-state fact", { inference: { ...V2_INFERENCE, routes: { openrouter: { state: "works", key_source: "someone", key_hash: "ABC", hermes_key: 1, other_pool_keys: null } } } }, (s) => assert.deepStrictEqual(s.inference.routes?.openrouter, { state: "unknown", key_source: null, key_hash: null, hermes_key: "unknown", other_pool_keys: "unknown" })],
    ["key hash uppercase", { inference: { ...V2_INFERENCE, routes: { openrouter: { ...V2_INFERENCE.routes.openrouter, key_hash: KEY_HASH.toUpperCase() } } } }, (s) => assert.equal(s.inference.routes?.openrouter?.key_hash, null)],
    ["fp reason", { inference: { ...V2_INFERENCE, routes: { finite_private: { state: "ready", reason: "because" } } } }, (s) => assert.deepStrictEqual(s.inference.routes?.finite_private, { state: "unknown", reason: null })],
    ["codex timestamps", { inference: { ...V2_INFERENCE, routes: { openai_codex: { state: "quota_limited", quota_reset_at_ms: 2 ** 53, reported_quota_reset_at_ms: 1.5, login: null } } } }, (s) => assert.deepStrictEqual(s.inference.routes?.openai_codex, { state: "quota_limited", quota_reset_at_ms: null, reported_quota_reset_at_ms: null, login: null })],
    ["fallback extra entries", { inference: { ...V2_INFERENCE, fallback: { state: "configured", reason: "nope", model: 3, extra_entries: 17 } } }, (s) => assert.deepStrictEqual(s.inference.fallback, { state: "configured", reason: null, model: null, extra_entries: 0 })],
    ["fallback state", { inference: { ...V2_INFERENCE, fallback: { state: "answers", reason: null, model: null, extra_entries: 0 } } }, (s) => assert.equal(s.inference.fallback?.state, "unknown")],
    ["operation id", { inference: { ...V2_INFERENCE, operation: { ...V2_INFERENCE.operation, id: "op_1" } } }, (s) => assert.equal(s.inference.operation, undefined)],
    ["operation phase", { inference: { ...V2_INFERENCE, operation: { ...V2_INFERENCE.operation, phase: "done" } } }, (s) => assert.equal(s.inference.operation, undefined)],
    ["operation attempts", { inference: { ...V2_INFERENCE, operation: { ...V2_INFERENCE.operation, attempts: 11 } } }, (s) => assert.equal(s.inference.operation, undefined)],
    ["operation error code", { inference: { ...V2_INFERENCE, operation: { ...V2_INFERENCE.operation, state: "failed", error_code: "rm -rf" } } }, (s) => assert.equal(s.inference.operation?.error_code, null)],
  ];
  for (const [name, patch, check] of cases) {
    const status = parseConnectionsStatus({ ...V2_STATUS, ...(patch as object) });
    check(status);
    assert.equal(status.inference.model, "anthropic/claude-sonnet-4.6", name);
    assert.equal(status.telegram.connected, false, name);
  }
});

test("Codex login bounds", () => {
  const login = V2_INFERENCE.routes.openai_codex.login;
  const parse = (patch: Record<string, unknown>) =>
    parseConnectionsStatus({
      ...V2_STATUS,
      inference: { ...V2_INFERENCE, routes: { openai_codex: { ...V2_INFERENCE.routes.openai_codex, login: { ...login, ...patch } } } },
    }).inference.routes?.openai_codex?.login;
  assert.deepStrictEqual(parse({}), login);
  for (const patch of [
    { attempt_id: "cxl_short" }, { state: "working" }, { expires_at_ms: 0 }, { poll_interval_s: 0 }, { poll_interval_s: 61 },
  ]) {
    assert.equal(parse(patch), null, JSON.stringify(patch));
  }
  assert.equal(parse({ user_code: "<b>" })?.user_code, null);
  assert.equal(parse({ retry_after_s: 3601 })?.retry_after_s, null);
  assert.equal(parse({ error_code: "surprise" })?.error_code, null);
  assert.equal(parse({ verification_uri: null })?.state, "approved");
  assert.deepStrictEqual(parse({ state: "pending", verification_uri: "https://evil.example/codex/device" }), {
    ...login, state: "failed", user_code: null, verification_uri: null, error_code: "start_failed",
  });
});

test("OpenRouter usage parses by allowlist and never turns a missing number into 0", () => {
  const ok = {
    state: "ok", fetched_at_ms: 1_790_000_000_000, retry_after_s: null, other_pool_keys: "none", hermes_key: "saved_key",
    key: {
      key_hash: KEY_HASH, limit_usd: 50, limit_remaining_usd: 49.5, limit_reset: "never", include_byok_in_limit: false,
      usage_usd: { total: 0.5, daily: 0, weekly: 0.5, monthly: 0.5 }, byok_usage_usd: { total: 0 },
      is_free_tier: false, expires_at: "2026-12-26T17:20:35.900Z", label: "sk-or-v1-abc...xyz",
    },
  };
  const expectedKey: Record<string, unknown> = { ...ok.key };
  delete expectedKey.label;
  assert.deepStrictEqual(parseOpenRouterUsage(ok), { ...ok, key: expectedKey });
  const degraded = parseOpenRouterUsage({
    ...ok,
    key: {
      key_hash: KEY_HASH, limit_usd: "50", limit_remaining_usd: Number.NaN, limit_reset: "yearly",
      usage_usd: { total: "1", daily: Infinity }, byok_usage_usd: null, expires_at: "not a date", is_free_tier: "no",
    },
  });
  assert.deepStrictEqual(degraded.key, { key_hash: KEY_HASH, usage_usd: {}, byok_usage_usd: {}, limit_reset: "unknown" });
  assert.equal(parseOpenRouterUsage({ ...ok, key: { ...ok.key, limit_usd: null } }).key?.limit_usd, null);
  assert.deepStrictEqual(parseOpenRouterUsage({ ...ok, key: { ...ok.key, key_hash: "nope" } }).key, null);
  assert.equal(parseOpenRouterUsage({ ...ok, key: { ...ok.key, key_hash: "nope" } }).state, "unavailable");
  assert.deepStrictEqual(parseOpenRouterUsage({ state: "rate_limited", retry_after_s: 30, key: ok.key }), {
    state: "rate_limited", fetched_at_ms: null, retry_after_s: 30, other_pool_keys: "unknown", hermes_key: "unknown", key: null,
  });
  assert.equal(parseOpenRouterUsage({ state: "exploded" }).state, "unavailable");
  assert.equal(parseOpenRouterUsage(null).state, "unavailable");
});

test("the Codex model list is bounded", () => {
  assert.deepStrictEqual(parseCodexModels({ state: "live", models: ["gpt-5.3-codex", "gpt-5.3-codex", "bad id", 7, "x".repeat(129)] }), {
    state: "live", models: ["gpt-5.3-codex"], reason: null,
  });
  assert.equal(parseCodexModels({ state: "live", models: Array.from({ length: 250 }, (_, i) => `m${i}`) }).models.length, 200);
  assert.deepStrictEqual(parseCodexModels({ state: "unavailable", reason: "not_signed_in" }), {
    state: "unavailable", models: [], reason: "not_signed_in",
  });
  assert.deepStrictEqual(parseCodexModels({ state: "live" }), { state: "unavailable", models: [], reason: null });
});

test("new actions parse with their bounds", () => {
  const cases: Array<[unknown, unknown]> = [
    [{ action: "inference_select", route: "finite_private" }, { action: "inference_select", route: "finite_private" }],
    [{ action: "inference_select", route: "finite_private", model: null }, { action: "inference_select", route: "finite_private" }],
    [{ action: "inference_select", route: "openrouter", model: "openai/gpt-5:free" }, { action: "inference_select", route: "openrouter", model: "openai/gpt-5:free" }],
    [{ action: "inference_select", route: "openai_codex", model: "gpt-5.3-codex" }, { action: "inference_select", route: "openai_codex", model: "gpt-5.3-codex" }],
    [{ action: "inference_disconnect", route: "openrouter" }, { action: "inference_disconnect", route: "openrouter" }],
    [{ action: "inference_disconnect", route: "openai_codex" }, { action: "inference_disconnect", route: "openai_codex" }],
    [{ action: "openrouter_connect_key", apiKey: "sk-or-v1-fake" }, { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake" }],
    [{ action: "openrouter_connect_key", apiKey: "sk-or-v1-fake", activate: null }, { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake" }],
    [{ action: "openrouter_connect_key", apiKey: "sk-or-v1-fake", activate: { model: "openai/gpt-5", extra: 1 } }, { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake", activate: { model: "openai/gpt-5" } }],
    [{ action: "codex_login_start" }, { action: "codex_login_start" }],
    [{ action: "codex_login_cancel", attemptId: ATTEMPT_ID }, { action: "codex_login_cancel", attemptId: ATTEMPT_ID }],
  ];
  for (const [payload, expected] of cases) {
    assert.deepStrictEqual(parseAgentConnectionAction(payload), expected);
  }
  for (const payload of [
    { action: "inference_select", route: "other" },
    { action: "inference_select", route: "finite_private", model: "glm-5-3-flash" },
    { action: "inference_select", route: "openrouter" },
    { action: "inference_select", route: "openrouter", model: "has space" },
    { action: "inference_select", route: "openrouter", model: "tab\there" },
    { action: "inference_select", route: "openrouter", model: "m".repeat(257) },
    { action: "inference_select", route: "openai_codex", model: "gpt/5" },
    { action: "inference_select", route: "openai_codex", model: "m".repeat(129) },
    { action: "inference_disconnect", route: "finite_private" },
    { action: "openrouter_connect_key" },
    { action: "openrouter_connect_key", apiKey: "k".repeat(16 * 1024 + 1) },
    { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake", activate: {} },
    { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake", activate: "openai/gpt-5" },
    { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake", activate: { model: "two words" } },
    { action: "codex_login_cancel", attemptId: "cxl_123" },
    { action: "codex_login_cancel" },
  ]) {
    assert.throws(() => parseAgentConnectionAction(payload), HostedAgentControlError, JSON.stringify(payload).slice(0, 80));
  }
});

test("a rejected action never repeats the key it carried", () => {
  const apiKey = "sk-or-v1-FAKE-W1-PARSE-CANARY";
  for (const payload of [
    { action: "openrouter_connect_key", apiKey, activate: { model: "two words" } },
    { action: "openrouter_connect_key", apiKey: `${apiKey}${"k".repeat(16 * 1024)}` },
    { action: "inference", profile: "anything", apiKey },
  ]) {
    assert.throws(
      () => parseAgentConnectionAction(payload),
      (error: unknown) => error instanceof HostedAgentControlError && !error.message.includes(apiKey)
    );
  }
});

test("the OpenRouter model policy reads the saved model only when it decides the outcome", async () => {
  let reads = 0;
  const saved = (model: string | null) => async () => {
    reads += 1;
    return model;
  };
  const catalog = new Set(["openai/gpt-5"]);
  assert.deepStrictEqual(await checkOpenRouterModel("openai/gpt-5", catalog, saved("x")), { catalogChecked: true });
  assert.equal(reads, 0);
  assert.deepStrictEqual(await checkOpenRouterModel("retired/model", catalog, saved("retired/model")), { catalogChecked: true });
  assert.equal(reads, 1);
  await assert.rejects(checkOpenRouterModel("retired/model", catalog, saved("openai/gpt-5")), (error: unknown) =>
    error instanceof HostedAgentControlError &&
    error.status === 400 &&
    error.message === "That model isn't available on OpenRouter for agents. Pick one from the list."
  );
  reads = 0;
  assert.deepStrictEqual(await checkOpenRouterModel("anything/new", null, saved(null)), { catalogChecked: false });
  assert.equal(reads, 0);
});

test("the saved OpenRouter model comes from v2 saved, else the legacy profile", () => {
  assert.equal(savedOpenRouterModel(parseConnectionsStatus(V2_STATUS)), "anthropic/claude-sonnet-4.6");
  const fpSaved = { ...V2_STATUS, inference: { ...V2_INFERENCE, saved: { route: "finite_private", provider: "custom", model: "glm-5-3-flash" } } };
  assert.equal(savedOpenRouterModel(parseConnectionsStatus(fpSaved)), null);
  assert.equal(savedOpenRouterModel(parseConnectionsStatus(LEGACY_OPENROUTER)), "anthropic/claude-sonnet-4.6");
  assert.equal(savedOpenRouterModel(parseConnectionsStatus(LEGACY_FINITE_PRIVATE)), null);
  // v2 fields are trusted only with the capability.
  assert.equal(savedOpenRouterModel(parseConnectionsStatus({ ...fpSaved, capabilities: [] })), "anthropic/claude-sonnet-4.6");
});

// --- Route harness ---------------------------------------------------------------------------------
// Drives the real route handlers through the real session and machine-access code: a dev account, a
// fake Core that lists only the owner's machine, and a fake hosted web device that answers runtime
// commands.

const MY_MACHINE = "runtime-mine";
const OTHER_MACHINE = "runtime-other";

type RuntimeReply = { status: "succeeded" | "failed"; body?: unknown; error?: { code: string; message: string } };
type SentCommand = { command: string; body: unknown };
type World = {
  commands: SentCommand[];
  fetches: Array<{ url: string; body: string }>;
  logs: string[];
};

function installWorld(
  t: TestContext,
  options: {
    status?: unknown;
    signedIn?: boolean;
    runtime?: (command: string, body: unknown) => RuntimeReply | Response | Promise<RuntimeReply | Response>;
  } = {}
): World {
  const scratch = mkdtempSync(path.join(tmpdir(), "w1-connections-"));
  const env: Record<string, string> = {
    FC_WORKOS_AUTH_ENABLED: options.signedIn === false ? "1" : "",
    FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: options.signedIn === false ? "" : "1",
    FC_DASHBOARD_DEV_EMAIL: "owner@example.test",
    FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_owner",
    FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "fake-access-token",
    FC_CORE_BASE_URL: "https://core.test",
    FC_HOSTED_WEB_DEVICE_URL: "https://hwd.test",
    FINITECHAT_HOSTED_API_TOKEN: "fake-hwd-token",
    // No local control plane: the viewer model falls back to empty, and no `finited` is spawned.
    FC_REPO_ROOT: scratch,
    FC_WORKSPACE_ROOT: scratch,
    FC_FINITED_BIN: "",
    PATH: scratch,
  };
  const previous = Object.fromEntries(Object.keys(env).map((name) => [name, process.env[name]]));
  Object.assign(process.env, env);
  t.after(() => {
    for (const [name, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  });

  const world: World = { commands: [], fetches: [], logs: [] };
  for (const method of ["log", "info", "warn", "error", "debug"] as const) {
    t.mock.method(console, method, (...args: unknown[]) => {
      world.logs.push(args.map((arg) => (arg instanceof Error ? `${arg.message} ${arg.stack}` : JSON.stringify(arg))).join(" "));
    });
  }
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init: RequestInit = {}) => {
    const url = new URL(String(input instanceof Request ? input.url : input));
    const body = typeof init.body === "string" ? init.body : "";
    world.fetches.push({ url: url.toString(), body });
    if (url.origin === "https://core.test") {
      if (url.pathname === "/api/core/v1/me") {
        return Response.json({
          workos_user_id: "user_owner",
          email: "owner@example.test",
          projects: [
            { project: { id: "project-mine", display_name: "Mine" }, runtime: { id: MY_MACHINE, contact_endpoint: null } },
          ],
        });
      }
      return Response.json({ error: "not found" }, { status: 404 });
    }
    if (url.origin === "https://hwd.test") {
      if (url.pathname === "/v1/app/agent-bindings/open") {
        assert.deepStrictEqual(JSON.parse(body), { project_id: "project-mine" });
        return Response.json({ hosted_agent_binding: { canonical_room_id: "room-mine", agent_account_id: "agent-mine" } });
      }
      if (url.pathname === "/v1/app/runtime-commands") {
        const request = JSON.parse(body) as { command: string; body: unknown };
        world.commands.push({ command: request.command, body: request.body });
        if (request.command === "agent.owner.claim") {
          return Response.json({ request_id: "r", status: "succeeded", body: { connected: true } });
        }
        if (request.command === "agent.connections.status") {
          return Response.json({ request_id: "r", status: "succeeded", body: options.status ?? V2_STATUS });
        }
        const reply = options.runtime
          ? await options.runtime(request.command, request.body)
          : { status: "succeeded" as const, body: { changed: false } };
        return reply instanceof Response ? reply : Response.json({ request_id: "r", ...reply });
      }
    }
    throw new Error(`unexpected fetch ${url}`);
  });
  return world;
}

const routeContext = (machineId: string) => ({ params: Promise.resolve({ machineId }) });

async function postAction(machineId: string, payload: unknown) {
  const { POST } = await import("../app/api/connections/machines/[machineId]/route");
  return POST(
    new Request(`https://dashboard.test/api/connections/machines/${machineId}`, {
      method: "POST",
      body: JSON.stringify(payload),
    }),
    routeContext(machineId)
  );
}

async function getUsage(machineId: string) {
  const { GET } = await import("../app/api/connections/machines/[machineId]/openrouter/usage/route");
  return GET(new Request(`https://dashboard.test/api/connections/machines/${machineId}/openrouter/usage`), routeContext(machineId));
}

async function getCodexModels(machineId: string) {
  const { GET } = await import("../app/api/connections/machines/[machineId]/codex/models/route");
  return GET(new Request(`https://dashboard.test/api/connections/machines/${machineId}/codex/models`), routeContext(machineId));
}

const commandNames = (world: World) => world.commands.map((entry) => entry.command);

test("new-action flow: claim, status gate, command, status; the reply comes back allowlisted", async (t) => {
  const world = installWorld(t, {
    runtime: () => ({ status: "succeeded", body: { accepted: true, operation_id: OPERATION_ID, extra: "dropped" } }),
  });
  const response = await postAction(MY_MACHINE, { action: "inference_select", route: "finite_private" });
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("cache-control"), "no-store");
  const payload = await response.json();
  assert.deepStrictEqual(payload.result, { accepted: true, operation_id: OPERATION_ID });
  assert.equal(payload.status.inference.operation.id, OPERATION_ID);
  assert.equal("catalog_checked" in payload, false);
  assert.deepStrictEqual(commandNames(world), [
    "agent.owner.claim", "agent.connections.status", "agent.inference.select", "agent.connections.status",
  ]);
  assert.deepStrictEqual(world.commands[2].body, { route: "finite_private", model: null });
});

test("new actions send the command bodies", async (t) => {
  const cases: Array<[unknown, string, unknown, unknown]> = [
    [{ action: "inference_select", route: "openrouter", model: "openai/gpt-5" }, "agent.inference.select", { route: "openrouter", model: "openai/gpt-5" }, { changed: false }],
    [{ action: "inference_select", route: "openai_codex", model: "gpt-5.3-codex" }, "agent.inference.select", { route: "openai_codex", model: "gpt-5.3-codex" }, { changed: false }],
    [{ action: "inference_disconnect", route: "openrouter" }, "agent.inference.disconnect", { route: "openrouter" }, { changed: false }],
    [{ action: "openrouter_connect_key", apiKey: "sk-or-v1-fake-body" }, "agent.openrouter.connect", { credential: { kind: "api_key", api_key: "sk-or-v1-fake-body" }, activate: null }, { changed: true, activated: false }],
    [{ action: "openrouter_connect_key", apiKey: "sk-or-v1-fake-body", activate: { model: "openai/gpt-5" } }, "agent.openrouter.connect", { credential: { kind: "api_key", api_key: "sk-or-v1-fake-body" }, activate: { model: "openai/gpt-5" } }, { accepted: true, operation_id: OPERATION_ID }],
    [{ action: "codex_login_start" }, "agent.codex.login.start", {}, V2_INFERENCE.routes.openai_codex.login],
    [{ action: "codex_login_cancel", attemptId: ATTEMPT_ID }, "agent.codex.login.cancel", { attempt_id: ATTEMPT_ID }, V2_INFERENCE.routes.openai_codex.login],
  ];
  for (const [payload, command, body, reply] of cases) {
    await t.test(command, async (t) => {
      const world = installWorld(t, { runtime: () => ({ status: "succeeded", body: reply }) });
      const response = await postAction(MY_MACHINE, payload);
      assert.equal(response.status, 200);
      assert.deepStrictEqual(world.commands[2], { command, body });
      assert.deepStrictEqual((await response.json()).result, reply);
    });
  }
});

test("a storage-only connect reply is not confused with a select reply, and an unknown reply is null", async (t) => {
  const world = installWorld(t, { runtime: () => ({ status: "succeeded", body: { changed: true, activated: false } }) });
  const select = await postAction(MY_MACHINE, { action: "inference_select", route: "finite_private" });
  assert.equal((await select.json()).result, null);
  const connect = await postAction(MY_MACHINE, { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake" });
  assert.deepStrictEqual((await connect.json()).result, { changed: true, activated: false });
  assert.equal(world.commands.filter((entry) => entry.command === "agent.openrouter.connect").length, 1);
});

test("a legacy agent keeps today's flow and response for the legacy inference action", async (t) => {
  const world = installWorld(t, { status: LEGACY_OPENROUTER, runtime: () => ({ status: "succeeded", body: { applied: true } }) });
  const response = await postAction(MY_MACHINE, {
    action: "inference", profile: "openrouter", apiKey: "sk-or-v1-fake-legacy", model: "openai/gpt-5",
  });
  assert.equal(response.status, 200);
  assert.deepStrictEqual(await response.json(), JSON.parse(JSON.stringify(parseConnectionsStatus(LEGACY_OPENROUTER))));
  assert.deepStrictEqual(commandNames(world), ["agent.owner.claim", "agent.inference.apply", "agent.connections.status"]);
  assert.deepStrictEqual(world.commands[1].body, { profile: "openrouter", api_key: "sk-or-v1-fake-legacy", model: "openai/gpt-5" });
});

test("new actions are gated on advertised capabilities and never reach an agent that lacks them", async (t) => {
  const cases: Array<[string, unknown, unknown]> = [
    ["legacy agent, select", LEGACY_FINITE_PRIVATE, { action: "inference_select", route: "finite_private" }],
    ["legacy agent, connect", LEGACY_FINITE_PRIVATE, { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake" }],
    ["legacy agent, disconnect", LEGACY_CODEX, { action: "inference_disconnect", route: "openrouter" }],
    ["legacy agent, codex login", LEGACY_CODEX, { action: "codex_login_start" }],
    ["PR1 agent, Codex select", { ...V2_STATUS, capabilities: ["inference.status.v2", "inference.select.v1", "inference.disconnect.v1"] }, { action: "inference_select", route: "openai_codex", model: "gpt-5.3-codex" }],
    ["PR1 agent, Codex disconnect", { ...V2_STATUS, capabilities: ["inference.status.v2", "inference.select.v1", "inference.disconnect.v1"] }, { action: "inference_disconnect", route: "openai_codex" }],
    ["PR1 agent, connect", { ...V2_STATUS, capabilities: ["inference.status.v2", "inference.select.v1", "inference.disconnect.v1"] }, { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake" }],
  ];
  for (const [name, status, payload] of cases) {
    await t.test(name, async (t) => {
      const world = installWorld(t, { status });
      const response = await postAction(MY_MACHINE, payload);
      assert.equal(response.status, 409);
      assert.deepStrictEqual(await response.json(), { error: "This agent needs an update for this.", code: "agent_update_required" });
      assert.deepStrictEqual(commandNames(world), ["agent.owner.claim", "agent.connections.status"]);
    });
  }
});

test("unsupported_command maps to 409 agent_update_required", async (t) => {
  installWorld(t, {
    runtime: () => ({ status: "failed", error: { code: "unsupported_command", message: 'Command "agent.inference.select" is not supported.' } }),
  });
  const response = await postAction(MY_MACHINE, { action: "inference_select", route: "finite_private" });
  assert.equal(response.status, 409);
  assert.deepStrictEqual(await response.json(), { error: "This agent needs an update for this.", code: "agent_update_required" });
  // The legacy action gets the same backstop.
  const legacy = await postAction(MY_MACHINE, { action: "simplex_connect" });
  assert.equal(legacy.status, 409);
});

test("agentd error codes pass through with agentd's message", async (t) => {
  for (const code of [
    "credential_rejected", "key_allowance_exhausted", "activation_not_recorded", "disconnect_in_progress",
    "finite_private_unavailable", "operation_in_progress", "not_connected", "provider_unavailable", "config_invalid",
  ]) {
    await t.test(code, async (t) => {
      const message = `Fixed agentd copy for ${code}.`;
      installWorld(t, { runtime: () => ({ status: "failed", error: { code, message } }) });
      const response = await postAction(MY_MACHINE, { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake", activate: { model: "openai/gpt-5" } });
      assert.equal(response.status, 502);
      assert.equal(response.headers.get("cache-control"), "no-store");
      assert.deepStrictEqual(await response.json(), { error: message, code });
    });
  }
  await t.test("unauthorized stays 403", async (t) => {
    installWorld(t, { runtime: () => ({ status: "failed", error: { code: "unauthorized", message: "Not the owner." } }) });
    const response = await postAction(MY_MACHINE, { action: "inference_disconnect", route: "openrouter" });
    assert.equal(response.status, 403);
    assert.equal((await response.json()).code, "unauthorized");
  });
  await t.test("a malformed code is dropped", async (t) => {
    installWorld(t, { runtime: () => ({ status: "failed", error: { code: "<b>Bad</b>", message: "x" } }) });
    const response = await postAction(MY_MACHINE, { action: "inference_disconnect", route: "openrouter" });
    assert.deepStrictEqual(await response.json(), { error: "x", code: null });
  });
});

test("a code the dashboard has no entry for, like facts_unavailable, passes through with agentd's message", async (t) => {
  for (const [code, message] of [
    ["facts_unavailable", "The agent couldn't check its setup right now. Try again in a moment."],
    ["some_future_code", "A later agentd's own copy."],
  ]) {
    await t.test(code, async (t) => {
      installWorld(t, { runtime: () => ({ status: "failed", error: { code, message } }) });
      const response = await postAction(MY_MACHINE, { action: "inference_disconnect", route: "openrouter" });
      assert.equal(response.status, 502, "the same status as every other agentd domain error");
      assert.equal(response.headers.get("cache-control"), "no-store");
      assert.deepStrictEqual(await response.json(), { error: message, code });
    });
  }
});

test("every OpenRouter model save goes through the model policy, without an extra status read in PR1", async (t) => {
  const world = installWorld(t, { runtime: () => ({ status: "succeeded", body: { changed: false } }) });
  const select = await (await postAction(MY_MACHINE, { action: "inference_select", route: "openrouter", model: "new/model" })).json();
  assert.equal(select.catalog_checked, false);
  const activate = await (await postAction(MY_MACHINE, {
    action: "openrouter_connect_key", apiKey: "sk-or-v1-fake", activate: { model: "new/model" },
  })).json();
  assert.equal(activate.catalog_checked, false);
  const storageOnly = await (await postAction(MY_MACHINE, { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake" })).json();
  assert.equal("catalog_checked" in storageOnly, false);
  world.commands.length = 0;
  await postAction(MY_MACHINE, { action: "inference", profile: "openrouter", model: "new/model" });
  assert.deepStrictEqual(commandNames(world), ["agent.owner.claim", "agent.inference.apply", "agent.connections.status"]);
  world.commands.length = 0;
  const invalid = await postAction(MY_MACHINE, { action: "inference", profile: "openrouter", model: "two words" });
  assert.equal(invalid.status, 400);
  assert.deepStrictEqual(commandNames(world), []);
});

test("the usage and models routes return parsed data with no-store", async (t) => {
  const usage = {
    state: "no_key", fetched_at_ms: 1_790_000_000_000, retry_after_s: null, other_pool_keys: "none", hermes_key: "none", key: null,
  };
  const world = installWorld(t, {
    runtime: (command) =>
      command === "agent.openrouter.usage"
        ? { status: "succeeded", body: usage }
        : { status: "succeeded", body: { state: "live", models: ["gpt-5.3-codex"] } },
  });
  const usageResponse = await getUsage(MY_MACHINE);
  assert.equal(usageResponse.status, 200);
  assert.equal(usageResponse.headers.get("cache-control"), "no-store");
  assert.deepStrictEqual(await usageResponse.json(), usage);
  const modelsResponse = await getCodexModels(MY_MACHINE);
  assert.equal(modelsResponse.status, 200);
  assert.equal(modelsResponse.headers.get("cache-control"), "no-store");
  assert.deepStrictEqual(await modelsResponse.json(), { state: "live", models: ["gpt-5.3-codex"], reason: null });
  assert.deepStrictEqual(commandNames(world), [
    "agent.owner.claim", "agent.connections.status", "agent.openrouter.usage",
    "agent.owner.claim", "agent.connections.status", "agent.codex.models",
  ]);
  for (const entry of world.commands.filter((command) => command.command !== "agent.owner.claim")) {
    assert.deepStrictEqual(entry.body, {});
  }
});

test("without the capability the usage and models routes return agent_update_required, not a 500", async (t) => {
  for (const status of [LEGACY_FINITE_PRIVATE, { ...V2_STATUS, capabilities: ["inference.status.v2", "inference.select.v1"] }]) {
    const world = installWorld(t, { status });
    for (const load of [getUsage, getCodexModels]) {
      const response = await load(MY_MACHINE);
      assert.equal(response.status, 409);
      assert.equal(response.headers.get("cache-control"), "no-store");
      assert.deepStrictEqual(await response.json(), { error: "This agent needs an update for this.", code: "agent_update_required" });
    }
    assert.deepStrictEqual(commandNames(world).filter((name) => name !== "agent.owner.claim"), [
      "agent.connections.status", "agent.connections.status",
    ]);
  }
});

test("runtime errors on the read routes use the {error, code} shape", async (t) => {
  installWorld(t, { runtime: () => ({ status: "failed", error: { code: "provider_unavailable", message: "Couldn't reach OpenRouter." } }) });
  for (const load of [getUsage, getCodexModels]) {
    const response = await load(MY_MACHINE);
    assert.equal(response.status, 502);
    assert.deepStrictEqual(await response.json(), { error: "Couldn't reach OpenRouter.", code: "provider_unavailable" });
  }
});

test("authorization: no session reaches no agent through any Connections route", async (t) => {
  const world = installWorld(t, { signedIn: false });
  for (const response of [
    await getUsage(MY_MACHINE),
    await getCodexModels(MY_MACHINE),
    await postAction(MY_MACHINE, { action: "inference_select", route: "finite_private" }),
  ]) {
    assert.equal(response.status, 401);
    assert.equal(response.headers.get("cache-control"), "no-store");
    assert.deepStrictEqual(await response.json(), { error: "Sign in again to manage this agent.", code: null });
  }
  assert.deepStrictEqual(world.fetches, []);
});

test("authorization: another account's machine is unreachable through the new routes", async (t) => {
  const world = installWorld(t);
  for (const response of [
    await getUsage(OTHER_MACHINE),
    await getCodexModels(OTHER_MACHINE),
    await postAction(OTHER_MACHINE, { action: "inference_select", route: "finite_private" }),
  ]) {
    assert.equal(response.status, 404);
    assert.deepStrictEqual(await response.json(), { error: "Agent not found.", code: null });
  }
  assert.equal(world.fetches.some((entry) => entry.url.startsWith("https://hwd.test")), false);
  assert.deepStrictEqual(world.commands, []);
  // The same session reaches its own machine.
  assert.equal((await getUsage(MY_MACHINE)).status, 200);
});

test("secrets: a pasted key reaches only the runtime command, on success and every failure path", async (t) => {
  const apiKey = "sk-or-v1-FAKE-W1-SECRET-CANARY-0123456789abcdef";
  const echo = `upstream said ${apiKey} is bad`;
  const runtimes: Array<[string, (command: string) => RuntimeReply | Response]> = [
    ["success", () => ({ status: "succeeded", body: { accepted: true, operation_id: OPERATION_ID } })],
    ["agentd error", () => ({ status: "failed", error: { code: "credential_rejected", message: "OpenRouter didn't accept this key." } })],
    ["agentd error echoing the key", () => ({ status: "failed", error: { code: "credential_rejected", message: echo } })],
    ["hosted device HTTP error echoing the key", () => Response.json({ error: echo }, { status: 500 })],
    ["hosted device transport error echoing the key", () => {
      throw new Error(echo);
    }],
    ["reply echoing the key", () => ({ status: "succeeded", body: { changed: true, activated: false, api_key: apiKey } })],
  ];
  const actions = [
    { action: "openrouter_connect_key", apiKey },
    { action: "openrouter_connect_key", apiKey, activate: { model: "openai/gpt-5" } },
    { action: "inference", profile: "openrouter", apiKey, model: "openai/gpt-5" },
  ];
  for (const [name, runtime] of runtimes) {
    for (const payload of actions) {
      await t.test(`${name}: ${payload.action}${"activate" in payload ? " + activate" : ""}`, async (t) => {
        const world = installWorld(t, { runtime });
        const response = await postAction(MY_MACHINE, payload);
        const text = await response.text();
        assert.equal(text.includes(apiKey), false, "response body");
        assert.equal([...response.headers.values()].join(" ").includes(apiKey), false, "response headers");
        assert.equal(world.logs.join("\n").includes(apiKey), false, "logs");
        const carriers = world.fetches.filter((entry) => entry.url.includes(apiKey) || entry.body.includes(apiKey));
        assert.equal(carriers.length, 1, "exactly one request carries the key");
        assert.equal(carriers[0].url, "https://hwd.test/v1/app/runtime-commands");
        assert.equal(carriers[0].url.includes(apiKey), false);
        const sent = JSON.parse(carriers[0].body) as { command: string };
        assert.ok(["agent.openrouter.connect", "agent.inference.apply"].includes(sent.command));
      });
    }
  }
  await t.test("rejected before dispatch", async (t) => {
    const world = installWorld(t, { status: LEGACY_FINITE_PRIVATE });
    for (const payload of [
      { action: "openrouter_connect_key", apiKey },
      { action: "openrouter_connect_key", apiKey, activate: { model: "two words" } },
      { action: "inference", profile: "openrouter", apiKey, model: "two words" },
    ]) {
      const response = await postAction(MY_MACHINE, payload);
      assert.ok(response.status >= 400);
      assert.equal((await response.text()).includes(apiKey), false);
    }
    assert.equal(world.fetches.some((entry) => entry.body.includes(apiKey) || entry.url.includes(apiKey)), false);
    assert.equal(world.logs.join("\n").includes(apiKey), false);
  });
});
