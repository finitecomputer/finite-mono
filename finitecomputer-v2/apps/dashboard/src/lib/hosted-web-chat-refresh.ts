import type { HostedChatState } from "@/lib/hosted-web-device";

export const PENDING_CHAT_REFRESH_DELAY_MS = 12_000;

export type PendingChatRefreshTarget = {
  room_id: string;
  topic_id: string;
  chat_id: string;
  after_seq: number;
};

export function latestPendingChatMessageSeq(
  state: HostedChatState,
  target: PendingChatRefreshTarget
) {
  return Math.max(
    0,
    ...state.messages
      .filter(
        (message) =>
          message.room_id === target.room_id
          && message.conversation_id === target.topic_id
          && message.chat_id === target.chat_id
      )
      .map((message) => message.seq)
  );
}

export function pendingChatRefreshAdvancesTranscript(
  current: HostedChatState,
  next: HostedChatState,
  target: PendingChatRefreshTarget
) {
  if (
    next.rev < current.rev
    || current.selected_room_id !== target.room_id
    || current.selected_topic_id !== target.topic_id
    || current.selected_chat_id !== target.chat_id
  ) {
    return false;
  }
  const currentSeq = latestPendingChatMessageSeq(current, target);
  const nextSeq = latestPendingChatMessageSeq(next, target);
  return nextSeq > Math.max(currentSeq, target.after_seq);
}

export function pendingChatRefreshIsDue(
  nowMs: number,
  observedAtMs: number,
  refreshedAtMs: number | null,
  delayMs = PENDING_CHAT_REFRESH_DELAY_MS
) {
  const lastProgressAtMs = Math.max(observedAtMs, refreshedAtMs ?? 0);
  return nowMs >= lastProgressAtMs
    && nowMs - lastProgressAtMs >= delayMs;
}

export function preservePendingChatRefreshSelection(
  next: HostedChatState,
  target: PendingChatRefreshTarget
): HostedChatState {
  return {
    ...next,
    selected_room_id: target.room_id,
    selected_topic_id: target.topic_id,
    selected_chat_id: target.chat_id,
  };
}
