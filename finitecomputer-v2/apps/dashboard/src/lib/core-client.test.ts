import assert from "node:assert/strict";
import test from "node:test";

import {
  coreAdminRuntimeSupportsRecovery,
  coreAdminRuntimeSupportsRestart,
  coreAdminRuntimeSupportsRetirement,
  coreAdminRuntimeSupportsStop,
  coreAdminRuntimeSupportsUpgrade,
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
  coreProjectSupportsHostedRecovery,
  coreProjectSupportsHostedRestart,
  coreProjectSupportsHostedRuntimeControl,
  coreProjectSupportsHostedRuntimeUpgrade,
  coreProjectSupportsHostedStop,
  coreProjectSupportsRetirement,
  coreProductProjectForLegacyMachineId,
  coreProductProjectForRouteId,
  coreProductProjects,
  coreRuntimeCapabilitiesSupport,
  type CoreAgentCreationRequestSummary,
  type CoreAdminRuntimeOverview,
  type CoreRuntimeCapabilities,
  type CoreVisibleProject,
  loadCoreFinitePrivateUsageStatus,
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

test("runtime capability helpers fail closed when the advertisement is absent", () => {
  const project = {
    project: { id: "project_1" },
    runtime: { id: "runtime_1" },
  } as CoreVisibleProject;
  const projectHelpers = [
    coreProjectSupportsHostedRuntimeControl,
    coreProjectSupportsHostedRestart,
    coreProjectSupportsHostedRecovery,
    coreProjectSupportsHostedRuntimeUpgrade,
    coreProjectSupportsHostedStop,
    coreProjectSupportsRetirement,
  ];
  const adminHelpers = [
    coreAdminRuntimeSupportsRestart,
    coreAdminRuntimeSupportsRecovery,
    coreAdminRuntimeSupportsUpgrade,
    coreAdminRuntimeSupportsStop,
    coreAdminRuntimeSupportsRetirement,
  ];

  for (const helper of projectHelpers) {
    assert.equal(helper(undefined), false);
    assert.equal(helper(project), false);
  }
  for (const helper of adminHelpers) {
    assert.equal(helper(undefined), false);
    assert.equal(helper({} as CoreAdminRuntimeOverview), false);
  }
  for (const operation of [
    "restart",
    "recover_known_good_chat",
    "runtime_upgrade",
    "stop",
    "runtime_retirement",
  ] as const) {
    assert.equal(coreRuntimeCapabilitiesSupport(undefined, operation), false);
    assert.equal(coreRuntimeCapabilitiesSupport(null, operation), false);
    assert.equal(coreRuntimeCapabilitiesSupport({}, operation), false);
  }
});

test("runtime capability helpers gate only their exact advertised operation", () => {
  const operations = [
    {
      capability: "restart",
      projectHelper: coreProjectSupportsHostedRestart,
      adminHelper: coreAdminRuntimeSupportsRestart,
    },
    {
      capability: "recover_known_good_chat",
      projectHelper: coreProjectSupportsHostedRecovery,
      adminHelper: coreAdminRuntimeSupportsRecovery,
    },
    {
      capability: "runtime_upgrade",
      projectHelper: coreProjectSupportsHostedRuntimeUpgrade,
      adminHelper: coreAdminRuntimeSupportsUpgrade,
    },
    {
      capability: "stop",
      projectHelper: coreProjectSupportsHostedStop,
      adminHelper: coreAdminRuntimeSupportsStop,
    },
    {
      capability: "runtime_retirement",
      projectHelper: coreProjectSupportsRetirement,
      adminHelper: coreAdminRuntimeSupportsRetirement,
    },
  ] as const;

  for (const advertised of operations) {
    const capabilities: CoreRuntimeCapabilities = {
      restart: false,
      recover_known_good_chat: false,
      runtime_upgrade: false,
      stop: false,
      runtime_retirement: false,
      [advertised.capability]: true,
    };
    const project = {
      project: { id: "project_1" },
      runtime: { id: "runtime_1", runtime_capabilities: capabilities },
    } as CoreVisibleProject;
    const adminRuntime = {
      runtime_capabilities: capabilities,
    } as CoreAdminRuntimeOverview;

    for (const operation of operations) {
      const expected = operation.capability === advertised.capability;
      assert.equal(operation.projectHelper(project), expected);
      assert.equal(operation.adminHelper(adminRuntime), expected);
      assert.equal(
        coreRuntimeCapabilitiesSupport(capabilities, operation.capability),
        expected
      );
    }
  }
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

test("Finite Private usage is N-1 fail-soft on 404 but surfaces real Core failures", async () => {
  const names = [
    "FC_CORE_BASE_URL",
    "FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH",
    "FC_DASHBOARD_DEV_EMAIL",
    "FC_DASHBOARD_DEV_WORKOS_USER_ID",
    "FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN",
    "FC_WORKOS_AUTH_ENABLED",
  ] as const;
  const previous = Object.fromEntries(names.map((name) => [name, process.env[name]]));
  const previousFetch = globalThis.fetch;
  const requests: Array<{ url: string; authorization: string | null }> = [];

  process.env.FC_CORE_BASE_URL = "https://core.example.com";
  process.env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH = "1";
  process.env.FC_DASHBOARD_DEV_EMAIL = "usage-test@finite.vip";
  process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID = "user_usage_test";
  process.env.FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN = "dev-access-token";
  delete process.env.FC_WORKOS_AUTH_ENABLED;

  try {
    for (const fixture of [
      { status: 404, error: "route not found", expectedError: null },
      { status: 503, error: "Core is warming up", expectedError: "Core is warming up" },
    ]) {
      globalThis.fetch = (async (input, init) => {
        requests.push({
          url: String(input),
          authorization: new Headers(init?.headers).get("authorization"),
        });
        return new Response(JSON.stringify({ error: fixture.error }), {
          status: fixture.status,
          headers: { "content-type": "application/json" },
        });
      }) as typeof fetch;

      const result = await loadCoreFinitePrivateUsageStatus();
      assert.equal(result.usage, null);
      assert.equal(result.error, fixture.expectedError);
    }

    assert.deepEqual(
      requests.map((request) => request.url),
      Array(2).fill("https://core.example.com/api/core/v1/me/finite-private/usage")
    );
    assert.deepEqual(
      requests.map((request) => request.authorization),
      Array(2).fill("Bearer dev-access-token")
    );
  } finally {
    for (const name of names) {
      const value = previous[name];
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
    globalThis.fetch = previousFetch;
  }
});
