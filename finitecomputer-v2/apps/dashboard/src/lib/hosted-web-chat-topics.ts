import type { HostedChatSummary, HostedChatTopic } from "@/lib/hosted-web-device";

export const HOME_TOPIC_ID = "home";

export function canonicalNewChatTopic(topics: HostedChatTopic[]) {
  return topics.find((topic) => topic.topic_id === HOME_TOPIC_ID) ?? null;
}

export type SidebarChat = HostedChatSummary & { source_topic_id: string };
export type SidebarTopic = Omit<HostedChatTopic, "chats"> & { chats: SidebarChat[] };

export function sidebarChatKey(chat: SidebarChat) {
  return JSON.stringify([chat.source_topic_id, chat.chat_id]);
}

/** A view of canonical routes. Never pass a displayed folder as a chat's route. */
export function sidebarTopics(topics: HostedChatTopic[]): SidebarTopic[] {
  const folders = topics.map((topic) => ({ ...topic, chats: [] as SidebarChat[], unread_count: 0 }));
  for (const topic of topics) {
    for (const chat of topic.chats) {
      const destination = folders.find((folder) => folder.room_id === topic.room_id
        && folder.topic_id === chat.placement?.topic_id) ?? folders.find((folder) =>
          folder.room_id === topic.room_id && folder.topic_id === topic.topic_id)!;
      destination.chats.push({ ...chat, source_topic_id: topic.topic_id });
      destination.unread_count += chat.unread_count || 0;
    }
  }
  for (const folder of folders) {
    folder.chats.sort((a, b) => {
      if (!a.placement || !b.placement) return 0; // Older service retains its original order.
      const left = a.placement.position;
      const right = b.placement.position;
      return left < right ? -1 : left > right ? 1 : sidebarChatKey(a).localeCompare(sidebarChatKey(b));
    });
  }
  return folders;
}
