import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";

import {
  ONE_TIME_KEY_WARNING,
  adminRuntimeMatchesSearch,
  canAccessAdminOps,
  finitePrivateGrantSummaryForRuntime,
  heartbeatAgeLabel,
  oneTimeKeyDisplay,
  oneTimeKeyError,
  type AdminOpsFinitePrivateState,
  type AdminOpsRuntime,
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

test("finitePrivateGrantSummaryForRuntime resolves the runtime-scoped active grant", () => {
  const runtime = adminRuntimeFixture();
  const state: AdminOpsFinitePrivateState = {
    grants: [
      grantFixture("fp_grant_project", "user_project"),
      grantFixture("fp_grant_runtime", "user_runtime"),
    ],
    apiKeys: [
      {
        id: "fp_key_project",
        grant_id: "fp_grant_project",
        project_id: runtime.project_id,
        agent_runtime_id: null,
        status: "active",
        updated_at: "2026-07-02T12:10:00Z",
      },
      {
        id: "fp_key_runtime",
        grant_id: "fp_grant_runtime",
        project_id: runtime.project_id,
        agent_runtime_id: runtime.agent_runtime_id,
        status: "active",
        updated_at: "2026-07-02T12:00:00Z",
      },
    ],
  };

  const summary = finitePrivateGrantSummaryForRuntime(runtime, state);

  assert.ok(summary);
  assert.equal(summary.grantId, "fp_grant_runtime");
  assert.equal(summary.grantUserId, "user_runtime");
  assert.equal(summary.keyId, "fp_key_runtime");
  assert.equal(summary.matchScope, "runtime");
});

test("finitePrivateGrantSummaryForRuntime falls back to the newest matching key", () => {
  const runtime = adminRuntimeFixture();
  const state: AdminOpsFinitePrivateState = {
    grants: [
      grantFixture("fp_grant_old", "user_old"),
      grantFixture("fp_grant_new", "user_new"),
    ],
    apiKeys: [
      {
        id: "fp_key_old",
        grant_id: "fp_grant_old",
        project_id: runtime.project_id,
        agent_runtime_id: null,
        status: "revoked",
        updated_at: "2026-07-02T12:00:00Z",
      },
      {
        id: "fp_key_new",
        grant_id: "fp_grant_new",
        project_id: runtime.project_id,
        agent_runtime_id: null,
        status: "revoked",
        updated_at: "2026-07-02T12:05:00Z",
      },
    ],
  };

  const summary = finitePrivateGrantSummaryForRuntime(runtime, state);

  assert.ok(summary);
  assert.equal(summary.grantId, "fp_grant_new");
  assert.equal(summary.keyId, "fp_key_new");
  assert.equal(summary.matchScope, "project");
});

test("adminRuntimeMatchesSearch filters by agent, Kata box, grant, and key", () => {
  const runtime = adminRuntimeFixture();
  const summary = finitePrivateGrantSummaryForRuntime(runtime, {
    grants: [grantFixture("fp_grant_agent_m", "user_agent_m")],
    apiKeys: [
      {
        id: "fp_key_agent_m",
        grant_id: "fp_grant_agent_m",
        project_id: runtime.project_id,
        agent_runtime_id: runtime.agent_runtime_id,
        status: "active",
        updated_at: "2026-07-02T12:00:00Z",
      },
    ],
  });

  assert.equal(adminRuntimeMatchesSearch(runtime, summary, ""), true);
  assert.equal(adminRuntimeMatchesSearch(runtime, summary, "agent kata-b4a553"), true);
  assert.equal(adminRuntimeMatchesSearch(runtime, summary, "fp_grant_agent_m runtime"), true);
  assert.equal(adminRuntimeMatchesSearch(runtime, summary, "fp_key_agent_m owner@example.test"), true);
  assert.equal(adminRuntimeMatchesSearch(runtime, summary, "missing-token"), false);
});

test("admin runtime controls use exact fail-closed capabilities", async () => {
  const [actionsSource, adminPageSource, adminPanelSource, upgradePageSource] = await Promise.all([
    readFile(path.resolve(process.cwd(), "src/app/actions.ts"), "utf8"),
    readFile(
      path.resolve(process.cwd(), "src/app/dashboard/admin/page.tsx"),
      "utf8"
    ),
    readFile(
      path.resolve(
        process.cwd(),
        "src/components/admin-provisioned-boxes-panel.tsx"
      ),
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

  assert.doesNotMatch(adminPageSource, /supports_runtime_control/u);
  assert.doesNotMatch(adminPanelSource, /supports_runtime_control/u);
  assert.doesNotMatch(upgradePageSource, /supports_runtime_control/u);
  assert.doesNotMatch(actionsSource, /supports_runtime_control/u);

  assert.match(adminPageSource, /<AdminProvisionedBoxesPanel/u);
  assert.match(adminPageSource, /finitePrivateState=\{finitePrivate\.state\}/u);
  assert.match(adminPanelSource, /const canRestart = runtimeSupports\(runtime, "restart"\)/u);
  assert.match(adminPanelSource, /disabled=\{!canRestart\}/u);
  assert.match(adminPanelSource, /const canRecover = runtimeSupports\(runtime, "recover_known_good_chat"\)/u);
  assert.match(adminPanelSource, /disabled=\{!canRecover\}/u);
  assert.match(adminPanelSource, /const canUpgrade = runtimeSupports\(runtime, "runtime_upgrade"\)/u);
  assert.match(adminPanelSource, /\{canUpgrade \? \(/u);

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
  assert.doesNotMatch(adminPageSource, /name="targetRuntimeArtifactId"/u);
  assert.match(
    adminPanelSource,
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

function adminRuntimeFixture(): AdminOpsRuntime {
  return {
    project_display_name: "Agent M",
    owner_email: "owner@example.test",
    project_id: "project_agent_m",
    agent_runtime_id: "runtime_agent_m",
    source_host_id: "finite-lat-3",
    source_machine_id: "finite-kata-b4a553a277f06141b934",
    runtime_artifact_id: "artifact_agent_m",
    runtime_artifact_version_label: "candidate-2026-07-02",
    runtime_status: "online",
    last_heartbeat_at: "2026-07-02T12:00:00Z",
    hermes_available: true,
    published_app_urls: [],
    active_finite_private_key_count: 1,
    runtime_link_active: true,
    runtime_capabilities: {
      restart: true,
      recover_known_good_chat: true,
      runtime_upgrade: true,
    },
  };
}

function grantFixture(id: string, userId: string) {
  return {
    id,
    user_id: userId,
    limit_profile_id: "friend_daily",
    status: "active" as const,
    current_window_started_at: "2026-07-02T12:00:00Z",
    current_window_used_units: 123,
  };
}

function sourceBetween(source: string, start: string, end: string) {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);
  assert.notEqual(startIndex, -1, `missing source marker: ${start}`);
  assert.notEqual(endIndex, -1, `missing source marker: ${end}`);
  return source.slice(startIndex, endIndex);
}
