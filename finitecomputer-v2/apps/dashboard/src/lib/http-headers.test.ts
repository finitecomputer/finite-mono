import test from "node:test";
import assert from "node:assert/strict";

import { inlineContentDisposition } from "./http-headers";

test("inlineContentDisposition keeps headers ByteString-safe for screenshot filenames", () => {
  const header = inlineContentDisposition("Screenshot 2026-04-28 at 10.14.49\u202fPM.png");

  assert.doesNotThrow(() => {
    new Headers().set("content-disposition", header);
  });
  assert.match(header, /filename="Screenshot 2026-04-28 at 10\.14\.49 PM\.png"/u);
  assert.match(header, /filename\*=UTF-8''Screenshot%202026-04-28%20at%2010\.14\.49%E2%80%AFPM\.png/u);
});
