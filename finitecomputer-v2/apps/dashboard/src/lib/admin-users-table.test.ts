import assert from "node:assert/strict";
import test from "node:test";
import { adminRuntimeMatchesSearch, type RuntimeFinitePrivateGrantSummary } from "./admin-ops";
import { adminUserAuditEvents, adminUserTableCells, compareAdminTableCells } from "./admin-users-table";
import type { CoreAdminRuntimeOverview } from "./core-client";

const runtime: CoreAdminRuntimeOverview = {
  project_id: "project-one", project_display_name: "Research bot",
  owner_email: "owner@example.test", agent_runtime_id: "runtime-one",
  source_host_id: "host-one", source_machine_id: "machine-one",
  runtime_status: "online", runtime_updated_at: "2026-09-18T10:30:00Z",
  runtime_health: { status: "not_ready", reason: "Waiting for heartbeat" },
  hermes_available: false, runtime_link_active: false,
  active_finite_private_key_count: 0, published_app_urls: ["https://research.example.test"],
};
const grant: RuntimeFinitePrivateGrantSummary = {
  grantId: "grant-one", grantStatus: "active", grantUserId: "user-one",
  limitProfileId: "custom-profile", currentWindowStartedAt: "2026-09-18T09:00:00Z",
  currentWindowUsedUnits: 1234567, keyId: "key-one", keyStatus: "active",
  keyProjectId: "project-one", keyAgentRuntimeId: "runtime-one", matchScope: "runtime",
};

test("every displayed admin table value is searchable, including formatted usage and health", () => {
  const cells = adminUserTableCells(runtime, null, grant);
  const values = cells.map((cell) => cell.value);
  for (const value of values) {
    assert.equal(adminRuntimeMatchesSearch(runtime, null, value, grant, values), true, value);
  }
  for (const query of ["OWNER@EXAMPLE.TEST 1,234,567", "waiting heartbeat unlinked", "1234567 research"] ) {
    assert.equal(adminRuntimeMatchesSearch(runtime, null, query, grant, values), true, query);
  }
  assert.equal(adminRuntimeMatchesSearch(runtime, null, "other@example.test", grant, values), false);
});

test("missing usage remains unavailable while reported zero and false values remain visible", () => {
  const cells = new Map(adminUserTableCells(runtime, null, null).map((cell) => [cell.label, cell.value]));
  assert.equal(cells.get("Usage (weighted tokens)"), "Unavailable");
  assert.equal(cells.get("Name"), "Unavailable");
  assert.equal(cells.get("Active keys"), "0");
  assert.equal(cells.get("Hermes"), "No");
  assert.equal(cells.get("Runtime link"), "Unlinked");
  const zero = adminUserTableCells(runtime, null, { ...grant, currentWindowUsedUnits: 0 });
  assert.equal(zero.find((cell) => cell.label === "Usage (weighted tokens)")?.value, "0");
});

test("audit activity uses exact identifiers and excludes unrelated accounts", () => {
  const event = { id: "event-one", action: "reset", actor: "operator", target_type: "grant", target_id: grant.grantId, created_at: "2026-09-18T12:00:00Z", metadata: {} };
  const events = adminUserAuditEvents(runtime, grant, { grants: [], apiKeys: [], adminAuditEvents: [event, { ...event, id: "unrelated", target_id: "another-grant" }] });
  assert.deepEqual(events.map((item) => item.id), ["event-one"]);
  assert.deepEqual(adminUserAuditEvents(runtime, grant, null), []);
});

test("additional operational fields remain searchable without exposing key hashes", () => {
  const detailedRuntime = { ...runtime, offboarding_phase: "retiring", runtime_health: { status: "ready" as const, report_interval_seconds: 30 }, runtime_capabilities: { restart: true, stop: false } };
  const cells = adminUserTableCells(detailedRuntime, null, grant);
  const values = cells.map((cell) => cell.value);
  for (const query of ["retiring", "restart: yes", "stop: no", "30"]) {
    assert.equal(adminRuntimeMatchesSearch(detailedRuntime, null, query, grant, values), true);
  }
  assert.equal(cells.some((cell) => /hash/i.test(cell.label)), false);
});

test("column sorting handles numeric usage, natural text, timestamps and missing values", () => {
  const cell = (value: string) => ({ value });
  assert.ok(compareAdminTableCells(cell("9,000"), cell("100,000"), "ascending") < 0);
  assert.ok(compareAdminTableCells({ value: "90%", usage: { usedUnits: 900 } }, { value: "50%", usage: { usedUnits: 5000 } }, "ascending") < 0);
  assert.ok(compareAdminTableCells(cell("Agent 2"), cell("Agent 10"), "ascending") < 0);
  assert.ok(compareAdminTableCells(cell("2026-09-18T10:00:00+02:00"), cell("2026-09-18T09:00:00Z"), "ascending") < 0);
  assert.ok(compareAdminTableCells(cell("9"), cell("100"), "descending") > 0);
  for (const direction of ["ascending", "descending"] as const) {
    assert.ok(compareAdminTableCells(cell("Unavailable"), cell("0"), direction) > 0);
    assert.equal(compareAdminTableCells(cell("same"), cell("same"), direction), 0);
  }
});
