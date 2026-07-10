import assert from "node:assert/strict";
import test from "node:test";

import { fetchRuntimeAgentNpub, truncateNpub } from "./agent-contact";

const CONTACT_URL = "https://runtime.example.com/contact";

test("fetchRuntimeAgentNpub reads and validates the Agent Principal", async () => {
  const previousFetch = globalThis.fetch;
  globalThis.fetch = (async () =>
    new Response(JSON.stringify({ agent_npub: "npub1agentexamplekey" }))) as typeof fetch;

  try {
    assert.equal(await fetchRuntimeAgentNpub(CONTACT_URL), "npub1agentexamplekey");
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("fetchRuntimeAgentNpub rejects missing, malformed, and non-http contacts", async () => {
  const previousFetch = globalThis.fetch;
  const responses = [
    new Response(JSON.stringify({ agent_npub: "not-an-npub" })),
    new Response("<html>bad gateway</html>"),
    new Response("{}", { status: 503 }),
  ];
  globalThis.fetch = (async () => responses.shift() ?? new Response("{}")) as typeof fetch;

  try {
    assert.equal(await fetchRuntimeAgentNpub(CONTACT_URL), null);
    assert.equal(await fetchRuntimeAgentNpub(CONTACT_URL), null);
    assert.equal(await fetchRuntimeAgentNpub(CONTACT_URL), null);
    assert.equal(await fetchRuntimeAgentNpub("finite://join"), null);
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("truncateNpub preserves short values and middle-truncates long values", () => {
  assert.equal(truncateNpub("npub1short"), "npub1short");
  assert.equal(truncateNpub("npub1abcdefghijklmnopqrstuvwxyz", 10, 5), "npub1abcde…vwxyz");
});
