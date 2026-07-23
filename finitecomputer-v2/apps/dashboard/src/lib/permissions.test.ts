import test from "node:test";
import assert from "node:assert/strict";

import type { DashboardState, MachineRecord } from "./fc-dashboard";
import { canAccessPublishedAuth, canOperateMachineRecord, isAdminEmail, normalizeEmail, visibleMachinesForViewer } from "./permissions";

const dashboardState: DashboardState = {
  admins: ["paul@finite.vip", "austin@finite.vip"],
  invites: [],
};

function machine(overrides: Partial<MachineRecord> & { id: string; email: string }): MachineRecord {
  const workloadOwnerEmail = overrides.workload?.owner_email ?? overrides.email;
  return {
    workload: {
      id: overrides.id,
      owner: overrides.id,
      owner_email: workloadOwnerEmail,
      namespace: overrides.id,
      runtime_profile: "main",
      home_volume_size: "20Gi",
      opencode: {
        port: 4096,
        hostname: `${overrides.id}.finite.vip`,
        project_dir: "/home/node/.hermes",
        auth: overrides.workload?.opencode?.auth,
      },
      ssh: {
        enable: true,
        node_port: 32221,
      },
      ...overrides.workload,
    },
    invite: overrides.invite ?? null,
    ownerEmail: overrides.ownerEmail ?? overrides.email,
    siteUrl: overrides.siteUrl ?? `https://${overrides.id}.finite.vip`,
    runtimeProfile: overrides.runtimeProfile ?? "main",
    runtimeProfileLabel: overrides.runtimeProfileLabel ?? "Hermes Runtime",
    runtimeImage: overrides.runtimeImage ?? "fc-agent-runtime:main",
    runtimeBaseImage: overrides.runtimeBaseImage ?? overrides.runtimeImage ?? "fc-agent-runtime:main",
    authMode: overrides.authMode ?? "self",
    authSummary: overrides.authSummary ?? overrides.email,
    publishedEndpoints: overrides.publishedEndpoints ?? [],
  };
}

test("normalizeEmail lowercases and trims", () => {
  assert.equal(normalizeEmail(" Paul@Finite.VIP "), "paul@finite.vip");
  assert.equal(normalizeEmail(""), null);
});

test("admins see every machine", () => {
  const machines = [
    machine({ id: "fixture-owner", email: "fixture-owner@example.test" }),
    machine({ id: "test-finite", email: "test@finite.vip" }),
  ];

  assert.equal(isAdminEmail(dashboardState, "paul@finite.vip"), true);
  assert.deepEqual(
    visibleMachinesForViewer(machines, dashboardState, "paul@finite.vip").map((entry) => entry.workload.id),
    ["fixture-owner", "test-finite"],
  );
});

test("non-admin only sees owned or invited machines", () => {
  const machines = [
    machine({ id: "fixture-owner", email: "fixture-owner@example.test" }),
    machine({
      id: "test-finite",
      email: "someone-else@finite.vip",
      invite: {
        machineId: "test-finite",
        email: "test@finite.vip",
        displayName: "Test",
        claimToken: "test-token",
        createdAt: "2026-04-04T00:00:00Z",
      },
    }),
  ];

  assert.deepEqual(
    visibleMachinesForViewer(machines, dashboardState, "test@finite.vip").map((entry) => entry.workload.id),
    ["test-finite"],
  );
  assert.equal(canOperateMachineRecord(machines[0], dashboardState, "test@finite.vip"), false);
  assert.equal(canOperateMachineRecord(machines[1], dashboardState, "test@finite.vip"), true);
});

test("published auth self uses fallback owner email", () => {
  assert.equal(canAccessPublishedAuth({ mode: "self" }, "test@finite.vip", "test@finite.vip"), true);
  assert.equal(canAccessPublishedAuth({ mode: "self" }, "test@finite.vip", "paul@finite.vip"), false);
});

test("published auth emails requires explicit allowlist", () => {
  assert.equal(
    canAccessPublishedAuth(
      { mode: "emails", emails: ["Paul@finite.vip", "austin@finite.vip"] },
      null,
      "paul@finite.vip",
    ),
    true,
  );
  assert.equal(
    canAccessPublishedAuth(
      { mode: "emails", emails: ["Paul@finite.vip", "austin@finite.vip"] },
      null,
      "test@finite.vip",
    ),
    false,
  );
});

test("published auth org matches domain", () => {
  assert.equal(
    canAccessPublishedAuth({ mode: "org", org_domain: "finitesupply.xyz" }, null, "skyler@finitesupply.xyz"),
    true,
  );
  assert.equal(
    canAccessPublishedAuth({ mode: "org", org_domain: "finitesupply.xyz" }, null, "paul@finite.vip"),
    false,
  );
});

test("published auth public always allows", () => {
  assert.equal(canAccessPublishedAuth({ mode: "public" }, null, null), true);
  assert.equal(canAccessPublishedAuth({ mode: "public" }, null, "anyone@example.com"), true);
});
