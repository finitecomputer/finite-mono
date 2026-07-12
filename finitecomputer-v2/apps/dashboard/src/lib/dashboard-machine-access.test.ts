import assert from "node:assert/strict";
import test from "node:test";

import type { CoreVisibleProject } from "./core-client";
import { coreProjectOverviewHref } from "./dashboard-machine-access";

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
