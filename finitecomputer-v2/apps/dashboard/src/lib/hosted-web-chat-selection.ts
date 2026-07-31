import type { HostedChatAction, HostedChatState } from "@/lib/hosted-web-device";

export type HostedChatSelection = {
  selected_room_id: string | null;
  selected_topic_id: string | null;
  selected_chat_id: string | null;
};

export type HostedChatSelectionProjection = {
  state: HostedChatState;
  selection: HostedChatSelection;
  decision: "initial" | "preserved" | "fallback";
};

export function hostedChatSelectionFromState(state: HostedChatState): HostedChatSelection {
  return {
    selected_room_id: state.selected_room_id ?? null,
    selected_topic_id: state.selected_topic_id ?? null,
    selected_chat_id: state.selected_chat_id ?? null,
  };
}

/** Target selection of a navigation action; null when the target is unknown
 * (StartTopicChatIntent selects a chat that does not exist yet). */
export function hostedChatSelectionIntentTarget(
  action: HostedChatAction
): HostedChatSelection | null {
  if ("OpenChat" in action) {
    return {
      selected_room_id: action.OpenChat.room_id,
      selected_topic_id: action.OpenChat.topic_id,
      selected_chat_id: action.OpenChat.chat_id,
    };
  }
  if ("OpenTopic" in action) {
    return {
      selected_room_id: action.OpenTopic.room_id,
      selected_topic_id: action.OpenTopic.topic_id,
      selected_chat_id: null,
    };
  }
  if ("OpenRoom" in action) {
    return {
      selected_room_id: action.OpenRoom.room_id,
      selected_topic_id: null,
      selected_chat_id: null,
    };
  }
  return null;
}

/**
 * Project the browser-owned visible route onto a full daemon snapshot.
 *
 * Snapshot selection is used only for the initial route and for deterministic
 * fallback after the visible route disappears. Transcript, activity, send,
 * upload, and refresh snapshots therefore cannot navigate the browser.
 */
export function projectHostedChatVisibleSelection(
  visible: HostedChatSelection | null,
  next: HostedChatState
): HostedChatSelectionProjection {
  const selection = visible && hostedChatSelectionExists(visible, next)
    ? visible
    : fallbackHostedChatSelection(next);
  const decision = visible === null
    ? "initial"
    : selection === visible
      ? "preserved"
      : "fallback";
  return {
    state: {
      ...next,
      ...selection,
    },
    selection,
    decision,
  };
}

export function hostedChatSelectionExists(
  selection: HostedChatSelection,
  state: HostedChatState
) {
  const roomId = selection.selected_room_id;
  if (!roomId) return state.rooms.length === 0;
  if (!state.rooms.some((room) => room.room_id === roomId)) return false;

  const topicId = selection.selected_topic_id;
  if (!topicId) return true;
  const topic = state.topics.find(
    (candidate) =>
      candidate.room_id === roomId
      && candidate.topic_id === topicId
  );
  if (!topic) return false;

  const chatId = selection.selected_chat_id;
  if (!chatId) return true;
  return topic.chats.some((chat) => chat.chat_id === chatId);
}

function fallbackHostedChatSelection(state: HostedChatState): HostedChatSelection {
  const server = hostedChatSelectionFromState(state);
  if (hostedChatSelectionExists(server, state)) return server;

  const canonicalRoomId = state.hosted_agent_binding?.canonical_room_id;
  const room = state.rooms.find((candidate) => candidate.room_id === canonicalRoomId)
    ?? [...state.rooms].sort((left, right) =>
      left.room_id.localeCompare(right.room_id)
    )[0];
  if (!room) {
    return {
      selected_room_id: null,
      selected_topic_id: null,
      selected_chat_id: null,
    };
  }

  const topics = state.topics
    .filter((topic) => topic.room_id === room.room_id && !topic.archived)
    .sort((left, right) => {
      if (left.topic_id === "home") return -1;
      if (right.topic_id === "home") return 1;
      return left.topic_id.localeCompare(right.topic_id);
    });
  const topic = topics[0];
  if (!topic) {
    return {
      selected_room_id: room.room_id,
      selected_topic_id: null,
      selected_chat_id: null,
    };
  }

  const chats = topic.chats.filter((chat) => !chat.archived);
  const chat = chats.find((candidate) => candidate.chat_id === topic.active_chat_id)
    ?? chats.find((candidate) => candidate.active)
    ?? [...chats].sort((left, right) => left.chat_id.localeCompare(right.chat_id))[0];
  return {
    selected_room_id: room.room_id,
    selected_topic_id: topic.topic_id,
    selected_chat_id: chat?.chat_id ?? null,
  };
}
