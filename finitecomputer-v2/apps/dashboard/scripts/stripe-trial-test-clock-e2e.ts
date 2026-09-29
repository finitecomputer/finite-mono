/** Run under devfinity's isolated Postgres profile with a test-only Stripe key.
 * Uses real Stripe Checkout and test clocks. Replays the resulting Stripe events
 * through our signed webhook handler locally; does not change webhook endpoints.
 * The printed Checkout URL needs one browser completion with Stripe's test card.
 */
import assert from "node:assert/strict";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import { createServer } from "node:net";
import { createWriteStream } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import Stripe from "stripe";
import { STRIPE_API_VERSION } from "../src/lib/stripe-billing";
import type { TrialCampaign, TrialAccess } from "../src/lib/trial-types";

type Billing = { trial_access?: TrialAccess; can_create_agent: boolean; billing_account?: { subscription_status: string } };
const root = path.resolve(process.cwd(), "../../..");
const runId = randomUUID();
const state = path.join(root, ".local-state", "stripe-trial-e2e", runId);
const key = process.env.STRIPE_SECRET_KEY?.trim() ?? "";
const dbName = `stripe_trial_${runId.replaceAll("-", "")}`;
const serviceToken = randomUUID();
const signingSecret = `whsec_${randomUUID()}`;
const processes: ChildProcess[] = [];
let stripe: Stripe;
let clockId: string | undefined;
let priceId: string | undefined;
let productId: string | undefined;
let coreUrl: string;
let customerJwt: string;
let operatorJwt: string;
let post: (r: Request) => Promise<Response>;
const replayed = new Set<string>();
let interrupted = false;
for (const signal of ["SIGINT", "SIGTERM"] as const) process.once(signal, () => { interrupted = true; });
const pause = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

async function port() {
  const server = createServer();
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const address = server.address(); assert(address && typeof address !== "string");
  await new Promise<void>(resolve => server.close(() => resolve()));
  return address.port;
}
function start(binary: string, args: string[], env: NodeJS.ProcessEnv, name: string) {
  const child = spawn(path.join(root, "target/debug", binary), args, { env, cwd: root });
  const log = createWriteStream(path.join(state, `${name}.log`), { mode: 0o600 });
  child.stdout.pipe(log); child.stderr.pipe(log);
  processes.push(child);
  child.on("error", () => { process.exitCode = 1; });
}
async function until<T>(description: string, fn: () => Promise<T | null>, seconds = 90): Promise<T> {
  const deadline = Date.now() + seconds * 1000;
  while (Date.now() < deadline) {
    assert(!interrupted, "Sandbox rehearsal interrupted");
    const value = await fn();
    if (value !== null) return value;
    await pause(2000);
  }
  throw new Error(`Timed out: ${description}`);
}
async function core<T>(url: string, token: string, body?: unknown): Promise<T> {
  const response = await fetch(coreUrl + url, { method: body === undefined ? "GET" : "POST", headers: {
    authorization: `Bearer ${token}`, "content-type": "application/json",
  }, body: body === undefined ? undefined : JSON.stringify(body) });
  assert(response.ok, `Core ${url}: HTTP ${response.status}`);
  return response.status === 204 ? undefined as T : await response.json() as T;
}
async function replay(type: Stripe.Event.Type, id: string) {
  const event = await until(`Stripe ${type} event`, async () => {
    const events = await stripe.events.list({ types: [type], limit: 100 });
    return events.data.find(e => "id" in e.data.object && e.data.object.id === id && !replayed.has(e.id)) ?? null;
  });
  assert.equal(event.livemode, false);
  const payload = JSON.stringify(event);
  const response = await post(new Request("http://localhost/api/stripe/webhook", { method: "POST", body: payload,
    headers: { "stripe-signature": stripe.webhooks.generateTestHeaderString({ payload, secret: signingSecret }) },
  }));
  assert.equal(response.status, 200, `webhook ${type}`);
  replayed.add(event.id);
}
async function billing(status: string, blocked: boolean) {
  const b = await core<Billing>("/api/core/v1/me/billing", customerJwt);
  assert.equal(b.billing_account?.subscription_status, status);
  assert.equal(b.trial_access?.blocked, blocked);
  assert.equal(b.can_create_agent, !blocked);
  console.log(`billing=${status}, dashboard_blocked=${blocked}`);
}
async function advance(time: number) {
  await stripe.testHelpers.testClocks.advance(clockId!, { frozen_time: time });
  await until("clock advance", async () => {
    const clock = await stripe.testHelpers.testClocks.retrieve(clockId!);
    assert.notEqual(clock.status, "internal_failure");
    return clock.status === "ready" ? clock : null;
  });
}
async function finalizeClockInvoice(subscription: string) {
  // Invoice finalization can be deferred by account webhook settings. Advance
  // to Stripe's advertised deadline rather than assuming one fixed hour.
  for (let attempt = 0; attempt < 3; attempt++) {
    const sub = await stripe.subscriptions.retrieve(subscription);
    const invoice = await stripe.invoices.retrieve(typeof sub.latest_invoice === "string" ? sub.latest_invoice : sub.latest_invoice!.id);
    if (invoice.status !== "draft") return;
    assert(invoice.automatically_finalizes_at, "Stripe must provide a finalization deadline");
    const clock = await stripe.testHelpers.testClocks.retrieve(clockId!);
    await advance(Math.max(clock.frozen_time + 1, invoice.automatically_finalizes_at + 1));
  }
}
async function main() {
  assert(/^(sk|rk)_test_/.test(key), "A test-only Stripe API key is required");
  const maintenance = new URL(process.env.FC_CORE_POSTGRES_TEST_URL ?? "");
  assert(["localhost", "127.0.0.1"].includes(maintenance.hostname), "Use isolated local devfinity Postgres");
  assert.equal(maintenance.port, process.env.DEVFINITY_POSTGRES_PORT);
  stripe = new Stripe(key, { apiVersion: STRIPE_API_VERSION });
  assert.equal((await stripe.balance.retrieve()).livemode, false);
  await mkdir(state, { recursive: true, mode: 0o700 });
  console.log(`sandbox_run=${runId}`);
  console.log(`evidence=${state}`);
  execFileSync("createdb", ["--maintenance-db", maintenance.toString(), dbName]);
  maintenance.pathname = `/${dbName}`;
  const workosUrl = `http://127.0.0.1:${await port()}`;
  coreUrl = `http://127.0.0.1:${await port()}`;
  const fixture = path.join(state, "workos");
  const childEnv: NodeJS.ProcessEnv = { NODE_ENV: "development", PATH: process.env.PATH, HOME: process.env.HOME, RUST_LOG: "warn" };
  start("devfinity", ["workos-fixture", "--listen", new URL(workosUrl).host, "--state-dir", fixture], childEnv, "workos");
  await until("WorkOS fixture", async () => {
    try { return (await fetch(`${workosUrl}/sso/jwks/client_devfinity`)).ok ? true : null; } catch { return null; }
  });
  customerJwt = (await readFile(path.join(fixture, "dashboard-customer.jwt"), "utf8")).trim();
  operatorJwt = (await readFile(path.join(fixture, "operator.jwt"), "utf8")).trim();
  productId = (await stripe.products.create({ name: `Finite trial rehearsal ${runId}`, metadata: { finite_e2e_run_id: runId } })).id;
  priceId = (await stripe.prices.create({ product: productId, unit_amount: 20000, currency: "usd", recurring: { interval: "month" }, metadata: { finite_e2e_run_id: runId } })).id;
  start("finite-saas-core", ["serve"], { ...childEnv,
    FC_CORE_DATABASE_URL: maintenance.toString(), FC_CORE_BIND: new URL(coreUrl).host,
    FC_CORE_API_TOKEN: serviceToken, FC_CORE_RUNNER_API_TOKEN: randomUUID(), FC_FINITE_PRIVATE_USAGE_API_TOKEN: randomUUID(),
    FC_CORE_STANDARD_STRIPE_PRICE_ID: priceId, WORKOS_CLIENT_ID: "client_devfinity",
    WORKOS_API_KEY: (await readFile(path.join(fixture, "workos-fixture-api-key"), "utf8")).trim(),
    WORKOS_API_BASE_URL: workosUrl, WORKOS_ISSUER: workosUrl, WORKOS_JWKS_URL: `${workosUrl}/sso/jwks/client_devfinity`,
    FC_WORKOS_OPERATOR_ORG_ID: "org_devfinity_operator",
  }, "core");
  await until("Core", async () => {
    try { return (await fetch(`${coreUrl}/healthz`)).ok ? true : null; } catch { return null; }
  });
  Object.assign(process.env, {
    FC_CORE_BASE_URL: coreUrl, FC_CORE_API_TOKEN: serviceToken, STRIPE_WEBHOOK_SECRET: signingSecret,
    STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID: priceId, FC_DASHBOARD_BASE_URL: coreUrl,
    NODE_ENV: "development", FC_WORKOS_AUTH_ENABLED: "0", FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
    FC_DASHBOARD_DEV_EMAIL: "devfinity@finite.computer", FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_devfinity",
    FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: customerJwt,
  });
  post = (await import("../src/app/api/stripe/webhook/route")).POST;
  const { trialCheckoutDestination } = await import("../src/lib/trial-checkout");
  const clock = await stripe.testHelpers.testClocks.create({ frozen_time: Math.floor(Date.now() / 1000), name: `Finite trial ${runId}` });
  clockId = clock.id;
  const customer = await stripe.customers.create({ test_clock: clock.id, email: `finite-trial-${runId}@example.test`, name: "Synthetic trial attendee", metadata: { finite_e2e_run_id: runId } });
  const org = await core<{customer_org_id: string}>("/api/core/v1/me/billing/stripe-customer", customerJwt, { stripeCustomerId: customer.id });
  const campaign = await core<{id: string; code: string}>("/api/core/v1/admin/trial-campaigns", operatorJwt, { name: `Sandbox ${runId}`, seatLimit: 1, trialDays: 7 });
  const input = { stripeCustomerId: customer.id, customerOrgId: org.customer_org_id, priceId,
    successUrl: `${coreUrl}/healthz?checkout=success`, cancelUrl: `${coreUrl}/healthz?checkout=cancelled` };
  const abandonedUrl = await trialCheckoutDestination(campaign.code, input);
  assert.equal(await trialCheckoutDestination(campaign.code, input), abandonedUrl, "retry reuses reserved session");
  const abandoned = (await stripe.checkout.sessions.list({ customer: customer.id, limit: 10 })).data.find(s => s.status === "open")!;
  assert(abandoned);
  await stripe.checkout.sessions.expire(abandoned.id);
  await replay("checkout.session.expired", abandoned.id);
  let campaigns = await core<TrialCampaign[]>("/api/core/v1/admin/trial-campaigns", operatorJwt);
  assert.equal(campaigns[0].seatsRemaining, 1);
  console.log("checkout_retry_and_expiry=passed");
  const url = await trialCheckoutDestination(campaign.code, input);
  const session = (await stripe.checkout.sessions.list({ customer: customer.id, limit: 10 })).data.find(s => s.status === "open")!;
  assert.equal(session.livemode, false);
  await writeFile(path.join(state, "checkout-url.txt"), url, { mode: 0o600 });
  console.log(`checkout_url=${url}`);
  console.log("Complete this TEST checkout using 4242 4242 4242 4242, a future date and any CVC. Waiting up to 15 minutes.");
  const completed = await until("browser Checkout completion", async () => {
    const s = await stripe.checkout.sessions.retrieve(session.id);
    return s.status === "complete" ? s : null;
  }, 900);
  const subId = typeof completed.subscription === "string" ? completed.subscription : completed.subscription!.id;
  await replay("checkout.session.completed", completed.id);
  await replay("customer.subscription.created", subId);
  await billing("trialing", false);
  let sub = await stripe.subscriptions.retrieve(subId);
  assert.equal(sub.trial_end! - sub.trial_start!, 7 * 86400);
  campaigns = await core<TrialCampaign[]>("/api/core/v1/admin/trial-campaigns", operatorJwt);
  assert.equal(campaigns[0].redeemedSeats, 1); assert.equal(campaigns[0].seatsRemaining, 0);
  await advance(sub.trial_end! + 1);
  await advance(sub.trial_end! + 3605);
  await finalizeClockInvoice(subId);
  sub = await until("successful trial conversion", async () => {
    const s = await stripe.subscriptions.retrieve(subId);
    const invoice = await stripe.invoices.retrieve(typeof s.latest_invoice === "string" ? s.latest_invoice : s.latest_invoice!.id);
    return s.status === "active" && invoice.status === "paid" && invoice.amount_paid === 20000 ? s : null;
  });
  await replay("customer.subscription.updated", subId);
  await billing("active", false);
  console.log("seven_day_trial_to_usd_200=passed");
  const decline = await stripe.paymentMethods.attach("pm_card_chargeCustomerFail", { customer: customer.id });
  await stripe.subscriptions.update(subId, { default_payment_method: decline.id });
  await advance(sub.items.data[0].current_period_end + 1);
  await advance(sub.items.data[0].current_period_end + 3605);
  await finalizeClockInvoice(subId);
  sub = await until("failed renewal", async () => { const s = await stripe.subscriptions.retrieve(subId); return s.status === "past_due" ? s : null; });
  await replay("customer.subscription.updated", subId);
  await billing("past_due", true);
  const paid = await stripe.paymentMethods.attach("pm_card_visa", { customer: customer.id });
  await stripe.subscriptions.update(subId, { default_payment_method: paid.id });
  const invoiceId = typeof sub.latest_invoice === "string" ? sub.latest_invoice : sub.latest_invoice!.id;
  assert.equal((await stripe.invoices.pay(invoiceId, { payment_method: paid.id })).amount_paid, 20000);
  await replay("customer.subscription.updated", subId);
  await billing("active", false);
  console.log("failed_payment_and_recovery=passed");
  await stripe.subscriptions.cancel(subId);
  await replay("customer.subscription.deleted", subId);
  await billing("canceled", true);
  campaigns = await core<TrialCampaign[]>("/api/core/v1/admin/trial-campaigns", operatorJwt);
  assert.equal(campaigns[0].redeemedSeats, 1);
  await writeFile(path.join(state, "result.json"), JSON.stringify({ runId, passed: true, checkout: session.id, campaign: campaign.id, subscription: subId, checks: ["checkout retry", "expiry seat release", "trial admission", "7-day conversion at USD 200", "failed payment block", "payment recovery", "cancellation preserves attribution"], webhookDelivery: "Actual Stripe events replayed with local signing secret" }, null, 2));
  console.log("stripe_trial_e2e=passed");
}
async function cleanup() {
  for (const p of processes.reverse()) p.kill("SIGTERM");
  const results = await Promise.allSettled([
    ...(clockId ? [stripe.testHelpers.testClocks.del(clockId)] : []),
    ...(priceId ? [stripe.prices.update(priceId, { active: false })] : []),
    ...(productId ? [stripe.products.update(productId, { active: false })] : []),
  ]);
  assert(results.every(r => r.status === "fulfilled"), "Sandbox cleanup failed");
  // The containing devfinity run removes its isolated Postgres instance.
}
main().catch(error => {
  // Never print SDK request objects or headers, which can contain credentials.
  const message = error instanceof Error ? error.message : "Sandbox rehearsal failed";
  console.error(key ? message.replaceAll(key, "[redacted]") : message);
  process.exitCode = 1;
}).finally(async () => { await cleanup().catch(() => { console.error("Sandbox cleanup failed; inspect this run's test objects."); process.exitCode = 1; }); });
