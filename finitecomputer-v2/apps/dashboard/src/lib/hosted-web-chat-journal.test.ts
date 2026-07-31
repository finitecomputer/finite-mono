import assert from "node:assert/strict";
import test from "node:test";

import {
  hostedChatNavigationJournalForTest,
  recordHostedChatNavigation,
} from "@/lib/hosted-web-chat-journal";

test("navigation journal is inert outside a browser", () => {
  recordHostedChatNavigation({
    source: "sse",
    snapshot_sequence: 2,
    snapshot_rev: 8,
    snapshot_selection: selection("chat-a"),
    visible_selection: selection("chat-b"),
    navigation_intent_generation: 3,
    decision: "preserved",
  });

  assert.deepEqual(hostedChatNavigationJournalForTest(), []);
});

function selection(chatId: string) {
  return {
    selected_room_id: "room",
    selected_topic_id: "topic",
    selected_chat_id: chatId,
  };
}
