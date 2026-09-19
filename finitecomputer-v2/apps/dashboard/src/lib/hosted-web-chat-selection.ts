import type { HostedChatAction, HostedChatState } from "@/lib/hosted-web-device";

export type HostedChatSelection = {
  selected_room_id: string | null;
  selected_topic_id: string | null;
  selected_chat_id: string | null;
};

/**
 * The user's most recent navigation click, pinned client-side. Selection-only
 * actions do not bump the daemon revision, so stream snapshots generated
 * before the new view is ready may still carry the previous selection.
 * Only a matching transcript can complete the navigation.
 */
export type HostedChatSelectionIntent = HostedChatSelection & { token: number };

/**
 * Bound on a pinned navigation click. Matches the hosted-device request
 * deadline (15s) in hosted-web-device.ts: the navigation request rides the
 * same endpoint, so a longer pin would outlive the request that could
 * confirm it. On expiry the provider releases the intent and restores the
 * last coherent applied snapshot.
 */
export const HOSTED_CHAT_NAVIGATION_TIMEOUT_MS = 15_000;

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

/** A snapshot confirms the requested view at the intent's granularity:
 * chat click → same chat; topic click → same topic
 * (the server may pick any chat inside it); room click → same room. */
export function hostedChatSelectionIntentSatisfied(
  intent: HostedChatSelection,
  state: HostedChatState
): boolean {
  if (intent.selected_chat_id) {
    return (state.selected_chat_id ?? null) === intent.selected_chat_id
      && (state.selected_topic_id ?? null) === intent.selected_topic_id
      && (state.selected_room_id ?? null) === intent.selected_room_id;
  }
  if (intent.selected_topic_id) {
    return (
      (state.selected_topic_id ?? null) === intent.selected_topic_id
      && (state.selected_room_id ?? null) === intent.selected_room_id
    );
  }
  return (state.selected_room_id ?? null) === intent.selected_room_id;
}

/** Reject a different selected transcript instead of relabeling its messages. */
export function hostedChatSnapshotMatchesSelection(
  selection: HostedChatSelection | null,
  next: HostedChatState
) {
  return !selection || !hostedChatSelectionExists(selection, next)
    || hostedChatSelectionIntentSatisfied(selection, next);
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
