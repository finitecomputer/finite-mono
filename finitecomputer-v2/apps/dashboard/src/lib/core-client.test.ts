import assert from "node:assert/strict";
import test from "node:test";

import {
  coreAgentCreationRequestForProject,
  coreAgentCreationRequestBody,
  coreBridgeStatus,
  coreIdentityHeaders,
  coreLaunchCodeBatchRequestBody,
  coreProjectLabel,
  coreProjectLaunchStatusLabel,
  coreProjectLocationLabel,
  coreProjectPrimaryUrl,
  coreProjectRuntimeId,
  coreProjectSupportsRetirement,
  coreProductProjectForLegacyMachineId,
  coreProductProjectForRouteId,
  coreProductProjects,
  type CoreAgentCreationRequestSummary,
  type CoreVisibleProject,
  loadCoreSourceHostRelayEndpoint,
} from "./core-client";

test("Launch Code issuance requests default to Standard and carry explicit Confidential", () => {
  assert.deepEqual(
    coreLaunchCodeBatchRequestBody({
      name: "Default batch",
      codeCount: 1,
      expiresInHours: 24,
    }),
    {
      name: "Default batch",
      codeCount: 1,
      expiresInHours: 24,
      hostingTier: "standard",
    }
  );
  assert.deepEqual(
    coreLaunchCodeBatchRequestBody({
      name: "Confidential batch",
      codeCount: 2,
      expiresInHours: 48,
      hostingTier: "confidential",
    }),
    {
      name: "Confidential batch",
      codeCount: 2,
      expiresInHours: 48,
      hostingTier: "confidential",
    }
  );
});

test("agent creation payload cannot submit provider placement", () => {
  const staleInput = {
    displayName: "Moss",
    launchCode: "launch-fixture",
    idempotencyKey: "request-fixture",
    profilePictureUrl: "https://chat.example/profile.png",
    runnerClass: "phala",
  };
  const body = coreAgentCreationRequestBody(staleInput);
  assert.deepEqual(body, {
    displayName: "Moss",
    launchCode: "launch-fixture",
    idempotencyKey: "request-fixture",
    profilePictureUrl: "https://chat.example/profile.png",
  });
  assert.equal("runnerClass" in body, false);
  assert.deepEqual(
    coreAgentCreationRequestBody({
      displayName: "Moss",
      launchCode: "",
      idempotencyKey: "request-without-picture",
    }),
    {
      displayName: "Moss",
      launchCode: "",
      idempotencyKey: "request-without-picture",
    }
  );
});

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

test("core project helpers use stable runtime identity and normalized contact", () => {
  const project: CoreVisibleProject = {
    project: {
      id: "project_1",
      display_name: "Smoke",
      created_at: "2026-05-25T12:00:00Z",
      updated_at: "2026-05-25T12:00:00Z",
    },
    runtime: {
      id: "runtime_1",
      project_id: "project_1",
      contact_endpoint: "https://smoke.example.com/contact",
      runtime_status: "online",
      hermes_available: true,
      created_at: "2026-05-25T12:00:00Z",
      updated_at: "2026-05-25T12:00:00Z",
    },
  };

  assert.equal(coreProjectRuntimeId(project), "runtime_1");
  assert.equal(coreProjectLabel(project), "Smoke");
  assert.equal(coreProjectPrimaryUrl(project), "https://smoke.example.com/contact");
  assert.equal(coreProjectLocationLabel(project, null), "Ready to use");
});

test("route helpers prefer stable ids and isolate N-1 legacy alias reads", () => {
  const first = {
    project: {
      id: "project_first",
      import_candidate_id: null,
    },
    runtime: {
      id: "runtime_first",
      source_machine_id: "legacy-first",
    },
  } as CoreVisibleProject;
  const second = {
    project: {
      id: "project_second",
      import_candidate_id: null,
    },
    runtime: {
      id: "runtime_second",
    },
  } as CoreVisibleProject;

  assert.deepEqual(coreProductProjects([first, second]), [first, second]);
  assert.equal(
    coreProductProjectForRouteId([first, second], "runtime_second"),
    second
  );
  assert.equal(
    coreProductProjectForRouteId([first, second], "project_first"),
    first
  );
  assert.equal(
    coreProductProjectForLegacyMachineId([first, second], "legacy-first"),
    first
  );

  const imported = {
    project: { id: "project_imported", import_candidate_id: "import_1" },
    runtime: { id: "runtime_imported", source_machine_id: "legacy-import" },
  } as CoreVisibleProject;
  assert.deepEqual(coreProductProjects([imported, first]), [first]);
  assert.equal(
    coreProductProjectForLegacyMachineId([imported, first], "legacy-import"),
    null
  );
});

test("runtime retirement requires an explicit provider-neutral capability", () => {
  const project = {
    project: { id: "project_1" },
    runtime: { id: "runtime_1" },
  } as CoreVisibleProject;
  assert.equal(coreProjectSupportsRetirement(project), false);

  const advertised = {
    ...project,
    runtime: {
      ...project.runtime,
      runtime_capabilities: { runtime_retirement: true },
    },
  } as CoreVisibleProject;
  assert.equal(coreProjectSupportsRetirement(advertised), true);
});

test("core project helpers expose self-serve launch status without fake runtime links", () => {
  const project: CoreVisibleProject = {
    project: {
      id: "project_1",
      display_name: "Oslo Agent",
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
  assert.equal(coreProjectRuntimeId(project), null);
  assert.equal(coreProjectLaunchStatusLabel(project, request), "Starting");
  assert.equal(coreProjectLocationLabel(project, request), "Starting your agent");
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
