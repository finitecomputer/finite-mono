import assert from "node:assert/strict";
import { test } from "node:test";
import { boundNextVersionCheck } from "../browser/next-dev-network.mjs";

test("an unavailable optional Next version lookup has a bounded deadline", async () => {
  const keepAlive = setInterval(() => {}, 100);
  try {
    const fetch = boundNextVersionCheck((_input, { signal }) => new Promise((_resolve, reject) => {
      signal.addEventListener("abort", () => reject(signal.reason), { once: true });
    }));
    await assert.rejects(fetch("https://registry.npmjs.org/-/package/next/dist-tags"), { name: "TimeoutError" });
  } finally {
    clearInterval(keepAlive);
  }
});

test("service requests and other registry requests preserve their exact arguments", async () => {
  for (const input of ["http://localhost:3000/api/core/v1/me",
    new URL("https://registry.npmjs.org/another-package"),
    new Request("https://chat.example.test/sync")]) {
    const init = { headers: { authorization: "synthetic-test-only" } };
    const response = new Response("unchanged");
    const fetch = boundNextVersionCheck((actualInput, actualInit) => {
      assert.equal(actualInput, input);
      assert.equal(actualInit, init);
      return Promise.resolve(response);
    });
    assert.equal(await fetch(input, init), response);
  }
});

test("the optional lookup preserves an existing abort signal", async () => {
  const controller = new AbortController();
  controller.abort(new Error("caller cancelled"));
  const input = new Request("https://registry.npmjs.org/-/package/next/dist-tags", { signal: controller.signal });
  const fetch = boundNextVersionCheck((actualInput, { signal }) => {
    assert.equal(actualInput, input);
    assert.equal(signal.aborted, true);
    assert.equal(signal.reason, controller.signal.reason);
    return Promise.reject(signal.reason);
  });
  await assert.rejects(fetch(input), { message: "caller cancelled" });
});
