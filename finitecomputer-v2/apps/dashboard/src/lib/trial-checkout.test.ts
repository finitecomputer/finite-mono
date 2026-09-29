import assert from "node:assert/strict";
import test from "node:test";
import { createServer } from "node:http";
import { once } from "node:events";
import { trialCheckoutDestination, trialCheckoutParams } from "./trial-checkout";
import { requireStripeClient } from "./stripe-billing";

test("event checkout starts a real trial on the monthly price and retains attribution", () => {
  const before = Math.floor(Date.now() / 1000);
  const params = trialCheckoutParams({
    stripeCustomerId: "cus_trial", customerOrgId: "org_trial", priceId: "price_standard",
    successUrl: "https://finite.computer/dashboard?billing=success",
    cancelUrl: "https://finite.computer/dashboard?billing=cancelled",
  }, { campaignId: "campaign_summit", trialDays: 7 }, "attempt_1");
  assert.equal(params.mode, "subscription");
  assert.deepEqual(params.line_items, [{ price: "price_standard", quantity: 1 }]);
  assert.equal(params.subscription_data?.trial_period_days, 7);
  assert.equal(params.payment_method_collection, "always");
  assert.equal(params.allow_promotion_codes, false);
  assert.equal(params.discounts, undefined);
  assert.equal(params.metadata?.finite_customer_org_id, "org_trial");
  assert.equal(params.metadata?.finite_trial_campaign_id, "campaign_summit");
  assert.equal(params.metadata?.finite_trial_attempt_id, "attempt_1");
  assert.deepEqual(params.subscription_data?.metadata, params.metadata);
  assert(params.expires_at! >= before + 30 * 60);
  assert(params.expires_at! <= before + 32 * 60);
});

test("checkout exposes only reserved sessions and expires rejected or duplicate attempts", async (t) => {
  const saved = { ...process.env };
  t.after(() => { process.env = saved; });
  let reject = false;
  let reuse = false;
  let reserved = false;
  const server = createServer(async (request, response) => {
    let raw = ""; for await (const chunk of request) raw += chunk;
    response.setHeader("content-type", "application/json");
    if (request.url === "/api/core/v1/me/billing/trial-offer") {
      assert.equal(request.headers.authorization, "Bearer account-fixture");
      response.end(JSON.stringify({ campaignId: "campaign", trialDays: 14 }));
    } else if (request.url === "/api/core/v1/billing/trial-reservation") {
      assert.equal(request.headers.authorization, "Bearer service-fixture");
      const body = JSON.parse(raw);
      assert.equal(body.workosUserId, "user_fixture");
      assert.equal(body.reservation.stripeSessionId, "cs_new");
      assert.equal(body.reservation.trialDays, 14);
      reserved = !reject;
      response.statusCode = reject ? 409 : 200;
      response.end(JSON.stringify(reject ? { error: "All seats claimed" } : { stripeSessionId: reuse ? "cs_existing" : "cs_new" }));
    } else { response.statusCode = 404; response.end("{}"); }
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  t.after(async () => { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); });
  const address = server.address(); assert(address && typeof address !== "string");
  Object.assign(process.env, {
    NODE_ENV: "development", FC_WORKOS_AUTH_ENABLED: "0", FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
    FC_DASHBOARD_DEV_EMAIL: "trial@example.test", FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_fixture",
    FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "account-fixture", FC_CORE_API_TOKEN: "service-fixture",
    FC_CORE_BASE_URL: `http://127.0.0.1:${address.port}`, STRIPE_SECRET_KEY: "sk_test_fixture",
    STRIPE_WEBHOOK_SECRET: "whsec_fixture", STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID: "price_standard",
    FC_DASHBOARD_BASE_URL: "https://finite.computer",
  });
  const stripe = requireStripeClient();
  const expired: string[] = [];
  let priceAmount = 20000;
  let created = 0;
  t.mock.method(stripe.prices, "retrieve", async () => ({ active: true, currency: "usd", unit_amount: priceAmount, recurring: { interval: "month", interval_count: 1 } }));
  t.mock.method(stripe.checkout.sessions, "create", async (params: { subscription_data: { trial_period_days: number } }) => {
    assert.equal(params.subscription_data.trial_period_days, 14);
    created++;
    return { id: "cs_new", status: "open", url: "https://checkout.stripe.com/new", expires_at: Math.floor(Date.now() / 1000) + 1860 };
  });
  t.mock.method(stripe.checkout.sessions, "expire", async (id: string) => { expired.push(id); });
  t.mock.method(stripe.checkout.sessions, "retrieve", async (id: string) => {
    assert(reserved); assert.equal(id, "cs_existing");
    return { status: "open", url: "https://checkout.stripe.com/existing" };
  });
  const input = { stripeCustomerId: "cus_trial", customerOrgId: "org_trial", priceId: "price_standard", successUrl: "https://finite.computer/dashboard", cancelUrl: "https://finite.computer/dashboard" };
  assert.equal(await trialCheckoutDestination("event", input), "https://checkout.stripe.com/new");
  assert(reserved); assert.deepEqual(expired, []);
  reuse = true;
  assert.equal(await trialCheckoutDestination("event", input), "https://checkout.stripe.com/existing");
  reject = true;
  await assert.rejects(trialCheckoutDestination("event", input), /All seats claimed/);
  assert.deepEqual(expired, ["cs_new", "cs_new"]);
  priceAmount = 19900;
  await assert.rejects(trialCheckoutDestination("event", input), /plan is unavailable/);
  assert.equal(created, 3, "incorrect price is rejected before creating checkout");
});
