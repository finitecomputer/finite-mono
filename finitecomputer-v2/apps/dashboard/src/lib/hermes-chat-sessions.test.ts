import test from "node:test";
import assert from "node:assert/strict";

import {
  hermesSessionReadOnly,
  hermesTopicGroups,
  hermesTranscript,
  parseHermesMessages,
  parseHermesSessionPage,
} from "./hermes-chat-sessions";
import { HOME_TOPIC_ID } from "./hosted-web-chat-topics";

const page = parseHermesSessionPage({
  total: 3,
  sessions: [
    {
      id: "native_1",
      source: "desktop",
      title: "Web chat",
      preview: "hello",
      chat_id: null,
      started_at: 100,
      last_active: 300,
      message_count: 2,
      archived: 0,
    },
    {
      id: "room_a_1",
      source: "finitechat",
      title: null,
      preview: "first line\nsecond line",
      chat_id: "room_a",
      thread_id: "segment_1",
      started_at: 50,
      last_active: 200,
      message_count: 4,
      archived: 1,
    },
    {
      id: "room_a_2",
      source: "finitechat",
      title: "Later",
      preview: "",
      chat_id: "room_a",
      started_at: 250,
      last_active: 400,
      message_count: 1,
      archived: false,
    },
    { source: "desktop" },
  ],
});

test("parses session rows and drops rows without an id", () => {
  assert.equal(page.total, 3);
  assert.deepEqual(page.sessions.map((session) => session.id), ["native_1", "room_a_1", "room_a_2"]);
  assert.equal(page.sessions[1]!.archived, true);
  assert.equal(page.sessions[1]!.threadId, "segment_1");
});

test("platform-routed sessions are read-only and native sessions are not", () => {
  assert.equal(hermesSessionReadOnly(page.sessions[0]!), false);
  assert.equal(hermesSessionReadOnly(page.sessions[1]!), true);
});

test("groups native chats under Home and each platform room under its own topic", () => {
  const groups = hermesTopicGroups(page.sessions);
  assert.equal(groups[0]!.topic.topic_id, HOME_TOPIC_ID);
  assert.equal(groups[0]!.readOnly, false);
  assert.deepEqual(groups[0]!.topic.chats.map((chat) => chat.chat_id), ["native_1"]);

  const room = groups.find((group) => group.topic.topic_id === "platform:finitechat:room_a");
  assert.ok(room);
  assert.equal(room.readOnly, true);
  assert.equal(room.topic.title, "Finite Chat");
  // Most recent first; archived history stays visible and marked.
  assert.deepEqual(room.topic.chats.map((chat) => chat.chat_id), ["room_a_2", "room_a_1"]);
  assert.equal(room.topic.chats[1]!.archived, true);
  assert.equal(room.topic.chats[1]!.title, "first line");
});

test("scheduled runs and carried-over Finite Chat history get their own read-only topics", () => {
  const rows = parseHermesSessionPage({
    sessions: [
      { id: "cron_1", source: "cron", title: "Daily digest", started_at: 1, last_active: 1 },
      { id: "finitechat-import-abc", source: "finitechat", title: "yo", chat_id: null, started_at: 2, last_active: 2, archived: 1 },
    ],
  }).sessions;
  assert.ok(rows.every((row) => hermesSessionReadOnly(row)));
  const groups = hermesTopicGroups(rows);
  assert.deepEqual(groups[0]!.topic.chats, []);
  assert.deepEqual(
    groups.slice(1).map((group) => [group.topic.title, group.readOnly, group.topic.chats.map((chat) => chat.chat_id)]),
    [["Scheduled tasks", true, ["cron_1"]], ["Finite Chat · earlier", true, ["finitechat-import-abc"]]],
  );
});

test("Home exists with no native chats so New chat has a destination", () => {
  const groups = hermesTopicGroups([]);
  assert.equal(groups.length, 1);
  assert.equal(groups[0]!.topic.topic_id, HOME_TOPIC_ID);
});

test("maps stored rows to prose and collapsible tool rows", () => {
  const rows = parseHermesMessages({
    messages: [
      { id: 1, role: "user", content: "what time is it?", timestamp: 10 },
      {
        id: 2,
        role: "assistant",
        content: "",
        reasoning: "check the clock",
        tool_calls: JSON.stringify([{ function: { name: "terminal", arguments: "{\"command\":\"date\"}" } }]),
        timestamp: 11,
      },
      { id: 3, role: "tool", tool_name: "terminal", content: "Tue", timestamp: 12 },
      { id: 4, role: "assistant", content: "It is Tuesday.", timestamp: 13 },
      { id: 5, role: "user", content: "summary", display_kind: "hidden", timestamp: 14 },
      { id: 6, role: "system", content: "prompt", timestamp: 15 },
      { id: 7, role: "user", content: "notice", display_kind: "internal_notification", timestamp: 16 },
      { id: 8, role: "assistant", content: "raw", display_content: "Earlier turns", timestamp: 17 },
    ],
  });
  const transcript = hermesTranscript(rows, "native_1", HOME_TOPIC_ID, "Agent");
  assert.deepEqual(
    transcript.map((message) => [message.kind, message.is_mine, message.text]),
    [
      ["message", true, "what time is it?"],
      ["tool", false, "check the clock"],
      ["tool", false, "terminal: {\"command\":\"date\"}"],
      ["tool", false, "terminal: Tue"],
      ["message", false, "It is Tuesday."],
      ["tool", false, "Context summary: Earlier turns"],
    ],
  );
  assert.equal(new Set(transcript.map((message) => message.message_id)).size, transcript.length);
  assert.ok(transcript.every((message) => message.conversation_id === HOME_TOPIC_ID));
});

test("rejects a payload that is not a session list or transcript", () => {
  assert.throws(() => parseHermesSessionPage({ detail: "nope" }));
  assert.throws(() => parseHermesMessages({ detail: "nope" }));
});
