/* eslint-disable @typescript-eslint/no-explicit-any -- test-only HTTP/Docker snapshots */
/** Real Stripe TEST -> Core -> Docker Runner -> Hosted Device qualification.
 * Run only in a disposable Linux CI worker. Never acknowledge Runner work here.
 */
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { createWriteStream } from "node:fs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import Stripe from "stripe";
import { chromium, type Browser, type BrowserContext, type Page } from "playwright";
import { STRIPE_API_VERSION } from "../src/lib/stripe-billing";
import { assertRecovered, localOrigin, ownsContainer, type RuntimeProof } from "./billing-runtime-proof";

const root = path.resolve(import.meta.dirname, "../../../..");
const runRoot = path.resolve(process.env.BILLING_SMOKE_ROOT ?? "");
assert(process.env.BILLING_SMOKE_ROOT && runRoot !== root, "explicit disposable state root required");
const evidence = path.join(runRoot, "evidence");
const contextPath = path.join(runRoot, "context.json");
const image = process.env.BILLING_SMOKE_IMAGE ?? "";
assert(/^ghcr\.io\/finitecomputer\/agent-runtime@sha256:[a-f0-9]{64}$/.test(image), "pinned canonical image required");
const key = process.env.STRIPE_SECRET_KEY ?? "";
assert(/^(sk|rk)_test_/.test(key), "Stripe TEST key required");
const stripe = new Stripe(key, { apiVersion: STRIPE_API_VERSION, timeout: 30_000, maxNetworkRetries: 1 });
const pause = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
let interrupted = false;
for (const signal of ["SIGTERM", "SIGINT"] as const) process.once(signal, () => { interrupted = true; });
let stage = "preflight";
let report: Record<string, any> = { passed: false, image, provider: "local_docker", simulatedRunner: false,
  webhookDelivery: "real Stripe TEST events replayed locally; not deployed delivery" };
async function atomicJson(file: string, value: unknown) {
  const temporary = `${file}.${randomUUID()}.tmp`;
  await writeFile(temporary, JSON.stringify(value, null, 2) + "\n", { mode: 0o600, flag: "wx" });
  await rename(temporary, file);
}
async function save() { await atomicJson(path.join(evidence, "report.json"), report); }
async function until<T>(name: string, fn: () => Promise<T | null>, seconds = 180): Promise<T> {
  stage = name;
  const deadline = Date.now() + seconds * 1000;
  while (Date.now() < deadline) {
    assert(!interrupted, "interrupted");
    const result = await fn(); if (result !== null) return result;
    await pause(1000);
  }
  throw new Error("bounded step timed out");
}
function docker(args: string[]) { return execFileSync("docker", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 30_000 }); }
async function context() { return JSON.parse(await readFile(contextPath, "utf8")); }
async function patchContext(patch: object) { await atomicJson(contextPath, { ...await context(), ...patch }); }

async function boot() {
  assert(process.platform === "linux" && process.arch === "x64", "Linux AMD64 worker required");
  assert(process.env.FINITE_PRIVATE_SMOKE_API_KEY, "approved smoke inference key required");
  await mkdir(runRoot, { recursive: false, mode: 0o700 }); // refuse reuse of another run
  await mkdir(evidence, { mode: 0o700 });
  const run = randomUUID();
  await atomicJson(contextPath, { run });
  assert.equal((await stripe.balance.retrieve()).livemode, false);
  let child: ReturnType<typeof spawn> | undefined;
  try {
    const product = await stripe.products.create({ name: `Billing runtime smoke ${run}`, metadata: { finite_billing_smoke: run } });
    await patchContext({ product: product.id });
    const price = await stripe.prices.create({ product: product.id, currency: "usd", unit_amount: 20000, recurring: { interval: "month" }, metadata: { finite_billing_smoke: run } });
    await patchContext({ price: price.id });
    const log = createWriteStream(path.join(runRoot, "private-stack.log"), { mode: 0o600 });
    child = spawn("nix", ["run", ".#devfinity", "--", "--state-dir", path.join(runRoot, "stack"), "up", "--headless", "--docker-runtime", "--prebuilt-runtime-image", image, "--", "sh", "-c", "cd finitecomputer-v2/apps/dashboard && node --import tsx scripts/billing-runtime-smoke.ts"], {
      cwd: root, stdio: ["ignore", "pipe", "pipe"], env: { ...process.env,
        BILLING_SMOKE_CHILD: "1", DEVFINITY_APPLE_CONTAINER_NAME_PREFIX: `billing-${run}`,
        FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE: process.env.FINITE_PRIVATE_SMOKE_API_KEY,
        FC_RUNNER_HEALTH_REPORT_INTERVAL_SECS: "5",
        FC_CORE_STANDARD_STRIPE_PRICE_ID: price.id, STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID: price.id,
        STRIPE_WEBHOOK_SECRET: `whsec_${randomUUID()}`, FC_DASHBOARD_RUNTIME_MODE: "customer",
        FC_DASHBOARD_TRIALS_ENABLED: "true", FC_DASHBOARD_BASE_URL: "http://127.0.0.1:13002",
      },
    });
    child.stdout?.pipe(log); child.stderr?.pipe(log);
    await new Promise<void>((resolve, reject) => {
      const timer = setInterval(() => { if (interrupted) child?.kill("SIGTERM"); }, 500);
      child!.once("error", () => { clearInterval(timer); reject(new Error("stack spawn failed")); });
      child!.once("exit", () => { clearInterval(timer); resolve(); });
    });
    report = JSON.parse(await readFile(path.join(evidence, "report.json"), "utf8"));
    assert.equal(child.exitCode, 0, "stack failed"); assert(report.passed);
  } finally {
    child?.kill("SIGTERM");
    await cleanup();
  }
}

async function cleanup() {
  const c = await context();
  const failures: string[] = [];
  // Enrollment can launch before the project id reaches this process. Use exact
  // run-directory ownership as fallback; never stop by image or generic prefix alone.
  const stoppedContainers: string[] = [];
  try {
    const ids = docker(["ps", "-aq"]).trim().split(/\s+/).filter(Boolean);
    for (const id of ids) {
      const item = JSON.parse(docker(["inspect", id]))[0];
      if (!ownsContainer(item, image, runRoot, c.run)) continue;
      if (item.State.Running) docker(["stop", "--time", "20", id]);
      assert.equal(docker(["ps", "-q", "--no-trunc", "--filter", `id=${id}`]).trim(), "");
      stoppedContainers.push(id);
    }
  } catch { failures.push("owned runtime or network-probe stop"); }
  for (const kind of ["clock", "price", "product"] as const) {
    if (!c[kind]) continue;
    try {
      if (kind === "clock") {
        try { const clock = await stripe.testHelpers.testClocks.retrieve(c.clock); assert.equal(clock.name, `Billing runtime smoke ${c.run}`); await stripe.testHelpers.testClocks.del(c.clock); }
        catch (error) { if (!(error instanceof Stripe.errors.StripeInvalidRequestError && error.code === "resource_missing")) throw error; }
      }
      if (kind === "price") { const price = await stripe.prices.retrieve(c.price); assert.equal(price.metadata.finite_billing_smoke, c.run); await stripe.prices.update(c.price, { active: false }); }
      if (kind === "product") { const product = await stripe.products.retrieve(c.product); assert(!product.deleted && product.metadata.finite_billing_smoke === c.run); await stripe.products.update(c.product, { active: false }); }
    } catch { failures.push(`${kind} cleanup`); }
  }
  report.cleanup = { passed: failures.length === 0, failures, stoppedContainers, objects: { clock: c.clock, product: c.product, price: c.price }, persistentDataPurged: false };
  if (failures.length) report.passed = false;
  await save();
  assert.equal(failures.length, 0, "cleanup incomplete");
}

// Use Stripe's actual hosted page with a test-only session, never fake its completion.
async function checkout(page: Page, url: string, customer: string) {
  const target = new URL(url); assert.equal(target.hostname, "checkout.stripe.com"); assert.equal(target.protocol, "https:");
  const sessions = await stripe.checkout.sessions.list({ customer, limit: 10 });
  const session = sessions.data.find(s => s.url === url);
  assert(session && session.id.startsWith("cs_test_") && !session.livemode);
  await page.goto(url);
  await page.getByText(/test mode/i).first().waitFor({ timeout: 30_000 });
  async function fill(name: string, value: string, required = true) {
    let found = false;
    await until(`test Checkout field ${name}`, async () => {
      for (const frame of page.frames()) {
        const field = frame.locator(`[name="${name}"]`).first();
        if (await field.isVisible()) { await field.fill(value); found = true; return true; }
      }
      return required ? null : true;
    }, required ? 30 : 1);
    return found;
  }
  await fill("cardNumber", "4242424242424242");
  await fill("cardExpiry", "12/35"); await fill("cardCvc", "123");
  await fill("billingName", "Disposable QA", false);
  const country = page.locator('select[name="billingCountry"]').first();
  if (await country.isVisible()) await country.selectOption("US");
  await fill("billingAddressLine1", "123 Test Street", false);
  await fill("billingLocality", "Portland", false);
  const region = page.locator('select[name="billingAdministrativeArea"]').first();
  if (await region.isVisible()) await region.selectOption("OR");
  await fill("billingPostalCode", "97205", false);
  await page.getByRole("button", { name: /start trial|subscribe/i }).click();
  return until("actual Stripe Checkout completion", async () => {
    const s = await stripe.checkout.sessions.retrieve(session.id); return s.status === "complete" ? s : null;
  }, 120);
}

async function scenario() {
  assert.equal(process.env.DEVFINITY_PROFILE, "docker-saas");
  const state = process.env.DEVFINITY_STATE_DIR!;
  assert(path.resolve(state).startsWith(path.join(runRoot, "stack") + path.sep));
  const c = await context();
  const dashboard = localOrigin(process.env.FC_DASHBOARD_URL!);
  const core = localOrigin(process.env.FC_CORE_URL!);
  localOrigin(process.env.FC_HOSTED_WEB_DEVICE_URL!); localOrigin(process.env.FINITECHAT_SERVER_URL!);
  const jwt = (await readFile(path.join(state, "workos-fixture/dashboard-customer.jwt"), "utf8")).trim();
  const operator = (await readFile(path.join(state, "workos-fixture/operator.jwt"), "utf8")).trim();
  async function api(route: string, body?: object, admin = false): Promise<any> {
    const response = await fetch(core + route, { method: body ? "POST" : "GET", headers: { authorization: `Bearer ${admin ? operator : jwt}`, "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined, signal: AbortSignal.timeout(30_000) });
    assert(response.ok, "local Core request failed"); return response.status === 204 ? null : response.json();
  }
  const me = () => api("/api/core/v1/me");
  const billing = () => api("/api/core/v1/me/billing");
  assert.equal((await me()).projects.length, 0, "only empty disposable account allowed");
  const clock = await stripe.testHelpers.testClocks.create({ frozen_time: Math.floor(Date.now() / 1000), name: `Billing runtime smoke ${c.run}` });
  await patchContext({ clock: clock.id });
  const customer = await stripe.customers.create({ test_clock: clock.id, email: `billing-${c.run}@example.test`, metadata: { finite_billing_smoke: c.run } });
  await api("/api/core/v1/me/billing/stripe-customer", { stripeCustomerId: customer.id });
  const campaign = await api("/api/core/v1/admin/trial-campaigns", { name: `Billing ${c.run}`, seatLimit: 1, trialDays: 7 }, true);
  const browser: Browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH });
  const browserContext: BrowserContext = await browser.newContext();
  const page = await browserContext.newPage();
  const replayed = new Set<string>();
  let lastEvent: Stripe.Event | undefined;
  async function replay(event: Stripe.Event) {
    assert.equal(event.livemode, false);
    const payload = JSON.stringify(event);
    const response = await fetch(dashboard + "/api/stripe/webhook", { method: "POST", body: payload, headers: { "stripe-signature": stripe.webhooks.generateTestHeaderString({ payload, secret: process.env.STRIPE_WEBHOOK_SECRET! }) }, signal: AbortSignal.timeout(30_000) });
    assert.equal(response.status, 200, "real dashboard webhook failed");
  }
  async function sync() {
    const events: Stripe.Event[] = [];
    let scanned = 0;
    for await (const event of stripe.events.list({ created: { gte: clock.frozen_time - 60 }, limit: 100 })) {
      if (++scanned > 1000) throw new Error("event bound exceeded");
      if ((event.data.object as any).customer === customer.id && ["checkout.session.completed", "customer.subscription.created", "customer.subscription.updated", "customer.subscription.deleted"].includes(event.type)) events.push(event);
    }
    for (const event of events.reverse()) if (!replayed.has(event.id)) { await replay(event); replayed.add(event.id); lastEvent = event; }
  }
  let updates: AbortController | undefined;
  try {
    const response = await browserContext.request.post(dashboard + "/dashboard/agent-creation-requests", { form: { displayName: `Billing QA ${c.run}`, hostingTier: "standard", access: "stripe", trialCode: campaign.code, idempotencyKey: c.run }, maxRedirects: 0 });
    assert.equal(response.status(), 303);
    const complete = await checkout(page, response.headers().location, customer.id);
    const subscription = typeof complete.subscription === "string" ? complete.subscription : complete.subscription!.id;
    await until("trial event reconciliation", async () => { await sync(); return (await billing()).billing_account?.subscription_status === "trialing" ? true : null; });
    await page.goto(dashboard + "/dashboard/agent-creation-requests/complete");
    const entry = await until("real Runner launch", async () => {
      const projects = (await me()).projects; assert(projects.length <= 1);
      const item = projects[0];
      if (item?.project) await patchContext({ project: item.project.id });
      return item?.runtime?.runtime_health?.status === "ready" ? item : null;
    }, 600);
    const project = entry.project.id, runtime = entry.runtime.id;
    const runtimeHealthUrl = new URL(entry.runtime.contact_endpoint);
    localOrigin(runtimeHealthUrl.toString());
    runtimeHealthUrl.pathname = runtimeHealthUrl.pathname.replace(/\/contact\/?$/, "/healthz");
    async function principal() {
      const r = await fetch(runtimeHealthUrl, { signal: AbortSignal.timeout(5000) }); assert(r.ok);
      const health = await r.json(); assert(typeof health.npub === "string" && health.npub.startsWith("npub1")); return health.npub as string;
    }
    await patchContext({ project, runtime });
    const provenance = JSON.parse(await readFile(path.join(state, "runtime-image/build-report.json"), "utf8"));
    assert.equal(provenance.status, "verified_prebuilt"); assert.equal(provenance.image, image);
    report = { ...report, applicationSha: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(), runtimeSourceSha: provenance.mono_sha, hermesVersion: provenance.image_metadata.hermes_nix_runtime.version, platform: provenance.platform, project, runtime, subscription, clock: clock.id, customer: customer.id, paymentRestoration: "Stripe TEST invoice API; portal navigation verified separately" };
    const containerIds = () => docker(["ps", "-aq", "--filter", `label=computer.finite.v2.project_id=${project}`]).trim().split(/\s+/).filter(Boolean);
    function physical() { const ids = containerIds(); assert.equal(ids.length, 1); return JSON.parse(docker(["inspect", ids[0]]))[0]; }
    const container = physical().Id;
    assert.equal(physical().Config.Image, image);
    const marker = `billing-${c.run}`;
    const markerPath = `/data/workspace/${marker}.txt`;
    docker(["exec", container, "sh", "-c", 'printf "%s" "$1" > "$2"', "sh", marker, markerPath]);
    const hash = () => createHash("sha256").update(docker(["exec", container, "cat", markerPath])).digest("hex");
    const chatUrl = dashboard + `/api/chat/machines/${runtime}/hosted-device`;
    async function chat(): Promise<any> { const r = await browserContext.request.get(chatUrl + "/state"); assert(r.ok(), "Hosted Device unavailable"); return r.json(); }
    function stream() {
      updates?.abort(); updates = new AbortController();
      void fetch(chatUrl + "/updates", { signal: updates.signal }).then(async r => {
        assert(r.ok); const reader = r.body?.getReader();
        if (reader) { try { while (!(await reader.read()).done) { /* drain real updates */ } } finally { reader.releaseLock(); } }
      }).catch(() => {});
    }
    stream();
    const initialChat = await until("real Hosted Device connection", async () => { const s = await chat(); return s.rooms?.some((r: any) => r.is_agent_chat && r.state === "Connected") ? s : null; });
    const room = initialChat.hosted_agent_binding.canonical_room_id;
    const topic = initialChat.topics.find((t: any) => t.room_id === room && t.topic_id === "home");
    assert(topic); const chatId = topic.chats.find((t: any) => t.active)?.chat_id ?? topic.chats[0]?.chat_id; assert(chatId);
    const scoped = (s: any) => s.messages.filter((m: any) => m.room_id === room && m.conversation_id === topic.topic_id && m.chat_id === chatId).sort((a: any, b: any) => Number(a.seq) - Number(b.seq));
    async function turn(text: string) {
      const current = await chat(); const previous = Math.max(0, ...scoped(current).map((m: any) => Number(m.seq)));
      const r = await browserContext.request.post(chatUrl + "/actions", { data: { SendChatMessage: { room_id: room, topic_id: topic.topic_id, chat_id: chatId, text: `Reply with exactly: ${text}` } } }); assert(r.ok());
      return until("real model reply", async () => { const s = await chat(); return scoped(s).some((m: any) => m.sender_account_id !== s.identity.account_id && Number(m.seq) > previous && m.final_delivery === true && String(m.display_content ?? m.text ?? "").includes(text)) ? s : null; }, 240);
    }
    async function snapshot(): Promise<RuntimeProof> {
      const s = await chat(), item = (await me()).projects.find((x: any) => x.project.id === project); assert(item?.runtime);
      const p = physical();
      const home = s.topics.find((t: any) => t.room_id === s.hosted_agent_binding.canonical_room_id && t.topic_id === "home");
      assert(home?.chats.some((t: any) => t.chat_id === chatId), "original chat missing");
      return { project: item.project.id, runtime: item.runtime.id, principal: await principal(), room: s.hosted_agent_binding.canonical_room_id, topic: home.topic_id, chat: chatId, fileHash: hash(), messageIds: scoped(s).map((m: any) => m.id ?? m.message_id), running: p.State.Running, startedAt: p.State.StartedAt };
    }
    await turn(`before-${c.run}`);
    const before = await snapshot(); assert(before.messageIds.every(Boolean));
    await page.goto(dashboard + "/dashboard"); await page.screenshot({ path: path.join(evidence, "normal.png") });
    const decline = await stripe.paymentMethods.attach("pm_card_chargeCustomerFail", { customer: customer.id });
    await stripe.subscriptions.update(subscription, { default_payment_method: decline.id });
    async function advance(time: number) { await stripe.testHelpers.testClocks.advance(clock.id, { frozen_time: time }); await until("Stripe clock ready", async () => { const c = await stripe.testHelpers.testClocks.retrieve(clock.id); assert.notEqual(c.status, "internal_failure"); return c.status === "ready" ? true : null; }); }
    const sub = await stripe.subscriptions.retrieve(subscription); assert(sub.trial_end);
    await advance(sub.trial_end + 1); await advance(sub.trial_end + 3605);
    for (let i = 0; i < 3; i++) {
      const s = await stripe.subscriptions.retrieve(subscription); const invoice = await stripe.invoices.retrieve(typeof s.latest_invoice === "string" ? s.latest_invoice : s.latest_invoice!.id);
      if (invoice.status !== "draft") break;
      assert(invoice.automatically_finalizes_at); const c = await stripe.testHelpers.testClocks.retrieve(clock.id); await advance(Math.max(c.frozen_time + 1, invoice.automatically_finalizes_at + 1));
    }
    await until("billing blocked and physical container stopped", async () => {
      await sync(); const b = await billing(); const p = (await me()).projects.find((x: any) => x.project.id === project);
      return b.billing_account?.subscription_status === "past_due" && b.trial_access?.blocked === true && p.runtime?.lifecycle_status === "offline" && !physical().State.Running ? true : null;
    });
    const stopped = { ...before, running: physical().State.Running };
    await page.goto(dashboard + "/dashboard"); await page.getByText(/Your trial has ended/).first().waitFor();
    await page.screenshot({ path: path.join(evidence, "trial-ended.png") });
    // The trial notice and Agent card both offer the same account-level action.
    await page.getByRole("button", { name: "Manage billing", exact: true }).first().click();
    await page.waitForURL(url => url.protocol === "https:" && url.hostname === "billing.stripe.com");
    await page.getByText(/test mode/i).first().waitFor();
    report.manageBillingReachedTestPortal = true;
    await page.goto(dashboard + "/dashboard");
    const denied = await browserContext.request.get(chatUrl + "/state"); assert([402, 403, 404].includes(denied.status()), "chat route remained open");
    await page.goto(dashboard + `/dashboard/machines/${runtime}/chat`); assert(!new URL(page.url()).pathname.endsWith("/chat"), "direct chat route remained open");
    const paid = await stripe.paymentMethods.attach("pm_card_visa", { customer: customer.id }); await stripe.subscriptions.update(subscription, { default_payment_method: paid.id });
    const failed = await stripe.subscriptions.retrieve(subscription); await stripe.invoices.pay(typeof failed.latest_invoice === "string" ? failed.latest_invoice : failed.latest_invoice!.id, { payment_method: paid.id });
    await until("payment accepted", async () => { await sync(); const b = await billing(); return b.billing_account?.subscription_status === "active" && b.trial_access?.blocked === false ? true : null; });
    await page.goto(dashboard + "/dashboard");
    const waiting = page.getByText(/Restarting your agent automatically\. You can leave this page and return\.|Waiting for your agent to be ready\. This page updates automatically\./);
    await waiting.waitFor({ timeout: 15_000 }); // fail rather than claim an unobserved refresh
    await page.screenshot({ path: path.join(evidence, "payment-restored.png") });
    await until("real Runner restarted", async () => { const p = physical(); return p.State.Running && p.State.StartedAt !== before.startedAt ? true : null; });
    stream(); await until("chat connected after real restart", async () => { const s = await chat(); return s.rooms?.some((r: any) => r.room_id === room && r.state === "Connected") ? true : null; });
    await turn(`after-${c.run}`); const after = await snapshot(); assertRecovered(before, stopped, after);
    await until("Core recovery observation cleared", async () => { const p = (await me()).projects.find((x: any) => x.project.id === project); return !p.runtime_recovery && p.runtime?.runtime_status === "online" && p.runtime.runtime_health?.status === "ready" ? true : null; });
    await waiting.waitFor({ state: "hidden", timeout: 180_000 });
    await page.getByText("Your agent is ready.", { exact: true }).first().waitFor({ timeout: 30_000 });
    report.recoveryUiRefreshedWithoutReload = true;
    await page.screenshot({ path: path.join(evidence, "recovered.png") });
    assert(lastEvent); await replay(lastEvent); await pause(7000); assert.equal(physical().State.StartedAt, after.startedAt, "duplicate webhook restarted runtime again");
    report = { ...report, passed: true, before, stopped, after, events: [...replayed], duplicateEventDidNotRestart: true };
    await save();
  } finally { updates?.abort(); await browser.close(); }
}

async function main() {
  try {
    if (process.env.BILLING_SMOKE_CHILD === "1") await scenario();
    else if (process.argv.includes("--cleanup")) { report = JSON.parse(await readFile(path.join(evidence, "report.json"), "utf8").catch(() => "{}")); await cleanup(); }
    else await boot();
  } catch {
    report.passed = false; report.failedStage = stage;
    await save().catch(() => {});
    console.error(`Billing runtime smoke failed at ${stage}; private diagnostics remain in the run directory.`);
    process.exitCode = 1;
  }
}
await main();
