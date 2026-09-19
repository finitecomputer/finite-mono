import assert from "node:assert/strict";
import { test } from "node:test";
import { activeNavigationMachine, type MachineNavItem } from "./dashboard-machine-navigation";

test("first chat renders agent navigation before the layout fleet catches up", () => {
  const machine = activeNavigationMachine([], "runtime_new", "runtime_new", true);
  assert.equal(machine?.id, "runtime_new");
  assert.equal(machine?.runtimeStatus, "unknown");
});

test("a new agent route never falls back to another existing agent", () => {
  const existing: MachineNavItem = { id: "runtime_existing", ownerLabel: "Moss", runtimeStatus: "online" };
  assert.equal(activeNavigationMachine([existing], "runtime_new", "runtime_new", true)?.id, "runtime_new");
  assert.equal(activeNavigationMachine([existing], existing.id, existing.id, true), existing);
});

test("an unknown query selection and non-SaaS route do not synthesize agent navigation", () => {
  assert.equal(activeNavigationMachine([], "runtime_unknown", null, true), null);
  assert.equal(activeNavigationMachine([], "runtime_unknown", "runtime_unknown", false), null);
});
