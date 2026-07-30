import assert from "node:assert/strict";
import test from "node:test";

import {
  BRAIN_CLIENT_CONTENT_SECURITY_POLICY,
  brainProxyRequestHeaders,
  brainUpstreamOrigin,
  proxyBrainRequest,
  readBoundedBrainRequestBody,
  responseStatusHasNoBody,
} from "./brain-proxy";
import { NextRequest } from "next/server";

test("Brain client CSP permits its bounded confirmation dialogs", () => {
  assert.match(BRAIN_CLIENT_CONTENT_SECURITY_POLICY, /sandbox[^;]*\ballow-modals\b/u);
});

test("Brain upstream accepts only a bare HTTP origin", () => {
  assert.equal(brainUpstreamOrigin("http://127.0.0.1:3015"), "http://127.0.0.1:3015");
  assert.equal(brainUpstreamOrigin("https://brain.example/"), "https://brain.example");
  assert.equal(brainUpstreamOrigin("https://brain.example/client"), null);
  assert.equal(brainUpstreamOrigin("file:///tmp/brain"), null);
  assert.equal(brainUpstreamOrigin("not a URL"), null);
  assert.equal(brainUpstreamOrigin(""), null);
});

test("Brain proxy preserves signed identity headers but not browser credentials", () => {
  const headers = brainProxyRequestHeaders(
    new Headers({
      authorization: "Nostr signed-event",
      cookie: "wos-session=secret",
      "x-finitebrain-nostr": "legacy-signed-event",
      "x-nostr-authorization": "signed-event",
      "x-workos-session": "secret",
    }),
  );

  assert.equal(headers.get("authorization"), "Nostr signed-event");
  assert.equal(headers.get("x-finitebrain-nostr"), "legacy-signed-event");
  assert.equal(headers.get("x-nostr-authorization"), "signed-event");
  assert.equal(headers.get("cookie"), null);
  assert.equal(headers.get("x-workos-session"), null);
});

test("Brain proxy omits bodies for HTTP statuses that forbid them", () => {
  for (const status of [101, 204, 205, 304]) {
    assert.equal(responseStatusHasNoBody(status), true);
  }
  for (const status of [200, 201, 400, 500]) {
    assert.equal(responseStatusHasNoBody(status), false);
  }
});

test("Brain proxy bounds actual streamed request bytes", async () => {
  const accepted = new Request("https://finite.computer/v1/object", {
    method: "POST",
    body: "1234",
  });
  assert.equal(
    new TextDecoder().decode(await readBoundedBrainRequestBody(accepted, 4)),
    "1234",
  );

  const chunks = [new Uint8Array([1, 2, 3]), new Uint8Array([4, 5])];
  const oversized = new Request("https://finite.computer/v1/object", {
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
  await assert.rejects(readBoundedBrainRequestBody(oversized, 4));

  const declaredOversized = new Request("https://finite.computer/v1/object", {
    method: "POST",
    headers: { "content-length": "5" },
    body: "12345",
  });
  await assert.rejects(readBoundedBrainRequestBody(declaredOversized, 4));
});

test("Brain proxy body reads stop when their deadline aborts", async () => {
  const request = new Request("https://finite.computer/v1/object", {
    method: "POST",
    body: new ReadableStream({
      pull() {
        return new Promise(() => undefined);
      },
    }),
    duplex: "half",
  } as RequestInit);
  const controller = new AbortController();
  const reading = readBoundedBrainRequestBody(request, 4, controller.signal);
  controller.abort();
  await assert.rejects(
    reading,
    (error: unknown) => error instanceof DOMException && error.name === "AbortError",
  );
});

test("Brain update notification streams remain open past the ordinary request deadline", async (t) => {
  const originalFetch = globalThis.fetch;
  const originalUpstream = process.env.FC_BRAIN_UPSTREAM_URL;
  process.env.FC_BRAIN_UPSTREAM_URL = "http://brain.internal";
  t.mock.timers.enable({ apis: ["setTimeout"] });
  globalThis.fetch = async (_input, init) => {
    const signal = init?.signal;
    return new Response(
      new ReadableStream<Uint8Array>({
        start(controller) {
          const timer = setTimeout(() => {
            controller.enqueue(new TextEncoder().encode("event: brain_update\ndata: {}\n\n"));
            controller.close();
          }, 60_001);
          signal?.addEventListener(
            "abort",
            () => {
              clearTimeout(timer);
              controller.error(new DOMException("upstream aborted", "AbortError"));
            },
            { once: true },
          );
        },
      }),
      { headers: { "content-type": "text/event-stream" } },
    );
  };

  try {
    const response = await proxyBrainRequest(
      new NextRequest("https://finite.computer/v1/brain-updates"),
      "/v1",
      ["brain-updates"],
    );
    const body = response.text();
    t.mock.timers.tick(60_001);
    assert.match(await body, /event: brain_update/u);
  } finally {
    globalThis.fetch = originalFetch;
    if (originalUpstream === undefined) delete process.env.FC_BRAIN_UPSTREAM_URL;
    else process.env.FC_BRAIN_UPSTREAM_URL = originalUpstream;
  }
});
