import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";

import { brainServerOrigin, hostedSignedBrainRequest } from "./brain-hosted-client";

test("brainServerOrigin accepts only origin-shaped HTTP(S) URLs", () => {
  assert.equal(brainServerOrigin("http://127.0.0.1:3015"), "http://127.0.0.1:3015");
  assert.equal(brainServerOrigin("https://brain.example/"), "https://brain.example");
  assert.equal(brainServerOrigin("https://brain.example/extra"), null);
  assert.equal(brainServerOrigin("ftp://brain.example"), null);
  assert.equal(brainServerOrigin("not a url"), null);
  assert.equal(brainServerOrigin(undefined), null);
  assert.equal(brainServerOrigin("  "), null);
});

function listen(handler: http.RequestListener) {
  return new Promise<{ url: string; close: () => Promise<void> }>((resolve) => {
    const server = http.createServer(handler);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      assert(address && typeof address === "object");
      resolve({
        url: `http://127.0.0.1:${address.port}`,
        close: () => new Promise((done) => server.close(() => done())),
      });
    });
  });
}

function readBody(request: http.IncomingMessage) {
  return new Promise<string>((resolve) => {
    let body = "";
    request.on("data", (chunk) => (body += chunk));
    request.on("end", () => resolve(body));
  });
}

test("hostedSignedBrainRequest signs through the device and relays the Nostr header", async () => {
  const captured: Array<{ method: string; path: string; authorization?: string; body: string }> = [];
  const brain = await listen(async (request, response) => {
    captured.push({
      method: request.method ?? "",
      path: request.url ?? "",
      authorization: request.headers.authorization,
      body: await readBody(request),
    });
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ status: "ok", brains: [] }));
  });
  const deviceInputs: Array<Record<string, unknown>> = [];
  const device = await listen(async (request, response) => {
    const body = await readBody(request);
    const parsed = JSON.parse(body) as Record<string, unknown>;
    deviceInputs.push(parsed);
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ kind: 27235, pubkey: "ab".repeat(32), tags: [], content: "" }));
  });

  try {
    const result = (await hostedSignedBrainRequest(
      { baseUrl: device.url, apiToken: "device-token" } as never,
      { workosUserId: "user_1", emailVerified: true } as never,
      brain.url,
      "POST",
      "/v1/brains/acme/approvals",
      JSON.stringify({ hello: "world" })
    )) as { status?: string };
    assert.equal(result.status, "ok");
    assert.equal(captured.length, 1);
    assert.equal(captured[0].method, "POST");
    assert.equal(captured[0].path, "/v1/brains/acme/approvals");
    assert.match(captured[0].authorization ?? "", /^Nostr /);
    const decoded = JSON.parse(
      Buffer.from((captured[0].authorization ?? "").slice(6), "base64").toString("utf8")
    );
    assert.equal(decoded.kind, 27235);
    assert.equal(deviceInputs.length, 1);
    assert.equal(deviceInputs[0].operation, "authorizeHttpRequest");
    const input = deviceInputs[0].input as { url: string; method: string; bodyText: string };
    assert.equal(input.url, `${brain.url}/v1/brains/acme/approvals`);
    assert.equal(input.method, "POST");
    assert.equal(input.bodyText, JSON.stringify({ hello: "world" }));
  } finally {
    await brain.close();
    await device.close();
  }
});
