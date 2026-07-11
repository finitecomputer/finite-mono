import assert from "node:assert/strict";
import test from "node:test";

import {
  brainProxyRequestHeaders,
  brainUpstreamOrigin,
  readBoundedBrainRequestBody,
  responseStatusHasNoBody,
} from "./brain-proxy";

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
  const accepted = new Request("https://finite.computer/_admin/object", {
    method: "POST",
    body: "1234",
  });
  assert.equal(
    new TextDecoder().decode(await readBoundedBrainRequestBody(accepted, 4)),
    "1234",
  );

  const chunks = [new Uint8Array([1, 2, 3]), new Uint8Array([4, 5])];
  const oversized = new Request("https://finite.computer/_admin/object", {
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

  const declaredOversized = new Request("https://finite.computer/_admin/object", {
    method: "POST",
    headers: { "content-length": "5" },
    body: "12345",
  });
  await assert.rejects(readBoundedBrainRequestBody(declaredOversized, 4));
});

test("Brain proxy body reads stop when their deadline aborts", async () => {
  const request = new Request("https://finite.computer/_admin/object", {
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
