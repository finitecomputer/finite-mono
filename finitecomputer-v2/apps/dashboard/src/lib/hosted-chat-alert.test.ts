import assert from "node:assert/strict";
import test from "node:test";

import { hostedChatAlert } from "@/lib/hosted-chat-alert";

const none = {
  sessionError: null,
  transportError: null,
  claimError: null,
  composerError: null,
};

test("a refused or failed send outranks connection and claim errors", () => {
  const refused = { message: "This attachment was refused", fromSend: true };
  assert.deepEqual(
    hostedChatAlert({ ...none, transportError: "Chat could not refresh.", composerError: refused }),
    { kind: "send", message: "This attachment was refused" }
  );
  assert.deepEqual(
    hostedChatAlert({ ...none, claimError: "Claim failed", composerError: refused }),
    { kind: "send", message: "This attachment was refused" }
  );
});

test("session expiry outranks the send result", () => {
  assert.deepEqual(
    hostedChatAlert({
      ...none,
      sessionError: "Your session expired",
      transportError: "Chat could not refresh.",
      composerError: { message: "This attachment was refused", fromSend: true },
    }),
    { kind: "session", message: "Your session expired" }
  );
});

test("other composer errors stay below connection and claim errors", () => {
  const fileLimit = { message: "You can attach up to 10 files at a time.", fromSend: false };
  assert.deepEqual(
    hostedChatAlert({ ...none, transportError: "Chat could not refresh.", composerError: fileLimit }),
    { kind: "transport", message: "Chat could not refresh." }
  );
  assert.deepEqual(
    hostedChatAlert({ ...none, claimError: "Claim failed", composerError: fileLimit }),
    { kind: "claim", message: "Claim failed" }
  );
  assert.deepEqual(
    hostedChatAlert({ ...none, composerError: fileLimit }),
    { kind: "composer", message: "You can attach up to 10 files at a time." }
  );
});

test("no recorded error shows no alert", () => {
  assert.equal(hostedChatAlert(none), null);
});
