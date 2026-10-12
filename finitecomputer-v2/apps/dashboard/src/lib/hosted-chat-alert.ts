/** The room, topic and chat a send went to. */
export type HostedChatSendTarget = { room_id: string; topic_id: string; chat_id: string };

/**
 * A composer error. When it is the result of the person's own send, it
 * carries the chat the send went to, so it only shows in that chat.
 */
export type HostedChatComposerError = { message: string; sentTo: HostedChatSendTarget | null };

export type HostedChatAlert = {
  kind: "session" | "send" | "transport" | "claim" | "composer";
  message: string;
};

/**
 * Picks the one alert shown above the composer. Session expiry comes first.
 * The result of the person's own send (a refused or failed send) comes next,
 * so a connection or claim error cannot hide it. A send result from another
 * chat is not shown, so it cannot hide this chat's connection or setup
 * action. Other composer errors, such as file limits, audio or rename
 * failures, stay below connection and claim.
 */
export function hostedChatAlert(errors: {
  sessionError: string | null;
  transportError: string | null;
  claimError: string | null;
  composerError: HostedChatComposerError | null;
  selectedChat: HostedChatSendTarget | null;
}): HostedChatAlert | null {
  const { sessionError, transportError, claimError, composerError, selectedChat } = errors;
  const sentTo = composerError?.sentTo;
  const sendResult = sentTo
    && selectedChat
    && sentTo.room_id === selectedChat.room_id
    && sentTo.topic_id === selectedChat.topic_id
    && sentTo.chat_id === selectedChat.chat_id
    ? composerError
    : null;
  if (sessionError) return { kind: "session", message: sessionError };
  if (sendResult) return { kind: "send", message: sendResult.message };
  if (transportError) return { kind: "transport", message: transportError };
  if (claimError) return { kind: "claim", message: claimError };
  if (composerError && !sentTo) return { kind: "composer", message: composerError.message };
  return null;
}
