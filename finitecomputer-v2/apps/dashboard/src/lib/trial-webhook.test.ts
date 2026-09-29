import assert from "node:assert/strict";
import test from "node:test";
import { createServer } from "node:http";
import { once } from "node:events";
import { POST } from "@/app/api/stripe/webhook/route";
import { requireStripeClient } from "./stripe-billing";

test("signed trial events preserve attribution, use trial end, and release only expired sessions", async (t) => {
  const saved = { ...process.env }; t.after(() => { process.env = saved; });
  const received: { path: string; body: Record<string, unknown> }[] = [];
  const server = createServer(async (request, response) => {
    let body = ""; for await (const chunk of request) body += chunk;
    received.push({ path: request.url!, body: JSON.parse(body) });
    response.setHeader("content-type", "application/json"); response.end("{}");
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  t.after(async () => { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); });
  const address = server.address(); assert(address && typeof address !== "string");
  Object.assign(process.env, {
    STRIPE_SECRET_KEY: "sk_test_local_fixture", STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID: "price_trial",
    STRIPE_WEBHOOK_SECRET: "whsec_local_fixture", FC_DASHBOARD_BASE_URL: "https://finite.computer",
    FC_CORE_BASE_URL: `http://127.0.0.1:${address.port}`, FC_CORE_API_TOKEN: "fixture",
  });
  const stripe = requireStripeClient();
  const subscription = {
    id: "sub_trial", customer: "cus_trial", status: "trialing", trial_end: 1_800_000_000,
    cancel_at_period_end: false,
    metadata: { finite_customer_org_id: "org_trial", finite_trial_attempt_id: "attempt_trial" },
    items: { data: [{ price: { id: "price_trial" }, current_period_end: 1_900_000_000 }] },
  };
  t.mock.method(stripe.subscriptions, "retrieve", async () => subscription);
  async function deliver(type: string, object: unknown, valid = true) {
    const body = JSON.stringify({ id: `evt_${received.length}`, type, created: 1_799_000_000, data: { object } });
    const signature = stripe.webhooks.generateTestHeaderString({ payload: body, secret: valid ? "whsec_local_fixture" : "wrong" });
    return POST(new Request("https://finite.computer/api/stripe/webhook", { method: "POST", body, headers: { "stripe-signature": signature } }));
  }
  assert.equal((await deliver("customer.subscription.created", subscription, false)).status, 400);
  assert.equal(received.length, 0);
  assert.equal((await deliver("customer.subscription.created", subscription)).status, 200);
  assert.equal(received[0].body.trialAttemptId, "attempt_trial");
  assert.equal(received[0].body.currentPeriodEnd, new Date(subscription.trial_end * 1000).toISOString());
  const session = { id: "cs_trial", customer: "cus_trial", mode: "subscription", subscription: "sub_trial", metadata: subscription.metadata };
  assert.equal((await deliver("checkout.session.completed", session)).status, 200);
  assert.equal(received[1].path, "/api/core/v1/billing/stripe/subscription");
  assert.equal((await deliver("checkout.session.expired", session)).status, 200);
  assert.deepEqual(received[2], { path: "/api/core/v1/billing/trial-expired", body: { stripeSessionId: "cs_trial", stripeCustomerId: "cus_trial" } });
  assert.equal((await deliver("checkout.session.expired", { ...session, metadata: {} })).status, 200);
  assert.equal(received.length, 3, "ordinary checkout expiry does not touch campaign seats");
});
