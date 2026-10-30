import assert from "node:assert/strict";
import { test } from "node:test";
import { proxyAgentExportDownload } from "./agent-export-download";
import { CoreFetchError } from "./core-client";

const runtime = "runtime_moss";
const fileName = "finite-export-moss-20261029-1804.zip";
const grant = { baseUrl: "https://agents-lat3.finite.computer/runtimes/runtime_moss/", accessToken: "synthetic-token", expiresAt: 1 };

type Call = { url: string; headers: Record<string, string> };
function fixture(answers: Array<Response | Error>, options: { access?: string | null; grants?: Array<typeof grant | Error> } = {}) {
  const calls: Call[] = [];
  let grants = 0;
  return {
    calls,
    grants: () => grants,
    dependencies: {
      access: async (id: string) => options.access === null ? null : { machineId: options.access ?? id },
      grant: async () => {
        const next = options.grants?.[grants] ?? grant;
        grants++;
        if (next instanceof Error) throw next;
        return next;
      },
      fetch: async (url: URL, init: RequestInit) => {
        calls.push({ url: url.href, headers: init.headers as Record<string, string> });
        const next = answers.shift();
        if (!next) throw new Error("unexpected fetch");
        if (next instanceof Error) throw next;
        return next;
      },
    },
  };
}
function zip(status: number, headers: Record<string, string> = {}) {
  return new Response("PK zip bytes", {
    status,
    headers: {
      "content-type": "application/zip",
      "content-disposition": `attachment; filename="${fileName}"`,
      "content-length": "12",
      "set-cookie": "never=forwarded",
      ...headers,
    },
  });
}
const request = (headers: Record<string, string> = {}) =>
  new Request(`https://finite.computer/api/agents/${runtime}/export/download`, { headers });

test("the owner's download streams from the agent with a fresh grant and a single forwarded range", async () => {
  const agent = fixture([zip(206, { "content-range": "bytes 100-111/5000", etag: '"abc"' })]);
  const response = await proxyAgentExportDownload(
    request({ "sec-fetch-site": "same-origin", range: "bytes=100-", "if-range": '"abc"', cookie: "session=private" }),
    runtime, agent.dependencies,
  );
  assert.equal(response.status, 206);
  assert.equal(await response.text(), "PK zip bytes");
  assert.equal(response.headers.get("content-type"), "application/zip");
  assert.equal(response.headers.get("content-disposition"), `attachment; filename="${fileName}"`);
  assert.equal(response.headers.get("content-range"), "bytes 100-111/5000");
  assert.equal(response.headers.get("etag"), '"abc"');
  assert.equal(response.headers.get("cache-control"), "private, no-store");
  assert.equal(response.headers.get("x-content-type-options"), "nosniff");
  assert.equal(response.headers.get("set-cookie"), null);
  assert.deepEqual(agent.calls, [{
    url: `${grant.baseUrl}api/plugins/finite-export/download`,
    headers: { range: "bytes=100-", "if-range": '"abc"', authorization: "Bearer synthetic-token" },
  }]);
});

test("other sites, strangers and unknown agents never reach the agent", async () => {
  for (const site of ["cross-site", "same-site"]) {
    const agent = fixture([]);
    const response = await proxyAgentExportDownload(request({ "sec-fetch-site": site }), runtime, agent.dependencies);
    assert.equal(response.status, 403);
    assert.equal(agent.grants(), 0);
  }
  for (const access of [null, "runtime_other"]) {
    const agent = fixture([], { access });
    assert.equal((await proxyAgentExportDownload(request(), runtime, agent.dependencies)).status, 404);
    assert.equal(agent.grants(), 0);
  }
  const forbidden = fixture([], { grants: [new CoreFetchError("not owner", 403)] });
  assert.equal((await proxyAgentExportDownload(request(), runtime, forbidden.dependencies)).status, 403);
  const notReady = fixture([], { grants: [new CoreFetchError("pending", 409)] });
  assert.equal((await proxyAgentExportDownload(request(), runtime, notReady.dependencies)).status, 409);
  const down = fixture([], { grants: [new Error("Core unavailable")] });
  assert.equal((await proxyAgentExportDownload(request(), runtime, down.dependencies)).status, 503);
});

test("an expired grant is renewed once; the agent's refusals map to plain errors", async () => {
  const renewed = fixture([new Response(null, { status: 401 }), zip(200)]);
  assert.equal((await proxyAgentExportDownload(request(), runtime, renewed.dependencies)).status, 200);
  assert.equal(renewed.grants(), 2);
  const refused = fixture([new Response(null, { status: 401 }), new Response(null, { status: 401 })]);
  assert.equal((await proxyAgentExportDownload(request(), runtime, refused.dependencies)).status, 403);
  assert.equal(refused.grants(), 2);
  const cases: Array<[Response | Error, number]> = [
    [new Response(JSON.stringify({ error: "notReady" }), { status: 404 }), 404],
    [new Response(null, { status: 403 }), 403],
    [new Response("private", { status: 500 }), 502],
    [zip(200, { "content-type": "text/html" }), 502],
    [new Error("connect ECONNREFUSED"), 502],
  ];
  for (const [answer, status] of cases) {
    const response = await proxyAgentExportDownload(request(), runtime, fixture([answer]).dependencies);
    assert.equal(response.status, status);
    assert.doesNotMatch(await response.text(), /private|ECONNREFUSED/u);
  }
  const unsatisfiable = await proxyAgentExportDownload(request({ range: "bytes=9000-" }), runtime,
    fixture([new Response(null, { status: 416, headers: { "content-range": "bytes */5000" } })]).dependencies);
  assert.equal(unsatisfiable.status, 416);
  assert.equal(unsatisfiable.headers.get("content-range"), "bytes */5000");
});

test("only a well-formed range and the exporter's own file name pass through", async () => {
  const agent = fixture([zip(200, { "content-disposition": 'attachment; filename="../../evil.html"' })]);
  const response = await proxyAgentExportDownload(request({ range: "bytes=0-1, 5-9", "if-range": "x" }), runtime, agent.dependencies);
  assert.equal(response.headers.get("content-disposition"), 'attachment; filename="finite-export.zip"');
  assert.deepEqual(agent.calls[0].headers, { authorization: "Bearer synthetic-token" });
});
