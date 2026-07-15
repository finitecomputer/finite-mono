import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import { dashboardChatMachineIdFromPath } from "./dashboard-chat-route";

test("extracts a machine id from an exact dashboard chat route", () => {
  assert.equal(
    dashboardChatMachineIdFromPath("/dashboard/machines/runtime_47195f2e23c41a4f6dfa/chat"),
    "runtime_47195f2e23c41a4f6dfa",
  );
  assert.equal(
    dashboardChatMachineIdFromPath("/dashboard/machines/runtime_47195f2e23c41a4f6dfa/chat/"),
    "runtime_47195f2e23c41a4f6dfa",
  );
});

test("decodes the machine id path segment", () => {
  assert.equal(
    dashboardChatMachineIdFromPath("/dashboard/machines/runtime%5Fexample/chat"),
    "runtime_example",
  );
});

test("rejects non-chat and nested chat routes", () => {
  assert.equal(dashboardChatMachineIdFromPath("/dashboard/machines/runtime_example"), null);
  assert.equal(dashboardChatMachineIdFromPath("/dashboard/machines/runtime_example/chat/topic"), null);
  assert.equal(dashboardChatMachineIdFromPath("/dashboard/machines//chat"), null);
});

test("rejects a malformed encoded machine id", () => {
  assert.equal(dashboardChatMachineIdFromPath("/dashboard/machines/runtime%ZZ/chat"), null);
});

test("direct chat provides context from the route before the machine list refreshes", async () => {
  const dashboardShellSource = await readFile(
    new URL("../components/dashboard-shell.tsx", import.meta.url),
    "utf8",
  );

  assert.match(
    dashboardShellSource,
    /const chatMachineId = dashboardChatMachineIdFromPath\(pathname\)/u,
  );
  assert.match(
    dashboardShellSource,
    /<HostedChatProvider key=\{chatMachineId\} machineId=\{chatMachineId\}>/u,
  );
  assert.doesNotMatch(
    dashboardShellSource,
    /isChatSurface && activeMachine[\s\S]*<HostedChatProvider/u,
  );
});
