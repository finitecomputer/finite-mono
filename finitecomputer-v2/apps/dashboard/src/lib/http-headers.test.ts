import test from "node:test";
import assert from "node:assert/strict";

import {
  inlineContentDisposition,
  requestHasExactOrigin,
  requestOriginMatchesHost,
} from "./http-headers";

test("inlineContentDisposition keeps headers ByteString-safe for screenshot filenames", () => {
  const header = inlineContentDisposition("Screenshot 2026-04-28 at 10.14.49\u202fPM.png");

  assert.doesNotThrow(() => {
    new Headers().set("content-disposition", header);
  });
  assert.match(header, /filename="Screenshot 2026-04-28 at 10\.14\.49 PM\.png"/u);
  assert.match(header, /filename\*=UTF-8''Screenshot%202026-04-28%20at%2010\.14\.49%E2%80%AFPM\.png/u);
});

test("requestOriginMatchesHost supports a public host alias without trusting siblings", () => {
  assert.equal(
    requestOriginMatchesHost(
      new Request("http://127.0.0.1:3000/dashboard/remove", {
        method: "POST",
        headers: {
          host: "localhost:3000",
          origin: "http://localhost:3000",
        },
      })
    ),
    true
  );
  assert.equal(
    requestOriginMatchesHost(
      new Request("http://127.0.0.1:3000/dashboard/remove", {
        method: "POST",
        headers: {
          host: "finite.computer",
          origin: "https://agent.finite.computer",
          "x-forwarded-proto": "https",
        },
      })
    ),
    false
  );
});

test("requestHasExactOrigin rejects missing and sibling-site origins", () => {
  assert.equal(
    requestHasExactOrigin(
      new Request("https://finite.computer/dashboard/remove", {
        method: "POST",
        headers: { origin: "https://finite.computer" },
      })
    ),
    true
  );
  assert.equal(
    requestHasExactOrigin(
      new Request("https://finite.computer/dashboard/remove", { method: "POST" })
    ),
    false
  );
  assert.equal(
    requestHasExactOrigin(
      new Request("https://finite.computer/dashboard/remove", {
        method: "POST",
        headers: { origin: "https://agent.finite.computer" },
      })
    ),
    false
  );
});
