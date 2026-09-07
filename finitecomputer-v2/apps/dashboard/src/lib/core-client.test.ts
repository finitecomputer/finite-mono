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
  coreInitialAgentCreationRequests,
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
  coreRuntimeControlConflictMessage,
  CoreFetchError,
  isCoreRuntimeControlConflict,
  type CoreAgentCreationRequestSummary,
  type CoreAdminRuntimeOverview,
  type CorePublicRuntimeControl,
  type CoreRuntimeCapabilities,
  type CoreVisibleProject,
  loadCoreDashboardSummary,
  loadCoreFinitePrivateUsageStatus,
  runtimeRetirementProductEnabled,
} from "./core-client";

test("owner-facing retirement has an independent default-off product gate", () => {
  const previous = process.env.FC_DASHBOARD_ENABLE_RUNTIME_RETIREMENT;
  delete process.env.FC_DASHBOARD_ENABLE_RUNTIME_RETIREMENT;
  assert.equal(runtimeRetirementProductEnabled(), false);
  process.env.FC_DASHBOARD_ENABLE_RUNTIME_RETIREMENT = "true";
  assert.equal(runtimeRetirementProductEnabled(), true);
  if (previous === undefined) {
    delete process.env.FC_DASHBOARD_ENABLE_RUNTIME_RETIREMENT;
  } else {
    process.env.FC_DASHBOARD_ENABLE_RUNTIME_RETIREMENT = previous;
  }
});

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
    hostingTier: "confidential" as const,
    profilePictureUrl: "https://chat.example/profile.png",
    runnerClass: "phala",
  };
  const body = coreAgentCreationRequestBody(staleInput);
  assert.deepEqual(body, {
    displayName: "Moss",
    launchCode: "launch-fixture",
    idempotencyKey: "request-fixture",
    hostingTier: "confidential",
    profilePictureUrl: "https://chat.example/profile.png",
  });
  assert.equal("runnerClass" in body, false);
  assert.deepEqual(
    coreAgentCreationRequestBody({
      displayName: "Moss",
      launchCode: "",
      idempotencyKey: "request-without-picture",
      hostingTier: "standard",
    }),
    {
      displayName: "Moss",
      launchCode: "",
      idempotencyKey: "request-without-picture",
      hostingTier: "standard",
    }
  );
});

test("agent creation payload carries the owner chat account id only when known", () => {
  const ownerChatAccountId = "ab".repeat(32);
  assert.deepEqual(
    coreAgentCreationRequestBody({
      displayName: "Moss",
      launchCode: "launch-fixture",
      idempotencyKey: "request-owner-npub",
      hostingTier: "standard",
      ownerChatAccountId,
    }),
    {
      displayName: "Moss",
      launchCode: "launch-fixture",
      idempotencyKey: "request-owner-npub",
      hostingTier: "standard",
      ownerChatAccountId,
    }
  );
  // Fail-open launches (hosted device unreachable) omit the key entirely so
  // Core leases the runtime without FINITECHAT_OWNER_NPUBS.
  const failOpen = coreAgentCreationRequestBody({
    displayName: "Moss",
    launchCode: "launch-fixture",
    idempotencyKey: "request-owner-npub-absent",
    hostingTier: "standard",
    ownerChatAccountId: null,
  });
  assert.equal("ownerChatAccountId" in failOpen, false);
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
      lifecycle_status: "online",
      runtime_health: {
        status: "ready",
        reported_at: "2026-05-25T12:00:00Z",
        observed_at: "2026-05-25T11:59:59Z",
        report_interval_seconds: 60,
      },
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
  } as unknown as CoreVisibleProject;
  const second = {
    project: {
      id: "project_second",
      import_candidate_id: null,
    },
    runtime: {
      id: "runtime_second",
    },
  } as unknown as CoreVisibleProject;

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
  } as unknown as CoreVisibleProject;
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

test("relocation requests stay out of initial agent creation presentation", () => {
  const initialRequest: CoreAgentCreationRequestSummary = {
    id: "agent_creation_request_initial",
    project_id: "project_1",
    display_name: "Oslo Agent",
    status: "running",
    agent_runtime_id: "runtime_1",
    failure_message: null,
    created_at: "2026-05-25T12:00:00Z",
    updated_at: "2026-05-25T12:01:00Z",
  };
  const relocationRequest: CoreAgentCreationRequestSummary = {
    ...initialRequest,
    id: "agent_creation_request_relocation",
    is_relocation: true,
    status: "failed",
    failure_message: "operator-only relocation failed",
    updated_at: "2026-07-25T14:00:00Z",
  };
  const project: CoreVisibleProject = {
    project: {
      id: "project_1",
      display_name: "Oslo Agent",
      created_at: "2026-05-25T12:00:00Z",
      updated_at: "2026-07-25T14:00:00Z",
    },
    runtime: null,
  };

  assert.deepEqual(
    coreInitialAgentCreationRequests([relocationRequest, initialRequest]),
    [initialRequest]
  );
  assert.equal(
    coreAgentCreationRequestForProject(project, [relocationRequest, initialRequest]),
    initialRequest
  );
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

test("dashboard summary uses one Core request and falls back safely to independent routes", async () => {
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
  const requests: string[] = [];
  const me = {
    email: "summary-test@finite.vip",
    workos_user_id: "user_summary_test",
    projects: [],
    agent_creation_requests: [],
  };
  const billing = {
    customer_org: {
      id: "org_summary",
      owner_user_id: "user_summary",
      name: "summary-test@finite.vip",
      billing_class: "standard",
      created_at: "2026-07-23T12:00:00Z",
      updated_at: "2026-07-23T12:00:00Z",
    },
    billing_account: null,
    agent_creation_entitlement: null,
    can_create_agent: false,
    requires_billing: true,
  };

  process.env.FC_CORE_BASE_URL = "https://core.example.com";
  process.env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH = "1";
  process.env.FC_DASHBOARD_DEV_EMAIL = "summary-test@finite.vip";
  process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID = "user_summary_test";
  process.env.FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN = "dev-access-token";
  delete process.env.FC_WORKOS_AUTH_ENABLED;

  try {
    globalThis.fetch = (async (input, init) => {
      requests.push(String(input));
      assert.equal(
        new Headers(init?.headers).get("authorization"),
        "Bearer dev-access-token"
      );
      return new Response(
        JSON.stringify({ me, billing, finite_private_usage: null }),
        { status: 200, headers: { "content-type": "application/json" } }
      );
    }) as typeof fetch;

    const aggregate = await loadCoreDashboardSummary();
    assert.deepEqual(requests, [
      "https://core.example.com/api/core/v1/me/dashboard-summary",
    ]);
    assert.deepEqual(aggregate.core.me, me);
    assert.deepEqual(aggregate.billing.billing, billing);
    assert.equal(aggregate.finitePrivateUsage.usage, null);
    assert.equal(aggregate.core.error, null);

    requests.length = 0;
    globalThis.fetch = (async (input, init) => {
      const url = String(input);
      requests.push(url);
      assert.equal(
        new Headers(init?.headers).get("authorization"),
        "Bearer dev-access-token"
      );
      if (url.endsWith("/api/core/v1/me/dashboard-summary")) {
        return new Response(JSON.stringify({ error: "route not found" }), {
          status: 404,
          headers: { "content-type": "application/json" },
        });
      }
      if (url.endsWith("/api/core/v1/me")) {
        return Response.json(me);
      }
      if (url.endsWith("/api/core/v1/me/billing")) {
        return Response.json(billing);
      }
      if (url.endsWith("/api/core/v1/me/finite-private/usage")) {
        return new Response(JSON.stringify({ error: "route not found" }), {
          status: 404,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response(JSON.stringify({ error: "unexpected route" }), {
        status: 500,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;

    const fallback = await loadCoreDashboardSummary();
    assert.equal(requests[0], "https://core.example.com/api/core/v1/me/dashboard-summary");
    assert.deepEqual(
      new Set(requests.slice(1)),
      new Set([
        "https://core.example.com/api/core/v1/me",
        "https://core.example.com/api/core/v1/me/billing",
        "https://core.example.com/api/core/v1/me/finite-private/usage",
      ])
    );
    assert.deepEqual(fallback.core.me, me);
    assert.deepEqual(fallback.billing.billing, billing);
    assert.equal(fallback.finitePrivateUsage.usage, null);
    assert.equal(fallback.finitePrivateUsage.error, null);

    requests.length = 0;
    globalThis.fetch = (async (input, init) => {
      const url = String(input);
      requests.push(url);
      assert.equal(
        new Headers(init?.headers).get("authorization"),
        "Bearer dev-access-token"
      );
      if (url.endsWith("/api/core/v1/me/dashboard-summary")) {
        return new Response(JSON.stringify({ error: "aggregate failed" }), {
          status: 500,
          headers: { "content-type": "application/json" },
        });
      }
      if (url.endsWith("/api/core/v1/me")) {
        return Response.json(me);
      }
      if (url.endsWith("/api/core/v1/me/billing")) {
        return Response.json(billing);
      }
      if (url.endsWith("/api/core/v1/me/finite-private/usage")) {
        return new Response(JSON.stringify({ error: "usage failed" }), {
          status: 500,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response(JSON.stringify({ error: "unexpected route" }), {
        status: 500,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;

    const degraded = await loadCoreDashboardSummary();
    assert.deepEqual(degraded.core.me, me);
    assert.equal(degraded.core.error, null);
    assert.deepEqual(degraded.billing.billing, billing);
    assert.equal(degraded.billing.error, null);
    assert.equal(degraded.finitePrivateUsage.usage, null);
    assert.match(degraded.finitePrivateUsage.error ?? "", /usage failed/);
  } finally {
    for (const name of names) {
      const value = previous[name];
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
    globalThis.fetch = previousFetch;
  }
});

test("a Core 409 is a runtime-control conflict; other failures are not", () => {
  assert.equal(
    isCoreRuntimeControlConflict(new CoreFetchError("restart already running", 409)),
    true
  );
  assert.equal(
    isCoreRuntimeControlConflict(new CoreFetchError("core unavailable", 500)),
    false
  );
  assert.equal(isCoreRuntimeControlConflict(new Error("conflict")), false);
  assert.equal(isCoreRuntimeControlConflict("409"), false);
});

test("the runtime-control conflict message names the in-progress request", () => {
  const control = (kind: CorePublicRuntimeControl["kind"]): CorePublicRuntimeControl => ({
    id: "control_1",
    kind,
    status: "launching",
    retrying: false,
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
  });

  assert.match(coreRuntimeControlConflictMessage(control("restart")), /restart is already in progress/u);
  assert.match(coreRuntimeControlConflictMessage(control("stop")), /stop is already in progress/u);
  assert.match(
    coreRuntimeControlConflictMessage(control("recover_known_good_chat_runtime")),
    /chat recovery is already in progress/u
  );
  assert.match(
    coreRuntimeControlConflictMessage(null),
    /Another request is already in progress/u
  );
});
