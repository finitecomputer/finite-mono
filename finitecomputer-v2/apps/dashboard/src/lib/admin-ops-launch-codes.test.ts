import assert from "node:assert/strict";
import test from "node:test";

import {
  launchCodeBatchFormInput,
  launchCodeDownloadFilename,
  launchCodeDownloadText,
} from "@/lib/admin-ops";

test("Launch Code form accepts a one-code 24-hour canary and a 12-person batch", () => {
  const canary = new FormData();
  canary.set("name", "Paul canary");
  canary.set("codeCount", "1");
  canary.set("expiresInHours", "24");
  assert.deepEqual(launchCodeBatchFormInput(canary), {
    name: "Paul canary",
    codeCount: 1,
    expiresInHours: 24,
  });

  const training = new FormData();
  training.set("name", "July training");
  training.set("codeCount", "12");
  training.set("expiresInHours", "168");
  assert.deepEqual(launchCodeBatchFormInput(training), {
    name: "July training",
    codeCount: 12,
    expiresInHours: 168,
  });
});

test("Launch Code form refuses indefinite or oversized issuance", () => {
  const invalid = new FormData();
  invalid.set("name", "No expiry");
  invalid.set("codeCount", "1001");
  invalid.set("expiresInHours", "721");
  assert.throws(() => launchCodeBatchFormInput(invalid), /Code count/u);

  invalid.set("codeCount", "1");
  assert.throws(() => launchCodeBatchFormInput(invalid), /Expiry hours/u);
});

test("one-time download text includes only plaintext codes, not database ids", () => {
  assert.equal(
    launchCodeDownloadText([
      { id: "launch_code_1", code: "finite_first" },
      { id: "launch_code_2", code: "finite_second" },
    ]),
    "finite_first\nfinite_second\n"
  );
  assert.equal(launchCodeDownloadFilename("July Training / 12"), "july-training-12-codes.txt");
});
