import assert from "node:assert/strict";
import test from "node:test";

import { canonicalNewChatTopic } from "@/lib/hosted-web-chat-topics";
import type { HostedChatTopic } from "@/lib/hosted-web-device";

function topic(roomId: string, topicId: string): HostedChatTopic {
  return {
    room_id: roomId,
    topic_id: topicId,
    title: topicId,
    last_message_preview: "",
    unread_count: 0,
    message_count: 0,
    created_seq: 0,
    updated_seq: 0,
    archived: false,
    chats: [],
  };
}

test("the global New chat target comes only from canonical topics", () => {
  const legacySelection = topic("legacy-room", "legacy-topic");
  const canonicalTopics = [
    topic("canonical-room", "brain-stuff"),
    topic("canonical-room", "home"),
  ];

  assert.equal(legacySelection.room_id, "legacy-room");
  assert.deepEqual(canonicalNewChatTopic(canonicalTopics), canonicalTopics[1]);
  assert.equal(canonicalNewChatTopic([]), null);
});
