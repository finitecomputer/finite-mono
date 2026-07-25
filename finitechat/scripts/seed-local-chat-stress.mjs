#!/usr/bin/env node

const hostedUrl = required("FINITECHAT_STRESS_HOSTED_URL").replace(/\/+$/, "");
const apiToken = required("FINITECHAT_STRESS_HOSTED_API_TOKEN");
const workosUserId = required("FINITECHAT_STRESS_WORKOS_USER_ID");
const roomId = required("FINITECHAT_STRESS_ROOM_ID");
const messageTarget = positiveInteger(
  process.env.FINITECHAT_STRESS_MESSAGE_COUNT || "5200",
  "FINITECHAT_STRESS_MESSAGE_COUNT",
);
const chatTarget = positiveInteger(
  process.env.FINITECHAT_STRESS_CHAT_COUNT || "81",
  "FINITECHAT_STRESS_CHAT_COUNT",
);
const completionTitle = `Stress chat ${String(chatTarget).padStart(3, "0")} · fixture complete ${messageTarget}`;
const expectedArchiveCount = Math.floor((chatTarget - 1) / 10);

if (messageTarget < 5_001) {
  throw new Error("FINITECHAT_STRESS_MESSAGE_COUNT must be at least 5001");
}
if (chatTarget < 65) {
  throw new Error("FINITECHAT_STRESS_CHAT_COUNT must be at least 65");
}

const headers = {
  authorization: `Bearer ${apiToken}`,
  "content-type": "application/json",
  "x-finite-workos-user-id": workosUserId,
};

let initial = await request("/v1/app/state");
if (!initial.rooms.some((room) => room.room_id === roomId && room.is_agent_chat)) {
  throw new Error("the local source does not expose the canonical paired-agent Room");
}
if (initial.paired_agent?.canonical_room_id !== roomId) {
  initial = await action({ PairAgent: { room_id: roomId } });
}
if (initial.paired_agent?.canonical_room_id !== roomId) {
  throw new Error("the local source did not retain its canonical paired agent");
}

const existingStressChats = initial.topics.flatMap((topic) =>
  topic.chats.filter(
    (chat) =>
      chat.title === "Long history — 1,200 messages" ||
      chat.title.startsWith("Stress chat "),
  ),
);
const existingStressMessages = existingStressChats.reduce(
  (total, chat) => total + chat.message_count,
  0,
);
if (existingStressMessages > 0) {
  const archivedStressChats = existingStressChats.filter((chat) => chat.archived).length;
  if (
    existingStressMessages < messageTarget ||
    existingStressChats.length !== chatTarget ||
    archivedStressChats !== expectedArchiveCount
  ) {
    throw new Error(
      `found an incomplete stress fixture (${existingStressMessages} messages across ${existingStressChats.length} chats, ${archivedStressChats} archived); use a fresh state root`,
    );
  }
  if (!existingStressChats.some((chat) => chat.title === completionTitle)) {
    const finalChat = existingStressChats.find(
      (chat) => chat.title === `Stress chat ${String(chatTarget).padStart(3, "0")}`,
    );
    const finalTopic = initial.topics.find((topic) =>
      topic.chats.some((chat) => chat.chat_id === finalChat?.chat_id),
    );
    if (!finalChat || !finalTopic) {
      throw new Error("the complete stress fixture is missing its final chat");
    }
    await action({
      RenameChat: {
        room_id: roomId,
        topic_id: finalTopic.topic_id,
        chat_id: finalChat.chat_id,
        title: completionTitle,
      },
    });
  }
  process.stdout.write(
    `Stress source already ready: ${existingStressMessages} messages across ${existingStressChats.length} chats; ${archivedStressChats} archived.\n`,
  );
  process.exit(0);
}

const baselineMessages = messageCount(initial);
const topics = [...initial.topics];
for (const title of ["Stress Alpha", "Stress Beta", "Stress Gamma"]) {
  let topic = topics.find((candidate) => candidate.title === title);
  if (!topic) {
    const next = await action({ CreateTopic: { room_id: roomId, title } });
    topic = next.topics.find((candidate) => candidate.title === title);
    if (!topic) {
      throw new Error(`could not create topic ${title}`);
    }
    topics.push(topic);
  }
}

const activeTopics = topics.filter((topic) => topic.room_id === roomId && !topic.archived);
if (activeTopics.length < 4) {
  throw new Error("stress seeding requires Home plus three synthetic topics");
}

const longChatMessages = Math.min(1_200, messageTarget - (chatTarget - 1));
const remainingMessages = messageTarget - longChatMessages;
const archiveTargets = [];
let completionTarget = null;
let sentMessages = 0;

for (let chatIndex = 0; chatIndex < chatTarget; chatIndex += 1) {
  const topic = activeTopics[chatIndex % activeTopics.length];
  const started = await action({
    StartTopicChat: {
      room_id: roomId,
      topic_id: topic.topic_id,
      reason: `Local pairing stress chat ${chatIndex + 1}`,
    },
  });
  const chatId = started.selected_chat_id;
  if (!chatId) {
    throw new Error(`chat ${chatIndex + 1} did not become selected`);
  }

  await action({
    RenameChat: {
      room_id: roomId,
      topic_id: topic.topic_id,
      chat_id: chatId,
      title:
        chatIndex === 0
          ? "Long history — 1,200 messages"
          : `Stress chat ${String(chatIndex + 1).padStart(3, "0")}`,
    },
  });

  const chatMessages =
    chatIndex === 0
      ? longChatMessages
      : Math.floor(remainingMessages / (chatTarget - 1)) +
        (chatIndex <= remainingMessages % (chatTarget - 1) ? 1 : 0);
  for (let messageIndex = 0; messageIndex < chatMessages; messageIndex += 1) {
    await action(
      {
        SendChatMessage: {
          room_id: roomId,
          topic_id: topic.topic_id,
          chat_id: chatId,
          text: `Synthetic stress message ${String(sentMessages + 1).padStart(5, "0")} · chat ${String(chatIndex + 1).padStart(3, "0")} · item ${String(messageIndex + 1).padStart(4, "0")}`,
        },
      },
      false,
    );
    sentMessages += 1;
    if (sentMessages % 250 === 0 || sentMessages === messageTarget) {
      process.stdout.write(`Seeded ${sentMessages}/${messageTarget} messages\n`);
    }
  }

  if (chatIndex > 0 && chatIndex % 10 === 0) {
    archiveTargets.push({ topicId: topic.topic_id, chatId });
  }
  if (chatIndex === chatTarget - 1) {
    completionTarget = { topicId: topic.topic_id, chatId };
  }
}

const seededState = await request("/v1/app/state");
const seededMessages = messageCount(seededState);
const addedMessages = seededMessages - baselineMessages;
const seededChats = seededState.topics.reduce((count, topic) => count + topic.chats.length, 0);
if (addedMessages !== messageTarget) {
  throw new Error(`seeded ${addedMessages} messages; expected exactly ${messageTarget}`);
}
if (seededChats < chatTarget) {
  throw new Error(`source exposes ${seededChats} chats; expected at least ${chatTarget}`);
}

for (const target of archiveTargets) {
  await action({
    SetChatArchived: {
      room_id: roomId,
      topic_id: target.topicId,
      chat_id: target.chatId,
      archived: true,
    },
  });
}
if (!completionTarget) {
  throw new Error("stress fixture did not retain its completion target");
}
await action({
  RenameChat: {
    room_id: roomId,
    topic_id: completionTarget.topicId,
    chat_id: completionTarget.chatId,
    title: completionTitle,
  },
});

process.stdout.write(
  `Stress source ready: ${addedMessages} new messages across ${seededChats} chats and ${seededState.topics.length} topics; ${archiveTargets.length} chats archived.\n`,
);

async function action(value, parseResponse = true) {
  return request("/v1/app/actions", {
    method: "POST",
    body: JSON.stringify(value),
    parseResponse,
  });
}

async function request(pathname, options = {}) {
  const response = await fetch(`${hostedUrl}${pathname}`, {
    method: options.method || "GET",
    headers,
    body: options.body,
  });
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`${pathname} failed (${response.status}): ${body.slice(0, 512)}`);
  }
  return options.parseResponse === false ? null : JSON.parse(body);
}

function messageCount(state) {
  return state.topics.reduce(
    (topicTotal, topic) =>
      topicTotal +
      topic.chats.reduce((chatTotal, chat) => chatTotal + chat.message_count, 0),
    0,
  );
}

function required(name) {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function positiveInteger(value, name) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isSafeInteger(parsed) || parsed <= 0 || String(parsed) !== value) {
    throw new Error(`${name} must be a positive integer`);
  }
  return parsed;
}
