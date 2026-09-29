import type { HermesSessionRow } from "@/lib/hermes-chat-sessions";
import type { HostedChatState } from "@/lib/hosted-web-device";

/**
 * Which of an agent's Finite Chat chats Hermes can already show.
 *
 * The Finite Chat bridge stores each chat as a Hermes session with the room
 * as `chat_id` and the chat (segment) as `thread_id`. Counts are compared, not
 * content: Finite Chat also counts tool progress, status notices and edits,
 * which Hermes stores differently, so a count gap is a prompt to look, not a
 * loss by itself. A chat with messages and no session is history Hermes lacks.
 */
export type HermesCoverageChat = {
  topicTitle: string;
  chatId: string;
  title: string;
  finiteChatMessages: number;
  hermesMessages: number | null;
  hermesSessions: number;
};

export type HermesCoverage = {
  roomId: string;
  chatsWithMessages: number;
  covered: number;
  missing: HermesCoverageChat[];
  chats: HermesCoverageChat[];
};

export function hermesHistoryCoverage(
  finiteChat: HostedChatState,
  sessions: HermesSessionRow[],
): HermesCoverage | null {
  const roomId = finiteChat.hosted_agent_binding?.canonical_room_id;
  if (!roomId) return null;
  const byChat = new Map<string, HermesSessionRow[]>();
  for (const session of sessions) {
    if (session.source !== "finitechat" || session.chatId !== roomId || !session.threadId) continue;
    const rows = byChat.get(session.threadId) ?? [];
    rows.push(session);
    byChat.set(session.threadId, rows);
  }
  const chats: HermesCoverageChat[] = [];
  for (const topic of finiteChat.topics) {
    if (topic.room_id !== roomId) continue;
    for (const chat of topic.chats) {
      if (chat.message_count === 0) continue;
      const matches = byChat.get(chat.chat_id) ?? [];
      chats.push({
        topicTitle: topic.title,
        chatId: chat.chat_id,
        title: chat.title,
        finiteChatMessages: chat.message_count,
        hermesMessages: matches.length
          ? matches.reduce((total, session) => total + session.messageCount, 0)
          : null,
        hermesSessions: matches.length,
      });
    }
  }
  const missing = chats.filter((chat) => chat.hermesSessions === 0);
  return {
    roomId,
    chatsWithMessages: chats.length,
    covered: chats.length - missing.length,
    missing,
    chats,
  };
}
