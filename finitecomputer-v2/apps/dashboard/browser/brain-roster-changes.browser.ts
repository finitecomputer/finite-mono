/** Admin roster changes, with a synthetic agent plugin and no production access. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { test } from "node:test";
import { chromium, type Browser, type Page, type Route } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

const key = "npub1ccz8l9zpa47k6vz9gphftsrumpw80rjt3nhnefat4symjhrsnmjs38mnyd";
const npub = (tag: string) => key.replace("ccz8", tag);
const acting = npub("caz8"), mia = npub("cdz8"), dan = npub("cez8"), una = npub("cfz8"), gus = npub("cgz8"), new1 = npub("chz8");
const job = (n: number) => n.toString(16).padStart(32, "0");
const short = (value: string) => `${value.slice(0, 12)}…${value.slice(-6)}`;

type Fake = {
  identities: Record<string, unknown>[]; invitations: Record<string, unknown>[];
  jobs: Map<string, { brainId: string; npub: string; polls: number; final: string }>;
  changes: { method: string; path: string; body: unknown }[];
  inviteAnswer: (target: string) => { status: number; json: unknown } | null;
  removalFinal: (npub: string) => string;
  /** The Agent is no longer an admin, or the Brain is gone. */
  gone: boolean;
};

function fake(): Fake {
  const blank = { kind: null, name: null, email: null, ownerEmail: null, folders: [] };
  return {
    identities: [
      { ...blank, npub: acting, role: "admin", kind: "agent", name: "Moss", ownerEmail: "owner@example.test" },
      { ...blank, npub: mia, role: "member", kind: "human", email: "mia@example.test" },
      { ...blank, npub: dan, role: "admin", kind: "agent", name: "Dan Agent", ownerEmail: "dan@example.test" },
      { ...blank, npub: una, role: "member", publicName: "una@finite.vip" },
      { ...blank, npub: gus, role: "guest" },
    ],
    invitations: [{ id: `invitation-${"1".repeat(32)}`, targetNpub: npub("cjz8"), targetName: "jo@finite.vip", expiresAt: "2026-11-02T00:00:00Z", expired: false }],
    jobs: new Map(), changes: [],
    inviteAnswer: () => null,
    removalFinal: () => "removed",
    gone: false,
  };
}

async function serveAgent(page: Page, base: string, state: Fake) {
  await page.route("**/api/chat/**", route => route.fulfill({ status: 503, json: {} }));
  await page.route("**/api/agents/*/hermes-access", route => route.fulfill({ status: 200,
    json: { baseUrl: "https://roster.fixture.test/", accessToken: "synthetic-test", expiresAt: 100 } }));
  let jobs = 0;
  await page.route("https://roster.fixture.test/**", async (route: Route) => {
    const request = route.request();
    const headers = { "access-control-allow-origin": base, "access-control-allow-headers": "authorization, content-type",
      "access-control-allow-methods": "GET, POST, DELETE" };
    if (request.method() === "OPTIONS") return route.fulfill({ status: 204, headers });
    assert.equal(request.headers().authorization, "Bearer synthetic-test");
    const url = new URL(request.url());
    const reply = (status: number, json: unknown) => route.fulfill({ status, headers, json });
    const parts = url.pathname.replace("/api/plugins/finite-brain/", "").split("/");
    if (request.method() !== "GET") {
      state.changes.push({ method: request.method(), path: url.pathname, body: request.postDataJSON() });
      if (request.method() !== "DELETE") assert.equal(request.headers()["content-type"], "application/json");
    }
    if (url.pathname.endsWith("/overview")) return reply(200, { version: 1, brains: [
      { id: "acme", name: "Acme Brain", kind: "organization", role: "admin", folders: [] },
      { id: "old", name: "Old Brain", kind: "organization", role: "admin", folders: [] },
      { id: "home", name: "Moss Personal", kind: "personal", role: "personal_agent", folders: [] },
    ] });
    if (state.gone && ["identities", "invitations", "removals"].includes(parts[0])) return reply(403, { error: "notAdmin" });
    if (parts[0] === "identities") {
      const report = { version: 1, brainId: parts[1], identities: state.identities };
      if (parts[1] === "old") return reply(200, report);
      // The Personal Brain is advertised too, so only the dashboard's own role check hides it.
      return reply(200, { ...report, changes: { actingNpub: acting, actions: ["invite", "revokeInvitation", "remove"] } });
    }
    if (parts[0] === "invitations" && request.method() === "GET") return reply(200, { version: 1, brainId: parts[1], invitations: state.invitations });
    if (parts[0] === "invitations" && request.method() === "POST") {
      const target = (request.postDataJSON() as { target: string }).target;
      const answer = state.inviteAnswer(target);
      if (answer) return reply(answer.status, answer.json);
      const invitation = { id: `invitation-${String(state.invitations.length + 2).repeat(32)}`, targetNpub: target.startsWith("npub1") ? target : new1,
        targetName: target.startsWith("npub1") ? null : target, expiresAt: "2026-11-03T00:00:00Z", expired: false };
      state.invitations.push(invitation);
      return reply(200, { version: 1, invitation });
    }
    if (parts[0] === "invitations" && request.method() === "DELETE") {
      state.invitations = state.invitations.filter(item => item.id !== parts[2]);
      return reply(200, { version: 1 });
    }
    if (parts[0] === "removals" && request.method() === "POST") {
      const target = (request.postDataJSON() as { npub: string }).npub;
      const id = job(++jobs);
      state.jobs.set(id, { brainId: parts[1], npub: target, polls: 0, final: state.removalFinal(target) });
      return reply(202, { version: 1, job: { id, state: "running" } });
    }
    if (parts[0] === "removals") {
      const entry = state.jobs.get(parts[2]);
      if (!entry) return reply(200, { version: 1, job: { id: parts[2], state: "unknown" } });
      if (++entry.polls < 2) return reply(200, { version: 1, job: { id: parts[2], state: "running" } });
      // "lost" models a Hermes restart: the agent no longer knows the job.
      if (entry.final === "lost") return reply(200, { version: 1, job: { id: parts[2], state: "unknown" } });
      if (entry.final === "removed") state.identities = state.identities.filter(row => row.npub !== entry.npub);
      if (entry.final === "demotedNotRemoved") state.identities = state.identities.map(row => row.npub === entry.npub ? { ...row, role: "member" } : row);
      return reply(200, { version: 1, job: { id: parts[2], state: entry.final } });
    }
    throw new Error(`unexpected agent request ${request.method()} ${url.pathname}`);
  });
}

async function startFixture() {
  const reservation = http.createServer().listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  const base = `http://127.0.0.1:${address.port}`;
  await new Promise<void>(resolve => reservation.close(() => resolve()));
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(address.port), FC_WEB_DESIGN_PREVIEWS: "0" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", chunk => { output += chunk; });
  server.stderr.on("data", chunk => { output += chunk; });
  const deadline = Date.now() + 60_000;
  while (!await fetch(`${base}/healthz`).then(r => r.ok).catch(() => false)) {
    assert.equal(server.exitCode, null, output);
    assert(Date.now() < deadline, output);
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  return { base, server };
}

async function shot(page: Page, name: string) {
  const artifacts = process.env.FC_BROWSER_ARTIFACT_DIR;
  if (!artifacts) return;
  // Let dialog fade-ins finish so the picture shows the settled state.
  await page.waitForFunction(`[...document.querySelectorAll("[role=dialog]")].every(d => getComputedStyle(d).opacity === "1")`);
  await mkdir(artifacts, { recursive: true });
  await page.screenshot({ path: path.join(artifacts, `brain-roster-changes-${name}.png`), fullPage: true });
}

test("an Organization Brain admin invites, revokes and removes through the agent", { timeout: 240_000 }, async () => {
  const { base, server } = await startFixture();
  let browser: Browser | undefined;
  try {
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    const state = fake();
    await serveAgent(page, base, state);
    await page.goto(`${base}/dashboard/machines/runtime_web_design/brain`);
    await page.getByRole("button", { name: "Acme Brain", exact: true }).click();
    const roster = page.getByRole("region", { name: "Identities in Acme Brain", exact: true });
    await roster.getByText("mia@example.test").waitFor();
    // Buttons only on Members and Admins, never on the acting agent's own key or a Guest.
    assert.deepEqual(await roster.getByRole("button").evaluateAll(buttons => buttons.map(button => button.getAttribute("aria-label"))),
      [`Remove mia@example.test (${short(mia)})`, `Remove Dan Agent (${short(dan)})`, `Remove una@finite.vip (${short(una)})`]);
    const pending = page.getByRole("region", { name: "Pending invitations for Acme Brain" });
    await pending.getByText("jo@finite.vip").waitFor();
    await shot(page, "admin-roster");

    // Invite by public name (typed in mixed case, sent lowercase), using the keyboard.
    const field = page.getByLabel("Invite someone");
    await field.fill("  New1@Finite.VIP ");
    await field.press("Enter");
    const status = page.getByRole("status").filter({ hasText: "Invited" });
    await status.getByText(`Invited new1@finite.vip (${short(new1)}) to Acme Brain as a Member. They must accept with the invited key.`).waitFor();
    await pending.getByText("new1@finite.vip").waitFor();
    assert.deepEqual(state.changes.at(-1), { method: "POST", path: "/api/plugins/finite-brain/invitations/acme", body: { target: "new1@finite.vip" } });
    assert.equal(await field.inputValue(), "");
    await shot(page, "invited");

    // Malformed input never reaches the agent.
    const before = state.changes.length;
    await field.fill("--server");
    await page.getByRole("button", { name: "Invite", exact: true }).click();
    await page.getByText("Enter an npub or a public name such as name@finite.vip.").waitFor();
    assert.equal(state.changes.length, before);

    // Revoke a pending invitation.
    await pending.getByRole("button", { name: `Revoke invitation for jo@finite.vip (${short(npub("cjz8"))})` }).click();
    await page.getByRole("status").getByText(`Revoked the invitation for jo@finite.vip (${short(npub("cjz8"))}).`).waitFor();
    await pending.getByText("jo@finite.vip").waitFor({ state: "detached" });
    assert.equal(state.changes.at(-1)?.method, "DELETE");

    // Busy and failed invitations show one plain line and are sent once.
    state.inviteAnswer = target => target === "busy@finite.vip" ? { status: 409, json: { error: "busy" } }
      : target === "fail@finite.vip" ? { status: 502, json: { error: "failed" } } : null;
    for (const [target, message] of [["busy@finite.vip", "Moss is already making a Brain change. Try again when it finishes."],
      ["fail@finite.vip", "Could not invite fail@finite.vip. Ask Moss in Chat for details."]]) {
      const sent = state.changes.length;
      await field.fill(target);
      await field.press("Enter");
      await page.getByRole("alert").filter({ hasText: message }).waitFor();
      assert.equal(state.changes.length, sent + 1);
    }
    await shot(page, "invite-failed");

    // Remove a Member with the keyboard: the dialog names them and explains re-keying.
    await roster.getByRole("button", { name: `Remove mia@example.test (${short(mia)})` }).focus();
    await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog", { name: "Remove mia@example.test?" });
    await dialog.getByText("will lose access to Acme Brain.", { exact: false }).waitFor();
    await dialog.getByText(short(mia), { exact: true }).waitFor();
    await dialog.getByText("Moss re-keys the Folders they could read, so they cannot read anything added after this.").waitFor();
    await dialog.getByText("Removal cannot take back what they already read or copied.").waitFor();
    await shot(page, "remove-dialog");
    await page.keyboard.press("Escape");
    await dialog.waitFor({ state: "detached" });
    assert.equal(state.changes.filter(item => item.path.includes("removals")).length, 0, "cancel sends nothing");
    await roster.getByRole("button", { name: `Remove mia@example.test (${short(mia)})` }).click();
    await dialog.getByRole("button", { name: "Remove", exact: true }).click();
    await page.getByRole("status").getByText(`Removing mia@example.test (${short(mia)}). This can take a few minutes.`).waitFor();
    // Every change button waits while one change runs.
    assert.equal(await page.getByRole("button", { name: `Remove Dan Agent (${short(dan)})` }).isDisabled(), true);
    await shot(page, "removing");
    await page.getByRole("status").getByText(`mia@example.test (${short(mia)}) was removed. Their Folders were re-keyed.`).waitFor({ timeout: 20_000 });
    await roster.getByText("mia@example.test").waitFor({ state: "detached" });
    await shot(page, "removed");

    // Remove an Admin whose demotion succeeds but removal fails.
    state.removalFinal = target => target === dan ? "demotedNotRemoved" : target === una ? "lost" : "removed";
    await roster.getByRole("button", { name: `Remove Dan Agent (${short(dan)})` }).click();
    const adminDialog = page.getByRole("dialog", { name: "Remove Dan Agent?" });
    await adminDialog.getByText("Dan Agent is an Admin. Moss removes the Admin role first, then removes them.").waitFor();
    await adminDialog.getByRole("button", { name: "Remove", exact: true }).click();
    await page.getByRole("alert").filter({ hasText: `Dan Agent (${short(dan)}) is now a Member and was not removed. Ask Moss in Chat for details.` })
      .waitFor({ timeout: 20_000 });
    await shot(page, "admin-partial");

    // A job the agent no longer knows (a restart) reads as unknown.
    await roster.getByRole("button", { name: `Remove una@finite.vip (${short(una)})` }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Remove", exact: true }).click();
    await page.getByRole("alert").filter({ hasText: `Could not confirm whether una@finite.vip (${short(una)}) was removed. Refresh to check.` })
      .waitFor({ timeout: 20_000 });
    await shot(page, "unknown-job");
    // Exactly one start request per removal: nothing was retried.
    assert.deepEqual(state.changes.filter(item => item.path.endsWith("/removals/acme")).map(item => (item.body as { npub: string }).npub), [mia, dan, una]);

    // An older agent: no buttons, one short line.
    await page.getByRole("button", { name: "Old Brain", exact: true }).click();
    await page.getByText("Update Moss to invite or remove people here.").waitFor();
    const oldRoster = page.getByRole("region", { name: "Identities in Old Brain", exact: true });
    await oldRoster.waitFor();
    assert.equal(await oldRoster.getByRole("button").count(), 0);
    assert.equal(await page.getByLabel("Invite someone").count(), 0);
    await shot(page, "older-agent");

    // A Personal Brain: no buttons and no update line, even when advertised.
    await page.getByRole("button", { name: "Moss Personal", exact: true }).click();
    await page.getByRole("region", { name: "Identities in Moss Personal", exact: true }).waitFor();
    assert.equal(await page.getByRole("region", { name: "Identities in Moss Personal", exact: true }).getByRole("button").count(), 0);
    assert.equal(await page.getByText("Update Moss to invite").count(), 0);
    assert.equal(await page.getByLabel("Invite someone").count(), 0);

    // Mobile: the page does not scroll sideways and the dialog fits.
    await page.setViewportSize({ width: 390, height: 844 });
    await page.getByRole("button", { name: "Acme Brain", exact: true }).click();
    await page.getByLabel("Invite someone").waitFor();
    await page.waitForFunction(() => {
      const sidebar = document.querySelector(".finite-agent-shell__sidebar");
      return (!sidebar || sidebar.getBoundingClientRect().right <= 0)
        && document.documentElement.scrollWidth <= innerWidth;
    });
    await shot(page, "mobile");
    await page.getByRole("region", { name: "Identities in Acme Brain", exact: true }).getByRole("button", { name: `Remove una@finite.vip (${short(una)})` }).click();
    const mobileDialog = page.getByRole("dialog");
    await mobileDialog.waitFor();
    const box = await mobileDialog.boundingBox();
    assert(box && box.x >= 0 && box.x + box.width <= 390, JSON.stringify(box));
    await shot(page, "mobile-dialog");
    await page.keyboard.press("Escape");
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    server.kill("SIGTERM");
    if (server.exitCode === null) await once(server, "exit");
  }
});

test("same names and private aliases never change which key a removal targets", { timeout: 240_000 }, async () => {
  const { base, server } = await startFixture();
  let browser: Browser | undefined;
  try {
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    const state = fake();
    // Two accounts chose the same name, and this viewer gave both keys the same private alias.
    const sam = { role: "member", kind: "human", name: "Sam Lee", ownerEmail: null, folders: [] };
    state.identities = [state.identities[0], { ...sam, npub: mia, email: "sam@one.test" }, { ...sam, npub: una, email: "sam@two.test" }];
    await serveAgent(page, base, state);
    // Stub of the dashboard alias lookup only; Core alias storage is proven by the Core proofs.
    const lookups: string[][] = [];
    await page.route("**/api/brain/key-aliases", route => {
      assert.equal(route.request().method(), "POST");
      const { npubs } = route.request().postDataJSON() as { npubs: string[] };
      lookups.push(npubs);
      return route.fulfill({ json: { aliases: npubs.filter(value => value !== acting).map(value => ({ npub: value, alias: "Research lead" })) } });
    });
    await page.goto(`${base}/dashboard/machines/runtime_web_design/brain`);
    await page.getByRole("button", { name: "Acme Brain", exact: true }).click();
    const roster = page.getByRole("region", { name: "Identities in Acme Brain", exact: true });
    const first = roster.getByRole("row").filter({ hasText: "sam@one.test" });
    const second = roster.getByRole("row").filter({ hasText: "sam@two.test" });
    await second.getByText("Research lead", { exact: true }).waitFor();
    await first.getByText("Research lead", { exact: true }).waitFor();
    assert.equal(await roster.locator("tbody").getByText("Your alias", { exact: true }).count(), 2);

    // The Remove button and dialog name the key from Finite's records; the alias sits apart, beside the full key.
    await second.getByRole("button", { name: `Remove Sam Lee (${short(una)})`, exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Remove Sam Lee?" });
    await dialog.getByText("Sam Lee, sam@two.test", { exact: false }).waitFor();
    await dialog.getByText(una, { exact: true }).waitFor();
    assert.equal(await dialog.getByText(mia, { exact: true }).count(), 0);
    assert.deepEqual(await dialog.locator("dl dt").allTextContents(), ["Your alias", "Key"]);
    assert.equal(await dialog.locator("dl dd").first().textContent(), "Research lead");
    await shot(page, "alias-remove-dialog");
    await dialog.getByRole("button", { name: "Remove", exact: true }).click();
    await page.getByRole("status").getByText(`Sam Lee (${short(una)}) was removed. Their Folders were re-keyed.`).waitFor({ timeout: 20_000 });
    assert.deepEqual(state.changes.filter(item => item.path.endsWith("/removals/acme")).map(item => item.body), [{ npub: una }]);
    await second.waitFor({ state: "detached" });
    // The refreshed key set is looked up again and the other Sam Lee keeps the alias.
    await first.getByText("Research lead", { exact: true }).waitFor();
    assert(lookups.some(npubs => npubs.join() === [acting, mia].join()), JSON.stringify(lookups));

    // An older Agent plugin and a Personal Brain still show names and aliases, with no change buttons.
    for (const [brainName, update] of [["Old Brain", true], ["Moss Personal", false]] as const) {
      await page.getByRole("button", { name: brainName, exact: true }).click();
      const view = page.getByRole("region", { name: `Identities in ${brainName}`, exact: true });
      await view.getByRole("row").filter({ hasText: "sam@one.test" }).getByText("Research lead", { exact: true }).waitFor();
      assert.equal(await view.getByRole("button", { name: /^Remove / }).count(), 0);
      assert.equal(await page.getByLabel("Invite someone").count(), 0);
      assert.equal(await page.getByText("Update Moss to invite or remove people here.").count(), update ? 1 : 0);
    }

    // The Agent loses admin (or the Brain is deleted) while this roster is open: the refusal stays visible.
    await page.getByRole("button", { name: "Acme Brain", exact: true }).click();
    await page.getByLabel("Invite someone").waitFor();
    state.gone = true;
    await page.getByLabel("Invite someone").fill(new1);
    await page.getByLabel("Invite someone").press("Enter");
    await page.getByRole("alert").filter({ hasText: "Moss could not confirm it is an admin of this Brain." }).waitFor();
    await page.getByText("Identities unavailable", { exact: true }).waitFor();
    assert.equal(await page.getByLabel("Invite someone").count(), 0);
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    server.kill("SIGTERM");
    if (server.exitCode === null) await once(server, "exit");
  }
});
