/** Invoked by Core's brain_release_composition test under devfinity: one
 * synthetic through-line across Brain deletion, people management and
 * account names/private aliases.
 * Real: Core, its identity listener and Postgres (from the Rust harness); the
 * Brain server asking that Core for descriptions; the Hosted Device; two
 * production dashboard servers; Chromium; Runner-rendered Caddy; native
 * Hermes with the sealed finite-brain plugin; fbrain.
 * Fixtures: WorkOS (Core's test signer), the Agent key pin, the Core
 * Hermes-access grant (answered with a real native Hermes token) and Chat.
 * Identity registration: the dashboards reach the real Hosted Device and
 * Core identity listener through pass-throughs. Fixtures there: the Device's
 * agent-binding open (no Chat server; a synthetic Chat state), an identity
 * read redirected to a synthetic account with no or an unreadable key, and
 * Core's hosted-observation path answered 404 (older Core) or 503 (outage).
 * Set FC_BRAIN_COMPOSITION_LEGACY_PLUGIN=1 when FC_BRAIN_COMPOSITION_HERMES
 * carries an older plugin that sends no identity resolution.
 * Env: FC_BRAIN_COMPOSITION_BIN_DIR (finite-brain, fbrain,
 * finitechat-hosted-device, finite-saas-runner), FC_BRAIN_COMPOSITION_HERMES,
 * FC_BRAIN_COMPOSITION_CADDY; optional FC_BRAIN_COMPOSITION_OLD_FBRAIN (a
 * released older fbrain), FC_BRAIN_COMPOSITION_LAB and FC_BROWSER_ARTIFACT_DIR.
 * Tokens arrive on stdin and are never printed.
 */
import assert from "node:assert/strict";
import { type ChildProcess, execFileSync, spawn, spawnSync } from "node:child_process";
import { createHash, randomBytes, X509Certificate } from "node:crypto";
import { once } from "node:events";
import { existsSync, readFileSync } from "node:fs";
import { cp, mkdir, mkdtemp, readFile, symlink, writeFile } from "node:fs/promises";
import http from "node:http";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import { connect } from "node:tls";
import { chromium, type Browser, type BrowserContext, type Locator, type Page } from "playwright";
import type { AccountAuthContext } from "../src/lib/dashboard-auth";
import { chromiumLaunchOptions } from "./playwright-browser";

type Input = {
  coreUrl: string; identityUrl: string; brainPort: number; runtimeId: string; owner: string; other: string;
  agentSecretHex: string; agentNpub: string; observationToken: string; descriptionToken: string;
};
type Json = Record<string, unknown>;

const required = (name: string) => {
  const value = process.env[name]?.trim();
  assert(value, `${name} is required`);
  return value;
};
const BIN = required("FC_BRAIN_COMPOSITION_BIN_DIR");
const HERMES = required("FC_BRAIN_COMPOSITION_HERMES");
const CADDY = required("FC_BRAIN_COMPOSITION_CADDY");
const OLD_FBRAIN = process.env.FC_BRAIN_COMPOSITION_OLD_FBRAIN?.trim();
const ARTIFACTS = process.env.FC_BROWSER_ARTIFACT_DIR?.trim();
const DEVICE_TOKEN = "composition-hosted-device-token";
// The current plugin forwards Core's resolution; an older Runtime plugin sends none.
const LEGACY_PLUGIN = process.env.FC_BRAIN_COMPOSITION_LEGACY_PLUGIN?.trim() === "1";
const NOT_ACTED = LEGACY_PLUGIN ? "Identity details are unavailable." : "This key has not acted in this Brain yet.";
const NOT_SHARED = LEGACY_PLUGIN ? "Identity details are unavailable." : "Identity details are not available to this Brain.";
const HOSTED_OBSERVATION_PATH = "/api/core/internal/v1/hosted-account-observations";
const DESCRIPTION_VERSIONS = {
  v1: "finite-core-brain-identity-descriptions-v1",
  v2: "finite-core-brain-identity-descriptions-v2",
  v3: "finite-core-brain-descriptions-v3",
} as const;
const OWNER = { workosUserId: "runtime-auth-user", email: "runtime-auth@finite.test" };
const OTHER = { workosUserId: "composition-other", email: "other@finite.test" };
const short = (npub: string) => `${npub.slice(0, 12)}…${npub.slice(-6)}`;
/** bech32 npub to its 32-byte hex key (checksum already verified by its issuer). */
function npubHex(npub: string) {
  const charset = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
  const bytes: number[] = [];
  let acc = 0, bits = 0;
  for (const char of npub.slice(5, -6)) {
    acc = ((acc << 5) | charset.indexOf(char)) & 0xffff;
    bits += 5;
    while (bits >= 8) { bits -= 8; bytes.push((acc >> bits) & 0xff); }
  }
  return Buffer.from(bytes.slice(0, 32)).toString("hex");
}
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
const children: ChildProcess[] = [];
const log: string[] = [];
const note = (line: string) => { console.log(line); log.push(line); };

async function freePort() {
  const server = http.createServer().listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  await new Promise<void>(resolve => server.close(() => resolve()));
  return address.port;
}

function start(name: string, command: string, args: string[], env: Record<string, string | undefined>) {
  const child = spawn(command, args, { env: env as NodeJS.ProcessEnv, stdio: ["ignore", "pipe", "pipe"] });
  let output = "";
  child.stdout?.on("data", chunk => { output = (output + chunk).slice(-40_000); });
  child.stderr?.on("data", chunk => { output = (output + chunk).slice(-40_000); });
  children.push(child);
  return Object.assign(child, { label: name, logs: () => output });
}

async function stop(child: ChildProcess) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, "exit");
  child.kill("SIGTERM");
  await Promise.race([exited, delay(8_000)]);
  if (child.exitCode === null && child.signalCode === null) { child.kill("SIGKILL"); await exited; }
}

async function waitFor(what: string, check: () => Promise<boolean>, child?: ChildProcess & { logs: () => string }, ms = 120_000) {
  const deadline = Date.now() + ms;
  while (!(await check().catch(() => false))) {
    if (child) assert.equal(child.exitCode, null, `${what} exited: ${child.logs().slice(-3_000)}`);
    assert(Date.now() < deadline, `${what} was not ready in time`);
    await delay(200);
  }
}

type Relayed = { status: number; type: string; body: Buffer };
const reply = (status: number, json?: unknown): Relayed =>
  ({ status, type: "application/json", body: Buffer.from(json === undefined ? "" : JSON.stringify(json)) });
const relays: http.Server[] = [];

/** A loopback pass-through to a real service; `handle` may answer instead. */
async function passThrough(upstream: string, handle: (request: http.IncomingMessage, body: Buffer, forward: () => Promise<Relayed>) => Promise<Relayed>) {
  const server = http.createServer(async (request, response) => {
    const chunks: Buffer[] = [];
    for await (const chunk of request) chunks.push(chunk as Buffer);
    const body = Buffer.concat(chunks);
    const forward = async (): Promise<Relayed> => {
      const headers = Object.fromEntries(Object.entries(request.headers).filter((entry): entry is [string, string] =>
        typeof entry[1] === "string" && !["host", "connection", "content-length", "transfer-encoding", "accept-encoding"].includes(entry[0])));
      const upstreamResponse = await fetch(`${upstream}${request.url}`, { method: request.method, headers,
        body: request.method === "GET" || request.method === "HEAD" ? undefined : body });
      return { status: upstreamResponse.status, type: upstreamResponse.headers.get("content-type") ?? "application/octet-stream",
        body: Buffer.from(await upstreamResponse.arrayBuffer()) };
    };
    try {
      const out = await handle(request, body, forward);
      response.writeHead(out.status, { "content-type": out.type });
      response.end(out.body);
    } catch {
      response.writeHead(502);
      response.end();
    }
  }).listen(0, "127.0.0.1");
  await once(server, "listening");
  relays.push(server);
  const address = server.address();
  assert(address && typeof address === "object");
  return `http://127.0.0.1:${address.port}`;
}

async function main() {
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  const config = JSON.parse(input) as Input;
  for (const origin of [config.coreUrl, config.identityUrl]) assert.equal(new URL(origin).hostname, "127.0.0.1");
  const lab = process.env.FC_BRAIN_COMPOSITION_LAB?.trim() || await mkdtemp(path.join(os.tmpdir(), "brain-composition-"));
  await mkdir(lab, { recursive: true });
  if (ARTIFACTS) await mkdir(ARTIFACTS, { recursive: true });
  let browser: Browser | undefined;
  let hermes: (ChildProcess & { logs: () => string }) | undefined;
  const credentials = { username: `run-${randomBytes(4).toString("hex")}`, password: randomBytes(24).toString("base64url") };
  try {
    // Brain server, asking Core's identity listener to describe keys.
    const brainUrl = `http://127.0.0.1:${config.brainPort}`;
    const brain = start("brain", path.join(BIN, "finite-brain"), [], {
      FINITE_BRAIN_ADDR: `127.0.0.1:${config.brainPort}`, FINITE_BRAIN_PUBLIC_BASE_URL: brainUrl,
      FINITE_BRAIN_DB: path.join(lab, "brain.sqlite3"), FINITE_BRAIN_PROTECTED_RATE_LIMIT: "100000:60",
      FINITE_BRAIN_CORE_IDENTITY_URL: config.identityUrl, FINITE_BRAIN_CORE_DESCRIPTION_TOKEN: config.descriptionToken,
    });
    await waitFor("Brain server", async () => (await fetch(`${brainUrl}/health`)).ok, brain);

    // Hosted Device holding each account's human Brain key.
    const devicePort = await freePort();
    const deviceUrl = `http://127.0.0.1:${devicePort}`;
    const device = start("hosted-device", path.join(BIN, "finitechat-hosted-device"), [
      "--bind", `127.0.0.1:${devicePort}`, "--data-root", path.join(lab, "hosted"),
      "--server-url", "http://127.0.0.1:9", "--public-url", "http://chat.test",
    ], { FINITECHAT_HOSTED_API_TOKEN: DEVICE_TOKEN });
    const provision = (user: string) => fetch(`${deviceUrl}/v1/app/state`, {
      headers: { authorization: `Bearer ${DEVICE_TOKEN}`, "x-finite-workos-user-id": user },
    }).then(response => response.ok);
    await waitFor("Hosted Device", () => provision(OWNER.workosUserId), device);
    assert(await provision(OTHER.workosUserId));
    process.env.FC_BRAIN_PUBLIC_ORIGIN = brainUrl;
    const { hostedBrainDeletionTransport } = await import("../src/lib/brain-deletion");
    const hosted = (account: typeof OWNER) => {
      const context: AccountAuthContext = { ...account, emailVerified: true, source: "dev" };
      return hostedBrainDeletionTransport({ baseUrl: deviceUrl, apiToken: DEVICE_TOKEN }, context, brainUrl);
    };
    const owner = hosted(OWNER);
    const other = hosted(OTHER);
    const ownerNpub = (await owner.identify()).npub;
    const otherNpub = (await other.identify()).npub;
    const identifyHex = async (user: string) => {
      const response = await fetch(`${deviceUrl}/v1/brain/identity-provider`, { method: "POST", headers: {
        authorization: `Bearer ${DEVICE_TOKEN}`, "x-finite-workos-user-id": user, "x-finite-brain-public-origin": brainUrl,
        "content-type": "application/json" }, body: JSON.stringify({ version: "finite-brain-identity-provider-v1", operation: "identifyMember", input: {} }) });
      assert.equal(response.status, 200);
      return String(((await response.json()) as Json).publicKeyHex);
    };
    const ownerHex = await identifyHex(OWNER.workosUserId);
    // Core's own description reads (the Brain server's private interface).
    const describeMany = async (version: keyof typeof DESCRIPTION_VERSIONS, brain: string, keys: string[], participating: string[] = []) => {
      const request: Json = { version: DESCRIPTION_VERSIONS[version], brainServer: brainUrl, brainId: brain,
        requestedByPublicKeyHex: keys[0], publicKeysHex: keys, ...(version === "v3" ? { participatingPublicKeysHex: participating } : {}) };
      const response = await fetch(`${config.identityUrl}/api/core/internal/v1/brain-identity-descriptions`, { method: "POST",
        headers: { "content-type": "application/json", "x-finite-brain-description-credential": config.descriptionToken }, body: JSON.stringify(request) });
      assert.equal(response.status, 200, `describe ${version}: HTTP ${response.status}`);
      return ((await response.json()) as { results: Json[] }).results;
    };
    const describe = async (version: keyof typeof DESCRIPTION_VERSIONS, brain: string, key: string) => (await describeMany(version, brain, [key]))[0];
    assert.equal(npubHex(ownerNpub), ownerHex);

    // FIXTURE (Chat transport): no Chat server runs, so the Device's
    // agent-binding open answers a synthetic Chat state. Every other Device
    // request from the dashboards reaches the real Device unchanged, except an
    // identity read redirected to a synthetic account for the refusal cases.
    const chatState = { identity: { account_id: "c".repeat(64), device_id: "hosted-web" }, rev: 1, rooms: [] };
    const deviceTap = { openFails: false, identifyAs: null as string | null, identify: [] as Array<{ status: number }>, operations: [] as string[] };
    const deviceRelay = await passThrough(deviceUrl, async (request, body, forward) => {
      if (request.method === "POST" && request.url === "/v1/app/agent-bindings/open") {
        return deviceTap.openFails ? reply(503, { error: "synthetic Chat outage" }) : reply(200, chatState);
      }
      if (request.url !== "/v1/brain/identity-provider") return forward();
      const operation = String((JSON.parse(body.toString()) as Json).operation);
      deviceTap.operations.push(operation);
      if (operation === "identifyMember" && deviceTap.identifyAs) request.headers["x-finite-workos-user-id"] = deviceTap.identifyAs;
      const response = await forward();
      if (operation === "identifyMember") deviceTap.identify.push({ status: response.status });
      return response;
    });
    // FIXTURE (Core version/outage): the hosted-observation path can answer
    // like an older Core without the route (404) or an outage (503).
    const coreTap = { hosted: "forward" as "forward" | 404 | 503, posts: [] as Array<{ status: number; body: Json; outcome?: unknown }> };
    const coreRelay = await passThrough(config.identityUrl, async (request, body, forward) => {
      if (request.url !== HOSTED_OBSERVATION_PATH) return forward();
      const posted = JSON.parse(body.toString()) as Json;
      if (coreTap.hosted !== "forward") {
        coreTap.posts.push({ status: coreTap.hosted, body: posted });
        return reply(coreTap.hosted);
      }
      const response = await forward();
      coreTap.posts.push({ status: response.status, body: posted,
        outcome: response.status === 200 ? (JSON.parse(response.body.toString()) as Json).outcome : undefined });
      return response;
    });

    // Agent and CLI identities. The Agent's key is the one Core pinned for its runtime.
    const agentEnv = (label: string) => ({ FINITE_HOME: path.join(lab, label), FBRAIN_CONFIG_DIR: path.join(lab, label, "brain-config"),
      FINITE_BRAIN_SERVER_URL: brainUrl, HOME: path.join(lab, label) });
    const run = (binary: string, label: string, args: string[], cwd = lab, stdin?: string) =>
      spawnSync(binary, args, { cwd, input: stdin, env: { NODE_ENV: "test", PATH: process.env.PATH, ...agentEnv(label) }, encoding: "utf8" });
    const cli = (label: string, args: string[], cwd = lab, stdin?: string) => {
      const result = run(path.join(BIN, "fbrain"), label, args, cwd, stdin);
      assert.equal(result.status, 0, `fbrain ${args.slice(0, 3).join(" ")} failed: ${result.stderr.slice(-1_500)}`);
      return args.includes("--json") ? JSON.parse(result.stdout) as Json : result.stdout;
    };
    const npub: Record<string, string> = {};
    for (const [label, secret] of [["moss", config.agentSecretHex], ["bob", ""], ["dave", ""], ["carol", ""]]) {
      cli(label, ["auth", "import", "--json"], lab, secret || randomBytes(32).toString("hex"));
      npub[label] = String((cli(label, ["auth", "status", "--json"]) as Json).npub);
    }
    assert.equal(npub.moss, config.agentNpub);
    const brainId = String((cli("moss", ["brain", "create", "organization", "Field Lab", "--json"]) as Json).brainId);
    const mossTree = path.join(lab, "moss-tree");
    cli("moss", ["open", brainId, mossTree, "--json"]);
    cli("moss", ["folder", "create", "Field Notes", "--access", "all_members", "--json"], mossTree);
    await writeFile(path.join(mossTree, "Field Notes", "intro.md"), "# Intro\n\nWritten before any change.\n");
    cli("moss", ["sync", "now", "--json"], mossTree);
    const keyVersion = () => ((cli("moss", ["brain", "metadata", "--brain", brainId, "--json"]) as { folders: Array<{ id: string; currentKeyVersion: number }> })
      .folders.find(folder => folder.id === "field-notes")!.currentKeyVersion);
    const accessNpubs = () => ((cli("moss", ["access", "list", "--brain", brainId, "--json"]) as { identities: Array<{ npub: string }> })
      .identities.map(row => row.npub).sort());
    // Two unrelated Brains that must survive the deletion unchanged.
    const others = ["content-brain", "marketing-comms-brain"];
    for (const [id, name] of [["content-brain", "Content"], ["marketing-comms-brain", "Marketing Comms"]]) {
      await owner.signedRequest("POST", "/v1/brains", JSON.stringify({ brainId: id, kind: "organization", name }));
    }
    const digest = async (id: string) => String(((await owner.signedRequest("GET", `/v1/brains/${id}/deletion-preview`)) as Json).stateDigest);
    const othersBefore = await Promise.all(others.map(digest));
    note(`brain=${brainId} agent=${short(npub.moss)} owner=${short(ownerNpub)} other=${short(otherNpub)}`);

    // Native Hermes as the Agent, with the sealed finite-brain plugin and this tree's fbrain.
    const hermesPort = await freePort();
    const hermesDir = path.join(lab, "hermes");
    const fbrainOnly = path.join(lab, "fbrain-bin");
    await mkdir(fbrainOnly, { recursive: true });
    if (!existsSync(path.join(fbrainOnly, "fbrain"))) await symlink(path.join(BIN, "fbrain"), path.join(fbrainOnly, "fbrain"));
    await mkdir(path.join(hermesDir, "home"), { recursive: true, mode: 0o700 });
    await writeFile(path.join(hermesDir, "home", "config.yaml"), "dashboard:\n  basic_auth: {}\nplugins:\n  disabled: [dashboard_auth/nous]\n");
    for (const dir of ["codex", "kanban", "state", "cache", "config"]) await mkdir(path.join(hermesDir, dir), { recursive: true });
    hermes = start("hermes", HERMES, ["serve", "--host", "127.0.0.1", "--port", String(hermesPort), "--no-open"], {
      ...agentEnv("moss"), PATH: `${fbrainOnly}:${process.env.PATH}`, HOME: hermesDir, LANG: "en_US.UTF-8",
      HERMES_HOME: path.join(hermesDir, "home"), CODEX_HOME: path.join(hermesDir, "codex"), HERMES_KANBAN_HOME: path.join(hermesDir, "kanban"),
      XDG_STATE_HOME: path.join(hermesDir, "state"), XDG_CACHE_HOME: path.join(hermesDir, "cache"), XDG_CONFIG_HOME: path.join(hermesDir, "config"),
      FINITE_PRIVATE_CONTROL_URL: "http://127.0.0.1:9/", HERMES_DASHBOARD_PUBLIC_URL: `https://hermes-proof.invalid/runtimes/${config.runtimeId}/`,
      HERMES_DASHBOARD_BASIC_AUTH_USERNAME: credentials.username, HERMES_DASHBOARD_BASIC_AUTH_PASSWORD: credentials.password,
      HERMES_DASHBOARD_BASIC_AUTH_SECRET: randomBytes(24).toString("hex"), HERMES_DASHBOARD_BASIC_AUTH_TTL_SECONDS: "3600",
      HERMES_NO_AUTO_UPDATE: "1", PYTHONDONTWRITEBYTECODE: "1",
    });
    const hermesChild = hermes;
    await waitFor("Hermes", async () => (await fetch(`http://127.0.0.1:${hermesPort}/api/auth/providers`)).ok, hermesChild, 240_000);

    // Two production dashboard servers on real Core, one per account.
    async function dashboard(token: string, account: typeof OWNER) {
      const port = await freePort();
      const base = `http://127.0.0.1:${port}`;
      const env: NodeJS.ProcessEnv = { NODE_ENV: "production" };
      for (const name of ["PATH", "HOME", "TMPDIR", "SSL_CERT_FILE", "NIX_SSL_CERT_FILE"]) if (process.env[name]) env[name] = process.env[name];
      const server = start(`dashboard-${account.workosUserId}`, process.execPath,
        ["node_modules/next/dist/bin/next", "start", "--hostname", "127.0.0.1", "--port", String(port)], {
          ...env, NEXT_TELEMETRY_DISABLED: "1", FC_CORE_BASE_URL: config.coreUrl, FC_DASHBOARD_BASE_URL: base,
          FC_WORKOS_AUTH_ENABLED: "0", FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1", FC_DASHBOARD_DEV_EMAIL: account.email,
          FC_DASHBOARD_DEV_WORKOS_USER_ID: account.workosUserId, FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: token,
          FC_HOSTED_WEB_DEVICE_URL: deviceRelay, FINITECHAT_HOSTED_API_TOKEN: DEVICE_TOKEN,
          FC_BRAIN_UPSTREAM_URL: brainUrl, FC_BRAIN_PUBLIC_ORIGIN: brainUrl,
          FC_CORE_BRAIN_IDENTITY_URL: coreRelay, FC_CORE_BRAIN_OBSERVATION_TOKEN: config.observationToken,
        });
      await waitFor(`dashboard ${account.email}`, async () => (await fetch(`${base}/healthz`)).ok, server);
      return base;
    }
    const ownerBase = await dashboard(config.owner, OWNER);
    const otherBase = await dashboard(config.other, OTHER);

    // Runner-rendered Caddy edge that allows exactly the owner's dashboard origin.
    const edgePort = await freePort();
    const edgeOrigin = `https://localhost:${edgePort}`;
    const runtimePath = `/runtimes/${config.runtimeId}/`;
    const caddyDir = path.join(lab, "caddy");
    await mkdir(caddyDir, { recursive: true });
    await writeFile(path.join(caddyDir, "manifest.json"), JSON.stringify({ public_origin: edgeOrigin, listen: `127.0.0.1:${edgePort}`,
      admin_socket: path.join(await mkdtemp("/tmp/bcomp-"), "admin.sock"), allowed_origins: [ownerBase], routes: [{ runtime_id: config.runtimeId, host_port: hermesPort }] }));
    await writeFile(path.join(caddyDir, "caddy.json"), execFileSync(path.join(BIN, "finite-saas-runner"),
      ["render-hosted-hermes-caddy", "--manifest", path.join(caddyDir, "manifest.json")]));
    const caddy = start("caddy", CADDY, ["run", "--config", path.join(caddyDir, "caddy.json")],
      { PATH: process.env.PATH, HOME: caddyDir, XDG_DATA_HOME: path.join(caddyDir, "data"), XDG_CONFIG_HOME: path.join(caddyDir, "config") });
    const caPath = path.join(caddyDir, "data/caddy/pki/authorities/local/root.crt");
    let ca = "";
    const edge = (method: string, pathname: string, body?: string) => new Promise<{ status: number; cookies: string[] }>((resolve, reject) => {
      const request = https.request({ host: "127.0.0.1", port: edgePort, servername: "localhost", ca, method, path: pathname,
        headers: { host: `localhost:${edgePort}`, ...(body ? { "content-type": "application/json", "content-length": Buffer.byteLength(body) } : {}) } }, response => {
        response.resume();
        response.on("end", () => resolve({ status: response.statusCode ?? 0, cookies: response.headers["set-cookie"] ?? [] }));
      });
      request.on("error", reject);
      request.end(body);
    });
    await waitFor("Caddy edge", async () => {
      ca = existsSync(caPath) ? readFileSync(caPath, "utf8") : "";
      return !!ca && (await edge("GET", `${runtimePath}api/auth/providers`)).status === 200;
    }, caddy);
    const leaf = await new Promise<Buffer>((resolve, reject) => {
      const socket = connect({ host: "127.0.0.1", port: edgePort, servername: "localhost", ca }, () => {
        resolve(socket.getPeerCertificate().raw); socket.end();
      });
      socket.on("error", reject);
    });
    const spki = [ca, leaf].map(cert => createHash("sha256").update(new X509Certificate(cert).publicKey
      .export({ type: "spki", format: "der" })).digest("base64")).join(",");
    // FIXTURE: Core's owner-only Hermes-access grant. One real native session, minted once.
    let nativeToken: string | null = null;
    const grantToken = async () => {
      if (nativeToken) return nativeToken;
      const login = await edge("POST", `${runtimePath}auth/password-login`, JSON.stringify({ provider: "basic", ...credentials }));
      assert.equal(login.status, 200);
      const cookie = login.cookies.find(value => value.split("=", 1)[0].endsWith("hermes_session_at"));
      assert(cookie, "native login set no session cookie");
      nativeToken = cookie.split(";")[0].slice(cookie.indexOf("=") + 1);
      return nativeToken;
    };

    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions(), args: [`--ignore-certificate-errors-spki-list=${spki}`] });
    const context: BrowserContext = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
    await context.route("**/api/chat/**", route => route.fulfill({ status: 503, json: {} }));
    await context.route("**/api/agents/*/hermes-access", async route => route.request().method() !== "POST"
      ? route.fulfill({ status: 404, json: {} })
      : route.fulfill({ status: 200, json: { baseUrl: `${edgeOrigin}${runtimePath}`, accessToken: await grantToken(), expiresAt: Math.floor(Date.now() / 1000) + 300 } }));
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    let shots = 0;
    const shot = async (name: string, target: Page = page) => {
      if (!ARTIFACTS) return;
      await target.waitForFunction(`[...document.querySelectorAll("[role=dialog]")].every(d => getComputedStyle(d).opacity === "1")`);
      const dialogOpen = await target.getByRole("dialog").count() > 0;
      const file = `${String(++shots).padStart(2, "0")}-${name}.png`;
      await target.screenshot({ path: path.join(ARTIFACTS, file), fullPage: !dialogOpen });
      note(`screenshot ${file}`);
    };
    const rosterOf = (target: Page) => target.getByRole("region", { name: "Identities in Field Lab", exact: true });
    const roster = () => rosterOf(page);
    const row = (text: string) => roster().getByRole("row").filter({ hasText: text });
    const status = (text: string) => page.getByRole("status").getByText(text, { exact: false });
    const alert = (text: string, target: Page = page) => target.getByRole("alert").filter({ hasText: text });
    async function openRoster(target: Page) {
      await target.goto(`${ownerBase}/dashboard/machines/${config.runtimeId}/brain`);
      await target.getByRole("button", { name: "Field Lab", exact: true }).click({ timeout: 120_000 });
      await rosterOf(target).waitFor({ timeout: 60_000 });
    }
    async function refresh() {
      await page.getByRole("button", { name: "Refresh brains", exact: true }).click();
      await page.waitForFunction(() => document.querySelector('section[aria-label="Brain memberships"]')?.getAttribute("aria-busy") === "false");
      await delay(300);
      await page.waitForFunction(() => [...document.querySelectorAll("section[aria-busy]")].every(s => s.getAttribute("aria-busy") === "false"));
    }
    async function invite(value: string) {
      const field = page.getByLabel("Invite someone");
      await field.fill(value);
      await field.press("Enter");
    }
    // A human joins through the dashboard's retained invitation routes (no
    // current UI caller): list, then accept with the hosted key. The accept
    // route records Core's account observation.
    async function humanAccepts(base: string) {
      const listed = await (await fetch(`${base}/api/brain/invitations`)).json() as { invitations: Array<{ brainId: string; inviteCode: string; expired: boolean }> };
      const pending = listed.invitations.filter(item => item.brainId === brainId && !item.expired);
      assert.equal(pending.length, 1, JSON.stringify(listed));
      const accepted = await fetch(`${base}/api/brain/invitations/accept`, { method: "POST",
        headers: { "content-type": "application/json", origin: base }, body: JSON.stringify({ inviteCode: pending[0].inviteCode, shareAccountContact: true }) });
      assert.equal(accepted.status, 200, await accepted.text());
      // The admin Agent's sync issues the new member's Folder key grants.
      cli("moss", ["sync", "now", "--json"], mossTree);
    }
    function agentAccepts(label: string) {
      const inbox = (cli(label, ["invite", "brain", "list", "--json"]) as { invitations: Array<{ id: string }> }).invitations;
      assert.equal(inbox.length, 1);
      cli(label, ["invite", "brain", "accept", inbox[0].id, "--json"]);
      cli("moss", ["sync", "now", "--json"], mossTree);
      return inbox[0].id;
    }

    await openRoster(page);
    const agentName = /accessible to (.+?)\.(?: \d+ pending\.)?$/u.exec(await page.locator(".brain-heading p").innerText())?.[1];
    assert(agentName, "agent name missing from the Brain heading");
    await page.getByText("1 identities").waitFor();
    // The Agent's own row is named by Core through Brain, fbrain and the plugin.
    await row(short(npub.moss)).getByText(OWNER.email, { exact: true }).waitFor({ timeout: 30_000 });
    await shot("roster-agent-named-by-core");

    // Identity: after a successful Hosted Chat bootstrap the dashboard loads
    // the owner's existing hosted key and registers it with Core, after the
    // response. The page is parked so only these requests reach the Device.
    await page.goto("about:blank");
    const states = async (key: string, brain = brainId) =>
      [(await describe("v1", brain, key)).state, (await describe("v2", brain, key)).state, (await describe("v3", brain, key)).state];
    assert.deepEqual(await states(ownerHex), ["notShared", "notShared", "notShared"]);
    const chatUrl = `${ownerBase}/api/chat/machines/${config.runtimeId}/hosted-device/state`;
    async function openChat(expect: { identify: boolean; posts: number }) {
      const before = { identify: deviceTap.identify.length, operations: deviceTap.operations.length, posts: coreTap.posts.length };
      const response = await fetch(chatUrl, { cache: "no-store" });
      const delivered = await response.json() as Json;
      if (expect.identify) await waitFor("deferred identity read", async () => deviceTap.identify.length > before.identify, undefined, 15_000);
      if (expect.posts) await waitFor("deferred Core observation", async () => coreTap.posts.length >= before.posts + expect.posts, undefined, 15_000);
      await delay(1_500);
      assert.equal(deviceTap.identify.length, before.identify + (expect.identify ? 1 : 0));
      assert.equal(coreTap.posts.length, before.posts + expect.posts);
      assert(deviceTap.operations.slice(before.operations).every(operation => operation === "identifyMember"), "registration signed something");
      return { status: response.status, delivered, identify: deviceTap.identify.slice(before.identify), posts: coreTap.posts.slice(before.posts) };
    }
    const outcomes: string[] = [];
    const delivered = async (expect: { identify: boolean; posts: number }) => {
      const opened = await openChat(expect);
      assert.equal(opened.status, 200);
      assert.deepEqual(opened.delivered, chatState);
      return opened;
    };
    // No key on the Device for the account read: the Device refuses; nothing reaches Core.
    deviceTap.identifyAs = "composition-no-hosted-key";
    let opened = await delivered({ identify: true, posts: 0 });
    outcomes.push(`missing key: Device ${opened.identify[0].status}`);
    assert.equal(opened.identify[0].status, 428);
    // An unreadable key file: the Device refuses; nothing reaches Core.
    const unreadable = path.join(lab, "hosted", "users", createHash("sha256").update("composition-unreadable-key").digest("hex"));
    await mkdir(path.join(unreadable, "finite-home", "identity"), { recursive: true, mode: 0o700 });
    await mkdir(path.join(unreadable, "chat"), { recursive: true, mode: 0o700 });
    await writeFile(path.join(unreadable, "finite-home", "identity", "identity.json"), "not an identity", { mode: 0o600 });
    await writeFile(path.join(unreadable, "chat", "client.sqlite3"), "", { mode: 0o600 });
    deviceTap.identifyAs = "composition-unreadable-key";
    opened = await delivered({ identify: true, posts: 0 });
    outcomes.push(`unreadable key: Device ${opened.identify[0].status}`);
    assert(opened.identify[0].status >= 400);
    deviceTap.identifyAs = null;
    // An older Core (no route) and a Core outage leave Chat untouched; the outage retries the same operation once.
    coreTap.hosted = 404;
    opened = await delivered({ identify: true, posts: 1 });
    coreTap.hosted = 503;
    opened = await delivered({ identify: true, posts: 2 });
    assert.equal(opened.posts[0].body.operationId, opened.posts[1].body.operationId);
    assert.deepEqual(await states(ownerHex), ["notShared", "notShared", "notShared"]);
    outcomes.push("older Core 404 and outage 503: Chat 200, nothing stored");
    // A failed Chat bootstrap schedules nothing.
    coreTap.hosted = "forward";
    deviceTap.openFails = true;
    opened = await openChat({ identify: false, posts: 0 });
    deviceTap.openFails = false;
    assert.notEqual(opened.status, 200);
    outcomes.push(`failed Chat bootstrap: HTTP ${opened.status}, no identity read`);
    // Success: the verified Account and the Device-loaded key, nothing else.
    opened = await delivered({ identify: true, posts: 1 });
    assert.deepEqual(opened.identify, [{ status: 200 }]);
    assert.deepEqual(Object.keys(opened.posts[0].body).sort(), ["observedAt", "operationId", "publicKeyHex", "version"]);
    assert.equal(opened.posts[0].body.publicKeyHex, ownerHex);
    assert.deepEqual([opened.posts[0].status, opened.posts[0].outcome], [200, "recorded"]);
    const registered = await describe("v2", brainId, ownerHex);
    assert.deepEqual([registered.state, registered.kind, registered.accountEmail, registered.displayName, (registered.source as Json).kind],
      ["resolved", "human", OWNER.email, "Ada Lovelace", "hostedDeviceObservation"]);
    // Registration is not disclosure: no sharing scope (v1), no participation (v3), no membership.
    assert.deepEqual(await states(ownerHex), ["notShared", "resolved", "notShared"]);
    assert(!accessNpubs().includes(ownerNpub));
    opened = await delivered({ identify: true, posts: 1 });
    assert.deepEqual([opened.posts[0].status, opened.posts[0].outcome], [200, "unchanged"]);
    outcomes.push("success: recorded, then unchanged; v1/v2/v3 = notShared/resolved/notShared");
    note(`hosted registration: ${outcomes.join("; ")}`);
    await openRoster(page);

    // People: every invitation is sent from the roster and signed by the Agent.
    await invite(ownerNpub);
    await status(`Invited ${short(ownerNpub)} to Field Lab as a Member.`).waitFor();
    await humanAccepts(ownerBase);
    cli("moss", ["admin", "role", "grant", "admin", "--brain", brainId, "--target", ownerNpub, "--json"]);
    await invite(otherNpub);
    await status(`Invited ${short(otherNpub)}`).waitFor();
    // The other account signs in once; the dashboard layout enrolls it with Core.
    const otherContext = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
    await otherContext.route("**/api/chat/**", route => route.fulfill({ status: 503, json: {} }));
    await (await otherContext.newPage()).goto(`${otherBase}/dashboard`, { waitUntil: "load" });
    await otherContext.close();
    await humanAccepts(otherBase);
    await invite(npub.bob);
    await status(`Invited ${short(npub.bob)}`).waitFor();
    agentAccepts("bob");
    const bobTree = path.join(lab, "bob-tree");
    cli("bob", ["open", brainId, bobTree, "--json"]);
    assert.match(await readFile(path.join(bobTree, "Field Notes", "intro.md"), "utf8"), /Written before/u);
    const pending = page.getByRole("region", { name: "Pending invitations for Field Lab" });
    await invite(npub.dave);
    await pending.getByText(short(npub.dave)).waitFor();
    await pending.getByRole("button", { name: `Revoke invitation for ${short(npub.dave)}` }).click();
    await status(`Revoked the invitation for ${short(npub.dave)}.`).waitFor();
    assert.deepEqual((cli("dave", ["invite", "brain", "list", "--json"]) as Json).invitations, []);

    // Names: both humans carry the same WorkOS name; Core supplies each account's email.
    for (let attempt = 0; ; attempt++) {
      await refresh();
      const text = await roster().innerText().catch(() => "");
      if (text.includes(OTHER.email) && text.includes(OWNER.email) && (await page.getByText("4 identities").count())) break;
      assert(attempt < 20, `names did not arrive: ${text}`);
      await delay(1_000);
    }
    const ownerRow = row(short(ownerNpub));
    const otherRow = row(short(otherNpub));
    const bobRow = row(short(npub.bob));
    for (const [target, email, role] of [[ownerRow, OWNER.email, "Admin"], [otherRow, OTHER.email, "Member"]] as const) {
      assert.match(await target.innerText(), new RegExp(`Ada Lovelace[\\s\\S]*${email}[\\s\\S]*${role}`, "u"));
    }
    // Bob participates but no Account is shared for his key; the copy never names a cause.
    await bobRow.getByText(NOT_SHARED, { exact: true }).waitFor();
    await shot("roster-same-name-humans");

    // Private aliases through real Core: the same alias on two different keys.
    async function setAlias(target: Locator, alias: string, duplicate: boolean) {
      await target.getByRole("button", { name: /^(Add|Edit) alias for / }).click();
      await page.getByRole("textbox", { name: "Alias", exact: true }).fill(alias);
      if (duplicate) await page.getByText("You already use this alias for another key here. Check the key before saving.", { exact: true }).waitFor();
      await page.getByRole("button", { name: "Save alias", exact: true }).click();
      await page.getByRole("dialog").waitFor({ state: "hidden" });
      await target.getByText(alias, { exact: true }).waitFor();
    }
    await setAlias(otherRow, "Research lead", false);
    await waitFor("alias focus", () => otherRow.getByRole("button", { name: "Edit alias for Research lead", exact: true })
      .evaluate(element => element === document.activeElement), undefined, 10_000);
    await setAlias(bobRow, "Research lead", true);
    assert.match(await otherRow.innerText(), /Research lead[\s\S]*Your alias[\s\S]*Ada Lovelace[\s\S]*other@finite\.test/u);
    await shot("aliases-over-names");
    const lookup = async (base: string) => (await (await fetch(`${base}/api/brain/key-aliases`, { method: "POST",
      headers: { "content-type": "application/json", origin: base }, body: JSON.stringify({ npubs: [ownerNpub, otherNpub, npub.bob] }) })).json()) as { aliases: Json[] };
    assert.equal((await lookup(ownerBase)).aliases.length, 2);
    assert.deepEqual(await lookup(otherBase), { aliases: [] });

    // Same name, same alias: the removal confirmation shows the exact key and only that key is removed.
    let version = keyVersion();
    await otherRow.getByRole("button", { name: `Remove Ada Lovelace (${short(otherNpub)})`, exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Remove Ada Lovelace?" });
    await dialog.getByText(`Ada Lovelace, ${OTHER.email}`, { exact: false }).waitFor();
    await dialog.getByText(otherNpub, { exact: true }).waitFor();
    assert.equal(await dialog.getByText(ownerNpub, { exact: true }).count(), 0);
    assert.equal(await dialog.locator("dl dd").first().textContent(), "Research lead");
    await shot("remove-same-name-and-alias");
    await dialog.getByRole("button", { name: "Remove", exact: true }).click();
    await status(`Ada Lovelace (${short(otherNpub)}) was removed. Their Folders were re-keyed.`).waitFor({ timeout: 120_000 });
    await page.getByText("3 identities").waitFor();
    assert.equal(keyVersion(), version + 1);
    assert.deepEqual(accessNpubs(), [ownerNpub, npub.moss, npub.bob].sort());
    await bobRow.getByText("Research lead", { exact: true }).waitFor();
    const otherBrains = (await other.signedRequest("GET", "/v1/brains")) as { brains: Array<{ brainId: string }> };
    assert(!otherBrains.brains.some(item => item.brainId === brainId));
    await shot("after-exact-key-removal");

    // Mobile: no sideways page scroll; both dialogs fit.
    await page.setViewportSize({ width: 390, height: 844 });
    await page.waitForFunction(() => {
      const sidebar = document.querySelector(".finite-agent-shell__sidebar");
      return (!sidebar || sidebar.getBoundingClientRect().right <= 0) && document.documentElement.scrollWidth <= innerWidth;
    });
    await shot("mobile-roster");
    for (const name of [/^Remove /u, /^Edit alias for /u]) {
      await bobRow.getByRole("button", { name }).click();
      const box = await page.getByRole("dialog").boundingBox();
      assert(box && box.x >= 0 && box.x + box.width <= 390, JSON.stringify(box));
      if (String(name).includes("Remove")) await shot("mobile-remove-dialog");
      await page.keyboard.press("Escape");
      await page.getByRole("dialog").waitFor({ state: "hidden" });
    }
    await page.setViewportSize({ width: 1440, height: 1000 });

    // Deletion preview, then roster changes through the Agent invalidate it.
    const panel = page.getByRole("region", { name: "Delete Field Lab", exact: true });
    const facts = () => panel.locator("dl").first().evaluate(list => Object.fromEntries(
      [...list.querySelectorAll("dt")].map(term => [term.textContent, term.nextElementSibling?.textContent])));
    const previewDigest = () => page.evaluate(async id => (await (await fetch(`/api/brain/deletion?brainId=${id}`, { cache: "no-store" })).json()).stateDigest as string, brainId);
    const confirmInput = panel.getByLabel(`Type ${brainId} to confirm`);
    // A refused preview disables the input until the fresh review arrives.
    const reviewAgain = async () => {
      await panel.getByRole("button", { name: "Review the current Brain again" }).click();
      await waitFor("fresh deletion preview", () => confirmInput.isEnabled(), undefined, 30_000);
    };
    const deleteButton = panel.getByRole("button", { name: "Delete Brain permanently" });
    await panel.getByRole("button", { name: "Review deletion…" }).click();
    await panel.getByText("Delete Field Lab permanently?").waitFor();
    const first = await facts();
    assert.equal(first.Members, "3 (2 admins)");
    assert.equal(first["Pending invitations"], "0");
    const firstDigest = await previewDigest();
    // A private alias is account state in Core, not Brain state.
    await setAlias(bobRow, "Contractor", false);
    assert.equal(await previewDigest(), firstDigest);
    await shot("deletion-preview");
    // Unchanged count: invite and revoke through the roster.
    await invite(npub.dave);
    await pending.getByText(short(npub.dave)).waitFor();
    await pending.getByRole("button", { name: `Revoke invitation for ${short(npub.dave)}` }).click();
    await status(`Revoked the invitation for ${short(npub.dave)}.`).waitFor();
    await confirmInput.fill(brainId);
    await deleteButton.click();
    await alert("changed after its deletion preview").waitFor();
    await shot("stale-preview-after-invite-and-revoke");
    await reviewAgain();
    const second = await facts();
    assert.deepEqual([second.Members, second["Pending invitations"]], [first.Members, first["Pending invitations"]]);
    const secondDigest = await previewDigest();
    assert.notEqual(secondDigest, firstDigest);
    // Changed count: remove the aliased unknown key through the roster.
    version = keyVersion();
    await bobRow.getByRole("button", { name: /^Remove / }).click();
    const bobDialog = page.getByRole("dialog", { name: "Remove Unknown key?" });
    await bobDialog.getByText(npub.bob, { exact: true }).waitFor();
    assert.equal(await bobDialog.locator("dl dd").first().textContent(), "Contractor");
    await bobDialog.getByRole("button", { name: "Remove", exact: true }).click();
    await status(`Unknown key (${short(npub.bob)}) was removed. Their Folders were re-keyed.`).waitFor({ timeout: 120_000 });
    assert.equal(keyVersion(), version + 1);
    await confirmInput.fill(brainId);
    await deleteButton.click();
    await alert("changed after its deletion preview").waitFor();
    // Pending work that the deletion must stop.
    await invite(npub.dave);
    await pending.getByText(short(npub.dave)).waitFor();
    const daveInvitation = (cli("dave", ["invite", "brain", "list", "--json"]) as { invitations: Array<{ id: string }> }).invitations[0].id;
    const token = String((cli("moss", ["invite-token", "create", "--brain", brainId, "--role", "member", "--json"]) as Json).token);
    // An older client install for the Agent key with its own tree, opened before deletion.
    const oldHome = path.join(lab, "moss-old");
    if (OLD_FBRAIN) {
      await cp(path.join(lab, "moss", "identity"), path.join(oldHome, "identity"), { recursive: true });
      const opened = run(OLD_FBRAIN, "moss-old", ["open", brainId, path.join(lab, "moss-old-tree"), "--json"]);
      assert.equal(opened.status, 0, opened.stderr);
    }
    await reviewAgain();
    const third = await facts();
    assert.equal(third.Members, "2 (2 admins)");
    assert.equal(third["Pending invitations"], "1");
    assert.equal(third["Open invite tokens"], "1");
    // Two more tabs keep the pre-deletion roster open, one per stale action.
    const staleInviteTab = await context.newPage();
    await openRoster(staleInviteTab);
    const staleRemoveTab = await context.newPage();
    await openRoster(staleRemoveTab);
    await page.bringToFront();
    await confirmInput.fill(brainId);
    await deleteButton.click();
    await panel.getByRole("status").filter({ hasText: `Deleted Field Lab (${brainId}).` }).waitFor({ timeout: 60_000 });
    await shot("deleted-receipt");

    // The Brain is gone for the Agent and for the browser.
    await panel.getByRole("button", { name: "Done" }).click();
    await page.waitForFunction(() => document.querySelector('section[aria-label="Brain memberships"]')?.getAttribute("aria-busy") === "false");
    await waitFor("Field Lab to leave the list", async () => (await page.getByRole("button", { name: "Field Lab", exact: true }).count()) === 0, undefined, 30_000);
    assert(!((cli("moss", ["brain", "list", "--json"]) as { brains: Array<{ brainId: string }> }).brains.some(item => item.brainId === brainId)));
    await shot("after-deletion");
    await assert.rejects(owner.signedRequest("GET", `/v1/brains/${brainId}/deletion-preview`), (error: { status?: number }) => error.status === 410);

    // Stale roster actions from the old tab are refused, each with one plain line.
    const refusal = async (staleTab: Page) => {
      const notice = staleTab.getByRole("alert").filter({ hasText: "could not confirm it is an admin of this Brain." });
      await notice.waitFor({ timeout: 60_000 }).catch(async () => {
        throw new Error(`no refusal shown; notices: ${JSON.stringify(await staleTab.locator("[role=alert],[role=status]").allInnerTexts())}`);
      });
      return (await notice.innerText()).trim();
    };
    await staleInviteTab.getByLabel("Invite someone").fill(npub.carol);
    await staleInviteTab.getByLabel("Invite someone").press("Enter");
    const staleInvite = await refusal(staleInviteTab);
    await shot("stale-invite-refused", staleInviteTab);
    await rosterOf(staleRemoveTab).getByRole("row").filter({ hasText: short(ownerNpub) }).getByRole("button", { name: /^Remove / }).click();
    await staleRemoveTab.getByRole("dialog").getByRole("button", { name: "Remove", exact: true }).click();
    const staleRemove = await refusal(staleRemoveTab);
    await shot("stale-remove-refused", staleRemoveTab);
    note(`stale tabs: invite -> "${staleInvite}"; remove -> "${staleRemove}"`);
    assert.deepEqual((cli("carol", ["invite", "brain", "list", "--json"]) as Json).invitations, []);

    // Invitations and invite tokens stop; the other Brains are untouched.
    const daveAccept = run(path.join(BIN, "fbrain"), "dave", ["invite", "brain", "accept", daveInvitation, "--json"]);
    assert.notEqual(daveAccept.status, 0);
    const redeemed = run(path.join(BIN, "fbrain"), "carol", ["invite-accept", token, "--json"]);
    assert.notEqual(redeemed.status, 0);
    note(`after deletion: invitation accept: ${daveAccept.stderr.trim().slice(0, 200)}; invite token: ${redeemed.stderr.trim().slice(0, 200)}`);
    assert.deepEqual(await Promise.all(others.map(digest)), othersBefore);
    const listed = (await owner.signedRequest("GET", "/v1/brains")) as { brains: Array<{ brainId: string }> };
    assert.deepEqual(listed.brains.map(item => item.brainId).sort(), others);

    // The current client stops at the deletion and keeps local files; an older client cannot bring it back.
    const stopped = run(path.join(BIN, "fbrain"), "moss", ["sync", "now", "--json"], mossTree);
    assert.notEqual(stopped.status, 0);
    assert(existsSync(path.join(mossTree, "Field Notes", "intro.md")));
    if (OLD_FBRAIN) {
      await writeFile(path.join(lab, "moss-old-tree", "Field Notes", "older.md"), "# Older\n");
      const oldSync = run(OLD_FBRAIN, "moss-old", ["sync", "now", "--json"], path.join(lab, "moss-old-tree"));
      assert.notEqual(oldSync.status, 0);
      assert.match(oldSync.stderr, /410/u);
      const recreate = run(OLD_FBRAIN, "moss-old", ["brain", "create", "organization", "Field Lab", "--json"]);
      assert.notEqual(recreate.status, 0);
      assert.match(recreate.stderr, /409/u);
      assert(existsSync(path.join(lab, "moss-old-tree", "Field Notes", "older.md")));
      note("older client: sync refused (410), recreate refused (409), local file kept");
    } else {
      note("SKIPPED older client: FC_BRAIN_COMPOSITION_OLD_FBRAIN not set");
    }
    assert.deepEqual(((await owner.signedRequest("GET", "/v1/brains")) as { brains: Array<{ brainId: string }> }).brains.map(item => item.brainId).sort(), others);

    // Identity Lab: the registered owner key, added by the Agent without acting,
    // stays noParticipation; Bob acts with no shared Account. Each row explains
    // itself without a name, and mutations still confirm the exact key.
    const identityLab = String((cli("moss", ["brain", "create", "organization", "Identity Lab", "--json"]) as Json).brainId);
    cli("moss", ["admin", "member", "add", "--brain", identityLab, "--target", ownerNpub, "--json"]);
    cli("moss", ["invite", "brain", "create", "--brain", identityLab, "--target", npub.bob, "--json"]);
    const bobInbox = (cli("bob", ["invite", "brain", "list", "--json"]) as { invitations: Json[] }).invitations
      .filter(item => JSON.stringify(item).includes(identityLab));
    assert.equal(bobInbox.length, 1);
    cli("bob", ["invite", "brain", "accept", String(bobInbox[0].id), "--json"]);
    const labAccess = () => ((cli("moss", ["access", "list", "--brain", identityLab, "--json"]) as { identities: Array<{ npub: string }> })
      .identities.map(item => item.npub).sort());
    assert.deepEqual(labAccess(), [ownerNpub, npub.moss, npub.bob].sort());
    // Core knows the owner's Account; Brain alone withholds it for want of participation.
    assert.equal((await describe("v2", identityLab, ownerHex)).state, "resolved");
    // The owner's participating Agent is described, but its owner hints never
    // include the owner's registered, non-participating key.
    const [agentRow, ownerKeyRow] = await describeMany("v3", identityLab, [npubHex(npub.moss), ownerHex], [npubHex(npub.moss)]);
    assert.deepEqual([agentRow.state, agentRow.kind, ownerKeyRow.state], ["resolved", "agent", "notShared"]);
    assert.equal((agentRow.responsibleAccount as Json).email, OWNER.email);
    assert(!JSON.stringify(agentRow).includes(ownerHex), JSON.stringify(agentRow));
    await page.goto(`${ownerBase}/dashboard/machines/${config.runtimeId}/brain`);
    await page.getByRole("button", { name: "Identity Lab", exact: true }).click({ timeout: 120_000 });
    const labRoster = page.getByRole("region", { name: "Identities in Identity Lab", exact: true });
    await labRoster.waitFor({ timeout: 60_000 });
    const labRow = (key: string) => labRoster.getByRole("row").filter({ hasText: short(key) });
    await labRow(ownerNpub).getByText(NOT_ACTED, { exact: true }).waitFor({ timeout: 30_000 });
    await labRow(npub.bob).getByText(NOT_SHARED, { exact: true }).waitFor();
    await labRow(npub.moss).getByText(OWNER.email, { exact: true }).waitFor();
    for (const key of [ownerNpub, npub.bob]) {
      const text = await labRow(key).innerText();
      assert(!text.includes(OWNER.email) && !text.includes("Ada Lovelace") && !text.includes("Unknown"), text);
    }
    // A private alias leads; the safe explanation stays beside it.
    await labRow(ownerNpub).getByRole("button", { name: /^Add alias for / }).click();
    await page.getByRole("textbox", { name: "Alias", exact: true }).fill("Chat owner");
    await page.getByRole("button", { name: "Save alias", exact: true }).click();
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    await labRow(ownerNpub).getByText("Chat owner", { exact: true }).waitFor();
    assert.match(await labRow(ownerNpub).innerText(), new RegExp(`Chat owner[\\s\\S]*Your alias[\\s\\S]*${NOT_ACTED.replace(".", "\\.")}`, "u"));
    await shot("identity-lab-roster");
    // Removal confirms the full key and the alias, never the explanation.
    await labRow(ownerNpub).getByRole("button", { name: /^Remove / }).click();
    const labDialog = page.getByRole("dialog", { name: "Remove Unknown key?" });
    await labDialog.getByText(ownerNpub, { exact: true }).waitFor();
    assert.equal(await labDialog.locator("dl dd").first().textContent(), "Chat owner");
    assert.equal(await labDialog.getByText(NOT_ACTED, { exact: false }).count(), 0);
    await shot("identity-lab-remove-dialog");
    await labDialog.getByRole("button", { name: "Remove", exact: true }).click();
    await status(`Unknown key (${short(ownerNpub)}) was removed.`).waitFor({ timeout: 120_000 });
    assert.deepEqual(labAccess(), [npub.moss, npub.bob].sort());
    note(`identity lab: owner "${NOT_ACTED}", Bob "${NOT_SHARED}"; alias + exact-key removal${LEGACY_PLUGIN ? " (older plugin)" : ""}`);
    assert.deepEqual(errors, []);
    note(`agent=${agentName} native-token-minted=${nativeToken !== null}`);
    console.log("PASS: hosted key registration after a successful Chat bootstrap (missing/unreadable key, older Core, outage and failed bootstrap leave Chat unchanged; no scope, participation or membership); names via Core->Brain->fbrain->plugin; invite/accept/revoke/remove signed by the Agent with re-keying; same-name and same-alias removal of the exact key; alias privacy and digest independence; stale deletion previews after invite+revoke and after removal; Hosted Device deletion; stale roster actions refused; invites stopped; other Brains unchanged; no resurrection");
    console.log("BOUNDARY: WorkOS provider, Agent key pin, Core Hermes-access grant, Chat (incl. the Device's agent-binding open), the refusal accounts' identity reads and Core 404/503 answers are fixtures; humans accept through retained dashboard routes without a UI caller; production rollout is not claimed.");
  } catch (error) {
    note(`FAIL ${error instanceof Error ? error.stack : String(error)}`);
    if (hermes) await writeFile(path.join(lab, "hermes-tail.log"), hermes.logs().replaceAll(credentials.password, "[redacted]")).catch(() => undefined);
    for (const child of children) await writeFile(path.join(lab, `${(child as ChildProcess & { label?: string }).label ?? "child"}.log`),
      ((child as ChildProcess & { logs?: () => string }).logs?.() ?? "").replaceAll(credentials.password, "[redacted]")).catch(() => undefined);
    process.exitCode = 1;
  } finally {
    await writeFile(path.join(lab, "run.log"), `${log.join("\n")}\n`).catch(() => undefined);
    await browser?.close();
    for (const relay of relays) { relay.closeAllConnections(); relay.close(); }
    for (const child of children.reverse()) await stop(child);
  }
}

main().catch(error => { console.error(error instanceof Error ? error.message : "composition proof failed"); process.exitCode = 1; });
