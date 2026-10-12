import assert from "node:assert/strict";
import test from "node:test";

import { hostedChatAlert } from "@/lib/hosted-chat-alert";

const designReview = { room_id: "room_design", topic_id: "topic_design", chat_id: "chat_design" };

const none = {
  sessionError: null,
  transportError: null,
  claimError: null,
  composerError: null,
  selectedChat: designReview,
};

test("a refused or failed send outranks connection and claim errors", () => {
  const refused = { message: "This attachment was refused", sentTo: designReview };
  assert.deepEqual(
    hostedChatAlert({ ...none, transportError: "Chat could not refresh.", composerError: refused }),
    { kind: "send", message: "This attachment was refused" }
  );
  assert.deepEqual(
    hostedChatAlert({ ...none, claimError: "Claim failed", composerError: refused }),
    { kind: "send", message: "This attachment was refused" }
  );
});

test("a send result shows only in the chat it was sent to", () => {
  const refused = { message: "This attachment was refused", sentTo: designReview };
  const otherChats = [
    { ...designReview, chat_id: "chat_other" },
    { ...designReview, topic_id: "topic_other" },
    { ...designReview, room_id: "room_other" },
    null,
  ];
  for (const selectedChat of otherChats) {
    assert.deepEqual(
      hostedChatAlert({ ...none, selectedChat, transportError: "Chat could not refresh.", composerError: refused }),
      { kind: "transport", message: "Chat could not refresh." }
    );
    assert.deepEqual(
      hostedChatAlert({ ...none, selectedChat, claimError: "Claim failed", composerError: refused }),
      { kind: "claim", message: "Claim failed" }
    );
    assert.equal(hostedChatAlert({ ...none, selectedChat, composerError: refused }), null);
  }
  // Returning to the chat shows the result again until it is dismissed.
  assert.deepEqual(
    hostedChatAlert({ ...none, transportError: "Chat could not refresh.", composerError: refused }),
    { kind: "send", message: "This attachment was refused" }
  );
});

test("session expiry outranks the send result", () => {
  assert.deepEqual(
    hostedChatAlert({
      ...none,
      sessionError: "Your session expired",
      transportError: "Chat could not refresh.",
      composerError: { message: "This attachment was refused", sentTo: designReview },
    }),
    { kind: "session", message: "Your session expired" }
  );
});

test("other composer errors stay below connection and claim errors in every chat", () => {
  const fileLimit = { message: "You can attach up to 10 files at a time.", sentTo: null };
  for (const selectedChat of [designReview, { ...designReview, chat_id: "chat_other" }, null]) {
    assert.deepEqual(
      hostedChatAlert({ ...none, selectedChat, transportError: "Chat could not refresh.", composerError: fileLimit }),
      { kind: "transport", message: "Chat could not refresh." }
    );
    assert.deepEqual(
      hostedChatAlert({ ...none, selectedChat, claimError: "Claim failed", composerError: fileLimit }),
      { kind: "claim", message: "Claim failed" }
    );
    assert.deepEqual(
      hostedChatAlert({ ...none, selectedChat, composerError: fileLimit }),
      { kind: "composer", message: "You can attach up to 10 files at a time." }
    );
  }
});

test("no recorded error shows no alert", () => {
  assert.equal(hostedChatAlert(none), null);
});
