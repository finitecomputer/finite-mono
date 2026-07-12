import { createServer } from "node:net";
import { createWriteStream } from "node:fs";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";

import Stripe from "stripe";

type CoreBillingOverview = {
  customer_org: {
    id: string;
  };
  billing_account?: {
    stripe_customer_id?: string | null;
    stripe_subscription_id?: string | null;
    stripe_price_id?: string | null;
    subscription_status?: string | null;
    last_stripe_event_id?: string | null;
  } | null;
  agent_creation_entitlement?: {
    allowed_new_agent_runtimes: number;
    launch_code?: string | null;
  } | null;
  can_create_agent: boolean;
  requires_billing: boolean;
};

type CoreCustomerBillingAccount = {
  customer_org_id: string;
  stripe_customer_id?: string | null;
};

type ManagedProcess = {
  child: ChildProcessWithoutNullStreams;
  name: string;
  logPath: string;
};

const dashboardRoot = process.cwd();
const repoRoot = path.resolve(dashboardRoot, "../..");
const runId = process.env.FC_STRIPE_BILLING_E2E_RUN_ID?.trim() || randomUUID().slice(0, 8);
const stateRoot =
  process.env.FC_STRIPE_BILLING_E2E_STATE_ROOT?.trim() ||
  path.join(repoRoot, ".local-state", "stripe-billing-e2e");
const runRoot = path.join(stateRoot, runId);
const keepServices = process.env.FC_STRIPE_BILLING_E2E_KEEP_SERVICES === "1";
const keepStripeObjects = process.env.FC_STRIPE_BILLING_E2E_KEEP_STRIPE === "1";
const stripeSecretKey = requiredEnv("STRIPE_SECRET_KEY");
const standardPriceId = requiredEnv("STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID");
const webhookSecret =
  process.env.STRIPE_WEBHOOK_SECRET?.trim() || `whsec_finite_test_${runId}`;
const coreToken = process.env.FC_CORE_API_TOKEN?.trim() || `local-core-token-${runId}`;
const runnerToken = `local-runner-token-${runId}`;
const usageToken = `local-usage-token-${runId}`;
const workosClientId = "client_devfinity";
const workosOperatorOrgId = "org_devfinity_operator";
const workosCustomerEmail = "devfinity@finite.computer";
const postgresContainer =
  process.env.FC_STRIPE_BILLING_E2E_POSTGRES_CONTAINER?.trim() ||
  `finite-v2-stripe-e2e-${runId}`;
const postgresPassword =
  process.env.FC_STRIPE_BILLING_E2E_POSTGRES_PASSWORD?.trim() ||
  `finite-stripe-e2e-${runId}`;
const postgresDb = process.env.FC_STRIPE_BILLING_E2E_POSTGRES_DB?.trim() || "finite_core";
const stripe = new Stripe(stripeSecretKey, {
  appInfo: {
    name: "finitecomputer-v2-stripe-billing-clock-e2e",
  },
});

let clockId: string | null = null;
let subscriptionId: string | null = null;
let customerJwt = "";
const managedProcesses: ManagedProcess[] = [];
const cleanupTasks: Array<() => Promise<void>> = [];

async function main() {
  await mkdir(runRoot, { recursive: true });
  process.env.FC_CORE_API_TOKEN = coreToken;
  process.env.STRIPE_SECRET_KEY = stripeSecretKey;
  process.env.STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID = standardPriceId;
  process.env.STRIPE_WEBHOOK_SECRET = webhookSecret;

  const postgresPort = await freePort();
  const corePort = await freePort();
  const workosPort = await freePort();
  const coreUrl = `http://127.0.0.1:${corePort}`;
  const workosUrl = `http://127.0.0.1:${workosPort}`;
  const workosStateRoot = path.join(runRoot, "workos-fixture");
  const databaseUrl = `postgres://finite:${postgresPassword}@127.0.0.1:${postgresPort}/${postgresDb}`;
  process.env.FC_CORE_BASE_URL = coreUrl;
  process.env.FC_DASHBOARD_BASE_URL = "http://127.0.0.1:3000";

  console.log("finitecomputer-v2 Stripe billing test-clock E2E");
  console.log(`run_id=${runId}`);
  console.log(`logs=${runRoot}`);

  startWorkosFixture(workosUrl, workosStateRoot);
  await waitForHttp(`${workosUrl}/sso/jwks/${workosClientId}`, "WorkOS fixture");
  const workosApiKey = (await readFile(path.join(workosStateRoot, "workos-fixture-api-key"), "utf8")).trim();
  customerJwt = (await readFile(path.join(workosStateRoot, "dashboard-customer.jwt"), "utf8")).trim();
  if (!workosApiKey || !customerJwt) {
    throw new Error("WorkOS fixture did not prepare its private credentials.");
  }

  await startPostgres(postgresPort);
  startCore(coreUrl, databaseUrl, workosUrl, workosApiKey);
  await waitForHttp(`${coreUrl}/healthz`, "Core");

  const webhook = await import("../src/app/api/stripe/webhook/route");
  const frozenTime = Math.floor(Date.now() / 1000);
  const clock = await stripe.testHelpers.testClocks.create({
    frozen_time: frozenTime,
    name: `finite billing e2e ${runId}`,
  });
  clockId = clock.id;
  console.log(`stripe_clock=${clock.id}`);

  const customer = await stripe.customers.create({
    email: workosCustomerEmail,
    name: `Finite Billing E2E ${runId}`,
    test_clock: clock.id,
    metadata: {
      finite_e2e_run_id: runId,
    },
  });
  console.log(`stripe_customer=${customer.id}`);

  const account = await coreIdentityFetch<CoreCustomerBillingAccount>(
    coreUrl,
    "/api/core/v1/me/billing/stripe-customer",
    {
      method: "POST",
      body: JSON.stringify({
        stripeCustomerId: customer.id,
      }),
    }
  );
  const customerOrgId = account.customer_org_id;
  console.log(`customer_org=${customerOrgId}`);

  const subscription = await stripe.subscriptions.create({
    customer: customer.id,
    collection_method: "send_invoice",
    days_until_due: 1,
    items: [
      {
        price: standardPriceId,
      },
    ],
    metadata: {
      finite_customer_org_id: customerOrgId,
      finite_e2e_run_id: runId,
    },
  });
  subscriptionId = subscription.id;
  console.log(`stripe_subscription=${subscription.id}`);

  const eventTime = Math.floor(Date.now() / 1000);
  await assertWebhookSignatureRejection(webhook.POST, subscription);
  await postStripeWebhook(
    webhook.POST,
    "checkout.session.completed",
    {
      id: `cs_test_finite_${runId}`,
      object: "checkout.session",
      mode: "subscription",
      customer: customer.id,
      subscription: subscription.id,
      metadata: { finite_customer_org_id: customerOrgId },
    },
    "checkout_completed",
    eventTime
  );
  await assertBilling(coreUrl, {
    label: "active checkout subscription",
    expectedStatus: "active",
    expectedCanCreateAgent: true,
    expectedRequiresBilling: false,
    expectedAllowedNewAgents: 1,
  });

  await postStripeWebhook(
    webhook.POST,
    "customer.subscription.created",
    subscription,
    "created",
    eventTime + 1
  );
  await assertBilling(coreUrl, {
    label: "active subscription",
    expectedStatus: "active",
    expectedCanCreateAgent: true,
    expectedRequiresBilling: false,
    expectedAllowedNewAgents: 1,
  });

  await stripe.testHelpers.testClocks.advance(clock.id, {
    frozen_time: frozenTime + 3 * 24 * 60 * 60,
  });
  await waitForClockReady(clock.id);
  const pastDueSubscription = await waitForSubscriptionStatus(
    subscription.id,
    "past_due",
    "subscription did not become past_due after send_invoice due date"
  );
  await postStripeWebhook(
    webhook.POST,
    "customer.subscription.updated",
    pastDueSubscription,
    "past_due",
    eventTime + 2
  );
  await assertBilling(coreUrl, {
    label: "past_due subscription",
    expectedStatus: "past_due",
    expectedCanCreateAgent: false,
    expectedRequiresBilling: true,
    expectedAllowedNewAgents: 0,
  });
  await assertAgentCreationBlocked(coreUrl);

  const canceledSubscription = await stripe.subscriptions.cancel(subscription.id);
  await postStripeWebhook(
    webhook.POST,
    "customer.subscription.deleted",
    canceledSubscription,
    "canceled",
    eventTime + 4
  );
  await assertBilling(coreUrl, {
    label: "canceled subscription",
    expectedStatus: "canceled",
    expectedCanCreateAgent: false,
    expectedRequiresBilling: true,
    expectedAllowedNewAgents: 0,
  });

  const staleActivePayload = {
    ...canceledSubscription,
    status: "active",
  } as Stripe.Subscription;
  await postStripeWebhook(
    webhook.POST,
    "customer.subscription.updated",
    staleActivePayload,
    "stale_active_after_cancel",
    eventTime + 3
  );
  await assertBilling(coreUrl, {
    label: "stale active event after cancellation",
    expectedStatus: "canceled",
    expectedCanCreateAgent: false,
    expectedRequiresBilling: true,
    expectedAllowedNewAgents: 0,
  });

  console.log("stripe_billing_clock_e2e=passed");
}

async function postStripeWebhook(
  post: (request: Request) => Promise<Response>,
  type: string,
  object: unknown,
  label: string,
  created: number
) {
  const payload = stripeEventPayload(type, object, label, created);
  const signature = stripe.webhooks.generateTestHeaderString({
    payload,
    secret: webhookSecret,
  });
  const response = await post(
    new Request("http://127.0.0.1/api/stripe/webhook", {
      method: "POST",
      body: payload,
      headers: {
        "content-type": "application/json",
        "stripe-signature": signature,
      },
    })
  );
  if (!response.ok) {
    const body = await response.text();
    throw new Error(`webhook ${type}/${label} failed: ${response.status} ${body}`);
  }
  console.log(`webhook_${label}=ok`);
}

function stripeEventPayload(type: string, object: unknown, label: string, created: number) {
  return JSON.stringify({
    id: `evt_finite_${runId}_${label}`,
    object: "event",
    api_version: "2025-12-15.clover",
    created,
    data: {
      object,
    },
    livemode: false,
    pending_webhooks: 1,
    request: {
      id: null,
      idempotency_key: null,
    },
    type,
  });
}

async function assertWebhookSignatureRejection(
  post: (request: Request) => Promise<Response>,
  object: unknown
) {
  const payload = stripeEventPayload(
    "customer.subscription.created",
    object,
    "signature_negative",
    Math.floor(Date.now() / 1000)
  );
  const missing = await post(
    new Request("http://127.0.0.1/api/stripe/webhook", {
      method: "POST",
      body: payload,
      headers: { "content-type": "application/json" },
    })
  );
  assertEqual(missing.status, 400, "missing Stripe signature");
  const invalid = await post(
    new Request("http://127.0.0.1/api/stripe/webhook", {
      method: "POST",
      body: payload,
      headers: {
        "content-type": "application/json",
        "stripe-signature": "invalid-signature",
      },
    })
  );
  assertEqual(invalid.status, 400, "invalid Stripe signature");
  if (missing.ok || invalid.ok) {
    throw new Error("Stripe webhook signature checks failed open.");
  }
  console.log("webhook_signature_negative=ok");
}

async function assertBilling(
  coreUrl: string,
  expected: {
    label: string;
    expectedStatus: string;
    expectedCanCreateAgent: boolean;
    expectedRequiresBilling: boolean;
    expectedAllowedNewAgents: number;
  }
) {
  const overview = await coreIdentityFetch<CoreBillingOverview>(
    coreUrl,
    "/api/core/v1/me/billing"
  );
  await writeFile(
    path.join(runRoot, `${slug(expected.label)}.billing.json`),
    `${JSON.stringify(overview, null, 2)}\n`
  );
  const status = overview.billing_account?.subscription_status;
  const entitlement = overview.agent_creation_entitlement;
  assertEqual(status, expected.expectedStatus, `${expected.label}: subscription_status`);
  assertEqual(
    overview.can_create_agent,
    expected.expectedCanCreateAgent,
    `${expected.label}: can_create_agent`
  );
  assertEqual(
    overview.requires_billing,
    expected.expectedRequiresBilling,
    `${expected.label}: requires_billing`
  );
  assertEqual(
    entitlement?.launch_code ?? null,
    null,
    `${expected.label}: billing entitlement must not become a launch-code grant`
  );
  assertEqual(
    entitlement?.allowed_new_agent_runtimes ?? 0,
    expected.expectedAllowedNewAgents,
    `${expected.label}: allowed_new_agent_runtimes`
  );
  console.log(
    `billing_${slug(expected.label)}=status:${status},can_create:${overview.can_create_agent}`
  );
}

async function assertAgentCreationBlocked(coreUrl: string) {
  const response = await coreIdentityFetchRaw(
    coreUrl,
    "/api/core/v1/me/agent-creation-requests",
    {
      method: "POST",
      body: JSON.stringify({
        displayName: "Blocked Billing E2E Agent",
        launchCode: "",
        idempotencyKey: `stripe-e2e-blocked-${runId}`,
      }),
    }
  );
  const text = await response.text();
  if (response.status !== 402) {
    throw new Error(`expected agent creation to return 402, got ${response.status}: ${text}`);
  }
  console.log("agent_creation_past_due_blocked=ok");
}

async function coreIdentityFetch<T>(
  coreUrl: string,
  pathname: string,
  init: RequestInit = {}
) {
  const response = await coreIdentityFetchRaw(coreUrl, pathname, init);
  const text = await response.text();
  const parsed = text ? JSON.parse(text) : {};
  if (!response.ok) {
    throw new Error(
      `Core ${pathname} returned ${response.status}: ${JSON.stringify(parsed)}`
    );
  }
  return parsed as T;
}

async function coreIdentityFetchRaw(
  coreUrl: string,
  pathname: string,
  init: RequestInit = {}
) {
  return fetch(new URL(pathname, coreUrl), {
    ...init,
    headers: {
      authorization: `Bearer ${customerJwt}`,
      "content-type": "application/json",
      ...Object.fromEntries(new Headers(init.headers).entries()),
    },
  });
}

async function waitForSubscriptionStatus(
  id: string,
  expectedStatus: Stripe.Subscription.Status,
  failureMessage: string
) {
  let lastSubscription: Stripe.Subscription | null = null;
  for (let attempt = 0; attempt < 45; attempt += 1) {
    lastSubscription = await stripe.subscriptions.retrieve(id);
    if (lastSubscription.status === expectedStatus) {
      return lastSubscription;
    }
    await sleep(1_000);
  }
  throw new Error(
    `${failureMessage}; last status was ${lastSubscription?.status ?? "unknown"}`
  );
}

async function waitForClockReady(id: string) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const clock = await stripe.testHelpers.testClocks.retrieve(id);
    if (clock.status === "ready") {
      return clock;
    }
    if (clock.status === "internal_failure") {
      throw new Error(`Stripe test clock ${id} failed to advance.`);
    }
    await sleep(1_000);
  }
  throw new Error(`Stripe test clock ${id} did not become ready.`);
}

async function startPostgres(port: number) {
  await run("docker", ["rm", "-f", postgresContainer], { allowFailure: true });
  cleanupTasks.push(async () => {
    if (!keepServices) {
      await run("docker", ["rm", "-f", postgresContainer], { allowFailure: true });
    }
  });
  await run("docker", [
    "run",
    "-d",
    "--name",
    postgresContainer,
    "-e",
    `POSTGRES_PASSWORD=${postgresPassword}`,
    "-e",
    "POSTGRES_USER=finite",
    "-e",
    `POSTGRES_DB=${postgresDb}`,
    "-p",
    `127.0.0.1:${port}:5432`,
    "postgres:16-alpine",
  ]);
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const result = await run(
      "docker",
      ["exec", postgresContainer, "pg_isready", "-U", "finite", "-d", postgresDb],
      { allowFailure: true }
    );
    if (result.status === 0) {
      console.log("postgres=ready");
      return;
    }
    await sleep(1_000);
  }
  throw new Error("Postgres did not become ready.");
}

function startWorkosFixture(workosUrl: string, fixtureStateRoot: string) {
  const fixtureLogPath = path.join(runRoot, "workos-fixture.log");
  const child = spawn(
    "cargo",
    [
      "run",
      "-p",
      "devfinity",
      "--",
      "workos-fixture",
      "--listen",
      new URL(workosUrl).host,
      "--state-dir",
      fixtureStateRoot,
    ],
    { cwd: repoRoot, env: process.env }
  );
  managedProcesses.push({ child, name: "workos-fixture", logPath: fixtureLogPath });
  pipeProcessLog(child, fixtureLogPath);
}

function startCore(
  coreUrl: string,
  databaseUrl: string,
  workosUrl: string,
  workosApiKey: string
) {
  const coreLogPath = path.join(runRoot, "core.log");
  const child = spawn("cargo", ["run", "-p", "finite-saas-core", "--", "serve"], {
    cwd: repoRoot,
    env: {
      ...process.env,
      FC_CORE_DATABASE_URL: databaseUrl,
      FC_CORE_API_TOKEN: coreToken,
      FC_CORE_RUNNER_API_TOKEN: runnerToken,
      FC_FINITE_PRIVATE_USAGE_API_TOKEN: usageToken,
      FC_CORE_BIND: new URL(coreUrl).host,
      FC_CORE_STANDARD_STRIPE_PRICE_ID: standardPriceId,
      WORKOS_API_KEY: workosApiKey,
      WORKOS_CLIENT_ID: workosClientId,
      WORKOS_API_BASE_URL: workosUrl,
      WORKOS_JWKS_URL: `${workosUrl}/sso/jwks/${workosClientId}`,
      WORKOS_ISSUER: workosUrl,
      FC_WORKOS_OPERATOR_ORG_ID: workosOperatorOrgId,
    },
  });
  managedProcesses.push({
    child,
    name: "core",
    logPath: coreLogPath,
  });
  pipeProcessLog(child, coreLogPath);
}

async function waitForHttp(url: string, name: string) {
  let lastError = "not attempted";
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        console.log(`${name.toLowerCase()}=ready`);
        return;
      }
      lastError = `${response.status} ${await response.text()}`;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
    await sleep(1_000);
  }
  throw new Error(`${name} did not become ready at ${url}: ${lastError}`);
}

function pipeProcessLog(child: ChildProcessWithoutNullStreams, logPath: string) {
  const stream = createWriteStream(logPath, { flags: "a" });
  child.stdout.pipe(stream, { end: false });
  child.stderr.pipe(stream, { end: false });
  child.on("exit", () => stream.end());
}

async function run(
  command: string,
  args: string[],
  options: {
    allowFailure?: boolean;
    cwd?: string;
    env?: NodeJS.ProcessEnv;
  } = {}
): Promise<{ status: number; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env ?? process.env,
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout.on("data", (chunk: Buffer) => stdout.push(chunk));
    child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
    child.on("error", reject);
    child.on("exit", (status) => {
      const result = {
        status: status ?? 1,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      };
      if (result.status !== 0 && !options.allowFailure) {
        reject(
          new Error(
            `${command} ${args.join(" ")} failed with ${result.status}: ${result.stderr || result.stdout}`
          )
        );
      } else {
        resolve(result);
      }
    });
  });
}

async function freePort() {
  return new Promise<number>((resolve, reject) => {
    const server = createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        reject(new Error("Could not allocate a local TCP port."));
        return;
      }
      server.close((error) => {
        if (error) {
          reject(error);
        } else {
          resolve(address.port);
        }
      });
    });
  });
}

function requiredEnv(name: string) {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`${name} is required.`);
  }
  return value;
}

function assertEqual<T>(actual: T, expected: T, label: string) {
  if (actual !== expected) {
    throw new Error(`${label}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function slug(value: string) {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function cleanup() {
  for (const processInfo of managedProcesses.reverse()) {
    if (!keepServices && !processInfo.child.killed) {
      processInfo.child.kill("SIGTERM");
    }
  }
  if (!keepStripeObjects && clockId) {
    if (subscriptionId) {
      await stripe.subscriptions.cancel(subscriptionId).catch(() => undefined);
    }
    await stripe.testHelpers.testClocks.del(clockId).catch((error: unknown) => {
      console.warn(
        `Could not delete Stripe test clock ${clockId}: ${
          error instanceof Error ? error.message : String(error)
        }`
      );
    });
  }
  for (const task of cleanupTasks.reverse()) {
    await task();
  }
  if (!keepServices) {
    await rm(runRoot, { recursive: true, force: true }).catch(() => undefined);
  }
}

main()
  .catch(async (error: unknown) => {
    console.error(error instanceof Error ? error.message : String(error));
    for (const processInfo of managedProcesses) {
      const log = await readFile(processInfo.logPath, "utf8").catch(() => "");
      if (log) {
        console.error(`\n--- ${processInfo.name} log (${processInfo.logPath}) ---`);
        console.error(log.split("\n").slice(-80).join("\n"));
      }
    }
    process.exitCode = 1;
  })
  .finally(async () => {
    await cleanup();
  });
