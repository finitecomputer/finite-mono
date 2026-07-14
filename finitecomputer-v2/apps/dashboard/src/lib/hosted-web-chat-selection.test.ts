import assert from "node:assert/strict";
import test from "node:test";

import {
  applyHostedChatSelectionIntent,
  hostedChatSelectionFromState,
  hostedChatSelectionIntentSatisfied,
  hostedChatSelectionIntentTarget,
  type HostedChatSelectionIntent,
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
    rooms: [],
    selected_room_id: selection.room ?? null,
    topics: [],
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

test("no pending intent applies snapshots verbatim", () => {
  const next = snapshot({ room: "r1", topic: "t1", chat: "c1" });
  const applied = applyHostedChatSelectionIntent(null, next);
  assert.equal(applied.confirmed, false);
  assert.equal(applied.state, next);
});

test("server selection is recoverable from the snapshot for refusal fallback", () => {
  assert.deepEqual(hostedChatSelectionFromState(snapshot({ room: "r1", topic: "t1", chat: "c1" })), {
    selected_room_id: "r1",
    selected_topic_id: "t1",
    selected_chat_id: "c1",
  });
});
