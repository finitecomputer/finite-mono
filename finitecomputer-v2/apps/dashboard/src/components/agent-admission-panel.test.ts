import assert from "node:assert/strict";
import { test } from "node:test";

import {
  ADMISSION_TIMEOUT_MESSAGE,
  admissionErrorMessage,
  admissionSuccessText,
  parseAdmissionAccountId,
} from "./agent-admission-panel";

const accountId = "ab".repeat(32);

test("admission account ids must be 64 hex characters", () => {
  assert.equal(parseAdmissionAccountId(accountId), accountId);
  assert.equal(parseAdmissionAccountId(`  ${accountId.toUpperCase()}  `), accountId);
  assert.equal(parseAdmissionAccountId("npub1not-hex"), null);
  assert.equal(parseAdmissionAccountId(accountId.slice(0, 63)), null);
  assert.equal(parseAdmissionAccountId(""), null);
});

test("admission failures map timeouts to a friendly message", () => {
  const timeout = new Error("timed out");
  timeout.name = "TimeoutError";
  assert.equal(admissionErrorMessage(timeout), ADMISSION_TIMEOUT_MESSAGE);
  assert.equal(
    admissionErrorMessage(new Error("sender is not on the Welcome allowlist")),
    "sender is not on the Welcome allowlist"
  );
  assert.equal(
    admissionErrorMessage("not an error"),
    "Chat admission is unavailable right now."
  );
});

test("admission success copy stays honest about the silent apply and restart", () => {
  const sent = admissionSuccessText("sent", "revoke", "Atlas");
  assert.match(sent, /Revoked chat access — command sent/u);
  assert.match(sent, /without a confirmation reply/u);
  assert.match(sent, /next gateway restart/u);

  const applied = admissionSuccessText("applied", "grant", "Atlas");
  assert.match(applied, /Granted chat access/u);
  assert.match(applied, /next gateway restart/u);
  assert.doesNotMatch(applied, /command sent/u);
});
