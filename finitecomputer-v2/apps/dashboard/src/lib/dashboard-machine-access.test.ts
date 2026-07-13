import assert from "node:assert/strict";
import test from "node:test";

import type { CoreVisibleProject } from "./core-client";
import {
  coreProjectOverviewHref,
  dashboardMachineProjectFromSnapshot,
} from "./dashboard-machine-access";

test("dashboard overview links use stable runtime ids, never provider machine aliases", () => {
  const project = {
    project: { id: "project-1" },
    runtime: {
      id: "runtime-1",
      source_machine_id: "legacy-provider-machine",
    },
  } as CoreVisibleProject;

  assert.equal(coreProjectOverviewHref(project), "/dashboard/machines/runtime-1");
  assert.equal(coreProjectOverviewHref({ ...project, runtime: null }), null);
});

test("machine recovery route identity comes from one Core snapshot", () => {
  const current = {
    project: { id: "project-current" },
    runtime: { id: "runtime-current", source_machine_id: "legacy-current" },
  } as CoreVisibleProject;
  const changed = {
    project: { id: "project-changed" },
    runtime: { id: "runtime-changed", source_machine_id: "legacy-changed" },
  } as CoreVisibleProject;
  const me = {
    projects: [current, changed],
  } as Parameters<typeof dashboardMachineProjectFromSnapshot>[0];

  assert.equal(dashboardMachineProjectFromSnapshot(me, "runtime-current"), current);
  assert.equal(dashboardMachineProjectFromSnapshot(me, "project-current"), current);
  assert.equal(dashboardMachineProjectFromSnapshot(me, "legacy-current"), current);
  assert.equal(
    dashboardMachineProjectFromSnapshot(me, "runtime-not-in-snapshot"),
    null
  );
});
