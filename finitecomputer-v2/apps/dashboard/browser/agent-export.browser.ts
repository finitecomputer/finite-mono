/** Browser contract proof for the Agent export. The agent's finite-export
 * plugin is a local HTTPS stand-in with a throwaway certificate; the
 * dashboard page, its download route, Core's grant and the chat composer are
 * the real ones. Every credential here is synthetic. */
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import http from "node:http";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

const RUNTIME = "runtime_web_design";
const PERSONAL = "personal-0123456789abcdef";
const FILE_NAME = "finite-export-moss-20261029-1804.zip";

type Job = { state: "running" | "ready"; reads: number };

test("the owner surveys, exports, downloads and deletes the whole agent; /export in chat opens the page", { timeout: 240_000 }, async () => {
  const scratch = await mkdtemp(path.join(os.tmpdir(), "agent-export-browser-"));
  const certificate = path.join(scratch, "cert.pem");
  const key = path.join(scratch, "key.pem");
  execFileSync("openssl", [
    "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "1",
    "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1", "-keyout", key, "-out", certificate,
  ], { stdio: "ignore" });

  const archive = randomBytes(3 * 1024 * 1024);
  let survey: Job | null = null;
  let exported: Job | null = null;
  const bodies: unknown[] = [];
  const downloads: Array<{ authorization?: string; cookie?: string; origin?: string; range?: string }> = [];
  let base = "";
  const agent = https.createServer({ key: await readFile(key), cert: await readFile(certificate) }, async (request, response) => {
    const cors = { "access-control-allow-origin": base, "access-control-allow-headers": "authorization, content-type", "access-control-allow-methods": "GET, POST, DELETE" };
    if (request.method === "OPTIONS") { response.writeHead(204, cors).end(); return; }
    const json = (status: number, body: unknown) => response.writeHead(status, { ...cors, "content-type": "application/json", "cache-control": "no-store" }).end(JSON.stringify(body));
    if (request.headers.authorization !== "Bearer synthetic-test") { json(401, { detail: "Unauthorized" }); return; }
    const route = new URL(request.url ?? "/", "https://agent").pathname.replace(`/${RUNTIME}/api/plugins/finite-export`, "");
    const started = "2026-10-29T18:00:00Z";
    const view = () => ({
      version: 1,
      survey: survey && { state: survey.state, startedAt: started, finishedAt: survey.state === "ready" ? "2026-10-29T18:00:05Z" : null,
        ...(survey.state === "ready" ? { summary: {
          conversations: 214, memories: 2, skills: 3, scheduledJobs: 2, files: 5, fileBytes: 38 * 1024 * 1024, problems: 0,
          brains: [{ id: PERSONAL, name: "Moss notes", kind: "personal", bytes: 31 }, { id: "team-garden", name: "Team", kind: "organization", bytes: null }],
          sites: [{ slug: "recipes", url: "https://recipes.finite.site", owned: true }, { slug: "team-site", url: "https://team-site.finite.site", owned: false }],
        } } : {}) },
      export: exported && { state: exported.state, startedAt: started, finishedAt: exported.state === "ready" ? "2026-10-29T18:04:00Z" : null,
        ...(exported.state === "ready" ? { expiresAt: "2026-11-05T18:04:00Z", bytes: archive.length, fileName: FILE_NAME, problems: 1 } : {}) },
    });
    const advance = (job: Job | null, after: number) => { if (job && job.state === "running" && ++job.reads >= after) job.state = "ready"; };
    if (request.method === "GET" && route === "/status") {
      advance(survey, 1); advance(exported, 2);
      json(200, view()); return;
    }
    if (request.method === "POST" && route === "/survey") { survey = { state: "running", reads: 0 }; json(202, { version: 1 }); return; }
    if (request.method === "POST" && route === "/export") {
      const chunks: Buffer[] = [];
      for await (const chunk of request) chunks.push(chunk as Buffer);
      bodies.push(JSON.parse(Buffer.concat(chunks).toString()));
      exported = { state: "running", reads: 0 }; json(202, { version: 1 }); return;
    }
    if (request.method === "DELETE" && route === "/export") { exported = null; json(200, { version: 1 }); return; }
    if (request.method === "GET" && route === "/download") {
      downloads.push({ authorization: request.headers.authorization, cookie: request.headers.cookie, origin: request.headers.origin, range: request.headers.range });
      if (exported?.state !== "ready") { json(404, { error: "notReady" }); return; }
      const headers = { "content-type": "application/zip", "content-disposition": `attachment; filename="${FILE_NAME}"`, "accept-ranges": "bytes" };
      const range = /^bytes=(\d+)-(\d*)$/.exec(request.headers.range ?? "");
      if (range) {
        const start = Number(range[1]);
        const end = range[2] ? Number(range[2]) : archive.length - 1;
        response.writeHead(206, { ...headers, "content-range": `bytes ${start}-${end}/${archive.length}`, "content-length": end - start + 1 });
        response.end(archive.subarray(start, end + 1)); return;
      }
      response.writeHead(200, { ...headers, "content-length": archive.length }).end(archive); return;
    }
    json(404, { detail: "Not Found" });
  });
  agent.listen(0, "127.0.0.1");
  await once(agent, "listening");
  const agentAddress = agent.address();
  assert(agentAddress && typeof agentAddress !== "string");

  const reservation = http.createServer().listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  base = `http://127.0.0.1:${address.port}`;
  await new Promise<void>(resolve => reservation.close(() => resolve()));
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: {
      ...process.env, FC_WEB_DESIGN_PORT: String(address.port), FC_WEB_DESIGN_PREVIEWS: "0",
      FC_WEB_DESIGN_HOSTED_HERMES_URL: `https://127.0.0.1:${agentAddress.port}/`, NODE_EXTRA_CA_CERTS: certificate,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", chunk => { output += chunk; });
  server.stderr.on("data", chunk => { output += chunk; });
  let browser: Browser | undefined;
  try {
    await waitFor(async () => {
      assert.equal(server.exitCode, null, output);
      return fetch(`${base}/healthz`).then(response => response.ok).catch(() => false);
    });
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce", ignoreHTTPSErrors: true, acceptDownloads: true });
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    const artifacts = process.env.FC_BROWSER_ARTIFACT_DIR;
    if (artifacts) await mkdir(artifacts, { recursive: true });

    await page.goto(`${base}/dashboard/machines/${RUNTIME}`);
    await page.getByRole("navigation", { name: "Agent navigation", exact: true }).getByRole("link", { name: "Export", exact: true }).click();
    await page.waitForURL(`${base}/dashboard/machines/${RUNTIME}/export`);
    await page.getByRole("heading", { name: "Export", exact: true }).waitFor();
    await page.getByRole("button", { name: "Check what to export", exact: true }).click();
    const personal = page.getByRole("checkbox", { name: /^Moss notes/ });
    await personal.waitFor();
    assert.equal(await personal.isChecked(), true);
    assert.equal(await page.getByRole("checkbox", { name: /^Team/ }).isChecked(), false);
    assert.equal(await page.getByRole("checkbox", { name: /^recipes\.finite\.site/ }).isChecked(), true);
    const teamSite = page.getByRole("checkbox", { name: /^team-site\.finite\.site/ });
    assert.equal(await teamSite.isChecked(), false);
    await page.getByText("214", { exact: true }).waitFor();
    await teamSite.check();
    if (artifacts) await page.screenshot({ path: path.join(artifacts, "export-choices-desktop.png"), fullPage: true });
    await page.getByRole("button", { name: "Create export", exact: true }).click();
    await page.getByRole("heading", { name: "Making your export…", exact: true }).waitFor();
    await page.getByRole("heading", { name: "Your export is ready", exact: true }).waitFor({ timeout: 30_000 });
    assert.deepEqual(bodies, [{ brains: [PERSONAL], sites: ["recipes", "team-site"] }]);
    await page.getByText("1 item couldn’t be included.", { exact: false }).waitFor();
    if (artifacts) await page.screenshot({ path: path.join(artifacts, "export-ready-desktop.png"), fullPage: true });
    await page.setViewportSize({ width: 390, height: 844 });
    await waitFor(async () => page.evaluate(() => {
      const sidebar = document.querySelector(".finite-agent-shell__sidebar");
      return (!sidebar || sidebar.getBoundingClientRect().right <= 0) && document.documentElement.scrollWidth <= innerWidth;
    }));
    if (artifacts) await page.screenshot({ path: path.join(artifacts, "export-ready-mobile.png"), fullPage: true });
    await page.setViewportSize({ width: 1440, height: 1000 });

    // The download goes through the dashboard's own route, which fetches it
    // from the agent with Core's grant; the browser's cookie never leaves.
    const [download] = await Promise.all([
      page.waitForEvent("download"),
      page.getByRole("link", { name: "Download", exact: true }).click(),
    ]);
    assert.equal(download.suggestedFilename(), FILE_NAME);
    assert.deepEqual(await readFile(await download.path()), archive);
    assert.deepEqual(downloads.at(-1), { authorization: "Bearer synthetic-test", cookie: undefined, origin: undefined, range: undefined });
    const downloadUrl = `${base}/api/agents/${RUNTIME}/export/download`;
    const resumed = await page.request.get(downloadUrl, { headers: { range: "bytes=1000-1999" } });
    assert.equal(resumed.status(), 206);
    assert.deepEqual(await resumed.body(), archive.subarray(1000, 2000));
    assert.equal((await page.request.get(downloadUrl, { headers: { "sec-fetch-site": "cross-site" } })).status(), 403);

    await page.getByRole("button", { name: "Delete export", exact: true }).click();
    await page.getByRole("heading", { name: "What to include", exact: true }).waitFor();
    assert.equal(await page.getByRole("heading", { name: "Your export is ready", exact: true }).count(), 0);
    assert.equal((await page.request.get(downloadUrl)).status(), 404);

    const sent: string[] = [];
    page.on("request", request => { if (request.method() === "POST" && request.url().includes("/hosted-device/")) sent.push(request.postData() ?? ""); });
    await page.goto(`${base}/dashboard/machines/${RUNTIME}/chat`);
    await page.locator("textarea").fill("/export");
    await page.getByRole("button", { name: "Send message", exact: true }).click();
    await page.waitForURL(`${base}/dashboard/machines/${RUNTIME}/export`);
    assert(sent.every(body => !body.includes("/export")), "the command never reaches the agent");
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    server.kill("SIGTERM");
    if (server.exitCode === null) await once(server, "exit");
    agent.close();
    await rm(scratch, { recursive: true, force: true });
  }
});

async function waitFor(check: () => Promise<boolean>) {
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error("Timed out waiting for the export flow");
}
