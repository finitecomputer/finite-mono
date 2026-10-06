import assert from "node:assert/strict";
import { test } from "node:test";
import { HostedHermesStatusError } from "./hosted-hermes-status";
import { CONSENT_REFUSED_MESSAGE, PersonalBrainSetupError, UPDATE_AGENT_MESSAGE, setUpPersonalBrain } from "./personal-brain-setup-flow";

const owner = `npub1${"q".repeat(58)}`;
const agent = `npub1${"p".repeat(58)}`;
const brainId = "personal-0123456789abcdef";
const consent = {
  id: "a".repeat(64), pubkey: "b".repeat(64), created_at: 1_790_000_000, kind: 30_078,
  tags: [["d", brainId]], content: "{}", sig: "d".repeat(128),
};

function harness({ ownerStatus = 200, createStatus = 200, agentResult = { version: 1, agentNpub: agent, ownerNpub: owner, brainId, consent } as unknown } = {}) {
  const calls: { path: string; init?: RequestInit }[] = [];
  const dependencies = {
    fetch: async (path: string, init: RequestInit) => {
      calls.push({ path, init });
      if (init.method === "POST") {
        return Response.json(createStatus === 200 ? { brainId } : { error: "Personal Brain already exists." }, { status: createStatus });
      }
      return Response.json(ownerStatus === 200 ? { ownerNpub: owner } : { error: "Open chat once." }, { status: ownerStatus });
    },
    readAgentJson: async (runtimeId: string, path: string) => {
      calls.push({ path: `${runtimeId}:${path}` });
      if (agentResult instanceof Error) throw agentResult;
      return agentResult;
    },
  };
  return { calls, run: () => setUpPersonalBrain("runtime_1", new AbortController().signal, dependencies) };
}

test("Setup reads the owner, asks the selected Agent to consent to that owner, then creates", async () => {
  const { calls, run } = harness();
  assert.equal(await run(), brainId);
  assert.deepEqual(calls.map(call => call.path), [
    "/api/brain/personal",
    `runtime_1:api/plugins/finite-brain/personal-agent-consent/${owner}`,
    "/api/brain/personal",
  ]);
  assert.equal(calls[0].init?.method, undefined);
  assert.equal(calls[2].init?.method, "POST");
  assert.deepEqual(JSON.parse(String(calls[2].init?.body)), { agentNpub: agent, brainId, consent });
});

test("An Agent runtime without the consent route or with another owner's consent is asked to update", async () => {
  for (const agentResult of [
    new HostedHermesStatusError("This agent does not support this page yet.", "unsupported"),
    { version: 1, agentNpub: agent, ownerNpub: `npub1${"z".repeat(58)}`, brainId, consent },
  ]) {
    const { calls, run } = harness({ agentResult });
    await assert.rejects(run, (error: Error) => error.message === UPDATE_AGENT_MESSAGE);
    assert.equal(calls.filter(call => call.init?.method === "POST").length, 0);
  }
  const access = harness({ agentResult: new HostedHermesStatusError("Access to this agent is no longer available.", "access") });
  await assert.rejects(access.run, (error: Error) => error.message === CONSENT_REFUSED_MESSAGE);
});

test("Route errors are shown as sent, and only a create conflict suggests the Brain may exist", async () => {
  const setupRequired = harness({ ownerStatus: 409 });
  await assert.rejects(setupRequired.run, (error: PersonalBrainSetupError) => error.message === "Open chat once." && !error.mayExist);
  assert.equal(setupRequired.calls.length, 1);
  const conflict = harness({ createStatus: 409 });
  await assert.rejects(conflict.run, (error: PersonalBrainSetupError) => error.message === "Personal Brain already exists." && error.mayExist);
  const unavailable = harness({ createStatus: 502 });
  await assert.rejects(unavailable.run, (error: PersonalBrainSetupError) => !error.mayExist);
});
