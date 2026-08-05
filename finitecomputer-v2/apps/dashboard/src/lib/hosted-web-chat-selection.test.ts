import assert from "node:assert/strict";
import test from "node:test";

import {
  applyHostedChatSelectionIntent,
  HOSTED_CHAT_NAVIGATION_TIMEOUT_MS,
  hostedChatSelectionFromState,
  hostedChatSelectionIntentSatisfied,
  hostedChatSelectionIntentTarget,
  settleHostedChatSnapshotSelection,
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

test("a stale stream snapshot cannot move the selection away from a pending chat click", () => {
  const intent: HostedChatSelectionIntent = {
    token: 1,
    selected_room_id: "r1",
    selected_topic_id: "t1",
    selected_chat_id: "c2",
  };
  const stale = snapshot({ room: "r1", topic: "t1", chat: "c1" });

  const applied = applyHostedChatSelectionIntent(intent, stale);
  assert.equal(applied.confirmed, false);
  assert.equal(applied.state.selected_chat_id, "c2");
  assert.equal(applied.state.selected_topic_id, "t1");
  assert.equal(applied.state.rev, stale.rev, "content is untouched");
});

test("a snapshot carrying the clicked chat confirms and clears the pin untouched", () => {
  const intent: HostedChatSelectionIntent = {
    token: 1,
    selected_room_id: "r1",
    selected_topic_id: "t1",
    selected_chat_id: "c2",
  };
  const confirming = snapshot({ room: "r1", topic: "t1", chat: "c2" });

  const applied = applyHostedChatSelectionIntent(intent, confirming);
  assert.equal(applied.confirmed, true);
  assert.equal(applied.state, confirming);
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

test("the first snapshot adopts the daemon selection and transcript untouched", () => {
  const initial = snapshot({ room: "r1", topic: "t1", chat: "c2" });
  initial.messages = [message("c2")];

  const settled = settleHostedChatSnapshotSelection(null, initial);

  assert.equal(settled.decision, "initial");
  assert.equal(
    settled.state,
    initial,
    "foreground and windowed transcript come from the same snapshot object"
  );
  assert.equal(settled.selection.selected_chat_id, "c2");
});

test("a divergent daemon selection cannot move a valid local foreground", () => {
  const local = {
    selected_room_id: "r1",
    selected_topic_id: "t1",
    selected_chat_id: "c2",
  };
  // Another device selected c1; this browser did not. The snapshot's content
  // still merges, but the foreground stays where the local user put it.
  const next = snapshot({ room: "r1", topic: "t1", chat: "c1" });
  next.rev = 11;

  const settled = settleHostedChatSnapshotSelection(local, next);

  assert.equal(settled.decision, "preserved");
  assert.equal(settled.selection, local);
  assert.equal(settled.state.selected_chat_id, "c2");
  assert.equal(settled.state.selected_topic_id, "t1");
  assert.equal(settled.state.rev, next.rev, "content is untouched");
});

test("a vanished local chat falls back to the daemon snapshot untouched", () => {
  const next = snapshot({ room: "r1", topic: "t1", chat: "c1" });
  next.messages = [message("c1")];
  next.topics[0]!.chats = next.topics[0]!.chats.filter(
    (chat) => chat.chat_id !== "c2"
  );

  const settled = settleHostedChatSnapshotSelection(
    {
      selected_room_id: "r1",
      selected_topic_id: "t1",
      selected_chat_id: "c2",
    },
    next
  );

  assert.equal(settled.decision, "fallback");
  assert.equal(
    settled.state,
    next,
    "the fallback foreground keeps the snapshot's own selection and transcript"
  );
  assert.deepEqual(settled.selection, {
    selected_room_id: "r1",
    selected_topic_id: "t1",
    selected_chat_id: "c1",
  });
});

test("the navigation pin is bounded to the hosted-device request deadline", () => {
  assert.equal(HOSTED_CHAT_NAVIGATION_TIMEOUT_MS, 15_000);
});

test("server selection is recoverable from the snapshot for refusal fallback", () => {  assert.deepEqual(hostedChatSelectionFromState(snapshot({ room: "r1", topic: "t1", chat: "c1" })), {
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
