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
  loadCoreSourceHostRelayEndpoint,
} from "./core-client";

test("coreBridgeStatus requires the Core URL but not a service token for user routes", () => {
  assert.deepEqual(coreBridgeStatus({}), {
    configured: false,
    missing: ["FC_CORE_BASE_URL"],
  });
  assert.deepEqual(
    coreBridgeStatus({
      FC_CORE_BASE_URL: "http://127.0.0.1:4200",
    }),
    {
      configured: true,
      missing: [],
    }
  );
});

test("coreIdentityHeaders forwards the AuthKit bearer and no caller-supplied identity", () => {
  assert.deepEqual(
    coreIdentityHeaders(
      {
        email: "test@finite.vip",
        workosUserId: "user_123",
        emailVerified: true,
        accessToken: "authkit-access-token",
        source: "workos",
      }
    ),
    {
      authorization: "Bearer authkit-access-token",
      "content-type": "application/json",
    }
  );

  assert.throws(
    () =>
      coreIdentityHeaders(
        {
          email: "test@finite.vip",
          workosUserId: "user_123",
          emailVerified: true,
          source: "workos",
        }
      ),
    /Sign in again/
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
