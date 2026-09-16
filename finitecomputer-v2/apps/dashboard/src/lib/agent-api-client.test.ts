import assert from "node:assert/strict";
import { test } from "node:test";
import { parseHermesBootstrap } from "./agent-api-client";

const token = "synthetic_native_token_123456";
test("native bootstrap parses the JSON token without executing HTML", () => {
  const html = `<script>window.__HERMES_SESSION_TOKEN__=${JSON.stringify(token)};window.__HERMES_AUTH_REQUIRED__=false;</script>`;
  assert.equal(parseHermesBootstrap(html), token);
  assert.throws(() => parseHermesBootstrap(html.replace(JSON.stringify(token), "runCode()")), /bootstrap unavailable/);
});
test("gated or malformed bootstrap fails without disclosing returned token", () => {
  for (const html of [
    `window.__HERMES_SESSION_TOKEN__="${token}";window.__HERMES_AUTH_REQUIRED__=true;`,
    `window.__HERMES_SESSION_TOKEN__="invalid token";window.__HERMES_AUTH_REQUIRED__=false;`,
    "<html>Sign in</html>",
  ]) {
    assert.throws(() => parseHermesBootstrap(html), (error: unknown) => {
      assert(error instanceof Error);
      assert(!error.message.includes(token));
      return true;
    });
  }
});
