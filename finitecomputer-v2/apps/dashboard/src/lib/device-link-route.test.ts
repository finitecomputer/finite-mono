import assert from "node:assert/strict";
import test from "node:test";

import { POST as approve } from "@/app/api/device-links/approve/route";
import { POST as status } from "@/app/api/device-links/status/route";

test("device-link APIs reject form-style and malformed requests before account work", async () => {
  for (const handler of [approve, status]) {
    const form = await handler(
      new Request("https://finite.test/api/device-links/test", {
        method: "POST",
        headers: { "content-type": "text/plain" },
        body: "link_session_id=link-a&target_device_id=electron-a",
      })
    );
    assert.equal(form.status, 415);
    assert.equal(form.headers.get("cache-control"), "private, no-store");

    const malformed = await handler(
      new Request("https://finite.test/api/device-links/test", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: "{}",
      })
    );
    assert.equal(malformed.status, 400);

    const missingLength = await handler(
      new Request("https://finite.test/api/device-links/test", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          link_session_id: "x".repeat(5_000),
          target_device_id: "electron-a",
        }),
      })
    );
    assert.equal(missingLength.status, 413);

    const lyingLength = await handler(
      new Request("https://finite.test/api/device-links/test", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "content-length": "1",
        },
        body: JSON.stringify({
          link_session_id: "x".repeat(5_000),
          target_device_id: "electron-a",
        }),
      })
    );
    assert.equal(lyingLength.status, 413);
  }
});
