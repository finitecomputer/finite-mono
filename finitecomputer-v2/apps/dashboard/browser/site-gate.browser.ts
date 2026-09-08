/** Real browser + real gate/Sites + legacy registry. Account authentication uses
 * the existing explicit local dev-account fixture, not a live WorkOS login.
 * Run after `cargo build -p finite-gated -p finitesitesd` from the pinned shell.
 */
import assert from "node:assert/strict";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { createECDH, randomBytes } from "node:crypto";
import { once } from "node:events";
import { copyFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

const dashboardDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repo = resolve(dashboardDir, "../../..");
async function freePort() {
  const server = createServer().listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address !== "string");
  await new Promise<void>((done) => server.close(() => done()));
  return address.port;
}
async function stop(child: ChildProcess) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, "exit");
  // Each fixture owns its process group, including Next's child server.
  process.kill(-child.pid!, "SIGTERM");
  const timer = setTimeout(() => { try { process.kill(-child.pid!, "SIGKILL"); } catch {} }, 3000);
  await exited;
  clearTimeout(timer);
}
async function ready(url: string, logs: () => string) {
  for (let i = 0; i < 240; i++) {
    try { if ((await fetch(url, { signal: AbortSignal.timeout(1000) })).ok) return; } catch {}
    await new Promise((done) => setTimeout(done, 250));
  }
  throw new Error(`Fixture did not start: ${url}\n${logs()}`);
}

test("existing local account opens legacy email-shared site directly and embedded without another challenge", { timeout: 120_000 }, async () => {
  const temp = await mkdtemp(join(tmpdir(), "finite-site-gate-browser-"));
  const dist = `.next-site-gate-${process.pid}`;
  const tsconfig = `.site-gate-tsconfig-${process.pid}.json`;
  const children: ChildProcess[] = [];
  let browser: Browser | undefined;
  let output = "";
  const start = (command: string, args: string[], env: Partial<NodeJS.ProcessEnv>) => {
    const child = spawn(command, args, { cwd: dashboardDir, env: { ...process.env, ...env }, stdio: "pipe", detached: true });
    child.stdout!.on("data", (data) => { output += data.toString(); });
    child.stderr!.on("data", (data) => { output += data.toString(); });
    children.push(child);
    return child;
  };
  try {
    const [dashboardPort, gatePort, sitesPort] = await Promise.all([freePort(), freePort(), freePort()]);
    const accountOrigin = `http://localhost:${dashboardPort}`;
    const gateOrigin = `http://localhost:${gatePort}`;
    const siteOrigin = `http://legacy-email.sites.localhost:${sitesPort}`;
    const token = randomBytes(32).toString("hex");
    const key = createECDH("secp256k1"); key.generateKeys();
    const data = join(temp, "sites");
    await mkdir(data);
    execFileSync("tar", ["-xzf", join(repo, "finite-sites/crates/finitesitesd/tests/fixtures/legacy-email-v053/state.tar.gz"), "-C", data]);
    start(join(repo, "target/debug/finite-gated"), [], {
      FINITE_GATE_LISTEN: `127.0.0.1:${gatePort}`, FINITE_GATE_PUBLIC_URL: gateOrigin,
      FINITE_GATE_SIGNING_KEY: key.getPrivateKey().toString("hex"),
      FINITE_GATE_ACCOUNT_URL: `${accountOrigin}/site-auth`, FINITE_GATE_ACCOUNT_TOKEN: token,
      FINITE_GATE_SITE_BASE_DOMAIN: "sites.localhost",
    });
    start(join(repo, "target/debug/finitesitesd"), ["serve", "--data", data, "--listen", `127.0.0.1:${sitesPort}`, "--base-domain", "sites.localhost", "--site-scheme", "http", "--site-port", String(sitesPort), "--mailer", "dev", "--git-auto-reconcile", "false"], {
      FINITE_SITES_AUTH_GATE_URL: gateOrigin,
      FINITE_SITES_AUTH_GATE_PUBKEY: key.getPublicKey(undefined, "compressed").subarray(1).toString("hex"),
      FINITE_SITES_VIEWER_SESSION_TOKEN: "",
    });
    await copyFile(join(dashboardDir, "tsconfig.json"), join(dashboardDir, tsconfig));
    start(process.execPath, ["node_modules/next/dist/bin/next", "dev", "--webpack", "--hostname", "127.0.0.1", "--port", String(dashboardPort)], {
      NODE_ENV: "development", NEXT_DIST_DIR: dist, NEXT_TSCONFIG_PATH: tsconfig,
      FC_WORKOS_AUTH_ENABLED: "0", FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
      FC_DASHBOARD_DEV_EMAIL: "friend@example.com", FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_local_browser_fixture",
      FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "local-browser-fixture-access-token",
      FC_SITES_AUTH_GATE_URL: gateOrigin, FC_SITES_GATE_BASE_DOMAIN: "sites.localhost", FINITE_GATE_ACCOUNT_TOKEN: token,
    });
    await Promise.all([
      ready(`http://127.0.0.1:${gatePort}/healthz`, () => output),
      ready(`http://127.0.0.1:${sitesPort}/api/v2/healthz`, () => output),
      ready(`http://127.0.0.1:${dashboardPort}/health`, () => output),
    ]);
    browser = await chromium.launch({ ...chromiumLaunchOptions(), headless: true });
    for (const embedded of [false, true]) {
      const context = await browser.newContext();
      assert.deepEqual(await context.cookies(), []);
      const page = await context.newPage();
      const navigations: string[] = [];
      page.on("request", (request) => { if (request.isNavigationRequest()) navigations.push(request.url()); });
      if (embedded) {
        await page.goto(`${accountOrigin}/health`);
        const src = `${accountOrigin}/site-auth?${new URLSearchParams({ output: siteOrigin, return_to: "/" })}`;
        await page.evaluate((src) => { const frame = document.createElement("iframe"); frame.src = src; document.body.append(frame); }, src);
        await page.frameLocator("iframe").getByRole("heading", { name: "published by fsite/v0.5.3" }).waitFor();
      } else {
        await page.goto(siteOrigin);
        await page.getByRole("heading", { name: "published by fsite/v0.5.3" }).waitFor();
        assert(navigations.some((url) => url.startsWith(`${gateOrigin}/authorize?`)));
      }
      assert(navigations.some((url) => url.startsWith(`${accountOrigin}/site-auth?`)));
      assert(navigations.some((url) => url.startsWith(`${siteOrigin}/_finite/auth?`)));
      assert(!navigations.some((url) => /\/(login|dev\/confirm|_finite\/request-link)(?:\?|$)/.test(url)));
      const cookies = await context.cookies(siteOrigin);
      assert(cookies.some((cookie) => cookie.name.includes("finite_site_auth")), "Sites minted its viewer cookie");
      assert(!(await context.cookies(gateOrigin)).some((cookie) => cookie.name.includes("gate")), "Gate owns no browser session");
      await context.close();
    }
  } catch (error) {
    throw new Error(`Local site-gate browser acceptance failed\n${output}`, { cause: error });
  } finally {
    await browser?.close();
    await Promise.all(children.map(stop));
    await Promise.all([rm(temp, { recursive: true, force: true }), rm(join(dashboardDir, dist), { recursive: true, force: true }), rm(join(dashboardDir, tsconfig), { force: true })]);
  }
});
