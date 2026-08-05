import type { HostedChatAction, HostedChatState } from "@/lib/hosted-web-device";

export type HostedChatSelection = {
  selected_room_id: string | null;
  selected_topic_id: string | null;
  selected_chat_id: string | null;
};

/**
 * The user's most recent navigation click, pinned client-side. Selection-only
 * actions do not bump the daemon revision, so stream snapshots generated
 * before the click persists still carry the previous selection and legally
 * apply. Without the pin, every such snapshot yanks the highlight back to the
 * old chat until reconciliation flips it forward again.
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

/** A snapshot confirms the intent once the server persisted the selection at
 * the intent's granularity: chat click → same chat; topic click → same topic
 * (the server may pick any chat inside it); room click → same room. */
export function hostedChatSelectionIntentSatisfied(
  intent: HostedChatSelectionIntent,
  state: HostedChatState
): boolean {
  if (intent.selected_chat_id) {
    return (state.selected_chat_id ?? null) === intent.selected_chat_id;
  }
  if (intent.selected_topic_id) {
    return (
      (state.selected_topic_id ?? null) === intent.selected_topic_id
      && (state.selected_room_id ?? null) === intent.selected_room_id
    );
  }
  return (state.selected_room_id ?? null) === intent.selected_room_id;
}

/**
 * Apply a pending intent to an incoming snapshot. A satisfied snapshot is
 * returned untouched and reports confirmed so the caller drops the pin; an
 * unsatisfied one keeps its content but presents the intent's selection so
 * stale stream snapshots cannot fight the user's click.
 */
export function applyHostedChatSelectionIntent(
  intent: HostedChatSelectionIntent | null,
  next: HostedChatState
): { state: HostedChatState; confirmed: boolean } {
  // A full snapshot's messages are windowed to the daemon selection. Once a
  // click settles, keep those fields together instead of projecting a second
  // long-lived browser selection over a different Chat's transcript.
  if (!intent) return { state: next, confirmed: false };
  if (hostedChatSelectionIntentSatisfied(intent, next)) {
    return { state: next, confirmed: true };
  }
  return {
    state: {
      ...next,
      selected_room_id: intent.selected_room_id,
      selected_topic_id: intent.selected_topic_id,
      selected_chat_id: intent.selected_chat_id,
    },
    confirmed: false,
  };
}

export type HostedChatSettledSelection = {
  state: HostedChatState;
  selection: HostedChatSelection;
  decision: "initial" | "preserved" | "fallback";
};

/**
 * Settle an applied snapshot against the browser's local foreground
 * selection. Selection is device-scoped: the daemon-selected foreground is
 * adopted only when the browser has no local selection (first load) or the
 * local selection genuinely vanished from the snapshot. A daemon selection
 * that merely differs — another device's click, a scoped send elsewhere —
 * updates transcript, unread, and background state freely but must not move
 * this browser's foreground.
 *
 * Both adopt paths return the snapshot object untouched so the foreground
 * and its windowed transcript always come from the same snapshot.
 */
export function settleHostedChatSnapshotSelection(
  local: HostedChatSelection | null,
  next: HostedChatState
): HostedChatSettledSelection {
  if (local && hostedChatSelectionExists(local, next)) {
    return {
      state: { ...next, ...local },
      selection: local,
      decision: "preserved",
    };
  }
  return {
    state: next,
    selection: hostedChatSelectionFromState(next),
    decision: local === null ? "initial" : "fallback",
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
