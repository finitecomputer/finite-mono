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
      (state.selected_topic_id ?? null) === intent.selected_topic_id &&
      (state.selected_room_id ?? null) === intent.selected_room_id
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
