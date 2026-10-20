import assert from "node:assert/strict";
import type { IncomingMessage, ServerResponse } from "node:http";

import type { Dialog, Locator, Page } from "playwright";

// Synthetic admin runtimes and Finite Private state for the fake Core. Admin
// mutations change this state so the dashboard's revalidated reads differ.
export function createAdminAgentFixture() {
  return {
    runtimes: [privateRuntime()],
    finitePrivate: finitePrivateAdminState(),
    rotations: 0,
    mutations: [] as string[],
  };
}

export type AdminAgentFixture = ReturnType<typeof createAdminAgentFixture>;
type FixtureGrant = AdminAgentFixture["finitePrivate"]["grants"][number];
type FixtureKey = AdminAgentFixture["finitePrivate"]["apiKeys"][number];

const MUTATED_AT = "2026-05-29T09:00:00Z";
const ROTATED_RAW_KEY = "fpk_live_browser_rotated_raw_key_000001";

/** Returns false when the request is not an admin agent route. */
export async function handleAdminAgentRequest(
  request: IncomingMessage,
  response: ServerResponse,
  fixture: AdminAgentFixture
) {
  const url = request.url ?? "";
  if (request.method === "GET" && url === "/api/core/v1/admin/runtimes") {
    return writeJson(response, fixture.runtimes);
  }
  if (request.method === "GET" && url === "/api/core/v1/finite-private/admin-state") {
    return writeJson(response, fixture.finitePrivate);
  }
  if (request.method !== "POST") return false;

  const grantRoute = /^\/api\/core\/v1\/admin\/finite-private\/grants\/([^/]+)\/(window-reset|limit-profile)$/u.exec(url);
  const keyRoute = /^\/api\/core\/v1\/admin\/finite-private\/keys\/([^/]+)\/(rotate|revoke)$/u.exec(url);
  const restartRoute = /^\/api\/core\/v1\/admin\/projects\/([^/]+)\/runtime\/restart$/u.exec(url);
  if (!grantRoute && !keyRoute && !restartRoute) return false;

  const body = JSON.parse((await readBody(request)) || "{}") as { limitProfileId?: string };
  fixture.mutations.push(url);
  if (grantRoute) {
    const grantId = decodeURIComponent(grantRoute[1]);
    const changes = grantRoute[2] === "window-reset"
      ? { current_window_used_units: 0, current_window_started_at: MUTATED_AT }
      : { limit_profile_id: String(body.limitProfileId) };
    return writeJson(response, updateGrant(fixture, grantId, changes));
  }
  if (keyRoute) {
    const keyId = decodeURIComponent(keyRoute[1]);
    const revoked = updateKey(fixture, keyId, { status: "revoked" });
    if (keyRoute[2] === "revoke") return writeJson(response, revoked);
    fixture.rotations += 1;
    const replacement = { ...revoked, id: `fp_key_rotated_${fixture.rotations}`, status: "active", created_at: MUTATED_AT };
    replaceKeys(fixture, (keys) => [...keys, replacement]);
    return writeJson(response, {
      api_key: replacement,
      grant: fixture.finitePrivate.grants.find((grant) => grant.id === replacement.grant_id) ?? null,
      raw_api_key: ROTATED_RAW_KEY,
      raw_api_key_note: "Shown once.",
    });
  }
  const projectId = decodeURIComponent(restartRoute![1]);
  fixture.runtimes = fixture.runtimes.map((runtime) =>
    runtime.project_id === projectId
      ? { ...runtime, runtime_status: "offline", lifecycle_status: "requested", status_updated_at: MUTATED_AT }
      : runtime
  );
  const runtime = fixture.runtimes.find((candidate) => candidate.project_id === projectId);
  assert(runtime, `unknown fixture project ${projectId}`);
  return writeJson(response, {
    id: "control_restart_browser",
    project_id: projectId,
    agent_runtime_id: runtime.agent_runtime_id,
    source_host_id: runtime.source_host_id,
    source_machine_id: runtime.source_machine_id,
    requested_by_user_id: "user_admin",
    kind: "restart",
    status: "requested",
    created_at: MUTATED_AT,
    updated_at: MUTATED_AT,
  });
}

/**
 * Mutates through the open agent dialog and checks that each refresh replaces
 * the dialog's data, keeps rotation receipts and handles a removed runtime.
 */
export async function exerciseAgentDialogMutations(page: Page, fixture: AdminAgentFixture) {
  const acceptConfirm = (dialog: Dialog) => void dialog.accept();
  page.on("dialog", acceptConfirm);
  try {
    await page.getByRole("row", { name: "Open controls for Private Agent" }).click();
    const dialog = page.getByRole("dialog", { name: "Private Agent" });
    await dialog.getByText("84 of 200,000,000 weighted tokens", { exact: true }).waitFor({ state: "visible" });

    await dialog.getByRole("button", { name: "Reset usage", exact: true }).click();
    await dialog.getByText("0 of 200,000,000 weighted tokens", { exact: true }).waitFor({ state: "visible" });
    // The modal hides the table from role queries; it refreshes behind it.
    await page.locator("table").getByText("0 of 200,000,000 weighted tokens", { exact: true }).waitFor({ state: "attached" });

    await dialog.getByLabel("Usage limit").click();
    await page.getByRole("option", { name: "500M weighted tokens / 5h" }).click();
    await dialog.getByRole("button", { name: "Save limit" }).click();
    await dialog.getByText("0 of 500,000,000 weighted tokens", { exact: true }).waitFor({ state: "visible" });
    assert.equal(await dialog.getByLabel("Usage limit").textContent(), "500M weighted tokens / 5h");

    await dialog.getByRole("button", { name: "Rotate", exact: true }).click();
    await dialog.getByText(ROTATED_RAW_KEY).waitFor({ state: "visible" });
    await keyRow(dialog, "fp_key_rotated_1").getByText("active", { exact: true }).waitFor({ state: "visible" });
    await keyRow(dialog, "fp_key_1").getByText("revoked", { exact: true }).waitFor({ state: "visible" });
    assert.equal(await keyRow(dialog, "fp_key_1").getByRole("button", { name: "Rotate" }).count(), 0);
    assert.equal(await dialog.getByText(ROTATED_RAW_KEY).count(), 1, "the one-time receipt survives the refresh");

    await keyRow(dialog, "fp_key_rotated_1").getByRole("button", { name: "Revoke" }).click();
    await keyRow(dialog, "fp_key_rotated_1").getByText("revoked", { exact: true }).waitFor({ state: "visible" });
    assert.equal(fixture.mutations.at(-1), "/api/core/v1/admin/finite-private/keys/fp_key_rotated_1/revoke");
    await dialog.getByText(ROTATED_RAW_KEY).waitFor({ state: "visible" });

    await dialog.getByRole("button", { name: "Restart", exact: true }).click();
    await dialog.getByText(/lifecycle requested/u).waitFor({ state: "visible" });
    await dialog.locator("header").getByText("offline", { exact: true }).waitFor({ state: "visible" });

    fixture.runtimes = [];
    await dialog.getByRole("button", { name: "Reset usage", exact: true }).click();
    const unavailable = page.getByRole("dialog", { name: "Agent unavailable" });
    await unavailable.getByText(/Runtime runtime_private is not in the latest admin data/u).waitFor({ state: "visible" });
    await unavailable.getByText(ROTATED_RAW_KEY).waitFor({ state: "visible" });
    await unavailable.getByRole("button", { name: "Close", exact: true }).click();
    await page.getByText("No users with provisioned agents yet.").waitFor({ state: "visible" });
  } finally {
    page.off("dialog", acceptConfirm);
  }
}

function keyRow(dialog: Locator, keyId: string) {
  return dialog.getByRole("group", { name: `Finite Private key ${keyId}` });
}

function updateGrant(fixture: AdminAgentFixture, grantId: string, changes: Partial<FixtureGrant>) {
  const state = fixture.finitePrivate;
  const update = (grant: FixtureGrant) => (grant.id === grantId ? { ...grant, ...changes, updated_at: MUTATED_AT } : grant);
  fixture.finitePrivate = {
    ...state,
    grants: state.grants.map(update),
    accounts: state.accounts.map((account) => ({ ...account, grant: update(account.grant) })),
  };
  const grant = fixture.finitePrivate.grants.find((candidate) => candidate.id === grantId);
  assert(grant, `unknown fixture grant ${grantId}`);
  return grant;
}

function updateKey(fixture: AdminAgentFixture, keyId: string, changes: Partial<FixtureKey>) {
  replaceKeys(fixture, (keys) => keys.map((key) => (key.id === keyId ? { ...key, ...changes, updated_at: MUTATED_AT } : key)));
  const key = fixture.finitePrivate.apiKeys.find((candidate) => candidate.id === keyId);
  assert(key, `unknown fixture key ${keyId}`);
  return key;
}

function replaceKeys(fixture: AdminAgentFixture, change: (keys: FixtureKey[]) => FixtureKey[]) {
  const state = fixture.finitePrivate;
  fixture.finitePrivate = {
    ...state,
    apiKeys: change(state.apiKeys),
    accounts: state.accounts.map((account) => ({ ...account, apiKeys: change(account.apiKeys) })),
  };
}

function writeJson(response: ServerResponse, body: unknown) {
  response.writeHead(200, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
  return true;
}

async function readBody(request: IncomingMessage) {
  let body = "";
  for await (const chunk of request) {
    body += chunk.toString();
  }
  return body;
}

function privateRuntime() {
  return {
    project_display_name: "Private Agent",
    owner_email: "private@finite.vip",
    project_id: "project_private",
    agent_runtime_id: "runtime_private",
    source_host_id: "finite-host-browser",
    source_machine_id: "finite-kata-browser",
    runtime_artifact_id: "artifact_browser",
    runtime_artifact_version_label: "browser-test",
    runtime_status: "online",
    lifecycle_status: null as string | null,
    status_updated_at: null as string | null,
    last_heartbeat_at: "2026-05-28T12:00:00Z",
    runtime_updated_at: "2026-05-28T12:01:00Z",
    hermes_available: true,
    published_app_urls: [] as string[],
    active_finite_private_key_count: 1,
    runtime_link_active: true,
    runtime_capabilities: {
      restart: true,
      recover_known_good_chat: true,
      runtime_upgrade: false,
    },
  };
}

function finitePrivateAdminState() {
  const grant = {
    id: "fp_grant_1",
    user_id: "user_private",
    limit_profile_id: "finite-private-generous-v2",
    status: "active",
    current_window_started_at: "2026-05-28T12:00:00Z",
    current_window_used_units: 84,
    created_at: "2026-05-28T12:00:00Z",
    updated_at: "2026-05-28T12:01:00Z",
  };
  const apiKey = {
    id: "fp_key_1",
    grant_id: "fp_grant_1",
    project_id: "project_private",
    agent_runtime_id: "runtime_private",
    key_hash: "hash-only",
    status: "active",
    created_at: "2026-05-28T12:00:00Z",
    updated_at: "2026-05-28T12:01:00Z",
  };
  return {
    accounts: [
      {
        userId: "user_private",
        email: "private@finite.vip",
        grant,
        apiKeys: [apiKey],
        projects: [
          {
            id: "project_private",
            displayName: "Private Agent",
            agentRuntimeId: "runtime_private",
          },
        ],
      },
    ],
    profiles: [
      {
        id: "finite-private-generous-v2",
        burst_window_seconds: 18_000,
        burst_limit_units: 200_000_000,
        weekly_limit_units: null,
        created_at: "2026-05-28T12:00:00Z",
        updated_at: "2026-05-28T12:01:00Z",
      },
      {
        id: "finite-private-generous-5x-v1",
        burst_window_seconds: 18_000,
        burst_limit_units: 500_000_000,
        weekly_limit_units: null,
        created_at: "2026-05-28T12:00:00Z",
        updated_at: "2026-05-28T12:01:00Z",
      },
    ],
    grants: [grant],
    apiKeys: [apiKey],
    adminAuditEvents: [
      {
        id: "fp_event_1",
        action: "finite_private.grant.approve",
        target_type: "grant",
        target_id: "fp_grant_1",
        grant_id: "fp_grant_1",
        api_key_id: null,
        actor: "core-service",
        metadata: {},
        created_at: "2026-05-28T12:00:00Z",
      },
    ],
  };
}
