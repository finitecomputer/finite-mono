import assert from "node:assert/strict";
import test from "node:test";
import { createServer } from "node:http";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  createSitePreviewSession,
  localOutputsEnabled,
  parseSitePreviewTarget,
  parseViewerSessionResponse,
  readBoundedSitePreviewUrl,
  SitePreviewError,
  sitesUpstreamOrigin,
} from "@/lib/site-preview";

test("authorized previews fall back to email when their exchange is unavailable", async (t) => {
  const saved = { ...process.env };
  t.after(() => { process.env = saved; });
  const root = await mkdtemp(join(tmpdir(), "site-preview-fallback-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const exchanges: string[] = [];
  let status = 503;
  let disconnect = false;
  async function backend() {
    const server = createServer((request, response) => {
      if (request.url === "/api/core/v1/me") {
        response.writeHead(200, { "content-type": "application/json" });
        response.end(JSON.stringify({ projects: [{
          project: { id: "project-preview", display_name: "Preview" },
          runtime: { id: "runtime-preview" },
        }] }));
      } else if (request.url === "/internal/v1/viewer-sessions") {
        exchanges.push("sites");
        if (disconnect) request.socket.destroy();
        else {
          response.writeHead(status);
          response.end();
        }
      } else {
        response.writeHead(404);
        response.end();
      }
    });
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    t.after(async () => {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    });
    const address = server.address();
    assert(address && typeof address !== "string");
    return `http://127.0.0.1:${address.port}`;
  }
  const sites = await backend();
  process.env = {
    ...process.env, NODE_ENV: "development", PATH: "", FC_FINITED_BIN: "",
    FC_REPO_ROOT: root, FC_WORKSPACE_ROOT: root, FC_CONTROL_PLANE_ROOT: root,
    FC_WORKOS_AUTH_ENABLED: "0", FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
    FC_DASHBOARD_DEV_EMAIL: "friend@example.com", FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_preview",
    FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "local-preview-fixture",
    FC_CORE_BASE_URL: sites, FC_SITES_UPSTREAM_URL: sites,
    FINITE_SITES_VIEWER_SESSION_TOKEN: "ab".repeat(32),
  };
  const url = "https://hello.finite.site/docs?view=one";
  const fallback = { url: "https://hello.finite.site/_finite/sign-in", originalUrl: url };
  for (status of [503, 401, 429]) {
    const before = exchanges.length;
    assert.deepEqual(await createSitePreviewSession("runtime-preview", url), fallback);
    assert.deepEqual(exchanges.slice(before), ["sites"]);
  }
  disconnect = true;
  assert.deepEqual(await createSitePreviewSession("runtime-preview", url), fallback);
  disconnect = false;
  for (const origin of ["", "https://finite.site/internal", "file:///tmp/sites"]) {
    process.env.FC_SITES_UPSTREAM_URL = origin;
    const before = exchanges.length;
    assert.deepEqual(await createSitePreviewSession("runtime-preview", url), fallback);
    assert.equal(exchanges.length, before);
  }
  process.env.FC_SITES_UPSTREAM_URL = sites;
  const beforeDenied = exchanges.length;
  await assert.rejects(createSitePreviewSession("other-runtime", url),
    (error: unknown) => error instanceof SitePreviewError && error.status === 404);
  for (const invalid of ["http://127.0.0.1/private", "https://evil.example/", "https://hello.finite.site/_finite/auth?token=x"]) {
    await assert.rejects(createSitePreviewSession("runtime-preview", invalid),
      (error: unknown) => error instanceof SitePreviewError && error.status === 400);
  }
  assert.equal(exchanges.length, beforeDenied);
  const beforeRedirect = exchanges.length;
  const previousUrl = "https://app.finite.chat/docs?view=one";
  assert.deepEqual(await createSitePreviewSession("runtime-preview", previousUrl), {
    url: previousUrl, originalUrl: previousUrl,
  });
  assert.equal(exchanges.length, beforeRedirect);
});

test("Finite site preview targets split the canonical output origin from navigation", () => {
  assert.deepEqual(
    parseSitePreviewTarget("https://hello.finite.chat/docs/start?mode=full#intro"),
    {
      outputUrl: "https://hello.finite.chat/",
      returnTo: "/docs/start?mode=full#intro",
      originalUrl: "https://hello.finite.chat/docs/start?mode=full#intro",
    }
  );
  assert.equal(
    parseSitePreviewTarget("https://guide.docs.finite.chat/readme").outputUrl,
    "https://guide.docs.finite.chat/"
  );
  assert.equal(
    parseSitePreviewTarget("http://browser-proof.sites.localhost:18789/", {
      allowLocalOutputs: true,
    }).outputUrl,
    "http://browser-proof.sites.localhost:18789/"
  );
});

test("site preview targets reject non-output and ambiguous URLs", () => {
  for (const value of [
    "https://example.com/",
    "http://hello.finite.chat/",
    "https://api.finite.chat/",
    "https://git.finite.chat/project.git",
    "https://a.b.finite.chat/",
    "https://user:secret@hello.finite.chat/",
    "https://hello.finite.chat/\\evil",
    "http://browser-proof.sites.localhost:18789/",
    `https://hello.finite.chat/${"a".repeat(1100)}`,
  ]) {
    assert.throws(() => parseSitePreviewTarget(value), /Choose a Finite site/u, value);
  }
});

test("local output previews require an explicit non-production opt-in", () => {
  assert.equal(
    localOutputsEnabled({ NODE_ENV: "development", FC_SITES_ALLOW_LOCAL_OUTPUTS: "1" }),
    true
  );
  assert.equal(
    localOutputsEnabled({ NODE_ENV: "production", FC_SITES_ALLOW_LOCAL_OUTPUTS: "1" }),
    false
  );
  assert.equal(localOutputsEnabled({ NODE_ENV: "development" }), false);
});

test("Sites upstream is a bare server-only HTTP origin", () => {
  assert.equal(sitesUpstreamOrigin("http://127.0.0.1:8787"), "http://127.0.0.1:8787");
  assert.equal(sitesUpstreamOrigin("https://api.finite.chat/"), "https://api.finite.chat");
  assert.equal(sitesUpstreamOrigin("https://api.finite.chat/internal"), null);
  assert.equal(sitesUpstreamOrigin("file:///tmp/sites"), null);
});

test("viewer-session responses stay on the requested output and preserve return path", () => {
  const target = parseSitePreviewTarget("https://hello.finite.chat/gallery?view=one#photo");
  const token = "ab".repeat(32);
  const redeemUrl = `https://hello.finite.chat/_finite/auth?session_token=${token}&return_to=%2Fgallery%3Fview%3Done%23photo`;
  assert.equal(parseViewerSessionResponse({ redeem_url: redeemUrl }, target), redeemUrl);
  const sessionRedeemUrl = `https://hello.finite.chat/_finite/auth?session_token=${token}&return_to=%2Fgallery%3Fview%3Done%23photo`;
  assert.equal(
    parseViewerSessionResponse({ redeem_url: sessionRedeemUrl }, target),
    sessionRedeemUrl
  );

  for (const value of [
    `https://evil.example/_finite/auth?token=${token}&return_to=%2Fgallery%3Fview%3Done%23photo`,
    `https://hello.finite.chat/_finite/auth?token=${token}`,
    `https://hello.finite.chat/_finite/auth?token=${token}&return_to=%2Fother`,
    `https://hello.finite.chat/_finite/auth?token=short&return_to=%2Fgallery%3Fview%3Done%23photo`,
  ]) {
    assert.throws(
      () => parseViewerSessionResponse({ redeem_url: value }, target),
      /Site previews aren't available/u
    );
  }
});

test("site preview requests enforce the real streamed body size", async () => {
  const accepted = new Request("https://finite.computer/api/site-previews/machines/oslo/session", {
    method: "POST",
    body: JSON.stringify({ url: "https://hello.finite.chat/" }),
  });
  assert.equal(
    await readBoundedSitePreviewUrl(accepted),
    "https://hello.finite.chat/",
  );

  const chunks = [new Uint8Array(3_000), new Uint8Array(2_000)];
  const chunked = new Request("https://finite.computer/api/site-previews/machines/oslo/session", {
    method: "POST",
    body: new ReadableStream({
      pull(controller) {
        const chunk = chunks.shift();
        if (chunk) controller.enqueue(chunk);
        else controller.close();
      },
    }),
    duplex: "half",
  } as RequestInit);
  await assert.rejects(
    readBoundedSitePreviewUrl(chunked),
    (error: unknown) => error instanceof SitePreviewError && error.status === 413,
  );
});

test("site preview body reads time out when a chunked request never closes", async () => {
  const request = new Request("https://finite.computer/api/site-previews/machines/oslo/session", {
    method: "POST",
    body: new ReadableStream({
      pull() {
        return new Promise(() => undefined);
      },
    }),
    duplex: "half",
  } as RequestInit);
  await assert.rejects(
    readBoundedSitePreviewUrl(request, 4 * 1024, 5),
    (error: unknown) => error instanceof SitePreviewError && error.status === 408,
  );
});

test("account bridge sends only verified email evidence to the Sites exchange", async (t) => {
  const { createSiteAccountSession } = await import("@/lib/site-preview");
  const saved = { ...process.env };
  t.after(() => { process.env = saved; });
  process.env.FC_SITES_UPSTREAM_URL = "https://finite.site";
  process.env.FINITE_SITES_VIEWER_SESSION_TOKEN = "ab".repeat(32);
  const target = parseSitePreviewTarget("https://hello.finite.site/docs?a=1");
  const redeem = `https://hello.finite.site/_finite/auth?session_token=${"cd".repeat(32)}&return_to=%2Fdocs%3Fa%3D1`;
  const fetch = t.mock.method(globalThis, "fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
    assert.equal(input, "https://finite.site/internal/v1/viewer-sessions");
    assert.equal(init?.redirect, "error");
    assert.equal(init?.cache, "no-store");
    assert.deepEqual(JSON.parse(String(init?.body)), {
      site_url: "https://hello.finite.site/", verified_email: "friend@example.com", return_to: "/docs?a=1",
    });
    return Response.json({ redeem_url: redeem });
  });
  const result = await createSiteAccountSession(target, {
    workosUserId: "user_fixture", email: "friend@example.com", emailVerified: true, source: "workos",
  });
  assert.equal(result.url, redeem);
  assert.equal(fetch.mock.callCount(), 1);
});

test("account bridge rejects unverified, caller-header, and production dev identities before requesting access", async (t) => {
  const { createSiteAccountSession } = await import("@/lib/site-preview");
  const saved = { ...process.env };
  t.after(() => { process.env = saved; });
  process.env = { ...process.env, NODE_ENV: "production" };
  const fetch = t.mock.method(globalThis, "fetch", async () => { throw new Error("must not fetch"); });
  for (const account of [
    { source: "workos" as const, emailVerified: false },
    { source: "header" as const, emailVerified: true },
    { source: "dev" as const, emailVerified: true },
  ]) {
    await assert.rejects(createSiteAccountSession(parseSitePreviewTarget("https://hello.finite.site/"), {
      workosUserId: "user_fixture", email: "friend@example.com", ...account,
    }), /verified email/);
  }
  assert.equal(fetch.mock.callCount(), 0);
});

test("account bridge bounds upstream responses and rejects a different redemption origin", async (t) => {
  const { createSiteAccountSession } = await import("@/lib/site-preview");
  const saved = { ...process.env };
  t.after(() => { process.env = saved; });
  process.env.FC_SITES_UPSTREAM_URL = "https://finite.site";
  process.env.FINITE_SITES_VIEWER_SESSION_TOKEN = "ab".repeat(32);
  const responses = [new Response("x".repeat(8193)), Response.json({ redeem_url: `https://other.finite.site/_finite/auth?session_token=${"cd".repeat(32)}&return_to=%2F` })];
  t.mock.method(globalThis, "fetch", async () => responses.shift()!);
  for (let i = 0; i < 2; i++) {
    await assert.rejects(createSiteAccountSession(parseSitePreviewTarget("https://hello.finite.site/"), {
      workosUserId: "user_fixture", email: "friend@example.com", emailVerified: true, source: "workos",
    }), (error: unknown) => error instanceof SitePreviewError && error.status === 502);
  }
});


test("Sites exchanges reject failures and never follow credential redirects", async (t) => {
  const { createSiteAccountSession } = await import("@/lib/site-preview");
  const saved = { ...process.env };
  t.after(() => { process.env = saved; });
  const account = {
    workosUserId: "user_fixture", email: "friend@example.com", emailVerified: true, source: "workos" as const,
  };
  let calls = 0;
  let receiverCalls = 0;
  let status = 200;
  let mode = "normal";
  async function listen(server: ReturnType<typeof createServer>) {
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    t.after(async () => { server.closeAllConnections(); await new Promise<void>((resolve) => server.close(() => resolve())); });
    const address = server.address();
    assert(address && typeof address !== "string");
    return `http://127.0.0.1:${address.port}`;
  }
  const receiver = await listen(createServer((_request, response) => { receiverCalls++; response.end(); }));
  const origin = await listen(createServer(async (request, response) => {
    calls++;
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks).toString()) as Record<string, string>;
    assert.equal(request.headers.authorization, `Bearer ${"ab".repeat(32)}`);
    assert.equal(body.output_url, undefined);
    if (mode === "disconnect") { request.socket.destroy(); return; }
    if (mode === "redirect") { response.writeHead(307, { location: receiver }); response.end(); return; }
    const redeem = new URL("/_finite/auth", mode === "wrong-origin" ? "https://other.finite.site/" : body.site_url);
    redeem.searchParams.set("session_token", "cd".repeat(32));
    redeem.searchParams.set("return_to", body.return_to);
    response.writeHead(status, { "content-type": "application/json" });
    response.end(JSON.stringify({ redeem_url: redeem.toString() }));
  }));
  process.env.FC_SITES_UPSTREAM_URL = origin;
  process.env.FINITE_SITES_VIEWER_SESSION_TOKEN = "ab".repeat(32);
  const target = parseSitePreviewTarget("https://hello.finite.site/docs?a=1#intro");
  const result = await createSiteAccountSession(target, account);
  assert.equal(new URL(result.url).searchParams.get("return_to"), target.returnTo);
  for (const host of ["app.finite.chat", "guide.docs.finite.chat", "hello.v2.finite.chat"]) {
    const previous = parseSitePreviewTarget(`https://${host}/docs?a=1#intro`);
    const before = calls;
    assert.deepEqual(await createSiteAccountSession(previous, account), { url: previous.originalUrl, originalUrl: previous.originalUrl });
    assert.equal(calls, before);
  }
  for (const value of [undefined, "", "https://finite.site/internal", "file:///tmp/sites"]) {
    if (value === undefined) delete process.env.FC_SITES_UPSTREAM_URL;
    else process.env.FC_SITES_UPSTREAM_URL = value;
    const before = calls;
    await assert.rejects(createSiteAccountSession(target, account), (e: unknown) => e instanceof SitePreviewError && e.status === 503);
    assert.equal(calls, before);
  }
  process.env.FC_SITES_UPSTREAM_URL = origin;
  for (status of [401, 403, 500]) {
    const before = calls;
    if (status === 403) {
      assert.deepEqual(await createSiteAccountSession(target, account), { url: target.originalUrl, originalUrl: target.originalUrl });
    } else {
      await assert.rejects(createSiteAccountSession(target, account), (e: unknown) => e instanceof SitePreviewError && e.status === (status === 401 ? 401 : 502));
    }
    assert.equal(calls, before + 1);
  }
  status = 200;
  for (mode of ["redirect", "wrong-origin", "disconnect"]) {
    const before = calls;
    await assert.rejects(createSiteAccountSession(target, account), (e: unknown) => e instanceof SitePreviewError && e.status === 502);
    assert.equal(calls, before + 1);
  }
  assert.equal(receiverCalls, 0);
});
