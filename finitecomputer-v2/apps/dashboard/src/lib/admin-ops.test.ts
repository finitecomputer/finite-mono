import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";

import {
  ONE_TIME_KEY_WARNING,
  adminRuntimeMatchesSearch,
  canAccessAdminOps,
  finitePrivateAccountForProject,
  finitePrivateAssignableProfiles,
  finitePrivateGrantSummaryForRuntime,
  finitePrivateProfileLabel,
  groupAdminRuntimesByOwner,
  heartbeatAgeLabel,
  oneTimeKeyDisplay,
  oneTimeKeyError,
  type OneTimeKeyActionState,
} from "./admin-ops";

test("canAccessAdminOps only allows dashboard admins", () => {
  assert.equal(canAccessAdminOps({ isAdmin: true }), true);
  assert.equal(canAccessAdminOps({ isAdmin: false }), false);
  assert.equal(canAccessAdminOps(null), false);
  assert.equal(canAccessAdminOps(undefined), false);
});

test("heartbeatAgeLabel formats ages and degrades safely", () => {
  const now = Date.parse("2026-07-02T12:00:00Z");
  assert.equal(heartbeatAgeLabel(null, now), "never");
  assert.equal(heartbeatAgeLabel(undefined, now), "never");
  assert.equal(heartbeatAgeLabel("not-a-date", now), "unknown");
  assert.equal(heartbeatAgeLabel("2026-07-02T12:00:05Z", now), "just now");
  assert.equal(heartbeatAgeLabel("2026-07-02T11:59:30Z", now), "30s ago");
  assert.equal(heartbeatAgeLabel("2026-07-02T11:45:00Z", now), "15m ago");
  assert.equal(heartbeatAgeLabel("2026-07-02T02:00:00Z", now), "10h ago");
  assert.equal(heartbeatAgeLabel("2026-06-28T12:00:00Z", now), "4d ago");
});

test("Finite Private helpers keep the curated 1x/5x order and exact project correlation", () => {
  const profiles = finitePrivateAssignableProfiles([
    { id: "finite-private-generous-5x-v1", burst_limit_units: 500_000_000 },
    { id: "legacy-custom", burst_limit_units: 7 },
    { id: "finite-private-generous-v2", burst_limit_units: 100_000_000 },
  ]);
  assert.deepEqual(
    profiles.map((profile) => profile.id),
    ["finite-private-generous-v2", "finite-private-generous-5x-v1"]
  );
  assert.equal(
    finitePrivateProfileLabel(profiles[0].id),
    "1× · 100M units / 5h"
  );
  assert.equal(
    finitePrivateProfileLabel(profiles[1].id),
    "5× · 500M units / 5h"
  );

  const accounts = [
    { email: "a@example.test", projects: [{ id: "project-a" }] },
    { email: "b@example.test", projects: [{ id: "project-b" }] },
  ];
  assert.equal(
    finitePrivateAccountForProject(accounts, "project-b")?.email,
    "b@example.test"
  );
  assert.equal(finitePrivateAccountForProject(accounts, "project-missing"), null);
});

test("finitePrivateGrantSummaryForRuntime resolves the runtime-scoped active grant", () => {
  const summary = finitePrivateGrantSummaryForRuntime(
    { project_id: "project-a", agent_runtime_id: "runtime-a" },
    {
      grants: [
        {
          id: "fp_grant_project",
          user_id: "user-project",
          limit_profile_id: "finite-private-generous-v2",
          status: "active",
          current_window_started_at: "2026-07-02T10:00:00Z",
          current_window_used_units: 111,
        },
        {
          id: "fp_grant_runtime",
          user_id: "user-runtime",
          limit_profile_id: "finite-private-generous-5x-v1",
          status: "active",
          current_window_started_at: "2026-07-02T11:00:00Z",
          current_window_used_units: 222,
        },
      ],
      apiKeys: [
        {
          id: "fp_key_project",
          grant_id: "fp_grant_project",
          project_id: "project-a",
          agent_runtime_id: null,
          status: "active",
          updated_at: "2026-07-02T12:00:00Z",
        },
        {
          id: "fp_key_runtime",
          grant_id: "fp_grant_runtime",
          project_id: "project-a",
          agent_runtime_id: "runtime-a",
          status: "active",
          updated_at: "2026-07-02T11:00:00Z",
        },
      ],
    },
  );

  assert.deepEqual(summary, {
    grantId: "fp_grant_runtime",
    grantStatus: "active",
    grantUserId: "user-runtime",
    limitProfileId: "finite-private-generous-5x-v1",
    currentWindowStartedAt: "2026-07-02T11:00:00Z",
    currentWindowUsedUnits: 222,
    keyId: "fp_key_runtime",
    keyStatus: "active",
    keyProjectId: "project-a",
    keyAgentRuntimeId: "runtime-a",
    matchScope: "runtime",
  });
});

test("finitePrivateGrantSummaryForRuntime falls back to the newest matching project key", () => {
  const summary = finitePrivateGrantSummaryForRuntime(
    { project_id: "project-a", agent_runtime_id: "runtime-a" },
    {
      grants: [
        {
          id: "fp_grant_old",
          user_id: "user-old",
          limit_profile_id: "finite-private-generous-v2",
          status: "revoked",
          current_window_used_units: 10,
        },
        {
          id: "fp_grant_new",
          user_id: "user-new",
          limit_profile_id: "finite-private-generous-5x-v1",
          status: "revoked",
          current_window_started_at: null,
          current_window_used_units: 20,
        },
      ],
      apiKeys: [
        {
          id: "fp_key_unrelated",
          grant_id: "fp_grant_old",
          project_id: "project-other",
          agent_runtime_id: "runtime-other",
          status: "active",
          updated_at: "2026-07-02T13:00:00Z",
        },
        {
          id: "fp_key_old",
          grant_id: "fp_grant_old",
          project_id: "project-a",
          agent_runtime_id: null,
          status: "revoked",
          updated_at: "2026-07-02T10:00:00Z",
        },
        {
          id: "fp_key_new",
          grant_id: "fp_grant_new",
          project_id: "project-a",
          agent_runtime_id: null,
          status: "revoked",
          updated_at: "2026-07-02T12:00:00Z",
        },
      ],
    },
  );

  assert.equal(summary?.grantId, "fp_grant_new");
  assert.equal(summary?.keyId, "fp_key_new");
  assert.equal(summary?.matchScope, "project");
});

test("admin runtimes group into one sorted card per owner", () => {
  const groups = groupAdminRuntimesByOwner([
    { owner_email: "z@example.test", project_id: "project-z" },
    { owner_email: "A@Example.Test", project_id: "project-a-1" },
    { owner_email: "a@example.test", project_id: "project-a-2" },
  ]);
  assert.equal(groups.length, 2);
  assert.equal(groups[0].email, "a@example.test");
  assert.deepEqual(
    groups[0].runtimes.map((runtime) => runtime.project_id),
    ["project-a-1", "project-a-2"]
  );
  assert.equal(groups[1].email, "z@example.test");
});

test("admin runtime search matches agent, box, user, grant, key, and profile fields", () => {
  const runtime = {
    owner_email: "agent@example.test",
    project_id: "project_agent_m",
    project_display_name: "Agent M",
    agent_runtime_id: "runtime_agent_m",
    source_host_id: "finite-lat-3",
    source_machine_id: "finite-kata-b4a553a277f06141b934",
    runtime_artifact_id: "artifact_agent",
    runtime_artifact_version_label: "candidate",
    runtime_status: "online",
    published_app_urls: ["https://agent.example.test"],
  };
  const account = {
    userId: "user_agent_m",
    email: "agent@example.test",
    grant: {
      id: "fp_grant_agent_m",
      user_id: "user_agent_m",
      limit_profile_id: "finite-private-generous-5x-v1",
      status: "active",
      current_window_used_units: 8432,
    },
    apiKeys: [
      {
        id: "fp_key_agent_m",
        grant_id: "fp_grant_agent_m",
        project_id: "project_agent_m",
        agent_runtime_id: "runtime_agent_m",
        status: "active",
      },
    ],
    projects: [
      {
        id: "project_agent_m",
        displayName: "Agent M",
        agentRuntimeId: "runtime_agent_m",
      },
    ],
  };
  const finitePrivateGrant = {
    grantId: "fp_grant_agent_m",
    grantStatus: "active" as const,
    grantUserId: "user_agent_m",
    limitProfileId: "finite-private-generous-5x-v1",
    currentWindowStartedAt: "2026-07-02T12:00:00Z",
    currentWindowUsedUnits: 8432,
    keyId: "fp_key_agent_m",
    keyStatus: "active" as const,
    keyProjectId: "project_agent_m",
    keyAgentRuntimeId: "runtime_agent_m",
    matchScope: "runtime" as const,
  };

  assert.equal(adminRuntimeMatchesSearch(runtime, account, ""), true);
  assert.equal(adminRuntimeMatchesSearch(runtime, account, "agent m"), true);
  assert.equal(
    adminRuntimeMatchesSearch(runtime, account, "kata-b4a553a277f06141b934"),
    true
  );
  assert.equal(
    adminRuntimeMatchesSearch(runtime, account, "fp_grant_agent_m 5x"),
    true
  );
  assert.equal(
    adminRuntimeMatchesSearch(runtime, account, "fp_key_agent_m runtime_agent_m"),
    true
  );
  assert.equal(
    adminRuntimeMatchesSearch(runtime, null, "fp_grant_agent_m"),
    false
  );
  assert.equal(
    adminRuntimeMatchesSearch(
      runtime,
      null,
      "fp_grant_agent_m 5x",
      finitePrivateGrant,
    ),
    true
  );
  assert.equal(
    adminRuntimeMatchesSearch(
      runtime,
      null,
      "fp_key_agent_m runtime_agent_m",
      finitePrivateGrant,
    ),
    true
  );
  assert.equal(adminRuntimeMatchesSearch(runtime, account, "agent missing"), false);
});

test("admin page has three tabs and enriches user cards instead of separate grant/key lists", async () => {
  const [pageSource, usersPanelSource] = await Promise.all([
    readFile(path.resolve(process.cwd(), "src/app/dashboard/admin/page.tsx"), "utf8"),
    readFile(
      path.resolve(process.cwd(), "src/components/admin-users-panel.tsx"),
      "utf8"
    ),
  ]);
  assert.match(pageSource, /<TabsTrigger value="users">Users<\/TabsTrigger>/u);
  assert.match(pageSource, /<TabsTrigger value="invites">Invites<\/TabsTrigger>/u);
  assert.match(
    pageSource,
    /<TabsTrigger value="finite-private">Finite Private<\/TabsTrigger>/u
  );
  assert.match(pageSource, /<AdminUsersPanel result=\{runtimes\} finitePrivate=\{finitePrivate\} \/>/u);
  assert.match(usersPanelSource, /function ProvisionedUserCard/u);
  assert.match(usersPanelSource, /function FinitePrivateAccountControls/u);
  assert.match(usersPanelSource, /Filter agents/u);
  assert.match(usersPanelSource, /adminRuntimeMatchesSearch/u);
  assert.match(usersPanelSource, /No agents match that filter/u);
  assert.match(usersPanelSource, /adminOpsResetFinitePrivateWindowAction/u);
  assert.match(usersPanelSource, />\s*Reset usage\s*</u);
  assert.match(usersPanelSource, /AdminFinitePrivateProfileForm/u);
  assert.match(usersPanelSource, /Finite Private details unavailable/u);
  assert.doesNotMatch(pageSource + usersPanelSource, /function AdminGrantList/u);
  assert.doesNotMatch(pageSource + usersPanelSource, /function AdminKeyList/u);
});

test("oneTimeKeyDisplay only renders for a real issued key", () => {
  assert.equal(oneTimeKeyDisplay(null), null);
  assert.equal(oneTimeKeyDisplay(undefined), null);
  assert.equal(oneTimeKeyDisplay({ status: "idle" }), null);
  assert.equal(oneTimeKeyDisplay({ status: "error", error: "nope" }), null);
  assert.equal(
    oneTimeKeyDisplay({
      status: "issued",
      keyId: "fp_key_1",
      grantId: "fp_grant_1",
      rawKey: "   ",
      note: "",
    }),
    null,
  );

  const display = oneTimeKeyDisplay({
    status: "issued",
    keyId: "fp_key_1",
    grantId: "fp_grant_1",
    rawKey: " fpk_live_abc123 ",
    note: "",
  });
  assert.ok(display);
  assert.equal(display.keyId, "fp_key_1");
  assert.equal(display.grantId, "fp_grant_1");
  assert.equal(display.rawKey, "fpk_live_abc123");
  assert.equal(display.warning, ONE_TIME_KEY_WARNING);
});

test("oneTimeKeyDisplay keeps a Core-provided one-time note", () => {
  const display = oneTimeKeyDisplay({
    status: "issued",
    keyId: "fp_key_2",
    grantId: null,
    rawKey: "fpk_live_next",
    note: "This raw key is shown once.",
  });
  assert.ok(display);
  assert.equal(display.warning, "This raw key is shown once.");
});

test("oneTimeKeyError surfaces only error states", () => {
  assert.equal(oneTimeKeyError(null), null);
  assert.equal(oneTimeKeyError({ status: "idle" }), null);
  const issued: OneTimeKeyActionState = {
    status: "issued",
    keyId: "fp_key_1",
    grantId: null,
    rawKey: "fpk_live_x",
    note: "",
  };
  assert.equal(oneTimeKeyError(issued), null);
  assert.equal(oneTimeKeyError({ status: "error", error: "denied" }), "denied");
  assert.equal(
    oneTimeKeyError({ status: "error", error: "  " }),
    "The admin action failed.",
  );
});

test("admin runtime controls use exact fail-closed capabilities", async () => {
  const [actionsSource, adminUsersSource, upgradePageSource] = await Promise.all([
    readFile(path.resolve(process.cwd(), "src/app/actions.ts"), "utf8"),
    readFile(
      path.resolve(process.cwd(), "src/components/admin-users-panel.tsx"),
      "utf8"
    ),
    readFile(
      path.resolve(
        process.cwd(),
        "src/app/dashboard/admin/runtime-upgrade/page.tsx"
      ),
      "utf8"
    ),
  ]);

  const restartActionSource = sourceBetween(
    actionsSource,
    "export async function adminOpsRestartRuntimeAction",
    "export async function adminOpsRecoverRuntimeAction"
  );
  const recoverActionSource = sourceBetween(
    actionsSource,
    "export async function adminOpsRecoverRuntimeAction",
    "export async function adminOpsUpgradeRuntimeAction"
  );
  const upgradeActionSource = sourceBetween(
    actionsSource,
    "export async function adminOpsUpgradeRuntimeAction",
    "export async function adminOpsRevokeFinitePrivateKeyAction"
  );

  assert.doesNotMatch(adminUsersSource, /supports_runtime_control/u);
  assert.doesNotMatch(upgradePageSource, /supports_runtime_control/u);
  assert.doesNotMatch(actionsSource, /supports_runtime_control/u);

  assert.match(adminUsersSource, /const canRestart = adminRuntimeSupportsRestart\(runtime\)/u);
  assert.match(adminUsersSource, /disabled=\{!canRestart\}/u);
  assert.match(adminUsersSource, /const canRecover = adminRuntimeSupportsRecovery\(runtime\)/u);
  assert.match(adminUsersSource, /disabled=\{!canRecover\}/u);
  assert.match(adminUsersSource, /const canUpgrade = adminRuntimeSupportsUpgrade\(runtime\)/u);
  assert.match(adminUsersSource, /\{canUpgrade \? \(/u);

  assert.match(restartActionSource, /requireAdminViewer\("restart hosted runtimes"\)/u);
  assert.match(restartActionSource, /loadAdminRuntimeForAction\(projectId\)/u);
  assert.match(restartActionSource, /coreAdminRuntimeSupportsRestart\(runtime\)/u);
  assert.match(restartActionSource, /adminRestartCoreRuntime\(projectId\)/u);
  assert.doesNotMatch(
    restartActionSource,
    /coreAdminRuntimeSupports(?:Recovery|Upgrade)\(/u
  );

  assert.match(recoverActionSource, /requireAdminViewer\("recover hosted runtimes"\)/u);
  assert.match(recoverActionSource, /loadAdminRuntimeForAction\(projectId\)/u);
  assert.match(recoverActionSource, /coreAdminRuntimeSupportsRecovery\(runtime\)/u);
  assert.match(recoverActionSource, /adminRecoverCoreRuntime\(projectId\)/u);
  assert.doesNotMatch(
    recoverActionSource,
    /coreAdminRuntimeSupports(?:Restart|Upgrade)\(/u
  );

  assert.match(upgradeActionSource, /requireAdminViewer\("upgrade hosted runtimes"\)/u);
  assert.match(upgradeActionSource, /loadAdminRuntimeForAction\(projectId\)/u);
  assert.match(upgradeActionSource, /coreAdminRuntimeSupportsUpgrade\(runtime\)/u);
  assert.match(upgradeActionSource, /adminUpgradeCoreRuntime\(\{/u);
  assert.doesNotMatch(
    upgradeActionSource,
    /coreAdminRuntimeSupports(?:Restart|Recovery)\(/u
  );

  assert.match(
    actionsSource,
    /adminOpsUpgradeRuntimeAction[\s\S]*requireAdminViewer\("upgrade hosted runtimes"\)[\s\S]*adminUpgradeCoreRuntime\([\s\S]*targetRuntimeArtifactId[\s\S]*redirect\("\/dashboard\/admin"\)/u
  );
  assert.doesNotMatch(adminUsersSource, /name="targetRuntimeArtifactId"/u);
  assert.match(
    adminUsersSource,
    /pathname: "\/dashboard\/admin\/runtime-upgrade"[\s\S]*query: \{ projectId: runtime\.project_id \}/u
  );
  assert.match(upgradePageSource, /canAccessAdminOps\(viewer\)/u);
  assert.match(upgradePageSource, /loadCoreAdminRuntimes\(\)/u);
  assert.match(
    upgradePageSource,
    /candidate\.project_id === projectId/u
  );
  assert.match(upgradePageSource, /name="targetRuntimeArtifactId"/u);
  assert.match(
    upgradePageSource,
    /!coreAdminRuntimeSupportsUpgrade\(runtime\)/u
  );
  assert.match(upgradePageSource, /required/u);
  assert.match(upgradePageSource, /<FormActionButton/u);
  assert.doesNotMatch(upgradePageSource, /ConfirmSubmitButton/u);
  assert.match(
    upgradePageSource,
    /No\s+candidate is selected automatically\./u
  );
});

function sourceBetween(source: string, start: string, end: string) {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);
  assert.notEqual(startIndex, -1, `missing source marker: ${start}`);
  assert.notEqual(endIndex, -1, `missing source marker: ${end}`);
  return source.slice(startIndex, endIndex);
}
