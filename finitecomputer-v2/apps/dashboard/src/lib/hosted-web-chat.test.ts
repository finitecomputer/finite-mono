import assert from "node:assert/strict";
import test from "node:test";

import {
  HostedWebChatError,
  hostedWebChatErrorMessage,
  parseHostedChatAction,
} from "@/lib/hosted-web-chat";
import { CHAT_UNAVAILABLE_MESSAGE } from "@/lib/chat-product-copy";

test("unexpected chat infrastructure errors are replaced with plain product copy", () => {
  assert.equal(
    hostedWebChatErrorMessage(new Error("Hosted Web Device update stream failed")),
    CHAT_UNAVAILABLE_MESSAGE
  );
  assert.equal(
    hostedWebChatErrorMessage(new HostedWebChatError("Sign in again to use chat.", 401)),
    "Sign in again to use chat."
  );
});

test("parseHostedChatAction accepts the bounded message operations used by web chat", () => {
  assert.deepEqual(
    parseHostedChatAction({
      SendChatMessage: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        text: "hello",
      },
    }),
    {
      SendChatMessage: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        text: "hello",
      },
    }
  );

  assert.deepEqual(
    parseHostedChatAction({
      RenameChat: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        title: "Launch checklist",
      },
    }),
    {
      RenameChat: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        title: "Launch checklist",
      },
    }
  );

  assert.deepEqual(
    parseHostedChatAction({
      StartTopicChat: {
        room_id: "room-1",
        topic_id: "topic-1",
        reason: null,
      },
    }),
    {
      StartTopicChat: {
        room_id: "room-1",
        topic_id: "topic-1",
        reason: null,
      },
    }
  );

  assert.deepEqual(
    parseHostedChatAction({
      SetTyping: { room_id: "room-1", is_typing: true },
    }),
    { SetTyping: { room_id: "room-1", is_typing: true } }
  );
});

test("parseHostedChatAction keeps pairing and product management off the browser action surface", () => {
  assert.throws(
    () => parseHostedChatAction({ ScanTarget: { value: "finite://join?secret" } }),
    (error: unknown) =>
      error instanceof HostedWebChatError
      && error.status === 400
      && /Unsupported chat action/.test(error.message)
  );
  assert.throws(
    () =>
      parseHostedChatAction({
        StartProfileChat: { profile: {}, display_name: "Injected room" },
      }),
    /Unsupported chat action/
  );
  assert.throws(
    () => parseHostedChatAction({ DeleteEverything: null }),
    /Unsupported chat action/
  );
});

test("parseHostedChatAction rejects ambiguous and oversized input", () => {
  assert.throws(
    () => parseHostedChatAction({ StartRuntime: null, OpenRoom: { room_id: "room-1" } }),
    /exactly one operation/
  );
  assert.throws(
    () =>
      parseHostedChatAction({
        SendMessage: { room_id: "room-1", text: "x".repeat(70 * 1024) },
      }),
    /Invalid text/
  );
  assert.throws(
    () =>
      parseHostedChatAction({
        LoadOlderMessages: {
          room_id: "room-1",
          before_message_id: "message-1",
          limit: 101,
        },
      }),
    /Invalid limit/
  );
  assert.throws(
    () => parseHostedChatAction({ SetTyping: { room_id: "room-1", is_typing: "yes" } }),
    /Invalid is_typing/
  );
});
