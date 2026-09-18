import type { HostedChatSelection } from "@/lib/hosted-web-chat-selection";

export function hostedChatViewQuery(selection: HostedChatSelection | null, limit = 50) {
  if (!selection?.selected_room_id) return "";
  const query = new URLSearchParams({ room_id: selection.selected_room_id });
  if (selection.selected_topic_id) query.set("topic_id", selection.selected_topic_id);
  if (selection.selected_chat_id) query.set("chat_id", selection.selected_chat_id);
  query.set("limit", String(limit));
  return `?${query}`;
}

/** Forward only the read-only transcript scope, never arbitrary upstream parameters. */
export function hostedChatViewQueryFromRequest(request: Request) {
  const input = new URL(request.url).searchParams;
  const query = new URLSearchParams();
  for (const key of ["room_id", "topic_id", "chat_id", "limit"]) {
    const value = input.get(key);
    if (value !== null) query.set(key, value);
  }
  return query.size ? `?${query}` : "";
}
