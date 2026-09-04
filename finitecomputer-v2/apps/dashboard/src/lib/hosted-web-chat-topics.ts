import type { HostedChatTopic } from "@/lib/hosted-web-device";

export const HOME_TOPIC_ID = "home";

export function canonicalNewChatTopic(topics: HostedChatTopic[]) {
  // finitechat always has a Home topic; transports without one (the hermes
  // gateway renders projects + Recents) fall back to the first topic so New
  // chat still works.
  return topics.find((topic) => topic.topic_id === HOME_TOPIC_ID) ?? topics[0] ?? null;
}
