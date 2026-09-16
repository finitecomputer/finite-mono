/** Real Sites registries + fsite. Core/account/Hosted Device evidence is synthetic;
 * the test isolates registry-bound authorization, not a live WorkOS/Chat login. */
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { once } from "node:events";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { createHostedRequesterContext } from "../src/lib/hosted-web-chat";

const execute = promisify(execFile);
const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");

test("dashboard requester assertion authorizes only the publishing registry, including dry-run and replay", { timeout: 60_000 }, async (t) => {
  const previous = { ...process.env };
  t.after(() => { process.env = previous; });
  const temp = await mkdtemp(join(tmpdir(), "sites-requester-"));
  t.after(() => rm(temp, { recursive: true, force: true }));
  const serviceToken = randomBytes(32).toString("hex");
  const human = "aa".repeat(32);
  let agent = "";
  let redirectTo = "";
  const account = { email: "requester@example.test", workosUserId: "user_requester",
    accessToken: "synthetic-account-token", emailVerified: true, source: "workos" as const };
  const fixture = createServer((request, response) => {
    response.setHeader("content-type", "application/json");
    if (request.url === "/api/core/v1/me") {
      response.end(JSON.stringify({ email: account.email, workos_user_id: account.workosUserId }));
    } else if (request.url === "/v1/app/state") {
      response.end(JSON.stringify({ identity: { account_id: human }, hosted_agent_binding: { agent_npub: agent } }));
    } else if (request.url === "/internal/v1/hosted-requester-assertions" && redirectTo) {
      response.writeHead(307, { location: `${redirectTo}/internal/v1/hosted-requester-assertions` });
      response.end();
    } else { response.writeHead(404); response.end(); }
  }).listen(0, "127.0.0.1");
  await once(fixture, "listening");
  t.after(async () => {
    fixture.closeAllConnections();
    await new Promise<void>((done) => fixture.close(() => done()));
  });
  const address = fixture.address();
  assert(address && typeof address !== "string");
  const fixtureOrigin = `http://127.0.0.1:${address.port}`;
  process.env.FC_CORE_BASE_URL = fixtureOrigin;
  process.env.FINITE_SITES_VIEWER_SESSION_TOKEN = serviceToken;

  async function sites(name: string) {
    const reservation = createServer().listen(0, "127.0.0.1");
    await once(reservation, "listening");
    const address = reservation.address();
    assert(address && typeof address !== "string");
    const port = address.port;
    await new Promise<void>((done) => reservation.close(() => done()));
    const origin = `http://127.0.0.1:${port}`;
    const child = spawn(join(repo, "target/debug/finitesitesd"), [
      "serve", "--data", join(temp, name), "--listen", `127.0.0.1:${port}`,
      "--base-domain", "sites.localhost", "--api-url", origin, "--git-url", origin,
      "--site-scheme", "http", "--site-port", String(port), "--mailer", "dev",
    ], { env: { ...process.env, FINITE_SITES_ACCOUNT_LOGIN_URL: "" }, stdio: "ignore" });
    t.after(async () => {
      if (child.exitCode !== null || child.signalCode !== null) return;
      const exited = once(child, "exit");
      child.kill("SIGTERM");
      const timer = setTimeout(() => child.kill("SIGKILL"), 3000);
      await exited;
      clearTimeout(timer);
    });
    for (let attempt = 0; attempt < 100; attempt++) {
      try {
        if ((await fetch(`${origin}/api/v2/healthz`, { signal: AbortSignal.timeout(500) })).ok) return origin;
      } catch { /* bounded service startup */ }
      if (child.exitCode !== null) throw new Error(`${name} service exited`);
      await new Promise((done) => setTimeout(done, 100));
    }
    throw new Error(`${name} service did not start`);
  }
  const publishing = await sites("publishing");
  const otherRegistry = await sites("other-registry");
  process.env.FC_SITES_UPSTREAM_URL = otherRegistry;
  process.env.FC_SITES_V2_UPSTREAM_URL = publishing;
  const home = join(temp, "agent");
  const session = "synthetic-authenticated-turn";
  const cliEnv = { ...process.env, FINITE_HOME: home, FINITE_SITES_API: publishing,
    HERMES_SESSION_PLATFORM: "finitechat", HERMES_SESSION_KEY: session, HERMES_SESSION_USER_ID: human };
  async function cli(...args: string[]) {
    const { stdout } = await execute(join(repo, "target/debug/fsite"), args, { env: cliEnv, timeout: 15_000 });
    return JSON.parse(stdout);
  }
  await cli("auth", "register", "--output", "json");
  agent = (await cli("auth", "status", "--output", "json")).npub;
  const config = join(temp, "finite.toml");
  await writeFile(config, '[project]\nslug = "requester-proof"\n\n[site]\npath = "site"\nbranch = "main"\n');
  const context = { config: { baseUrl: fixtureOrigin, apiToken: "synthetic-device-token" }, account };
  const leaseDir = join(home, "requester-context-v2");
  await mkdir(leaseDir, { recursive: true, mode: 0o700 });
  async function lease(assertion: string) {
    await writeFile(join(leaseDir, `${createHash("sha256").update(session).digest("hex")}.json`), JSON.stringify({
      version: 2, session_key: session, platform: "finitechat", requesting_user_id: human,
      owner_email: account.email, hosted_requester_assertion: assertion,
      expires_at_unix: Math.floor(Date.now() / 1000) + 120,
    }), { mode: 0o600 });
  }
  const init = ["project", "init", "--config", config, "--output", "json"];
  process.env.FC_SITES_V2_UPSTREAM_URL = otherRegistry;
  const wrong = await createHostedRequesterContext(context);
  assert(wrong);
  await lease(wrong.sitesAssertion);
  await assert.rejects(cli(...init, "--dry-run"), /not_authorized|not authorized/i);

  process.env.FC_SITES_V2_UPSTREAM_URL = publishing;
  const requester = await createHostedRequesterContext(context);
  assert(requester);
  await lease(requester.sitesAssertion);
  assert.equal((await cli(...init, "--dry-run")).dry_run, true);
  assert.equal((await cli(...init, "--dry-run")).dry_run, true);
  // A dry-run must not persist an email grant that hides a wrong-registry token.
  await lease(wrong.sitesAssertion);
  await assert.rejects(cli(...init, "--dry-run"), /not_authorized|not authorized/i);
  await lease(requester.sitesAssertion);
  const created = await cli(...init);
  assert.equal(created.site.requesting_user_shared, true);
  assert.equal((await cli(...init)).project_id, created.project_id);

  // A redirect must not move the service credential or issue a foreign token.
  redirectTo = otherRegistry;
  process.env.FC_SITES_V2_UPSTREAM_URL = fixtureOrigin;
  assert.equal(await createHostedRequesterContext(context), undefined);
});
