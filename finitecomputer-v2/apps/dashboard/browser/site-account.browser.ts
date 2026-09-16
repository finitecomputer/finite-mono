/** Real browser + real Sites with fresh and legacy state. Account authentication uses
 * the existing explicit local dev-account fixture, not a live WorkOS login.
 * Run after `cargo build -p finitesitesd -p fsite-cli` from the pinned shell.
 */
import assert from "node:assert/strict";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { once } from "node:events";
import { copyFile, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
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

for (const scenario of ["creator", "authorized", "anonymous", "unshared"] as const) {
test(`account bridge: ${scenario === "creator" ? "first visit after a fresh agent publish" : `${scenario} viewer of a deployed-version email share`}`, { timeout: 120_000 }, async () => {
  const temp = await mkdtemp(join(tmpdir(), "finite-site-account-browser-"));
  const dist = `.next-site-account-${process.pid}`;
  const tsconfig = `.site-account-tsconfig-${process.pid}.json`;
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
    const [dashboardPort, sitesPort] = await Promise.all([freePort(), freePort()]);
    const accountOrigin = `http://localhost:${dashboardPort}`;
    let siteOrigin = `http://legacy-email.sites.localhost:${sitesPort}`;
    const heading = scenario === "creator" ? "Fresh private site" : "published by fsite/v0.5.3";
    const token = randomBytes(32).toString("hex");
    const data = join(temp, "sites");
    await mkdir(data);
    if (scenario !== "creator") {
      execFileSync("tar", ["-xzf", join(repo, "finite-sites/crates/finitesitesd/tests/fixtures/legacy-email-v053/state.tar.gz"), "-C", data]);
    }
    start(join(repo, "target/debug/finitesitesd"), ["serve", "--data", data, "--listen", `127.0.0.1:${sitesPort}`, "--base-domain", "sites.localhost", "--api-url", `http://localhost:${sitesPort}`, "--git-url", `http://localhost:${sitesPort}`, "--site-scheme", "http", "--site-port", String(sitesPort), "--mailer", "dev", "--git-auto-reconcile", scenario === "creator" ? "true" : "false"], {
      FINITE_SITES_ACCOUNT_LOGIN_URL: `${accountOrigin}/site-auth`,
      FINITE_SITES_VIEWER_SESSION_TOKEN: token,
    });
    await copyFile(join(dashboardDir, "tsconfig.json"), join(dashboardDir, tsconfig));
    start(process.execPath, ["node_modules/next/dist/bin/next", "dev", "--webpack", "--hostname", "127.0.0.1", "--port", String(dashboardPort)], {
      NODE_ENV: "development", NEXT_DIST_DIR: dist, NEXT_TSCONFIG_PATH: tsconfig,
      FC_WORKOS_AUTH_ENABLED: "0", FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: scenario === "anonymous" ? "0" : "1",
      FC_DASHBOARD_DEV_EMAIL: scenario === "unshared" ? "stranger@example.com" : "friend@example.com", FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_local_browser_fixture",
      FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "local-browser-fixture-access-token",
      FC_SITES_UPSTREAM_URL: `http://localhost:${sitesPort}`, FC_SITES_ALLOW_LOCAL_OUTPUTS: "1", FINITE_SITES_VIEWER_SESSION_TOKEN: token,
    });
    await Promise.all([
      ready(`http://127.0.0.1:${sitesPort}/api/v2/healthz`, () => output),
      ready(`http://127.0.0.1:${dashboardPort}/health`, () => output),
    ]);
    if (scenario === "creator") {
      // Account evidence is synthetic, but assertion issuance, turn lease,
      // CLI, Git publishing and the browser's first visit use real services.
      const home = join(temp, "agent");
      const session = "first-publish-browser-fixture";
      const human = "aa".repeat(32);
      const env = { ...process.env, HOME: home, FINITE_HOME: join(home, ".finite"),
        FINITE_SITES_API: `http://localhost:${sitesPort}`, GIT_TERMINAL_PROMPT: "0",
        GIT_CONFIG_NOSYSTEM: "1", HERMES_SESSION_PLATFORM: "finitechat",
        HERMES_SESSION_KEY: session, HERMES_SESSION_USER_ID: human };
      await mkdir(home);
      const cli = (...args: string[]) => JSON.parse(execFileSync(join(repo, "target/debug/fsite"), args, { env, encoding: "utf8", timeout: 15_000 }));
      const agent = cli("auth", "register", "--output", "json").npub;
      const response = await fetch(`http://localhost:${sitesPort}/internal/v1/hosted-requester-assertions`, {
        method: "POST", headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
        body: JSON.stringify({ email: "friend@example.com", requester_npub: human, agent_npub: agent }),
      });
      assert.equal(response.status, 200);
      const assertion = await response.json();
      const leaseDir = join(env.FINITE_HOME, "requester-context-v2");
      await mkdir(leaseDir, { recursive: true, mode: 0o700 });
      await writeFile(join(leaseDir, `${createHash("sha256").update(session).digest("hex")}.json`), JSON.stringify({
        version: 2, session_key: session, platform: "finitechat", requesting_user_id: human,
        owner_email: "friend@example.com", hosted_requester_assertion: assertion.assertion,
        expires_at_unix: Math.floor(Date.now() / 1000) + 120,
      }), { mode: 0o600 });
      const source = join(temp, "source");
      await mkdir(join(source, "site"), { recursive: true });
      const config = join(source, "finite.toml");
      await writeFile(config, '[project]\nslug = "fresh-creator"\n[site]\nbranch = "main"\npath = "site"\n');
      await writeFile(join(source, "site/index.html"), "<!doctype html><h1>Fresh private site</h1>");
      const init = ["project", "init", "--config", config, "--output", "json"];
      assert.equal(cli(...init, "--dry-run").dry_run, true);
      const created = cli(...init);
      assert.equal(created.site.requesting_user_shared, true);
      assert.equal(cli(...init).project_id, created.project_id);
      const credential = cli("auth", "git", "fresh-creator", "--store", "--output", "json");
      const git = (...args: string[]) => execFileSync("git", args, { cwd: source, env, stdio: "pipe", timeout: 15_000 });
      git("init", "-b", "main");
      git("config", "user.name", "Sites browser fixture");
      git("config", "user.email", "fixture@example.test");
      git("remote", "add", "origin", credential.git_remote_url);
      git("add", "finite.toml", "site");
      git("commit", "-m", "First private publish");
      git("push", "origin", "main");
      const status = cli("project", "status", "fresh-creator", "--output", "json");
      assert.equal(status.site.visibility, "private");
      siteOrigin = status.site.url.replace(/\/$/, "");
    }
    browser = await chromium.launch({ ...chromiumLaunchOptions(), headless: true });
    for (const embedded of scenario === "authorized" || scenario === "creator" ? [false, true] : [false]) {
      const context = await browser.newContext();
      assert.deepEqual(await context.cookies(), []);
      const page = await context.newPage();
      const navigations: string[] = [];
      page.on("request", (request) => { if (request.isNavigationRequest()) navigations.push(request.url()); });
      if (scenario !== "authorized" && scenario !== "creator") {
        await page.goto(siteOrigin);
        if (scenario === "unshared") {
          await page.getByRole("heading", { name: "You don’t have access yet" }).waitFor();
          await page.getByRole("link", { name: "Try another email" }).click();
        }
        await page.getByRole("heading", { name: "This site is private" }).waitFor();
        assert.equal(navigations.filter((url) => url.startsWith(`${accountOrigin}/site-auth?`)).length, 1);
        assert(!navigations.some((url) => new URL(url).pathname === "/login"));
        // The retained guest challenge must still grant access to an allowed
        // mailbox, without provisioning a Finite account or a Chat identity.
        await page.locator('input[name="email"]').fill("friend@example.com");
        await page.getByRole("button", { name: "Send link" }).click();
        await page.getByRole("heading", { name: "Check your email" }).waitFor();
        const outbox = join(data, "outbox");
        const messages = await Promise.all((await readdir(outbox)).map((name) => readFile(join(outbox, name), "utf8")));
        const link = messages.join("\n").match(/http:\/\/[^\s]+\/_finite\/auth\?token=[a-f0-9]{64}/)?.[0];
        assert(link, "the real dev mailer delivered a guest sign-in link");
        await page.goto(link);
        await page.getByRole("heading", { name: heading }).waitFor();
      } else {
        if (embedded) {
          await page.goto(`${accountOrigin}/health`);
          const src = `${accountOrigin}/site-auth?${new URLSearchParams({ url: siteOrigin })}`;
          await page.evaluate((src) => { const frame = document.createElement("iframe"); frame.src = src; document.body.append(frame); }, src);
          await page.frameLocator("iframe").getByRole("heading", { name: heading }).waitFor();
        } else {
          await page.goto(siteOrigin);
          await page.getByRole("heading", { name: heading }).waitFor();
        }
        assert(navigations.some((url) => url.startsWith(`${accountOrigin}/site-auth?`)));
        assert(navigations.some((url) => url.startsWith(`${siteOrigin}/_finite/auth?session_token=`)));
        assert(!navigations.some((url) => /\/(login|_finite\/request-link|_finite\/sign-in)(?:\?|$)/.test(url)));
      }
      const cookies = await context.cookies(siteOrigin);
      assert(cookies.some((cookie) => cookie.name.includes("finite_site_auth")), "Sites minted its viewer cookie");
      await context.close();
    }
  } catch (error) {
    throw new Error(`Local site-account browser acceptance failed\n${output}`, { cause: error });
  } finally {
    await browser?.close();
    await Promise.all(children.map(stop));
    await Promise.all([rm(temp, { recursive: true, force: true }), rm(join(dashboardDir, dist), { recursive: true, force: true }), rm(join(dashboardDir, tsconfig), { force: true })]);
  }
});

}
