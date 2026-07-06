import assert from "node:assert/strict";
import test from "node:test";

import {
  coreAgentCreationRequestForProject,
  coreBridgeStatus,
  coreIdentityHeaders,
  coreProjectLabel,
  coreProjectLaunchStatusLabel,
  coreProjectLocationLabel,
  coreProjectMachineId,
  coreProjectPrimaryUrl,
  type CoreAgentCreationRequestSummary,
  type CoreVisibleProject,
  loadCoreFinitePrivateAdminState,
  loadCoreSourceHostRelayEndpoint,
} from "./core-client";

test("coreBridgeStatus requires both Core URL and service token", () => {
  assert.deepEqual(coreBridgeStatus({}), {
    configured: false,
    missing: ["FC_CORE_BASE_URL", "FC_CORE_API_TOKEN"],
  });
  assert.deepEqual(
    coreBridgeStatus({
      FC_CORE_BASE_URL: "http://127.0.0.1:4200",
      FC_CORE_API_TOKEN: "secret",
    }),
    {
      configured: true,
      missing: [],
    }
  );
});

test("coreIdentityHeaders forwards only verified WorkOS account identity", () => {
  assert.deepEqual(
    coreIdentityHeaders(
      {
        email: "test@finite.vip",
        workosUserId: "user_123",
        emailVerified: true,
        source: "workos",
      },
      "core-token"
    ),
    {
      authorization: "Bearer core-token",
      "content-type": "application/json",
      "x-finite-workos-user-id": "user_123",
      "x-finite-workos-email": "test@finite.vip",
      "x-finite-workos-email-verified": "true",
    }
  );

  assert.throws(
    () =>
      coreIdentityHeaders(
        {
          email: "test@finite.vip",
          workosUserId: null,
          emailVerified: false,
          source: "dev",
        },
        "core-token"
      ),
    /verified WorkOS account/
  );
});

test("core project helpers map imported runtimes to dashboard overview state", () => {
  const project: CoreVisibleProject = {
    project: {
      id: "project_1",
      customer_org_id: "org_1",
      owner_user_id: "user_1",
      display_name: "Smoke",
      import_candidate_id: "import_1",
      created_at: "2026-05-25T12:00:00Z",
      updated_at: "2026-05-25T12:00:00Z",
    },
    runtime: {
      id: "runtime_1",
      project_id: "project_1",
      source_host_id: "smoke",
      source_machine_id: "test-smoke",
      source_import_key: "smoke:test-smoke",
      created_at: "2026-05-25T12:00:00Z",
      updated_at: "2026-05-25T12:00:00Z",
      host_facts: {
        display_name: "Smoke VPS",
        hostname: "smoke.example.com",
        runtime_host: "smoke",
        runtime_status: "online",
        active_inference_profile: "finite-private",
        hermes_available: true,
        published_app_urls: ["notaurl", "https://smoke.example.com/app"],
      },
    },
  };

  assert.equal(coreProjectMachineId(project), "test-smoke");
  assert.equal(coreProjectLabel(project), "Smoke");
  assert.equal(coreProjectPrimaryUrl(project), "https://smoke.example.com/app");
});

test("core project helpers expose self-serve launch status without fake runtime links", () => {
  const project: CoreVisibleProject = {
    project: {
      id: "project_1",
      customer_org_id: "org_1",
      owner_user_id: "user_1",
      display_name: "Oslo Agent",
      import_candidate_id: null,
      created_at: "2026-05-25T12:00:00Z",
      updated_at: "2026-05-25T12:00:00Z",
    },
    runtime: null,
  };
  const request: CoreAgentCreationRequestSummary = {
    id: "agent_creation_request_1",
    project_id: "project_1",
    display_name: "Oslo Agent",
    status: "launching",
    agent_runtime_id: null,
    failure_message: null,
    created_at: "2026-05-25T12:00:00Z",
    updated_at: "2026-05-25T12:01:00Z",
  };

  assert.equal(coreAgentCreationRequestForProject(project, [request]), request);
  assert.equal(coreProjectMachineId(project), null);
  assert.equal(coreProjectLaunchStatusLabel(project, request), "Starting");
  assert.equal(coreProjectLocationLabel(project, request), "Starting your bot");
});

test("loadCoreSourceHostRelayEndpoint reads relay routing through service auth", async () => {
  const previousBaseUrl = process.env.FC_CORE_BASE_URL;
  const previousToken = process.env.FC_CORE_API_TOKEN;
  const previousFetch = globalThis.fetch;
  let requestedUrl: string | null = null;
  let requestedAuth: string | null = null;

  process.env.FC_CORE_BASE_URL = "https://core.example.com";
  process.env.FC_CORE_API_TOKEN = "core-token";
  globalThis.fetch = (async (input, init) => {
    requestedUrl = String(input);
    requestedAuth = new Headers(init?.headers).get("authorization");
    return new Response(
      JSON.stringify({
        source_host_id: "smoke",
        url: "https://relay.smoke.finite.computer",
        admin_token: "smoke-token",
        created_at: "2026-05-25T12:00:00Z",
        updated_at: "2026-05-25T12:00:00Z",
      }),
      { status: 200, headers: { "content-type": "application/json" } }
    );
  }) as typeof fetch;

  try {
    const endpoint = await loadCoreSourceHostRelayEndpoint(" Smoke ");
    assert.equal(
      requestedUrl,
      "https://core.example.com/api/core/v1/source-host-relays/smoke"
    );
    assert.equal(requestedAuth, "Bearer core-token");
    assert.equal(endpoint?.url, "https://relay.smoke.finite.computer");
    assert.equal(endpoint?.admin_token, "smoke-token");
  } finally {
    if (previousBaseUrl === undefined) {
      delete process.env.FC_CORE_BASE_URL;
    } else {
      process.env.FC_CORE_BASE_URL = previousBaseUrl;
    }
    if (previousToken === undefined) {
      delete process.env.FC_CORE_API_TOKEN;
    } else {
      process.env.FC_CORE_API_TOKEN = previousToken;
    }
    globalThis.fetch = previousFetch;
  }
});

test("loadCoreFinitePrivateAdminState reads grant and key status through service auth", async () => {
  const previousBaseUrl = process.env.FC_CORE_BASE_URL;
  const previousToken = process.env.FC_CORE_API_TOKEN;
  const previousFetch = globalThis.fetch;
  let requestedUrl: string | null = null;
  let requestedAuth: string | null = null;

  process.env.FC_CORE_BASE_URL = "https://core.example.com";
  process.env.FC_CORE_API_TOKEN = "core-token";
  globalThis.fetch = (async (input, init) => {
    requestedUrl = String(input);
    requestedAuth = new Headers(init?.headers).get("authorization");
    return new Response(
      JSON.stringify({
        grants: [
          {
            id: "fp_grant_1",
            user_id: "user_1",
            limit_profile_id: "finite-private-generous",
            status: "active",
            current_window_started_at: null,
            current_window_used_units: 42,
            created_at: "2026-05-26T12:00:00Z",
            updated_at: "2026-05-26T12:00:00Z",
          },
        ],
        apiKeys: [
          {
            id: "fp_key_1",
            grant_id: "fp_grant_1",
            project_id: "project_1",
            agent_runtime_id: "runtime_1",
            key_hash: "hash-only",
            status: "active",
            created_at: "2026-05-26T12:00:00Z",
            updated_at: "2026-05-26T12:00:00Z",
          },
        ],
        adminAuditEvents: [],
      }),
      { status: 200, headers: { "content-type": "application/json" } }
    );
  }) as typeof fetch;

  try {
    const result = await loadCoreFinitePrivateAdminState();
    assert.equal(
      requestedUrl,
      "https://core.example.com/api/core/v1/finite-private/admin-state"
    );
    assert.equal(requestedAuth, "Bearer core-token");
    assert.equal(result.state?.grants[0]?.current_window_used_units, 42);
    assert.equal(result.state?.apiKeys[0]?.key_hash, "hash-only");
  } finally {
    if (previousBaseUrl === undefined) {
      delete process.env.FC_CORE_BASE_URL;
    } else {
      process.env.FC_CORE_BASE_URL = previousBaseUrl;
    }
    if (previousToken === undefined) {
      delete process.env.FC_CORE_API_TOKEN;
    } else {
      process.env.FC_CORE_API_TOKEN = previousToken;
    }
    globalThis.fetch = previousFetch;
  }
});
