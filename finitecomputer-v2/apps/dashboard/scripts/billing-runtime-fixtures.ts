/** Test-only contract inputs and deterministic inference, never payment/model evidence. */
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { once } from "node:events";

export function subscriptionFixture(input: {
  run: string; customerOrgId: string; status: "trialing" | "past_due" | "active";
  sequence: number; now: number;
}) {
  assert(input.run && input.customerOrgId && Number.isSafeInteger(input.now));
  assert(Number.isSafeInteger(input.sequence) && input.sequence > 0);
  return {
    trialAttemptId: input.run, customerOrgId: input.customerOrgId,
    stripeCustomerId: `cus_contract_${input.run}`, stripeSubscriptionId: `sub_contract_${input.run}`,
    stripePriceId: "price_contract_fixture", subscriptionStatus: input.status,
    currentPeriodEnd: new Date((input.now + (input.status === "past_due" ? -60 : 7 * 86400)) * 1000).toISOString(),
    cancelAtPeriodEnd: false, stripeEventId: `evt_contract_${input.run}_${input.sequence}`,
    stripeEventCreated: input.now + input.sequence,
  };
}

// Never inherit payment, cloud or inference credentials into this disposable stack.
// Nix's build environment is retained; none of these values is a product API key.
export function fixtureEnvironment(source: Record<string, string | undefined>): Record<string, string | undefined> {
  const safe: Record<string, string | undefined> = {};
  for (const [key, value] of Object.entries(source)) {
    if (/^(PATH|HOME|USER|LOGNAME|SHELL|TMPDIR|TMP|TEMP|LANG|LC_ALL|IN_NIX_SHELL|PNPM_HOME|PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH|BILLING_SMOKE_ROOT|BILLING_SMOKE_IMAGE)$/.test(key)
      || /^(NIX_|PKG_CONFIG_|LD_LIBRARY_PATH$|DYLD_LIBRARY_PATH$)/.test(key)) safe[key] = value;
  }
  return safe;
}

/** Same chat-completions/SSE fixture pattern as hermes-chat-interruption-docker-smoke.py.
 * Real Hermes and its crypto/chat bridge consume these replies; no model executes.
 * Bound to the disposable worker so Docker's host-gateway can reach it.
 */
export async function startModelFixture(maxRequests = 32, host = "0.0.0.0") {
  const token = `local-fixture-${randomUUID()}`;
  let requests = 0;
  const server = createServer(async (request, response) => {
    const fail = (code: number) => { response.writeHead(code); response.end(); };
    if (request.headers.authorization !== `Bearer ${token}`) return fail(401);
    if (request.method === "GET" && request.url === "/control/usage") {
      response.setHeader("content-type", "application/json");
      return response.end(JSON.stringify({ notice: null }));
    }
    if (request.method === "GET" && request.url === "/v1/models") {
      response.setHeader("content-type", "application/json");
      return response.end(JSON.stringify({ object: "list", data: [{ id: "glm-5-3-flash", object: "model" }] }));
    }
    if (request.method !== "POST" || request.url !== "/v1/chat/completions") return fail(404);
    if (++requests > maxRequests) return fail(429);
    try {
      const chunks: Buffer[] = []; let size = 0;
      for await (const chunk of request) {
        size += chunk.length; if (size > 1024 * 1024) return fail(413);
        chunks.push(Buffer.from(chunk));
      }
      const payload = JSON.parse(Buffer.concat(chunks).toString());
      const user = (payload.messages as {role: string; content: unknown}[] | undefined)?.findLast(m => m.role === "user");
      const content = typeof user?.content === "string" ? user.content : JSON.stringify(user?.content ?? "");
      const reply = /Reply with exactly:\s*([a-zA-Z0-9-]+)/.exec(content)?.[1] ?? "finite deterministic reply";
      const common = { id: `chatcmpl-fixture-${requests}`, created: 0, model: payload.model };
      if (payload.stream) {
        response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
        response.end(`data: ${JSON.stringify({ ...common, object: "chat.completion.chunk", choices: [{ index: 0, delta: { role: "assistant", content: reply }, finish_reason: "stop" }] })}\n\ndata: [DONE]\n\n`);
      } else {
        response.setHeader("content-type", "application/json");
        response.end(JSON.stringify({ ...common, object: "chat.completion", choices: [{ index: 0, message: { role: "assistant", content: reply }, finish_reason: "stop" }] }));
      }
    } catch { fail(400); }
  });
  server.requestTimeout = 10_000;
  server.listen(0, host); await once(server, "listening");
  const address = server.address(); assert(address && typeof address !== "string");
  return { token, port: address.port, count: () => requests, close: async () => {
    server.closeAllConnections(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  } };
}

export function assertFixtureInference(environment: string[], port: number) {
  const env = Object.fromEntries(environment.map(item => { const i = item.indexOf("="); return [item.slice(0, i), item.slice(i + 1)]; }));
  const origin = `http://host.docker.internal:${port}`;
  for (const key of ["FINITE_PRIVATE_BASE_URL", "FINITECHAT_HERMES_BASE_URL"]) assert.equal(env[key], `${origin}/v1`, "model must use the local fixture");
  assert.equal(env.FINITE_PRIVATE_CONTROL_URL, `${origin}/control`, "usage control must use the local fixture");
  assert.equal(env.FINITECHAT_HERMES_PROVIDER, "custom");
  assert.equal(env.FINITECHAT_HERMES_API_MODE, "chat_completions");
  assert(env.FINITE_PRIVATE_API_KEY?.startsWith("local-fixture-"), "external inference key forbidden");
  assert.equal(env.OPENAI_API_KEY, env.FINITE_PRIVATE_API_KEY);
}
