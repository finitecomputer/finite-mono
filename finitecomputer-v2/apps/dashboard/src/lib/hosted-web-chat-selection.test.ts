import assert from "node:assert/strict";
import test from "node:test";

import {
  HOSTED_CHAT_NAVIGATION_TIMEOUT_MS,
  hostedChatSelectionFromState,
  hostedChatSelectionIntentSatisfied,
  hostedChatSelectionIntentTarget,
  hostedChatSnapshotMatchesSelection,
  type HostedChatSelectionIntent,
} from "@/lib/hosted-web-chat-selection";
import type { HostedChatMessage, HostedChatState } from "@/lib/hosted-web-device";

function snapshot(selection: {
  room?: string | null;
  topic?: string | null;
  chat?: string | null;
}): HostedChatState {
  return {
    rev: 10,
    identity: { account_id: "acct", device_id: "hosted-web" },
    rooms: [
      {
        room_id: "r1",
        display_name: "Room",
        state: "Connected",
        status: "",
        user_status_text: "",
        last_message_preview: "",
        unread_count: 0,
        can_load_older: false,
        is_agent_chat: true,
      },
    ],
    selected_room_id: selection.room ?? null,
    topics: [
      {
        room_id: "r1",
        topic_id: "t1",
        title: "One",
        last_message_preview: "",
        unread_count: 0,
        message_count: 0,
        created_seq: 1,
        updated_seq: 1,
        archived: false,
        active_chat_id: "c1",
        chats: [
          {
            chat_id: "c1",
            title: "One",
            last_message_preview: "",
            unread_count: 0,
            message_count: 0,
            started_seq: 1,
            updated_seq: 1,
            active: true,
            archived: false,
          },
          {
            chat_id: "c2",
            title: "Two",
            last_message_preview: "",
            unread_count: 0,
            message_count: 0,
            started_seq: 2,
            updated_seq: 2,
            active: false,
            archived: false,
          },
        ],
      },
      {
        room_id: "r1",
        topic_id: "t2",
        title: "Two",
        last_message_preview: "",
        unread_count: 0,
        message_count: 0,
        created_seq: 2,
        updated_seq: 2,
        archived: false,
        active_chat_id: "c7",
        chats: [
          {
            chat_id: "c7",
            title: "Seven",
            last_message_preview: "",
            unread_count: 0,
            message_count: 0,
            started_seq: 3,
            updated_seq: 3,
            active: true,
            archived: false,
          },
        ],
      },
    ],
    selected_topic_id: selection.topic ?? null,
    selected_chat_id: selection.chat ?? null,
    status: "ready",
    messages: [],
    profiles: [],
    devices: [],
    typing_members: [],
    flow: {
      notice_busy: false,
      scan_in_flight: false,
      scan_result: "",
    },
  };
}

test("navigation actions map to their target selection", () => {
  assert.deepEqual(
    hostedChatSelectionIntentTarget({
      OpenChat: { room_id: "r1", topic_id: "t1", chat_id: "c2" },
    }),
    { selected_room_id: "r1", selected_topic_id: "t1", selected_chat_id: "c2" }
  );
  assert.deepEqual(
    hostedChatSelectionIntentTarget({ OpenTopic: { room_id: "r1", topic_id: "t2" } }),
    { selected_room_id: "r1", selected_topic_id: "t2", selected_chat_id: null }
  );
  assert.deepEqual(
    hostedChatSelectionIntentTarget({ OpenRoom: { room_id: "r9" } }),
    { selected_room_id: "r9", selected_topic_id: null, selected_chat_id: null }
  );
  assert.equal(
    hostedChatSelectionIntentTarget({
      StartTopicChatIntent: {
        room_id: "r1",
        topic_id: "t1",
        reason: null,
        intent_key: "k",
      },
    }),
    null
  );
});

test("a topic click is confirmed by topic match even when the server picks a chat", () => {
  const intent: HostedChatSelectionIntent = {
    token: 2,
    selected_room_id: "r1",
    selected_topic_id: "t2",
    selected_chat_id: null,
  };
  assert.equal(
    hostedChatSelectionIntentSatisfied(intent, snapshot({ room: "r1", topic: "t2", chat: "c7" })),
    true
  );
  assert.equal(
    hostedChatSelectionIntentSatisfied(intent, snapshot({ room: "r1", topic: "t1", chat: "c7" })),
    false
  );
});

test("only a matching transcript may replace an existing local view", () => {
  const current = snapshot({ room: "r1", topic: "t1", chat: "c2" });
  current.messages = [message("c2")];
  const selection = hostedChatSelectionFromState(current);
  const other = snapshot({ room: "r1", topic: "t1", chat: "c1" });
  other.messages = [message("c1")];
  assert.equal(hostedChatSnapshotMatchesSelection(selection, other), false);
  assert.equal(hostedChatSnapshotMatchesSelection(selection, current), true);
  assert.equal(hostedChatSnapshotMatchesSelection(null, other), true);
  const anotherRoom = { ...current, selected_room_id: "r2" };
  assert.equal(hostedChatSnapshotMatchesSelection(selection, anotherRoom), false);
});

test("a vanished chat permits the server's complete fallback snapshot", () => {
  const next = snapshot({ room: "r1", topic: "t1", chat: "c1" });
  next.topics[0]!.chats = next.topics[0]!.chats.filter(chat => chat.chat_id !== "c2");
  assert.equal(hostedChatSnapshotMatchesSelection({
    selected_room_id: "r1", selected_topic_id: "t1", selected_chat_id: "c2",
  }, next), true);
});

test("the navigation pin is bounded to the hosted-device request deadline", () => {
  assert.equal(HOSTED_CHAT_NAVIGATION_TIMEOUT_MS, 15_000);
});

test("selection is recovered from a coherent snapshot for refusal fallback", () => {
  assert.deepEqual(hostedChatSelectionFromState(snapshot({ room: "r1", topic: "t1", chat: "c1" })), {
    selected_room_id: "r1",
    selected_topic_id: "t1",
    selected_chat_id: "c1",
  });
});

function message(chatId: string): HostedChatMessage {
  return {
    room_id: "r1",
    conversation_id: "t1",
    chat_id: chatId,
    seq: 1,
    message_id: `message-${chatId}`,
    sender_account_id: "acct",
    sender_device_id: "hosted-web",
    sender_display_name: "User",
    text: "message",
    display_content: "message",
    kind: "message",
    status: "complete",
    final_delivery: false,
    is_mine: true,
    media: [],
    timestamp_unix_seconds: 1,
    display_timestamp: "now",
  };
}
