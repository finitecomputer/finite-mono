import test from "node:test";
import assert from "node:assert/strict";

import { parseHermesSessionPage } from "./hermes-chat-sessions";
import { hermesHistoryCoverage } from "./hermes-history-coverage";
import type { HostedChatState } from "./hosted-web-device";

const chat = (chat_id: string, title: string, message_count: number) => ({
  chat_id, title, message_count, last_message_preview: "", unread_count: 0,
  started_seq: 0, updated_seq: 0, active: false, archived: false,
});
const topic = (room_id: string, topic_id: string, title: string, chats: ReturnType<typeof chat>[]) => ({
  room_id, topic_id, title, last_message_preview: "", unread_count: 0, message_count: 0,
  created_seq: 0, updated_seq: 0, archived: false, chats,
});

const state = {
  hosted_agent_binding: { canonical_room_id: "room-a" },
  topics: [
    topic("room-a", "home", "Home", [chat("seg-1", "plan", 10), chat("seg-2", "lost", 4), chat("seg-empty", "New chat", 0)]),
    topic("room-a", "topic-fun", "Fun", [chat("seg-3", "yo", 2)]),
    topic("room-other", "home", "Home", [chat("seg-9", "other agent", 3)]),
  ],
} as unknown as HostedChatState;

test("reports chats Hermes lacks for the agent's own room only", () => {
  const sessions = parseHermesSessionPage({
    sessions: [
      { id: "h1", source: "finitechat", chat_id: "room-a", thread_id: "seg-1", message_count: 7 },
      { id: "h2", source: "finitechat", chat_id: "room-a", thread_id: "seg-3", message_count: 1 },
      { id: "h3", source: "finitechat", chat_id: "room-a", thread_id: "seg-3", message_count: 2 },
      { id: "h4", source: "finitechat", chat_id: "room-other", thread_id: "seg-2", message_count: 5 },
      { id: "h5", source: "desktop", chat_id: null, thread_id: "seg-2", message_count: 5 },
    ],
  }).sessions;
  const coverage = hermesHistoryCoverage(state, sessions);
  assert.ok(coverage);
  assert.equal(coverage.chatsWithMessages, 3);
  assert.equal(coverage.covered, 2);
  assert.deepEqual(coverage.missing.map((row) => row.chatId), ["seg-2"]);
  const fun = coverage.chats.find((row) => row.chatId === "seg-3");
  assert.deepEqual([fun?.hermesMessages, fun?.hermesSessions, fun?.topicTitle], [3, 2, "Fun"]);
});

test("returns nothing without a bound Finite Chat room", () => {
  assert.equal(hermesHistoryCoverage({ ...state, hosted_agent_binding: null }, []), null);
});
