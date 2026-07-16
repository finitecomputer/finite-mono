import assert from "node:assert/strict";
import test from "node:test";

import {
  HostedWebChatError,
  hostedWebChatErrorMessage,
  isAgentBindingAuthorizationRequired,
  isCanonicalNewChatTarget,
  parseHostedChatAction,
} from "@/lib/hosted-web-chat";
import { CHAT_UNAVAILABLE_MESSAGE } from "@/lib/chat-product-copy";
import {
  HostedDeviceRequestError,
  type HostedChatState,
} from "@/lib/hosted-web-device";

function targetState(): HostedChatState {
  return {
    rev: 1,
    identity: { account_id: "human-1", device_id: "hosted-web" },
    rooms: [],
    topics: [{
      room_id: "canonical-room",
      topic_id: "home",
      title: "Home",
      last_message_preview: "",
      unread_count: 0,
      message_count: 0,
      created_seq: 0,
      updated_seq: 0,
      archived: false,
      chats: [],
    }],
    status: "ready",
    messages: [],
    profiles: [],
    devices: [],
    typing_members: [],
    hosted_agent_binding: {
      version: 1,
      project_id: "project-1",
      human_account_id: "human-1",
      agent_account_id: "agent-1",
      agent_npub: "npub1agent",
      canonical_room_id: "canonical-room",
      associated_room_ids: ["legacy-room"],
    },
    flow: {
      notice_busy: false,
      scan_in_flight: false,
      scan_result: "",
    },
  };
}

test("new chat validation rejects legacy and cross-room topic targets", () => {
  const state = targetState();
  assert.equal(isCanonicalNewChatTarget(state, {
    room_id: "canonical-room",
    topic_id: "home",
    intent_key: "intent-1",
  }), true);
  assert.equal(isCanonicalNewChatTarget(state, {
    room_id: "legacy-room",
    topic_id: "home",
    intent_key: "intent-2",
  }), false);
  assert.equal(isCanonicalNewChatTarget(state, {
    room_id: "canonical-room",
    topic_id: "legacy-topic",
    intent_key: "intent-3",
  }), false);
});

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

test("hosted-device authorization recovery recognizes its service-unavailable contract", () => {
  assert.equal(
    isAgentBindingAuthorizationRequired(
      new HostedDeviceRequestError(
        "canonical Agent conversation requires recovery: first-time binding bootstrap was not authorized by Project creation",
        503
      )
    ),
    true
  );
  assert.equal(
    isAgentBindingAuthorizationRequired(
      new HostedDeviceRequestError(
        "the observed Agent Principal changed",
        503
      )
    ),
    false
  );
  assert.equal(
    isAgentBindingAuthorizationRequired(
      new HostedDeviceRequestError(
        "first-time binding bootstrap was not authorized by Project creation",
        409
      )
    ),
    true
  );
  assert.equal(
    isAgentBindingAuthorizationRequired(
      new HostedDeviceRequestError(
        "first-time binding bootstrap was not authorized by Project creation",
        503
      )
    ),
    false
  );
});

test("parseHostedChatAction accepts the bounded message operations used by web chat", () => {
  assert.deepEqual(
    parseHostedChatAction({
      StartTopicChatIntent: {
        room_id: "room-1",
        topic_id: "topic-1",
        reason: null,
        intent_key: "intent-1",
      },
    }),
    {
      StartTopicChatIntent: {
        room_id: "room-1",
        topic_id: "topic-1",
        reason: null,
        intent_key: "intent-1",
      },
    }
  );

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
      SetTyping: { room_id: "room-1", is_typing: true },
    }),
    { SetTyping: { room_id: "room-1", is_typing: true } }
  );

  assert.deepEqual(parseHostedChatAction({ RefreshDevices: null }), {
    RefreshDevices: null,
  });
  assert.deepEqual(
    parseHostedChatAction({
      RevokeDevice: { account_id: "account-1", device_id: "electron-alpha" },
    }),
    { RevokeDevice: { account_id: "account-1", device_id: "electron-alpha" } }
  );
});

test("parseHostedChatAction keeps pairing and unsupported operations off the browser action surface", () => {
  assert.throws(
    () =>
      parseHostedChatAction({
        StartTopicChat: { room_id: "legacy-room", topic_id: "home", reason: null },
      }),
    /Unsupported chat action/
  );
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
    () =>
      parseHostedChatAction({
        StartGroupChat: { profiles: [], display_name: "Injected recovery room" },
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
