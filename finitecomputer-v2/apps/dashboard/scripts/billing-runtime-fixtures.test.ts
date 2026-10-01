import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fixtureEnvironment, startModelFixture, subscriptionFixture, assertFixtureInference } from "./billing-runtime-fixtures";

test("billing contracts retain identity and monotonic event ordering through expiry/recovery", () => {
  const input = { run: "isolated", customerOrgId: "org", now: 1_800_000_000 };
  const states = (["trialing", "past_due", "active"] as const).map((status, index) => subscriptionFixture({ ...input, status, sequence: index + 1 }));
  assert(states.every(s => s.stripeCustomerId === states[0].stripeCustomerId && s.stripeSubscriptionId === states[0].stripeSubscriptionId && s.trialAttemptId === input.run));
  assert.equal(Date.parse(states[1].currentPeriodEnd) < input.now * 1000, true);
  assert.equal(Date.parse(states[2].currentPeriodEnd) > input.now * 1000, true);
  assert.deepEqual(states.map(s => s.stripeEventCreated), [input.now + 1, input.now + 2, input.now + 3]);
  assert.deepEqual(subscriptionFixture({ ...input, status: "active", sequence: 3 }), states[2], "duplicate delivery must keep event identity");
  assert(!("now" in states[0]), "Core must use its real clock, not a fixture clock override");
});

test("isolated environment excludes external credentials and routes", () => {
  const env = fixtureEnvironment({ PATH: "/bin", HOME: "/home/ci", NIX_CONFIG: "build-config", BILLING_SMOKE_ROOT: "/tmp/run", STRIPE_SECRET_KEY: "forbidden", OPENAI_API_KEY: "forbidden", FINITE_PRIVATE_SMOKE_API_KEY: "forbidden", FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY: "forbidden", FC_RUNNER_FINITE_PRIVATE_BASE_URL: "https://external.invalid", WORKOS_API_KEY: "forbidden", AWS_SECRET_ACCESS_KEY: "forbidden" });
  assert.deepEqual(env, { PATH: "/bin", HOME: "/home/ci", NIX_CONFIG: "build-config", BILLING_SMOKE_ROOT: "/tmp/run" });
});

test("local model fixture handles authenticated streaming and nonstreaming with a hard request bound", async () => {
  const model = await startModelFixture(2, "127.0.0.1");
  try {
    const url = `http://127.0.0.1:${model.port}/v1/chat/completions`;
    const post = (stream: boolean, authorized = true) => fetch(url, { method: "POST", headers: { "content-type": "application/json", ...(authorized ? { authorization: `Bearer ${model.token}` } : {}) }, body: JSON.stringify({ model: "fixture", stream, messages: [{role: "user", content: "Reply with exactly: preserved-123"}] }) });
    assert.equal((await post(false, false)).status, 401);
    assert.equal(model.count(), 0);
    const usage = await fetch(`http://127.0.0.1:${model.port}/control/usage`, { headers: { authorization: `Bearer ${model.token}` } });
    assert.deepEqual(await usage.json(), { notice: null });
    const reply = await (await post(false)).json();
    assert.equal(reply.choices[0].message.content, "preserved-123");
    assert.equal(reply.choices[0].message.tool_calls, undefined);
    const streamed = await (await post(true)).text();
    assert(streamed.includes('"content":"preserved-123"'));
    assert(streamed.endsWith("data: [DONE]\n\n"));
    assert.equal((await post(false)).status, 429);
  } finally { await model.close(); }
});

test("billing workflow has no provider secrets and the harness uses Core HTTP rather than Stripe", async () => {
  const workflow = await readFile(new URL("../../../../.github/workflows/hermes-runtime-smoke.yml", import.meta.url), "utf8");
  const billingJob = workflow.split("\n  billing-runtime-smoke:")[1];
  assert(billingJob);
  assert(!/secrets\.|STRIPE_SECRET_KEY|FINITE_PRIVATE_SMOKE_API_KEY/.test(billingJob));
  const harness = await readFile(new URL("billing-runtime-smoke.ts", import.meta.url), "utf8");
  assert(harness.includes('"/api/core/v1/billing/stripe/subscription"'));
  assert(!/from "stripe"|stripe\.|psql|UPDATE customer|https:\/\/(api|checkout|billing)\.stripe/.test(harness));
});

test("effective container model and control destinations fail closed on external fallback", () => {
  const environment = ["FINITE_PRIVATE_BASE_URL=http://host.docker.internal:123/v1", "FINITECHAT_HERMES_BASE_URL=http://host.docker.internal:123/v1", "FINITE_PRIVATE_CONTROL_URL=http://host.docker.internal:123/control", "FINITECHAT_HERMES_PROVIDER=custom", "FINITECHAT_HERMES_API_MODE=chat_completions", "FINITE_PRIVATE_API_KEY=local-fixture-test", "OPENAI_API_KEY=local-fixture-test"];
  assertFixtureInference(environment, 123);
  assert.throws(() => assertFixtureInference(environment.filter(v => !v.startsWith("FINITE_PRIVATE_CONTROL_URL=")), 123));
  assert.throws(() => assertFixtureInference(environment.map(v => v.startsWith("FINITECHAT_HERMES_BASE_URL=") ? "FINITECHAT_HERMES_BASE_URL=https://external.invalid/v1" : v), 123));
});
