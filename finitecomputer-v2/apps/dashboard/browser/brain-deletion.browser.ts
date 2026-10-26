/** Delete Brain UI against the real hosted device and Brain server binaries:
 * the browser clicks through the dashboard route, which signs with the
 * account's hosted Brain key. Synthetic Brains and accounts only. Build the
 * binaries, then set FC_BRAIN_DELETION_PROOF_BIN_DIR=<cargo target/debug>. */
import assert from "node:assert/strict";
import { type ChildProcess, spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";
import { renameBrainFixtureEvent } from "./brain-deletion-fixture";

const BIN_DIR = process.env.FC_BRAIN_DELETION_PROOF_BIN_DIR?.trim();
// The web design fixture's hosted-device credential and account.
const DEVICE_TOKEN = "web-design-hosted-device-token";
const WORKOS_USER_ID = "user_web_design";

async function freePort() {
  const server = http.createServer().listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  await new Promise<void>((resolve) => server.close(() => resolve()));
  return address.port;
}

async function waitFor(what: string, child: ChildProcess, ready: () => Promise<boolean>, output = () => "") {
  const deadline = Date.now() + 120_000;
  while (!(await ready().catch(() => false))) {
    assert.equal(child.exitCode, null, `${what} exited: ${output()}`);
    assert(Date.now() < deadline, `${what} did not become ready: ${output()}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
}

test("Delete Brain previews, refuses a stale confirmation, and deletes through the hosted key", {
  skip: BIN_DIR ? false : "set FC_BRAIN_DELETION_PROOF_BIN_DIR to the built binaries",
  timeout: 300_000,
}, async () => {
  const scratch = await mkdtemp(path.join(os.tmpdir(), "brain-deletion-browser-"));
  const [brainPort, devicePort, dashboardPort] = [await freePort(), await freePort(), await freePort()];
  const brainOrigin = `http://127.0.0.1:${brainPort}`;
  const deviceUrl = `http://127.0.0.1:${devicePort}`;
  const base = `http://127.0.0.1:${dashboardPort}`;
  const children: ChildProcess[] = [];
  let browser: Browser | undefined;
  try {
    const brain = spawn(path.join(BIN_DIR!, "finite-brain"), [], {
      env: {
        NODE_ENV: "test",
        FINITE_BRAIN_ADDR: `127.0.0.1:${brainPort}`,
        FINITE_BRAIN_PUBLIC_BASE_URL: brainOrigin,
        FINITE_BRAIN_DB: path.join(scratch, "brain.sqlite3"),
      },
      stdio: "ignore",
    });
    const device = spawn(path.join(BIN_DIR!, "finitechat-hosted-device"), [
      "--bind", `127.0.0.1:${devicePort}`,
      "--data-root", path.join(scratch, "hosted"),
      "--server-url", "http://127.0.0.1:9",
      "--public-url", "http://chat.test",
    ], { env: { NODE_ENV: "test", FINITECHAT_HOSTED_API_TOKEN: DEVICE_TOKEN }, stdio: "ignore" });
    children.push(brain, device);
    await waitFor("Brain server", brain, async () => (await fetch(`${brainOrigin}/health`)).ok);
    await waitFor("hosted device", device, async () => (await fetch(`${deviceUrl}/v1/app/state`, {
      headers: { authorization: `Bearer ${DEVICE_TOKEN}`, "x-finite-workos-user-id": WORKOS_USER_ID },
    })).ok);

    // Seed synthetic Brains with the fixture account's hosted key.
    process.env.FC_BRAIN_PUBLIC_ORIGIN = brainOrigin;
    const { hostedBrainDeletionTransport } = await import("../src/lib/brain-deletion");
    const admin = hostedBrainDeletionTransport(
      { baseUrl: deviceUrl, apiToken: DEVICE_TOKEN },
      { email: "fixture-user@example.test", workosUserId: WORKOS_USER_ID, emailVerified: true, source: "dev" },
      brainOrigin,
    );
    const { npub } = await admin.identify();
    for (const [brainId, name] of [["v1", "V1"], ["content-brain", "Content"], ["marketing-comms-brain", "Marketing Comms"]]) {
      await admin.signedRequest("POST", "/v1/brains", JSON.stringify({ brainId, kind: "organization", name }));
    }

    const reset = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "reset"]);
    await once(reset, "exit");
    const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
      env: {
        ...process.env,
        FC_WEB_DESIGN_PORT: String(dashboardPort),
        FC_WEB_DESIGN_PREVIEWS: "0",
        FC_WEB_DESIGN_BRAIN_IDENTITY_PROVIDER_URL: deviceUrl,
        FC_WEB_DESIGN_BRAIN_URL: brainOrigin,
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    children.push(server);
    let output = "";
    server.stdout?.on("data", (chunk) => { output += chunk; });
    server.stderr?.on("data", (chunk) => { output += chunk; });
    await waitFor("dashboard", server, async () => (await fetch(`${base}/healthz`)).ok, () => output);

    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.route("**/api/chat/**", (route) => route.fulfill({ status: 503, json: {} }));
    await page.route("**/api/agents/*/hermes-access", (route) => route.fulfill({
      status: 200, json: { baseUrl: "https://agent.fixture.test/", accessToken: "synthetic-test", expiresAt: 100 },
    }));
    // The agent's own Brain inventory is synthetic; deletion never uses it.
    await page.route("https://agent.fixture.test/**", (route) => {
      const headers = { "access-control-allow-origin": base, "access-control-allow-headers": "authorization" };
      if (route.request().method() === "OPTIONS") return route.fulfill({ status: 200, headers });
      const url = new URL(route.request().url());
      return route.fulfill({ status: 200, headers, json: url.pathname.endsWith("/overview")
        ? { version: 1, brains: [
          { id: "v1", name: "V1", kind: "organization", role: "admin", folders: [] },
          { id: "content-brain", name: "Content", kind: "organization", role: "admin", folders: [] },
        ] }
        : { version: 1, brainId: "v1", identities: [] } });
    });

    await page.goto(`${base}/dashboard/machines/runtime_web_design/brain`);
    await page.getByRole("button", { name: "V1", exact: true }).click();
    const panel = page.getByRole("region", { name: "Delete V1", exact: true });
    await panel.getByRole("button", { name: "Review deletion…" }).click();
    await panel.getByText("Delete V1 permanently?").waitFor();
    await panel.getByText(brainOrigin, { exact: true }).waitFor();
    const remove = panel.getByRole("button", { name: "Delete Brain permanently" });
    assert.equal(await remove.isDisabled(), true);

    // The Brain changes after the preview: the confirmation is refused.
    const rename = await admin.signBrainEvent(renameBrainFixtureEvent({
      brainId: "v1", adminNpub: npub, note: "V1 renamed",
      changeId: "rename-during-review", createdAt: Math.floor(Date.now() / 1000),
    }));
    await admin.signedRequest("POST", "/v1/brains/v1/rename", JSON.stringify({ accessChangeEvent: rename }));
    await panel.getByLabel("Type v1 to confirm").fill("v1");
    await remove.click();
    await panel.getByRole("alert").filter({ hasText: "changed after its deletion preview" }).waitFor();
    assert.equal(await (await admin.signedRequest("GET", "/v1/brains") as { brains: unknown[] }).brains.length, 3);

    const artifacts = process.env.FC_BROWSER_ARTIFACT_DIR;
    await panel.getByRole("button", { name: "Review the current Brain again" }).click();
    await panel.getByText("V1 renamed (v1)", { exact: true }).waitFor();
    await panel.getByLabel("Type v1 to confirm").fill("v1");
    if (artifacts) {
      await mkdir(artifacts, { recursive: true });
      await page.screenshot({ path: path.join(artifacts, "brain-deletion-preview.png"), fullPage: true });
    }
    await panel.getByRole("button", { name: "Delete Brain permanently" }).click();
    await panel.getByRole("status").filter({ hasText: "Deleted V1 renamed (v1)." }).waitFor();
    if (artifacts) await page.screenshot({ path: path.join(artifacts, "brain-deletion-receipt.png"), fullPage: true });

    const listed = await admin.signedRequest("GET", "/v1/brains") as { brains: Array<{ brainId: string }> };
    assert.deepEqual(listed.brains.map((entry) => entry.brainId).sort(), ["content-brain", "marketing-comms-brain"]);
    await assert.rejects(admin.signedRequest("GET", "/v1/brains/v1/deletion-preview"), (error: { status?: number }) => error.status === 410);
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    for (const child of children.reverse()) {
      child.kill("SIGTERM");
      if (child.exitCode === null) await once(child, "exit").catch(() => undefined);
    }
  }
});
