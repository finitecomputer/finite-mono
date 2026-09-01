import assert from "node:assert/strict";
import test from "node:test";

import {
  localOutputsEnabled,
  parseSitePreviewTarget,
  readBoundedSitePreviewUrl,
  SitePreviewError,
  sitesUpstreamOrigin,
} from "@/lib/site-preview";

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

test("preview targets keep the exact original URL for the plain-output handoff", () => {
  // The dashboard no longer mints site sessions: it validates the target and
  // hands back the plain output URL. The Auth Gate redirect plus the user's
  // WorkOS/AuthKit SSO makes private-output viewing silent in the browser.
  const target = parseSitePreviewTarget("https://hello.finite.chat/gallery?view=one#photo");
  assert.equal(target.originalUrl, "https://hello.finite.chat/gallery?view=one#photo");
  assert.equal(target.outputUrl, "https://hello.finite.chat/");
  assert.equal(target.returnTo, "/gallery?view=one#photo");
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
