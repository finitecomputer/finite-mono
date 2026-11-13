/** Invoked by Core's key_alias_browser_composition test under devfinity.
 * Real: dashboard server/API/UI, Core auth/description/aliases, Postgres.
 * Fixtures: WorkOS provider and unrelated Agent/Brain roster transport.
 * Tokens arrive through stdin, never argv, output or committed artifacts.
 */
import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "./playwright-browser";

async function main() {
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  const config = JSON.parse(input) as {
    coreUrl: string; identityUrl: string; runtimeId: string;
    owner: string; other: string; keys: string[]; npubs: string[];
    observationToken: string; descriptionToken: string;
  };
  for (const origin of [config.coreUrl, config.identityUrl]) {
    const url = new URL(origin);
    assert.equal(url.hostname, "127.0.0.1");
    assert.equal(url.protocol, "http:");
  }
  const servers: ChildProcess[] = [];
  let browser: Browser | undefined;
  async function request(base: string, endpoint: string, token: string, body?: unknown, credential?: [string, string]) {
    const response = await fetch(`${base}${endpoint}`, {
      method: body ? "POST" : "GET",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json", ...(credential ? { [credential[0]]: credential[1] } : {}) },
      body: body ? JSON.stringify(body) : undefined,
    });
    assert.equal(response.status, 200, `HTTP ${response.status} for ${endpoint}`);
    return response.json();
  }
  async function startDashboard(token: string, subject: string, email: string) {
    const reservation = http.createServer().listen(0, "127.0.0.1");
    await once(reservation, "listening");
    const address = reservation.address();
    assert(address && typeof address !== "string");
    await new Promise<void>(resolve => reservation.close(() => resolve()));
    const base = `http://127.0.0.1:${address.port}`;
    const environment: NodeJS.ProcessEnv = { NODE_ENV: "production" };
    for (const name of ["PATH", "HOME", "TMPDIR", "SSL_CERT_FILE", "NIX_SSL_CERT_FILE"]) {
      if (process.env[name]) environment[name] = process.env[name];
    }
    const child = spawn(process.execPath, ["node_modules/next/dist/bin/next", "start", "--hostname", "127.0.0.1", "--port", String(address.port)], {
      env: { ...environment, NODE_ENV: "production", NEXT_TELEMETRY_DISABLED: "1",
        FC_CORE_BASE_URL: config.coreUrl, FC_DASHBOARD_BASE_URL: base, FC_WORKOS_AUTH_ENABLED: "0",
        FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1", FC_DASHBOARD_DEV_EMAIL: email,
        FC_DASHBOARD_DEV_WORKOS_USER_ID: subject, FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: token,
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    servers.push(child);
    // Drain logs without echoing request or credential-bearing diagnostics.
    child.stdout?.resume(); child.stderr?.resume();
    await waitFor(async () => {
      assert.equal(child.exitCode, null, "dashboard exited before readiness");
      return fetch(`${base}/healthz`).then(response => response.ok).catch(() => false);
    });
    return base;
  }
  try {
    await request(config.coreUrl, "/api/core/v1/me", config.owner);
    await request(config.coreUrl, "/api/core/v1/me", config.other);
    await request(config.identityUrl, "/api/core/internal/v1/brain-account-observations", config.owner, {
      version: "finite-core-brain-account-observation-v1", operationId: "browser_alias_observation_0001",
      brainServer: "https://brain.test", brainId: "brain_alpha", observedAt: new Date().toISOString(),
      actionKind: "humanHostedAction", observedHumanPublicKeyHex: config.keys[0], participatingPublicKeyHex: config.keys[0],
    }, ["x-finite-brain-observation-credential", config.observationToken]);
    await request(config.identityUrl, "/api/core/internal/v1/brain-account-observations", config.other, {
      version: "finite-core-brain-account-observation-v1", operationId: "browser_alias_observation_0002",
      brainServer: "https://brain.test", brainId: "brain_alpha", observedAt: new Date().toISOString(),
      actionKind: "humanHostedAction", observedHumanPublicKeyHex: config.keys[5], participatingPublicKeyHex: config.keys[5],
    }, ["x-finite-brain-observation-credential", config.observationToken]);
    const descriptions = await request(config.identityUrl, "/api/core/internal/v1/brain-identity-descriptions", "", {
      version: "finite-core-brain-descriptions-v3", brainServer: "https://brain.test", brainId: "brain_alpha",
      requestedByPublicKeyHex: config.keys[1], publicKeysHex: config.keys, participatingPublicKeysHex: config.keys,
    }, ["x-finite-brain-description-credential", config.descriptionToken]);
    const human = descriptions.results.find((row: { publicKeyHex: string }) => row.publicKeyHex === config.keys[0]);
    assert.equal(human.displayName, "Ada Lovelace");
    const unnamed = descriptions.results.find((row: { publicKeyHex: string }) => row.publicKeyHex === config.keys[5]);
    assert.equal(unnamed.displayName, undefined);
    assert.equal(unnamed.accountEmail, "other@finite.test");
    const base = await startDashboard(config.owner, "runtime-auth-user", "runtime-auth@finite.test");
    const otherBase = await startDashboard(config.other, "alias-browser-other", "other@finite.test");
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.route("**/api/chat/**", route => route.fulfill({ status: 503, json: {} }));
    await page.route("**/api/agents/*/hermes-access", route => route.fulfill({ json: {
      baseUrl: "https://brain-alias.fixture.test/", accessToken: "synthetic-roster", expiresAt: 100,
    } }));
    // Only the unchanged runtime/Brain transport is synthetic. The human's
    // name/email above were read through the real private Core HTTP listener.
    await page.route("https://brain-alias.fixture.test/**", async route => {
      const headers = { "access-control-allow-origin": base, "access-control-allow-headers": "authorization" };
      if (route.request().method() === "OPTIONS") return route.fulfill({ headers });
      const overview = route.request().url().endsWith("/overview");
      const json = overview ? { version: 1, brains: [{ id: "brain_alpha", name: "Staff brain", kind: "organization", role: "admin", folders: [] }] }
        : { version: 1, brainId: "brain_alpha", identities: [
          { npub: config.npubs[0], role: "member", kind: "human", name: human.displayName, email: human.accountEmail, ownerEmail: null, folders: [] },
          { npub: config.npubs[1], role: "admin", kind: "agent", name: "Moss", lifecycle: "retired", email: null, ownerEmail: "owner@example.test", folders: [] },
          { npub: config.npubs[2], role: "member", kind: null, name: null, email: null, ownerEmail: null, folders: [] },
          { npub: config.npubs[3], role: "member", kind: null, publicName: "Stored person", publicNameStoredAt: "2026-10-01T00:00:00Z", folders: [] },
          { npub: config.npubs[4], role: "member", kind: null, operatorAnnotation: { kind: "historical_identity", label: "Earlier person", observedAt: "2026-10-01T00:00:00Z" }, folders: [] },
          { npub: config.npubs[5], role: "member", kind: "human", email: unnamed.accountEmail, folders: [] },
        ] };
      await route.fulfill({ headers, json });
    });
    const openRoster = async () => {
      await page.goto(`${base}/dashboard/machines/${config.runtimeId}/brain`);
      await page.getByRole("button", { name: "Staff brain", exact: true }).click();
      await page.getByRole("button", { name: /Add alias for Ada|Edit alias for/ }).first().waitFor();
    };
    await openRoster();
    const roster = page.getByRole("region", { name: "Identities in Staff brain", exact: true });
    const humanRow = roster.getByRole("row").filter({ hasText: "runtime-auth@finite.test" });
    await humanRow.getByRole("button", { name: "Add alias for Ada Lovelace", exact: true }).focus();
    await page.keyboard.press("Enter");
    await page.getByRole("textbox", { name: "Alias", exact: true }).fill("  Ada   (staff) ");
    await page.getByRole("button", { name: "Save alias", exact: true }).click();
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    await humanRow.getByText("Ada (staff)", { exact: true }).waitFor();
    await waitFor(() => humanRow.getByRole("button", { name: "Edit alias for Ada (staff)", exact: true }).evaluate(element => element === document.activeElement));
    assert.match(await humanRow.innerText(), /Ada Lovelace[\s\S]*runtime-auth@finite.test/);
    await openRoster();
    await humanRow.getByText("Ada (staff)", { exact: true }).waitFor();
    const privateLookup = await fetch(`${otherBase}/api/brain/key-aliases`, {
      method: "POST", headers: { "content-type": "application/json", origin: otherBase }, body: JSON.stringify({ npubs: [config.npubs[0]] }),
    });
    assert.equal(privateLookup.status, 200);
    assert.deepEqual(await privateLookup.json(), { aliases: [] });
    const forged = await fetch(`${base}/api/brain/key-aliases`, {
      method: "PUT", headers: { "content-type": "application/json", origin: "https://untrusted.example" },
      body: JSON.stringify({ npub: config.npubs[0], alias: "forged" }),
    });
    assert.equal(forged.status, 403);
    await humanRow.getByRole("button", { name: "Edit alias for Ada (staff)", exact: true }).click();
    await page.getByRole("textbox", { name: "Alias", exact: true }).fill("x".repeat(65));
    assert(await page.getByRole("button", { name: "Save alias", exact: true }).isDisabled());
    await page.getByRole("alert").filter({ hasText: "64 characters" }).waitFor();
    await page.getByRole("textbox", { name: "Alias", exact: true }).fill("Research lead");
    await page.route("**/api/brain/key-aliases", route => route.request().method() === "PUT"
      ? route.fulfill({ status: 503, json: { error: "Try again later." } }) : route.continue());
    await page.getByRole("button", { name: "Save alias", exact: true }).click();
    await page.getByRole("alert").filter({ hasText: "Try again later." }).waitFor();
    assert.equal(await page.getByRole("textbox", { name: "Alias", exact: true }).inputValue(), "Research lead");
    await page.unroute("**/api/brain/key-aliases");
    await page.getByRole("button", { name: "Save alias", exact: true }).click();
    await humanRow.getByText("Research lead", { exact: true }).waitFor();
    for (const label of ["Moss", "Identity details are unavailable.", "Stored person", "Earlier person", "other@finite.test"]) {
      const row = roster.getByRole("row").filter({ hasText: label });
      await row.getByRole("button", { name: /Add alias/ }).click();
      await page.getByRole("textbox", { name: "Alias", exact: true }).fill("Research lead");
      await page.getByText("You already use this alias for another key here. Check the key before saving.", { exact: true }).waitFor();
      await page.getByRole("button", { name: "Save alias", exact: true }).click();
      await row.getByText("Your alias", { exact: true }).waitFor();
      assert((await row.innerText()).includes(label));
      if (label === "Moss") await row.getByText("Agent (retired)", { exact: true }).waitFor();
      if (label === "Stored person") await row.getByText("Public name", { exact: true }).waitFor();
      if (label === "Earlier person") await row.getByText("Historical identity · Evidence 2026-10-01", { exact: true }).waitFor();
      if (label === "Moss") assert.match(await row.innerText(), /owner@example.test/);
      await row.getByRole("button", { name: /Edit alias/ }).click();
      await page.getByRole("button", { name: "Clear alias", exact: true }).click();
      await row.getByRole("button", { name: /Add alias/ }).waitFor();
    }
    const artifacts = process.env.FC_BROWSER_ARTIFACT_DIR;
    if (artifacts) {
      await mkdir(artifacts, { recursive: true });
      await page.screenshot({ path: path.join(artifacts, "brain-alias-desktop.png"), fullPage: true });
    }
    await page.setViewportSize({ width: 390, height: 844 });
    await waitFor(() => page.evaluate(() => {
      const sidebar = document.querySelector(".finite-agent-shell__sidebar");
      return (!sidebar || sidebar.getBoundingClientRect().right <= 0) && document.documentElement.scrollWidth <= innerWidth;
    }));
    if (artifacts) await page.screenshot({ path: path.join(artifacts, "brain-alias-mobile.png"), fullPage: true });
    await humanRow.getByRole("button", { name: "Edit alias for Research lead", exact: true }).click();
    if (artifacts) await page.screenshot({ path: path.join(artifacts, "brain-alias-dialog-mobile.png"), fullPage: true });
    await page.getByRole("button", { name: "Clear alias", exact: true }).click();
    await humanRow.getByRole("button", { name: "Add alias for Ada Lovelace", exact: true }).waitFor();
    await openRoster();
    assert.equal(await humanRow.getByText("Your alias", { exact: true }).count(), 0);
    for (const status of [404, 503]) {
      await page.route("**/api/brain/key-aliases", route => route.fulfill({ status, json: { error: "Unavailable" } }));
      await page.getByRole("button", { name: "Refresh brains", exact: true }).click();
      await page.getByText("Your private aliases are unavailable right now.", { exact: true }).waitFor();
      assert.match(await humanRow.innerText(), /Ada Lovelace/);
      assert.equal(await roster.getByRole("button", { name: /alias/ }).count(), 0);
      await page.unroute("**/api/brain/key-aliases");
    }
    assert.deepEqual(errors, []);
    console.log("PASS: real Core/Postgres + dashboard v3 name/fallback display, scoped-label preservation, alias save/reload/edit/clear, viewer isolation, origin rejection, keyboard, bounds, mobile and unavailable fallback");
    console.log("BOUNDARY: WorkOS provider and Agent/Brain roster transport are fixtures; live OAuth, runtime/Brain services and production rollout are not claimed.");
  } finally {
    await browser?.close();
    await Promise.all(servers.map(async server => {
      if (server.exitCode !== null) return;
      server.kill("SIGTERM");
      await once(server, "exit");
    }));
  }
}
async function waitFor(check: () => Promise<boolean>) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error("Timed out waiting for browser proof readiness");
}
main().catch(error => { console.error(error instanceof Error ? error.message : "Browser proof failed"); process.exitCode = 1; });
