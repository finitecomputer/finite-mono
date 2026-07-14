import type {
  AppChatSummary,
  AppRoomDetailsState,
  AppRoomSummary,
  AppState,
  AppTopicSummary,
  AppTypingMember,
  ChatMessage,
  Identity,
} from "./model";

export const HOME_TOPIC_ID = "home";

export type ChatSelection = {
  room: AppRoomSummary | null;
  topic: AppTopicSummary | null;
  chat: AppChatSummary | null;
};

export type PendingChatTurn = {
  room_id: string;
  topic_id: string | null;
  chat_id: string | null;
  after_seq: number;
  started_at_ms: number;
};

export type TranscriptItem =
  | { type: "message"; message: ChatMessage }
  | { type: "tools"; id: string; messages: ChatMessage[] };

const TOOL_LINE_RE = /^(?:⚙️?|🔧|🛠️?|🔍|🔎|📖|💻|🌐|⚡)\s+/u;

export const LIVE_ACTIVITY_LEASE_MS = 15_000;

export function selectedChat(state: AppState | null, preferAgentRoom = false): ChatSelection {
  if (!state) return { room: null, topic: null, chat: null };
  const room =
    state.rooms.find((candidate) => candidate.room_id === state.selected_room_id)
    ?? (preferAgentRoom ? state.rooms.find((candidate) => candidate.is_agent_chat) : null)
    ?? state.rooms[0]
    ?? null;
  const topics = topicsForRoom(state, room?.room_id);
  const topic =
    topics.find((candidate) => candidate.topic_id === state.selected_topic_id)
    ?? topics.find((candidate) => candidate.topic_id === HOME_TOPIC_ID)
    ?? topics[0]
    ?? null;
  const chat =
    topic?.chats.find((candidate) => candidate.chat_id === state.selected_chat_id)
    ?? topic?.chats.find((candidate) => candidate.active)
    ?? topic?.chats[0]
    ?? null;
  return { room, topic, chat };
}

export function topicsForRoom(state: AppState | null, roomId: string | null | undefined) {
  if (!state || !roomId) return [];
  return state.topics
    .filter((topic) => topic.room_id === roomId && !topic.archived)
    .sort((left, right) => {
      if (left.topic_id === HOME_TOPIC_ID) return -1;
      if (right.topic_id === HOME_TOPIC_ID) return 1;
      return right.updated_seq - left.updated_seq || left.title.localeCompare(right.title);
    });
}

export function messagesForChat(state: AppState | null, selection: ChatSelection) {
  if (!state) return [];
  return state.messages.filter(
    (message) =>
      (!selection.room || message.room_id === selection.room.room_id)
      && (!selection.topic || message.conversation_id === selection.topic.topic_id)
      && (!selection.chat || message.chat_id === selection.chat.chat_id)
  );
}

export function activitiesForChat(state: AppState | null, selection: ChatSelection) {
  if (!state) return [];
  return state.typing_members.filter(
    (member) =>
      member.room_id === selection.room?.room_id
      && (!member.topic_id || member.topic_id === selection.topic?.topic_id)
      && (!member.chat_id || member.chat_id === selection.chat?.chat_id)
  );
}

export function roomDetailsForSelection(
  state: AppState | null,
  selection: ChatSelection
): AppRoomDetailsState | null {
  const details = state?.room_details ?? null;
  return details?.room_id === selection.room?.room_id ? details : null;
}

/**
 * Project the durable message stream into the product transcript. Agent
 * status records are activity, not prose; adjacent tool records form one
 * inspectable work group; and edits replace the message they target.
 */
export function transcriptItems(
  messages: ChatMessage[],
  ownAccountId: string | null | undefined
): TranscriptItem[] {
  const projected = collapseMessageEdits(messages);
  const items: TranscriptItem[] = [];
  for (const message of projected) {
    const fromUserPrincipal = ownAccountId
      ? message.sender_account_id === ownAccountId
      : message.is_mine;
    if (!fromUserPrincipal && messageKind(message) === "status") continue;
    if (!fromUserPrincipal && messageKind(message) === "tool") {
      const previous = items[items.length - 1];
      if (previous?.type === "tools") {
        previous.messages.push(message);
      } else {
        items.push({ type: "tools", id: `tools-${message.message_id}`, messages: [message] });
      }
      continue;
    }
    const previous = items[items.length - 1];
    if (previous?.type === "tools") {
      previous.messages = previous.messages.map((toolMessage) => ({
        ...toolMessage,
        status: "complete",
      }));
    }
    items.push({ type: "message", message });
  }
  return items;
}

export function messageContent(message: ChatMessage) {
  return message.display_content || message.text || "";
}

export function attachmentSendError(
  state: Pick<AppState, "status" | "toast">
) {
  return state.status === "attachment unavailable"
    ? state.toast || "Attachment upload failed."
    : null;
}

function collapseMessageEdits(messages: ChatMessage[]) {
  const result: ChatMessage[] = [];
  const indexById = new Map<string, number>();
  for (const message of messages) {
    const target = message.edit_of_message_id;
    if (target && indexById.has(target)) {
      const index = indexById.get(target)!;
      const original = result[index]!;
      result[index] = {
        ...message,
        kind:
          message.kind === "message" && original.kind !== "message"
            ? original.kind
            : message.kind,
        message_id: target,
      };
      continue;
    }
    indexById.set(message.message_id, result.length);
    result.push(message);
  }
  return result;
}

function messageKind(message: ChatMessage) {
  if (message.kind) return message.kind;
  const lines = messageContent(message).trim().split(/\n+/u).filter(Boolean);
  return lines.length > 0 && lines.every((line) => TOOL_LINE_RE.test(line))
    ? "tool"
    : "message";
}

/**
 * `is_mine` is Device-local. Product-side presentation belongs to the User
 * Principal, so a message from another linked Device of the same account is
 * still rendered on the user's side.
 */
export function isUserPrincipalMessage(message: ChatMessage, identity: Identity) {
  return message.sender_account_id === identity.account_id;
}

export function hasFinalRemoteResponse(
  messages: ChatMessage[],
  afterSeq: number,
  ownAccountId?: string | null
) {
  return messages.some(
    (message) =>
      (ownAccountId ? message.sender_account_id !== ownAccountId : !message.is_mine)
      && message.seq > afterSeq
      && message.final_delivery === true
  );
}

export function beginPendingChatTurn(
  selection: ChatSelection,
  visibleMessages: ChatMessage[],
  nowMs = Date.now()
): PendingChatTurn | null {
  if (!selection.room) return null;
  return {
    room_id: selection.room.room_id,
    topic_id: selection.topic?.topic_id ?? null,
    chat_id: selection.chat?.chat_id ?? null,
    after_seq: Math.max(0, ...visibleMessages.map((message) => message.seq)),
    started_at_ms: nowMs,
  };
}

export function activityLeaseIsFresh(
  streamConnected: boolean,
  observedAtMs: number | null,
  nowMs = Date.now(),
  leaseMs = LIVE_ACTIVITY_LEASE_MS
) {
  return streamConnected
    && observedAtMs !== null
    && nowMs >= observedAtMs
    && nowMs - observedAtMs < leaseMs;
}

export function pendingTurnLeaseIsFresh(
  turn: PendingChatTurn,
  streamConnected: boolean,
  nowMs = Date.now(),
  leaseMs = LIVE_ACTIVITY_LEASE_MS
) {
  return activityLeaseIsFresh(streamConnected, turn.started_at_ms, nowMs, leaseMs);
}

export function pendingTurnMatchesSelection(
  turn: PendingChatTurn,
  selection: ChatSelection
) {
  return turn.room_id === selection.room?.room_id
    && turn.topic_id === (selection.topic?.topic_id ?? null)
    && turn.chat_id === (selection.chat?.chat_id ?? null);
}

export function pendingTurnIsComplete(
  turn: PendingChatTurn,
  messages: ChatMessage[],
  ownAccountId: string
) {
  const scoped = messages.filter(
    (message) =>
      message.room_id === turn.room_id
      && (turn.topic_id === null || message.conversation_id === turn.topic_id)
      && (turn.chat_id === null || message.chat_id === turn.chat_id)
  );
  return hasFinalRemoteResponse(scoped, turn.after_seq, ownAccountId);
}

export function liveActivityLabel(
  members: AppTypingMember[],
  fallbackName = "Someone"
) {
  const working = members.find((member) => member.activity_kind === "working");
  const thinking = members.find((member) => member.activity_kind === "thinking");
  const typing = members.find((member) => member.activity_kind === "typing");
  const member = working ?? thinking ?? typing ?? members[0];
  const name = member?.display_name || fallbackName;
  if (member?.activity_kind === "working") {
    return `${name} is working`;
  }
  if (member?.activity_kind === "thinking") {
    return `${name} is thinking`;
  }
  if (member) {
    return `${name} is typing`;
  }
  return null;
}
