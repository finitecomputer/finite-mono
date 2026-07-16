import type { HostedChatTopic } from "@/lib/hosted-web-device";

export const HOME_TOPIC_ID = "home";

export function canonicalNewChatTopic(topics: HostedChatTopic[]) {
  return topics.find((topic) => topic.topic_id === HOME_TOPIC_ID) ?? null;
}
