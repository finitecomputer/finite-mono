/* eslint-disable @typescript-eslint/no-explicit-any -- test-only HTTP/Docker snapshots */
/** Synthetic billing contract -> real Core -> Docker Runner -> Hosted Device qualification.
 * Run only in a disposable Linux CI worker. Never acknowledge Runner work here.
 */
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { createWriteStream } from "node:fs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium, type Browser, type BrowserContext } from "playwright";
import { fixtureEnvironment, subscriptionFixture, startModelFixture, assertFixtureInference } from "./billing-runtime-fixtures";
import { assertRecovered, localOrigin, ownsContainer, type RuntimeProof } from "./billing-runtime-proof";

const root = path.resolve(__dirname, "../../../..");
const runRoot = path.resolve(process.env.BILLING_SMOKE_ROOT ?? "");
assert(process.env.BILLING_SMOKE_ROOT && runRoot !== root, "explicit disposable state root required");
const evidence = path.join(runRoot, "evidence");
const contextPath = path.join(runRoot, "context.json");
const image = process.env.BILLING_SMOKE_IMAGE ?? "";
assert(/^ghcr\.io\/finitecomputer\/agent-runtime@sha256:[a-f0-9]{64}$/.test(image), "pinned canonical image required");
const pause = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
let interrupted = false;
for (const signal of ["SIGTERM", "SIGINT"] as const) process.once(signal, () => { interrupted = true; });
let stage = "preflight";
let report: Record<string, any> = { passed: false, image, provider: "local_docker", simulatedRunner: false,
  billingInput: "synthetic subscription contract posted to real Core HTTP ingestion; Stripe not exercised",
  inference: "local deterministic responder; real Hermes/chat transport, no model API",
  externalPaymentVerified: false };
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
  await mkdir(runRoot, { recursive: false, mode: 0o700 }); // refuse reuse of another run
  await mkdir(evidence, { mode: 0o700 });
  const run = randomUUID();
  await atomicJson(contextPath, { run });
  const model = await startModelFixture();
  await patchContext({ modelPort: model.port });
  let child: ReturnType<typeof spawn> | undefined;
  try {
    const log = createWriteStream(path.join(runRoot, "private-stack.log"), { mode: 0o600 });
    child = spawn("nix", ["run", ".#devfinity", "--", "--state-dir", path.join(runRoot, "stack"), "up", "--headless", "--docker-runtime", "--prebuilt-runtime-image", image, "--", "sh", "-c", '. "$DEVFINITY_STATE_DIR/secrets/core.sh"; cd finitecomputer-v2/apps/dashboard && node --import tsx scripts/billing-runtime-smoke.ts'], {
      cwd: root, stdio: ["ignore", "pipe", "pipe"], env: { ...fixtureEnvironment(process.env), NODE_ENV: "development",
        BILLING_SMOKE_CHILD: "1", DEVFINITY_APPLE_CONTAINER_NAME_PREFIX: `billing-${run}`,
        FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE: model.token,
        FC_RUNNER_FINITE_PRIVATE_BASE_URL: `http://host.docker.internal:${model.port}/v1`,
        DEVFINITY_FINITE_PRIVATE_CONTROL_URL: `http://host.docker.internal:${model.port}/control`,
        FC_RUNNER_HEALTH_REPORT_INTERVAL_SECS: "5",
        FC_CORE_STANDARD_STRIPE_PRICE_ID: "price_contract_fixture", FC_DASHBOARD_RUNTIME_MODE: "customer",
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
    assert(model.count() >= 2 && model.count() <= 32, "expected bounded local model traffic");
    report.localModelRequests = model.count();
  } finally {
    child?.kill("SIGTERM");
    try { await cleanup(); } finally { await model.close(); }
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
  report.cleanup = { passed: failures.length === 0, failures, stoppedContainers, persistentDataPurged: false };
  if (failures.length) report.passed = false;
  await save();
  assert.equal(failures.length, 0, "cleanup incomplete");
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
  const serviceToken = process.env.FC_CORE_API_TOKEN; assert(serviceToken, "isolated Core service credential required");
  async function api(route: string, body?: object, auth: "customer" | "operator" | "service" = "customer"): Promise<any> {
    const response = await fetch(core + route, { method: body ? "POST" : "GET", headers: { authorization: `Bearer ${auth === "service" ? serviceToken : auth === "operator" ? operator : jwt}`, "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined, signal: AbortSignal.timeout(30_000) });
    assert(response.ok, "local Core request failed"); return response.status === 204 ? null : response.json();
  }
  const me = () => api("/api/core/v1/me");
  const billing = () => api("/api/core/v1/me/billing");
  assert.equal((await me()).projects.length, 0, "only empty disposable account allowed");
  const customer = `cus_contract_${c.run}`, subscription = `sub_contract_${c.run}`;
  const linked = await api("/api/core/v1/me/billing/stripe-customer", { stripeCustomerId: customer });
  const campaign = await api("/api/core/v1/admin/trial-campaigns", { name: `Billing ${c.run}`, seatLimit: 1, trialDays: 7 }, "operator");
  const now = Math.floor(Date.now() / 1000);
  const workosUserId = JSON.parse(Buffer.from(jwt.split(".")[1], "base64url").toString()).sub;
  assert(typeof workosUserId === "string");
  await api("/api/core/v1/billing/trial-reservation", { workosUserId, reservation: {
    code: campaign.code, customerOrgId: linked.customer_org_id, stripeCustomerId: customer,
    stripeSessionId: `cs_contract_${c.run}`, attemptId: c.run, checkoutExpiresAt: now + 3600, trialDays: 7,
  } }, "service");
  const fixture = (status: "trialing" | "past_due" | "active", sequence: number) => subscriptionFixture({
    run: c.run, customerOrgId: linked.customer_org_id, status, sequence, now,
  });
  const ingest = (body: ReturnType<typeof fixture>) => api("/api/core/v1/billing/stripe/subscription", body, "service");
  await ingest(fixture("trialing", 1));
  assert.equal((await billing()).billing_account?.subscription_status, "trialing");
  assert.equal((await billing()).trial_access?.blocked, false);
  const browser: Browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH });
  const browserContext: BrowserContext = await browser.newContext();
  const page = await browserContext.newPage();
  let updates: AbortController | undefined;
  try {
    const response = await browserContext.request.post(dashboard + "/dashboard/agent-creation-requests", { form: { displayName: `Billing QA ${c.run}`, hostingTier: "standard", access: "entitled", idempotencyKey: c.run }, maxRedirects: 0 });
    assert.equal(response.status(), 303);
    const destination = new URL(response.headers().location, dashboard);
    assert.equal(destination.origin, dashboard);
    assert(!destination.searchParams.has("agentCreationError"), "dashboard creation failed");
    await page.goto(destination.toString());
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
    report = { ...report, applicationSha: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(), runtimeSourceSha: provenance.mono_sha, hermesVersion: provenance.image_metadata.hermes_nix_runtime.version, platform: provenance.platform, project, runtime, subscription, customer, paymentRestoration: "synthetic active contract input; no payment or portal" };
    const containerIds = () => docker(["ps", "-aq", "--filter", `label=computer.finite.v2.project_id=${project}`]).trim().split(/\s+/).filter(Boolean);
    function physical() { const ids = containerIds(); assert.equal(ids.length, 1); return JSON.parse(docker(["inspect", ids[0]]))[0]; }
    const container = physical().Id;
    assert.equal(physical().Config.Image, image);
    assertFixtureInference(physical().Config.Env, c.modelPort);
    const marker = `billing-${c.run}`;
    const markerPath = `/data/workspace/${marker}.txt`;
    docker(["exec", container, "sh", "-c", 'printf "%s" "$1" > "$2"', "sh", marker, markerPath]);
    const hash = () => createHash("sha256").update(docker(["exec", container, "cat", markerPath])).digest("hex");
    const chatUrl = dashboard + `/api/chat/machines/${runtime}/hosted-device`;
    let viewQuery = "";
    let streamError: unknown;
    async function chat(): Promise<any> {
      assert(!streamError, "Hosted Device update stream failed");
      const r = await browserContext.request.get(chatUrl + "/state" + viewQuery); assert(r.ok(), "Hosted Device unavailable"); return r.json();
    }
    function stream() {
      updates?.abort(); updates = new AbortController();
      const signal = updates.signal;
      streamError = undefined;
      void (async () => {
        let transportFailures = 0;
        // Trial streams deliberately expire after 30 seconds. Reconnect like
        // the browser, retaining the explicit transcript view across reads.
        while (!signal.aborted) {
          try {
            const r = await fetch(chatUrl + "/updates" + viewQuery, { signal });
            assert(r.ok && r.body, "Hosted Device update stream unavailable");
            const reader = r.body.getReader();
            try { while (!(await reader.read()).done) { transportFailures = 0; } } finally { reader.releaseLock(); }
          } catch (error) {
            if (signal.aborted) return;
            // The dashboard's bounded trial stream can terminate the body.
            // State reads still fail closed if access or the service is lost.
            if (!(error instanceof TypeError) || ++transportFailures > 3) throw error;
          }
          await pause(500);
        }
      })().catch(error => { if (!signal.aborted) streamError = error; });
    }
    await chat(); // bootstrap the one canonical binding before opening a stream
    const initialChat = await until("real Hosted Device connection", async () => { const s = await chat(); return s.rooms?.some((r: any) => r.is_agent_chat && r.state === "Connected") ? s : null; });
    const room = initialChat.hosted_agent_binding.canonical_room_id;
    const topic = initialChat.topics.find((t: any) => t.room_id === room && t.topic_id === "home");
    assert(topic); const chatId = topic.chats.find((t: any) => t.active)?.chat_id ?? topic.chats[0]?.chat_id; assert(chatId);
    viewQuery = "?" + new URLSearchParams({ room_id: room, topic_id: topic.topic_id, chat_id: chatId, limit: "100" });
    stream();
    const scoped = (s: any) => s.messages.filter((m: any) => m.room_id === room && m.conversation_id === topic.topic_id && m.chat_id === chatId).sort((a: any, b: any) => Number(a.seq) - Number(b.seq));
    async function turn(text: string) {
      const current = await chat(); const previous = Math.max(0, ...scoped(current).map((m: any) => Number(m.seq)));
      const r = await browserContext.request.post(chatUrl + "/actions", { data: { SendChatMessage: { room_id: room, topic_id: topic.topic_id, chat_id: chatId, text: `Reply with exactly: ${text}` } } }); assert(r.ok());
      return until("real Hermes reply from deterministic local responder", async () => { const s = await chat(); return scoped(s).some((m: any) => m.sender_account_id !== s.identity.account_id && Number(m.seq) > previous && m.final_delivery === true && String(m.display_content ?? m.text ?? "").includes(text)) ? s : null; }, 240);
    }
    async function snapshot(): Promise<RuntimeProof> {
      // Scoped transcript reads omit binding metadata. Observe the actual
      // persisted binding afresh at each proof boundary, never reuse a cached
      // pre-restart identity as evidence of recovery.
      const bindingResponse = await browserContext.request.get(chatUrl + "/state");
      assert(bindingResponse.ok());
      const binding = (await bindingResponse.json()).hosted_agent_binding; assert(binding);
      const s = await chat(), item = (await me()).projects.find((x: any) => x.project.id === project); assert(item?.runtime);
      const p = physical();
      const home = s.topics.find((t: any) => t.room_id === binding.canonical_room_id && t.topic_id === "home");
      assert(home?.chats.some((t: any) => t.chat_id === chatId), "original chat missing");
      return { project: item.project.id, runtime: item.runtime.id, principal: await principal(), room: binding.canonical_room_id, topic: home.topic_id, chat: chatId, fileHash: hash(), messageIds: scoped(s).map((m: any) => m.id ?? m.message_id), running: p.State.Running, startedAt: p.State.StartedAt };
    }
    await turn(`before-${c.run}`);
    const before = await snapshot(); assert(before.messageIds.every(Boolean));
    await page.goto(dashboard + "/dashboard"); await page.screenshot({ path: path.join(evidence, "normal.png") });
    updates?.abort(); // an expired account must not keep an authorized stream
    await ingest(fixture("past_due", 2));
    // A delayed pre-expiry contract input must not unblock the account.
    await ingest(fixture("trialing", 1));
    await until("billing blocked and physical container stopped", async () => {
      const b = await billing(); const p = (await me()).projects.find((x: any) => x.project.id === project);
      return b.billing_account?.subscription_status === "past_due" && b.trial_access?.blocked === true && p.runtime?.lifecycle_status === "offline" && !physical().State.Running ? true : null;
    });
    const stopped = { ...before, running: physical().State.Running };
    await page.goto(dashboard + "/dashboard"); await page.getByText(/Your trial has ended/).first().waitFor();
    await page.screenshot({ path: path.join(evidence, "trial-ended.png") });
    const denied = await browserContext.request.get(chatUrl + "/state"); assert([402, 403, 404].includes(denied.status()), "chat route remained open");
    await page.goto(dashboard + `/dashboard/machines/${runtime}/chat`); assert(!new URL(page.url()).pathname.endsWith("/chat"), "direct chat route remained open");
    const restored = fixture("active", 3);
    await ingest(restored);
    await until("synthetic billing access restored", async () => { const b = await billing(); return b.billing_account?.subscription_status === "active" && b.trial_access?.blocked === false ? true : null; });
    await page.goto(dashboard + "/dashboard");
    const waiting = page.getByText(/Restarting your agent automatically\. You can leave this page and return\.|Waiting for your agent to be ready\. This page updates automatically\./);
    await waiting.waitFor({ timeout: 15_000 }); // fail rather than claim an unobserved refresh
    await page.screenshot({ path: path.join(evidence, "payment-restored.png") });
    await until("real Runner restarted", async () => { const p = physical(); return p.State.Running && p.State.StartedAt !== before.startedAt ? true : null; });
    stream(); await until("chat connected after real restart", async () => { const s = await chat(); return s.rooms?.some((r: any) => r.room_id === room && r.state === "Connected") ? true : null; });
    assertFixtureInference(physical().Config.Env, c.modelPort);
    await turn(`after-${c.run}`); const after = await snapshot(); assertRecovered(before, stopped, after);
    await until("Core recovery observation cleared", async () => { const p = (await me()).projects.find((x: any) => x.project.id === project); return !p.runtime_recovery && p.runtime?.runtime_status === "online" && p.runtime.runtime_health?.status === "ready" ? true : null; });
    await waiting.waitFor({ state: "hidden", timeout: 180_000 });
    await page.getByText("Your agent is ready.", { exact: true }).first().waitFor({ timeout: 30_000 });
    report.recoveryUiRefreshedWithoutReload = true;
    await page.screenshot({ path: path.join(evidence, "recovered.png") });
    await ingest(restored); await pause(7000); assert.equal(physical().State.StartedAt, after.startedAt, "duplicate billing input restarted runtime again");
    report = { ...report, passed: true, before, stopped, after, syntheticEventIds: [1, 2, 3].map(n => `evt_contract_${c.run}_${n}`), duplicateEventDidNotRestart: true };
    await save();
  } finally { updates?.abort(); await browser.close(); }
}

async function main() {
  try {
    if (process.env.BILLING_SMOKE_CHILD === "1") await scenario();
    else if (process.argv.includes("--cleanup")) { report = JSON.parse(await readFile(path.join(evidence, "report.json"), "utf8").catch(() => "{}")); await cleanup(); }
    else await boot();
  } catch {
    report.passed = false; report.failedStage ??= stage;
    await save().catch(() => {});
    console.error(`Billing runtime smoke failed at ${stage}; private diagnostics remain in the run directory.`);
    process.exitCode = 1;
  }
}
void main();
