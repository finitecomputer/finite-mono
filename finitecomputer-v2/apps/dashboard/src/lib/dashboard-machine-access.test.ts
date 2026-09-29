import assert from "node:assert/strict";
import test from "node:test";
import { once } from "node:events";

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
  } as unknown as CoreVisibleProject;

  assert.equal(coreProjectOverviewHref(project), "/dashboard/machines/runtime-1");
  assert.equal(coreProjectOverviewHref({ ...project, runtime: null }), null);
});

test("machine recovery route identity comes from one Core snapshot", () => {
  const current = {
    project: { id: "project-current" },
    runtime: { id: "runtime-current", source_machine_id: "legacy-current" },
  } as unknown as CoreVisibleProject;
  const changed = {
    project: { id: "project-changed" },
    runtime: { id: "runtime-changed", source_machine_id: "legacy-changed" },
  } as unknown as CoreVisibleProject;
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


test("dashboard access checks fresh billing and only blocks marked trial accounts", async (t) => {
  const { createServer } = await import("node:http");
  const { loadDashboardMachineAccess } = await import("./dashboard-machine-access");
  const saved = { ...process.env };
  t.after(() => { process.env = saved; });
  let blocked = false;
  let unavailable = false;
  let checks = 0;
  const server = createServer((request, response) => {
    response.setHeader("content-type", "application/json");
    if (request.url === "/api/core/v1/me/billing") {
      checks++;
      response.statusCode = unavailable ? 503 : 200;
      response.end(JSON.stringify(unavailable ? { error: "unavailable" } : {
        customer_org: { id: "org" }, trial_access: { blocked },
      }));
    } else if (request.url === "/api/core/v1/me") {
      response.end(JSON.stringify({ projects: [{ project: { id: "p", display_name: "Trial" }, runtime: { id: "r" } }] }));
    } else { response.statusCode = 404; response.end("{}"); }
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(async () => { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); });
  const address = server.address(); assert(address && typeof address !== "string");
  process.env = { ...process.env, NODE_ENV: "development", FC_WORKOS_AUTH_ENABLED: "0",
    FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1", FC_DASHBOARD_DEV_EMAIL: "trial@example.com",
    FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_trial", FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "fixture",
    FC_CORE_BASE_URL: `http://127.0.0.1:${address.port}` };
  assert(await loadDashboardMachineAccess("r", { coreCacheMode: "swr" }));
  blocked = true;
  assert.equal(await loadDashboardMachineAccess("r", { coreCacheMode: "swr" }), null);
  blocked = false;
  assert(await loadDashboardMachineAccess("r", { coreCacheMode: "swr" }));
  unavailable = true;
  assert.equal(await loadDashboardMachineAccess("r"), null);
  assert.equal(checks, 4);
});
