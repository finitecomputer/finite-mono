import assert from "node:assert/strict";
import test from "node:test";

import { qrSvgModel } from "./qr-svg";

const INVITE_URL = "finite://join?invite=abc123&relay=wss%3A%2F%2Frelay.example";

test("qrSvgModel produces a deterministic module grid for an invite URL", () => {
  const model = qrSvgModel(INVITE_URL);

  // Smallest QR symbol is 21x21 modules; auto-sizing only grows from there.
  assert.ok(model.moduleCount >= 21, `moduleCount ${model.moduleCount}`);
  assert.ok(model.path.length > 0);
  // Path is exclusively 1x1 module squares.
  assert.match(model.path, /^(M\d+ \d+h1v1h-1z)+$/);
  // Deterministic: the same input always renders the same code.
  assert.deepEqual(qrSvgModel(INVITE_URL), model);
});

test("qrSvgModel includes the finder pattern corner", () => {
  const model = qrSvgModel(INVITE_URL);
  // The top-left finder pattern always starts with a dark module at 0,0.
  assert.ok(model.path.startsWith("M0 0"));
});

test("qrSvgModel rejects empty input", () => {
  assert.throws(() => qrSvgModel(""), /QR code text is required/);
  assert.throws(() => qrSvgModel("   "), /QR code text is required/);
});
