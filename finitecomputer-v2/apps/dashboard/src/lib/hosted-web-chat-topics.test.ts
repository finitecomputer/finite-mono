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
  assert.equal(
    canonicalNewChatTopic([topic("canonical-room", "brain-stuff")]),
    null,
    "a missing Home topic must not silently redirect New chat into an arbitrary topic"
  );
  assert.equal(canonicalNewChatTopic([]), null);
});

test("sidebar placement keeps canonical identity and handles Home, empty, unavailable, and legacy destinations", async () => {
  const { sidebarTopics, sidebarChatKey } = await import("@/lib/hosted-web-chat-topics");
  const chat = (id: string, destination?: string, position = "AV") => ({
    chat_id: id, title: id, archived: false, active: false, unread_count: 2,
    message_count: 3, started_seq: 1, updated_seq: 4, last_message_preview: "history",
    ...(destination ? { placement: { topic_id: destination, position } } : {}),
  });
  const home = { ...topic("room", "home"), chats: [chat("same-id", "home", "BV")] };
  const source = { ...topic("room", "source"), chats: [chat("same-id", "home"), chat("fallback", "deleted", "CV")] };
  const empty = topic("room", "empty");
  const original = structuredClone([home, source, empty]);
  const result = sidebarTopics([home, source, empty]);
  assert.deepEqual([home, source, empty], original, "derived sidebar must not rewrite canonical data");
  assert.deepEqual(result[0].chats.map((item) => item.source_topic_id), ["source", "home"]);
  assert.equal(new Set(result[0].chats.map(sidebarChatKey)).size, 2);
  assert.equal(result[0].unread_count, 4);
  assert.equal(result[1].chats[0].chat_id, "fallback");
  assert.deepEqual(result[2].chats, []);
  source.chats[0].updated_seq = 999;
  assert.deepEqual(sidebarTopics([home, source, empty])[0].chats.map(sidebarChatKey), result[0].chats.map(sidebarChatKey));
  const legacy = { ...topic("room", "legacy"), chats: [chat("z"), chat("a")] };
  assert.deepEqual(sidebarTopics([legacy])[0].chats.map((item) => item.chat_id), ["z", "a"]);
});
