import test from "node:test";
import assert from "node:assert/strict";
import { advanceEmailChange, type EmailChangeDependencies, type EmailChangePreview, type EmailChangeRequest } from "./account-email-change";

const request: EmailChangeRequest = { operationId: "change-1", userId: "core-1", workosUserId: "user-1", expectedEmail: "before@example.test", newEmail: "after@example.test", evidenceReference: "test-request" };
function fixture() {
  let email = request.expectedEmail;
  let status = "unprepared";
  const calls: string[] = [];
  const preview = (): EmailChangePreview => ({ request, status, blockers: [], projectIds: ["existing-agent"], customerOrgIds: ["existing-org"], finitePrivateGrantIds: [], externalChecksRequired: [] });
  const deps: EmailChangeDependencies = {
    core: async (action) => { calls.push(action); if (action === "prepare") status = "prepared"; if (action === "complete") status = "completed"; return preview(); },
    user: async () => { calls.push("user"); return { id: request.workosUserId, email, email_verified: true }; },
    occupied: async () => { calls.push("occupied"); return false; },
    send: async () => { calls.push("send"); },
    confirm: async () => { calls.push("confirm"); email = request.newEmail; },
  };
  return { deps, calls, setEmail: (value: string) => { email = value; } };
}

test("Core denial prevents all provider access, including forged direct send/confirm calls", async () => {
  for (const action of ["review", "send", "confirm", "resume"] as const) {
    const f = fixture(); f.deps.core = async () => { throw new Error("forbidden"); };
    await assert.rejects(advanceEmailChange(f.deps, action, request, "123456"), /forbidden/);
    assert.deepEqual(f.calls, []);
  }
});
test("occupied provider destination is never prepared or mutated", async () => {
  const f = fixture(); f.deps.occupied = async () => true;
  await assert.rejects(advanceEmailChange(f.deps, "send", request), /occupied/);
  assert.deepEqual(f.calls, ["preview", "user"]);
});
test("send persists intent before provider mutation; confirm rereads identity before completing", async () => {
  const f = fixture();
  await advanceEmailChange(f.deps, "send", request);
  assert.deepEqual(f.calls, ["preview", "user", "occupied", "prepare", "send"]);
  f.calls.length = 0;
  const result = await advanceEmailChange(f.deps, "confirm", request, "123456");
  assert.equal(result.preview.status, "completed");
  assert.deepEqual(f.calls, ["preview", "user", "occupied", "confirm", "user", "complete"]);
  assert.equal(result.preview.request.userId, "core-1");
});
test("lost successful provider response can resume without resending or confirming again", async () => {
  const f = fixture(); await advanceEmailChange(f.deps, "send", request);
  f.deps.confirm = async () => { f.setEmail(request.newEmail); throw new Error("timeout"); };
  await assert.rejects(advanceEmailChange(f.deps, "confirm", request, "123456"), /timeout/);
  f.calls.length = 0;
  assert.equal((await advanceEmailChange(f.deps, "resume", request)).preview.status, "completed");
  assert.deepEqual(f.calls, ["preview", "user", "complete"]);
});
test("Core failure after provider success remains resumable", async () => {
  const f = fixture(); await advanceEmailChange(f.deps, "send", request);
  const core = f.deps.core;
  f.deps.core = async (action, input) => { if (action === "complete") throw new Error("Core unavailable"); return core(action, input); };
  await assert.rejects(advanceEmailChange(f.deps, "confirm", request, "123456"), /Core unavailable/);
  f.deps.core = core;
  assert.equal((await advanceEmailChange(f.deps, "resume", request)).preview.status, "completed");
});
test("wrong subject, unexpected email, and unverified provider responses fail before mutation", async () => {
  for (const user of [
    { id: "different-user", email: request.expectedEmail, email_verified: true },
    { id: request.workosUserId, email: "unexpected@example.test", email_verified: true },
    { id: request.workosUserId, email: request.expectedEmail, email_verified: false },
  ]) {
    const f = fixture(); f.deps.user = async () => user;
    await assert.rejects(advanceEmailChange(f.deps, "send", request));
    assert.deepEqual(f.calls, ["preview"]);
  }
});
test("invalid code and unprepared operation cannot confirm", async () => {
  const f = fixture();
  await assert.rejects(advanceEmailChange(f.deps, "confirm", request, "123456"), /Send a verification/);
  await advanceEmailChange(f.deps, "send", request);
  await assert.rejects(advanceEmailChange(f.deps, "confirm", request, "bad"), /six-digit/);
  assert.ok(!f.calls.includes("confirm"));
});
test("completed replay has no provider side effects", async () => {
  const f = fixture(); await advanceEmailChange(f.deps, "send", request);
  await advanceEmailChange(f.deps, "confirm", request, "123456"); f.calls.length = 0;
  await advanceEmailChange(f.deps, "send", request);
  assert.deepEqual(f.calls, ["preview"]);
});
