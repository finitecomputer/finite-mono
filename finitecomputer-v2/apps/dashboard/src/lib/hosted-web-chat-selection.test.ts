import assert from "node:assert/strict";
import test from "node:test";

import {
  hostedChatSelectionFromState,
  hostedChatSelectionExists,
  hostedChatSelectionIntentTarget,
  projectHostedChatVisibleSelection,
} from "@/lib/hosted-web-chat-selection";
import type { HostedChatState } from "@/lib/hosted-web-device";

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

test("a later stream snapshot cannot move a settled browser selection", () => {
  const visible = {
    selected_room_id: "r1",
    selected_topic_id: "t1",
    selected_chat_id: "c2",
  };
  const streaming = snapshot({ room: "r1", topic: "t1", chat: "c1" });

  const projected = projectHostedChatVisibleSelection(visible, streaming);
  assert.equal(projected.decision, "preserved");
  assert.equal(projected.state.selected_chat_id, "c2");
  assert.equal(projected.state.selected_topic_id, "t1");
  assert.equal(projected.state.rev, streaming.rev, "content is untouched");
});

test("the first snapshot initializes visible navigation from daemon persistence", () => {
  const initial = snapshot({ room: "r1", topic: "t1", chat: "c2" });

  const projected = projectHostedChatVisibleSelection(null, initial);
  assert.equal(projected.decision, "initial");
  assert.equal(projected.selection.selected_chat_id, "c2");
});

test("a removed visible chat causes one deterministic server-selection fallback", () => {
  const next = snapshot({ room: "r1", topic: "t1", chat: "c1" });
  next.topics[0]!.chats = next.topics[0]!.chats.filter(
    (chat) => chat.chat_id !== "c2"
  );

  const projected = projectHostedChatVisibleSelection(
    {
      selected_room_id: "r1",
      selected_topic_id: "t1",
      selected_chat_id: "c2",
    },
    next
  );
  assert.equal(projected.decision, "fallback");
  assert.deepEqual(projected.selection, {
    selected_room_id: "r1",
    selected_topic_id: "t1",
    selected_chat_id: "c1",
  });
  assert.equal(
    projectHostedChatVisibleSelection(projected.selection, next).decision,
    "preserved"
  );
});

test("server selection is recoverable from the snapshot for refusal fallback", () => {
  assert.deepEqual(hostedChatSelectionFromState(snapshot({ room: "r1", topic: "t1", chat: "c1" })), {
    selected_room_id: "r1",
    selected_topic_id: "t1",
    selected_chat_id: "c1",
  });
});

test("partial room and topic intents remain valid while their response is pending", () => {
  assert.equal(
    hostedChatSelectionExists(
      {
        selected_room_id: "r1",
        selected_topic_id: null,
        selected_chat_id: null,
      },
      snapshot({ room: "r1", topic: "t1", chat: "c1" })
    ),
    true
  );
  assert.equal(
    hostedChatSelectionExists(
      {
        selected_room_id: "r1",
        selected_topic_id: "t2",
        selected_chat_id: null,
      },
      snapshot({ room: "r1", topic: "t1", chat: "c1" })
    ),
    true
  );
});
